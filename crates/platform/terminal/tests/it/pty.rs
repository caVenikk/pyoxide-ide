//! Sessions over a real pseudo-terminal (openpty, ConPTY on Windows)
//! against a small helper program built with the crate
//! (`src/bin/terminal-test-helper.rs`), the same on every OS. Every test
//! waits for the helper to exit, or kills it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use terminal::start::{Os, environment};
use terminal::{
    ExitStatus, Point, SearchDirection, SearchQuery, SelectionKind, Session, SessionEvent,
    SessionOptions, ShellCommand, Side, Snapshot, TerminalSize,
};

const HELPER: &str = env!("CARGO_BIN_EXE_terminal-test-helper");
const TIMEOUT: Duration = Duration::from_secs(30);

struct Harness {
    session: Option<Session>,
    rx: flume::Receiver<SessionEvent>,
    events: Vec<SessionEvent>,
}

fn options(args: &[&str], size: TerminalSize) -> SessionOptions {
    SessionOptions {
        command: ShellCommand {
            program: HELPER.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        },
        working_directory: std::env::temp_dir(),
        env: environment(Os::current(), &[], &[]),
        size,
        scrollback_lines: 0,
    }
}

fn start(options: SessionOptions) -> Harness {
    let (session, rx) = Session::spawn(options).expect("spawn the helper");
    Harness {
        session: Some(session),
        rx,
        events: Vec::new(),
    }
}

fn spawn(args: &[&str]) -> Harness {
    start(options(args, TerminalSize::new(120, 24)))
}

impl Harness {
    fn session(&self) -> &Session {
        self.session.as_ref().expect("session")
    }

    /// Takes snapshots on every event until `done` holds.
    fn wait(&mut self, what: &str, done: impl Fn(&Snapshot, &[SessionEvent]) -> bool) -> Snapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let snapshot = self.session().snapshot();
            if done(&snapshot, &self.events) {
                return snapshot;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(event) => self.events.push(event),
                Err(flume::RecvTimeoutError::Timeout) if !left.is_zero() => {}
                Err(e) => panic!(
                    "waiting for {what}: {e:?}; screen:\n{}\nevents: {:?}",
                    snapshot.text(),
                    self.events
                ),
            }
        }
    }

    fn wait_text(&mut self, needle: &str) -> Snapshot {
        self.wait(needle, |s, _| s.text().contains(needle))
    }

    fn wait_exit(&mut self) -> ExitStatus {
        self.wait("the exit", |_, events| {
            events.iter().any(|e| matches!(e, SessionEvent::Exited(_)))
        });
        match self.events.last() {
            Some(SessionEvent::Exited(status)) => *status,
            other => panic!("Exited is not the last event: {other:?}"),
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // A failed test still ends its helper.
        if let Some(session) = &self.session
            && session.exit_status().is_none()
        {
            session.kill();
        }
    }
}

/// The visible line holding `needle`.
fn line_with(snapshot: &Snapshot, needle: &str) -> usize {
    (0..snapshot.rows)
        .find(|&l| snapshot.line_text(l).contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle:?} in\n{}", snapshot.text()))
}

#[test]
fn echo_round_trip() {
    let mut h = spawn(&["echo"]);
    h.session().write(b"hello\r".to_vec());
    h.wait_text("got hello");
    h.session().write_text("exit\r");
    let status = h.wait_exit();
    assert_eq!(status.code, Some(0), "{status:?}");
    assert_eq!(h.session().exit_status(), Some(status));
}

#[test]
fn resize_reaches_the_program() {
    let mut h = start(options(&["size"], TerminalSize::new(80, 24)));
    h.wait_text("size 80x24");
    h.session().resize(TerminalSize::new(100, 30));
    h.session().write(b"\r".to_vec());
    let snapshot = h.wait_text("size 100x30");
    assert_eq!((snapshot.columns, snapshot.rows), (100, 30));
    assert_eq!(snapshot.cells.len(), 100 * 30);
    h.session().write(b"exit\r".to_vec());
    h.wait_exit();
}

#[test]
fn exit_code_is_reported() {
    let mut h = spawn(&["exit", "7"]);
    let status = h.wait_exit();
    assert_eq!(status.code, Some(7), "{status:?}");
    assert_eq!(status.exit_code(), 7);
}

