//! Headless editor tests (GPUI's test platform): typing through the input
//! handler, commands dispatched like key bindings or invoked by id, mouse
//! input, clipboard, undo, and virtualized painting of a large file.

use editor::commands::*;
use editor::{Editor, EditorConfig};
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ScrollDelta,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
};
use serde_json::json;
use text::{Buffer, Point};

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<Editor>, &'a mut VisualTestContext) {
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
    draw(cx);
    (editor, cx)
}

/// Renders a frame, as the display link would.
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, _| e.text())
}

fn caret(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Point {
    editor.read_with(cx, |e, _| e.caret_point())
}

fn act(cx: &mut VisualTestContext, action: impl gpui::Action) {
    cx.dispatch_action(action);
    draw(cx);
}

fn invoke(cx: &mut VisualTestContext, id: &str, args: serde_json::Value) -> serde_json::Value {
    let out = cx
        .update(|window, cx| ui::command::invoke(id, args, window, cx))
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx);
    out
}

fn select(cx: &mut VisualTestContext, anchor: (usize, usize), head: (usize, usize)) {
    invoke(
        cx,
        "editor.set-selection",
        json!({"anchor": {"line": anchor.0, "column": anchor.1}, "head": {"line": head.0, "column": head.1}}),
    );
}

#[gpui::test]
fn typing_goes_through_the_input_handler(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "");
    cx.simulate_input("hello");
    draw(cx);
    assert_eq!(text(&editor, cx), "hello");
    assert_eq!(caret(&editor, cx), Point::new(0, 5));
    assert!(editor.read_with(cx, |e, _| e.is_modified()));
}

#[gpui::test]
fn typing_is_undone_as_one_group(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "");
    cx.simulate_input("hello world");
    draw(cx);
    act(cx, Undo);
    assert_eq!(text(&editor, cx), "");
    act(cx, Redo);
    assert_eq!(text(&editor, cx), "hello world");
    // A caret move starts a new group.
    act(cx, MoveLeft);
    cx.simulate_input("X");
    act(cx, Undo);
    assert_eq!(text(&editor, cx), "hello world");
    assert_eq!(caret(&editor, cx), Point::new(0, 10));
    act(cx, Undo);
    assert_eq!(text(&editor, cx), "");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));
}

#[gpui::test]
fn enter_keeps_and_adds_indentation(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "def f():\n    x = 1");
    editor.update(cx, |e, _| {
        e.set_config(EditorConfig {
            indent_after: vec![':'],
            ..EditorConfig::default()
        })
    });
    select(cx, (0, 8), (0, 8));
    act(cx, Newline);
    assert_eq!(text(&editor, cx), "def f():\n    \n    x = 1");
    assert_eq!(caret(&editor, cx), Point::new(1, 4));
    select(cx, (2, 9), (2, 9));
    act(cx, Newline);
    assert_eq!(caret(&editor, cx), Point::new(3, 4));
    // Backspace in leading spaces goes back one indentation level.
    act(cx, Backspace);
    assert_eq!(caret(&editor, cx), Point::new(3, 0));
}

#[gpui::test]
fn caret_moves(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "    foo.bar baz\nab\nlonger line here");
    select(cx, (0, 0), (0, 0));
    act(cx, MoveLineStart);
    assert_eq!(caret(&editor, cx), Point::new(0, 4));
    act(cx, MoveLineStart);
    assert_eq!(caret(&editor, cx), Point::new(0, 0));
    act(cx, MoveLineEnd);
    assert_eq!(caret(&editor, cx), Point::new(0, 15));
    act(cx, MoveWordLeft);
    assert_eq!(caret(&editor, cx), Point::new(0, 12));
    // Vertical moves keep the goal column across a short line.
    act(cx, MoveDown);
    assert_eq!(caret(&editor, cx), Point::new(1, 2));
    act(cx, MoveDown);
    assert_eq!(caret(&editor, cx), Point::new(2, 12));
    act(cx, MoveDocumentStart);
    assert_eq!(caret(&editor, cx), Point::new(0, 0));
    act(cx, MoveDocumentEnd);
    assert_eq!(caret(&editor, cx), Point::new(2, 16));
    act(cx, MoveDown);
    assert_eq!(caret(&editor, cx), Point::new(2, 16));
}

