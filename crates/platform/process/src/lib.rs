//! External processes of Pyoxide IDE, for the Run tool window.
//!
//! [`spawn`] starts a [`CommandLine`] with piped standard streams and returns
//! a [`ProcessHandle`] and a bounded channel of [`ProcessEvent`]s:
//!
//! - output arrives as raw byte chunks as soon as the process writes them (a
//!   prompt without a trailing newline shows up immediately), standard output
//!   and standard error separately, each stream in order;
//! - [`ProcessEvent::Exited`] is the last event, sent after both streams were
//!   drained;
//! - the channel is bounded: a process that prints faster than the consumer
//!   reads is throttled by the full pipe instead of growing memory;
//! - background threads read, write and wait, so no call blocks the caller's
//!   thread (in particular the UI thread).
//!
//! Stopping follows the reference IDE's stop action (`KillableProcessHandler`
//! and `StopAction` of IntelliJ Platform): the first [`ProcessHandle::stop`]
//! terminates gracefully (Ctrl+C: `SIGINT` on Unix, a console Ctrl+C event on
//! Windows), the next one kills the whole process tree. Dropping the handle
//! does not stop the process; closing a console must stop it explicitly.
//!
//! No UI toolkit and no async runtime here: the receiver is a
//! [`flume::Receiver`], usable both blocking (`recv`) and from any executor
//! (`recv_async`, `stream`).

mod command;
mod exit;
mod handle;
mod sys;

pub use command::CommandLine;
pub use exit::ExitStatus;
pub use flume::{self, Receiver};
pub use handle::{ProcessHandle, spawn};

/// An event of a running process, in the order the process produced it
/// (per stream). [`ProcessEvent::Exited`] is always the last event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEvent {
    /// A chunk of standard output, as read from the pipe (not split into lines,
    /// not decoded; a multi-byte character may be split between chunks).
    Stdout(Vec<u8>),
    /// A chunk of standard error, like [`ProcessEvent::Stdout`].
    Stderr(Vec<u8>),
    /// The process exited and its output was drained. Output a descendant
    /// writes after the process itself exited is not read, as in the
    /// reference.
    Exited(ExitStatus),
}

/// The number of events the channel holds before the readers wait for the
/// consumer. With chunks of up to [`READ_CHUNK_SIZE`] bytes this bounds the
/// output in flight to a few hundred kilobytes per process.
pub const EVENT_CHANNEL_CAPACITY: usize = 64;

/// The largest output chunk one read produces (the reference reads 8192
/// characters at a time).
pub const READ_CHUNK_SIZE: usize = 8192;
