use std::fmt;
use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::sys;
use crate::{
    CommandLine, EVENT_CHANNEL_CAPACITY, ExitStatus, ProcessEvent, READ_CHUNK_SIZE, Receiver,
};

/// After the process exited, how long its output pipes may stay empty but
/// open (held by a descendant) before [`ProcessEvent::Exited`] is sent
/// without waiting for their end. The reference reads what is available when
/// the process exits and stops.
const ORPHAN_PIPE_GRACE: Duration = Duration::from_millis(100);

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
    let mut command = Command::new(&cmd.program);
    command
        .args(&cmd.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &cmd.cwd {
        command.current_dir(dir);
    }
    if !cmd.inherit_env {
        command.env_clear();
    }
    for (name, value) in &cmd.env {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let mut child = sys::spawn(&mut command)?;
    let os = sys::OsProcess::new(&child)?;
    let pid = child.id();
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");

    let (events, receiver) = flume::bounded(EVENT_CHANNEL_CAPACITY);
    let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>();
    let inner = Arc::new(Inner {
        pid,
        os,
        state: Mutex::new(State::default()),
        stdin: Mutex::new(Some(stdin_tx)),
        stdin_broken: Arc::new(AtomicBool::new(false)),
    });
    let output = Arc::new(Output {
        events,
        closed: Mutex::new(false),
        drain: Mutex::new(Drain::default()),
        drained: Condvar::new(),
        readers: [ReaderState::default(), ReaderState::default()],
    });

    let pipes = [sys::pipe_of(&stdout), sys::pipe_of(&stderr)];
    spawn_reader(&output, 0, stdout, ProcessEvent::Stdout)?;
    spawn_reader(&output, 1, stderr, ProcessEvent::Stderr)?;
    spawn_writer(stdin, stdin_rx, inner.stdin_broken.clone())?;
    let waiter_inner = inner.clone();
    thread::Builder::new()
        .name(format!("process {pid} wait"))
        .spawn(move || wait(child, waiter_inner, output, pipes))?;
    Ok((ProcessHandle { inner }, receiver))
}

/// A started process. Cheap to clone; all clones control the same process.
/// Dropping the handles does not stop the process.
#[derive(Clone)]
pub struct ProcessHandle {
    inner: Arc<Inner>,
}

struct Inner {
    pid: u32,
    os: sys::OsProcess,
    state: Mutex<State>,
    stdin: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    stdin_broken: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    exit: Option<ExitStatus>,
    stop_requested: bool,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
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
        if self.inner.stdin_broken.load(Ordering::Acquire) {
            return Err(broken_stdin());
        }
        let stdin = self.inner.stdin.lock().unwrap_or_else(|e| e.into_inner());
        match stdin.as_ref() {
            Some(tx) => tx.send(data.to_vec()).map_err(|_| broken_stdin()),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "standard input is closed",
            )),
        }
    }

    /// Closes standard input after the queued data was written (the process
    /// reads end of file).
    pub fn close_stdin(&self) {
        self.inner
            .stdin
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }

    /// Graceful termination, the reference's first Stop: `SIGINT` to every
    /// process of the tree on Unix (descendants first, as the reference's
    /// `UnixProcessManager.sendSigIntToProcessTree`); on Windows a Ctrl+C
    /// event to the process's console, followed by a `0xFF` byte on standard
    /// input to unblock a pending console read, as the reference does. Does
    /// nothing when the process already exited. An error means no interrupt
    /// could be sent (on Windows, for example, when the IDE itself is
    /// attached to a console: then only [`ProcessHandle::kill`] works).
    ///
    /// Walks the process table: takes a few milliseconds.
    pub fn interrupt(&self) -> io::Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        sys::interrupt(&self.inner.os)?;
        if cfg!(windows) {
            let _ = self.write_stdin(&[0xFF]);
        }
        Ok(())
    }

    /// Kills the whole process tree at once (on Unix `SIGKILL` to every
    /// descendant, deepest first, then to the process and to its process
    /// group; on Windows every descendant started after the process, then
    /// the process, with exit code `-1`, as the reference's WinP does). Does
    /// nothing when the process already exited.
    ///
    /// Walks the process table: takes a few milliseconds.
    pub fn kill(&self) -> io::Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        sys::kill(&self.inner.os)
    }

    /// The reference's Stop action: the first call interrupts
    /// ([`ProcessHandle::interrupt`], or kills when no interrupt could be
    /// sent), any further call while the process still runs kills the tree.
    pub fn stop(&self) -> io::Result<()> {
        let first = {
            let mut state = self.inner.state();
            if state.exit.is_some() {
                return Ok(());
            }
            !std::mem::replace(&mut state.stop_requested, true)
        };
        if first && self.interrupt().is_ok() {
            return Ok(());
        }
        self.kill()
    }

    /// The process has not exited yet.
    pub fn is_running(&self) -> bool {
        self.inner.state().exit.is_none()
    }

    /// A stop was requested and the process still runs: the next
    /// [`ProcessHandle::stop`] kills (the reference then shows the Stop button
    /// as "Kill process").
    pub fn is_stopping(&self) -> bool {
        let state = self.inner.state();
        state.stop_requested && state.exit.is_none()
    }

    /// How the process ended, once it has. Available as soon as the process
    /// exited, possibly before [`ProcessEvent::Exited`] (which waits for the
    /// output to be drained).
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.inner.state().exit
    }
}

