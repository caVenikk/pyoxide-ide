//! The decorations API the debugger and version control paint through:
//! gutter clicks per button and modifier, the hover of line numbers,
//! change markers and after-line texts, decorations moving with edits and
//! their places on soft-wrapped lines.

use std::cell::RefCell;
use std::rc::Rc;

use editor::commands::{Newline, SetSelection, Undo};
use editor::decorations::layer;
use editor::{ChangeKind, Decoration, Editor, EditorEvent, TextPiece};
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    TestAppContext, VisualTestContext, point, px, size,
};
use text::Buffer;
use themes::{TextRole, UiRole};

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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

fn events(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<EditorEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &EditorEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
        .detach();
    });
    events
}

fn press(
    cx: &mut VisualTestContext,
    at: gpui::Point<Pixels>,
    button: MouseButton,
    mods: Modifiers,
) {
    cx.simulate_event(MouseDownEvent {
        button,
        position: at,
        modifiers: mods,
        click_count: 1,
        first_mouse: false,
    });
    draw(cx);
}

fn hover(cx: &mut VisualTestContext, at: gpui::Point<Pixels>) {
    cx.simulate_event(MouseMoveEvent {
        position: at,
        pressed_button: None,
        modifiers: Modifiers::none(),
    });
    draw(cx);
}

/// The middle of the line numbers' area of a line.
fn number_at(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    line: usize,
) -> gpui::Point<Pixels> {
    editor.read_with(cx, |e, _| {
        e.line_number_bounds(line).expect("drawn").center()
    })
}

fn caret_line(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, _| e.caret_point().line)
}

const TEXT: &str =
    "def f(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n";

#[gpui::test]
fn gutter_presses_are_reported_per_button_and_modifier(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    editor.update(cx, |e, _| e.set_line_number_clicks(true));
    draw(cx);
    let events = events(&editor, cx);
    let at = number_at(&editor, cx, 2);
    let alt = Modifiers {
        alt: true,
        ..Modifiers::none()
    };
    let shift = Modifiers {
        shift: true,
        ..Modifiers::none()
    };
    press(cx, at, MouseButton::Left, Modifiers::none());
    press(cx, at, MouseButton::Left, alt);
    press(cx, at, MouseButton::Left, shift);
    press(cx, at, MouseButton::Right, Modifiers::none());
    let clicks: Vec<(usize, MouseButton, Modifiers)> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::GutterClicked {
                line,
                button,
                modifiers,
                ..
            } => Some((*line, *button, *modifiers)),
            _ => None,
        })
        .collect();
    assert_eq!(
        clicks,
        vec![
            (2, MouseButton::Left, Modifiers::none()),
            (2, MouseButton::Left, alt),
            (2, MouseButton::Left, shift),
            (2, MouseButton::Right, Modifiers::none()),
        ]
    );
    // The presses were the owner's: the caret stayed.
    assert_eq!(caret_line(&editor, cx), 0);
}

#[gpui::test]
fn without_an_owner_a_line_number_press_moves_the_caret(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    let at = number_at(&editor, cx, 3);
    press(cx, at, MouseButton::Left, Modifiers::none());
    assert_eq!(caret_line(&editor, cx), 3);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::GutterClicked { .. }))
    );
}

#[gpui::test]
fn hovering_line_numbers_reports_the_line_and_shows_the_hover_icon(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    let at = number_at(&editor, cx, 1);
    hover(cx, at);
    hover(cx, point(at.x + px(1.0), at.y));
    editor.update(cx, |e, cx| {
        e.set_gutter_hover_icon(Some((1, ui::icons::IconName::BREAKPOINT)), cx)
    });
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.gutter_hovered_line()),
        Some(1)
    );
    // Over the text: off the line numbers.
    let (origin, _, _) = editor.read_with(cx, |e, _| e.text_geometry().unwrap());
    hover(cx, point(origin.x + px(20.0), at.y));
    let hovered: Vec<Option<usize>> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::GutterHovered { line } => Some(*line),
            _ => None,
        })
        .collect();
    assert_eq!(hovered, vec![Some(1), None]);
}

