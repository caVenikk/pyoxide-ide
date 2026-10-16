//! Measurements of the terminal view (#157), in the development terminal
//! (`terminal.dev-session`) until the Terminal tool window exists:
//!
//! - `terminal-typing`: keys typed into `cat` (which echoes them through
//!   the terminal's line discipline, as a shell's prompt does), the time
//!   from each key to the end of the first frame that shows its echo;
//! - `terminal-flood:<MiB>`: `cat` of a generated file of that size, the
//!   time to the frame that shows its last line, and the frames painted
//!   meanwhile;
//! - `terminal-scroll`: wheel events over a long scrollback, frame work and
//!   gaps between frames;
//! - `typing` with `PYOXIDE_BENCH_FLOOD=1`: the editor typing measurement
//!   while a terminal prints without pause.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, AsyncApp, Entity, Keystroke, ScrollDelta, ScrollWheelEvent, WindowHandle,
    point,
};
use serde_json::json;
use terminal_ui::SessionView;

use super::{
    KEY_INTERVAL, KEY_JITTER_MS, KEYS_PER_RUN, RUNS, TYPING_TEXT, display_frames, force_frame,
    frame_source, frames, ms, percentile, sleep, stats,
};
use crate::env;
use crate::workbench::Workbench;
use crate::workbench::terminal_dev::OpenDevTerminal;

/// Shows the development terminal running `shell` (a command line) and
/// waits for it to start.
async fn open(
    window: WindowHandle<Workbench>,
    shell: Option<String>,
    input: Option<String>,
    cx: &mut AsyncApp,
) -> Option<Entity<SessionView>> {
    let _ = window.update(cx, |w, window, cx| {
        w.open_dev_terminal(&OpenDevTerminal { input, shell }, window, cx)
    });
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        let view = window
            .read_with(cx, |w, cx| w.dev_terminal_session(cx))
            .ok()
            .flatten();
        if let Some(view) = view
            && view
                .read_with(cx, |v, _| v.session().is_some())
                .unwrap_or(false)
        {
            return Some(view);
        }
        // The session starts with the first frame that lays it out.
        force_frame(window, cx);
        sleep(cx, Duration::from_millis(20)).await;
    }
    env::marker("terminal {\"error\": \"the session did not start\"}");
    None
}

