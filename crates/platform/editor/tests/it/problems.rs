//! Problems in the editor as the user meets them: next / previous problem,
//! the inspection widget, the error stripe, and the hover input code
//! intelligence builds its popups on (simulated mouse and modifiers).

use std::cell::RefCell;
use std::rc::Rc;

use editor::commands::{NextProblem, PreviousProblem};
use editor::{Analysis, DiagnosticMark, Editor, EditorEvent, MarkMessage, MarkSeverity};
use gpui::{
    CursorStyle, Entity, Focusable, Modifiers, Pixels, TestAppContext, VisualTestContext, point, px,
};
use text::{Buffer, SelectionSet};

// "a = 1\n" is 0..5, "b = undefined\n" 6..19 (`undefined` 10..19),
// "c = None == x\n" 20..33 (`None == x` 24..33).
const TEXT: &str = "a = 1\nb = undefined\nc = None == x\n";

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
        ui::pointer::begin_frame(window);
        window.refresh();
        window.draw(cx).clear();
    });
}

fn message(severity: MarkSeverity, text: &str) -> MarkMessage {
    MarkMessage {
        severity,
        text: text.to_owned().into(),
        source: Some("Ruff".into()),
        code: Some("rule".into()),
        href: None,
    }
}

fn mark(range: std::ops::Range<usize>, severity: MarkSeverity) -> DiagnosticMark {
    DiagnosticMark {
        range,
        severity,
        unnecessary: false,
        deprecated: false,
        messages: vec![message(severity, "a problem")],
    }
}

fn set_marks(editor: &Entity<Editor>, cx: &mut VisualTestContext, marks: Vec<DiagnosticMark>) {
    editor.update(cx, |e, cx| {
        e.set_diagnostics(marks, cx);
        e.set_analysis(Analysis::Done, cx);
    });
    draw(cx);
}

fn caret(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, _| e.selections().primary().head)
}

fn set_caret(editor: &Entity<Editor>, cx: &mut VisualTestContext, pos: usize) {
    editor.update(cx, |e, cx| e.set_selections(SelectionSet::caret(pos), cx));
    draw(cx);
}

/// Window position inside the char at `line`, `column` (fraction of a
/// char across).
fn at(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    line: usize,
    column: f32,
) -> gpui::Point<Pixels> {
    editor.read_with(cx, |e, _| {
        let (origin, line_height, char_width) = e.text_geometry().expect("drawn");
        point(
            origin.x + char_width * column,
            origin.y + line_height * (line as f32 + 0.5),
        )
    })
}

/// Moves the mouse into the char at `line`, `column`.
fn move_to(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    line: usize,
    column: f32,
    button: impl Into<Option<gpui::MouseButton>>,
    modifiers: Modifiers,
) {
    let p = at(editor, cx, line, column);
    cx.simulate_mouse_move(p, button, modifiers);
}

fn events(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<EditorEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &EditorEvent, _| {
            if matches!(
                event,
                EditorEvent::Hovered { .. } | EditorEvent::ModifiedClick { .. }
            ) {
                sink.borrow_mut().push(event.clone());
            }
        })
        .detach();
    });
    events
}

