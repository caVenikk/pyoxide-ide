//! Code intelligence of Pyoxide IDE (ADR 0010): the language-agnostic
//! model of what language services answer, and the [`CodeIntel`] trait the
//! editor UI talks to. It is to language servers what `db-core` is to
//! database drivers: `lsp-client` implements the trait over LSP servers,
//! the UI (`code-intel-ui`) depends on the trait only, and tests use the
//! in-memory [`fake::FakeCodeIntel`] (feature `fake`).
//!
//! Conventions:
//!
//! - A document is a file open in an editor tab ([`DocumentId`], its
//!   absolute path). The caller reports its text and every change, with a
//!   version number that increases with each change it reports.
//! - Positions are [`text::Point`]s (zero-based line, column in chars) in
//!   the text of the document as last reported; ranges are
//!   `Range<Point>`. Implementations convert to their protocol's encoding.
//! - Requests return an [`Intel`] future. Dropping it cancels the request
//!   (a newer request replaces an older one by dropping it). Nothing here
//!   blocks: lifecycle calls return at once, answers arrive through the
//!   futures and [`IntelEvent`]s.
//! - Features a document has no service for answer
//!   [`IntelError::Unsupported`].
//! - Several providers (a product's and a module's, ADR 0022) are one
//!   [`RoutedCodeIntel`], which routes each document by its language id.
//!
//! No UI toolkit and no protocol types here.

mod action;
mod completion;
mod diagnostic;
mod document;
mod edit;
mod events;
mod markup;
pub mod positions;
mod routed;
mod server;
mod symbol;

#[cfg(any(test, feature = "fake"))]
pub mod fake;

use std::ops::Range;

use futures::future::BoxFuture;

pub use action::{ActionTrigger, CodeAction, CodeActionContext, CodeActionKind, Command};
pub use completion::{CompletionItem, CompletionTrigger, Completions, InsertText, SymbolKind};
pub use diagnostic::{Diagnostic, Severity};
pub use document::{DocumentId, FileChange, FileChangeKind, Token};
pub use edit::{
    DocumentHighlight, FileEdits, FormatOptions, HighlightKind, Location, TextEdit, WorkspaceEdit,
};
pub use events::{EditReply, EventHub, IntelEvent, MessageLevel};
pub use markup::{
    Hover, Markup, MarkupKind, ParameterInfo, Signature, SignatureContext, SignatureHelp,
    SignatureTrigger,
};
pub use routed::{Route, RoutedCodeIntel};
pub use server::{Feature, Features, ServerId, ServerState, ServerStatus};
pub use symbol::{DefinitionTarget, DocumentSymbol, ObjectTarget};
pub use text::{AppliedEdit, Point, Rope};

/// The answer of a request: a future that is cancelled by dropping it.
pub type Intel<T> = BoxFuture<'static, Result<T, IntelError>>;

/// Why a request has no answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntelError {
    /// No service of the document offers the feature.
    #[error("not supported for this file")]
    Unsupported,
    /// The request was cancelled, by the server (the text changed) or
    /// because the services shut down.
    #[error("the request was cancelled")]
    Cancelled,
    /// The server that would answer is not running (stopped, starting
    /// failed, or it terminated).
    #[error("the language server is not running")]
    NotRunning,
    /// The server answered with an error.
    #[error("{server}: {message}")]
    Server {
        server: ServerId,
        code: i64,
        message: String,
    },
    /// Anything else (a malformed answer, an I/O error).
    #[error("{0}")]
    Other(String),
}

/// An answer that is there already.
pub fn ready<T: Send + 'static>(result: Result<T, IntelError>) -> Intel<T> {
    Box::pin(futures::future::ready(result))
}

/// The answer of a feature no service offers.
pub fn unsupported<T: Send + 'static>() -> Intel<T> {
    ready(Err(IntelError::Unsupported))
}