/// A file of numbered, partly coloured lines of about `bytes`, ending with
/// `flood done`.
fn flood_file(bytes: usize) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    let dir = std::env::temp_dir().join(format!("pyoxide-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("flood-{bytes}.txt"));
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
    let (mut written, mut n) = (0usize, 0u64);
    while written < bytes {
        let line = format!(
            "line {n:08} \x1b[32mthe quick brown fox\x1b[0m jumps over the \x1b[1;31mlazy dog\x1b[0m 0123456789\n"
        );
        written += line.len();
        f.write_all(line.as_bytes())?;
        n += 1;
    }
    f.write_all(b"flood done\n")?;
    f.flush()?;
    Ok(path)
}

/// A command line that prints a file (`cat` on Unix, `type` in cmd.exe).
fn print_command(path: &std::path::Path) -> String {
    if cfg!(windows) {
        format!("cmd.exe /c type \"{}\"", path.display())
    } else {
        format!("/bin/cat {}", path.display())
    }
}

/// Keys typed into `cat`: key to the frame that shows the echo.
pub(super) async fn typing(window: WindowHandle<Workbench>, cx: &mut AsyncApp) {
    // `cat` echoes without a shell's start-up; cmd.exe echoes the same way.
    let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/cat" };
    let Some(view) = open(window, Some(shell.into()), None, cx).await else {
        return;
    };
    sleep(cx, Duration::from_secs(2)).await;
    let display = display_frames(window, cx).await;
    let _ = view.update(cx, |v, _| v.record_frames(true));
    struct Key {
        due: Instant,
        handled: Instant,
        sent: Instant,
        column: usize,
    }
    let mut keys = Vec::with_capacity(RUNS * KEYS_PER_RUN);
    let mut seed: u32 = 0x9E37_79B9;
    let mut jitter = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        Duration::from_millis(u64::from(seed % KEY_JITTER_MS))
    };
    let text: Vec<char> = TYPING_TEXT.chars().cycle().take(KEYS_PER_RUN).collect();
    let start_column = view
        .read_with(cx, |v, _| v.snapshot().cursor.map_or(0, |c| c.point.column))
        .unwrap_or(0);
    let dispatch = |key: &str, cx: &mut AsyncApp| {
        if let Ok(keystroke) = Keystroke::parse(key) {
            let _ = cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
        }
    };
    for _ in 0..RUNS {
        let mut due = Instant::now();
        let mut column = start_column;
        for (i, c) in text.iter().enumerate() {
            // A new line every 50 keys: `cat` prints it, the echo starts
            // over at the left.
            if i % 50 == 0 && i > 0 {
                dispatch("enter", cx);
                column = 0;
                sleep(cx, Duration::from_millis(100)).await;
                due = Instant::now();
            }
            due += KEY_INTERVAL + jitter();
            let now = Instant::now();
            if due > now {
                sleep(cx, due - now).await;
            }
            let key = if *c == ' ' {
                "space".to_owned()
            } else {
                c.to_string()
            };
            if !display {
                force_frame(window, cx);
            }
            let sent = Instant::now();
            dispatch(&key, cx);
            let handled = Instant::now();
            column += 1;
            keys.push(Key {
                due,
                handled,
                sent,
                column,
            });
            if !display {
                // The echo comes back from the PTY: draw once it is there.
                for _ in 0..50 {
                    sleep(cx, Duration::from_millis(1)).await;
                    force_frame(window, cx);
                    let shown = view
                        .read_with(cx, |v, _| {
                            v.snapshot().cursor.map(|c| c.point.column) == Some(column)
                        })
                        .unwrap_or(true);
                    if shown {
                        break;
                    }
                }
            }
        }
        dispatch("enter", cx);
        sleep(cx, Duration::from_secs(2)).await;
    }
    let log = view
        .update(cx, |v, _| {
            let log = v.take_frame_log();
            v.record_frames(false);
            log
        })
        .unwrap_or_default();
    let frames = frames();
    let (mut latency, mut handling, mut work, mut missed) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for k in &keys {
        // The first terminal frame after the key whose cursor is past it,
        // then the window frame that painted it.
        let Some(shown) = log
            .iter()
            .find(|r| r.at >= k.handled && r.cursor.map(|c| c.column) == Some(k.column))
        else {
            missed += 1;
            continue;
        };
        let Some(f) = frames.iter().find(|f| f.end >= shown.at) else {
            missed += 1;
            continue;
        };
        latency.push(ms(f.end - k.due));
        handling.push(ms(k.handled - k.sent));
        work.push(ms(f.end - f.start));
    }
    env::marker(&format!(
        "terminal-typing {}",
        json!({
            "frames": frame_source(display),
            "keys": keys.len(),
            "frames_missing": missed,
            "key_to_frame_ms": stats(latency),
            "key_handling_ms": stats(handling),
            "frame_ms": stats(work),
        })
    ));
}

/// `cat` of a large file: to the frame that shows its last line.
pub(super) async fn flood(window: WindowHandle<Workbench>, mib: usize, cx: &mut AsyncApp) {
    let path = match flood_file(mib << 20) {
        Ok(path) => path,
        Err(e) => {
            env::marker(&format!(
                "terminal-flood {}",
                json!({ "error": e.to_string() })
            ));
            return;
        }
    };
    // An idle shell first, so the session's start is not measured.
    let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    let Some(view) = open(window, Some(shell.into()), None, cx).await else {
        return;
    };
    sleep(cx, Duration::from_secs(2)).await;
    let display = display_frames(window, cx).await;
    let _ = view.update(cx, |v, _| v.record_frames(true));
    let command = format!("{}\r", print_command(&path));
    let t0 = Instant::now();
    let _ = view.update(cx, |v, cx| v.type_text(&command, cx));
    let until = t0 + Duration::from_secs(300);
    let mut done = None;
    while Instant::now() < until && done.is_none() {
        sleep(cx, Duration::from_millis(5)).await;
        if !display {
            force_frame(window, cx);
        }
        done = view
            .read_with(cx, |v, _| {
                v.snapshot()
                    .text()
                    .lines()
                    .any(|l| l == "flood done")
                    .then(Instant::now)
            })
            .ok()
            .flatten();
    }
    sleep(cx, Duration::from_millis(200)).await;
    let log = view
        .update(cx, |v, _| {
            let log = v.take_frame_log();
            v.record_frames(false);
            log
        })
        .unwrap_or_default();
    let shown = log
        .iter()
        .find(|r| r.at >= t0 && r.last_line == "flood done")
        .map(|r| r.at);
    let frames: Vec<_> = frames()
        .into_iter()
        .filter(|f| f.start >= t0 && shown.is_none_or(|s| f.end <= s + Duration::from_millis(50)))
        .collect();
    let seconds = shown.map(|s| (s - t0).as_secs_f64());
    let work: Vec<f64> = frames.iter().map(|f| ms(f.end - f.start)).collect();
    let _ = std::fs::remove_file(&path);
    env::marker(&format!(
        "terminal-flood {}",
        json!({
            "frames": frame_source(display),
            "mib": mib,
            "seconds": seconds,
            "mib_per_s": seconds.map(|s| (mib as f64 / s * 10.0).round() / 10.0),
            "frame_count": frames.len(),
            "terminal_frames": log.iter().filter(|r| r.at >= t0).count(),
            "frame_ms": stats(work),
        })
    ));
}

