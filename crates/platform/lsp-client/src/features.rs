//! The language features over the servers of a document: each request goes
//! to the running servers whose spec lists the feature and that offer it;
//! answers are converted to the model on the reader thread, in the text the
//! request was made for, and dropped when the document changed meanwhile.

use std::ops::Range;
use std::sync::Arc;

use code_intel::{
    CodeAction, CodeActionContext, CompletionItem, CompletionTrigger, Completions,
    DocumentHighlight, DocumentId, Feature, FormatOptions, Hover, Intel, IntelError, Location,
    Point, ServerId, SignatureContext, SignatureHelp, TextEdit, WorkspaceEdit, ready,
};
use futures::future::join_all;
use serde_json::{Value, json};

use crate::convert::{self, Raw};
use crate::documents::Snapshot;
use crate::encoding::Text;
use crate::rpc::RpcError;
use crate::server::{Context, LanguageServer, Session};

/// A running server a request goes to.
#[derive(Clone)]
pub(crate) struct Target {
    pub server: Arc<LanguageServer>,
    pub session: Arc<Session>,
    pub uri: String,
}

/// The document a request is about, as of the request.
#[derive(Clone)]
pub(crate) struct Request {
    pub cx: Arc<Context>,
    pub doc: DocumentId,
    pub snapshot: Snapshot,
}

/// The capability and the registration method of a feature.
pub(crate) fn provider(feature: Feature) -> (&'static str, &'static str) {
    match feature {
        Feature::Completion => ("completionProvider", "textDocument/completion"),
        Feature::Hover => ("hoverProvider", "textDocument/hover"),
        Feature::SignatureHelp => ("signatureHelpProvider", "textDocument/signatureHelp"),
        Feature::Definition => ("definitionProvider", "textDocument/definition"),
        Feature::TypeDefinition => ("typeDefinitionProvider", "textDocument/typeDefinition"),
        Feature::References => ("referencesProvider", "textDocument/references"),
        Feature::CodeActions | Feature::OrganizeImports => {
            ("codeActionProvider", "textDocument/codeAction")
        }
        Feature::Formatting => ("documentFormattingProvider", "textDocument/formatting"),
        Feature::Rename => ("renameProvider", "textDocument/rename"),
        Feature::DocumentHighlights => (
            "documentHighlightProvider",
            "textDocument/documentHighlight",
        ),
        Feature::Diagnostics => ("diagnosticProvider", "textDocument/diagnostic"),
        Feature::DocumentSymbols => ("documentSymbolProvider", "textDocument/documentSymbol"),
        Feature::WorkspaceSymbols => ("workspaceSymbolProvider", "workspace/symbol"),
        Feature::InlayHints => ("inlayHintProvider", "textDocument/inlayHint"),
        Feature::SemanticTokens => ("semanticTokensProvider", "textDocument/semanticTokens"),
    }
}

/// The model's error for a request's error.
pub(crate) fn intel_error(server: &ServerId, error: RpcError) -> IntelError {
    if error.is_cancelled() {
        return IntelError::Cancelled;
    }
    match error {
        RpcError::Server { code, message } => IntelError::Server {
            server: server.clone(),
            code,
            message,
        },
        RpcError::Closed => IntelError::NotRunning,
        RpcError::Outdated => IntelError::Cancelled,
        RpcError::Malformed(message) => IntelError::Other(message),
    }
}

impl Request {
    fn text(&self) -> Text {
        Text::new(self.snapshot.text.clone())
    }

    /// Fails when the document changed since the request.
    fn check(&self) -> Result<(), RpcError> {
        match self.cx.documents.get(&self.doc) {
            Some(now) if now.version == self.snapshot.version => Ok(()),
            _ => Err(RpcError::Outdated),
        }
    }

