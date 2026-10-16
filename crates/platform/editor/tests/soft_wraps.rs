//! Soft wraps (tests/scenarios/editor/soft-wraps.md): lines wider than the
//! view go on in the next visual rows; the caret moves by rows, Home and
//! End stop at the ends of rows, a click past a row's end puts the caret
//! there, edits wrap their lines again, and large texts are wrapped off
//! the UI thread.

use editor::Editor;
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, point, px, size,
};
use serde_json::{Value, json};
use text::{Buffer, SelectionSet};
use themes::Metric;
use ui::theme::ThemeExt as _;

struct T<'a> {
    editor: Entity<Editor>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> T<'a> {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let text = text.to_owned();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(Buffer::new(&text), window, cx));
    cx.update(|window, cx| {
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
    });
    let mut t = T { editor, cx };
    t.draw();
    t
}

impl T<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }

    fn state(&mut self) -> Value {
        self.editor.read_with(self.cx, |e, _| e.state_json())
    }

    fn run(&mut self, id: &str) {
        self.cx
            .update(|window, cx| ui::command::invoke(id, json!(null), window, cx))
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
    }

    /// Makes the view `cols` columns wide for soft wraps.
    fn width(&mut self, cols: usize) {
        let (origin, _, char_width) = self.geometry();
        let bar = self
            .cx
            .update(|_, cx| cx.theme().metric(Metric::ScrollbarWidth));
        let w = origin.x + char_width * (cols as f32 + 0.5) + bar;
        self.cx.simulate_resize(size(w, px(400.0)));
        self.draw();
    }

    fn geometry(&mut self) -> (gpui::Point<gpui::Pixels>, gpui::Pixels, gpui::Pixels) {
        self.editor
            .read_with(self.cx, |e, _| e.text_geometry().expect("drawn"))
    }

    fn wrap(&mut self, cols: usize) {
        self.width(cols);
        self.run("editor.toggle-soft-wraps");
        assert_eq!(self.state()["soft_wraps"], true);
        assert_eq!(self.state()["wrap_columns"], cols);
    }

    fn caret(&mut self, offset: usize) {
        self.editor.update(self.cx, |e, cx| {
            e.set_selections(SelectionSet::caret(offset), cx)
        });
        self.draw();
    }

    fn head(&mut self) -> usize {
        self.editor
            .read_with(self.cx, |e, _| e.selections().primary().head)
    }

    fn row(&mut self) -> u64 {
        self.state()["caret_row"].as_u64().unwrap()
    }

    fn caret_bounds(&mut self) -> gpui::Bounds<gpui::Pixels> {
        self.editor
            .read_with(self.cx, |e, _| e.caret_bounds().unwrap())
    }

    fn rows(&mut self) -> u64 {
        self.state()["rows"].as_u64().unwrap()
    }
}

/// A line of 60 chars, then two short lines. At 25 columns (the last kept
/// for the wrap sign) the first line takes three rows: "    alpha beta
/// gamma ", "delta epsilon zeta " and "eta theta iota kappa", the last two
/// indented 5 columns (the sign and the line's indent).
const TEXT: &str = "    alpha beta gamma delta epsilon zeta eta theta iota kappa\nshort\nend\n";
const ALPHA: usize = 4;
const BETA: usize = 10;
const DELTA: usize = 21;
const EPSILON: usize = 27;
const ZETA: usize = 35;
const ETA: usize = 40;
const LINE_END: usize = 60;
const SHORT: usize = 61;

#[gpui::test]
fn the_command_wraps_long_lines_into_rows_and_back(cx: &mut TestAppContext) {
    let mut t = open(cx, TEXT);
    assert_eq!(t.state()["soft_wraps"], false);
    assert_eq!(t.rows(), 4);
    t.wrap(25);
    // The long line takes three rows.
    assert_eq!(t.rows(), 6);
    // The second line starts below them.
    t.caret(SHORT);
    assert_eq!(t.row(), 3);
    let (origin, lh, _) = t.geometry();
    assert_eq!(t.caret_bounds().top(), origin.y + lh * 3.0);
    t.run("editor.toggle-soft-wraps");
    assert_eq!(t.state()["soft_wraps"], false);
    assert_eq!(t.rows(), 4);
    assert_eq!(t.row(), 1);
}

#[gpui::test]
fn rows_follow_the_width_and_the_text(cx: &mut TestAppContext) {
    let mut t = open(cx, TEXT);
    t.wrap(25);
    assert_eq!(t.rows(), 6);
    // Wider: one row less.
    t.width(40);
    assert_eq!(t.state()["wrap_columns"], 40);
    assert_eq!(t.rows(), 5);
    // Typing at the end of a short line until it wraps adds a row; undo
    // takes it back.
    t.caret(SHORT + 5);
    t.cx.simulate_input(" and a few more words to fill the row up");
    t.draw();
    assert_eq!(t.rows(), 6);
    t.run("editor.undo");
    assert_eq!(t.rows(), 5);
    // A new line in the middle of the long one.
    t.width(25);
    t.caret(DELTA);
    t.run("editor.newline");
    assert_eq!(t.state()["lines"], 5);
    assert_eq!(t.rows(), 6);
}

