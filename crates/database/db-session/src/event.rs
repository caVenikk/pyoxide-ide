use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use db_core::{Column, DataSourceId, Error, Purpose, ResultSet, Row, TxMode};
use serde::Serialize;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub(crate) fn next() -> Self {
                Self(next_id())
            }

            /// The number behind the id; unique in the process.
            pub fn get(self) -> u64 {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(
    /// A console session of a data source.
    SessionId
);
id_type!(
    /// One run of a job (statements, a refresh, a data read).
    JobId
);
id_type!(
    /// A result set a statement opened. Its rows can be paged with
    /// [`Session::fetch`](crate::Session::fetch) while the session keeps its
    /// cursor open (until [`SessionEvent::ResultClosed`]).
    ResultId
);

/// Receives every event of a data source and its sessions, on a thread of the
/// database runtime. It must return quickly: the UI crate turns an event into
/// a foreground task and a `notify`, tests push it into a channel.
#[derive(Clone)]
pub struct EventSink(Arc<dyn Fn(Event) + Send + Sync>);

impl EventSink {
    pub fn new(f: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// A sink that sends into a standard channel, for tests and headless
    /// callers that wait on a thread of their own.
    pub fn channel() -> (Self, std::sync::mpsc::Receiver<Event>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let sink = Self::new(move |event| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(event);
            }
        });
        (sink, rx)
    }

    /// A sink that drops every event.
    pub fn ignore() -> Self {
        Self::new(|_| {})
    }

    pub(crate) fn emit(&self, data_source: &DataSourceId, kind: EventKind) {
        (self.0)(Event {
            data_source: data_source.clone(),
            kind,
        });
    }
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

/// Something that happened in a data source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub data_source: DataSourceId,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// The data source moved to another state.
    State { state: DataSourceState },
    /// An event of one console session.
    Session {
        session: SessionId,
        event: SessionEvent,
    },
    /// An event of a statement job of a session.
    Job {
        session: SessionId,
        job: JobId,
        event: JobEvent,
    },
    /// A metadata refresh (on a connection of its own) progressed.
    Refresh { job: JobId, event: RefreshEvent },
    /// A read on a data connection ended.
    DataRead {
        job: JobId,
        result: Result<ResultSet, Error>,
    },
    /// An action waits for the user: sessions hold pending changes and the
    /// action would end their transactions. Nothing was done; repeat the
    /// action with [`TxPolicy::Commit`](crate::TxPolicy::Commit) or
    /// [`TxPolicy::Rollback`](crate::TxPolicy::Rollback), or drop it.
    DecisionNeeded {
        reason: DecisionReason,
        sessions: Vec<SessionId>,
    },
    /// The startup script, the time zone or the read-only mode could not be
    /// applied to a new connection; the connection is used anyway.
    SetupFailed { purpose: Purpose, error: Error },
}

/// Connection state of a data source (ADR 0021).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DataSourceState {
    /// No connection is open.
    Disconnected,
    /// The first connection is being opened.
    Connecting,
    /// At least one connection was opened and the data source was not
    /// disconnected since.
    Connected,
    /// The last attempt to connect failed.
    Failed { error: Error },
}

/// Why an action waits for a decision ([`EventKind::DecisionNeeded`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    /// [`DataSourceHandle::disconnect`](crate::DataSourceHandle::disconnect).
    Disconnect,
    /// The auto-disconnect time passed while changes are pending. Asked once
    /// per idle period; the connections stay open.
    IdleDisconnect,
    /// [`Session::close`](crate::Session::close).
    Close,
    /// [`Session::set_tx_mode`](crate::Session::set_tx_mode) to auto.
    SwitchToAuto,
}