    /// Sends a request to one server; `convert` runs on its reader thread
    /// while the document is unchanged.
    fn call<T: Send + 'static>(
        &self,
        target: &Target,
        method: &'static str,
        params: Value,
        convert: impl FnOnce(&Request, &Target, Value) -> T + Send + 'static,
    ) -> Intel<T> {
        let request = self.clone();
        let on_reader = target.clone();
        let id = target.server.id().clone();
        let pending = target
            .session
            .connection
            .request(method, params, move |value| {
                request.check()?;
                Ok(convert(&request, &on_reader, value))
            });
        Box::pin(async move { pending.await.map_err(|e| intel_error(&id, e)) })
    }

    fn text_document(&self, target: &Target) -> Value {
        json!({ "uri": target.uri })
    }

    fn position(&self, target: &Target, at: Point) -> Value {
        json!(self.text().position(at, target.session.encoding()))
    }
}

/// The answers of several servers; an error only when none answered.
async fn gather<T>(calls: Vec<Intel<T>>) -> Result<Vec<T>, IntelError> {
    let mut answers = Vec::new();
    let mut first_error = None;
    for result in join_all(calls).await {
        match result {
            Ok(answer) => answers.push(answer),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) if answers.is_empty() => Err(error),
        _ => Ok(answers),
    }
}

pub(crate) fn completion(
    request: Request,
    targets: Vec<Target>,
    at: Point,
    trigger: CompletionTrigger,
) -> Intel<Completions> {
    let calls = targets
        .iter()
        .map(|target| {
            let triggers = target
                .session
                .info()
                .map(|info| info.strings("completionProvider", "triggerCharacters"))
                .unwrap_or_default();
            let context = match trigger {
                CompletionTrigger::Character(c) if triggers.contains(&c.to_string()) => {
                    json!({"triggerKind": 2, "triggerCharacter": c.to_string()})
                }
                CompletionTrigger::Incomplete => json!({"triggerKind": 3}),
                _ => json!({"triggerKind": 1}),
            };
            let params = json!({
                "textDocument": request.text_document(target),
                "position": request.position(target, at),
                "context": context,
            });
            request.call(target, "textDocument/completion", params, |r, t, value| {
                convert::completions(
                    &value,
                    &r.text(),
                    t.session.encoding(),
                    t.server.id(),
                    t.session.generation,
                )
            })
        })
        .collect();
    Box::pin(async move {
        let answers = gather(calls).await?;
        let mut all = Completions::default();
        for answer in answers {
            all.incomplete |= answer.incomplete;
            all.items.extend(answer.items);
        }
        Ok(all)
    })
}

/// The trigger characters of the servers' completion.
pub(crate) fn completion_triggers(targets: &[Target]) -> Vec<char> {
    let mut triggers = Vec::new();
    for target in targets {
        let strings = target
            .session
            .info()
            .map(|info| info.strings("completionProvider", "triggerCharacters"))
            .unwrap_or_default();
        for s in strings {
            let mut chars = s.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && !triggers.contains(&c)
            {
                triggers.push(c);
            }
        }
    }
    triggers
}

pub(crate) fn resolve_completion(
    request: Request,
    target: Option<Target>,
    item: CompletionItem,
) -> Intel<CompletionItem> {
    let Some(target) = target else {
        return ready(Ok(item));
    };
    let Some(raw) = Raw::of(item.token.as_ref()) else {
        return ready(Ok(item));
    };
    let resolves = target.session.info().and_then(|info| {
        info.option("completionProvider", "resolveProvider")
            .cloned()
    }) == Some(Value::Bool(true));
    if raw.generation != target.session.generation || !resolves {
        return ready(Ok(item));
    }
    let params = raw.value.clone();
    let call = request.call(&target, "completionItem/resolve", params, |r, t, value| {
        convert::completion_item(
            &value,
            &r.text(),
            t.session.encoding(),
            t.server.id(),
            t.session.generation,
        )
    });
    Box::pin(async move { Ok(call.await?.unwrap_or(item)) })
}

