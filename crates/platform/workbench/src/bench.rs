//! Measurements run inside the application (`PYOXIDE_BENCH`, see
//! replica/perf.md). Keystrokes are injected through the window's own key
//! dispatch (the path a key event from the platform takes) at fixed
//! intervals; frame timings come from the workbench's draw: the root view's
//! render marks the start of a frame, a canvas painted last marks the end
//! of its paint. Results are printed as `PYOXIDE <name> <json>` lines.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use editor::{Editor, FindBar};
use gpui::{App, AppContext as _, AsyncApp, Entity, Keystroke, WindowHandle};
use serde_json::json;

use crate::env::{self, Bench};
use crate::search_everywhere::SearchEverywhere;
use crate::workbench::Workbench;

/// Text typed by the typing measurement: 200 characters of a comment, the
/// same text the reference runner types (scripts/perf/pycharm_typing.py).
pub const TYPING_TEXT: &str = "# the quick brown fox jumps over the lazy dog and keeps typing to fill \
     two hundred characters for the latency run of the reference editor ok done";
/// Characters per run, runs, and the interval between keys: 50 ms as in
/// the reference run, plus a pseudo-random 0-16 ms so that keys do not
/// keep the same phase to the display's frames (which would make the
/// key-to-frame time depend on that one phase).
const KEYS_PER_RUN: usize = 200;
const RUNS: usize = 3;
const KEY_INTERVAL: Duration = Duration::from_millis(50);
const KEY_JITTER_MS: u32 = 17;
/// Line (one-based) the caret goes to before typing.
const TYPING_LINE: usize = 50_000;

/// One drawn frame: start of the root render, end of the paint, and what
/// the active editor showed.
#[derive(Debug, Clone)]
struct Frame {
    start: Instant,
    end: Instant,
    version: u64,
    path: Option<PathBuf>,
    lines: usize,
    highlighted: bool,
    /// The query whose matches the find bar showed.
    find_text: Option<String>,
    /// The text whose rows Search Everywhere showed.
    se_text: Option<String>,
    /// The completion popup was drawn.
    completion: bool,
}

#[derive(Default)]
struct Log {
    start: Option<Instant>,
    frames: Vec<Frame>,
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::default();
}

/// Called at the start of the workbench's render (the start of a frame).
pub fn frame_started() {
    LOG.with(|l| l.borrow_mut().start = Some(Instant::now()));
}

/// Called from a canvas painted after everything else (the end of the
/// frame's paint).
pub fn frame_painted(
    editor: Option<&Entity<Editor>>,
    find: Option<&Entity<FindBar>>,
    se: &Entity<SearchEverywhere>,
    completion: bool,
    cx: &App,
) {
    let end = Instant::now();
    let find_text = find.and_then(|f| f.read(cx).searched_text().map(str::to_owned));
    let se = se.read(cx);
    let se_text = (se.is_open() && !se.is_searching()).then(|| se.field().read(cx).text());
    let (version, path, lines, highlighted) = match editor {
        Some(e) => {
            let e = e.read(cx);
            (
                e.buffer().version(),
                e.path().cloned(),
                e.shaped_line_count(),
                e.highlighting_current(),
            )
        }
        None => (0, None, 0, false),
    };
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        let start = l.start.take().unwrap_or(end);
        l.frames.push(Frame {
            start,
            end,
            version,
            path,
            lines,
            highlighted,
            find_text,
            se_text,
            completion,
        });
    });
}

fn frames() -> Vec<Frame> {
    LOG.with(|l| l.borrow().frames.clone())
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

/// p-th percentile (nearest rank) of sorted values.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn stats(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
    let r = |v: f64| (v * 100.0).round() / 100.0;
    json!({
        "n": values.len(),
        "p50": r(percentile(&values, 50.0)),
        "p90": r(percentile(&values, 90.0)),
        "p99": r(percentile(&values, 99.0)),
        "max": r(values.last().copied().unwrap_or(f64::NAN)),
        "mean": r(mean),
    })
}

/// Resident memory of this process in MB (`ps`), where available.
fn rss_mb() -> Option<f64> {
    if cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some((kb / 1024.0).round())
}