#[gpui::test]
fn shift_extends_and_plain_moves_collapse(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "hello world");
    select(cx, (0, 5), (0, 5));
    act(cx, SelectWordLeft);
    act(cx, SelectLeft);
    let s = editor.read_with(cx, |e, _| *e.selections().primary());
    assert_eq!((s.anchor, s.head), (5, 0));
    act(cx, SelectRight);
    let s = editor.read_with(cx, |e, _| *e.selections().primary());
    assert_eq!((s.anchor, s.head), (5, 1));
    act(cx, MoveRight);
    assert_eq!(caret(&editor, cx), Point::new(0, 5));
    act(cx, SelectAll);
    let state = invoke(cx, "editor.select-all", json!(null));
    assert_eq!(state, json!(null));
    act(cx, Type { text: "x".into() });
    assert_eq!(text(&editor, cx), "x");
}

#[gpui::test]
fn copy_cut_paste(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "hello world\nsecond");
    select(cx, (0, 6), (0, 11));
    act(cx, Copy);
    select(cx, (0, 11), (0, 11));
    act(cx, Paste);
    assert_eq!(text(&editor, cx), "hello worldworld\nsecond");
    // Without a selection, copy takes the line and paste puts it above.
    select(cx, (1, 3), (1, 3));
    act(cx, Copy);
    act(cx, Paste);
    assert_eq!(text(&editor, cx), "hello worldworld\nsecond\nsecond");
    assert_eq!(caret(&editor, cx), Point::new(2, 3));
    act(cx, Cut);
    assert_eq!(text(&editor, cx), "hello worldworld\nsecond");
    let clip = cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(clip.as_deref(), Some("second\n"));
}

#[gpui::test]
fn line_operations(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "a = 1\nb = 2\nc = 3");
    select(cx, (1, 2), (1, 2));
    act(cx, DuplicateLine);
    assert_eq!(text(&editor, cx), "a = 1\nb = 2\nb = 2\nc = 3");
    assert_eq!(caret(&editor, cx), Point::new(2, 2));
    act(cx, DeleteLine);
    assert_eq!(text(&editor, cx), "a = 1\nb = 2\nc = 3");
    assert_eq!(caret(&editor, cx), Point::new(2, 2));
    act(cx, DeleteLine);
    assert_eq!(text(&editor, cx), "a = 1\nb = 2");
    assert_eq!(caret(&editor, cx), Point::new(1, 2));
    // Tab on a multi-line selection indents the lines, Shift+Tab undoes it.
    select(cx, (0, 0), (1, 5));
    act(cx, Tab);
    assert_eq!(text(&editor, cx), "    a = 1\n    b = 2");
    act(cx, Unindent);
    assert_eq!(text(&editor, cx), "a = 1\nb = 2");
    select(cx, (0, 1), (0, 1));
    act(cx, Tab);
    // Spaces up to the next indentation stop (column 4).
    assert_eq!(text(&editor, cx), "a    = 1\nb = 2");
}

#[gpui::test]
fn input_method_composition(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (editor, cx) = open(cx, "ab");
    select(cx, (0, 1), (0, 1));
    editor.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
        assert_eq!(e.marked_text_range(window, cx), Some(1..2));
        e.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
        e.replace_text_in_range(None, "你", window, cx);
        assert_eq!(e.marked_text_range(window, cx), None);
    });
    draw(cx);
    assert_eq!(text(&editor, cx), "a你b");
    assert_eq!(caret(&editor, cx), Point::new(0, 2));
    act(cx, Undo);
    assert_eq!(text(&editor, cx), "ab");
}