pub(crate) fn hover(request: Request, targets: Vec<Target>, at: Point) -> Intel<Option<Hover>> {
    let calls = targets
        .iter()
        .map(|target| {
            let params = json!({
                "textDocument": request.text_document(target),
                "position": request.position(target, at),
            });
            request.call(target, "textDocument/hover", params, |r, t, value| {
                convert::hover(&value, &r.text(), t.session.encoding())
            })
        })
        .collect();
    // The first server (in the product's order) with something to say.
    Box::pin(async move { Ok(gather(calls).await?.into_iter().flatten().next()) })
}

pub(crate) fn signature_help(
    request: Request,
    targets: Vec<Target>,
    at: Point,
    context: SignatureContext,
) -> Intel<Option<SignatureHelp>> {
    let calls = targets
        .iter()
        .map(|target| {
            let encoding = target.session.encoding();
            let params = json!({
                "textDocument": request.text_document(target),
                "position": request.position(target, at),
                "context": convert::signature_context(&context, encoding),
            });
            request.call(
                target,
                "textDocument/signatureHelp",
                params,
                |_, t, value| convert::signature_help(&value, t.session.encoding()),
            )
        })
        .collect();
    Box::pin(async move { Ok(gather(calls).await?.into_iter().flatten().next()) })
}

/// Definition, type definition or references: every server's locations.
pub(crate) fn locations(
    request: Request,
    targets: Vec<Target>,
    method: &'static str,
    at: Point,
    include_declaration: Option<bool>,
) -> Intel<Vec<Location>> {
    let calls = targets
        .iter()
        .map(|target| {
            let mut params = json!({
                "textDocument": request.text_document(target),
                "position": request.position(target, at),
            });
            if let Some(include) = include_declaration {
                params["context"] = json!({ "includeDeclaration": include });
            }
            request.call(target, method, params, |r, t, value| {
                convert::locations(&value, t.session.encoding(), &|path| r.cx.text_of(path))
            })
        })
        .collect();
    Box::pin(async move {
        let mut all: Vec<Location> = Vec::new();
        for location in gather(calls).await?.into_iter().flatten() {
            if !all.contains(&location) {
                all.push(location);
            }
        }
        Ok(all)
    })
}

pub(crate) fn code_actions(
    request: Request,
    targets: Vec<Target>,
    range: Range<Point>,
    context: CodeActionContext,
) -> Intel<Vec<CodeAction>> {
    let calls = targets
        .iter()
        .map(|target| {
            let id = target.server.id().clone();
            let own: Vec<_> = context
                .diagnostics
                .iter()
                .filter(|d| Raw::of(d.token.as_ref()).is_some_and(|raw| raw.server == id))
                .cloned()
                .collect();
            let mut lsp_context = json!({
                "diagnostics": own
                    .iter()
                    .filter_map(|d| Raw::of(d.token.as_ref()).map(|raw| raw.value.clone()))
                    .collect::<Vec<_>>(),
                "triggerKind": match context.trigger {
                    code_intel::ActionTrigger::Invoked => 1,
                    code_intel::ActionTrigger::Automatic => 2,
                },
            });
            if let Some(only) = &context.only {
                lsp_context["only"] =
                    json!(only.iter().map(ToString::to_string).collect::<Vec<_>>());
            }
            let params = json!({
                "textDocument": request.text_document(target),
                "range": convert::lsp_range(&range, &request.text(), target.session.encoding()),
                "context": lsp_context,
            });
            let range = range.clone();
            let context = context.clone();
            request.call(
                target,
                "textDocument/codeAction",
                params,
                move |r, t, value| {
                    let mut actions = convert::code_actions(
                        &value,
                        t.session.encoding(),
                        t.server.id(),
                        t.session.generation,
                        &own,
                        &|path| r.cx.text_of(path),
                    );
                    actions.retain(|a| context.wants(&a.kind));
                    if let Some(customizer) = &t.server.spec().customizer {
                        customizer.code_actions(
                            &r.doc,
                            &r.snapshot.text,
                            range,
                            &context,
                            &mut actions,
                        );
                    }
                    actions
                },
            )
        })
        .collect();
    Box::pin(async move {
        let mut all: Vec<CodeAction> = gather(calls).await?.into_iter().flatten().collect();
        // Quick fixes first (stable: each server's order stays).
        all.sort_by_key(|a| !a.kind.is_quick_fix());
        Ok(all)
    })
}

