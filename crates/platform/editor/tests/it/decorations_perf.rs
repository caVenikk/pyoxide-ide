//! Frame work per keystroke on a 100,000-line file with and without
//! decorations, headless (the test platform's window, no display):
//! `cargo test --release -p editor --test it decorations_perf:: -- --ignored --nocapture`.
//! It measures the editor's own work (key handling, prepaint, paint
//! into the scene), not the time to the display.

use std::time::{Duration, Instant};

use editor::commands::{SetSelection, Type};
use editor::decorations::layer;
use editor::{ChangeKind, Decoration, Editor, TextPiece};
use gpui::{Focusable, TestAppContext, VisualTestContext, px, size};
use text::Buffer;
use themes::{TextRole, UiRole};

const LINES: usize = 100_000;
const KEYS: usize = 300;

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

/// Types `KEYS` characters in the middle of the file, a frame after each
/// (every twentieth is a line break); the frame work per character and per
/// line break, sorted.
fn type_keys(cx: &mut VisualTestContext) -> (Vec<Duration>, Vec<Duration>) {
    let p = editor::commands::Position {
        line: LINES / 2,
        column: 4,
    };
    cx.dispatch_action(SetSelection { anchor: p, head: p });
    draw(cx);
    let mut chars = Vec::with_capacity(KEYS);
    let mut breaks = Vec::with_capacity(KEYS / 20);
    for i in 0..KEYS {
        let line_break = i % 20 == 19;
        let c = if line_break { "\n" } else { "x" };
        let start = Instant::now();
        cx.dispatch_action(Type { text: c.into() });
        draw(cx);
        let time = start.elapsed();
        if line_break {
            breaks.push(time);
        } else {
            chars.push(time);
        }
    }
    chars.sort();
    breaks.sort();
    (chars, breaks)
}

fn report(name: &str, times: &[Duration]) {
    println!(
        "{name}: p50 {:.2} ms, p90 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        percentile(times, 0.5).as_secs_f64() * 1e3,
        percentile(times, 0.9).as_secs_f64() * 1e3,
        percentile(times, 0.99).as_secs_f64() * 1e3,
        times.last().unwrap().as_secs_f64() * 1e3,
    );
}

/// Decorations as a debugging session with version control shows them:
/// breakpoints, change markers and inline values all over the file and
/// around the caret.
fn decorate(editor: &gpui::Entity<Editor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        let mut items = Vec::new();
        for i in 0..500 {
            let line = i * (LINES / 500) + 3;
            items.push((
                line..line + 1,
                Decoration::default()
                    .gutter_icon(ui::icons::IconName::BREAKPOINT, None)
                    .background(UiRole::Breakpoint, layer::BREAKPOINT)
                    .stripe(UiRole::DiagnosticErrorStripe, false),
            ));
        }
        for i in 0..1000 {
            let line = i * (LINES / 1000) + 7;
            let kind = [ChangeKind::Added, ChangeKind::Modified, ChangeKind::Deleted][i % 3];
            let lines = if kind == ChangeKind::Deleted {
                line..line
            } else {
                line..line + 2
            };
            items.push((
                lines,
                Decoration::default()
                    .change(kind)
                    .stripe(UiRole::GutterModified, true),
            ));
        }
        // Inline values on the lines around the caret.
        for line in LINES / 2 - 20..LINES / 2 + 20 {
            items.push((
                line..line + 1,
                Decoration::default().after_text(
                    vec![
                        TextPiece::new("value: ", TextRole::InlayHint).italic(),
                        TextPiece::new(format!("{line}"), TextRole::InlayHint).italic(),
                    ],
                    true,
                ),
            ));
        }
        e.set_decorations("perf", items, cx);
    });
    draw(cx);
}

#[gpui::test]
#[ignore = "timing; run with --release --ignored --nocapture"]
fn keystroke_frame_work_with_and_without_decorations(cx: &mut TestAppContext) {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let text: String = (0..LINES)
        .map(|i| format!("    value_{i} = compute({i}, \"text\")  # line {i}\n"))
        .collect();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(Buffer::new(&text), window, cx));
    cx.simulate_resize(size(px(1200.0), px(900.0)));
    cx.update(|window, cx| {
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
    });
    draw(cx);
    // Warm up the glyph caches.
    let _ = type_keys(cx);
    let (plain, plain_breaks) = type_keys(cx);
    decorate(&editor, cx);
    let (decorated, decorated_breaks) = type_keys(cx);
    report("characters, without decorations", &plain);
    report("characters, with 1540 decorations", &decorated);
    report("line breaks, without decorations", &plain_breaks);
    report("line breaks, with 1540 decorations", &decorated_breaks);
    let count = editor.read_with(cx, |e, _| e.decorations_of("perf").len());
    println!("decorations kept through the typing: {count}");
}