#[gpui::test]
fn up_and_down_move_by_rows_keeping_the_column(cx: &mut TestAppContext) {
    let mut t = open(cx, TEXT);
    t.wrap(25);
    // On "beta" in the first row.
    t.caret(BETA + 1);
    let x = t.caret_bounds().left();
    t.run("editor.move-down");
    assert_eq!(t.row(), 1);
    assert_eq!(t.caret_bounds().left(), x);
    // Column 11 of the second row: "epsilon" after the indent and "delta ".
    assert_eq!(t.head(), EPSILON);
    t.run("editor.move-down");
    assert_eq!(t.row(), 2);
    assert_eq!(t.head(), ETA + 6);
    t.run("editor.move-down");
    // "short" is shorter: its end.
    assert_eq!(t.head(), SHORT + 5);
    assert_eq!(t.row(), 3);
    t.run("editor.move-up");
    assert_eq!(t.row(), 2);
    assert_eq!(t.caret_bounds().left(), x);
    // Up from the indent of a continuation row goes to the row above.
    t.run("editor.move-up");
    t.run("editor.move-up");
    assert_eq!(t.row(), 0);
    assert_eq!(t.head(), BETA + 1);
    // Selecting down extends by a row.
    t.run("editor.select-down");
    let s = t.state();
    assert_eq!(s["caret_row"], 1);
    assert_eq!(s["selections"][0]["anchor"]["column"], 11);
}

#[gpui::test]
fn end_stops_at_the_row_end_then_the_line_end(cx: &mut TestAppContext) {
    let mut t = open(cx, TEXT);
    t.wrap(25);
    t.caret(ALPHA);
    let top = t.caret_bounds().top();
    t.run("editor.move-line-end");
    // At the wrap, drawn at the end of the first row.
    assert_eq!(t.head(), DELTA);
    assert_eq!(t.row(), 0);
    assert_eq!(t.caret_bounds().top(), top);
    t.run("editor.move-line-end");
    assert_eq!(t.head(), LINE_END);
    assert_eq!(t.row(), 2);
    // Down from the end of the first row keeps that column.
    t.caret(ALPHA);
    t.run("editor.move-line-end");
    t.run("editor.move-down");
    assert_eq!(t.row(), 1);
    // Column 21 of the second row: "zeta" after "delta epsilon ".
    assert_eq!(t.head(), ZETA + 2);
    // End with Shift selects to the row end.
    t.caret(ALPHA);
    t.run("editor.select-line-end");
    let s = t.state();
    assert_eq!(s["selected_chars"], DELTA - ALPHA);
    assert_eq!(s["caret_row"], 0);
}

#[gpui::test]
fn home_stops_at_the_row_start_then_the_line_text(cx: &mut TestAppContext) {
    let mut t = open(cx, TEXT);
    t.wrap(25);
    t.caret(EPSILON + 2);
    assert_eq!(t.row(), 1);
    t.run("editor.move-line-start");
    assert_eq!(t.head(), DELTA);
    assert_eq!(t.row(), 1);
    t.run("editor.move-line-start");
    assert_eq!(t.head(), ALPHA);
    assert_eq!(t.row(), 0);
    t.run("editor.move-line-start");
    assert_eq!(t.head(), 0);
}

