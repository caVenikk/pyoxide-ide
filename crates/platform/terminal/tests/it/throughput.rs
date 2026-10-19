//! Throughput of a session (ADR 0014): a large output through the
//! pseudo-terminal (ConPTY on Windows) into the emulator, measured from the
//! spawn to the last line on the screen, while a view takes
//! a snapshot per frame; and the emulator alone for comparison. CI runs
//! both on every OS in its own step and prints the numbers:
//!
//! `cargo test -p terminal --test it throughput:: -- --ignored --nocapture`
//! (add `--release` for numbers comparable with `replica/perf.md`).

use std::io::Write;
use std::time::{Duration, Instant};

use terminal::start::{Os, environment};
use terminal::{Session, SessionEvent, SessionOptions, ShellCommand, TerminalSize};

const HELPER: &str = env!("CARGO_BIN_EXE_terminal-test-helper");
const BYTES: usize = 16 << 20;
/// A hang guard, not a budget: debug builds in CI are slow.
const LIMIT: Duration = Duration::from_secs(300);
/// The view's frame (60 Hz).
const FRAME: Duration = Duration::from_micros(16_667);
/// How often the run reports where it is, so a hang can be told from a
/// slow run in the CI log.
const PROGRESS: Duration = Duration::from_secs(5);

/// The number of the last numbered line on the screen (`line 00000042 ...`).
fn last_line_number(text: &str) -> Option<u64> {
    text.lines()
        .filter_map(|line| {
            let digits = line.trim_start().strip_prefix("line ")?.get(..8)?;
            digits.parse().ok()
        })
        .next_back()
}

fn build() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

#[test]
#[ignore = "measurement; CI runs it in its own step"]
fn bench_flood_through_the_pty() {
    let options = SessionOptions {
        command: ShellCommand {
            program: HELPER.into(),
            args: vec!["flood".into(), BYTES.to_string()],
        },
        working_directory: std::env::temp_dir(),
        env: environment(Os::current(), &[], &[]),
        size: TerminalSize::new(120, 40),
        scrollback_lines: 0,
    };
    let start = Instant::now();
    let (session, rx) = Session::spawn(options).expect("spawn the helper");
    let mut frames = 0usize;
    let mut snapshot = terminal::Snapshot::default();
    let mut snapshot_time = Duration::ZERO;
    let mut last_frame = Instant::now();
    let mut last_wakeup = Instant::now();
    let mut next_progress = start + PROGRESS;
    // The helper stays alive after its last line (ConPTY may drop the output
    // of a program that exits right after writing it), so the end of the run
    // is the last line on the screen.
    loop {
        let now = Instant::now();
        if now >= start + LIMIT {
            session.kill();
            session.snapshot_into(&mut snapshot);
            panic!(
                "no last line within {LIMIT:?}: {frames} frames, last wakeup {:?} ago, \
                 last numbered line {:?}; the screen:\n{}",
                last_wakeup.elapsed(),
                last_line_number(&snapshot.text()),
                snapshot.text(),
            );
        }
        match rx.recv_deadline(next_progress.min(start + LIMIT)) {
            // A view takes a snapshot after a wakeup, at most once per frame.
            Ok(SessionEvent::Wakeup) => {
                last_wakeup = Instant::now();
                std::thread::sleep(FRAME.saturating_sub(last_frame.elapsed()));
                frames += 1;
                let t = Instant::now();
                session.snapshot_into(&mut snapshot);
                snapshot_time += t.elapsed();
                last_frame = Instant::now();
                if snapshot.text().contains("flood done") {
                    break;
                }
            }
            Ok(SessionEvent::Exited(status)) => {
                panic!(
                    "the helper exited before the last line: {status:?}; the screen:\n{}",
                    snapshot.text()
                )
            }
            // Other events, or the time for a progress line.
            _ => {}
        }
        if Instant::now() >= next_progress {
            next_progress += PROGRESS;
            println!(
                "  {:>5.1} s: {frames} frames, last wakeup {:.1} s ago, last numbered line {:?}",
                start.elapsed().as_secs_f64(),
                last_wakeup.elapsed().as_secs_f64(),
                last_line_number(&snapshot.text()),
            );
        }
    }
    let elapsed = start.elapsed();
    session.write(b"\r".to_vec());
    let status = loop {
        match rx.recv_deadline(start + LIMIT) {
            Ok(SessionEvent::Exited(status)) => break status,
            Ok(_) => {}
            Err(e) => {
                session.kill();
                panic!("no exit within {LIMIT:?}: {e:?}");
            }
        }
    };
    assert!(status.success(), "{status:?}");
    // ConPTY may repaint the screen instead of scrolling it.
    if cfg!(unix) {
        assert_eq!(snapshot.history_size, terminal::DEFAULT_SCROLLBACK);
    }
    let mib = BYTES as f64 / (1 << 20) as f64;
    println!();
    println!(
        "terminal throughput ({}, {} build): {mib:.0} MiB in {:.2} s = {:.1} MiB/s; \
         {frames} frames, a snapshot of 120x40 in {:.1} us",
        std::env::consts::OS,
        build(),
        elapsed.as_secs_f64(),
        mib / elapsed.as_secs_f64(),
        snapshot_time.as_secs_f64() * 1e6 / frames.max(1) as f64,
    );
}

/// The emulator alone, without the PTY: the same lines parsed in chunks.
#[test]
#[ignore = "measurement; CI runs it in its own step"]
fn bench_emulator_alone() {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    let mut bytes = Vec::with_capacity(BYTES + 128);
    let mut n = 0u64;
    while bytes.len() < BYTES {
        n += 1;
        write!(
            bytes,
            "line {n:08} the quick brown fox jumps over the lazy dog 0123456789\r\n"
        )
        .unwrap();
    }
    let size = TerminalSize::new(120, 40);
    let mut term = Term::new(Config::default(), &size, VoidListener);
    let mut parser: Processor = Processor::new();
    let mut scanner = terminal::prescan::Scanner::default();
    let start = Instant::now();
    for chunk in bytes.chunks(64 << 10) {
        assert!(scanner.scan(chunk).is_none());
        parser.advance(&mut term, chunk);
    }
    let elapsed = start.elapsed();
    let mib = bytes.len() as f64 / (1 << 20) as f64;
    println!();
    println!(
        "emulator alone ({}, {} build): {mib:.0} MiB in {:.2} s = {:.1} MiB/s",
        std::env::consts::OS,
        build(),
        elapsed.as_secs_f64(),
        mib / elapsed.as_secs_f64(),
    );
}
