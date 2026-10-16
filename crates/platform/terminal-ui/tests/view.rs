//! The session view in a headless window with a real pseudo-terminal:
//! typed and composed text, keys, resizing, mouse selection, mouse
//! reporting with Shift selecting instead, the wheel on the main and the
//! alternate screen. `cat` stands in for a shell: it echoes what it gets
//! through the terminal's line discipline (control characters as `^X`),
//! so what the view sent can be read off the screen.
#![cfg(unix)]

use std::time::Duration;

use gpui::{
    Entity, EntityInputHandler as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px, size,
};
use serde_json::Value;
use terminal::start::{Os, environment};
use terminal::{SessionOptions, ShellCommand, TerminalSize};
use terminal_ui::SessionView;

fn options(program: &str, args: &[&str]) -> SessionOptions {
    SessionOptions {
        command: ShellCommand {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        },
        working_directory: std::env::temp_dir(),
        env: environment(Os::current(), &[], &[]),
        size: TerminalSize::default(),
        scrollback_lines: 0,
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    program: &str,
    args: &[&str],
) -> (Entity<SessionView>, &'a mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        terminal_ui::init(cx);
    });
    let options = options(program, args);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let view = SessionView::new(options, window, cx);
        window.focus(&gpui::Focusable::focus_handle(&view, cx));
        view
    });
    wait(cx, &view, "the session to start", |s| s["started"] == true);
    (view, cx)
}

/// Draws frames (each takes the session's snapshot) until `f` accepts the
/// view's state (at most about 20 s).
fn wait(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    what: &str,
    f: impl Fn(&Value) -> bool,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..2000 {
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        last = view.read_with(cx, |v, _| v.state());
        if f(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}: {last:#}");
}

fn has_line(s: &Value, f: impl Fn(&str) -> bool) -> bool {
    s["lines"]
        .as_array()
        .is_some_and(|l| l.iter().any(|l| f(l.as_str().unwrap())))
}

/// A point in the left half of a cell (a selection starts before it).
fn cell(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    line: usize,
    column: usize,
) -> Point<Pixels> {
    at(cx, view, line, column as f32 + 0.25)
}

/// A point in the right half of a cell (a selection ends after it).
fn cell_end(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    line: usize,
    column: usize,
) -> Point<Pixels> {
    at(cx, view, line, column as f32 + 0.75)
}

fn at(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    line: usize,
    x: f32,
) -> Point<Pixels> {
    let g = view.read_with(cx, |v, _| v.geometry()).expect("laid out");
    point(
        g.origin.x + g.cell.width * x,
        g.origin.y + g.cell.height * (line as f32 + 0.5),
    )
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>, count: usize, modifiers: Modifiers) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers,
        button: MouseButton::Left,
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers,
        button: MouseButton::Left,
        click_count: count,
    });
}

#[gpui::test]
fn typed_text_keys_and_resizing_reach_the_session(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "/bin/cat", &[]);
    cx.simulate_input("hello");
    wait(cx, &view, "the echo", |s| has_line(s, |l| l == "hello"));
    // Enter: `cat` prints the line again.
    cx.simulate_keystrokes("enter");
    let s = wait(cx, &view, "the line printed", |s| {
        s["lines"][0] == "hello" && s["lines"][1] == "hello"
    });
    assert_eq!(s["cursor"]["line"], 2);
    // Control keys go to the session (echoed as ^X).
    cx.simulate_keystrokes("ctrl-x");
    wait(cx, &view, "^X", |s| has_line(s, |l| l == "^X"));
    // The grid follows the window's size.
    let before = view.read_with(cx, |v, _| v.state())["columns"]
        .as_u64()
        .unwrap();
    cx.simulate_resize(size(px(500.0), px(300.0)));
    let s = wait(cx, &view, "fewer columns", |s| {
        s["columns"].as_u64().is_some_and(|c| c < before)
    });
    let session_columns = view.read_with(cx, |v, _| v.session().unwrap().snapshot().columns);
    assert_eq!(s["columns"].as_u64(), Some(session_columns as u64));
}

#[gpui::test]
fn composed_text_is_shown_then_sent(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "/bin/cat", &[]);
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "に", None, window, cx);
            assert_eq!(v.marked_text_range(window, cx), Some(0..1));
        })
    });
    // Composing sends nothing.
    cx.update(|window, cx| window.draw(cx).clear());
    std::thread::sleep(Duration::from_millis(100));
    let s = wait(cx, &view, "a frame", |_| true);
    assert!(!has_line(&s, |l| l.contains('に')), "{s:#}");
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(v.marked_text_range(window, cx), None);
        })
    });
    wait(cx, &view, "the committed text", |s| {
        has_line(s, |l| l == "日本")
    });
}

