//! What the services report on their own: diagnostics, server states,
//! messages, progress, and edits a server asks the application to make.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use futures::channel::oneshot;

use crate::diagnostic::Diagnostic;
use crate::document::DocumentId;
use crate::edit::WorkspaceEdit;
use crate::server::{ServerId, ServerStatus};

/// How important a server's message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MessageLevel {
    Error,
    Warning,
    Info,
    /// Log output, not shown to the user.
    Log,
}

/// The answer to an [`IntelEvent::ApplyEdit`]: whether the edit was
/// applied. The first answer counts; dropping every copy answers `false`.
#[derive(Clone)]
pub struct EditReply(Arc<Mutex<Option<oneshot::Sender<bool>>>>);

impl EditReply {
    /// A reply and the receiver of its answer (for implementations).
    pub fn new() -> (Self, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(tx)))), rx)
    }

    pub fn send(&self, applied: bool) {
        let sender = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(sender) = sender {
            let _ = sender.send(applied);
        }
    }
}

impl fmt::Debug for EditReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EditReply(..)")
    }
}

impl PartialEq for EditReply {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Something the services report without being asked.
#[derive(Debug, Clone, PartialEq)]
pub enum IntelEvent {
    /// The problems of a document now: every server's diagnostics of it,
    /// replacing the previous set, in the text of `version` (the
    /// implementation maps older results through the edits since). The UI
    /// shows diagnostics with the same range as one mark.
    Diagnostics {
        doc: DocumentId,
        version: u64,
        items: Vec<Diagnostic>,
    },
    /// A server's state, name, version, features or documents changed.
    Status(ServerStatus),
    /// A server's message (`window/showMessage`, `window/logMessage`).
    Message {
        server: ServerId,
        level: MessageLevel,
        text: String,
    },
    /// Background work of a server (`$/progress`).
    Progress {
        server: ServerId,
        title: String,
        message: Option<String>,
        /// 0 to 100 when the server knows it.
        percentage: Option<u32>,
        done: bool,
    },
    /// A server asks to apply an edit (`workspace/applyEdit`, after a
    /// command). The receiver applies it as one undo step and answers.
    ApplyEdit {
        server: ServerId,
        label: Option<String>,
        edit: WorkspaceEdit,
        reply: EditReply,
    },
}

/// Delivers events to every receiver (for implementations of
/// [`CodeIntel::events`](crate::CodeIntel::events)).
#[derive(Default)]
pub struct EventHub {
    receivers: Mutex<Vec<flume::Sender<IntelEvent>>>,
}

impl EventHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// A receiver of the events from now on.
    pub fn subscribe(&self) -> flume::Receiver<IntelEvent> {
        let (tx, rx) = flume::unbounded();
        self.lock().push(tx);
        rx
    }

    /// Sends an event to every receiver; dropped receivers are forgotten.
    pub fn emit(&self, event: IntelEvent) {
        let mut receivers = self.lock();
        receivers.retain(|tx| !tx.is_disconnected());
        if let Some((last, rest)) = receivers.split_last() {
            for tx in rest {
                let _ = tx.send(event.clone());
            }
            let _ = last.send(event);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<flume::Sender<IntelEvent>>> {
        self.receivers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for EventHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventHub")
            .field("receivers", &self.lock().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_receiver_gets_every_event() {
        let hub = EventHub::new();
        let a = hub.subscribe();
        let b = hub.subscribe();
        let message = IntelEvent::Message {
            server: ServerId::new("ruff"),
            level: MessageLevel::Warning,
            text: "hi".into(),
        };
        hub.emit(message.clone());
        assert_eq!(a.try_recv().unwrap(), message);
        assert_eq!(b.try_recv().unwrap(), message);
        drop(a);
        hub.emit(message.clone());
        assert_eq!(b.try_recv().unwrap(), message);
        assert!(format!("{hub:?}").contains("receivers: 1"));
    }

    #[test]
    fn the_first_reply_counts() {
        let (reply, rx) = EditReply::new();
        let copy = reply.clone();
        copy.send(true);
        reply.send(false);
        assert_eq!(futures::executor::block_on(rx), Ok(true));
        let (reply, rx) = EditReply::new();
        drop(reply);
        assert!(futures::executor::block_on(rx).is_err());
    }
}
