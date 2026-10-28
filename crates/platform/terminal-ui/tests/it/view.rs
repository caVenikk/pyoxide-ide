//! The session view in a headless window with a real pseudo-terminal:
//! typed and composed text, keys, resizing, mouse selection, mouse
//! reporting with Shift selecting instead, the wheel on the main and the
//! alternate screen, the copies drawn at once (the owner asked to redraw)
//! and the slower copies of a flood of output. `cat`
//! stands in for a shell: it echoes what it gets
//! through the terminal's line discipline (control characters as `^X`),
//! so what the view sent can be read off the screen.
#![cfg(unix)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Entity, EntityInputHandler as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px, size,
};
use serde_json::Value;
use terminal::start::{Os, environment};
use terminal::{SessionOptions, ShellCommand, TerminalSize};
use terminal_ui::{FLOOD_AFTER, FLOOD_FRAME, SessionView, SessionViewEvent};

pub(crate) fn options(program: &str, args: &[&str]) -> SessionOptions {
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

pub(crate) fn open<'a>(
    cx: &'a mut TestAppContext,
    program: &str,
    args: &[&str],
) -> (Entity<SessionView>, &'a mut VisualTestContext) {
    open_with(cx, options(program, args))
}

pub(crate) fn open_with(
    cx: &mut TestAppContext,
    options: SessionOptions,
) -> (Entity<SessionView>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        terminal_ui::init(cx);
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let view = SessionView::new(options, window, cx);
        window.focus(&gpui::Focusable::focus_handle(&view, cx));
        view
    });
    wait(cx, &view, "the session to start", |s| s["started"] == true);
    (view, cx)
}

/// Runs the background work (the view's copies of the screen) and draws a
/// frame.
pub(crate) fn frame(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
}

