//! An in-memory [`CodeIntel`] for tests of the code that uses the trait
//! (the editor UI, the workbench). Enabled by the `fake` feature.
//!
//! - Every call is recorded ([`FakeCodeIntel::calls`]).
//! - Documents are mirrored: the fake applies the reported edits to its own
//!   copy of each document's text ([`FakeCodeIntel::text`]), as a server
//!   would, so a test can check that the edits reach it intact.
//! - Answers are scripted per request kind (`on_completion`, `on_hover`,
//!   ...); a kind without a script answers [`IntelError::Unsupported`]
//!   (shutdown answers `Ok`).
//! - Answers come at once, or, for a kind that is held
//!   ([`FakeCodeIntel::hold`]), when the test releases them; a held request
//!   whose future was dropped counts as dropped (cancelled).
//! - Events are sent on demand ([`FakeCodeIntel::emit`]).
//!
//! ```ignore
//! let fake = FakeCodeIntel::new();
//! fake.on_completion(|_, _, _| Ok(Completions { items: vec![CompletionItem::new("norm")], incomplete: false }));
//! fake.hold(RequestKind::Completion);
//! let intel: Arc<dyn CodeIntel> = fake.clone();
//! // ... the UI asks; the test checks the popup is not shown yet ...
//! fake.release(RequestKind::Completion);
//! ```

use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::channel::oneshot;
use text::{AppliedEdit, Point, Rope};

use crate::{
    CodeAction, CodeActionContext, CodeIntel, Command, CompletionItem, CompletionTrigger,
    Completions, Diagnostic, DocumentHighlight, DocumentId, EventHub, FileChange, FormatOptions,
    Hover, Intel, IntelError, IntelEvent, Location, ServerId, ServerStatus, SignatureContext,
    SignatureHelp, TextEdit, WorkspaceEdit, ready,
};

/// The kinds of requests, for scripting and holding answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Completion,
    ResolveCompletion,
    Hover,
    SignatureHelp,
    Definition,
    TypeDefinition,
    References,
    CodeActions,
    ResolveCodeAction,
    Format,
    ExecuteCommand,
    Rename,
    DocumentHighlights,
    Shutdown,
}

/// A call the fake received.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    DidOpen {
        doc: DocumentId,
        language: String,
        version: u64,
        text: String,
    },
    DidChange {
        doc: DocumentId,
        version: u64,
        edits: Vec<AppliedEdit>,
    },
    DidReload {
        doc: DocumentId,
        version: u64,
        text: String,
    },
    DidSave {
        doc: DocumentId,
    },
    DidClose {
        doc: DocumentId,
    },
    FilesChanged(Vec<FileChange>),
    Completion {
        doc: DocumentId,
        at: Point,
        trigger: CompletionTrigger,
    },
    ResolveCompletion {
        doc: DocumentId,
        item: CompletionItem,
    },
    Hover {
        doc: DocumentId,
        at: Point,
    },
    SignatureHelp {
        doc: DocumentId,
        at: Point,
        context: SignatureContext,
    },
    Definition {
        doc: DocumentId,
        at: Point,
    },
    TypeDefinition {
        doc: DocumentId,
        at: Point,
    },
    References {
        doc: DocumentId,
        at: Point,
        include_declaration: bool,
    },
    CodeActions {
        doc: DocumentId,
        range: Range<Point>,
        context: CodeActionContext,
    },
    ResolveCodeAction {
        doc: DocumentId,
        action: CodeAction,
    },
    Format {
        doc: DocumentId,
        range: Option<Range<Point>>,
        options: FormatOptions,
    },
    ExecuteCommand(Command),
    Rename {
        doc: DocumentId,
        at: Point,
        new_name: String,
    },
    DocumentHighlights {
        doc: DocumentId,
        at: Point,
    },
    Restart(Option<ServerId>),
    Stop(ServerId),
    Shutdown,
}

