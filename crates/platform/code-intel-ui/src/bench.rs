//! Language services for measurements (`PYOXIDE_BENCH=completion`): a
//! service that answers completion after a fixed latency, so that the
//! time from its answer to the frame with the popup measures the client
//! alone.

use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use code_intel::{
    AppliedEdit, CodeAction, CodeActionContext, CodeIntel, CompletionItem, CompletionTrigger,
    Completions, DocumentId, EventHub, FileChange, FormatOptions, Hover, Intel, IntelError,
    IntelEvent, Location, Point, Rope, ServerId, ServerState, ServerStatus, SignatureContext,
    SignatureHelp, SymbolKind, TextEdit, ready, unsupported,
};
use futures::channel::oneshot;
use gpui::App;

use crate::hook::{CodeIntelHook, CodeIntelProvider};

/// When the last completion answer was sent.
static ANSWERED: Mutex<Option<Instant>> = Mutex::new(None);

/// When the bench service last sent a completion answer.
pub fn last_answer() -> Option<Instant> {
    *ANSWERED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hook of the bench service: `items` completion items of a method
/// each, answered `latency` after the request, `.` as trigger char.
pub fn hook(latency: Duration, items: usize) -> CodeIntelHook {
    CodeIntelHook::new(Provider { latency, items })
}

struct Provider {
    latency: Duration,
    items: usize,
}

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        Arc::new(LatencyIntel::new(self.latency, self.items))
    }
}

/// A service that knows every document of every language and completes
/// with the same items after a fixed time.
pub struct LatencyIntel {
    latency: Duration,
    items: Vec<CompletionItem>,
    events: EventHub,
    documents: Mutex<Vec<DocumentId>>,
}

impl LatencyIntel {
    pub fn new(latency: Duration, items: usize) -> Self {
        let items = (0..items)
            .map(|i| {
                let mut item =
                    CompletionItem::new(format!("member_{i:03}")).with_kind(SymbolKind::Method);
                item.type_text = Some(format!("bound method Bench.member_{i:03}() -> int"));
                item.sort_text = Some(format!("{i:05}"));
                item
            })
            .collect();
        Self {
            latency,
            items,
            events: EventHub::new(),
            documents: Mutex::new(Vec::new()),
        }
    }
}

impl CodeIntel for LatencyIntel {
    fn did_open(&self, doc: &DocumentId, _: &str, _: u64, _: &Rope) {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(doc.clone());
    }
    fn did_change(&self, _: &DocumentId, _: u64, _: &[AppliedEdit], _: &Rope) {}
    fn did_reload(&self, _: &DocumentId, _: u64, _: &Rope) {}
    fn did_save(&self, _: &DocumentId, _: &Rope) {}
    fn did_close(&self, doc: &DocumentId) {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|d| d != doc);
    }
    fn files_changed(&self, _: &[FileChange]) {}

    fn completion(&self, _: &DocumentId, _: Point, _: CompletionTrigger) -> Intel<Completions> {
        let (tx, rx) = oneshot::channel();
        let (latency, items) = (self.latency, self.items.clone());
        std::thread::spawn(move || {
            std::thread::sleep(latency);
            *ANSWERED.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
            let _ = tx.send(Completions {
                items,
                incomplete: false,
            });
        });
        Box::pin(async move { rx.await.map_err(|_| IntelError::Cancelled) })
    }

    fn completion_triggers(&self, _: &DocumentId) -> Vec<char> {
        vec!['.']
    }

    fn resolve_completion(&self, _: &DocumentId, item: CompletionItem) -> Intel<CompletionItem> {
        ready(Ok(item))
    }
    fn hover(&self, _: &DocumentId, _: Point) -> Intel<Option<Hover>> {
        unsupported()
    }
    fn signature_help(
        &self,
        _: &DocumentId,
        _: Point,
        _: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        unsupported()
    }
    fn definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
        unsupported()
    }
    fn type_definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
        unsupported()
    }
    fn references(&self, _: &DocumentId, _: Point, _: bool) -> Intel<Vec<Location>> {
        unsupported()
    }
    fn code_actions(
        &self,
        _: &DocumentId,
        _: Range<Point>,
        _: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        unsupported()
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
        let mut status = ServerStatus::new(ServerId::new("bench"), "bench");
        status.state = ServerState::Running;
        status.documents = self
            .documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        vec![status]
    }
    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.events.subscribe()
    }
    fn restart(&self, _: Option<&ServerId>) {}
    fn stop(&self, _: &ServerId) {}
    fn shutdown(&self) -> Intel<()> {
        ready(Ok(()))
    }
}
