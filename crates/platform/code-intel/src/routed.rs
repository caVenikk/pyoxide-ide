//! [`RoutedCodeIntel`]: the language services of several providers (the
//! product's and its modules', ADR 0022) as one [`CodeIntel`].
//!
//! - A document goes to the providers whose languages contain its
//!   language id (the one it was opened with); a provider with no
//!   languages serves every id. A document that was never opened goes to
//!   the providers of every id.
//! - Document calls reach the document's providers only; project-wide
//!   calls (`files_changed`, `restart`, `stop`, `shutdown`) reach all.
//! - List answers (completion, code actions, references, structure) are
//!   joined in provider order; single answers (hover, definition,
//!   signature help, format, rename, highlights) come from the first
//!   provider that answers something other than `Unsupported` or nothing.
//! - Completion items and code actions remember their provider, so that
//!   resolving them and running their commands reach it again.
//! - Events of all providers arrive through one hub. The diagnostics of a
//!   document served by several providers are joined: each provider's
//!   latest items are moved through the edits made since their version and
//!   sent as one report in the current text.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use futures::future::join_all;
use text::{AppliedEdit, Point, Rope};

use crate::positions::{map_offset, offset_to_point, point_to_offset};
use crate::{
    CodeAction, CodeActionContext, CodeIntel, Command, CompletionItem, CompletionTrigger,
    Completions, DefinitionTarget, Diagnostic, DocumentHighlight, DocumentId, DocumentSymbol,
    EventHub, FileChange, FormatOptions, Hover, Intel, IntelError, IntelEvent, Location, ServerId,
    ServerStatus, SignatureContext, SignatureHelp, TextEdit, Token, WorkspaceEdit, unsupported,
};

/// How many earlier texts of a document served by several providers are
/// kept, to move late diagnostics into the current text.
const KEPT_VERSIONS: usize = 32;

/// One provider of a [`RoutedCodeIntel`]: its services and the language
/// ids it serves (none: every id).
#[derive(Clone)]
pub struct Route {
    pub languages: Vec<&'static str>,
    pub services: Arc<dyn CodeIntel>,
}

impl Route {
    pub fn new(languages: &[&'static str], services: Arc<dyn CodeIntel>) -> Self {
        Self {
            languages: languages.to_vec(),
            services,
        }
    }

    /// Whether documents of a language id go to this provider.
    pub fn serves(&self, language: &str) -> bool {
        self.languages.is_empty() || self.languages.contains(&language)
    }
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("languages", &self.languages)
            .finish_non_exhaustive()
    }
}

/// The language services of several providers, routed by language id (see
/// the module documentation).
pub struct RoutedCodeIntel {
    routes: Vec<Route>,
    shared: Arc<Shared>,
}

/// What the calls and the event threads share.
#[derive(Default)]
struct Shared {
    hub: EventHub,
    docs: Mutex<HashMap<DocumentId, Doc>>,
}

/// An open document.
struct Doc {
    /// The indexes of its providers.
    providers: Vec<usize>,
    /// Its recent texts, when several providers serve it.
    texts: Option<Texts>,
    /// Each provider's latest diagnostics, when several serve it.
    diagnostics: Vec<Entry>,
}

/// A provider's diagnostics of a document, in the text of `version`.
struct Entry {
    provider: usize,
    version: u64,
    items: Vec<Diagnostic>,
}

/// The current text of a document and some before it.
struct Texts {
    version: u64,
    text: Rope,
    /// Earlier versions, oldest first, each with the edits that led from
    /// it to the next.
    history: VecDeque<Revision>,
}

struct Revision {
    version: u64,
    text: Rope,
    edits: Vec<AppliedEdit>,
}

impl Texts {
    fn new(version: u64, text: &Rope) -> Self {
        Self {
            version,
            text: text.clone(),
            history: VecDeque::new(),
        }
    }

    fn changed(&mut self, version: u64, edits: &[AppliedEdit], text: &Rope) {
        let old = std::mem::replace(&mut self.text, text.clone());
        self.history.push_back(Revision {
            version: self.version,
            text: old,
            edits: edits.to_vec(),
        });
        while self.history.len() > KEPT_VERSIONS {
            self.history.pop_front();
        }
        self.version = version;
    }