impl Call {
    /// The kind of a request call; `None` for lifecycle and server calls.
    pub fn request_kind(&self) -> Option<RequestKind> {
        Some(match self {
            Call::Completion { .. } => RequestKind::Completion,
            Call::ResolveCompletion { .. } => RequestKind::ResolveCompletion,
            Call::Hover { .. } => RequestKind::Hover,
            Call::SignatureHelp { .. } => RequestKind::SignatureHelp,
            Call::Definition { .. } => RequestKind::Definition,
            Call::TypeDefinition { .. } => RequestKind::TypeDefinition,
            Call::References { .. } => RequestKind::References,
            Call::CodeActions { .. } => RequestKind::CodeActions,
            Call::ResolveCodeAction { .. } => RequestKind::ResolveCodeAction,
            Call::Format { .. } => RequestKind::Format,
            Call::ExecuteCommand(_) => RequestKind::ExecuteCommand,
            Call::Rename { .. } => RequestKind::Rename,
            Call::DocumentHighlights { .. } => RequestKind::DocumentHighlights,
            Call::Shutdown => RequestKind::Shutdown,
            _ => return None,
        })
    }
}

/// A scripted answer: computed from the call when the request is made.
type Answer<T> = Arc<dyn Fn(&Call) -> Result<T, IntelError> + Send + Sync>;

/// A held answer waiting for the test to release it.
trait Pending: Send {
    fn kind(&self) -> RequestKind;
    /// Sends the answer; false when the request's future was dropped.
    fn deliver(self: Box<Self>) -> bool;
    fn is_dropped(&self) -> bool;
}

struct Reply<T> {
    kind: RequestKind,
    tx: oneshot::Sender<Result<T, IntelError>>,
    result: Result<T, IntelError>,
}

impl<T: Send> Pending for Reply<T> {
    fn kind(&self) -> RequestKind {
        self.kind
    }

    fn deliver(self: Box<Self>) -> bool {
        self.tx.send(self.result).is_ok()
    }

    fn is_dropped(&self) -> bool {
        self.tx.is_canceled()
    }
}

/// A document as the fake sees it.
struct Document {
    language: String,
    version: u64,
    text: Rope,
    /// The edits fit the mirrored text and left the text the caller
    /// reported.
    in_sync: bool,
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    documents: BTreeMap<DocumentId, Document>,
    answers: HashMap<RequestKind, Box<dyn Any + Send + Sync>>,
    held: HashSet<RequestKind>,
    pending: Vec<Box<dyn Pending>>,
    status: Vec<ServerStatus>,
    completion_triggers: Vec<char>,
}

/// An in-memory [`CodeIntel`] (see the module documentation).
#[derive(Default)]
pub struct FakeCodeIntel {
    state: Mutex<State>,
    events: EventHub,
}