pub(crate) fn resolve_code_action(
    request: Request,
    target: Option<Target>,
    action: CodeAction,
) -> Intel<CodeAction> {
    let Some(target) = target else {
        return ready(Ok(action));
    };
    let Some(raw) = Raw::of(action.token.as_ref()) else {
        return ready(Ok(action));
    };
    let resolves = target.session.info().and_then(|info| {
        info.option("codeActionProvider", "resolveProvider")
            .cloned()
    }) == Some(Value::Bool(true));
    if action.edit.is_some() || raw.generation != target.session.generation || !resolves {
        return ready(Ok(action));
    }
    let params = raw.value.clone();
    let diagnostics = action.diagnostics.clone();
    let call = request.call(&target, "codeAction/resolve", params, move |r, t, value| {
        convert::code_action(
            &value,
            t.session.encoding(),
            t.server.id(),
            t.session.generation,
            &diagnostics,
            &|path| r.cx.text_of(path),
        )
    });
    Box::pin(async move {
        let mut action = action;
        if let Some(resolved) = call.await? {
            // The product's title, nesting and kind stay.
            action.edit = resolved.edit;
            if resolved.command.is_some() {
                action.command = resolved.command;
            }
        }
        Ok(action)
    })
}

pub(crate) fn execute_command(target: Target, raw: &Raw) -> Intel<()> {
    let mut params = json!({ "command": raw.value.get("command").cloned().unwrap_or(Value::Null) });
    if let Some(arguments) = raw.value.get("arguments") {
        params["arguments"] = arguments.clone();
    }
    let id = target.server.id().clone();
    let pending = target
        .session
        .connection
        .request("workspace/executeCommand", params, |_| Ok(()));
    Box::pin(async move { pending.await.map_err(|e| intel_error(&id, e)) })
}

pub(crate) fn format(
    request: Request,
    target: Target,
    range: Option<Range<Point>>,
    options: FormatOptions,
) -> Intel<Vec<TextEdit>> {
    let mut params = json!({
        "textDocument": request.text_document(&target),
        "options": {"tabSize": options.tab_size, "insertSpaces": options.insert_spaces},
    });
    let method = match &range {
        Some(range) => {
            params["range"] = convert::lsp_range(range, &request.text(), target.session.encoding());
            "textDocument/rangeFormatting"
        }
        None => "textDocument/formatting",
    };
    request.call(&target, method, params, |r, t, value| {
        convert::text_edits(&value, &r.text(), t.session.encoding())
    })
}

pub(crate) fn rename(
    request: Request,
    target: Target,
    at: Point,
    new_name: String,
) -> Intel<WorkspaceEdit> {
    let params = json!({
        "textDocument": request.text_document(&target),
        "position": request.position(&target, at),
        "newName": new_name,
    });
    let call = request.call(&target, "textDocument/rename", params, |r, t, value| {
        if value.is_null() {
            return Ok(WorkspaceEdit::default());
        }
        convert::workspace_edit(&value, t.session.encoding(), &|path| r.cx.text_of(path))
    });
    Box::pin(async move { call.await?.map_err(IntelError::Other) })
}

pub(crate) fn document_highlights(
    request: Request,
    target: Target,
    at: Point,
) -> Intel<Vec<DocumentHighlight>> {
    let params = json!({
        "textDocument": request.text_document(&target),
        "position": request.position(&target, at),
    });
    request.call(
        &target,
        "textDocument/documentHighlight",
        params,
        |r, t, value| convert::highlights(&value, &r.text(), t.session.encoding()),
    )
}
