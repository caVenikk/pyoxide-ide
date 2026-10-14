//! [`CodeIntel`] over the language servers of a project.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use code_intel::{
    AppliedEdit, CodeAction, CodeActionContext, CodeActionKind, CodeIntel, Command, CompletionItem,
    CompletionTrigger, Completions, DocumentHighlight, DocumentId, Feature, FileChange,
    FileChangeKind, FormatOptions, Hover, Intel, IntelError, IntelEvent, Location, Point, Rope,
    ServerId, ServerState, ServerStatus, SignatureContext, SignatureHelp, TextEdit, WorkspaceEdit,
    ready,
};
use serde_json::json;

use crate::convert::Raw;
use crate::features::{self, Request, Target};
use crate::server::{Context, LanguageServer, Timeouts};
use crate::spec::ServerSpec;

/// The language services of a project over its servers: each document
/// goes to the servers of its language, each request to the servers that
/// serve its feature. Servers start when the first document of theirs
/// opens.
///
/// Routing, as ADR 0010: a feature goes to the running servers whose spec
/// lists it and that offer it. Completion, definitions, references and code
/// actions are merged from all of them (code actions quick fixes first);
/// hover and signature help come from the first that has one; formatting,
/// rename and highlights from the first server only. Diagnostics of a
/// document arrive with every server's items.
pub struct LspCodeIntel {
    cx: Arc<Context>,
    servers: Mutex<Vec<Arc<LanguageServer>>>,
    /// Servers replaced or removed by [`LspCodeIntel::set_specs`] whose
    /// processes may still be stopping.
    retired: Mutex<Vec<Arc<LanguageServer>>>,
    /// Shut down: no server starts again.
    closed: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LspCodeIntel {
    /// The services over these servers (none is fine: see
    /// [`LspCodeIntel::set_specs`]).
    pub fn new(specs: Vec<ServerSpec>) -> Arc<Self> {
        Self::with_timeouts(specs, Timeouts::default())
    }

    /// With other waiting times than the reference's (tests, slow
    /// machines).
    pub fn with_timeouts(specs: Vec<ServerSpec>, timeouts: Timeouts) -> Arc<Self> {
        let cx = Context::new(timeouts);
        let servers = specs
            .into_iter()
            .map(|spec| LanguageServer::new(spec, cx.clone()))
            .collect();
        Arc::new(Self {
            cx,
            servers: Mutex::new(servers),
            retired: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        })
    }

    /// The servers it runs.
    pub fn specs(&self) -> Vec<ServerSpec> {
        self.servers()
            .iter()
            .map(|server| ServerSpec::clone(&server.spec()))
            .collect()
    }

    /// Replaces the servers (the product found its executables, the
    /// interpreter or the settings changed). A server whose spec is gone,
    /// or changed in how it starts (command line, folder, initialization
    /// options, settings), is stopped; a changed or new one starts when a
    /// document of its languages is open, and gets the open documents. A
    /// server with only a new name, icon, features, languages or customizer
    /// keeps running.
    pub fn set_specs(&self, specs: Vec<ServerSpec>) {
        let mut stopping = Vec::new();
        let mut fresh = Vec::new();
        {
            let mut servers = lock(&self.servers);
            let mut old = std::mem::take(&mut *servers);
            for spec in specs {
                let same = old.iter().position(|s| *s.id() == spec.id);
                match same {
                    Some(i) if !old[i].needs_restart_for(&spec) => {
                        let server = old.remove(i);
                        server.update(spec);
                        servers.push(server);
                    }
                    _ => {
                        if let Some(i) = same {
                            stopping.push(old.remove(i));
                        }
                        let server = LanguageServer::new(spec, self.cx.clone());
                        fresh.push(server.clone());
                        servers.push(server);
                    }
                }
            }
            stopping.extend(old);
        }
        for server in &stopping {
            server.stop();
        }
        {
            let mut retired = lock(&self.retired);
            retired.retain(|server| server.is_stopping());
            retired.extend(stopping);
        }
        if self.is_closed() {
            return;
        }
        let languages: Vec<String> = self
            .cx
            .documents
            .all()
            .into_iter()
            .map(|(_, s)| s.language)
            .collect();
        for server in fresh {
            let spec = server.spec();
            if languages.iter().any(|language| spec.serves(language)) {
                server.ensure_started();
            }
        }
    }

    /// The settings or the customizers' answers changed: the running
    /// servers are told with an empty `workspace/didChangeConfiguration`
    /// and ask for their settings again (`workspace/configuration`).
    pub fn configuration_changed(&self) {
        for server in self.servers() {
            if let Some(session) = server.running_session() {
                session
                    .connection
                    .notify("workspace/didChangeConfiguration", json!({"settings": {}}));
            }
        }
    }

    fn servers(&self) -> Vec<Arc<LanguageServer>> {
        lock(&self.servers).clone()
    }

    fn server(&self, id: &ServerId) -> Option<Arc<LanguageServer>> {
        lock(&self.servers).iter().find(|s| s.id() == id).cloned()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// The servers of the document's language whose spec lists `feature`.
    fn candidates(&self, language: &str, feature: Feature) -> Vec<Arc<LanguageServer>> {
        self.servers()
            .into_iter()
            .filter(|s| {
                let spec = s.spec();
                spec.serves(language) && spec.features.contains(feature)
            })
            .collect()
    }

    /// Where a request about `doc` goes: the running servers that serve
    /// `feature` and offer `capability`. Unsupported when no server serves
    /// it, NotRunning when the ones that would are not running.
    fn route(
        &self,
        doc: &DocumentId,
        features: &[Feature],
        capability: (&str, &str),
    ) -> Result<(Request, Vec<Target>), IntelError> {
        let snapshot = self.cx.documents.get(doc).ok_or(IntelError::Unsupported)?;
        let mut candidates: Vec<Arc<LanguageServer>> = Vec::new();
        for feature in features {
            for server in self.candidates(&snapshot.language, *feature) {
                if !candidates.iter().any(|c| Arc::ptr_eq(c, &server)) {
                    candidates.push(server);
                }
            }
        }
        if candidates.is_empty() {
            return Err(IntelError::Unsupported);
        }
        let mut waiting = false;
        let mut targets = Vec::new();
        for server in candidates {
            let Some(session) = server.running_session() else {
                waiting |= server.status().state != ServerState::Running;
                continue;
            };
            if !session.provides(capability.0, capability.1) {
                continue;
            }
            let Some(uri) = session.uri_of(doc) else {
                waiting = true;
                continue;
            };
            targets.push(Target {
                server,
                session,
                uri,
            });
        }
        if targets.is_empty() {
            return Err(if waiting {
                IntelError::NotRunning
            } else {
                IntelError::Unsupported
            });
        }
        let request = Request {
            cx: self.cx.clone(),
            doc: doc.clone(),
            snapshot,
        };
        Ok((request, targets))
    }

    fn route_feature(
        &self,
        doc: &DocumentId,
        feature: Feature,
    ) -> Result<(Request, Vec<Target>), IntelError> {
        self.route(doc, &[feature], features::provider(feature))
    }

    /// The running server of a token's value, if it still runs that
    /// session.
    fn target_of(&self, doc: &DocumentId, raw: &Raw) -> Option<(Request, Target)> {
        let server = self.server(&raw.server)?;
        let session = server.running_session()?;
        if session.generation != raw.generation {
            return None;
        }
        let snapshot = self.cx.documents.get(doc)?;
        let uri = session.uri_of(doc)?;
        Some((
            Request {
                cx: self.cx.clone(),
                doc: doc.clone(),
                snapshot,
            },
            Target {
                server,
                session,
                uri,
            },
        ))
    }
}

fn routed<T: Send + 'static>(
    route: Result<(Request, Vec<Target>), IntelError>,
    call: impl FnOnce(Request, Vec<Target>) -> Intel<T>,
) -> Intel<T> {
    match route {
        Ok((request, targets)) => call(request, targets),
        Err(error) => ready(Err(error)),
    }
}

impl CodeIntel for LspCodeIntel {
    fn did_open(&self, doc: &DocumentId, language: &str, version: u64, text: &Rope) {
        self.cx.documents.open(doc, language, version, text);
        if self.is_closed() {
            return;
        }
        for server in self.servers() {
            if server.spec().serves(language) {
                server.ensure_started();
                server.open(doc);
            }
        }
    }

