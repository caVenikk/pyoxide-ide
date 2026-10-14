//! The error stripe: marks per problem, their tooltip and clicks.

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext, point, px};
use text::Buffer;
use themes::UiRole;
use ui::theme::ThemeExt;

use crate::diagnostics::{Analysis, DiagnosticMark, MarkMessage, MarkSeverity};
use crate::editor::Editor;

fn open(cx: &mut TestAppContext, lines: usize) -> (Entity<Editor>, &mut VisualTestContext) {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        crate::init(cx);
    });
    let text: String = (0..lines).map(|i| format!("line_{i} = {i}\n")).collect();
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

/// A problem from the first char of `line` to the first char of `end`.
fn mark(editor: &Editor, line: usize, end: usize, severity: MarkSeverity) -> DiagnosticMark {
    let b = editor.buffer();
    DiagnosticMark {
        range: b.line_start(line)..b.line_start(end) + 1,
        severity,
        unnecessary: false,
        deprecated: false,
        messages: vec![MarkMessage {
            severity,
            text: format!("problem on line {line}").into(),
            source: Some("Ruff".into()),
            code: Some("rule".into()),
            href: None,
        }],
    }
}

fn set(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    marks: &[(usize, usize, MarkSeverity)],
) {
    editor.update(cx, |e, cx| {
        let marks = marks
            .iter()
            .map(|&(line, end, severity)| mark(e, line, end, severity))
            .collect();
        e.set_diagnostics(marks, cx);
        e.set_analysis(Analysis::Done, cx);
    });
    draw(cx);
}

#[derive(Debug)]
struct StripeInfo {
    line: usize,
    top: f32,
    height: f32,
    role: Option<UiRole>,
}

fn stripes(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<StripeInfo> {
    editor.read_with(cx, |e, cx| {
        let theme = cx.theme();
        let layout = e.layout.as_ref().unwrap();
        layout
            .problem_stripes
            .iter()
            .map(|s| StripeInfo {
                line: e.buffer().char_to_line(e.diagnostics()[s.mark].range.start),
                top: f32::from(s.bounds.top()),
                height: f32::from(s.bounds.size.height),
                role: [
                    UiRole::DiagnosticErrorStripe,
                    UiRole::DiagnosticWarningStripe,
                    UiRole::DiagnosticWeakWarningStripe,
                ]
                .into_iter()
                .find(|&r| theme.color(r) == s.color),
            })
            .collect()
    })
}

#[gpui::test]
fn problems_are_marked_on_the_error_stripe(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, 400);
    set(
        &editor,
        cx,
        &[
            (10, 10, MarkSeverity::Error),
            (200, 230, MarkSeverity::Warning),
            (300, 300, MarkSeverity::WeakWarning),
            (350, 350, MarkSeverity::Information),
        ],
    );
    let s = stripes(&editor, cx);
    // Information has no mark; the most severe is drawn last, in its
    // level's stripe colour.
    assert_eq!(s.len(), 3, "{s:?}");
    assert_eq!(
        s.iter().map(|m| (m.line, m.role)).collect::<Vec<_>>(),
        [
            (300, Some(UiRole::DiagnosticWeakWarningStripe)),
            (200, Some(UiRole::DiagnosticWarningStripe)),
            (10, Some(UiRole::DiagnosticErrorStripe)),
        ]
    );
    // At least 2 px; a problem over several lines spans them; lower lines
    // lower on the stripe.
    assert!(s.iter().all(|m| m.height >= 2.0), "{s:?}");
    assert_eq!(s[2].height, 2.0);
    assert!(s[1].height > 2.0, "{s:?}");
    assert!(s[2].top < s[1].top && s[1].top < s[0].top, "{s:?}");
}

#[gpui::test]
fn a_short_text_has_the_stripe_too(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, 5);
    let stripe = editor.read_with(cx, |e, _| e.layout.as_ref().unwrap().stripe);
    assert!(stripe.is_none(), "no services, no scrolling: no stripe");
    set(&editor, cx, &[(1, 1, MarkSeverity::Error)]);
    let (stripe, scrollbar) = editor.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        (l.stripe, l.scrollbar.is_some())
    });
    assert!(stripe.is_some() && !scrollbar);
    assert_eq!(stripes(&editor, cx).len(), 1);
}

#[gpui::test]
fn hovering_a_stripe_mark_shows_its_messages_and_a_click_goes_there(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, 400);
    set(
        &editor,
        cx,
        &[
            (0, 0, MarkSeverity::Warning),
            (200, 200, MarkSeverity::Error),
        ],
    );
    let (stripe, mark_y, top_y) = editor.read_with(cx, |e, _| {
        let l = e.layout.as_ref().unwrap();
        let y = |line: usize| {
            l.problem_stripes
                .iter()
                .find(|s| e.diagnostics()[s.mark].range.start == e.buffer().line_start(line))
                .unwrap()
                .bounds
                .center()
                .y
        };
        (l.stripe.unwrap(), y(200), y(0))
    });
    // Hover: the tooltip with the problem's message, left of the stripe.
    let over = point(stripe.center().x, mark_y + px(1.0));
    cx.simulate_mouse_move(over, None, Modifiers::none());
    draw(cx);
    let hover = editor
        .read_with(cx, |e, _| e.stripe_hover.clone())
        .expect("hovered");
    assert_eq!(hover.marks.len(), 1);
    let tooltip = cx.debug_bounds("stripe-tooltip").expect("a tooltip");
    assert!(tooltip.right() <= stripe.left());
    // Away from the marks: no tooltip.
    cx.simulate_mouse_move(
        point(stripe.center().x, mark_y + px(40.0)),
        None,
        Modifiers::none(),
    );
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.stripe_hover.is_none()));
    // A click moves the caret to the problem and centres its line.
    cx.simulate_click(over, Modifiers::none());
    draw(cx);
    editor.read_with(cx, |e, _| {
        assert_eq!(e.caret_point().line, 200);
        let visible = e.visible_lines().unwrap();
        assert!(visible.contains(&200) && visible.start > 150, "{visible:?}");
    });
    // The first line's mark is under the thumb at the top: a click in place
    // still goes there.
    editor.update(cx, |e, cx| e.set_scroll_y(px(0.0), cx));
    draw(cx);
    cx.simulate_click(point(stripe.center().x, top_y), Modifiers::none());
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.caret_point().line), 0);
}