#[test]
fn bracketed_paste() {
    let mut h = spawn(&["paste"]);
    h.wait_text("ready");
    // Unix PTYs pass the mode through as the program wrote it; the system
    // ConPTY may keep it to itself.
    if cfg!(unix) {
        h.wait("bracketed paste mode", |s, _| s.modes.bracketed_paste);
    }
    h.session().paste("one\ntwo");
    h.session().write(vec![0x04]);
    let snapshot = h.wait_text("pasted:");
    let line = snapshot.line_text(line_with(&snapshot, "pasted:"));
    if cfg!(unix) {
        assert_eq!(line, "pasted: <ESC>[200~one<CR>two<ESC>[201~");
    } else {
        assert!(line.contains("one<CR>two"), "{line}");
    }
    // The helper stays alive until this byte: ConPTY may drop the output of
    // a program that exits right after writing it.
    h.session().write(b"\r".to_vec());
    h.wait_exit();
}

#[test]
fn environment_and_working_directory() {
    let dir = std::env::temp_dir().join(format!("pyoxide-terminal-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut opts = options(
        &[
            "env",
            "TERM",
            "COLORTERM",
            "TERMINAL_EMULATOR",
            "ALACRITTY_WINDOW_ID",
            "WINDOWID",
            "PYOXIDE_TERMINAL_TEST",
        ],
        TerminalSize::new(200, 24),
    );
    opts.env
        .push(("PYOXIDE_TERMINAL_TEST".into(), "yes".into()));
    let mut h = start(opts);
    let snapshot = h.wait_text("PYOXIDE_TERMINAL_TEST=");
    let text = snapshot.text();
    assert!(text.contains("PYOXIDE_TERMINAL_TEST=yes"), "{text}");
    assert!(text.contains("COLORTERM=truecolor"), "{text}");
    assert!(text.contains("TERMINAL_EMULATOR=Pyoxide"), "{text}");
    // Not Alacritty's window id (`tty` sets it on Unix; the session
    // removes it).
    assert!(text.contains("ALACRITTY_WINDOW_ID=<unset>"), "{text}");
    assert!(text.contains("WINDOWID=<unset>"), "{text}");
    if cfg!(unix) {
        assert!(text.contains("TERM=xterm-256color"), "{text}");
    }
    h.session().write(b"\r".to_vec());
    h.wait_exit();

    let mut opts = options(&["cwd"], TerminalSize::new(200, 24));
    opts.working_directory = dir.clone();
    let mut h = start(opts);
    let snapshot = h.wait_text("cwd ");
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let line = snapshot.line_text(line_with(&snapshot, "cwd "));
    assert!(line.ends_with(&name), "{line}");
    h.session().write(b"\r".to_vec());
    h.wait_exit();
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn missing_working_directory_is_an_error() {
    let mut opts = options(&["exit"], TerminalSize::default());
    opts.working_directory = PathBuf::from("/pyoxide/no/such/folder");
    let err = Session::spawn(opts).unwrap_err();
    assert!(
        matches!(err, terminal::SpawnError::WorkingDirectory(_)),
        "{err}"
    );
}

/// The system ConPTY of older Windows releases drops OSC sequences it does
/// not know; the scanner itself is tested on every OS (`prescan`).
#[cfg(unix)]
#[test]
fn shell_integration_sequences_become_events() {
    use terminal::MarkKind;

    let mut h = spawn(&["osc"]);
    h.wait_text("done");
    while let Ok(event) = h.rx.try_recv() {
        h.events.push(event);
    }
    let marks: Vec<(MarkKind, usize)> = h
        .events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Mark(m) => Some((m.kind, m.cursor_line)),
            _ => None,
        })
        .collect();
    assert_eq!(
        marks,
        [
            (MarkKind::PromptStart, 0),
            (MarkKind::CommandStart, 0),
            (MarkKind::CommandExecuted, 0),
            (MarkKind::CommandFinished { exit_code: Some(3) }, 1),
        ]
    );
    let cwd = PathBuf::from("/tmp/a b");
    assert!(
        h.events
            .contains(&SessionEvent::WorkingDirectory(cwd.clone()))
    );
    assert!(
        h.events
            .contains(&SessionEvent::Integration("hello".into()))
    );
    assert!(
        h.events
            .contains(&SessionEvent::Title(Some("helper title".into())))
    );
    assert_eq!(h.session().working_directory(), Some(cwd));
    assert_eq!(h.session().title().as_deref(), Some("helper title"));
    h.session().write(b"\r".to_vec());
    h.wait_exit();
}