impl FakeCodeIntel {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, call: Call) {
        self.lock().calls.push(call);
    }

    // ---- calls ---------------------------------------------------------------

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    /// The calls so far; the list starts again empty.
    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut self.lock().calls)
    }

    /// How many requests of a kind were made.
    pub fn requests(&self, kind: RequestKind) -> usize {
        self.lock()
            .calls
            .iter()
            .filter(|c| c.request_kind() == Some(kind))
            .count()
    }

    // ---- documents -----------------------------------------------------------

    /// The open documents.
    pub fn documents(&self) -> Vec<DocumentId> {
        self.lock().documents.keys().cloned().collect()
    }

    /// The text of an open document, as the reported edits left it.
    pub fn text(&self, doc: &DocumentId) -> Option<String> {
        self.lock().documents.get(doc).map(|d| d.text.to_string())
    }

    /// The last version reported for an open document.
    pub fn version(&self, doc: &DocumentId) -> Option<u64> {
        self.lock().documents.get(doc).map(|d| d.version)
    }

    /// The language an open document was opened with.
    pub fn language(&self, doc: &DocumentId) -> Option<String> {
        self.lock().documents.get(doc).map(|d| d.language.clone())
    }

    /// Every edit fit the mirrored text and left the text that was
    /// reported with it; false for a document that is not open.
    pub fn in_sync(&self, doc: &DocumentId) -> bool {
        self.lock().documents.get(doc).is_some_and(|d| d.in_sync)
    }

    // ---- answers -------------------------------------------------------------

    fn set_answer<T: Send + 'static>(&self, kind: RequestKind, answer: Answer<T>) {
        self.lock().answers.insert(kind, Box::new(answer));
    }

    /// Records the call and answers it with the script of its kind.
    fn request<T: Send + 'static>(&self, call: Call, default: Result<T, IntelError>) -> Intel<T> {
        let kind = call.request_kind().expect("a request");
        let answer = {
            let mut state = self.lock();
            state.calls.push(call.clone());
            state
                .answers
                .get(&kind)
                .and_then(|a| a.downcast_ref::<Answer<T>>())
                .cloned()
        };
        // The script runs without the lock: it may use the fake.
        let result = match answer {
            Some(answer) => answer(&call),
            None => default,
        };
        let mut state = self.lock();
        if !state.held.contains(&kind) {
            return ready(result);
        }
        let (tx, rx) = oneshot::channel();
        state.pending.push(Box::new(Reply { kind, tx, result }));
        Box::pin(async move { rx.await.unwrap_or(Err(IntelError::Cancelled)) })
    }

    pub fn on_completion(
        &self,
        f: impl Fn(&DocumentId, Point, CompletionTrigger) -> Result<Completions, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<Completions>(
            RequestKind::Completion,
            Arc::new(move |call| match call {
                Call::Completion { doc, at, trigger } => f(doc, *at, *trigger),
                _ => Err(IntelError::Other("not a completion".into())),
            }),
        );
    }

    pub fn on_resolve_completion(
        &self,
        f: impl Fn(&DocumentId, CompletionItem) -> Result<CompletionItem, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<CompletionItem>(
            RequestKind::ResolveCompletion,
            Arc::new(move |call| match call {
                Call::ResolveCompletion { doc, item } => f(doc, item.clone()),
                _ => Err(IntelError::Other("not a completion resolve".into())),
            }),
        );
    }

    pub fn on_hover(
        &self,
        f: impl Fn(&DocumentId, Point) -> Result<Option<Hover>, IntelError> + Send + Sync + 'static,
    ) {
        self.set_answer::<Option<Hover>>(
            RequestKind::Hover,
            Arc::new(move |call| match call {
                Call::Hover { doc, at } => f(doc, *at),
                _ => Err(IntelError::Other("not a hover".into())),
            }),
        );
    }

    pub fn on_signature_help(
        &self,
        f: impl Fn(&DocumentId, Point, &SignatureContext) -> Result<Option<SignatureHelp>, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<Option<SignatureHelp>>(
            RequestKind::SignatureHelp,
            Arc::new(move |call| match call {
                Call::SignatureHelp { doc, at, context } => f(doc, *at, context),
                _ => Err(IntelError::Other("not a signature help".into())),
            }),
        );
    }

    pub fn on_definition(
        &self,
        f: impl Fn(&DocumentId, Point) -> Result<Vec<Location>, IntelError> + Send + Sync + 'static,
    ) {
        self.set_answer::<Vec<Location>>(
            RequestKind::Definition,
            Arc::new(move |call| match call {
                Call::Definition { doc, at } => f(doc, *at),
                _ => Err(IntelError::Other("not a definition".into())),
            }),
        );
    }

    pub fn on_type_definition(
        &self,
        f: impl Fn(&DocumentId, Point) -> Result<Vec<Location>, IntelError> + Send + Sync + 'static,
    ) {
        self.set_answer::<Vec<Location>>(
            RequestKind::TypeDefinition,
            Arc::new(move |call| match call {
                Call::TypeDefinition { doc, at } => f(doc, *at),
                _ => Err(IntelError::Other("not a type definition".into())),
            }),
        );
    }

    pub fn on_references(
        &self,
        f: impl Fn(&DocumentId, Point, bool) -> Result<Vec<Location>, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<Vec<Location>>(
            RequestKind::References,
            Arc::new(move |call| match call {
                Call::References {
                    doc,
                    at,
                    include_declaration,
                } => f(doc, *at, *include_declaration),
                _ => Err(IntelError::Other("not a references request".into())),
            }),
        );
    }

    pub fn on_code_actions(
        &self,
        f: impl Fn(&DocumentId, Range<Point>, &CodeActionContext) -> Result<Vec<CodeAction>, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<Vec<CodeAction>>(
            RequestKind::CodeActions,
            Arc::new(move |call| match call {
                Call::CodeActions {
                    doc,
                    range,
                    context,
                } => f(doc, range.clone(), context),
                _ => Err(IntelError::Other("not a code action request".into())),
            }),
        );
    }

    pub fn on_resolve_code_action(
        &self,
        f: impl Fn(&DocumentId, CodeAction) -> Result<CodeAction, IntelError> + Send + Sync + 'static,
    ) {
        self.set_answer::<CodeAction>(
            RequestKind::ResolveCodeAction,
            Arc::new(move |call| match call {
                Call::ResolveCodeAction { doc, action } => f(doc, action.clone()),
                _ => Err(IntelError::Other("not a code action resolve".into())),
            }),
        );
    }

    pub fn on_format(
        &self,
        f: impl Fn(
            &DocumentId,
            Option<Range<Point>>,
            FormatOptions,
        ) -> Result<Vec<TextEdit>, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<Vec<TextEdit>>(
            RequestKind::Format,
            Arc::new(move |call| match call {
                Call::Format {
                    doc,
                    range,
                    options,
                } => f(doc, range.clone(), *options),
                _ => Err(IntelError::Other("not a format request".into())),
            }),
        );
    }

    pub fn on_execute_command(
        &self,
        f: impl Fn(&Command) -> Result<(), IntelError> + Send + Sync + 'static,
    ) {
        self.set_answer::<()>(
            RequestKind::ExecuteCommand,
            Arc::new(move |call| match call {
                Call::ExecuteCommand(command) => f(command),
                _ => Err(IntelError::Other("not a command".into())),
            }),
        );
    }

    pub fn on_rename(
        &self,
        f: impl Fn(&DocumentId, Point, &str) -> Result<WorkspaceEdit, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<WorkspaceEdit>(
            RequestKind::Rename,
            Arc::new(move |call| match call {
                Call::Rename { doc, at, new_name } => f(doc, *at, new_name),
                _ => Err(IntelError::Other("not a rename".into())),
            }),
        );
    }

    pub fn on_document_highlights(
        &self,
        f: impl Fn(&DocumentId, Point) -> Result<Vec<DocumentHighlight>, IntelError>
        + Send
        + Sync
        + 'static,
    ) {
        self.set_answer::<Vec<DocumentHighlight>>(
            RequestKind::DocumentHighlights,
            Arc::new(move |call| match call {
                Call::DocumentHighlights { doc, at } => f(doc, *at),
                _ => Err(IntelError::Other("not a highlights request".into())),
            }),
        );
    }

    // ---- holding answers ------------------------------------------------------

    /// Answers of this kind wait for [`release`](Self::release) from now on.
    pub fn hold(&self, kind: RequestKind) {
        self.lock().held.insert(kind);
    }

    /// Answers of this kind come at once again (held ones stay held).
    pub fn answer_at_once(&self, kind: RequestKind) {
        self.lock().held.remove(&kind);
    }

    /// Sends the held answers of a kind, oldest first; returns how many
    /// reached a request that was still waiting.
    pub fn release(&self, kind: RequestKind) -> usize {
        let taken: Vec<Box<dyn Pending>> = {
            let mut state = self.lock();
            let (taken, kept) = std::mem::take(&mut state.pending)
                .into_iter()
                .partition(|p| p.kind() == kind);
            state.pending = kept;
            taken
        };
        taken
            .into_iter()
            .map(|p| p.deliver())
            .filter(|delivered| *delivered)
            .count()
    }

    /// Sends the oldest held answer of a kind; false when there was none or
    /// its request was dropped.
    pub fn release_next(&self, kind: RequestKind) -> bool {
        let next = {
            let mut state = self.lock();
            let i = state.pending.iter().position(|p| p.kind() == kind);
            i.map(|i| state.pending.remove(i))
        };
        next.is_some_and(|p| p.deliver())
    }

    /// Held requests of a kind that still wait for their answer.
    pub fn pending(&self, kind: RequestKind) -> usize {
        self.lock()
            .pending
            .iter()
            .filter(|p| p.kind() == kind && !p.is_dropped())
            .count()
    }

    /// Held requests of a kind whose future was dropped (cancelled) before
    /// the answer was released.
    pub fn dropped(&self, kind: RequestKind) -> usize {
        self.lock()
            .pending
            .iter()
            .filter(|p| p.kind() == kind && p.is_dropped())
            .count()
    }

    // ---- events and servers ---------------------------------------------------

    /// Sends an event to every receiver.
    pub fn emit(&self, event: IntelEvent) {
        self.events.emit(event);
    }

    /// Sends the diagnostics of a document.
    pub fn publish_diagnostics(&self, doc: &DocumentId, version: u64, items: Vec<Diagnostic>) {
        self.emit(IntelEvent::Diagnostics {
            doc: doc.clone(),
            version,
            items,
        });
    }

    /// The characters every document reports as completion triggers.
    pub fn set_completion_triggers(&self, triggers: &[char]) {
        self.lock().completion_triggers = triggers.to_vec();
    }

    /// Replaces the servers' states and sends one [`IntelEvent::Status`]
    /// per server.
    pub fn set_status(&self, status: Vec<ServerStatus>) {
        self.lock().status = status.clone();
        for s in status {
            self.emit(IntelEvent::Status(s));
        }
    }
}

