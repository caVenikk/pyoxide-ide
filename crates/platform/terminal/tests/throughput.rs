//! Throughput of a session (ADR 0014): a large output through the
//! pseudo-terminal (ConPTY on Windows) into the emulator, measured from the
//! spawn to the exit with the last line on the screen, while a view takes
//! a snapshot per frame; and the emulator alone for comparison. CI runs
//! both on every OS in its own step and prints the numbers:
//!
//! `cargo test -p terminal --test throughput -- --ignored --nocapture`
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
    let status = loop {
        match rx.recv_deadline(start + LIMIT) {
            Ok(SessionEvent::Exited(status)) => break status,
            // A view takes a snapshot after a wakeup, at most once per frame.
            Ok(SessionEvent::Wakeup) => {
                std::thread::sleep(FRAME.saturating_sub(last_frame.elapsed()));
                frames += 1;
                let t = Instant::now();
                session.snapshot_into(&mut snapshot);
                snapshot_time += t.elapsed();
                last_frame = Instant::now();
            }
            Ok(_) => {}
            Err(e) => {
                session.kill();
                panic!("no exit within {LIMIT:?}: {e:?}");
            }
        }
    };
    let elapsed = start.elapsed();
    session.snapshot_into(&mut snapshot);
    assert!(status.success(), "{status:?}");
    assert!(
        snapshot.text().contains("flood done"),
        "{}",
        snapshot.text()
    );
    assert_eq!(snapshot.history_size, terminal::DEFAULT_SCROLLBACK);
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
