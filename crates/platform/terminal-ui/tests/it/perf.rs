//! Headless measurements of the terminal view (ADR 0014, #157), without a
//! window: what a frame costs before painting (the snapshot and the plan
//! of backgrounds, text runs and the cursor), `cat` of a large file
//! through the pseudo-terminal with a snapshot and a plan per frame, the
//! frame thread's work while `cat` floods the screen with the copy in the
//! frame and on a background thread, and the time from a key to the
//! snapshot that shows its echo.
//!
//! `cargo test --release -p terminal-ui --test it perf:: -- --ignored --nocapture --test-threads=1`
//!
//! Numbers go to `replica/perf.md`; the window measurements (key to
//! paint, scrolling) need a visible window and run separately.

#[cfg(unix)]
use std::io::Write as _;
#[cfg(unix)]
use std::time::Duration;
use std::time::Instant;

#[cfg(unix)]
use terminal::start::{Os, environment};
use terminal::{Cell, CellFlags, Color, Cursor, CursorShape, Modes, Point, Rgb, Snapshot};
#[cfg(unix)]
use terminal::{Screen, Session, SessionEvent, SessionOptions, ShellCommand, TerminalSize};
use terminal_ui::colors::{Resolver, TerminalColors};
use terminal_ui::plan::{PlanOptions, plan};
#[cfg(unix)]
use terminal_ui::{FLOOD_AFTER, FLOOD_FRAME, FRAME as COPY_FRAME};

// The PTY measurements run on Unix only (they drive `cat`).
#[cfg(unix)]
const FRAME: Duration = Duration::from_micros(16_667);
#[cfg(unix)]
const LIMIT: Duration = Duration::from_secs(300);

fn build() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn colors() -> TerminalColors {
    TerminalColors::from_theme(&themes::builtin::default_theme(themes::ThemeKind::Dark))
}

fn options() -> PlanOptions {
    PlanOptions {
        focused: true,
        blink_on: true,
    }
}

/// A screen like a colourful listing: words in six colours, bold names,
/// a background run per line, a wide character every few lines.
fn busy_screen(columns: usize, rows: usize) -> Snapshot {
    let words = ["drwxr-xr-x", "src", "main.py", "README.md", "12:30", "4096"];
    let mut cells = Vec::with_capacity(columns * rows);
    for line in 0..rows {
        let mut row = Vec::with_capacity(columns);
        let mut w = line;
        while row.len() < columns {
            let word = words[w % words.len()];
            let fg = Color::Indexed((w % 6 + 1) as u8);
            let flags = if w % 4 == 0 {
                CellFlags::BOLD
            } else {
                CellFlags::empty()
            };
            for c in word.chars() {
                row.push(Cell {
                    c,
                    fg,
                    bg: if w % 9 == 0 {
                        Color::Rgb(Rgb::new(30, 40, 50))
                    } else {
                        Color::Background
                    },
                    flags,
                });
            }
            row.push(Cell::default());
            w += 1;
        }
        row.truncate(columns);
        if line % 5 == 0 && columns > 4 {
            row[2] = Cell {
                c: '中',
                flags: CellFlags::WIDE,
                ..Cell::default()
            };
            row[3] = Cell {
                flags: CellFlags::WIDE_SPACER,
                ..Cell::default()
            };
        }
        cells.extend(row);
    }
    Snapshot {
        columns,
        rows,
        cells,
        cursor: Some(Cursor {
            point: Point::new(rows - 1, 2),
            shape: CursorShape::Block,
            blinking: true,
        }),
        modes: Modes {
            show_cursor: true,
            ..Modes::default()
        },
        ..Snapshot::default()
    }
}

#[test]
#[ignore = "measurement"]
fn bench_plan_of_a_frame() {
    let colors = colors();
    for (columns, rows) in [(120, 40), (240, 70)] {
        let snapshot = busy_screen(columns, rows);
        let mut resolver = Resolver::default();
        let mut fits = |_: char| true;
        // Warm the contrast cache, as a running view has it.
        let first = plan(&snapshot, &colors, &mut resolver, options(), &mut fits);
        let runs = first.runs.len();
        let quads = first.backgrounds.len();
        let n = 2000;
        let start = Instant::now();
        for _ in 0..n {
            std::hint::black_box(plan(
                &snapshot,
                &colors,
                &mut resolver,
                options(),
                &mut fits,
            ));
        }
        let per = start.elapsed() / n;
        println!(
            "plan of a {columns}x{rows} frame ({} build): {:.1} us ({runs} text runs, {quads} background quads)",
            build(),
            per.as_secs_f64() * 1e6,
        );
    }
}