/// Applies an edit to a mirrored text; false when it does not fit.
fn apply(text: &mut Rope, edit: &AppliedEdit) -> bool {
    let len = text.len_chars();
    if edit.start > edit.old_end || edit.old_end > len {
        return false;
    }
    text.remove(edit.start..edit.old_end);
    text.insert(edit.start, &edit.text);
    true
}

impl CodeIntel for FakeCodeIntel {
    fn did_open(&self, doc: &DocumentId, language: &str, version: u64, text: &Rope) {
        let mut state = self.lock();
        state.calls.push(Call::DidOpen {
            doc: doc.clone(),
            language: language.to_owned(),
            version,
            text: text.to_string(),
        });
        state.documents.insert(
            doc.clone(),
            Document {
                language: language.to_owned(),
                version,
                text: text.clone(),
                in_sync: true,
            },
        );
    }

    fn did_change(&self, doc: &DocumentId, version: u64, edits: &[AppliedEdit], text: &Rope) {
        let mut state = self.lock();
        state.calls.push(Call::DidChange {
            doc: doc.clone(),
            version,
            edits: edits.to_vec(),
        });
        if let Some(d) = state.documents.get_mut(doc) {
            for edit in edits {
                d.in_sync &= apply(&mut d.text, edit);
            }
            d.in_sync &= d.version < version && d.text == *text;
            d.version = version;
        }
    }