async fn sleep(cx: &AsyncApp, d: Duration) {
    cx.background_executor().timer(d).await;
}

fn active_editor(window: WindowHandle<Workbench>, cx: &mut AsyncApp) -> Option<Entity<Editor>> {
    window
        .read_with(cx, |w, _| w.active_editor().cloned())
        .ok()
        .flatten()
}

/// Waits until a file is open, nothing runs in the background and its
/// highlighting is current (at most `limit`).
async fn settle(window: WindowHandle<Workbench>, cx: &mut AsyncApp, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        let ready = window
            .read_with(cx, |w, cx| {
                !w.is_busy(cx)
                    && w.active_editor()
                        .is_some_and(|e| e.read(cx).highlighting_current())
            })
            .unwrap_or(false);
        if ready {
            return true;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    false
}

/// Starts the measurement requested with `PYOXIDE_BENCH` (other than
/// start-up, which the first frame reports itself); quits when done.
pub fn start(window: WindowHandle<Workbench>, cx: &mut App) {
    let Some(kind) = env::bench().cloned() else {
        return;
    };
    if kind == Bench::Startup {
        return;
    }
    // A covered window gets no frames from the display: keep it on top
    // (without the focus) while measuring.
    let _ = window.update(cx, |_, window, _| crate::shot::keep_visible(window));
    cx.spawn(async move |cx| {
        match kind {
            Bench::Typing => typing(window, cx).await,
            Bench::Open(path) => open(window, path, cx).await,
            Bench::Cursor => cursor(window, cx).await,
            Bench::Find(query) => find(window, query, cx).await,
            Bench::FindInFiles(query) => find_in_files(window, query, cx).await,
            Bench::GoToFile(query) => go_to_file(window, query, cx).await,
            Bench::Completion(latency) => completion(window, latency, cx).await,
            Bench::Startup => {}
        }
        if let Some(rss) = rss_mb() {
            env::marker(&format!("memory {}", json!({ "rss_mb": rss })));
        }
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Whether the display drives frames: a covered window, or one on a locked
/// screen, gets none, and the measurement then draws each frame itself.
async fn display_frames(window: WindowHandle<Workbench>, cx: &mut AsyncApp) -> bool {
    let before = frames().len();
    let _ = cx.update_window(window.into(), |_, window, _| window.refresh());
    sleep(cx, Duration::from_millis(500)).await;
    frames().len() > before
}

/// Draws a frame now (when the display does not drive frames).
fn force_frame(window: WindowHandle<Workbench>, cx: &mut AsyncApp) {
    let _ = cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear());
}

fn frame_source(display: bool) -> &'static str {
    if display { "display" } else { "forced" }
}

/// Typing in the middle of a large file: `RUNS` runs of `KEYS_PER_RUN`
/// keys, one every `KEY_INTERVAL` plus jitter.
async fn typing(window: WindowHandle<Workbench>, cx: &mut AsyncApp) {
    if !settle(window, cx, Duration::from_secs(120)).await {
        env::marker("typing {\"error\": \"the file did not open\"}");
        return;
    }
    let Some(editor) = active_editor(window, cx) else {
        return;
    };
    let marks = std::env::var("PYOXIDE_BENCH_MARKS")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let _ = editor.update(cx, |e, cx| {
        if marks > 0 {
            e.set_diagnostics(sample_marks(e.buffer(), marks), cx);
            e.set_analysis(editor::Analysis::Done, cx);
        }
        e.go_to(TYPING_LINE, Some(1), cx)
    });
    sleep(cx, Duration::from_secs(3)).await;
    let display = display_frames(window, cx).await;

    struct Key {
        due: Instant,
        sent: Instant,
        handled: Instant,
        version: u64,
    }
    let text: Vec<char> = TYPING_TEXT.chars().cycle().take(KEYS_PER_RUN).collect();
    let mut keys = Vec::with_capacity(RUNS * KEYS_PER_RUN);
    let mut seed: u32 = 0x9E37_79B9;
    let mut jitter = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        Duration::from_millis(u64::from(seed % KEY_JITTER_MS))
    };
    for _ in 0..RUNS {
        let mut due = Instant::now();
        for c in &text {
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
            let Ok(keystroke) = Keystroke::parse(&key) else {
                continue;
            };
            if !display {
                // GPUI draws a pending frame before it dispatches an event
                // (the background parse finishing since the last key, the
                // caret blinking): draw it now, outside the measurement, as
                // the display would have done.
                force_frame(window, cx);
            }
            let sent = Instant::now();
            let _ = cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
            let handled = Instant::now();
            let version = editor
                .read_with(cx, |e, _| e.buffer().version())
                .unwrap_or(0);
            if !display {
                force_frame(window, cx);
            }
            keys.push(Key {
                due,
                sent,
                handled,
                version,
            });
        }
        sleep(cx, Duration::from_secs(2)).await;
    }

    let frames = frames();
    let mut latency = Vec::new();
    let mut work = Vec::new();
    let mut handling = Vec::new();
    let mut paint = Vec::new();
    let mut missed = 0;
    for k in &keys {
        // The first frame drawn after the key that paints its text.
        let Some(f) = frames
            .iter()
            .find(|f| f.start >= k.handled && f.version >= k.version)
        else {
            missed += 1;
            continue;
        };
        latency.push(ms(f.end - k.due));
        work.push(ms(k.handled - k.sent) + ms(f.end - f.start));
        handling.push(ms(k.handled - k.sent));
        paint.push(ms(f.end - f.start));
    }
    let line = editor
        .read_with(cx, |e, _| e.caret_point().line + 1)
        .unwrap_or(0);
    env::marker(&format!(
        "typing {}",
        json!({
            "frames": frame_source(display),
            "marks": marks,
            "keys": keys.len(),
            "frames_missing": missed,
            "caret_line": line,
            "key_to_frame_ms": stats(latency),
            "app_work_ms": stats(work),
            "key_handling_ms": stats(handling),
            "frame_ms": stats(paint),
        })
    ));
}

/// `count` problems spread evenly over the lines of `buffer`, of every
/// level, some unused or deprecated, each over the first ten chars of its
/// line (one at the typing line).
fn sample_marks(buffer: &text::Buffer, count: usize) -> Vec<editor::DiagnosticMark> {
    use editor::{DiagnosticMark, MarkMessage, MarkSeverity};
    let lines = buffer.len_lines();
    let step = (lines / count.max(1)).max(1);
    (0..count)
        .map(|i| {
            let line = (i * step).min(lines - 1);
            let start = buffer.line_start(line);
            let end = start + buffer.line_len(line).min(10);
            let severity = match i % 4 {
                0 => MarkSeverity::Error,
                1 => MarkSeverity::Warning,
                2 => MarkSeverity::WeakWarning,
                _ => MarkSeverity::Information,
            };
            DiagnosticMark {
                range: start..end,
                severity,
                unnecessary: i % 5 == 0,
                deprecated: i % 7 == 0,
                messages: vec![MarkMessage {
                    severity,
                    text: "A problem for the measurement".into(),
                    source: Some("Bench".into()),
                    code: Some("B001".into()),
                    href: None,
                }],
            }
        })
        .collect()
}

/// Opening a file in the open project: from the command to the first frame
/// that paints its text, and to the first frame painted with its
/// highlighting current; then scrolling through it.
async fn open(window: WindowHandle<Workbench>, path: PathBuf, cx: &mut AsyncApp) {
    // The project tree has listed the folder; the window has drawn.
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if window
            .read_with(cx, |w, cx| !w.is_busy(cx))
            .unwrap_or(false)
        {
            break;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    sleep(cx, Duration::from_secs(3)).await;
    let display = display_frames(window, cx).await;
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let t0 = Instant::now();
    let _ = window.update(cx, |w, window, cx| w.open_file(path.clone(), window, cx));
    let until = Instant::now() + Duration::from_secs(60);
    let mut text_ms = None;
    let mut highlighted_ms = None;
    let (mut tab_drawn, mut highlight_drawn) = (false, false);
    while Instant::now() < until && highlighted_ms.is_none() {
        sleep(cx, Duration::from_millis(1)).await;
        if !display {
            // Draw when there is something new to show: the tab, then the
            // highlighting.
            let (tab, highlighted) = active_editor(window, cx)
                .and_then(|e| {
                    e.read_with(cx, |e, _| {
                        (e.path() == Some(&path), e.highlighting_current())
                    })
                    .ok()
                })
                .unwrap_or((false, false));
            if tab && !tab_drawn || tab && highlighted && !highlight_drawn {
                force_frame(window, cx);
                tab_drawn = true;
                highlight_drawn = highlighted;
            }
        }
        let frames = frames();
        let shown = |f: &&Frame| f.end > t0 && f.path.as_ref() == Some(&path) && f.lines > 0;
        text_ms = text_ms.or_else(|| frames.iter().find(shown).map(|f| ms(f.end - t0)));
        highlighted_ms = frames
            .iter()
            .filter(shown)
            .find(|f| f.highlighted)
            .map(|f| ms(f.end - t0));
    }
    env::marker(&format!(
        "open {}",
        json!({
            "frames": frame_source(display),
            "path": path.display().to_string(),
            "first_text_ms": text_ms,
            "highlighted_ms": highlighted_ms,
        })
    ));

    // Scroll down three lines per tick for five seconds: frame work and,
    // with display frames, the gaps between painted frames.
    let Some(editor) = active_editor(window, cx) else {
        return;
    };
    sleep(cx, Duration::from_secs(1)).await;
    let from = Instant::now();
    let mut line = 0;
    while from.elapsed() < Duration::from_secs(5) {
        line += 3;
        let _ = cx.update_window(window.into(), |_, window, cx| {
            let _ = ui::command::invoke("editor.scroll-to", json!({ "line": line }), window, cx);
        });
        if !display {
            force_frame(window, cx);
        }
        sleep(cx, Duration::from_millis(8)).await;
    }
    let until = Instant::now();
    let frames: Vec<Frame> = frames()
        .into_iter()
        .filter(|f| f.start >= from && f.end <= until)
        .collect();
    let work: Vec<f64> = frames.iter().map(|f| ms(f.end - f.start)).collect();
    let gaps: Vec<f64> = frames.windows(2).map(|w| ms(w[1].end - w[0].end)).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(f64::total_cmp);
    let typical = percentile(&sorted, 50.0);
    let long_gaps = gaps.iter().filter(|g| **g > typical * 1.5).count();
    let top = editor
        .read_with(cx, |e, _| e.visible_lines().map(|r| r.start))
        .ok()
        .flatten();
    env::marker(&format!(
        "scroll {}",
        json!({
            "frames": frame_source(display),
            "frame_count": frames.len(),
            "frame_ms": stats(work),
            "frame_gap_ms": display.then(|| stats(gaps)),
            "gaps_over_1_5x_median": display.then_some(long_gaps),
            "last_top_line": top,
        })
    ));
}

/// Typing a query into the find bar of the open file: `RUNS` runs that
/// clear the field and type the query char by char, one key every
/// `FIND_KEY_INTERVAL`; for each key, the time from its dispatch to the end
/// of the first frame that paints the matches of the query typed so far.
async fn find(window: WindowHandle<Workbench>, query: String, cx: &mut AsyncApp) {
    const FIND_RUNS: usize = 5;
    const FIND_KEY_INTERVAL: Duration = Duration::from_millis(150);
    if !settle(window, cx, Duration::from_secs(120)).await {
        env::marker("find {\"error\": \"the file did not open\"}");
        return;
    }
    sleep(cx, Duration::from_secs(2)).await;
    let display = display_frames(window, cx).await;
    let lines = active_editor(window, cx)
        .and_then(|e| e.read_with(cx, |e, _| e.buffer().len_lines()).ok())
        .unwrap_or(0);
    let mut latency = Vec::new();
    let mut handling = Vec::new();
    let mut missed = 0;
    let mut counts = Vec::new();
    for _ in 0..FIND_RUNS {
        let _ = cx.update_window(window.into(), |_, window, cx| {
            if !display {
                window.draw(cx).clear();
            }
            let _ = ui::command::invoke("find.search", json!({ "text": "" }), window, cx);
        });
        sleep(cx, Duration::from_millis(500)).await;
        let mut typed = String::new();
        for c in query.chars() {
            typed.push(c);
            let key = if c == ' ' {
                "space".to_owned()
            } else {
                c.to_string()
            };
            let Ok(keystroke) = Keystroke::parse(&key) else {
                continue;
            };
            if !display {
                force_frame(window, cx);
            }
            let sent = Instant::now();
            let _ = cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
            handling.push(ms(sent.elapsed()));
            // Wait for the first frame that shows the matches of `typed`.
            let until = Instant::now() + Duration::from_secs(10);
            let mut shown = None;
            while Instant::now() < until {
                sleep(cx, Duration::from_millis(1)).await;
                if !display {
                    force_frame(window, cx);
                }
                shown = frames()
                    .into_iter()
                    .find(|f| f.start >= sent && f.find_text.as_deref() == Some(typed.as_str()));
                if shown.is_some() {
                    break;
                }
            }
            match shown {
                Some(f) => latency.push(ms(f.end - sent)),
                None => missed += 1,
            }
            sleep(cx, FIND_KEY_INTERVAL).await;
        }
        let count = window
            .read_with(cx, |w, cx| {
                w.active_find_bar().map(|f| f.read(cx).counts(cx).0)
            })
            .ok()
            .flatten();
        counts.push(count);
    }
    env::marker(&format!(
        "find {}",
        json!({
            "frames": frame_source(display),
            "query": query,
            "lines": lines,
            "keys": FIND_RUNS * query.chars().count(),
            "frames_missing": missed,
            "matches": counts.last().copied().flatten(),
            "key_to_highlight_ms": stats(latency),
            "key_handling_ms": stats(handling),
        })
    ));
}

/// Searches the open project with Find in Files: the time from the
/// command to the first rows on the UI thread and to the end of the
/// search, and the longest frame drawn meanwhile.
async fn find_in_files(window: WindowHandle<Workbench>, query: String, cx: &mut AsyncApp) {
    const RUNS: usize = 5;
    // The project's tree is listed first.
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        let busy = window.read_with(cx, |w, cx| w.is_busy(cx)).unwrap_or(true);
        if !busy {
            break;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    sleep(cx, Duration::from_secs(1)).await;
    let display = display_frames(window, cx).await;
    let (mut first, mut done, mut longest) = (Vec::new(), Vec::new(), Vec::new());
    let mut result = serde_json::Value::Null;
    for _ in 0..RUNS {
        let started = Instant::now();
        let _ = cx.update_window(window.into(), |_, window, cx| {
            let args = json!({
                "text": query,
                "case_sensitive": false,
                "whole_words": false,
                "regex": false,
                "mask": "",
            });
            let _ = ui::command::invoke("find-in-files.search", args, window, cx);
        });
        let until = Instant::now() + Duration::from_secs(120);
        let mut timing = None;
        while Instant::now() < until {
            sleep(cx, Duration::from_millis(1)).await;
            if !display {
                force_frame(window, cx);
            }
            timing = window
                .read_with(cx, |w, cx| w.find_in_files().read(cx).timing())
                .ok()
                .flatten();
            if timing.is_some_and(|t| t.1.is_some()) {
                break;
            }
        }
        let Some((first_rows, finished)) = timing else {
            continue;
        };
        if let Some(t) = first_rows {
            first.push(ms(t));
        }
        if let Some(t) = finished {
            done.push(ms(t));
        }
        let end = Instant::now();
        let frames_ms: Vec<f64> = frames()
            .into_iter()
            .filter(|f| f.start >= started && f.end <= end)
            .map(|f| ms(f.end - f.start))
            .collect();
        longest.push(frames_ms.iter().copied().fold(0.0, f64::max));
        result = window
            .read_with(cx, |w, cx| {
                let s = w.find_in_files().read(cx).state_json(cx);
                json!({ "matches": s["matches"], "files": s["files"], "info": s["info"] })
            })
            .unwrap_or_default();
        sleep(cx, Duration::from_millis(300)).await;
    }
    env::marker(&format!(
        "find_in_files {}",
        json!({
            "frames": frame_source(display),
            "query": query,
            "runs": RUNS,
            "result": result,
            "first_rows_ms": stats(first),
            "done_ms": stats(done),
            "longest_frame_ms": stats(longest),
        })
    ));
}

/// Typing a query into Go to File: the index of the project's file names
/// is built first (its time is reported); then `RUNS` runs open the popup
/// with an empty field and type the query char by char. For each key, the
/// time from its dispatch to the end of the first frame drawn after its
/// rows came (the popup's pause after the last key included), and the
/// search itself.
async fn go_to_file(window: WindowHandle<Workbench>, query: String, cx: &mut AsyncApp) {
    const RUNS: usize = 5;
    const KEY_INTERVAL: Duration = Duration::from_millis(150);
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        let ready = window
            .read_with(cx, |w, cx| {
                let files = w.project_files().read(cx);
                !w.is_busy(cx) && files.index().is_some() && !files.is_building()
            })
            .unwrap_or(false);
        if ready {
            break;
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
    let (build, entries) = window
        .read_with(cx, |w, cx| {
            let files = w.project_files().read(cx);
            (
                files.build_time().map(ms),
                files.index().map(|i| i.project_len()),
            )
        })
        .unwrap_or_default();
    sleep(cx, Duration::from_secs(1)).await;
    let display = display_frames(window, cx).await;
    let se = window
        .read_with(cx, |w, _| w.search_everywhere().clone())
        .ok();
    let Some(se) = se else {
        return;
    };
    let (mut latency, mut search, mut handling) = (Vec::new(), Vec::new(), Vec::new());
    let mut missed = 0;
    let mut rows = 0;
    for _ in 0..RUNS {
        let _ = cx.update_window(window.into(), |_, window, cx| {
            let args = json!({ "text": "", "tab": "files" });
            let _ = ui::command::invoke("search-everywhere.search", args, window, cx);
        });
        sleep(cx, Duration::from_millis(500)).await;
        let mut typed = String::new();
        for c in query.chars() {
            typed.push(c);
            let key = if c == ' ' {
                "space".to_owned()
            } else {
                c.to_string()
            };
            let Ok(keystroke) = Keystroke::parse(&key) else {
                continue;
            };
            if !display {
                force_frame(window, cx);
            }
            let sent = Instant::now();
            let _ = cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
            handling.push(ms(sent.elapsed()));
            // The first frame that draws the rows of `typed`.
            let until = Instant::now() + Duration::from_secs(10);
            let mut shown = None;
            while Instant::now() < until {
                sleep(cx, Duration::from_millis(1)).await;
                if !display {
                    force_frame(window, cx);
                }
                shown = frames()
                    .into_iter()
                    .find(|f| f.start >= sent && f.se_text.as_deref() == Some(typed.as_str()));
                if shown.is_some() {
                    break;
                }
            }
            match shown {
                Some(f) => latency.push(ms(f.end - sent)),
                None => missed += 1,
            }
            if let Some((t, n)) = se
                .read_with(cx, |s, _| (s.timing(), s.rows().len()))
                .ok()
                .and_then(|(t, n)| t.map(|t| (t, n)))
            {
                search.push(ms(t));
                rows = n;
            }
            sleep(cx, KEY_INTERVAL).await;
        }
        let _ = cx.update_window(window.into(), |_, window, cx| {
            let _ = ui::command::invoke("search-everywhere.close", json!(null), window, cx);
        });
        sleep(cx, Duration::from_millis(300)).await;
    }
    env::marker(&format!(
        "go_to_file {}",
        json!({
            "frames": frame_source(display),
            "query": query,
            "index_entries": entries,
            "index_build_ms": build,
            "keys": RUNS * query.chars().count(),
            "frames_missing": missed,
            "rows": rows,
            "key_to_rows_ms": stats(latency),
            "search_ms": stats(search),
            "key_handling_ms": stats(handling),
        })
    ));
}

/// Waits until the window has handled a mouse move to `p` (posted events
/// are handled in the order the run loop picks), then a little longer.
async fn moved_to(
    window: WindowHandle<Workbench>,
    p: gpui::Point<gpui::Pixels>,
    cx: &mut AsyncApp,
) {
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        let at = cx
            .update_window(window.into(), |_, window, _| window.mouse_position())
            .ok();
        if at.is_some_and(|at| {
            (at.x - p.x).abs() < gpui::px(0.25) && (at.y - p.y).abs() < gpui::px(0.25)
        }) {
            break;
        }
        sleep(cx, Duration::from_millis(5)).await;
    }
    sleep(cx, Duration::from_millis(50)).await;
}

/// The system cursor over text, gutter, splitter and toolbar: each place is
/// reached with mouse moves posted to the application's event queue (the
/// path of a real mouse move through AppKit), and the cursor shown is read
/// back after each.
async fn cursor(window: WindowHandle<Workbench>, cx: &mut AsyncApp) {
    if !settle(window, cx, Duration::from_secs(60)).await {
        env::marker("cursor {\"error\": \"the file did not open\"}");
        return;
    }
    sleep(cx, Duration::from_secs(1)).await;
    let Some(editor) = active_editor(window, cx) else {
        return;
    };
    let Some((origin, lh, cw)) = editor
        .read_with(cx, |e, _| e.text_geometry())
        .ok()
        .flatten()
    else {
        return;
    };
    let (splitter_x, height) = window
        .update(cx, |w, window, cx| {
            let theme = ui::theme::ThemeExt::theme(&**cx);
            let stripe = theme.metric(themes::Metric::StripeWidth);
            let gap = theme.metric(themes::Metric::IslandGap);
            (
                stripe + gpui::px(0.5) + w.project_width() + gap / 2.0,
                window.viewport_size().height,
            )
        })
        .unwrap_or((gpui::px(0.0), gpui::px(0.0)));
    let y = origin.y + lh * 2.5;
    let places = [
        ("text", gpui::point(origin.x + cw * 3.0, y), "ibeam"),
        ("gutter", gpui::point(origin.x - gpui::px(24.0), y), "arrow"),
        (
            "splitter",
            gpui::point(splitter_x, height / 2.0),
            "resize-left-right",
        ),
        (
            "toolbar",
            gpui::point(gpui::px(300.0), gpui::px(20.0)),
            "arrow",
        ),
        (
            "text-again",
            gpui::point(origin.x + cw * 5.0, y + lh),
            "ibeam",
        ),
    ];
    let active = cx
        .update_window(window.into(), |_, window, _| window.is_window_active())
        .unwrap_or(false);
    let mut results = Vec::new();
    for (name, position, expected) in places {
        let mut posted = Vec::new();
        for step in 0..3 {
            let p = gpui::point(position.x + gpui::px(step as f32 * 0.5), position.y);
            let _ = cx.update_window(window.into(), |_, window, _| {
                crate::shot::post_mouse_move(window, p)
            });
            moved_to(window, p, cx).await;
            posted.push(crate::shot::current_cursor());
        }
        // What the frame view's tracking area does on a real move, then the
        // next move in the same place reaching GPUI.
        let p = gpui::point(position.x - gpui::px(0.5), position.y);
        let _ = cx.update_window(window.into(), |_, window, _| {
            crate::shot::frame_view_mouse_moved(window, p)
        });
        sleep(cx, Duration::from_millis(150)).await;
        let after_frame_view = crate::shot::current_cursor();
        let _ = cx.update_window(window.into(), |_, window, _| {
            crate::shot::post_mouse_move(window, p)
        });
        moved_to(window, p, cx).await;
        let after_frame_view_and_move = crate::shot::current_cursor();
        // The system shows the arrow behind GPUI's back, then the pointer
        // moves on inside the same element.
        crate::shot::system_sets_arrow();
        let p = gpui::point(position.x - gpui::px(1.0), position.y);
        let _ = cx.update_window(window.into(), |_, window, _| {
            crate::shot::post_mouse_move(window, p)
        });
        moved_to(window, p, cx).await;
        let after_system_arrow_and_move = crate::shot::current_cursor();
        let system = cx
            .update_window(window.into(), |_, window, _| {
                crate::shot::system_cursor(window)
            })
            .unwrap_or_default();
        results.push(json!({
            "system": system,
            "place": name,
            "x": f32::from(position.x),
            "y": f32::from(position.y),
            "expected": expected,
            "after_posted_moves": posted,
            "after_frame_view_move": after_frame_view,
            "after_frame_view_move_then_move": after_frame_view_and_move,
            "after_system_arrow_then_move": after_system_arrow_and_move,
        }));
    }
    let views = cx
        .update_window(window.into(), |_, window, _| {
            crate::shot::tracking_areas(window)
        })
        .unwrap_or_default();
    env::marker(&format!(
        "cursor {}",
        json!({ "window_active": active, "places": results, "views": views })
    ));
}

/// Completion items of the bench service, and the popups measured.
pub const COMPLETION_ITEMS: usize = 200;
const COMPLETION_RUNS: usize = 30;

/// The completion popup: `.` typed after a name at the end of the open
/// file opens it (the service's trigger char); from the key to the first
/// frame that draws the popup, and from the service's answer (`latency`
/// ms after the request) to that frame, the client's part, which the
/// budget limits to 100 ms (docs/performance.md).
async fn completion(window: WindowHandle<Workbench>, latency: u64, cx: &mut AsyncApp) {
    if !settle(window, cx, Duration::from_secs(120)).await {
        env::marker("completion {\"error\": \"the file did not open\"}");
        return;
    }
    let Some(editor) = active_editor(window, cx) else {
        return;
    };
    // A line of its own at the end of the file, the caret after a name.
    let _ = editor.update(cx, |e, cx| {
        let end = e.buffer().len_chars();
        e.set_selections(text::SelectionSet::caret(end), cx);
        e.insert("\nbench_object", cx);
    });
    sleep(cx, Duration::from_secs(2)).await;
    let display = display_frames(window, cx).await;
    let key = |name: &str| Keystroke::parse(name).ok();
    let (Some(dot), Some(escape), Some(backspace)) = (key("."), key("escape"), key("backspace"))
    else {
        return;
    };
    let mut key_to_popup = Vec::new();
    let mut answer_to_popup = Vec::new();
    let mut missed = 0;
    for _ in 0..COMPLETION_RUNS {
        if !display {
            force_frame(window, cx);
        }
        let sent = Instant::now();
        let _ = cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(dot.clone(), cx);
        });
        let handled = Instant::now();
        if !display {
            force_frame(window, cx);
        }
        let until = Instant::now() + Duration::from_secs(5);
        let mut shown = None;
        while Instant::now() < until {
            sleep(cx, Duration::from_millis(1)).await;
            let answered = code_intel_ui::bench::last_answer().filter(|a| *a >= sent);
            if !display && answered.is_some() {
                force_frame(window, cx);
            }
            if let Some(f) = frames()
                .into_iter()
                .find(|f| f.start >= handled && f.completion)
            {
                shown = Some((f, answered));
                break;
            }
        }
        match shown {
            Some((f, answered)) => {
                key_to_popup.push(ms(f.end - sent));
                let answered = answered.or(code_intel_ui::bench::last_answer());
                if let Some(a) = answered.filter(|a| *a <= f.end) {
                    answer_to_popup.push(ms(f.end - a));
                }
            }
            None => missed += 1,
        }
        let _ = cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(escape.clone(), cx);
            window.dispatch_keystroke(backspace.clone(), cx);
        });
        sleep(cx, Duration::from_millis(300)).await;
    }
    // With `PYOXIDE_SHOT`: a picture of the popup.
    if let Some(path) = env::shot_path() {
        let _ = cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(dot.clone(), cx);
        });
        sleep(cx, Duration::from_millis(latency + 500)).await;
        let _ = cx.update_window(window.into(), |_, window, _| {
            crate::shot::request_display(window)
        });
        sleep(cx, Duration::from_millis(300)).await;
        let _ = cx.update_window(window.into(), |_, window, _| {
            match crate::shot::save(window, &path) {
                Ok((w, h)) => env::marker(&format!("shot {} {w}x{h}", path.display())),
                Err(e) => eprintln!("PYOXIDE_SHOT failed: {e}"),
            }
        });
    }
    env::marker(&format!(
        "completion {}",
        json!({
            "frames": frame_source(display),
            "latency_ms": latency,
            "items": COMPLETION_ITEMS,
            "popups_missing": missed,
            "key_to_popup_ms": stats(key_to_popup),
            "answer_to_popup_ms": stats(answer_to_popup),
        })
    ));
}
