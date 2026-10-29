//! The I/O loop of a session, one thread each (ADR 0014): waits on the
//! pseudo-terminal, reads the output, pre-scans it and feeds it to the
//! emulator under the terminal's lock, writes the input, resizes, stops
//! the shell and reports its exit.
//!
//! The reading and writing follow `alacritty_terminal`'s own event loop
//! (`event_loop.rs`, Apache-2.0, see NOTICE), which is not used: reads are
//! parsed in batches while the lock is held, the lock is taken unfairly
//! by the reader and fairly by the view, and a full read buffer forces it.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite};
use alacritty_terminal::vte::ansi;
use parking_lot::MutexGuard;
use polling::{Event as PollEvent, Events, PollMode, Poller};
use process::ExitStatus;

use crate::prescan::{Scanner, Sequence};
use crate::session::{SessionEvent, Shared, ShellMark, State};

/// The most bytes read before the parser must catch up.
const READ_BUFFER_SIZE: usize = 0x10_0000;
/// The most bytes parsed while holding the lock at once.
const MAX_LOCKED_READ: usize = u16::MAX as usize;
/// How long a hung-up shell may take to exit when the session is dropped
/// before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(1500);

/// The poller key `alacritty_terminal::tty` gives the child's exit (the
/// same on every OS); every other key is the PTY itself.
const CHILD_EVENT_KEY: usize = 1;