    fn reloaded(&mut self, version: u64, text: &Rope) {
        self.history.clear();
        self.text = text.clone();
        self.version = version;
    }

    /// A range in the text of `version` in the current text; `None` when
    /// that text is not kept.
    fn map(&self, version: u64, range: &Range<Point>) -> Option<Range<Point>> {
        if version == self.version {
            return Some(range.clone());
        }
        let first = self.history.iter().position(|r| r.version == version)?;
        let text = &self.history[first].text;
        let mut start = point_to_offset(text, range.start);
        let mut end = point_to_offset(text, range.end).max(start);
        for revision in self.history.iter().skip(first) {
            for edit in &revision.edits {
                start = map_offset(start, edit, false);
                end = map_offset(end, edit, true).max(start);
            }
        }
        Some(offset_to_point(&self.text, start)..offset_to_point(&self.text, end))
    }
}

/// The provider a completion item or a code action came from, around the
/// provider's own token.
struct Routed {
    provider: usize,
    inner: Option<Token>,
}

fn wrap(provider: usize, token: Option<Token>) -> Token {
    Token::new(Routed {
        provider,
        inner: token,
    })
}

/// The provider of a wrapped token, and the provider's own token.
fn unwrap(token: &Token) -> Option<(usize, Option<Token>)> {
    token
        .downcast_ref::<Routed>()
        .map(|r| (r.provider, r.inner.clone()))
}

fn wrap_item(provider: usize, mut item: CompletionItem) -> CompletionItem {
    item.token = Some(wrap(provider, item.token.take()));
    item
}

fn wrap_action(provider: usize, mut action: CodeAction) -> CodeAction {
    action.token = Some(wrap(provider, action.token.take()));
    if let Some(command) = &mut action.command {
        command.token = wrap(provider, Some(command.token.clone()));
    }
    action.children = std::mem::take(&mut action.children)
        .into_iter()
        .map(|child| wrap_action(provider, child))
        .collect();
    action
}

/// Restores the provider's tokens of an action; returns its provider.
fn unwrap_action(action: &mut CodeAction) -> Option<usize> {
    let mut provider = None;
    if let Some((p, inner)) = action.token.as_ref().and_then(unwrap) {
        action.token = inner;
        provider = Some(p);
    }
    if let Some(command) = &mut action.command
        && let Some((p, Some(inner))) = unwrap(&command.token)
    {
        command.token = inner;
        provider = provider.or(Some(p));
    }
    for child in &mut action.children {
        let p = unwrap_action(child);
        provider = provider.or(p);
    }
    provider
}

/// An answer that may be nothing: a single answer goes on to the next
/// provider then.
trait Answer {
    fn is_nothing(&self) -> bool;
}

impl<T> Answer for Option<T> {
    fn is_nothing(&self) -> bool {
        self.is_none()
    }
}

impl<T> Answer for Vec<T> {
    fn is_nothing(&self) -> bool {
        self.is_empty()
    }
}

impl Answer for WorkspaceEdit {
    fn is_nothing(&self) -> bool {
        self.is_empty()
    }
}

/// The first answer of the requests, in order, that is not `Unsupported`
/// and not nothing; else nothing if some provider answered so, else the
/// first error.
fn first<T: Answer + Send + 'static>(requests: Vec<Intel<T>>) -> Intel<T> {
    Box::pin(async move {
        let mut nothing = None;
        let mut error = None;
        for request in requests {
            match request.await {
                Ok(answer) if !answer.is_nothing() => return Ok(answer),
                Ok(answer) => nothing = nothing.or(Some(answer)),
                Err(IntelError::Unsupported) => {}
                Err(e) => error = error.or(Some(e)),
            }
        }
        match (nothing, error) {
            (Some(answer), _) => Ok(answer),
            (None, Some(e)) => Err(e),
            (None, None) => Err(IntelError::Unsupported),
        }
    })
}

/// The answers of the requests joined in order; the first error when no
/// provider answered.
fn joined<T: Send + 'static>(requests: Vec<Intel<Vec<T>>>) -> Intel<Vec<T>> {
    Box::pin(async move {
        let mut items = Vec::new();
        let mut answered = false;
        let mut error = None;
        for result in join_all(requests).await {
            match result {
                Ok(answer) => {
                    answered = true;
                    items.extend(answer);
                }
                Err(IntelError::Unsupported) => {}
                Err(e) => error = error.or(Some(e)),
            }
        }
        if answered {
            Ok(items)
        } else {
            Err(error.unwrap_or(IntelError::Unsupported))
        }
    })
}