impl fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("pid", &self.inner.pid)
            .field("exit", &self.exit_status())
            .finish()
    }
}

fn broken_stdin() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "the process does not read its standard input",
    )
}

/// Output plumbing shared by the readers and the waiter.
struct Output {
    events: flume::Sender<ProcessEvent>,
    /// Set when `Exited` was sent; later output is dropped. Readers hold the
    /// lock while sending, so nothing is sent after `Exited`.
    closed: Mutex<bool>,
    drain: Mutex<Drain>,
    drained: Condvar,
    readers: [ReaderState; 2],
}

#[derive(Default)]
struct Drain {
    finished: [bool; 2],
}

#[derive(Default)]
struct ReaderState {
    /// The reader is blocked in `read` (the pipe was empty).
    in_read: AtomicBool,
}

impl Output {
    fn drain(&self) -> MutexGuard<'_, Drain> {
        self.drain.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, event: ProcessEvent) {
        let closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        if !*closed {
            // An error means the receiver is gone: keep draining the pipe so
            // the process does not block, and drop the output.
            let _ = self.events.send(event);
        }
    }

    fn close_with(&self, event: ProcessEvent) {
        let mut closed = self.closed.lock().unwrap_or_else(|e| e.into_inner());
        *closed = true;
        let _ = self.events.send(event);
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    output: &Arc<Output>,
    index: usize,
    mut pipe: R,
    event: fn(Vec<u8>) -> ProcessEvent,
) -> io::Result<()> {
    let output = output.clone();
    let name = if index == 0 { "stdout" } else { "stderr" };
    thread::Builder::new()
        .name(format!("process {name}"))
        .spawn(move || {
            let mut buf = vec![0u8; READ_CHUNK_SIZE];
            loop {
                output.readers[index].in_read.store(true, Ordering::Release);
                let read = pipe.read(&mut buf);
                output.readers[index]
                    .in_read
                    .store(false, Ordering::Release);
                match read {
                    Ok(0) => break,
                    Ok(n) => output.send(event(buf[..n].to_vec())),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            // Mark the end under the lock before the pipe is closed: the
            // waiter only queries the pipe of a reader that has not finished.
            let mut drain = output.drain();
            drain.finished[index] = true;
            output.drained.notify_all();
            drop(drain);
            drop(pipe);
        })?;
    Ok(())
}

fn spawn_writer(
    mut stdin: ChildStdin,
    queue: mpsc::Receiver<Vec<u8>>,
    broken: Arc<AtomicBool>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("process stdin".to_owned())
        .spawn(move || {
            for data in queue {
                if stdin.write_all(&data).and_then(|()| stdin.flush()).is_err() {
                    broken.store(true, Ordering::Release);
                    break;
                }
            }
        })?;
    Ok(())
}

fn wait(mut child: Child, inner: Arc<Inner>, output: Arc<Output>, pipes: [sys::Pipe; 2]) {
    let status = match child.wait() {
        Ok(status) => sys::exit_status(status),
        Err(_) => ExitStatus {
            code: Some(-1),
            signal: None,
        },
    };
    inner.state().exit = Some(status);

    // Wait until both streams ended. A descendant may keep a pipe open after
    // the process exited: once both open pipes stay empty for a moment, stop
    // waiting, as the reference stops reading when the process exits.
    let mut drain = output.drain();
    let mut empty_since: Option<Instant> = None;
    loop {
        if drain.finished.iter().all(|f| *f) {
            break;
        }
        let all_empty = (0..2).all(|i| {
            drain.finished[i]
                || (output.readers[i].in_read.load(Ordering::Acquire)
                    && sys::pipe_available(pipes[i]) == Some(0))
        });
        if all_empty {
            let since = *empty_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= ORPHAN_PIPE_GRACE {
                break;
            }
        } else {
            empty_since = None;
        }
        drain = output
            .drained
            .wait_timeout(drain, Duration::from_millis(10))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    drop(drain);
    output.close_with(ProcessEvent::Exited(status));
}