/// The offsets of the `Hovered` events since the last call.
fn hovered(events: &Rc<RefCell<Vec<EditorEvent>>>) -> Vec<(Option<usize>, bool)> {
    events
        .borrow_mut()
        .drain(..)
        .filter_map(|e| match e {
            EditorEvent::Hovered {
                offset, modifiers, ..
            } => Some((offset, modifiers.secondary())),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn next_problem_goes_to_errors_first_and_wraps(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    set_marks(
        &editor,
        cx,
        vec![
            mark(0..1, MarkSeverity::WeakWarning),
            mark(10..19, MarkSeverity::Error),
            mark(24..33, MarkSeverity::Warning),
            mark(30..31, MarkSeverity::Information),
        ],
    );
    cx.dispatch_action(NextProblem);
    draw(cx);
    assert_eq!(caret(&editor, cx), 10);
    // The only error: it wraps to itself.
    cx.dispatch_action(NextProblem);
    draw(cx);
    assert_eq!(caret(&editor, cx), 10);
    cx.dispatch_action(PreviousProblem);
    draw(cx);
    assert_eq!(caret(&editor, cx), 10);
    // Without the error: warnings, then (without warnings) weak warnings.
    set_marks(
        &editor,
        cx,
        vec![
            mark(0..1, MarkSeverity::WeakWarning),
            mark(4..5, MarkSeverity::Warning),
            mark(24..33, MarkSeverity::Warning),
        ],
    );
    cx.dispatch_action(NextProblem);
    draw(cx);
    assert_eq!(caret(&editor, cx), 24);
    cx.dispatch_action(NextProblem);
    draw(cx);
    assert_eq!(caret(&editor, cx), 4);
    cx.dispatch_action(PreviousProblem);
    draw(cx);
    assert_eq!(caret(&editor, cx), 24);
    // A selection and extra carets go.
    editor.update(cx, |e, cx| {
        e.set_selections(
            SelectionSet::from_vec(vec![text::Selection::new(0, 3), text::Selection::caret(8)]),
            cx,
        )
    });
    cx.dispatch_action(NextProblem);
    draw(cx);
    editor.read_with(cx, |e, _| {
        assert_eq!(e.selections().len(), 1);
        assert!(e.selections().primary().is_empty());
    });
}

#[gpui::test]
fn the_inspection_widget_counts_problems_and_navigates(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    // No services: no widget.
    assert!(cx.debug_bounds("inspection-widget").is_none());
    set_marks(
        &editor,
        cx,
        vec![
            // Two services on one range count twice, each by its level.
            DiagnosticMark {
                messages: vec![
                    message(MarkSeverity::Error, "Undefined name"),
                    message(MarkSeverity::Warning, "Unresolved reference"),
                ],
                ..mark(10..19, MarkSeverity::Error)
            },
            mark(24..33, MarkSeverity::Warning),
            mark(0..1, MarkSeverity::Information),
        ],
    );
    let counts = editor.read_with(cx, |e, _| e.problem_counts());
    assert_eq!(
        (counts.errors, counts.warnings, counts.weak_warnings),
        (1, 2, 0)
    );
    let state = editor.read_with(cx, |e, _| e.state_json());
    assert_eq!(state["analysis"], "done");
    assert_eq!(state["problems"]["warnings"], 2);
    let widget = cx.debug_bounds("inspection-widget").expect("the widget");
    let next = cx.debug_bounds("inspection-next").expect("the arrows");
    let previous = cx.debug_bounds("inspection-previous").expect("the arrows");
    // Top right, left of the error stripe, arrows after the counts.
    let bounds = editor.read_with(cx, |e, _| e.text_area_bounds().unwrap());
    assert!(widget.top() < bounds.top() + px(4.0));
    assert!(next.right() <= bounds.right());
    assert!(previous.left() >= widget.right() && next.left() >= previous.right());

    // A click on the counts goes to the first problem (errors first).
    set_caret(&editor, cx, 32);
    cx.simulate_click(widget.center(), Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), 10);
    cx.simulate_click(next.center(), Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), 10);
    // Without errors the arrows visit the warnings.
    set_marks(
        &editor,
        cx,
        vec![
            mark(4..5, MarkSeverity::Warning),
            mark(24..33, MarkSeverity::Warning),
        ],
    );
    set_caret(&editor, cx, 6);
    let next = cx.debug_bounds("inspection-next").unwrap();
    let previous = cx.debug_bounds("inspection-previous").unwrap();
    cx.simulate_click(next.center(), Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), 24);
    cx.simulate_click(previous.center(), Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), 4);
}

#[gpui::test]
fn a_clean_text_shows_the_check_mark_alone(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    editor.update(cx, |e, cx| e.set_analysis(Analysis::Running, cx));
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.state_json())["analysis"],
        "running"
    );
    set_marks(&editor, cx, Vec::new());
    // (Debug bounds outlive their elements: absence is checked in a window
    // that never had the arrows.)
    assert!(cx.debug_bounds("inspection-widget").is_some());
    assert!(cx.debug_bounds("inspection-next").is_none());
    // A click on a clean widget changes nothing.
    set_caret(&editor, cx, 7);
    let widget = cx.debug_bounds("inspection-widget").unwrap();
    cx.simulate_click(widget.center(), Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), 7);
}