#[cfg(unix)]
fn spawn(
    program: &str,
    args: &[&str],
    columns: u16,
    rows: u16,
) -> (Session, flume::Receiver<SessionEvent>) {
    Session::spawn(SessionOptions {
        command: ShellCommand {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        },
        working_directory: std::env::temp_dir(),
        env: environment(Os::current(), &[], &[]),
        size: TerminalSize::new(columns, rows),
        scrollback_lines: 0,
    })
    .expect("spawn")
}

/// A file of about `bytes` of numbered lines with SGR colours that ends
/// with `end`, in a folder of its own (removed by the caller).
#[cfg(unix)]
fn large_file(bytes: usize, end: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "pyoxide-terminal-ui-perf-{}-{bytes}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("large.txt");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let mut written = 0;
    let mut n = 0u64;
    while written < bytes {
        let line = format!(
            "line {n:08} \x1b[32mthe quick brown fox\x1b[0m jumps over the \x1b[1;31mlazy dog\x1b[0m 0123456789\n"
        );
        written += line.len();
        f.write_all(line.as_bytes()).unwrap();
        n += 1;
    }
    writeln!(f, "{end}").unwrap();
    f.flush().unwrap();
    (dir, path)
}

/// `cat` of a large file: from the spawn to the last line on the screen,
/// taking a snapshot and planning a frame after each wakeup, at most once
/// per frame, as the view does.
#[test]
#[ignore = "measurement"]
#[cfg(unix)]
fn bench_cat_a_large_file() {
    const BYTES: usize = 16 << 20;
    let (dir, path) = large_file(BYTES, "cat done");
    let colors = colors();
    let mut resolver = Resolver::default();
    let mut fits = |_: char| true;
    let mut snapshot = Snapshot::default();
    let start = Instant::now();
    let (session, rx) = spawn("/bin/cat", &[path.to_str().unwrap()], 120, 40);
    let (mut frames, mut snap_time, mut plan_time) = (0u32, Duration::ZERO, Duration::ZERO);
    let mut last_frame = Instant::now();
    let mut exited = false;
    loop {
        assert!(start.elapsed() < LIMIT, "no end within {LIMIT:?}");
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(SessionEvent::Wakeup) => {
                std::thread::sleep(FRAME.saturating_sub(last_frame.elapsed()));
            }
            Ok(SessionEvent::Exited(_)) => exited = true,
            Ok(_) => continue,
            Err(_) if exited => {}
            Err(e) => panic!("{e:?}"),
        }
        let t = Instant::now();
        session.snapshot_into(&mut snapshot);
        let t2 = Instant::now();
        std::hint::black_box(plan(
            &snapshot,
            &colors,
            &mut resolver,
            options(),
            &mut fits,
        ));
        snap_time += t2 - t;
        plan_time += t2.elapsed();
        frames += 1;
        last_frame = Instant::now();
        if snapshot.text().contains("cat done") {
            break;
        }
    }
    let elapsed = start.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    let mib = BYTES as f64 / (1 << 20) as f64;
    println!(
        "cat of {mib:.0} MiB ({} build): {:.2} s = {:.1} MiB/s; {frames} frames, snapshot {:.1} us + plan {:.1} us per frame",
        build(),
        elapsed.as_secs_f64(),
        mib / elapsed.as_secs_f64(),
        snap_time.as_secs_f64() * 1e6 / f64::from(frames.max(1)),
        plan_time.as_secs_f64() * 1e6 / f64::from(frames.max(1)),
    );
}