/// The language services of one project.
///
/// Implementations are shared between threads (`Send + Sync`) and never
/// block the caller: lifecycle calls queue their work and return,
/// requests return futures. The UI calls everything from its own thread
/// and awaits the futures on its executor.
pub trait CodeIntel: Send + Sync + 'static {
    // ---- documents ---------------------------------------------------------

    /// A file was opened in an editor: its language id (`"python"`), the
    /// first version number and the text.
    fn did_open(&self, doc: &DocumentId, language: &str, version: u64, text: &Rope);

    /// The text changed: the edits as applied (in order, each in the text
    /// the previous ones left, see [`text::AppliedEdit`]) and the text
    /// after them. `version` is higher than every version reported before.
    fn did_change(&self, doc: &DocumentId, version: u64, edits: &[AppliedEdit], text: &Rope);

    /// The whole text was replaced (reloaded from disk): no edits, only
    /// the new text.
    fn did_reload(&self, doc: &DocumentId, version: u64, text: &Rope);

    /// The document was written to disk with this text.
    fn did_save(&self, doc: &DocumentId, text: &Rope);

    /// The editor of the document was closed.
    fn did_close(&self, doc: &DocumentId);

    /// Files changed on disk by the application itself (saved, created,
    /// deleted, renamed outside an editor).
    fn files_changed(&self, changes: &[FileChange]);

    // ---- features ----------------------------------------------------------

    /// Completion items at a position.
    fn completion(
        &self,
        doc: &DocumentId,
        at: Point,
        trigger: CompletionTrigger,
    ) -> Intel<Completions>;

    /// Fills in the parts of a completion item a server sends on demand
    /// (documentation, additional edits). Items without a token come back
    /// as they are.
    fn resolve_completion(&self, doc: &DocumentId, item: CompletionItem) -> Intel<CompletionItem>;

    /// The characters that open completion when typed in the document (the
    /// servers' trigger characters, such as `.`).
    fn completion_triggers(&self, doc: &DocumentId) -> Vec<char> {
        let _ = doc;
        Vec::new()
    }

    /// Documentation of the symbol at a position (quick documentation,
    /// hover).
    fn hover(&self, doc: &DocumentId, at: Point) -> Intel<Option<Hover>>;

    /// Signatures of the call at a position (parameter info).
    fn signature_help(
        &self,
        doc: &DocumentId,
        at: Point,
        context: SignatureContext,
    ) -> Intel<Option<SignatureHelp>>;

    /// Where the symbol at a position is declared.
    fn definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>>;

    /// Where Go to Declaration goes from a position: places in files, or
    /// objects that are no place in a file (a database table). By default
    /// the places of [`definition`](Self::definition).
    fn definition_targets(&self, doc: &DocumentId, at: Point) -> Intel<Vec<DefinitionTarget>> {
        let definition = self.definition(doc, at);
        Box::pin(async move {
            let locations = definition.await?;
            Ok(locations.into_iter().map(DefinitionTarget::File).collect())
        })
    }

    /// Where the type of the symbol at a position is declared.
    fn type_definition(&self, doc: &DocumentId, at: Point) -> Intel<Vec<Location>>;

    /// The usages of the symbol at a position.
    fn references(
        &self,
        doc: &DocumentId,
        at: Point,
        include_declaration: bool,
    ) -> Intel<Vec<Location>>;

    /// Quick fixes and other actions for a range, from every service of
    /// the document, quick fixes first.
    fn code_actions(
        &self,
        doc: &DocumentId,
        range: Range<Point>,
        context: CodeActionContext,
    ) -> Intel<Vec<CodeAction>>;

    /// Fills in the edit of an action a server computes on demand.
    fn resolve_code_action(&self, doc: &DocumentId, action: CodeAction) -> Intel<CodeAction>;

    /// Edits that format the document, or a range of it, from the one
    /// service that formats it.
    fn format(
        &self,
        doc: &DocumentId,
        range: Option<Range<Point>>,
        options: FormatOptions,
    ) -> Intel<Vec<TextEdit>>;

    /// Runs a server command (of a code action without an edit). Edits it
    /// makes arrive as [`IntelEvent::ApplyEdit`].
    fn execute_command(&self, command: Command) -> Intel<()> {
        let _ = command;
        unsupported()
    }

    /// Edits that rename the symbol at a position.
    fn rename(&self, doc: &DocumentId, at: Point, new_name: String) -> Intel<WorkspaceEdit> {
        let _ = (doc, at, new_name);
        unsupported()
    }

    /// The occurrences in the document of the symbol at a position.
    fn document_highlights(&self, doc: &DocumentId, at: Point) -> Intel<Vec<DocumentHighlight>> {
        let _ = (doc, at);
        unsupported()
    }

    /// The structure of a document: its symbols with the symbols declared
    /// inside them, in the order of the text.
    fn document_symbols(&self, doc: &DocumentId) -> Intel<Vec<DocumentSymbol>> {
        let _ = doc;
        unsupported()
    }

    // ---- servers -----------------------------------------------------------

    /// The servers of the project and their states.
    fn status(&self) -> Vec<ServerStatus>;

    /// The servers that serve a document now.
    fn servers_for(&self, doc: &DocumentId) -> Vec<ServerId> {
        self.status()
            .into_iter()
            .filter(|s| s.serves(doc))
            .map(|s| s.id)
            .collect()
    }

    /// A new receiver of the events from now on. Every receiver gets every
    /// event; a dropped receiver is forgotten.
    fn events(&self) -> flume::Receiver<IntelEvent>;

    /// Restarts one server, or all with `None`. States arrive as
    /// [`IntelEvent::Status`].
    fn restart(&self, server: Option<&ServerId>);

    /// Stops a server; it starts again when a document needs it.
    fn stop(&self, server: &ServerId);

    /// Stops every server (the project closes or the application quits).
    /// The future ends when they have exited or were killed.
    fn shutdown(&self) -> Intel<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal implementation: the trait's provided methods work on it.
    struct Nothing(EventHub);

    impl CodeIntel for Nothing {
        fn did_open(&self, _: &DocumentId, _: &str, _: u64, _: &Rope) {}
        fn did_change(&self, _: &DocumentId, _: u64, _: &[AppliedEdit], _: &Rope) {}
        fn did_reload(&self, _: &DocumentId, _: u64, _: &Rope) {}
        fn did_save(&self, _: &DocumentId, _: &Rope) {}
        fn did_close(&self, _: &DocumentId) {}
        fn files_changed(&self, _: &[FileChange]) {}
        fn completion(&self, _: &DocumentId, _: Point, _: CompletionTrigger) -> Intel<Completions> {
            unsupported()
        }
        fn resolve_completion(
            &self,
            _: &DocumentId,
            item: CompletionItem,
        ) -> Intel<CompletionItem> {
            ready(Ok(item))
        }
        fn hover(&self, _: &DocumentId, _: Point) -> Intel<Option<Hover>> {
            ready(Ok(None))
        }
        fn signature_help(
            &self,
            _: &DocumentId,
            _: Point,
            _: SignatureContext,
        ) -> Intel<Option<SignatureHelp>> {
            ready(Ok(None))
        }
        fn definition(&self, doc: &DocumentId, _: Point) -> Intel<Vec<Location>> {
            ready(Ok(vec![Location {
                path: doc.path().to_owned(),
                range: Point::new(1, 0)..Point::new(1, 1),
            }]))
        }
        fn type_definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
            ready(Ok(Vec::new()))
        }
        fn references(&self, _: &DocumentId, _: Point, _: bool) -> Intel<Vec<Location>> {
            ready(Ok(Vec::new()))
        }
        fn code_actions(
            &self,
            _: &DocumentId,
            _: Range<Point>,
            _: CodeActionContext,
        ) -> Intel<Vec<CodeAction>> {
            ready(Ok(Vec::new()))
        }
        fn resolve_code_action(&self, _: &DocumentId, action: CodeAction) -> Intel<CodeAction> {
            ready(Ok(action))
        }
        fn format(
            &self,
            _: &DocumentId,
            _: Option<Range<Point>>,
            _: FormatOptions,
        ) -> Intel<Vec<TextEdit>> {
            unsupported()
        }
        fn status(&self) -> Vec<ServerStatus> {
            let mut ruff = ServerStatus::new(ServerId::new("ruff"), "ruff");
            ruff.documents.push(DocumentId::new("/p/a.py"));
            vec![ruff, ServerStatus::new(ServerId::new("ty"), "ty")]
        }
        fn events(&self) -> flume::Receiver<IntelEvent> {
            self.0.subscribe()
        }
        fn restart(&self, _: Option<&ServerId>) {}
        fn stop(&self, _: &ServerId) {}
        fn shutdown(&self) -> Intel<()> {
            ready(Ok(()))
        }
    }

    #[test]
    fn provided_methods() {
        let intel: std::sync::Arc<dyn CodeIntel> = std::sync::Arc::new(Nothing(EventHub::new()));
        let doc = DocumentId::new("/p/a.py");
        assert_eq!(intel.servers_for(&doc), [ServerId::new("ruff")]);
        assert!(intel.completion_triggers(&doc).is_empty());
        assert!(intel.servers_for(&DocumentId::new("/p/b.py")).is_empty());

        assert_eq!(
            futures::executor::block_on(intel.rename(&doc, Point::new(0, 0), "x".into())),
            Err(IntelError::Unsupported)
        );
        assert_eq!(
            futures::executor::block_on(intel.document_highlights(&doc, Point::new(0, 0))),
            Err(IntelError::Unsupported)
        );
        assert_eq!(
            futures::executor::block_on(intel.document_symbols(&doc)),
            Err(IntelError::Unsupported)
        );
        // The definition's places, as file targets.
        assert_eq!(
            futures::executor::block_on(intel.definition_targets(&doc, Point::new(0, 0))),
            Ok(vec![DefinitionTarget::File(Location {
                path: "/p/a.py".into(),
                range: Point::new(1, 0)..Point::new(1, 1),
            })])
        );
        let command = Command {
            title: "Organize".into(),
            token: Token::new(1u8),
        };
        assert_eq!(
            futures::executor::block_on(intel.execute_command(command)),
            Err(IntelError::Unsupported)
        );
        assert_eq!(
            IntelError::Server {
                server: ServerId::new("ty"),
                code: -32603,
                message: "boom".into()
            }
            .to_string(),
            "ty: boom"
        );
    }
}