impl RoutedCodeIntel {
    /// The providers, in order: the first one's answers come first.
    pub fn new(routes: Vec<Route>) -> Self {
        let shared = Arc::new(Shared::default());
        for (provider, route) in routes.iter().enumerate() {
            let events = route.services.events();
            let shared = Arc::downgrade(&shared);
            std::thread::Builder::new()
                .name(format!("code-intel-route-{provider}"))
                .spawn(move || forward(provider, &events, &shared))
                .expect("a thread for the events of a provider");
        }
        Self { routes, shared }
    }

    /// The providers, in order.
    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    fn docs(&self) -> MutexGuard<'_, HashMap<DocumentId, Doc>> {
        self.shared.docs()
    }

    /// The indexes of a document's providers.
    fn providers(&self, doc: &DocumentId) -> Vec<usize> {
        match self.docs().get(doc) {
            Some(d) => d.providers.clone(),
            None => (0..self.routes.len())
                .filter(|&i| self.routes[i].languages.is_empty())
                .collect(),
        }
    }

    /// The services of a document's providers, with their indexes.
    fn services(&self, doc: &DocumentId) -> Vec<(usize, &Arc<dyn CodeIntel>)> {
        self.providers(doc)
            .into_iter()
            .map(|i| (i, &self.routes[i].services))
            .collect()
    }

    fn each<T>(&self, doc: &DocumentId, f: impl Fn(&dyn CodeIntel) -> T) -> Vec<T> {
        self.services(doc)
            .into_iter()
            .map(|(_, s)| f(s.as_ref()))
            .collect()
    }
}

impl Shared {
    fn docs(&self) -> MutexGuard<'_, HashMap<DocumentId, Doc>> {
        self.docs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// An event of a provider: diagnostics of a document several providers
    /// serve are joined with the others' first.
    fn event(&self, provider: usize, event: IntelEvent) {
        let event = match event {
            IntelEvent::Diagnostics {
                doc,
                version,
                items,
            } => self.diagnostics(provider, doc, version, items),
            event => event,
        };
        self.hub.emit(event);
    }

    fn diagnostics(
        &self,
        provider: usize,
        doc: DocumentId,
        version: u64,
        items: Vec<Diagnostic>,
    ) -> IntelEvent {
        let mut docs = self.docs();
        let Some(Doc {
            texts: Some(texts),
            diagnostics,
            ..
        }) = docs.get_mut(&doc)
        else {
            // One provider serves it: its report as it is.
            return IntelEvent::Diagnostics {
                doc,
                version,
                items,
            };
        };
        diagnostics.retain(|e| e.provider != provider);
        diagnostics.push(Entry {
            provider,
            version,
            items,
        });
        diagnostics.sort_by_key(|e| e.provider);
        let mut joined = Vec::new();
        diagnostics.retain_mut(|entry| {
            let mut moved = Vec::with_capacity(entry.items.len());
            for item in &entry.items {
                let Some(range) = texts.map(entry.version, &item.range) else {
                    // Its text is gone: stale.
                    return false;
                };
                let mut item = item.clone();
                item.range = range;
                moved.push(item);
            }
            entry.version = texts.version;
            entry.items = moved;
            joined.extend(entry.items.iter().cloned());
            true
        });
        IntelEvent::Diagnostics {
            doc,
            version: texts.version,
            items: joined,
        }
    }
}

/// Forwards a provider's events until the provider or the router is gone.
fn forward(provider: usize, events: &flume::Receiver<IntelEvent>, shared: &Weak<Shared>) {
    while let Ok(event) = events.recv() {
        let Some(shared) = shared.upgrade() else {
            return;
        };
        shared.event(provider, event);
    }
}