/// Wheel events over a long scrollback: three lines per event, up for
/// three seconds, then down.
pub(super) async fn scroll(window: WindowHandle<Workbench>, cx: &mut AsyncApp) {
    let path = match flood_file(2 << 20) {
        Ok(path) => path,
        Err(e) => {
            env::marker(&format!(
                "terminal-scroll {}",
                json!({ "error": e.to_string() })
            ));
            return;
        }
    };
    let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
    let input = format!("{}\r", print_command(&path));
    let Some(view) = open(window, Some(shell.into()), Some(input), cx).await else {
        return;
    };
    let until = Instant::now() + Duration::from_secs(120);
    while Instant::now() < until {
        let done = view
            .read_with(cx, |v, _| v.snapshot().text().contains("flood done"))
            .unwrap_or(false);
        if done {
            break;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let _ = std::fs::remove_file(&path);
    sleep(cx, Duration::from_secs(1)).await;
    let display = display_frames(window, cx).await;
    let Some(geometry) = view.read_with(cx, |v, _| v.geometry()).ok().flatten() else {
        return;
    };
    let at = point(
        geometry.origin.x + geometry.cell.width * 10.0,
        geometry.origin.y + geometry.cell.height * 2.0,
    );
    let _ = view.update(cx, |v, _| v.record_frames(true));
    let from = Instant::now();
    for (direction, seconds) in [(3.0f32, 3u64), (-3.0, 3)] {
        let leg = Instant::now();
        while leg.elapsed() < Duration::from_secs(seconds) {
            // The view's wheel handling, as the element calls it.
            let event = ScrollWheelEvent {
                position: at,
                delta: ScrollDelta::Lines(point(0.0, direction)),
                ..ScrollWheelEvent::default()
            };
            let _ = view.update(cx, |v, cx| v.scroll_wheel(&event, cx));
            if !display {
                force_frame(window, cx);
            }
            sleep(cx, Duration::from_millis(8)).await;
        }
    }
    let until = Instant::now();
    let log = view
        .update(cx, |v, _| {
            let log = v.take_frame_log();
            v.record_frames(false);
            log
        })
        .unwrap_or_default();
    let frames: Vec<_> = frames()
        .into_iter()
        .filter(|f| f.start >= from && f.end <= until)
        .collect();
    let work: Vec<f64> = frames.iter().map(|f| ms(f.end - f.start)).collect();
    let gaps: Vec<f64> = frames.windows(2).map(|w| ms(w[1].end - w[0].end)).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(f64::total_cmp);
    let typical = percentile(&sorted, 50.0);
    let long_gaps = gaps.iter().filter(|g| **g > typical * 1.5).count();
    let max_offset = log.iter().map(|r| r.display_offset).max();
    env::marker(&format!(
        "terminal-scroll {}",
        json!({
            "frames": frame_source(display),
            "frame_count": frames.len(),
            "frame_ms": stats(work),
            "frame_gap_ms": display.then(|| stats(gaps)),
            "gaps_over_1_5x_median": display.then_some(long_gaps),
            "max_offset": max_offset,
        })
    ));
}

/// For `typing` with `PYOXIDE_BENCH_FLOOD=1`: a terminal that prints
/// without pause (the file again and again) while the editor is typed in;
/// the focus goes back to the editor.
pub(super) async fn start_flood(window: WindowHandle<Workbench>, cx: &mut AsyncApp) -> bool {
    let Ok(path) = flood_file(4 << 20) else {
        return false;
    };
    let shell = if cfg!(windows) {
        format!(
            "cmd.exe /c \"for /L %i in (0,0,1) do @type \"{}\"\"",
            path.display()
        )
    } else {
        format!("/bin/sh -c \"while :; do cat {}; done\"", path.display())
    };
    if open(window, Some(shell), None, cx).await.is_none() {
        return false;
    }
    let _ = cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke("workbench.focus-editor", json!(null), window, cx)
    });
    sleep(cx, Duration::from_secs(1)).await;
    true
}

/// Whether the typing measurement runs with a flooding terminal.
pub(super) fn flood_requested() -> bool {
    std::env::var_os("PYOXIDE_BENCH_FLOOD").is_some_and(|v| v == "1")
}