    fn did_change(&self, doc: &DocumentId, version: u64, edits: &[AppliedEdit], text: &Rope) {
        let Some(base) = self.cx.documents.change(doc, version, Some(edits), text) else {
            return;
        };
        for server in self.servers() {
            server.change(doc, Some(base), version, edits, text);
        }
    }

    fn did_reload(&self, doc: &DocumentId, version: u64, text: &Rope) {
        if self.cx.documents.change(doc, version, None, text).is_none() {
            return;
        }
        for server in self.servers() {
            server.change(doc, None, version, &[], text);
        }
    }

    fn did_save(&self, doc: &DocumentId, text: &Rope) {
        let change = [FileChange::new(doc.path(), FileChangeKind::Changed)];
        for server in self.servers() {
            server.save(doc, text);
            server.files_changed(&change);
        }
    }

    fn did_close(&self, doc: &DocumentId) {
        self.cx.documents.close(doc);
        self.cx.diagnostics.close(doc);
        for server in self.servers() {
            server.close(doc);
        }
    }

    fn files_changed(&self, changes: &[FileChange]) {
        for server in self.servers() {
            server.files_changed(changes);
        }
    }

    fn completion(
        &self,
        doc: &DocumentId,
        at: Point,
        trigger: CompletionTrigger,
    ) -> Intel<Completions> {
        routed(self.route_feature(doc, Feature::Completion), |r, t| {
            features::completion(r, t, at, trigger)
        })
    }