impl CodeIntel for RoutedCodeIntel {
    fn did_open(&self, doc: &DocumentId, language: &str, version: u64, text: &Rope) {
        let providers: Vec<usize> = (0..self.routes.len())
            .filter(|&i| self.routes[i].serves(language))
            .collect();
        let texts = (providers.len() > 1).then(|| Texts::new(version, text));
        self.docs().insert(
            doc.clone(),
            Doc {
                providers: providers.clone(),
                texts,
                diagnostics: Vec::new(),
            },
        );
        for i in providers {
            self.routes[i]
                .services
                .did_open(doc, language, version, text);
        }
    }

    fn did_change(&self, doc: &DocumentId, version: u64, edits: &[AppliedEdit], text: &Rope) {
        if let Some(texts) = self.docs().get_mut(doc).and_then(|d| d.texts.as_mut()) {
            texts.changed(version, edits, text);
        }
        self.each(doc, |s| s.did_change(doc, version, edits, text));
    }

    fn did_reload(&self, doc: &DocumentId, version: u64, text: &Rope) {
        if let Some(texts) = self.docs().get_mut(doc).and_then(|d| d.texts.as_mut()) {
            texts.reloaded(version, text);
        }
        self.each(doc, |s| s.did_reload(doc, version, text));
    }

    fn did_save(&self, doc: &DocumentId, text: &Rope) {
        self.each(doc, |s| s.did_save(doc, text));
    }

    fn did_close(&self, doc: &DocumentId) {
        self.each(doc, |s| s.did_close(doc));
        self.docs().remove(doc);
    }

    fn files_changed(&self, changes: &[FileChange]) {
        for route in &self.routes {
            route.services.files_changed(changes);
        }
    }

    fn completion(
        &self,
        doc: &DocumentId,
        at: Point,
        trigger: CompletionTrigger,
    ) -> Intel<Completions> {
        let requests: Vec<_> = self
            .services(doc)
            .into_iter()
            .map(|(i, s)| (i, s.completion(doc, at, trigger)))
            .collect();
        Box::pin(async move {
            let (providers, requests): (Vec<usize>, Vec<_>) = requests.into_iter().unzip();
            let mut answer = Completions::default();
            let mut answered = false;
            let mut error = None;
            for (provider, result) in providers.into_iter().zip(join_all(requests).await) {
                match result {
                    Ok(completions) => {
                        answered = true;
                        answer.incomplete |= completions.incomplete;
                        answer.items.extend(
                            completions
                                .items
                                .into_iter()
                                .map(|item| wrap_item(provider, item)),
                        );
                    }
                    Err(IntelError::Unsupported) => {}
                    Err(e) => error = error.or(Some(e)),
                }
            }
            if answered {
                Ok(answer)
            } else {
                Err(error.unwrap_or(IntelError::Unsupported))
            }
        })
    }

    fn resolve_completion(
        &self,
        doc: &DocumentId,
        mut item: CompletionItem,
    ) -> Intel<CompletionItem> {
        let routed = item.token.as_ref().and_then(unwrap);
        let provider = match routed {
            Some((provider, inner)) => {
                item.token = inner;
                provider
            }
            None => match self.providers(doc).first() {
                Some(&provider) => provider,
                None => return unsupported(),
            },
        };
        let Some(route) = self.routes.get(provider) else {
            return unsupported();
        };
        let request = route.services.resolve_completion(doc, item);
        Box::pin(async move { Ok(wrap_item(provider, request.await?)) })
    }

    fn completion_triggers(&self, doc: &DocumentId) -> Vec<char> {
        let mut triggers = Vec::new();
        for (_, services) in self.services(doc) {
            for c in services.completion_triggers(doc) {
                if !triggers.contains(&c) {
                    triggers.push(c);
                }
            }
        }
        triggers
    }

    fn hover(&self, doc: &DocumentId, at: Point) -> Intel<Option<Hover>> {
        first(self.each(doc, |s| s.hover(doc, at)))
    }

    fn signature_help(
        &self,
        doc: &DocumentId,
        at: Point,
        context: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        first(self.each(doc, |s| s.signature_help(doc, at, context.clone())))
    }

    fn definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>> {
        first(self.each(doc, |s| s.definition(doc, at)))
    }

    fn definition_targets(&self, doc: &DocumentId, at: Point) -> Intel<Vec<DefinitionTarget>> {
        first(self.each(doc, |s| s.definition_targets(doc, at)))
    }