#[test]
fn kill_ends_the_shell() {
    let mut h = spawn(&["sleep"]);
    h.wait_text("ready");
    h.session().kill();
    let status = h.wait_exit();
    if cfg!(unix) {
        assert_eq!(status.signal, Some(9), "{status:?}");
    } else {
        assert_eq!(status.code, Some(1), "{status:?}");
    }
}

#[test]
fn dropping_the_session_ends_the_shell() {
    let mut h = spawn(&["sleep"]);
    h.wait_text("ready");
    drop(h.session.take());
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match h.rx.recv_deadline(deadline) {
            Ok(SessionEvent::Exited(status)) => break status,
            Ok(_) => {}
            Err(e) => panic!("no exit after drop: {e:?}"),
        }
    };
    assert!(!status.success(), "{status:?}");
}

#[test]
fn selection_search_and_clear() {
    let mut h = spawn(&["echo"]);
    h.session().write(b"foo Foo FOO\r".to_vec());
    let snapshot = h.wait_text("got foo Foo FOO");
    let got = line_with(&snapshot, "got foo");

    h.session()
        .start_selection(SelectionKind::Simple, Point::new(got, 4), Side::Left);
    h.session()
        .update_selection(Point::new(got, 6), Side::Right);
    assert_eq!(h.session().selection_text().as_deref(), Some("foo"));
    let snapshot = h.session().snapshot();
    let selection = snapshot.selection.expect("selection in the snapshot");
    assert_eq!(
        (selection.start, selection.end),
        (Point::new(got, 4), Point::new(got, 6))
    );
    h.session()
        .start_selection(SelectionKind::Word, Point::new(got, 9), Side::Left);
    assert_eq!(h.session().selection_text().as_deref(), Some("Foo"));
    h.session().select_all();
    let all = h.session().selection_text().unwrap();
    assert!(all.contains("got foo Foo FOO"), "{all}");
    h.session().clear_selection();
    assert_eq!(h.session().selection_text(), None);

    // The typed line (echoed by the terminal) and the program's line.
    let query = |text: &str, match_case, regex| SearchQuery {
        text: text.into(),
        match_case,
        regex,
    };
    h.session()
        .set_search(Some(&query("foo", false, false)))
        .unwrap();
    assert_eq!(h.session().count_matches(100).total, 6);
    let snapshot = h.session().snapshot();
    assert_eq!(snapshot.matches.len(), 6);
    let last = h.session().find(SearchDirection::Previous).unwrap();
    assert_eq!(last.start, Point::new(got, 12));
    assert_eq!(h.session().count_matches(100).current, Some(5));
    let first = h.session().find(SearchDirection::Next).unwrap();
    assert!(first.start.line < got, "{first:?} wraps to the first match");
    assert_eq!(h.session().snapshot().current_match, Some(first));
    h.session()
        .set_search(Some(&query("foo", true, false)))
        .unwrap();
    assert_eq!(h.session().count_matches(100).total, 2);
    h.session()
        .set_search(Some(&query("F.O", true, true)))
        .unwrap();
    assert_eq!(h.session().count_matches(100).total, 2);
    assert!(
        h.session()
            .set_search(Some(&query("(", false, true)))
            .is_err()
    );
    h.session().set_search(None).unwrap();
    assert!(h.session().snapshot().matches.is_empty());

    h.session().clear();
    let snapshot = h.session().snapshot();
    assert_eq!(snapshot.history_size, 0);
    assert!(!snapshot.text().contains("foo"), "{}", snapshot.text());
    assert_eq!(snapshot.cursor.map(|c| c.point.line), Some(0));

    h.session().write(b"exit\r".to_vec());
    h.wait_exit();
}