/// What a session's transaction looks like now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct TxStatus {
    pub mode: TxMode,
    /// A transaction is open on the server.
    pub open: bool,
    /// Data-changing statements since the last commit or rollback: DML,
    /// anonymous blocks, and DDL where DDL is transactional (PostgreSQL,
    /// SQLite). Closing or disconnecting asks while this is not zero.
    pub pending: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// The session opened a connection (the first, or a new one after the
    /// old one was lost, abandoned or disconnected).
    Connected,
    /// Transaction mode, open transaction or pending count changed.
    Transaction { status: TxStatus },
    /// A transaction with pending changes was lost: the connection broke or
    /// was abandoned after a cancel the driver could not deliver. The changes
    /// are gone.
    TransactionLost { pending: u32 },
    /// The current schema (PostgreSQL: the first schema of the search path)
    /// is known or changed.
    Schema { schema: String },
    /// More rows of an open result ([`Session::fetch`](crate::Session::fetch)).
    Page {
        result: ResultId,
        /// Zero-based number of the first row of `rows` in the result.
        first_row: u64,
        rows: Vec<Row>,
        has_more: bool,
    },
    /// The cursor of a result was closed (another statement ran, the result
    /// was read to the end, a commit); its rows can no longer be fetched.
    ResultClosed { result: ResultId },
    /// A command other than a job failed (commit, rollback, set schema,
    /// fetch, set transaction mode).
    CommandFailed { command: Command, error: Error },
    /// A commit, rollback, schema or mode change was done.
    CommandDone { command: Command },
    /// The session was closed and accepts no more commands.
    Closed,
}

/// The session commands that report [`SessionEvent::CommandDone`] or
/// [`SessionEvent::CommandFailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Connect,
    Commit,
    Rollback,
    SetSchema,
    SetTxMode,
    SetIsolation,
    Fetch,
    Close,
}

/// Execute and fetch time of a statement, measured separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Timing {
    /// From sending the statement to its first answer.
    pub execute: Duration,
    /// Reading the first page of rows.
    pub fetch: Duration,
}

impl Timing {
    pub fn total(self) -> Duration {
        self.execute + self.fetch
    }
}

/// A statement failed: the server's error (with its SQLSTATE or `ORA-` code
/// in `error.code`) and where it points.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatementError {
    pub error: Error,
    /// Byte offset in the job's script where the error is: the position the
    /// server reported, or the start of the statement when it reported none.
    pub position: usize,
}

/// Events of a statement job, in order: for every statement `Started`, then
/// `ResultOpened` or `Affected`, then `Finished`; or `Started` and `Failed`.
/// The job ends with `Done`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobEvent {
    Started {
        /// Index of the statement in the job.
        index: usize,
        /// Byte range of the statement in the job's script.
        range: Range<usize>,
        /// The statement text as sent.
        text: String,
    },
    /// The statement returned rows; the first page is here.
    ResultOpened {
        index: usize,
        result: ResultId,
        columns: Vec<Column>,
        rows: Vec<Row>,
        /// More rows can be fetched with [`Session::fetch`](crate::Session::fetch).
        has_more: bool,
    },
    /// The statement changed `rows` rows (0 when the backend does not say).
    Affected {
        index: usize,
        rows: u64,
    },
    Finished {
        index: usize,
        timing: Timing,
    },
    Failed {
        index: usize,
        error: StatementError,
        timing: Timing,
    },
    Done {
        outcome: JobOutcome,
    },
}

/// How a statement job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    /// Every statement ran (some may have failed when the job's policy was
    /// to continue).
    Completed,
    /// A statement failed and the job stopped there.
    Failed,
    /// Cancelled by [`Session::cancel`](crate::Session::cancel).
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RefreshEvent {
    Started,
    /// The new model is in the data source's model handle.
    Finished {
        /// The model handle's generation after the swap.
        generation: u64,
        objects: usize,
        /// What the refresh read and dropped.
        summary: crate::refresh::RefreshSummary,
    },
    Failed {
        error: Error,
    },
    /// Cancelled; the model is unchanged.
    Cancelled,
}