    fn resolve_completion(&self, doc: &DocumentId, item: CompletionItem) -> Intel<CompletionItem> {
        let found = Raw::of(item.token.as_ref()).and_then(|raw| self.target_of(doc, raw));
        match found {
            Some((request, target)) => features::resolve_completion(request, Some(target), item),
            None => ready(Ok(item)),
        }
    }

    fn completion_triggers(&self, doc: &DocumentId) -> Vec<char> {
        match self.route_feature(doc, Feature::Completion) {
            Ok((_, targets)) => features::completion_triggers(&targets),
            Err(_) => Vec::new(),
        }
    }

    fn hover(&self, doc: &DocumentId, at: Point) -> Intel<Option<Hover>> {
        routed(self.route_feature(doc, Feature::Hover), |r, t| {
            features::hover(r, t, at)
        })
    }

    fn signature_help(
        &self,
        doc: &DocumentId,
        at: Point,
        context: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        routed(self.route_feature(doc, Feature::SignatureHelp), |r, t| {
            features::signature_help(r, t, at, context)
        })
    }

    fn definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>> {
        routed(self.route_feature(doc, Feature::Definition), |r, t| {
            features::locations(r, t, "textDocument/definition", at, None)
        })
    }

    fn type_definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>> {
        routed(self.route_feature(doc, Feature::TypeDefinition), |r, t| {
            features::locations(r, t, "textDocument/typeDefinition", at, None)
        })
    }

    fn references(
        &self,
        doc: &DocumentId,
        at: Point,
        include_declaration: bool,
    ) -> Intel<Vec<Location>> {
        routed(self.route_feature(doc, Feature::References), |r, t| {
            features::locations(
                r,
                t,
                "textDocument/references",
                at,
                Some(include_declaration),
            )
        })
    }

    fn code_actions(
        &self,
        doc: &DocumentId,
        range: Range<Point>,
        context: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        // Optimize Imports also goes to servers that only organize imports.
        let mut features = vec![Feature::CodeActions];
        if context.wants(&CodeActionKind::organize_imports()) {
            features.push(Feature::OrganizeImports);
        }
        let route = self.route(doc, &features, features::provider(Feature::CodeActions));
        routed(route, |r, t| features::code_actions(r, t, range, context))
    }

    fn resolve_code_action(&self, doc: &DocumentId, action: CodeAction) -> Intel<CodeAction> {
        let found = Raw::of(action.token.as_ref()).and_then(|raw| self.target_of(doc, raw));
        match found {
            Some((request, target)) => features::resolve_code_action(request, Some(target), action),
            None => ready(Ok(action)),
        }
    }

