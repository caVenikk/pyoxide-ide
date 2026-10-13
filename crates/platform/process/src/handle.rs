use std::fmt;
use std::io;
use std::sync::Arc;

use crate::{CommandLine, ExitStatus, ProcessEvent, Receiver};

/// Starts `cmd` with piped standard input, output and error.
///
/// Returns at once; background threads read the output into the returned
/// channel, write the queued input and wait for the exit. On Unix the
/// process leads its own process group (so interrupt and kill reach its
/// children); on Windows it gets its own hidden console (so a console Ctrl+C
/// event reaches it and no console window pops up).
///
/// An error means the process did not start (program not found, working
/// directory missing, ...).
pub fn spawn(cmd: &CommandLine) -> io::Result<(ProcessHandle, Receiver<ProcessEvent>)> {
    let _ = cmd;
    todo!("process::spawn")
}

/// A started process. Cheap to clone; all clones control the same process.
/// Dropping the handles does not stop the process.
#[derive(Clone)]
pub struct ProcessHandle {
    inner: Arc<Inner>,
}

struct Inner {
    pid: u32,
}

impl ProcessHandle {
    /// The process id.
    pub fn pid(&self) -> u32 {
        self.inner.pid
    }

    /// Queues `data` for the process's standard input and returns at once
    /// (a writer thread does the blocking write). Fails when the input was
    /// closed or the process stopped reading it (broken pipe).
    pub fn write_stdin(&self, data: &[u8]) -> io::Result<()> {
        let _ = data;
        todo!("ProcessHandle::write_stdin")
    }

    /// Closes standard input after the queued data was written (the process
    /// reads end of file).
    pub fn close_stdin(&self) {
        todo!("ProcessHandle::close_stdin")
    }

    /// Graceful termination, the reference's first Stop: `SIGINT` to the
    /// process group on Unix; on Windows a Ctrl+C event to the process's
    /// console, followed by a `0xFF` byte on standard input to unblock a
    /// pending console read, as the reference does. Does nothing when the
    /// process already exited. An error means no interrupt could be sent.
    pub fn interrupt(&self) -> io::Result<()> {
        todo!("ProcessHandle::interrupt")
    }

    /// Kills the whole process tree at once (`SIGKILL` to the process group
    /// on Unix; on Windows every descendant started after the process, then
    /// the process, with exit code `-1`, as the reference's WinP does). Does
    /// nothing when the process already exited.
    pub fn kill(&self) -> io::Result<()> {
        todo!("ProcessHandle::kill")
    }

    /// The reference's Stop action: the first call interrupts
    /// ([`ProcessHandle::interrupt`], or kills when no interrupt could be
    /// sent), any further call while the process still runs kills the tree.
    pub fn stop(&self) -> io::Result<()> {
        todo!("ProcessHandle::stop")
    }

    /// The process has not exited yet.
    pub fn is_running(&self) -> bool {
        todo!("ProcessHandle::is_running")
    }

    /// A stop was requested and the process still runs: the next
    /// [`ProcessHandle::stop`] kills (the reference then shows the Stop button
    /// as "Kill process").
    pub fn is_stopping(&self) -> bool {
        todo!("ProcessHandle::is_stopping")
    }

    /// How the process ended, once it has. Available as soon as the process
    /// exited, possibly before [`ProcessEvent::Exited`] (which waits for the
    /// output to be drained).
    pub fn exit_status(&self) -> Option<ExitStatus> {
        todo!("ProcessHandle::exit_status")
    }
}

impl fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("pid", &self.inner.pid)
            .finish()
    }
}
