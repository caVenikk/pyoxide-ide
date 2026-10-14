//! What the editor offers code intelligence: diagnostic marks and the link
//! highlight (stored; painting comes with the problems in the editor),
//! geometry for popups, and the reload event.

use std::cell::RefCell;
use std::rc::Rc;

use editor::{DiagnosticMark, Editor, EditorEvent, MarkMessage, MarkSeverity};
use gpui::{Entity, TestAppContext, VisualTestContext, point, px};
use text::Buffer;

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> (Entity<Editor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let text = text.to_owned();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(Buffer::new(&text), window, cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    (editor, cx)
}

fn mark(range: std::ops::Range<usize>, severity: MarkSeverity) -> DiagnosticMark {
    DiagnosticMark {
        range,
        severity,
        unnecessary: false,
        deprecated: false,
        messages: vec![MarkMessage {
            text: "`os` imported but unused".into(),
            source: Some("Ruff".into()),
            code: Some("unused-import".into()),
            href: None,
        }],
    }
}

#[gpui::test]
fn marks_and_the_link_highlight_are_kept_sorted(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "import os\nx = undefined\n");
    editor.update(cx, |e, cx| {
        assert!(e.diagnostics().is_empty());
        e.set_diagnostics(
            vec![
                mark(14..23, MarkSeverity::Error),
                mark(7..9, MarkSeverity::Warning),
            ],
            cx,
        );
        let ranges: Vec<_> = e.diagnostics().iter().map(|m| m.range.clone()).collect();
        assert_eq!(ranges, [7..9, 14..23]);
        assert!(MarkSeverity::Error < MarkSeverity::Information);
        e.set_link_highlight(Some(14..23), cx);
        assert_eq!(e.link_highlight(), Some(14..23));
        e.set_link_highlight(None, cx);
        assert_eq!(e.link_highlight(), None);
        e.set_diagnostics(Vec::new(), cx);
        assert!(e.diagnostics().is_empty());
    });
}

#[gpui::test]
fn offsets_and_bounds_from_the_last_frame(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "import os\nx = undefined\n");
    editor.read_with(cx, |e, _| {
        let (origin, line_height, char_width) = e.text_geometry().expect("drawn");
        let at = |line: usize, column: f32| {
            point(
                origin.x + char_width * column,
                origin.y + line_height * (line as f32 + 0.5),
            )
        };
        assert_eq!(e.offset_at(at(1, 4.2)), Some(14));
        assert_eq!(e.offset_at(at(0, 0.1)), Some(0));
        // In the gutter: not over the text.
        assert_eq!(e.offset_at(point(px(1.0), at(0, 0.0).y)), None);

        let word = e.bounds_for_range(14..23).unwrap();
        assert!((f32::from(word.size.width) - f32::from(char_width) * 9.0).abs() < 1.0);
        assert_eq!(word.size.height, line_height);
        let caret = e.bounds_for_range(14..14).unwrap();
        assert_eq!(caret.origin, word.origin);
        let lines = e.bounds_for_range(7..14).unwrap();
        assert_eq!(lines.top(), word.top() - line_height);
        assert_eq!(lines.bottom(), word.bottom());
    });
    // Before its first frame an editor has no geometry.
    let fresh = cx.update(|window, cx| editor::new_editor("x", window, cx));
    fresh.read_with(cx, |e, _| {
        assert_eq!(e.offset_at(point(px(10.0), px(10.0))), None);
        assert_eq!(e.bounds_for_range(0..1), None);
    });
}

#[gpui::test]
fn replacing_the_text_reports_a_reload(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "a\n");
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
        .detach();
    });
    editor.update(cx, |e, cx| {
        *e.buffer_mut() = Buffer::new("b\n");
        e.buffer_reloaded(cx);
    });
    cx.run_until_parked();
    assert!(
        matches!(
            &events.borrow()[..],
            [EditorEvent::Reloaded, EditorEvent::SelectionsChanged]
        ),
        "{:?}",
        events.borrow()
    );
}
