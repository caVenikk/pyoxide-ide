//! What the editor offers code intelligence: diagnostic marks (moved with
//! the text, reported in the state) and the link highlight, geometry for
//! popups, and the reload event.

use std::cell::RefCell;
use std::rc::Rc;

use editor::commands::{SetSelection, Undo};
use editor::{DiagnosticMark, Editor, EditorEvent, MarkMessage, MarkSeverity};
use gpui::{Entity, Focusable, TestAppContext, VisualTestContext, point, px};
use serde_json::json;
use text::Buffer;

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

fn ranges(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<std::ops::Range<usize>> {
    editor.read_with(cx, |e, _| {
        e.diagnostics().iter().map(|m| m.range.clone()).collect()
    })
}

/// Puts the caret at a zero-based line and column.
fn caret_at(cx: &mut VisualTestContext, line: usize, column: usize) {
    let p = editor::commands::Position { line, column };
    cx.dispatch_action(SetSelection { anchor: p, head: p });
    draw(cx);
}

fn undo(cx: &mut VisualTestContext) {
    cx.dispatch_action(Undo);
    draw(cx);
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

#[gpui::test]
fn marks_move_with_typing_and_undo(cx: &mut TestAppContext) {
    // `os` is 7..9, `undefined` 14..23.
    let (editor, cx) = open(cx, "import os\nx = undefined\n");
    editor.update(cx, |e, cx| {
        e.set_diagnostics(
            vec![
                DiagnosticMark {
                    unnecessary: true,
                    ..mark(7..9, MarkSeverity::Warning)
                },
                mark(14..23, MarkSeverity::Error),
            ],
            cx,
        );
    });
    // Typing inside a mark widens it; undo narrows it again.
    caret_at(cx, 1, 8);
    cx.simulate_input("zz");
    draw(cx);
    assert_eq!(ranges(&editor, cx), [7..9, 14..25]);
    undo(cx);
    assert_eq!(ranges(&editor, cx), [7..9, 14..23]);
    // Typing at the start of a mark or before it moves it.
    caret_at(cx, 1, 4);
    cx.simulate_input("y");
    draw(cx);
    assert_eq!(ranges(&editor, cx), [7..9, 15..24]);
    caret_at(cx, 0, 0);
    cx.simulate_input("# ");
    draw(cx);
    assert_eq!(ranges(&editor, cx), [9..11, 17..26]);
    undo(cx);
    undo(cx);
    assert_eq!(ranges(&editor, cx), [7..9, 14..23]);
    // Typing at the end of a mark leaves it as it is.
    caret_at(cx, 0, 9);
    cx.simulate_input("x");
    draw(cx);
    assert_eq!(ranges(&editor, cx), [7..9, 15..24]);
    undo(cx);
    // Replacing more than a mark's text removes it; undo does not bring it
    // back (the services report it again).
    cx.dispatch_action(SetSelection {
        anchor: editor::commands::Position { line: 1, column: 0 },
        head: editor::commands::Position {
            line: 1,
            column: 13,
        },
    });
    cx.simulate_input("y");
    draw(cx);
    assert_eq!(ranges(&editor, cx).len(), 1);
    assert_eq!(ranges(&editor, cx)[0], 7..9);
    undo(cx);
    assert_eq!(ranges(&editor, cx).len(), 1);
    assert_eq!(ranges(&editor, cx)[0], 7..9);
    assert_eq!(
        editor.read_with(cx, |e, _| e.text()),
        "import os\nx = undefined\n"
    );
}

#[gpui::test]
fn empty_marks_cover_the_next_char_unless_at_a_line_end(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "print(\nx\n");
    editor.update(cx, |e, cx| {
        e.set_diagnostics(
            vec![
                mark(6..6, MarkSeverity::Error),
                mark(7..7, MarkSeverity::Warning),
                mark(30..30, MarkSeverity::WeakWarning),
            ],
            cx,
        );
    });
    assert_eq!(ranges(&editor, cx), [6..6, 7..8, 9..9]);
    // Typing at the end of the line leaves the empty mark where it was; it
    // is still drawn after the end of its line.
    caret_at(cx, 0, 6);
    cx.simulate_input("1");
    draw(cx);
    assert_eq!(ranges(&editor, cx), [6..6, 8..9, 10..10]);
}

#[gpui::test]
fn the_state_reports_the_marks_and_the_link_highlight(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "import os\nx = undefined\n");
    editor.update(cx, |e, cx| {
        e.set_diagnostics(
            vec![DiagnosticMark {
                unnecessary: true,
                ..mark(7..9, MarkSeverity::Warning)
            }],
            cx,
        );
        e.set_link_highlight(Some(14..23), cx);
    });
    draw(cx);
    let state = editor.read_with(cx, |e, _| e.state_json());
    assert_eq!(
        state["diagnostics"],
        json!([{
            "range": {
                "start": {"line": 0, "column": 7},
                "end": {"line": 0, "column": 9},
            },
            "severity": "warning",
            "unnecessary": true,
            "deprecated": false,
            "messages": [{
                "text": "`os` imported but unused",
                "source": "Ruff",
                "code": "unused-import",
                "href": null,
            }],
        }])
    );
    assert_eq!(
        state["link_highlight"],
        json!({"start": {"line": 1, "column": 4}, "end": {"line": 1, "column": 13}})
    );
    // An edit ends the link highlight (it was about the old text).
    caret_at(cx, 0, 0);
    cx.simulate_input("#");
    draw(cx);
    let state = editor.read_with(cx, |e, _| e.state_json());
    assert_eq!(state["link_highlight"], json!(null));
    assert_eq!(state["diagnostics"][0]["range"]["start"]["column"], 8);
}

#[gpui::test]
fn a_reload_removes_the_marks(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "a\n");
    editor.update(cx, |e, cx| {
        e.set_diagnostics(vec![mark(0..1, MarkSeverity::Error)], cx);
        e.set_link_highlight(Some(0..1), cx);
        *e.buffer_mut() = Buffer::new("b\n");
        e.buffer_reloaded(cx);
        assert!(e.diagnostics().is_empty());
        assert_eq!(e.link_highlight(), None);
    });
}

#[gpui::test]
fn many_marks_on_a_large_file_are_drawn_and_moved(cx: &mut TestAppContext) {
    let text: String = (0..20_000).map(|i| format!("value_{i} = {i}\n")).collect();
    let (editor, cx) = open(cx, &text);
    editor.update(cx, |e, cx| {
        let marks = (0..1000)
            .map(|i| {
                let start = e.buffer().line_start(i * 20);
                let severity = match i % 4 {
                    0 => MarkSeverity::Error,
                    1 => MarkSeverity::Warning,
                    2 => MarkSeverity::WeakWarning,
                    _ => MarkSeverity::Information,
                };
                DiagnosticMark {
                    unnecessary: i % 5 == 0,
                    deprecated: i % 7 == 0,
                    ..mark(start..start + 5, severity)
                }
            })
            .chain([mark(11..11, MarkSeverity::Error)])
            .collect();
        e.set_diagnostics(marks, cx);
    });
    draw(cx);
    caret_at(cx, 0, 0);
    cx.simulate_input("x");
    draw(cx);
    let r = ranges(&editor, cx);
    assert_eq!(r.len(), 1001);
    // The first mark starts at the caret: typing there moves it.
    assert_eq!(r[0], 1..6);
    assert_eq!(r[1], 12..12);
    assert!(editor.read_with(cx, |e, _| e.shaped_line_count()) < 200);
}