/// The frame thread's work while `cat` floods the screen: a loop that
/// draws 60 frames a second takes the screen and plans a frame when it
/// changed, once copying the screen itself under the terminal's lock (which
/// the reader holds while it parses a batch of output), once taking the
/// copies a background thread made at the view's rates (one per frame,
/// one per `FLOOD_FRAME` after `FLOOD_AFTER` of output).
#[test]
#[ignore = "measurement"]
#[cfg(unix)]
fn bench_flood_frame_work() {
    const BYTES: usize = 128 << 20;
    let (dir, path) = large_file(BYTES, "flood done");
    let mib = BYTES as f64 / (1 << 20) as f64;
    for background in [false, true] {
        let colors = colors();
        let mut resolver = Resolver::default();
        let mut fits = |_: char| true;
        let mut shown = Snapshot::default();
        let start = Instant::now();
        let (session, rx) = spawn("/bin/cat", &[path.to_str().unwrap()], 120, 40);
        let (copy_tx, copy_rx) = flume::bounded::<Snapshot>(1);
        let (spare_tx, spare_rx) = flume::unbounded::<Snapshot>();
        if background {
            let (screen, events) = (session.screen(), rx.clone());
            std::thread::spawn(move || copier(screen, events, copy_tx, spare_rx));
        }
        let mut work = Vec::new();
        let mut next = Instant::now();
        loop {
            assert!(start.elapsed() < LIMIT, "no end within {LIMIT:?}");
            next += FRAME;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            let t = Instant::now();
            let changed = if background {
                copy_rx.try_recv().is_ok_and(|copy| {
                    let _ = spare_tx.send(std::mem::replace(&mut shown, copy));
                    true
                })
            } else {
                let woke = rx
                    .try_iter()
                    .any(|e| matches!(e, SessionEvent::Wakeup | SessionEvent::Exited(_)));
                if woke {
                    session.snapshot_into(&mut shown);
                }
                woke
            };
            if !changed {
                continue;
            }
            std::hint::black_box(plan(&shown, &colors, &mut resolver, options(), &mut fits));
            work.push(t.elapsed().as_secs_f64() * 1e3);
            if shown.text().contains("flood done") {
                break;
            }
        }
        let elapsed = start.elapsed().as_secs_f64();
        drop(session);
        work.sort_by(f64::total_cmp);
        let pct = |p: f64| work[((work.len() - 1) as f64 * p) as usize];
        println!(
            "flood of {mib:.0} MiB ({} build), copy {}: {elapsed:.2} s = {:.1} MiB/s; {} frames with a new screen, frame work p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
            build(),
            if background {
                "on a thread"
            } else {
                "in the frame"
            },
            mib / elapsed,
            work.len(),
            pct(0.5),
            pct(0.99),
            pct(1.0),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The view's copies, on a thread: after a change, at most one per frame,
/// one per `FLOOD_FRAME` once output kept coming for `FLOOD_AFTER`; the
/// next copy waits until the frame took the last one.
#[cfg(unix)]
fn copier(
    screen: Screen,
    events: flume::Receiver<SessionEvent>,
    copies: flume::Sender<Snapshot>,
    spares: flume::Receiver<Snapshot>,
) {
    let (mut last, mut busy_since): (Option<Instant>, Option<Instant>) = (None, None);
    while let Ok(event) = events.recv() {
        if !matches!(event, SessionEvent::Wakeup | SessionEvent::Exited(_)) {
            continue;
        }
        let now = Instant::now();
        if last.is_some_and(|t| now - t < FLOOD_FRAME) {
            busy_since.get_or_insert(now);
        } else {
            busy_since = None;
        }
        let flooding = busy_since.is_some_and(|t| t.elapsed() >= FLOOD_AFTER);
        let interval = if flooding { FLOOD_FRAME } else { COPY_FRAME };
        if let Some(t) = last {
            std::thread::sleep(interval.saturating_sub(t.elapsed()));
        }
        let mut buffer = spares.try_recv().unwrap_or_default();
        last = Some(Instant::now());
        screen.snapshot_into(&mut buffer);
        if copies.send(buffer).is_err() {
            return;
        }
    }
}

/// A key to the snapshot that shows its echo: `cat` without arguments
/// echoes through the terminal's line discipline, as a shell's prompt
/// does.
#[test]
#[ignore = "measurement"]
#[cfg(unix)]
fn bench_key_to_snapshot() {
    let (session, rx) = spawn("/bin/cat", &[], 80, 24);
    let mut snapshot = Snapshot::default();
    let mut times = Vec::new();
    let text = "the quick brown fox jumps over the lazy dog ";
    for (i, c) in text.chars().cycle().take(200).enumerate() {
        let column = i % 70;
        if column == 0 && i > 0 {
            session.write(b"\r".to_vec());
            std::thread::sleep(Duration::from_millis(20));
            while rx.try_recv().is_ok() {}
            session.snapshot_into(&mut snapshot);
        }
        let start = Instant::now();
        session.write(c.to_string().into_bytes());
        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(SessionEvent::Wakeup) => {}
                Ok(_) => continue,
                Err(e) => panic!("no echo: {e:?}"),
            }
            session.snapshot_into(&mut snapshot);
            let cursor = snapshot.cursor.map(|c| c.point.column);
            if cursor == Some(column + 1) {
                break;
            }
        }
        times.push(start.elapsed());
        std::thread::sleep(Duration::from_millis(5));
    }
    session.kill();
    times.sort();
    let pct = |p: f64| times[((times.len() - 1) as f64 * p) as usize].as_secs_f64() * 1e3;
    println!(
        "key to snapshot through the PTY ({} build, {} keys): median {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        build(),
        times.len(),
        pct(0.5),
        pct(0.95),
        pct(0.99),
        pct(1.0),
    );
}