#[gpui::test]
fn a_click_past_a_row_end_puts_the_caret_at_that_end(cx: &mut TestAppContext) {
    let mut t = open(cx, TEXT);
    t.wrap(25);
    let (origin, lh, cw) = t.geometry();
    // Far right on the first row (before the scrollbar).
    let at = point(origin.x + cw * 23.2, origin.y + lh * 0.5);
    t.cx.simulate_event(MouseDownEvent {
        position: at,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    t.cx.simulate_event(MouseUpEvent {
        position: at,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    t.draw();
    assert_eq!(t.head(), DELTA);
    assert_eq!(t.row(), 0);
    // On the second row, in its indent: the row's first char.
    let at = point(origin.x + cw * 1.0, origin.y + lh * 1.5);
    t.cx.simulate_event(MouseDownEvent {
        position: at,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    t.cx.simulate_event(MouseUpEvent {
        position: at,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    t.draw();
    assert_eq!(t.head(), DELTA);
    assert_eq!(t.row(), 1);
}

#[gpui::test]
fn scrolling_counts_rows(cx: &mut TestAppContext) {
    let long = "word ".repeat(30);
    let text = format!("{long}\n").repeat(100);
    let mut t = open(cx, &text);
    t.wrap(40);
    let rows = t.rows();
    assert!(rows > 300, "{rows}");
    let lh = t.geometry().1;
    t.cx.update(|window, cx| {
        ui::command::invoke("editor.scroll-to", json!({ "line": 50 }), window, cx).unwrap()
    });
    t.draw();
    // Five rows a line, then the empty last line.
    assert_eq!(rows, 501);
    let top = t.editor.read_with(t.cx, |e, _| e.scroll_top());
    assert_eq!(top, lh * 250.0);
    // The last line's last row is the last row.
    t.run("editor.move-document-end");
    assert_eq!(t.row(), rows - 1);
    t.run("editor.move-up");
    assert_eq!(t.row(), rows - 2);
}

#[gpui::test]
fn large_texts_wrap_in_the_background(cx: &mut TestAppContext) {
    // About 600 KB: 6,000 lines of 98 chars, each two rows at 60 columns.
    let line = format!("{} end", "abcd ".repeat(19));
    let text = format!("{line}\n").repeat(6000);
    let mut t = open(cx, &text);
    t.width(60);
    t.editor.update(t.cx, |e, cx| e.set_soft_wraps(true, cx));
    // An edit before the rows arrive: the first line becomes short.
    t.editor.update(t.cx, |e, cx| {
        e.set_selections(SelectionSet::single(text::Selection::new(5, 98)), cx);
        e.insert("", cx);
    });
    assert_eq!(t.state()["wrap_columns"], Value::Null);
    t.draw();
    assert_eq!(t.state()["wrap_columns"], 60);
    // One row, two for each other line, one for the empty last line.
    assert_eq!(t.rows(), 1 + 2 * 5999 + 1);
}

#[gpui::test]
fn signs_show_on_the_caret_line_and_the_indent_follows_the_option(cx: &mut TestAppContext) {
    let text = format!("{TEXT}{}", TEXT.replace("    ", ""));
    let mut t = open(cx, &text);
    t.wrap(25);
    // The caret's line: two wraps, each with a sign before and after.
    t.caret(EPSILON);
    assert_eq!(t.state()["wrap_signs"], 4);
    // On a short line: none.
    t.caret(SHORT);
    assert_eq!(t.state()["wrap_signs"], 0);
    // Every line's with "show all soft wraps".
    t.editor
        .update(t.cx, |e, cx| e.set_show_all_soft_wraps(true, cx));
    t.draw();
    let all = t.state()["wrap_signs"].as_u64().unwrap();
    assert!(all > 4, "{all}");
    // The second row starts after the sign and the line's indent, or after
    // the sign only without the original indent.
    let (origin, _, cw) = t.geometry();
    t.caret(DELTA);
    assert_eq!(t.caret_bounds().left(), origin.x + cw * 5.0);
    t.editor
        .update(t.cx, |e, cx| e.set_soft_wrap_indent(None, cx));
    t.draw();
    t.caret(DELTA + 2);
    let p = t.state()["caret_row"].as_u64().unwrap();
    assert_eq!(p, 1);
    let left = t.caret_bounds().left();
    t.run("editor.move-line-start");
    assert!(t.caret_bounds().left() < left);
    // The row starts after the sign.
    let x = f32::from(t.caret_bounds().left() - origin.x);
    assert!((x - f32::from(cw)).abs() < 0.01, "{x}");
    // A range over a wrap: its box spans both rows.
    let b = t
        .editor
        .read_with(t.cx, |e, _| e.bounds_for_range(ALPHA..EPSILON).unwrap());
    let lh = t.geometry().1;
    assert_eq!(b.size.height, lh * 2.0);
}

#[gpui::test]
fn many_edits_at_once_wrap_all_lines_again(cx: &mut TestAppContext) {
    // 100 short lines; a caret at the end of each; typing makes each line
    // wrap.
    let carets = || {
        SelectionSet::from_vec(
            (0..100)
                .map(|i| text::Selection::caret(i * 11 + 10))
                .collect(),
        )
    };
    let text = "abcdefghij\n".repeat(100);
    let mut t = open(cx, &text);
    t.wrap(25);
    assert_eq!(t.rows(), 101);
    t.editor.update(t.cx, |e, cx| {
        e.set_selections(carets(), cx);
        e.insert(" and some more words", cx);
    });
    t.draw();
    assert_eq!(t.rows(), 201);
    t.run("editor.undo");
    assert_eq!(t.rows(), 101);
    // The same in a large text while its rows are computed in the
    // background.
    let big = "abcdefghij\n".repeat(30_000);
    t.editor.update(t.cx, |e, cx| {
        *e.buffer_mut() = Buffer::new(&big);
        e.buffer_reloaded(cx);
        e.set_selections(carets(), cx);
        e.insert(" and some more words", cx);
    });
    t.draw();
    assert_eq!(t.rows(), 30_001 + 100);
}
