//! Tab and Shift+Tab in the fields of a form: a one-line field moves the
//! focus to the next and the previous field, a multi-line editor keeps Tab
//! as indentation.

use editor::{Editor, commands};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};
use text::Buffer;

/// Three one-line fields with a multi-line editor between the second and
/// the third.
struct Form {
    first: Entity<Editor>,
    second: Entity<Editor>,
    notes: Entity<Editor>,
    third: Entity<Editor>,
}

impl Render for Form {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(editor::field::frame(&self.first, true, false, window, cx))
            .child(editor::field::frame(&self.second, true, false, window, cx))
            .child(div().h(px(80.0)).child(self.notes.clone()))
            .child(editor::field::frame(&self.third, true, false, window, cx))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Form>, &mut VisualTestContext) {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let (form, cx) = cx.add_window_view(|window, cx| Form {
        first: editor::field::new("one", None, true, window, cx),
        second: editor::field::new("two", None, true, window, cx),
        notes: cx.new(|cx| Editor::new(Buffer::new("notes"), window, cx)),
        third: editor::field::new("three", None, true, window, cx),
    });
    cx.update(|window, cx| {
        let focus = form.read(cx).first.focus_handle(cx);
        window.focus(&focus);
    });
    draw(cx);
    (form, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

fn press(cx: &mut VisualTestContext, action: Box<dyn gpui::Action>) {
    cx.update(|window, cx| window.dispatch_action(action, cx));
    draw(cx);
}

fn focused(form: &Entity<Form>, cx: &mut VisualTestContext) -> &'static str {
    cx.update(|window, cx| {
        let form = form.read(cx);
        let fields = [
            ("first", &form.first),
            ("second", &form.second),
            ("notes", &form.notes),
            ("third", &form.third),
        ];
        fields
            .into_iter()
            .find(|(_, e)| e.focus_handle(cx).is_focused(window))
            .map_or("none", |(name, _)| name)
    })
}

#[gpui::test]
fn tab_walks_through_the_one_line_fields(cx: &mut TestAppContext) {
    let (form, cx) = open(cx);
    assert_eq!(focused(&form, cx), "first");
    press(cx, Box::new(commands::Tab));
    assert_eq!(focused(&form, cx), "second");
    // The multi-line editor is not a tab stop: Tab goes past it.
    press(cx, Box::new(commands::Tab));
    assert_eq!(focused(&form, cx), "third");
    // The last field wraps around to the first.
    press(cx, Box::new(commands::Tab));
    assert_eq!(focused(&form, cx), "first");
    // Nothing was typed.
    cx.update(|_, cx| {
        let form = form.read(cx);
        assert_eq!(form.first.read(cx).text(), "one");
        assert_eq!(form.second.read(cx).text(), "two");
        assert_eq!(form.third.read(cx).text(), "three");
    });
}

#[gpui::test]
fn shift_tab_walks_back(cx: &mut TestAppContext) {
    let (form, cx) = open(cx);
    press(cx, Box::new(commands::Unindent));
    assert_eq!(focused(&form, cx), "third");
    press(cx, Box::new(commands::Unindent));
    assert_eq!(focused(&form, cx), "second");
    press(cx, Box::new(commands::Unindent));
    assert_eq!(focused(&form, cx), "first");
}

#[gpui::test]
fn a_multi_line_editor_keeps_tab(cx: &mut TestAppContext) {
    let (form, cx) = open(cx);
    cx.update(|window, cx| {
        let focus = form.read(cx).notes.focus_handle(cx);
        window.focus(&focus);
    });
    draw(cx);
    press(cx, Box::new(commands::Tab));
    assert_eq!(focused(&form, cx), "notes");
    cx.update(|_, cx| {
        assert_eq!(form.read(cx).notes.read(cx).text(), "    notes");
    });
}