    fn format(
        &self,
        doc: &DocumentId,
        range: Option<Range<Point>>,
        options: FormatOptions,
    ) -> Intel<Vec<TextEdit>> {
        let capability = match range {
            Some(_) => (
                "documentRangeFormattingProvider",
                "textDocument/rangeFormatting",
            ),
            None => features::provider(Feature::Formatting),
        };
        // One server formats: the first the product lists.
        routed(
            self.route(doc, &[Feature::Formatting], capability),
            |r, mut t| features::format(r, t.swap_remove(0), range, options),
        )
    }

    fn execute_command(&self, command: Command) -> Intel<()> {
        let Some(raw) = command.token.downcast_ref::<Raw>() else {
            return ready(Err(IntelError::Unsupported));
        };
        let Some(server) = self.server(&raw.server) else {
            return ready(Err(IntelError::NotRunning));
        };
        match server.running_session() {
            Some(session) if session.generation == raw.generation => features::execute_command(
                Target {
                    server,
                    session,
                    uri: String::new(),
                },
                raw,
            ),
            _ => ready(Err(IntelError::NotRunning)),
        }
    }

    fn rename(&self, doc: &DocumentId, at: Point, new_name: String) -> Intel<WorkspaceEdit> {
        routed(self.route_feature(doc, Feature::Rename), |r, mut t| {
            features::rename(r, t.swap_remove(0), at, new_name)
        })
    }

    fn document_highlights(&self, doc: &DocumentId, at: Point) -> Intel<Vec<DocumentHighlight>> {
        routed(
            self.route_feature(doc, Feature::DocumentHighlights),
            |r, mut t| features::document_highlights(r, t.swap_remove(0), at),
        )
    }

    fn status(&self) -> Vec<ServerStatus> {
        self.servers().iter().map(|s| s.status()).collect()
    }

    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.cx.hub.subscribe()
    }

    fn restart(&self, server: Option<&ServerId>) {
        if self.is_closed() {
            return;
        }
        match server {
            Some(id) => {
                if let Some(server) = self.server(id) {
                    server.restart();
                }
            }
            None => {
                for server in self.servers() {
                    server.restart();
                }
            }
        }
    }

    fn stop(&self, server: &ServerId) {
        if let Some(server) = self.server(server) {
            server.stop();
        }
    }

    fn shutdown(&self) -> Intel<()> {
        self.closed.store(true, Ordering::Release);
        let servers = self.servers();
        for server in &servers {
            server.stop();
        }
        let stopped: Vec<_> = servers
            .iter()
            .chain(lock(&self.retired).iter())
            .map(|server| server.stopped())
            .collect();
        Box::pin(async move {
            futures::future::join_all(stopped).await;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use code_intel::{Feature, Features, IntelError, ServerState};
    use futures::executor::block_on;
    use process::CommandLine;

    use super::*;

    #[test]
    fn servers_are_listed_and_unrouted_features_unsupported() {
        let ruff = ServerSpec::new("ruff", CommandLine::new("ruff").arg("server"), "/p")
            .with_languages(["python"])
            .with_features(Features::of(&[Feature::Diagnostics, Feature::Formatting]));
        assert!(ruff.serves("python"));
        assert!(format!("{ruff:?}").contains("ruff"));
        let intel: Arc<dyn CodeIntel> = LspCodeIntel::new(vec![ruff]);
        let status = intel.status();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].state, ServerState::Stopped);
        assert!(status[0].features.contains(Feature::Formatting));
        let doc = DocumentId::new("/p/a.txt");
        // Not a language of the server: nothing starts.
        intel.did_open(&doc, "plaintext", 1, &Rope::from_str("x = 1\n"));
        assert_eq!(intel.status()[0].state, ServerState::Stopped);
        assert_eq!(
            block_on(intel.completion(&doc, Point::new(0, 1), CompletionTrigger::Invoked)),
            Err(IntelError::Unsupported)
        );
        // A document that is not open.
        assert_eq!(
            block_on(intel.hover(&DocumentId::new("/p/b.py"), Point::new(0, 0))),
            Err(IntelError::Unsupported)
        );
        assert!(intel.completion_triggers(&doc).is_empty());
        assert_eq!(block_on(intel.shutdown()), Ok(()));
    }
}