#[gpui::test]
fn hovering_reports_the_char_under_the_mouse_once(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    let none = Modifiers::none();
    move_to(&editor, cx, 0, 0.5, None, none);
    move_to(&editor, cx, 0, 0.7, None, none);
    move_to(&editor, cx, 0, 1.2, None, none);
    assert_eq!(hovered(&events), [(Some(0), false), (Some(1), false)]);
    // The right half of a char is still that char.
    move_to(&editor, cx, 1, 4.9, None, none);
    assert_eq!(hovered(&events), [(Some(10), false)]);
    assert_eq!(editor.read_with(cx, |e, _| e.hovered()), Some(10));
    // Past the end of the line: nothing under the mouse.
    move_to(&editor, cx, 0, 30.0, None, none);
    move_to(&editor, cx, 0, 31.0, None, none);
    assert_eq!(hovered(&events), [(None, false)]);
    // The navigation modifier, pressed while hovering and moved with.
    move_to(&editor, cx, 1, 5.5, None, none);
    cx.simulate_modifiers_change(Modifiers::secondary_key());
    move_to(&editor, cx, 1, 5.8, None, Modifiers::secondary_key());
    cx.simulate_modifiers_change(none);
    assert_eq!(
        hovered(&events),
        [(Some(11), false), (Some(11), true), (Some(11), false)]
    );
    // Leaving the text ends the hover once; dragging is not hovering.
    let y = at(&editor, cx, 0, 0.0).y;
    cx.simulate_mouse_move(point(px(2.0), y), None, none);
    let y = at(&editor, cx, 0, 0.0).y;
    cx.simulate_mouse_move(point(px(3.0), y), None, none);
    move_to(&editor, cx, 2, 0.5, gpui::MouseButton::Left, none);
    assert_eq!(hovered(&events), [(None, false)]);
}

#[gpui::test]
fn a_navigation_click_is_reported_and_leaves_the_caret(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    let events = events(&editor, cx);
    set_caret(&editor, cx, 2);
    let target = at(&editor, cx, 1, 6.3);
    cx.simulate_click(target, Modifiers::secondary_key());
    draw(cx);
    assert_eq!(caret(&editor, cx), 2);
    let clicks: Vec<usize> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::ModifiedClick { offset, .. } => Some(*offset),
            _ => None,
        })
        .collect();
    assert_eq!(clicks, [12]);
    // A plain click places the caret as always.
    cx.simulate_click(target, Modifiers::none());
    draw(cx);
    assert_eq!(caret(&editor, cx), 12);
}

#[gpui::test]
fn the_link_highlight_is_drawn_with_the_hand_pointer(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, TEXT);
    editor.update(cx, |e, cx| e.set_link_highlight(Some(10..19), cx));
    let over = at(&editor, cx, 1, 6.5);
    cx.simulate_mouse_move(over, None, Modifiers::secondary_key());
    draw(cx);
    let pointer = |cx: &mut VisualTestContext| cx.update(|window, _| ui::pointer::resolve(window));
    assert_eq!(pointer(cx), CursorStyle::PointingHand);
    move_to(&editor, cx, 2, 0.5, None, Modifiers::none());
    draw(cx);
    assert_eq!(pointer(cx), CursorStyle::IBeam);
}

#[gpui::test]
fn problems_at_an_offset_for_the_hover_popup(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "print(\nx\n");
    set_marks(
        &editor,
        cx,
        vec![
            mark(0..5, MarkSeverity::Warning),
            mark(2..4, MarkSeverity::Error),
            mark(6..6, MarkSeverity::Error),
        ],
    );
    editor.read_with(cx, |e, _| {
        let severities = |offset| -> Vec<MarkSeverity> {
            e.diagnostics_at(offset)
                .iter()
                .map(|m| m.severity)
                .collect()
        };
        assert_eq!(severities(3), [MarkSeverity::Error, MarkSeverity::Warning]);
        assert_eq!(severities(4), [MarkSeverity::Warning]);
        assert_eq!(severities(5), []);
        assert_eq!(severities(6), [MarkSeverity::Error]);
    });
    // The error after the end of the line is under the mouse in the cell
    // where it is drawn, not further right.
    let events = events(&editor, cx);
    move_to(&editor, cx, 0, 6.5, None, Modifiers::none());
    move_to(&editor, cx, 0, 8.5, None, Modifiers::none());
    assert_eq!(hovered(&events), [(Some(6), false), (None, false)]);
}