    fn did_reload(&self, doc: &DocumentId, version: u64, text: &Rope) {
        let mut state = self.lock();
        state.calls.push(Call::DidReload {
            doc: doc.clone(),
            version,
            text: text.to_string(),
        });
        if let Some(d) = state.documents.get_mut(doc) {
            d.in_sync &= d.version < version;
            d.text = text.clone();
            d.version = version;
        }
    }

    fn did_save(&self, doc: &DocumentId, _: &Rope) {
        self.record(Call::DidSave { doc: doc.clone() });
    }

    fn did_close(&self, doc: &DocumentId) {
        let mut state = self.lock();
        state.calls.push(Call::DidClose { doc: doc.clone() });
        state.documents.remove(doc);
    }

    fn files_changed(&self, changes: &[FileChange]) {
        self.record(Call::FilesChanged(changes.to_vec()));
    }

    fn completion(
        &self,
        doc: &DocumentId,
        at: Point,
        trigger: CompletionTrigger,
    ) -> Intel<Completions> {
        self.request(
            Call::Completion {
                doc: doc.clone(),
                at,
                trigger,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn completion_triggers(&self, doc: &DocumentId) -> Vec<char> {
        let state = self.lock();
        if state.documents.contains_key(doc) {
            state.completion_triggers.clone()
        } else {
            Vec::new()
        }
    }

    fn resolve_completion(&self, doc: &DocumentId, item: CompletionItem) -> Intel<CompletionItem> {
        self.request(
            Call::ResolveCompletion {
                doc: doc.clone(),
                item,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn hover(&self, doc: &DocumentId, at: Point) -> Intel<Option<Hover>> {
        self.request(
            Call::Hover {
                doc: doc.clone(),
                at,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn signature_help(
        &self,
        doc: &DocumentId,
        at: Point,
        context: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        self.request(
            Call::SignatureHelp {
                doc: doc.clone(),
                at,
                context,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>> {
        self.request(
            Call::Definition {
                doc: doc.clone(),
                at,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn type_definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>> {
        self.request(
            Call::TypeDefinition {
                doc: doc.clone(),
                at,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn references(
        &self,
        doc: &DocumentId,
        at: Point,
        include_declaration: bool,
    ) -> Intel<Vec<Location>> {
        self.request(
            Call::References {
                doc: doc.clone(),
                at,
                include_declaration,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn code_actions(
        &self,
        doc: &DocumentId,
        range: Range<Point>,
        context: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        self.request(
            Call::CodeActions {
                doc: doc.clone(),
                range,
                context,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn resolve_code_action(&self, doc: &DocumentId, action: CodeAction) -> Intel<CodeAction> {
        self.request(
            Call::ResolveCodeAction {
                doc: doc.clone(),
                action,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn format(
        &self,
        doc: &DocumentId,
        range: Option<Range<Point>>,
        options: FormatOptions,
    ) -> Intel<Vec<TextEdit>> {
        self.request(
            Call::Format {
                doc: doc.clone(),
                range,
                options,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn execute_command(&self, command: Command) -> Intel<()> {
        self.request(Call::ExecuteCommand(command), Err(IntelError::Unsupported))
    }

    fn rename(&self, doc: &DocumentId, at: Point, new_name: String) -> Intel<WorkspaceEdit> {
        self.request(
            Call::Rename {
                doc: doc.clone(),
                at,
                new_name,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn document_highlights(&self, doc: &DocumentId, at: Point) -> Intel<Vec<DocumentHighlight>> {
        self.request(
            Call::DocumentHighlights {
                doc: doc.clone(),
                at,
            },
            Err(IntelError::Unsupported),
        )
    }

    fn status(&self) -> Vec<ServerStatus> {
        self.lock().status.clone()
    }

    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.events.subscribe()
    }

    fn restart(&self, server: Option<&ServerId>) {
        self.record(Call::Restart(server.cloned()));
    }

    fn stop(&self, server: &ServerId) {
        self.record(Call::Stop(server.clone()));
    }

    fn shutdown(&self) -> Intel<()> {
        self.request(Call::Shutdown, Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use text::{Buffer, CaretPlacement, Edit, EditKind, Selection, SelectionSet};

    use super::*;
    use crate::{CodeActionKind, Markup, Severity};

    fn doc() -> DocumentId {
        DocumentId::new("/p/a.py")
    }

    #[test]
    fn documents_mirror_the_reported_edits() {
        let fake = FakeCodeIntel::new();
        let intel: Arc<dyn CodeIntel> = fake.clone();
        let mut buffer = Buffer::new("import os\n😀 = 1\n");
        let mut version = 1;
        intel.did_open(&doc(), "python", version, buffer.rope());
        assert_eq!(fake.language(&doc()).as_deref(), Some("python"));
        let mut report = |buffer: &Buffer, applied: Vec<AppliedEdit>| {
            version += 1;
            intel.did_change(&doc(), version, &applied, buffer.rope());
        };
        let mut sel = SelectionSet::from_vec(vec![Selection::caret(0), Selection::caret(10)]);
        let applied = buffer.transact(
            &mut sel,
            vec![Edit::insert(0, "# "), Edit::new(10..11, "x\ny")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        report(&buffer, applied);
        let (applied, _) = buffer.undo().unwrap();
        report(&buffer, applied);
        let (applied, _) = buffer.redo().unwrap();
        report(&buffer, applied);
        assert_eq!(fake.text(&doc()), Some(buffer.text()));
        assert!(fake.in_sync(&doc()));
        assert_eq!(fake.version(&doc()), Some(4));

        // Edits that do not fit are noticed, not applied blindly.
        let mut wrong = AppliedEdit {
            start: 99,
            old_end: 100,
            ..buffer
                .clone()
                .transact(
                    &mut sel,
                    vec![Edit::insert(0, "z")],
                    EditKind::Other,
                    CaretPlacement::AfterEachEdit,
                )
                .remove(0)
        };
        wrong.text = "!".into();
        intel.did_change(&doc(), 5, &[wrong], buffer.rope());
        assert!(!fake.in_sync(&doc()));

        intel.did_reload(&doc(), 6, &Rope::from_str("new"));
        assert_eq!(fake.text(&doc()).as_deref(), Some("new"));
        intel.did_save(&doc(), &Rope::from_str("new"));
        intel.did_close(&doc());
        assert!(fake.documents().is_empty());
        assert!(matches!(fake.calls().last(), Some(Call::DidClose { .. })));
        assert_eq!(fake.take_calls().len(), 8);
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn scripted_answers_and_the_default() {
        let fake = FakeCodeIntel::new();
        let intel: Arc<dyn CodeIntel> = fake.clone();
        let at = Point::new(1, 2);
        assert_eq!(
            block_on(intel.hover(&doc(), at)),
            Err(IntelError::Unsupported)
        );
        fake.on_hover(|_, at| {
            Ok(Some(Hover {
                contents: Markup::markdown(format!("at {}", at.column)),
                range: None,
            }))
        });
        let hover = block_on(intel.hover(&doc(), at)).unwrap().unwrap();
        assert_eq!(hover.contents.text, "at 2");
        assert_eq!(fake.requests(RequestKind::Hover), 2);

        fake.on_code_actions(|_, _, context| {
            let all = [
                CodeAction::new("Remove unused import", CodeActionKind::QuickFix),
                CodeAction::new("Organize imports", CodeActionKind::organize_imports()),
            ];
            Ok(all.into_iter().filter(|a| context.wants(&a.kind)).collect())
        });
        let actions =
            block_on(intel.code_actions(&doc(), at..at, CodeActionContext::organize_imports()))
                .unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].title, "Organize imports");
        assert_eq!(block_on(intel.shutdown()), Ok(()));
        assert_eq!(
            block_on(intel.format(&doc(), None, FormatOptions::default())),
            Err(IntelError::Unsupported)
        );
    }

    #[test]
    fn held_answers_wait_for_release_and_notice_cancellation() {
        let fake = FakeCodeIntel::new();
        let intel: Arc<dyn CodeIntel> = fake.clone();
        fake.on_completion(|_, _, trigger| {
            let label = match trigger {
                CompletionTrigger::Character(c) => c.to_string(),
                _ => "any".into(),
            };
            Ok(Completions {
                items: vec![CompletionItem::new(label)],
                incomplete: true,
            })
        });
        fake.hold(RequestKind::Completion);
        let first = intel.completion(&doc(), Point::new(0, 1), CompletionTrigger::Character('.'));
        let second = intel.completion(&doc(), Point::new(0, 2), CompletionTrigger::Invoked);
        assert_eq!(fake.pending(RequestKind::Completion), 2);
        // A newer request replaces the older one by dropping it.
        drop(first);
        assert_eq!(fake.pending(RequestKind::Completion), 1);
        assert_eq!(fake.dropped(RequestKind::Completion), 1);
        assert_eq!(fake.release(RequestKind::Completion), 1);
        let answer = block_on(second).unwrap();
        assert_eq!(answer.items[0].label, "any");
        assert!(answer.incomplete);

        let third = intel.completion(&doc(), Point::new(0, 3), CompletionTrigger::Character('.'));
        assert!(fake.release_next(RequestKind::Completion));
        assert_eq!(block_on(third).unwrap().items[0].label, ".");
        assert!(!fake.release_next(RequestKind::Completion));

        fake.answer_at_once(RequestKind::Completion);
        assert!(
            block_on(intel.completion(&doc(), Point::new(0, 0), CompletionTrigger::Invoked))
                .is_ok()
        );

        // Answers still held when the fake goes away are cancelled.
        fake.hold(RequestKind::Hover);
        let hover = intel.hover(&doc(), Point::new(0, 0));
        drop(intel);
        drop(fake);
        assert_eq!(block_on(hover), Err(IntelError::Cancelled));
    }

    #[test]
    fn events_and_status() {
        let fake = FakeCodeIntel::new();
        let intel: Arc<dyn CodeIntel> = fake.clone();
        let events = intel.events();
        let mut ruff = ServerStatus::new(ServerId::new("ruff"), "ruff");
        ruff.documents.push(doc());
        fake.set_status(vec![ruff.clone()]);
        assert_eq!(events.try_recv().unwrap(), IntelEvent::Status(ruff));
        assert_eq!(intel.servers_for(&doc()), [ServerId::new("ruff")]);
        // Trigger characters are those of open documents.
        fake.set_completion_triggers(&['.']);
        assert!(intel.completion_triggers(&doc()).is_empty());
        intel.did_open(&doc(), "python", 1, &Rope::from_str(""));
        assert_eq!(intel.completion_triggers(&doc()), ['.']);
        fake.take_calls();
        let problem = Diagnostic::new(
            Point::new(0, 0)..Point::new(0, 6),
            Severity::Error,
            "undefined name",
        );
        fake.publish_diagnostics(&doc(), 3, vec![problem.clone()]);
        assert_eq!(
            events.try_recv().unwrap(),
            IntelEvent::Diagnostics {
                doc: doc(),
                version: 3,
                items: vec![problem],
            }
        );
        intel.restart(None);
        intel.stop(&ServerId::new("ty"));
        assert_eq!(
            fake.calls(),
            [Call::Restart(None), Call::Stop(ServerId::new("ty"))]
        );
    }
}
