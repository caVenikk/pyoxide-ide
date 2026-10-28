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
//! Windows, sent by a helper process: see [`set_interrupt_helper`]), the next
//! one kills the whole process tree. Dropping the handle does not stop the
//! process; closing a console must stop it explicitly.
//!
//! No UI toolkit and no async runtime here: the receiver is a
//! [`flume::Receiver`], usable both blocking (`recv`) and from any executor
//! (`recv_async`, `stream`).

mod command;
mod exit;
mod handle;
mod helper;
mod sys;

pub use command::CommandLine;
pub use exit::ExitStatus;
pub use flume::{self, Receiver};
pub use handle::{ProcessHandle, spawn};
pub use helper::{
    InterruptHelper, interrupt_helper_arg, run_interrupt_helper_if_requested, set_interrupt_helper,
};

/// A process with this id is running (any process of the system, not only
/// those started here).
pub fn is_alive(pid: u32) -> bool {
    sys::is_alive(pid)
}

/// The direct children of the process `pid` (not their descendants), or none
/// when the process has none, has ended or the system cannot list them (a
/// Unix system other than Linux and macOS). The reference asks a terminal's
/// shell this to tell whether a command runs in it
/// (`TerminalUtil.hasRunningCommands`). On Windows the console hosts a pseudo-console starts
/// (`conhost.exe`, `OpenConsole.exe`) are not counted, and a process whose
/// parent id is reused (older than the parent) is not a child.
///
/// Reads the process table, which takes a few milliseconds and more on a
/// loaded machine: call it off the UI thread.
pub fn child_processes(pid: u32) -> Vec<u32> {
    sys::child_pids(pid)
}

/// Whether the process `pid` has a child: see [`child_processes`].
pub fn has_child_processes(pid: u32) -> bool {
    !child_processes(pid).is_empty()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_running_process_from_a_missing_one() {
        assert!(is_alive(std::process::id()));
        assert!(!is_alive(0));
        // Above the process id limit of Linux and macOS, not an id on Windows.
        assert!(!is_alive(0x7fff_fff0));
    }
}
