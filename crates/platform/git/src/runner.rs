use std::ffi::OsString;
use std::io::Read as _;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::Duration;

use vcs::{Cancel, VcsError};

use crate::exe::GitExe;

/// How a command touches the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads only: runs in parallel with other reads, with
    /// `GIT_OPTIONAL_LOCKS=0` so that it never rewrites the index.
    Read,
    /// Changes the repository (add, checkout, commit, ...): one at a time,
    /// never during a read.
    Write,
}

/// What a finished command printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// The exit code; `None` when the process was ended by a signal.
    pub code: Option<i32>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// How often a running command looks at its cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(10);

/// Runs git commands for one repository: reads in parallel, writes one at
/// a time and not during reads, every one cancellable.
pub struct Runner {
    exe: GitExe,
    lock: RwLock<()>,
    writes: AtomicUsize,
    /// Called after each write command (status refreshes).
    after_write: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl Runner {
    pub fn new(exe: GitExe) -> Self {
        Self {
            exe,
            lock: RwLock::new(()),
            writes: AtomicUsize::new(0),
            after_write: Mutex::new(Vec::new()),
        }
    }

    pub fn exe(&self) -> &GitExe {
        &self.exe
    }

    /// A write command is running: changes on disk are its own.
    pub fn is_writing(&self) -> bool {
        self.writes.load(Ordering::SeqCst) > 0
    }

    /// Calls `f` after every write command.
    pub fn on_write_finished(&self, f: impl Fn() + Send + Sync + 'static) {
        self.after_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Box::new(f));
    }

    /// Runs `git <args>` in `dir`. A non-zero exit is not an error here:
    /// callers look at [`Output::code`] (some commands exit with 1 to say
    /// "no").
    pub fn run<I, S>(
        &self,
        dir: &Path,
        args: I,
        access: Access,
        cancel: &Cancel,
    ) -> Result<Output, VcsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
        match access {
            Access::Read => {
                let _read = self.lock.read().unwrap_or_else(PoisonError::into_inner);
                self.spawn(dir, &args, access, cancel)
            }
            Access::Write => {
                self.writes.fetch_add(1, Ordering::SeqCst);
                let out = {
                    let _write = self.lock.write().unwrap_or_else(PoisonError::into_inner);
                    self.spawn(dir, &args, access, cancel)
                };
                self.writes.fetch_sub(1, Ordering::SeqCst);
                for f in self
                    .after_write
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                {
                    f();
                }
                out
            }
        }
    }

    /// Like [`Runner::run`], an exit code other than 0 being an error with
    /// git's message.
    pub fn run_ok<I, S>(
        &self,
        dir: &Path,
        args: I,
        access: Access,
        cancel: &Cancel,
    ) -> Result<Output, VcsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let out = self.run(dir, args, access, cancel)?;
        if out.success() {
            Ok(out)
        } else {
            Err(VcsError::failed(error_text(&out)))
        }
    }

    fn spawn(
        &self,
        dir: &Path,
        args: &[OsString],
        access: Access,
        cancel: &Cancel,
    ) -> Result<Output, VcsError> {
        if cancel.is_cancelled() {
            return Err(VcsError::cancelled());
        }
        let mut cmd = self.exe.command();
        cmd.args(args)
            .current_dir(native(dir))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if access == Access::Read {
            cmd.env("GIT_OPTIONAL_LOCKS", "0");
        }
        let mut child = cmd.spawn().map_err(|e| {
            VcsError::unavailable(format!("Cannot run {}: {e}", self.exe.path.display()))
        })?;
        wait(&mut child, cancel)
    }
}

/// The folder as the system's tools take it: on Windows without the
/// `\\?\` prefix that `canonicalize` adds.
fn native(dir: &Path) -> std::borrow::Cow<'_, Path> {
    #[cfg(windows)]
    if let Some(rest) = dir.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && !rest.starts_with("UNC\\")
    {
        return std::borrow::Cow::Owned(std::path::PathBuf::from(rest));
    }
    std::borrow::Cow::Borrowed(dir)
}

/// Reads the output of `child` until it ends, or kills it when cancelled.
fn wait(child: &mut Child, cancel: &Cancel) -> Result<Output, VcsError> {
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let (tx, rx) = flume::bounded(1);
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let stdout = loop {
        match rx.recv_timeout(CANCEL_POLL) {
            Ok(buf) => break buf,
            Err(flume::RecvTimeoutError::Timeout) if cancel.is_cancelled() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(VcsError::cancelled());
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(flume::RecvTimeoutError::Disconnected) => break Vec::new(),
        }
    };
    let status = child
        .wait()
        .map_err(|e| VcsError::unavailable(e.to_string()))?;
    let stderr = err.join().unwrap_or_default();
    Ok(Output {
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: status.code(),
    })
}

/// git's message of a failed command: its error output, else its output.
pub fn error_text(out: &Output) -> String {
    let err = out.stderr.trim();
    if err.is_empty() {
        out.stdout_text().trim().to_owned()
    } else {
        err.to_owned()
    }
}