#[gpui::test]
fn the_mouse_selects_words_lines_and_rectangles(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "/bin/cat", &[]);
    cx.simulate_input("hello world");
    cx.simulate_keystrokes("enter");
    wait(cx, &view, "two lines", |s| s["lines"][1] == "hello world");
    // Drag over `hello`.
    let (from, to) = (cell(cx, &view, 0, 0), cell_end(cx, &view, 0, 4));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    wait(cx, &view, "the dragged text", |s| s["selection"] == "hello");
    // A double click selects a word, a triple click the line.
    let p = cell(cx, &view, 0, 8);
    click(cx, p, 2, Modifiers::none());
    wait(cx, &view, "the word", |s| s["selection"] == "world");
    let p = cell(cx, &view, 1, 2);
    click(cx, p, 3, Modifiers::none());
    wait(cx, &view, "the line", |s| {
        s["selection"]
            .as_str()
            .is_some_and(|t| t.trim_end() == "hello world")
    });
    // Shift+click extends the selection.
    let p = cell(cx, &view, 0, 0);
    click(cx, p, 1, Modifiers::none());
    let p = cell_end(cx, &view, 0, 2);
    click(cx, p, 1, Modifiers::shift());
    wait(cx, &view, "the extended selection", |s| {
        s["selection"] == "hel"
    });
    // Alt (and Shift+Alt) drag: a rectangle over both lines.
    let alt = Modifiers {
        alt: true,
        shift: true,
        ..Modifiers::none()
    };
    let (from, to) = (cell(cx, &view, 0, 6), cell_end(cx, &view, 1, 8));
    cx.simulate_mouse_down(from, MouseButton::Left, alt);
    cx.simulate_mouse_move(to, MouseButton::Left, alt);
    cx.simulate_mouse_up(to, MouseButton::Left, alt);
    wait(cx, &view, "the rectangle", |s| s["selection"] == "wor\nwor");
}

#[gpui::test]
fn programs_get_the_mouse_unless_shift_is_held(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "/bin/cat", &[]);
    // `cat` prints the line back: its escape turns mouse reporting on.
    view.update(cx, |v, cx| v.type_text("\x1b[?1000h\r", cx));
    wait(cx, &view, "mouse reporting", |s| {
        s["mouse_reporting"] == true
    });
    let at = cell(cx, &view, 4, 2);
    click(cx, at, 1, Modifiers::none());
    // The press and the release reach `cat` and are echoed (ESC as ^[).
    wait(cx, &view, "the reports", |s| {
        has_line(s, |l| l.matches("^[[M").count() >= 2)
    });
    assert!(view.read_with(cx, |v, _| v.state())["selection"].is_null());
    // With Shift the view selects instead.
    let (from, to) = (cell(cx, &view, 0, 0), cell_end(cx, &view, 0, 3));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::shift());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::shift());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::shift());
    wait(cx, &view, "a selection", |s| !s["selection"].is_null());
}

#[gpui::test]
fn the_wheel_scrolls_the_history_or_sends_arrows(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "/bin/sh",
        &[
            "-c",
            "i=0; while [ $i -lt 300 ]; do echo line$i; i=$((i+1)); done; exec cat",
        ],
    );
    wait(cx, &view, "the lines", |s| has_line(s, |l| l == "line299"));
    let at = cell(cx, &view, 1, 1);
    let wheel = |cx: &mut VisualTestContext, lines: f32| {
        cx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Lines(point(0.0, lines)),
            ..ScrollWheelEvent::default()
        })
    };
    wheel(cx, 3.0);
    let s = wait(cx, &view, "scrolled back", |s| s["display_offset"] == 3);
    assert!(view.read_with(cx, |v, _| v.geometry().unwrap().scrollbar.is_some()));
    assert!(s["history"].as_u64().unwrap() > 200);
    wheel(cx, -3.0);
    wait(cx, &view, "at the bottom", |s| s["display_offset"] == 0);
    // On the alternate screen the wheel sends arrow keys (mode 1007).
    view.update(cx, |v, cx| v.type_text("\x1b[?1049h\r", cx));
    wait(cx, &view, "the alternate screen", |s| {
        s["alternate_screen"] == true
    });
    assert!(view.read_with(cx, |v, _| v.geometry().unwrap().scrollbar.is_none()));
    wheel(cx, 1.0);
    wait(cx, &view, "an arrow key", |s| {
        has_line(s, |l| l.contains("^[[A"))
    });
}