/// Draws frames until `f` accepts the view's state (at most about 20 s).
pub(crate) fn wait(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    what: &str,
    f: impl Fn(&Value) -> bool,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..2000 {
        frame(cx);
        last = view.read_with(cx, |v, _| v.state());
        if f(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}: {last:#}");
}

/// How many keys are typed into a flood.
const KEYS: usize = 5;

/// How many times a stretch is measured before the output is too uneven
/// (the shell not scheduled, frames late) for one the rates are defined for.
const TRIES: usize = 50;

/// What the view did while frames were drawn for a while.
#[derive(Clone, Copy, Debug)]
struct Stretch {
    /// Measured, not the time asked for: a frame may take long.
    elapsed: Duration,
    /// Copies of the screen the view made.
    copies: u64,
    /// Requests of the view to draw the window at once.
    redraws: u64,
    /// Whether the output kept coming from the start to the end: no pause
    /// in it, no key (a frame or the shell's output late by more than
    /// [`FLOOD_FRAME`] ends the flood in the view, which then copies and
    /// draws as for any output).
    unbroken: bool,
}

impl Stretch {
    /// The most copies a flood allows in the time: one per [`FLOOD_FRAME`]
    /// (the real clock, whatever the frames did), and two at the ends.
    fn most_in_a_flood(&self) -> u64 {
        (self.elapsed.as_millis() / FLOOD_FRAME.as_millis()) as u64 + 2
    }
}

/// Draws frames for `time` and tells what the view did meanwhile.
fn draw_frames(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    redraws: &Cell<u64>,
    time: Duration,
) -> Stretch {
    let (_, copies_before) = view.read_with(cx, |v, _| v.counters());
    let redraws_before = redraws.get();
    let busy_before = view.read_with(cx, |v, _| v.busy_since());
    let start = Instant::now();
    while start.elapsed() < time {
        frame(cx);
        std::thread::sleep(Duration::from_millis(2));
    }
    let (_, copies_after) = view.read_with(cx, |v, _| v.counters());
    let busy_after = view.read_with(cx, |v, _| v.busy_since());
    Stretch {
        elapsed: start.elapsed(),
        copies: copies_after - copies_before,
        redraws: redraws.get() - redraws_before,
        // Ending a flood and starting it anew makes a new instant.
        unbroken: busy_before.is_some() && busy_before == busy_after,
    }
}

/// Draws frames until the view takes the output for a flood.
fn wait_for_flood(cx: &mut VisualTestContext, view: &Entity<SessionView>) {
    for _ in 0..2000 {
        frame(cx);
        let busy = view.read_with(cx, |v, _| v.busy_since());
        if busy.is_some_and(|since| since.elapsed() >= FLOOD_AFTER) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for the view to take the output for a flood");
}

/// Measures until a stretch is one the rates are defined for. Output that
/// did not keep coming (the shell was not scheduled for a while, a frame
/// took long) is not a flood and says nothing about the rates: that
/// stretch is measured again. A stretch that is whole is judged by the
/// caller, always; one that never is fails the test with what it saw.
fn undisturbed(
    what: &str,
    whole: impl Fn(&Stretch) -> bool,
    mut measure: impl FnMut() -> Stretch,
) -> Stretch {
    let mut last = None;
    for _ in 0..TRIES {
        let stretch = measure();
        if whole(&stretch) {
            return stretch;
        }
        last = Some(stretch);
    }
    panic!("no {what} that was whole in {TRIES} tries, the last: {last:?}");
}

/// Types a key at the moment the view takes the output for a flood, and
/// checks that the key ended it. A flood that ended meanwhile (a stall of
/// the machine) says nothing about the key, so the key is only typed into
/// one.
fn type_into_a_flood(cx: &mut VisualTestContext, view: &Entity<SessionView>) {
    for _ in 0..TRIES {
        wait_for_flood(cx, view);
        let typed = view.update(cx, |v, cx| {
            let flooding = v
                .busy_since()
                .is_some_and(|since| since.elapsed() >= FLOOD_AFTER);
            if flooding {
                v.type_text("x", cx);
                assert!(
                    v.busy_since().is_none(),
                    "a key typed into a flood did not end it"
                );
            }
            flooding
        });
        if typed {
            return;
        }
    }
    panic!("no key typed into a flood in {TRIES} tries");
}

/// Draws frames until the view asks to draw the window at once (more than
/// `before` times in all), which is how it shows the echo of a key. No
/// time is set for that: how long the echo takes depends on the machine
/// (the shell and the reader being scheduled). A flood never asks.
fn wait_for_redraw(cx: &mut VisualTestContext, redraws: &Cell<u64>, before: u64) {
    for _ in 0..2000 {
        frame(cx);
        if redraws.get() > before {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the echo of a key typed into a flood was not drawn at once");
}

/// Counts the view's requests to draw the window now.
fn count_redraws(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
) -> (Rc<Cell<u64>>, gpui::Subscription) {
    let count = Rc::new(Cell::new(0));
    let counter = count.clone();
    let subscription = cx.update(|_, cx| {
        cx.subscribe(view, move |_, event, _| {
            if *event == SessionViewEvent::Redraw {
                counter.set(counter.get() + 1);
            }
        })
    });
    (count, subscription)
}

pub(crate) fn has_line(s: &Value, f: impl Fn(&str) -> bool) -> bool {
    s["lines"]
        .as_array()
        .is_some_and(|l| l.iter().any(|l| f(l.as_str().unwrap())))
}

/// A point in the left half of a cell (a selection starts before it).
pub(crate) fn cell(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    line: usize,
    column: usize,
) -> Point<Pixels> {
    at(cx, view, line, column as f32 + 0.25)
}

/// A point in the right half of a cell (a selection ends after it).
pub(crate) fn cell_end(
    cx: &mut VisualTestContext,
    view: &Entity<SessionView>,
    line: usize,
    column: usize,
) -> Point<Pixels> {
    at(cx, view, line, column as f32 + 0.75)
}

pub(crate) fn at(
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

pub(crate) fn click(
    cx: &mut VisualTestContext,
    at: Point<Pixels>,
    count: usize,
    modifiers: Modifiers,
) {
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
    let (redraws, _subscription) = count_redraws(cx, &view);
    cx.simulate_input("hello");
    wait(cx, &view, "the echo", |s| has_line(s, |l| l == "hello"));
    // The echo's copy asks the owner to draw it at once.
    assert!(redraws.get() >= 1, "no redraw for the echo");
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

#[gpui::test]
fn a_flood_of_output_is_copied_ten_times_a_second(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "/usr/bin/yes", &["flood"]);
    wait(cx, &view, "the flood", |s| has_line(s, |l| l == "flood"));
    let (redraws, _subscription) = count_redraws(cx, &view);
    let second = Duration::from_secs(1);
    // Output that keeps coming slows the copies down (a frame is drawn
    // about every 2 ms here, so every frame could show a copy). A shell
    // that was not scheduled for a while made no output and its copies
    // stopped: that second says nothing either.
    let flood = undisturbed(
        "second of flood",
        |s| s.unbroken && s.copies >= 2,
        || {
            wait_for_flood(cx, &view);
            draw_frames(cx, &view, &redraws, second)
        },
    );
    assert!(
        flood.copies <= flood.most_in_a_flood(),
        "{} copies in {:?} of flood",
        flood.copies,
        flood.elapsed
    );
    // A flood's copies wait for the next frame: no window is drawn at once.
    assert_eq!(flood.redraws, 0, "redraws asked for while flooding");
    // A key ends the flood: the view takes the output for a flood again
    // only after it kept coming for a while, and the echo is copied without
    // the flood's wait and drawn at once (a flood only ever repaints at the
    // next frame). Each key is typed into a flood of its own. How long the
    // echo takes to come back depends on the machine (the shell and the
    // reader being scheduled), so no time is set for the draw: a view that
    // kept the flood going never asks for it.
    for _ in 0..KEYS {
        type_into_a_flood(cx, &view);
        wait_for_redraw(cx, &redraws, redraws.get());
    }
}