    fn type_definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>> {
        first(self.each(doc, |s| s.type_definition(doc, at)))
    }

    fn references(
        &self,
        doc: &DocumentId,
        at: Point,
        include_declaration: bool,
    ) -> Intel<Vec<Location>> {
        joined(self.each(doc, |s| s.references(doc, at, include_declaration)))
    }

    fn code_actions(
        &self,
        doc: &DocumentId,
        range: Range<Point>,
        context: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        let requests: Vec<Intel<Vec<CodeAction>>> = self
            .services(doc)
            .into_iter()
            .map(|(provider, s)| {
                let request = s.code_actions(doc, range.clone(), context.clone());
                Box::pin(async move {
                    let actions = request.await?;
                    Ok(actions
                        .into_iter()
                        .map(|action| wrap_action(provider, action))
                        .collect())
                }) as Intel<Vec<CodeAction>>
            })
            .collect();
        joined(requests)
    }

    fn resolve_code_action(&self, doc: &DocumentId, mut action: CodeAction) -> Intel<CodeAction> {
        let provider = match unwrap_action(&mut action) {
            Some(provider) => provider,
            None => match self.providers(doc).first() {
                Some(&provider) => provider,
                None => return unsupported(),
            },
        };
        let Some(route) = self.routes.get(provider) else {
            return unsupported();
        };
        let request = route.services.resolve_code_action(doc, action);
        Box::pin(async move { Ok(wrap_action(provider, request.await?)) })
    }

    fn format(
        &self,
        doc: &DocumentId,
        range: Option<Range<Point>>,
        options: FormatOptions,
    ) -> Intel<Vec<TextEdit>> {
        first(self.each(doc, |s| s.format(doc, range.clone(), options)))
    }

    fn execute_command(&self, mut command: Command) -> Intel<()> {
        if let Some((provider, Some(inner))) = unwrap(&command.token) {
            command.token = inner;
            return match self.routes.get(provider) {
                Some(route) => route.services.execute_command(command),
                None => unsupported(),
            };
        }
        // Not one of ours: the first provider that knows it runs it, and
        // only that one.
        let services: Vec<Arc<dyn CodeIntel>> =
            self.routes.iter().map(|r| r.services.clone()).collect();
        Box::pin(async move {
            for s in services {
                match s.execute_command(command.clone()).await {
                    Err(IntelError::Unsupported) => {}
                    result => return result,
                }
            }
            Err(IntelError::Unsupported)
        })
    }

    fn rename(&self, doc: &DocumentId, at: Point, new_name: String) -> Intel<WorkspaceEdit> {
        first(self.each(doc, |s| s.rename(doc, at, new_name.clone())))
    }

    fn document_highlights(&self, doc: &DocumentId, at: Point) -> Intel<Vec<DocumentHighlight>> {
        first(self.each(doc, |s| s.document_highlights(doc, at)))
    }

    fn document_symbols(&self, doc: &DocumentId) -> Intel<Vec<DocumentSymbol>> {
        joined(self.each(doc, |s| s.document_symbols(doc)))
    }

