//! The right click on the text asks the owner for the context menu and
//! moves the caret unless it is inside the selection; the command does the
//! same below the caret.

use std::cell::RefCell;
use std::rc::Rc;

use editor::commands::{Position, SelectAll, SetSelection, ShowContextMenu};
use editor::{Editor, EditorEvent};
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext,
    point, px,
};
use text::Buffer;

struct T<'a> {
    editor: Entity<Editor>,
    asked: Rc<RefCell<Vec<gpui::Point<gpui::Pixels>>>>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> T<'a> {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(Buffer::new(text), window, cx));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    cx.update(|window, cx| {
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            if let EditorEvent::ContextMenu { position } = event {
                sink.borrow_mut().push(*position);
            }
        })
        .detach();
    });
    let mut t = T { editor, asked, cx };
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

    fn geometry(&mut self) -> (gpui::Point<gpui::Pixels>, gpui::Pixels, gpui::Pixels) {
        self.editor
            .read_with(self.cx, |e, _| e.text_geometry().unwrap())
    }

    fn at(&mut self, line: usize, col: usize) -> gpui::Point<gpui::Pixels> {
        let (origin, lh, cw) = self.geometry();
        point(
            origin.x + cw * col as f32 + px(1.0),
            origin.y + lh * line as f32 + lh / 2.0,
        )
    }

    fn right_click(&mut self, at: gpui::Point<gpui::Pixels>) {
        self.cx.simulate_event(MouseDownEvent {
            button: MouseButton::Right,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        });
        self.draw();
    }

    fn caret(&mut self) -> usize {
        self.editor
            .read_with(self.cx, |e, _| e.selections().primary().head)
    }

    fn selected(&mut self) -> bool {
        self.editor
            .read_with(self.cx, |e, _| e.selections().iter().any(|s| !s.is_empty()))
    }

    fn select(&mut self, from: (usize, usize), to: (usize, usize)) {
        self.cx.dispatch_action(SetSelection {
            anchor: Position {
                line: from.0,
                column: from.1,
            },
            head: Position {
                line: to.0,
                column: to.1,
            },
        });
        self.draw();
    }
}

#[gpui::test]
fn a_right_click_moves_the_caret_and_asks_for_the_menu(cx: &mut TestAppContext) {
    let mut t = open(cx, "hello world\nsecond line\nthird");
    let at = t.at(1, 3);
    t.right_click(at);
    assert_eq!(t.caret(), "hello world\nsec".len());
    assert_eq!(t.asked.borrow().as_slice(), &[at]);
}

#[gpui::test]
fn a_right_click_inside_the_selection_keeps_it(cx: &mut TestAppContext) {
    let mut t = open(cx, "hello world\nsecond line\nthird");
    t.cx.dispatch_action(SelectAll);
    t.draw();
    let at = t.at(1, 3);
    t.right_click(at);
    assert!(t.selected());
    assert_eq!(t.asked.borrow().len(), 1);

    // Outside the selection the caret goes to the click.
    t.select((0, 0), (0, 5));
    assert!(t.selected());
    let at = t.at(2, 2);
    t.right_click(at);
    assert!(!t.selected());
    assert_eq!(t.caret(), "hello world\nsecond line\nth".len());
    assert_eq!(t.asked.borrow().len(), 2);
}

#[gpui::test]
fn a_right_click_off_the_text_asks_for_nothing(cx: &mut TestAppContext) {
    let mut t = open(cx, "hello");
    // In the gutter, left of the text.
    let (origin, lh, _) = t.geometry();
    t.right_click(point(origin.x - px(20.0), origin.y + lh / 2.0));
    assert!(t.asked.borrow().is_empty());
}

#[gpui::test]
fn the_command_asks_for_the_menu_below_the_caret(cx: &mut TestAppContext) {
    let mut t = open(cx, "hello\nworld");
    t.select((1, 2), (1, 2));
    t.cx.dispatch_action(ShowContextMenu);
    let asked = t.asked.borrow().clone();
    assert_eq!(asked.len(), 1);
    let (origin, lh, _) = t.geometry();
    assert_eq!(asked[0].y, origin.y + lh * 2.0);
}