#[gpui::test]
fn change_markers_widen_under_the_mouse_and_report_clicks(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    let (added, deleted) = editor.update(cx, |e, cx| {
        let ids = e.set_decorations(
            "vcs",
            vec![
                (1..3, Decoration::default().change(ChangeKind::Added)),
                (4..4, Decoration::default().change(ChangeKind::Deleted)),
            ],
            cx,
        );
        (ids[0], ids[1])
    });
    draw(cx);
    let marker = editor.read_with(cx, |e, _| e.change_marker_bounds(added).expect("drawn"));
    let lh = editor.read_with(cx, |e, _| e.text_geometry().unwrap().1);
    // The marker covers its two lines.
    assert_eq!(marker.size.height, lh * 2.0);
    let at = point(marker.right() - px(2.0), marker.top() + lh * 1.5);
    hover(cx, at);
    assert_eq!(
        editor.read_with(cx, |e, _| e.hovered_change_marker()),
        Some(added)
    );
    press(cx, at, MouseButton::Left, Modifiers::none());
    // A deleted lines' marker reacts a third of a line around its boundary.
    let boundary = editor.read_with(cx, |e, _| e.change_marker_bounds(deleted).expect("drawn"));
    assert!((boundary.size.height - lh * 2.0 / 3.0).abs() < px(0.01));
    press(cx, boundary.center(), MouseButton::Left, Modifiers::none());
    let clicked: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::ChangeMarkerClicked { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(clicked, vec![added, deleted]);
    assert_eq!(caret_line(&editor, cx), 0);
}

#[gpui::test]
fn diff_gutter_actions_sit_before_the_text_and_report_clicks(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    let action = |d: Decoration| d.action(ui::icons::IconName::DIFF_CHECKBOX, None);
    let (changed, between) = editor.update(cx, |e, cx| {
        let ids = e.set_decorations(
            "diff",
            vec![
                (1..3, action(Decoration::default())),
                (4..4, action(Decoration::default())),
            ],
            cx,
        );
        (ids[0], ids[1])
    });
    draw(cx);
    // Without the diff gutter's action column nothing is drawn.
    assert_eq!(
        editor.read_with(cx, |e, _| e.gutter_action_bounds(changed)),
        None
    );
    editor.update(cx, |e, cx| {
        e.set_diff_gutter(editor::side_by_side::DiffGutter::Actions, cx)
    });
    draw(cx);
    let (first, second, offset, lh) = editor.read_with(cx, |e, _| {
        (
            e.gutter_action_bounds(changed).expect("drawn"),
            e.gutter_action_bounds(between).expect("drawn"),
            e.diff_action_offset().expect("laid out"),
            e.text_geometry().unwrap().1,
        )
    });
    // In the action column, on the first line (the line below a boundary).
    let size = px(editor::side_by_side::ACTION_SIZE);
    assert_eq!(first.size, gpui::size(size, size));
    assert_eq!(second.left(), first.left());
    // The editor fills the window: its left edge is the window's.
    assert_eq!(first.left(), offset);
    assert!((second.top() - first.top() - lh * 3.0).abs() < px(0.01));
    press(cx, first.center(), MouseButton::Left, Modifiers::none());
    press(cx, second.center(), MouseButton::Left, Modifiers::none());
    let clicked: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::GutterActionClicked { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(clicked, vec![changed, between]);
    assert_eq!(caret_line(&editor, cx), 0);
}

#[gpui::test]
fn after_line_texts_follow_the_line_end_and_report_clicks(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    let piece = |t: &str| TextPiece::new(t.to_owned(), TextRole::InlayHint).italic();
    let (first, second) = editor.update(cx, |e, cx| {
        let a = e.add_decoration(
            "debugger",
            0..1,
            Decoration::default().after_text(vec![piece("items: "), piece("[1, 2]")], true),
            cx,
        );
        let b = e.add_decoration(
            "debugger",
            0..1,
            Decoration::default().after_text(vec![piece("total: 0")], true),
            cx,
        );
        (a, b)
    });
    draw(cx);
    let (origin, _, cw) = editor.read_with(cx, |e, _| e.text_geometry().unwrap());
    let a = editor.read_with(cx, |e, _| e.after_text_bounds(first).expect("drawn"));
    let b = editor.read_with(cx, |e, _| e.after_text_bounds(second).expect("drawn"));
    // One space after the line end, the text half a space further; the
    // next text after it.
    let line_end = origin.x + cw * "def f(items):".len() as f32;
    assert!((a.left() - (line_end + cw * 2.5)).abs() < px(0.5), "{a:?}");
    assert!(b.left() > a.right() + cw * 2.0);
    hover(cx, a.center());
    assert_eq!(
        editor.read_with(cx, |e, _| e.hovered_after_text()),
        Some(first)
    );
    press(cx, b.center(), MouseButton::Left, Modifiers::none());
    assert!(
        events
            .borrow()
            .iter()
            .any(|e| matches!(e, EditorEvent::AfterLineTextClicked { id, .. } if *id == second))
    );
    assert_eq!(caret_line(&editor, cx), 0);
}

#[gpui::test]
fn decorations_move_with_typing_and_undo_and_go_with_their_lines(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let breakpoint = editor.update(cx, |e, cx| {
        e.add_decoration(
            "debugger",
            3..4,
            Decoration::default()
                .gutter_icon(ui::icons::IconName::BREAKPOINT, None)
                .background(UiRole::Breakpoint, layer::BREAKPOINT)
                .stripe(UiRole::DiagnosticErrorStripe, false),
            cx,
        )
    });
    let lines =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.decoration_lines(breakpoint));
    // Enter at the start of the file moves it down; undo moves it back.
    let p = editor::commands::Position { line: 0, column: 0 };
    cx.dispatch_action(SetSelection { anchor: p, head: p });
    cx.dispatch_action(Newline);
    draw(cx);
    assert_eq!(lines(cx), Some(4..5));
    cx.dispatch_action(Undo);
    draw(cx);
    assert_eq!(lines(cx), Some(3..4));
    // The state reports it.
    let state = editor.read_with(cx, |e, _| e.state_json());
    assert_eq!(state["decorations"][0]["lines"], serde_json::json!([3, 4]));
    assert_eq!(
        state["decorations"][0]["gutter_icon"],
        "breakpoints/breakpoint"
    );
    // Deleting its line removes it.
    editor.update(cx, |e, cx| {
        let start = e.buffer().line_start(3);
        let end = e.buffer().line_start(4);
        e.edit(
            vec![text::Edit::new(start..end, "")],
            text::EditKind::Other,
            text::CaretPlacement::AfterEachEdit,
            cx,
        );
    });
    draw(cx);
    assert_eq!(lines(cx), None);
}

#[gpui::test]
fn on_soft_wrapped_lines_icons_go_on_the_first_row_and_texts_after_the_last(
    cx: &mut TestAppContext,
) {
    let long = format!("x = [{}]\nshort\n", "1, ".repeat(60));
    let (editor, cx) = open(cx, &long);
    cx.simulate_resize(size(px(400.0), px(300.0)));
    editor.update(cx, |e, cx| e.set_soft_wraps(true, cx));
    draw(cx);
    let id = editor.update(cx, |e, cx| {
        e.add_decoration(
            "debugger",
            0..1,
            Decoration::default()
                .gutter_icon(ui::icons::IconName::BREAKPOINT, None)
                .after_text(vec![TextPiece::new("x: [...]", TextRole::InlayHint)], true),
            cx,
        )
    });
    draw(cx);
    let rows = editor.read_with(cx, |e, _| e.state_json()["rows"].as_u64().unwrap());
    assert!(rows > 3, "the first line wraps: {rows} rows");
    let (origin, lh, _) = editor.read_with(cx, |e, _| e.text_geometry().unwrap());
    let number = editor.read_with(cx, |e, _| e.line_number_bounds(0).unwrap());
    assert_eq!(number.top(), origin.y);
    let text = editor.read_with(cx, |e, _| e.after_text_bounds(id).unwrap());
    // Rows: the first line's, then `short` and the empty last line.
    assert_eq!(text.top(), origin.y + lh * (rows - 3) as f32);
    // The next line's number is below all rows of the first.
    let next = editor.read_with(cx, |e, _| e.line_number_bounds(1).unwrap());
    assert_eq!(next.top(), origin.y + lh * (rows - 2) as f32);
}