    fn status(&self) -> Vec<ServerStatus> {
        self.routes
            .iter()
            .flat_map(|r| r.services.status())
            .collect()
    }

    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.shared.hub.subscribe()
    }

    fn restart(&self, server: Option<&ServerId>) {
        for route in &self.routes {
            route.services.restart(server);
        }
    }

    fn stop(&self, server: &ServerId) {
        for route in &self.routes {
            route.services.stop(server);
        }
    }

    fn shutdown(&self) -> Intel<()> {
        let requests: Vec<Intel<()>> = self.routes.iter().map(|r| r.services.shutdown()).collect();
        Box::pin(async move {
            let mut error = None;
            for result in join_all(requests).await {
                if let Err(e) = result {
                    error = error.or(Some(e));
                }
            }
            error.map_or(Ok(()), Err)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::executor::block_on;
    use text::{Buffer, CaretPlacement, Edit, EditKind, SelectionSet};

    use super::*;
    use crate::fake::{Call, FakeCodeIntel, RequestKind};
    use crate::{CodeActionKind, Markup, Severity};

    fn sql_doc() -> DocumentId {
        DocumentId::new("/p/q.sql")
    }

    fn py_doc() -> DocumentId {
        DocumentId::new("/p/a.py")
    }

    /// A SQL provider and a Python provider, each for its language only.
    fn sql_and_python() -> (Arc<FakeCodeIntel>, Arc<FakeCodeIntel>, RoutedCodeIntel) {
        let sql = FakeCodeIntel::new();
        let python = FakeCodeIntel::new();
        let routed = RoutedCodeIntel::new(vec![
            Route::new(&["sql"], sql.clone()),
            Route::new(&["python"], python.clone()),
        ]);
        (sql, python, routed)
    }

    /// A SQL provider and a provider of every language (as a product's).
    fn sql_and_any() -> (Arc<FakeCodeIntel>, Arc<FakeCodeIntel>, RoutedCodeIntel) {
        let sql = FakeCodeIntel::new();
        let python = FakeCodeIntel::new();
        let routed = RoutedCodeIntel::new(vec![
            Route::new(&["sql"], sql.clone()),
            Route::new(&[], python.clone()),
        ]);
        (sql, python, routed)
    }

    fn opened(fake: &FakeCodeIntel) -> Vec<DocumentId> {
        fake.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::DidOpen { doc, .. } => Some(doc),
                _ => None,
            })
            .collect()
    }

    fn next(events: &flume::Receiver<IntelEvent>) -> IntelEvent {
        events
            .recv_timeout(Duration::from_secs(10))
            .expect("an event")
    }

    fn hover(text: &str) -> Hover {
        Hover {
            contents: Markup::plain(text),
            range: None,
        }
    }

    fn place(path: &str) -> Location {
        Location {
            path: path.into(),
            range: Point::new(0, 1)..Point::new(0, 1),
        }
    }

    #[test]
    fn documents_reach_the_providers_of_their_language() {
        let (sql, python, routed) = sql_and_python();
        let text = Rope::from_str("x\n");
        let other = DocumentId::new("/p/n.txt");
        routed.did_open(&sql_doc(), "sql", 1, &text);
        routed.did_open(&py_doc(), "python", 1, &text);
        routed.did_open(&other, "text", 1, &text);
        assert_eq!(opened(&sql), [sql_doc()]);
        assert_eq!(opened(&python), [py_doc()]);
        routed.did_change(&py_doc(), 2, &[], &text);
        routed.did_save(&py_doc(), &text);
        routed.did_close(&sql_doc());
        assert!(sql.documents().is_empty());
        assert_eq!(python.version(&py_doc()), Some(2));
        assert!(
            !sql.calls()
                .iter()
                .any(|c| matches!(c, Call::DidChange { .. } | Call::DidSave { .. }))
        );
        // Requests too.
        sql.on_hover(|_, _| Ok(Some(hover("sql"))));
        python.on_hover(|_, _| Ok(Some(hover("python"))));
        let at = Point::default();
        assert_eq!(
            block_on(routed.hover(&py_doc(), at)),
            Ok(Some(hover("python")))
        );
        assert_eq!(sql.requests(RequestKind::Hover), 0);
        // A document no provider serves has no services.
        assert_eq!(
            block_on(routed.hover(&other, at)),
            Err(IntelError::Unsupported)
        );
        // Project-wide calls reach every provider.
        routed.files_changed(&[]);
        routed.restart(None);
        routed.stop(&ServerId::new("ty"));
        for fake in [&sql, &python] {
            let calls = fake.calls();
            assert!(calls.contains(&Call::FilesChanged(Vec::new())));
            assert!(calls.contains(&Call::Restart(None)));
            assert!(calls.contains(&Call::Stop(ServerId::new("ty"))));
        }
        assert_eq!(block_on(routed.shutdown()), Ok(()));
        assert_eq!(sql.requests(RequestKind::Shutdown), 1);
        assert_eq!(python.requests(RequestKind::Shutdown), 1);
    }

    #[test]
    fn list_answers_are_joined_and_items_resolve_in_their_provider() {
        let (sql, python, routed) = sql_and_any();
        let doc = sql_doc();
        routed.did_open(&doc, "sql", 1, &Rope::from_str("select 1\n"));
        routed.did_open(&py_doc(), "python", 1, &Rope::from_str("a = 1\n"));
        assert_eq!(opened(&sql), std::slice::from_ref(&doc));
        assert_eq!(opened(&python), [doc.clone(), py_doc()]);

        sql.on_completion(|_, _, _| {
            Ok(Completions {
                items: vec![CompletionItem::new("select")],
                incomplete: false,
            })
        });
        python.on_completion(|_, _, _| {
            let mut item = CompletionItem::new("self");
            item.token = Some(Token::new(7u32));
            Ok(Completions {
                items: vec![item],
                incomplete: true,
            })
        });
        let at = Point::new(0, 1);
        let invoked = CompletionTrigger::Invoked;
        let completions = block_on(routed.completion(&doc, at, invoked)).unwrap();
        let labels: Vec<&str> = completions.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["select", "self"]);
        assert!(completions.incomplete);
        let python_only = block_on(routed.completion(&py_doc(), at, invoked)).unwrap();
        assert_eq!(python_only.items.len(), 1);

        // An item is resolved by its provider, with the provider's token.
        python.on_resolve_completion(|_, mut item| {
            let token = item.token.as_ref().and_then(|t| t.downcast_ref::<u32>());
            assert_eq!(token, Some(&7));
            item.detail = Some("resolved".into());
            Ok(item)
        });
        let resolved = block_on(routed.resolve_completion(&doc, completions.items[1].clone()));
        assert_eq!(resolved.unwrap().detail.as_deref(), Some("resolved"));
        assert_eq!(sql.requests(RequestKind::ResolveCompletion), 0);

        // A code action's command runs in its provider.
        sql.on_code_actions(|_, _, _| {
            let mut action = CodeAction::new("Qualify", CodeActionKind::QuickFix);
            action.command = Some(Command {
                title: "Qualify".into(),
                token: Token::new("qualify"),
            });
            Ok(vec![action])
        });
        python.on_code_actions(|_, _, _| {
            Ok(vec![CodeAction::new("Ignore", CodeActionKind::QuickFix)])
        });
        let context = CodeActionContext::default();
        let actions = block_on(routed.code_actions(&doc, at..at, context)).unwrap();
        let titles: Vec<&str> = actions.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Qualify", "Ignore"]);
        sql.on_execute_command(|command| {
            assert_eq!(command.token.downcast_ref::<&str>(), Some(&"qualify"));
            Ok(())
        });
        let command = actions[0].command.clone().unwrap();
        assert_eq!(block_on(routed.execute_command(command)), Ok(()));
        assert_eq!(python.requests(RequestKind::ExecuteCommand), 0);

        // References: joined; one provider without them does not matter.
        python.on_references(|_, _, _| Ok(vec![place("/p/b.py")]));
        assert_eq!(
            block_on(routed.references(&doc, at, true)),
            Ok(vec![place("/p/b.py")])
        );

        // Status lists both providers' servers.
        let mut sql_server = ServerStatus::new(ServerId::new("sql"), "SQL");
        sql_server.documents.push(doc.clone());
        sql.set_status(vec![sql_server]);
        python.set_status(vec![ServerStatus::new(ServerId::new("ty"), "ty")]);
        let ids: Vec<ServerId> = routed.status().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, [ServerId::new("sql"), ServerId::new("ty")]);
        assert_eq!(routed.servers_for(&doc), [ServerId::new("sql")]);
    }

    #[test]
    fn single_answers_come_from_the_first_provider_with_one() {
        let (sql, python, routed) = sql_and_any();
        let doc = sql_doc();
        routed.did_open(&doc, "sql", 1, &Rope::from_str("select 1\n"));
        let at = Point::new(0, 1);

        sql.on_hover(|_, _| Ok(None));
        python.on_hover(|_, _| Ok(Some(hover("from python"))));
        assert_eq!(
            block_on(routed.hover(&doc, at)),
            Ok(Some(hover("from python")))
        );
        sql.on_hover(|_, _| Ok(Some(hover("from sql"))));
        assert_eq!(
            block_on(routed.hover(&doc, at)),
            Ok(Some(hover("from sql")))
        );

        // The first provider does not support it, the second answers.
        python.on_definition(|_, _| Ok(vec![place("/p/b.py")]));
        assert_eq!(
            block_on(routed.definition(&doc, at)),
            Ok(vec![place("/p/b.py")])
        );
        assert_eq!(
            block_on(routed.definition_targets(&doc, at)),
            Ok(vec![DefinitionTarget::File(place("/p/b.py"))])
        );
        // An empty answer goes on to the next.
        sql.on_definition(|_, _| Ok(Vec::new()));
        assert_eq!(
            block_on(routed.definition(&doc, at)),
            Ok(vec![place("/p/b.py")])
        );
        // Nobody: unsupported.
        assert_eq!(
            block_on(routed.format(&doc, None, FormatOptions::default())),
            Err(IntelError::Unsupported)
        );
        // Nothing from anybody: nothing.
        python.on_definition(|_, _| Ok(Vec::new()));
        assert_eq!(block_on(routed.definition(&doc, at)), Ok(Vec::new()));
        // Structure: joined.
        let symbol = |name: &str| DocumentSymbol::new(name, crate::SymbolKind::Class, at..at);
        let (a, b) = (symbol("a"), symbol("b"));
        sql.on_document_symbols(move |_| Ok(vec![a.clone()]));
        python.on_document_symbols(move |_| Ok(vec![b.clone()]));
        assert_eq!(
            block_on(routed.document_symbols(&doc)),
            Ok(vec![symbol("a"), symbol("b")])
        );
    }

    #[test]
    fn diagnostics_of_a_shared_document_are_joined_in_the_current_text() {
        let (sql, python, routed) = sql_and_any();
        let events = routed.events();
        let doc = sql_doc();
        let mut buffer = Buffer::new("select 1\n");
        routed.did_open(&doc, "sql", 1, buffer.rope());
        let problem =
            |range: Range<Point>, message: &str| Diagnostic::new(range, Severity::Warning, message);
        let one = problem(Point::new(0, 7)..Point::new(0, 8), "one");
        sql.publish_diagnostics(&doc, 1, vec![one.clone()]);
        assert_eq!(
            next(&events),
            IntelEvent::Diagnostics {
                doc: doc.clone(),
                version: 1,
                items: vec![one],
            }
        );

        // Two characters typed in front: the first provider's item moves
        // with the text when the second one reports.
        let mut selections = SelectionSet::caret(0);
        let applied = buffer.transact(
            &mut selections,
            vec![Edit::insert(0, "  ")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        routed.did_change(&doc, 2, &applied, buffer.rope());
        let two = problem(Point::new(0, 2)..Point::new(0, 8), "two");
        python.publish_diagnostics(&doc, 2, vec![two.clone()]);
        assert_eq!(
            next(&events),
            IntelEvent::Diagnostics {
                doc: doc.clone(),
                version: 2,
                items: vec![problem(Point::new(0, 9)..Point::new(0, 10), "one"), two],
            }
        );

        // A document of one provider: its report as it is.
        routed.did_open(&py_doc(), "python", 4, &Rope::from_str("a\n"));
        let three = problem(Point::new(0, 0)..Point::new(0, 1), "three");
        python.publish_diagnostics(&py_doc(), 4, vec![three.clone()]);
        assert_eq!(
            next(&events),
            IntelEvent::Diagnostics {
                doc: py_doc(),
                version: 4,
                items: vec![three],
            }
        );

        // Other events pass through.
        let status = ServerStatus::new(ServerId::new("sql"), "SQL");
        sql.set_status(vec![status.clone()]);
        assert_eq!(next(&events), IntelEvent::Status(status));

        // After a reload the older items are stale.
        routed.did_reload(&doc, 3, &Rope::from_str("select 2\n"));
        sql.publish_diagnostics(&doc, 3, Vec::new());
        assert_eq!(
            next(&events),
            IntelEvent::Diagnostics {
                doc,
                version: 3,
                items: Vec::new(),
            }
        );
    }

    #[test]
    fn the_events_thread_ends_with_the_router() {
        let fake = FakeCodeIntel::new();
        let routed = RoutedCodeIntel::new(vec![Route::new(&[], fake.clone())]);
        let events = routed.events();
        drop(routed);
        // The hub went with the router: its receivers are disconnected.
        fake.emit(IntelEvent::Status(ServerStatus::new(
            ServerId::new("a"),
            "a",
        )));
        assert!(events.recv_timeout(Duration::from_secs(10)).is_err());
    }
}