pub(crate) enum Msg {
    Input(Cow<'static, [u8]>),
    Resize(WindowSize),
    Terminate,
    Kill,
    /// The session was dropped: hang up, kill after a grace period, stop.
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct LoopSender {
    tx: Sender<Msg>,
    poller: Arc<Poller>,
}

impl LoopSender {
    pub(crate) fn send(&self, msg: Msg) {
        if self.tx.send(msg).is_ok() {
            let _ = self.poller.notify();
        }
    }
}

pub(crate) fn channel() -> io::Result<(LoopSender, Receiver<Msg>)> {
    let (tx, rx) = mpsc::channel();
    let poller = Arc::new(Poller::new()?);
    Ok((LoopSender { tx, poller }, rx))
}

pub(crate) fn child_pid(pty: &tty::Pty) -> u32 {
    #[cfg(unix)]
    {
        pty.child().id()
    }
    #[cfg(windows)]
    {
        pty.child_watcher().pid().map_or(0, |p| p.get())
    }
}

/// The shell's process handle.
#[cfg(windows)]
pub(crate) fn child_handle(pty: &tty::Pty) -> windows_sys::Win32::Foundation::HANDLE {
    pty.child_watcher().raw_handle()
}

/// Starts the loop thread; it owns the PTY and drops it at the end.
pub(crate) fn start(
    pty: tty::Pty,
    shared: Arc<Shared>,
    sender: LoopSender,
    rx: Receiver<Msg>,
    id: u64,
) -> io::Result<()> {
    let poller = sender.poller.clone();
    let mut io = IoLoop {
        pty,
        poller,
        rx,
        shared,
        parser: ansi::Processor::new(),
        scanner: Scanner::default(),
        writes: VecDeque::new(),
        writing: None,
        shutdown_deadline: None,
        killed: false,
        read_again: false,
    };
    std::thread::Builder::new()
        .name(format!("terminal-{id}"))
        .spawn(move || io.run())
        .map(|_| ())
}

struct IoLoop {
    pty: tty::Pty,
    poller: Arc<Poller>,
    rx: Receiver<Msg>,
    shared: Arc<Shared>,
    parser: ansi::Processor,
    scanner: Scanner,
    writes: VecDeque<Cow<'static, [u8]>>,
    /// The chunk being written and how much of it was.
    writing: Option<(Cow<'static, [u8]>, usize)>,
    shutdown_deadline: Option<Instant>,
    killed: bool,
    /// The last read stopped at [`MAX_LOCKED_READ`] without seeing the PTY
    /// empty, so more output may be waiting that no poller event will
    /// announce: the Windows reader only posts an event after a read found
    /// the pipe empty. Level-triggered polling on Unix reports it anyway.
    read_again: bool,
}

impl IoLoop {
    fn run(&mut self) {
        let mut buf = vec![0u8; READ_BUFFER_SIZE];
        let mut interest = PollEvent::readable(0);
        let mode = PollMode::Level;
        // SAFETY: the PTY outlives its registration: it is deregistered
        // below before it is dropped with `self`.
        if unsafe { self.pty.register(&self.poller, interest, mode) }.is_err() {
            self.finish(None);
            return;
        }
        let mut events = Events::with_capacity(NonZeroUsize::new(64).unwrap_or(NonZeroUsize::MIN));
        let status = 'run: loop {
            let sync = self.parser.sync_timeout().sync_timeout();
            let timeout = if self.read_again {
                Some(Duration::ZERO)
            } else {
                [sync, self.shutdown_deadline]
                    .into_iter()
                    .flatten()
                    .min()
                    .map(|at| at.saturating_duration_since(Instant::now()))
            };
            events.clear();
            if let Err(e) = self.poller.wait(&mut events, timeout) {
                if e.kind() != ErrorKind::Interrupted {
                    break 'run None;
                }
                continue;
            }
            // A synchronized update (mode 2026) that timed out.
            if let Some(at) = sync
                && Instant::now() >= at
            {
                self.parser.stop_sync(&mut self.shared.state.lock().term);
                self.shared.notifier.wake();
            }
            if let Some(at) = self.shutdown_deadline
                && Instant::now() >= at
            {
                if self.killed {
                    // Killed and still no exit event: give up waiting.
                    break 'run None;
                }
                self.kill();
                self.shutdown_deadline = Some(Instant::now() + SHUTDOWN_GRACE);
            }
            loop {
                match self.rx.try_recv() {
                    Ok(msg) => self.handle(msg),
                    Err(TryRecvError::Empty) => break,
                    // The session is gone and so are its senders: stop.
                    Err(TryRecvError::Disconnected) => {
                        if self.shutdown_deadline.is_none() {
                            self.handle(Msg::Shutdown);
                        }
                        break;
                    }
                }
            }
            let mut read_due = std::mem::take(&mut self.read_again);
            for event in events.iter() {
                if event.key == CHILD_EVENT_KEY {
                    if let Some(ChildEvent::Exited(status)) = self.pty.next_child_event() {
                        // Output written just before the exit.
                        while matches!(self.read(&mut buf), Ok(false)) {}
                        break 'run Some(status);
                    }
                    continue;
                }
                read_due |= event.readable;
                if event.writable && self.write().is_err() {
                    break 'run None;
                }
            }
            if read_due {
                match self.read(&mut buf) {
                    Ok(drained) => self.read_again = !drained,
                    // Linux reports EIO once the shell's side is closed;
                    // the exit event follows.
                    #[cfg(target_os = "linux")]
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => {}
                    Err(_) => break 'run None,
                }
            }
            let wants_write = self.writing.is_some() || !self.writes.is_empty();
            if wants_write != interest.writable {
                interest.writable = wants_write;
                if self.pty.reregister(&self.poller, interest, mode).is_err() {
                    break 'run None;
                }
            }
        };
        let _ = self.pty.deregister(&self.poller);
        self.finish(status.flatten());
    }

    /// Reports the exit (`None`: the status is unknown).
    fn finish(&mut self, status: Option<std::process::ExitStatus>) {
        let status = status.map_or(
            ExitStatus {
                code: None,
                signal: None,
            },
            exit_status,
        );
        self.shared.state.lock().term.exit();
        self.shared.notifier.info().exit = Some(status);
        self.shared
            .notifier
            .exited
            .store(true, std::sync::atomic::Ordering::Release);
        self.shared.notifier.wake();
        self.shared.notifier.send(SessionEvent::Exited(status));
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Input(bytes) => self.writes.push_back(bytes),
            Msg::Resize(size) => self.resize(size),
            Msg::Terminate => self.terminate(),
            Msg::Kill => self.kill(),
            Msg::Shutdown => {
                if self.shutdown_deadline.is_none() {
                    self.terminate();
                    self.shutdown_deadline = Some(Instant::now() + SHUTDOWN_GRACE);
                }
            }
        }
    }

    /// Reads what is available and parses it, at most [`MAX_LOCKED_READ`]
    /// bytes per lock. Returns whether the PTY was seen empty (`false`: the
    /// cap was hit, call again).
    fn read(&mut self, buf: &mut [u8]) -> io::Result<bool> {
        let mut unprocessed = 0;
        let mut processed = 0;
        let mut drained = true;
        let shared = self.shared.clone();
        // Reserve the next turn of the lock for the reader.
        let lease = shared.state.lease();
        let mut state = None;
        loop {
            match self.pty.reader().read(&mut buf[unprocessed..]) {
                Ok(0) if unprocessed == 0 => break,
                Ok(n) => unprocessed += n,
                Err(e) => match e.kind() {
                    ErrorKind::Interrupted | ErrorKind::WouldBlock => {
                        if unprocessed == 0 {
                            break;
                        }
                    }
                    _ => return Err(e),
                },
            }
            let state = match &mut state {
                Some(state) => state,
                None => state.insert(match shared.state.try_lock_unfair() {
                    None if unprocessed >= buf.len() => shared.state.lock_unfair(),
                    None => continue,
                    Some(state) => state,
                }),
            };
            self.feed(state, &buf[..unprocessed]);
            processed += unprocessed;
            unprocessed = 0;
            if processed >= MAX_LOCKED_READ {
                drained = false;
                break;
            }
        }
        // Hand the lock to a waiting view right away: with a plain unlock
        // the reader takes it again before the view thread wakes up.
        if let Some(state) = state {
            MutexGuard::unlock_fair(state);
        }
        MutexGuard::unlock_fair(lease);
        if processed > 0 && self.parser.sync_bytes_count() < processed {
            shared.notifier.wake();
        }
        Ok(drained)
    }

    /// Parses `bytes`, stopping after each recognised OSC sequence to note
    /// where the cursor is.
    fn feed(&mut self, state: &mut State, mut bytes: &[u8]) {
        while let Some((end, sequence)) = self.scanner.scan(bytes) {
            self.parser.advance(&mut state.term, &bytes[..end]);
            bytes = &bytes[end..];
            let notifier = &self.shared.notifier;
            match sequence {
                Sequence::WorkingDirectory(path) => {
                    notifier.info().working_directory = Some(path.clone());
                    notifier.send(SessionEvent::WorkingDirectory(path));
                }
                Sequence::Mark(kind) => {
                    let cursor = state.term.grid().cursor.point;
                    notifier.send(SessionEvent::Mark(ShellMark {
                        kind,
                        cursor_line: cursor.line.0.max(0) as usize,
                        history_size: state.term.history_size(),
                    }));
                }
                Sequence::Own(payload) => notifier.send(SessionEvent::Integration(payload)),
                Sequence::IconName(title) => {
                    notifier.info().title = Some(title.clone());
                    notifier.send(SessionEvent::Title(Some(title)));
                }
            }
        }
        self.parser.advance(&mut state.term, bytes);
    }

    fn write(&mut self) -> io::Result<()> {
        loop {
            let (chunk, mut done) = match self.writing.take() {
                Some(current) => current,
                None => match self.writes.pop_front() {
                    Some(next) => (next, 0),
                    None => return Ok(()),
                },
            };
            while done < chunk.len() {
                match self.pty.writer().write(&chunk[done..]) {
                    Ok(0) => {
                        self.writing = Some((chunk, done));
                        return Ok(());
                    }
                    Ok(n) => done += n,
                    Err(e) => {
                        self.writing = Some((chunk, done));
                        return match e.kind() {
                            ErrorKind::Interrupted | ErrorKind::WouldBlock => Ok(()),
                            _ => Err(e),
                        };
                    }
                }
            }
        }
    }

    fn resize(&mut self, size: WindowSize) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let ws = libc::winsize {
                ws_row: size.num_lines,
                ws_col: size.num_cols,
                ws_xpixel: size.num_cols.saturating_mul(size.cell_width),
                ws_ypixel: size.num_lines.saturating_mul(size.cell_height),
            };
            // The crate's own resize exits the process when the call fails.
            // SAFETY: a valid descriptor and winsize.
            unsafe { libc::ioctl(self.pty.file().as_raw_fd(), libc::TIOCSWINSZ, &ws) };
        }
        #[cfg(windows)]
        {
            use alacritty_terminal::event::OnResize;
            // The crate asserts that ResizePseudoConsole succeeds.
            let pty = &mut self.pty;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pty.on_resize(size)));
        }
    }

    fn terminate(&mut self) {
        #[cfg(unix)]
        // SAFETY: a signal to our own child.
        unsafe {
            libc::kill(self.pty.child().id() as libc::pid_t, libc::SIGHUP);
        }
        #[cfg(windows)]
        self.kill();
    }

    fn kill(&mut self) {
        self.killed = true;
        #[cfg(unix)]
        // SAFETY: a signal to our own child.
        unsafe {
            libc::kill(self.pty.child().id() as libc::pid_t, libc::SIGKILL);
        }
        #[cfg(windows)]
        // SAFETY: the handle stays valid while the PTY lives.
        unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(
                self.pty.child_watcher().raw_handle(),
                1,
            );
        }
    }
}

fn exit_status(status: std::process::ExitStatus) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus {
            code: status.code(),
            signal: status.signal(),
        }
    }
    #[cfg(windows)]
    {
        ExitStatus {
            code: status.code(),
            signal: None,
        }
    }
}