#[gpui::test]
fn mouse_selects_and_scrolls(cx: &mut TestAppContext) {
    let lines: Vec<String> = (0..200).map(|i| format!("line {i} text")).collect();
    let (editor, cx) = open(cx, &lines.join("\n"));
    let (origin, lh, cw) = editor.read_with(cx, |e, _| e.text_geometry().unwrap());
    let at = |line: usize, col: usize| {
        point(
            origin.x + cw * col as f32 + px(1.0),
            origin.y + lh * line as f32 + lh / 2.0,
        )
    };
    cx.simulate_click(at(3, 5), Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), Point::new(3, 5));
    // Double click selects the word.
    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at(3, 1),
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at(3, 1),
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    draw(cx);
    let s = editor.read_with(cx, |e, _| e.selections().primary().range());
    let start = editor.read_with(cx, |e, _| e.buffer().line_start(3));
    assert_eq!(s, start..start + 4);
    // Scrolling the wheel moves the view, not the caret.
    cx.simulate_event(ScrollWheelEvent {
        position: at(5, 2),
        delta: ScrollDelta::Lines(point(0.0, -10.0)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    draw(cx);
    let top = editor.read_with(cx, |e, _| e.scroll_top());
    assert_eq!(top, lh * 10.0);
    assert_eq!(caret(&editor, cx).line, 3);
}

#[gpui::test]
fn a_large_file_paints_only_visible_lines(cx: &mut TestAppContext) {
    let lines: Vec<String> = (0..100_000)
        .map(|i| format!("def f{i}(x):\n    return x + {i}"))
        .collect();
    let (editor, cx) = open(cx, &lines.join("\n"));
    select(cx, (100_000, 0), (100_000, 0));
    let (visible, shaped) = editor.read_with(cx, |e, _| {
        (e.visible_lines().unwrap(), e.shaped_line_count())
    });
    assert!(visible.contains(&100_000), "{visible:?}");
    assert!(shaped < 200, "{shaped} lines shaped");
    cx.simulate_input("y");
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.buffer().line_text(100_000).into_owned()),
        "ydef f50000(x):"
    );
    let state = editor.read_with(cx, |e, _| e.state_json());
    assert_eq!(state["lines"], json!(200_000));
    assert_eq!(state["caret"], json!({"line": 100_000, "column": 1}));
}

#[gpui::test]
fn a_reload_from_disk_keeps_the_caret_line_and_the_view(cx: &mut TestAppContext) {
    let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = open(cx, &old);
    select(cx, (150, 3), (150, 3));
    invoke(cx, "editor.scroll-to", json!({"line": 120}));
    let top = editor.read_with(cx, |e, _| e.scroll_top());
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, e: &editor::EditorEvent, _| {
            seen.borrow_mut().push(format!("{e:?}"))
        })
        .detach()
    });
    // Two lines inserted at the top, the caret's line changed.
    let new = format!(
        "import os\n\n{}",
        old.replace("line 150\n", "line 150 # changed\n")
    );
    let decoded = text::decode(new.as_bytes()).unwrap();
    let ok = editor.update(cx, |e, cx| {
        let plan = text::ReloadPlan::new(e.buffer().rope(), e.buffer().version(), &decoded);
        e.reload(plan, false, cx)
    });
    draw(cx);
    assert!(ok);
    assert_eq!(text(&editor, cx), new);
    assert_eq!(caret(&editor, cx), Point::new(152, 3));
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), top);
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));
    assert!(
        events.borrow().iter().any(|e| e == "Reloaded"),
        "{events:?}"
    );
    // The reload is undone like an edit; the text then differs from the
    // file.
    act(cx, Undo);
    assert_eq!(text(&editor, cx), old);
    assert!(editor.read_with(cx, |e, _| e.is_modified()));

    // A plan made for an older text changes nothing.
    let plan = editor.read_with(cx, |e, _| {
        text::ReloadPlan::new(e.buffer().rope(), e.buffer().version(), &decoded)
    });
    cx.simulate_input("x");
    draw(cx);
    let ok = editor.update(cx, |e, cx| e.reload(plan, false, cx));
    assert!(!ok);
    assert!(text(&editor, cx).contains('x'));
}
