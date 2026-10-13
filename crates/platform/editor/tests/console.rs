//! An editor set up as a console (the Run tool window's view): output
//! appended outside the undo history to a read-only buffer, explicit
//! styles and links from the highlighter, links followed on click, input
//! forwarded instead of edited, and sticking to the end.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use editor::{Editor, EditorEvent, ForwardedInput, Highlighter, InlineStyle, Span};
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, point, px,
};
use text::{Buffer, Edit};

/// Line 1 of the text is a link from column 4 to 9, painted red.
struct Links;

impl Highlighter for Links {
    fn reset(&mut self, _: &Buffer) {}
    fn edited(&mut self, _: &Buffer, _: &[text::AppliedEdit]) {}
    fn line_spans(&self, _: usize, _: &mut Vec<Span>) {}
    fn line_styles(
        &self,
        line: usize,
        _: &ui::theme::Theme,
        out: &mut Vec<(Range<usize>, InlineStyle)>,
    ) {
        if line == 0 {
            out.push((
                0..3,
                InlineStyle {
                    color: Some(gpui::red()),
                    ..InlineStyle::default()
                },
            ));
        }
    }
    fn line_links(&self, line: usize, out: &mut Vec<Range<usize>>) {
        if line == 1 {
            out.push(4..9);
        }
    }
}

struct T<'a> {
    editor: Entity<Editor>,
    events: Rc<RefCell<Vec<EditorEvent>>>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> T<'a> {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut buffer = Buffer::new(text);
        buffer.set_read_only(true);
        let mut e = Editor::new(buffer, window, cx);
        e.set_console(true);
        e.set_forward_input(true);
        e.set_highlighter(Some(Box::new(Links)), cx);
        e
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|window, cx| {
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            if matches!(
                event,
                EditorEvent::Input(_) | EditorEvent::LinkClicked { .. }
            ) {
                sink.borrow_mut().push(event.clone());
            }
        })
        .detach();
    });
    let mut t = T { editor, events, cx };
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

    fn text(&mut self) -> String {
        self.editor.read_with(self.cx, |e, _| e.text())
    }

    fn at(&mut self, line: usize, col: usize) -> gpui::Point<gpui::Pixels> {
        let (origin, lh, cw) = self
            .editor
            .read_with(self.cx, |e, _| e.text_geometry().unwrap());
        point(
            origin.x + cw * col as f32 + px(1.0),
            origin.y + lh * line as f32 + lh / 2.0,
        )
    }

    fn press(&mut self, at: gpui::Point<gpui::Pixels>) {
        self.cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        });
    }

    fn release(&mut self, at: gpui::Point<gpui::Pixels>) {
        self.cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
        });
        self.draw();
    }

    fn take_events(&mut self) -> Vec<EditorEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

#[gpui::test]
fn output_is_appended_outside_the_history(cx: &mut TestAppContext) {
    let mut t = open(cx, "a\n");
    t.editor.update(t.cx, |e, cx| {
        let end = e.buffer().len_chars();
        e.replace_untracked(vec![Edit::insert(end, "bc\nde")], cx);
        // A lone `\r` replaced the last line: the console drops and rewrites it.
        let line = e.buffer().line_start(2);
        let end = e.buffer().len_chars();
        e.replace_untracked(vec![Edit::new(line..end, "xy")], cx);
    });
    t.draw();
    assert_eq!(t.text(), "a\nbc\nxy");
    t.editor.read_with(t.cx, |e, _| {
        assert!(!e.buffer().can_undo());
        assert!(!e.is_modified());
        assert!(e.buffer().is_read_only());
    });
    // Trimming the front keeps a selection on the same text.
    t.editor.update(t.cx, |e, cx| {
        e.set_selections(text::SelectionSet::single(text::Selection::new(2, 4)), cx);
        e.replace_untracked(vec![Edit::delete(0..2)], cx);
    });
    t.editor.read_with(t.cx, |e, _| {
        let s = e.selections().primary();
        assert_eq!((s.anchor, s.head), (0, 2));
        assert_eq!(e.buffer().slice(0..2), "bc");
    });
}

#[gpui::test]
fn input_is_forwarded_instead_of_edited(cx: &mut TestAppContext) {
    let mut t = open(cx, "prompt? ");
    t.cx.simulate_input("42");
    t.draw();
    for keys in ["enter", "backspace", "delete", "tab"] {
        t.cx.update(|window, cx| {
            let action: Box<dyn gpui::Action> = match keys {
                "enter" => Box::new(editor::commands::Newline),
                "backspace" => Box::new(editor::commands::Backspace),
                "delete" => Box::new(editor::commands::Delete),
                _ => Box::new(editor::commands::Tab),
            };
            window.dispatch_action(action, cx);
        });
        t.draw();
    }
    assert_eq!(t.text(), "prompt? ");
    assert_eq!(
        t.take_events()
            .into_iter()
            .map(|e| match e {
                EditorEvent::Input(i) => i,
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>(),
        [
            ForwardedInput::Text("4".into()),
            ForwardedInput::Text("2".into()),
            ForwardedInput::Enter,
            ForwardedInput::Backspace,
            ForwardedInput::Delete,
            ForwardedInput::Tab,
        ]
    );
}

#[gpui::test]
fn a_link_is_followed_on_a_plain_click(cx: &mut TestAppContext) {
    let mut t = open(cx, "out\nsee https://x\n");
    // Pressed and released at the same point: followed.
    let at = t.at(1, 6);
    t.press(at);
    t.release(at);
    let events = t.take_events();
    assert!(
        matches!(&events[..], [EditorEvent::LinkClicked { line: 1, range }] if range == &(4..9)),
        "{events:?}"
    );
    // A drag that ends elsewhere selects instead.
    let from = t.at(1, 5);
    t.press(from);
    let to = t.at(1, 8);
    t.release(to);
    assert!(t.take_events().is_empty());
    // Outside the link nothing is followed.
    let at = t.at(1, 1);
    t.press(at);
    t.release(at);
    assert!(t.take_events().is_empty());
}

#[gpui::test]
fn the_console_sticks_to_the_end(cx: &mut TestAppContext) {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let mut t = open(cx, &text);
    assert!(!t.editor.read_with(t.cx, |e, _| e.is_scrolled_to_end()));
    t.editor.update(t.cx, |e, cx| e.scroll_to_end(cx));
    t.draw();
    t.editor.read_with(t.cx, |e, _| {
        assert!(e.is_scrolled_to_end());
        // The last line is at the bottom of the view, not at the top.
        let visible = e.visible_lines().unwrap();
        assert!(visible.end >= e.buffer().len_lines());
        assert!(visible.start > 100);
    });
}
