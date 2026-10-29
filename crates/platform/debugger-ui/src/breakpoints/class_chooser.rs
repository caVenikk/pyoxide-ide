//! The class chooser of a new exception breakpoint (the reference's
//! `PyClassTreeChooserDialog` "Select Exception Class", its "Search by
//! Name" page): a modal dialog over the Breakpoints dialog with a name
//! field and the exception classes the product offers that match it.
//! Up and Down move in the list, Enter and a double click take the
//! selected class (or the dotted name typed when nothing matches, for a
//! class of the project), Escape cancels.

use editor::{Editor, EditorEvent};
use gpui::Focusable as _;
use gpui::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, IntoElement,
    ParentElement as _, Render, ScrollHandle, SharedString, Styled as _, Subscription, WeakEntity,
    Window, WindowHandle, div, prelude::*, px, size,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt as _;

use crate::breakpoints::commands::{
    ChooserCancel, ChooserNext, ChooserOk, ChooserPrevious, ChooserSetQuery,
};
use crate::breakpoints::dialog::BreakpointsDialog;

/// Key context of the chooser.
pub const CONTEXT: &str = "ExceptionClassChooser";
const WIDTH: f32 = 500.0;
const HEIGHT: f32 = 400.0;
const INSET: f32 = 12.0;
const ROW: f32 = 22.0;

/// The class chooser.
pub struct ClassChooser {
    title: SharedString,
    classes: Vec<String>,
    query: Entity<Editor>,
    /// The classes that match the query, in the list's order.
    rows: Vec<String>,
    selected: Option<usize>,
    scroll: ScrollHandle,
    /// The Breakpoints dialog that adds the chosen class, and its window.
    owner: WeakEntity<BreakpointsDialog>,
    owner_window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl ClassChooser {
    /// Opens the chooser over the Breakpoints dialog's window `window`.
    pub(crate) fn open(
        title: String,
        classes: Vec<String>,
        owner: WeakEntity<BreakpointsDialog>,
        activate: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<WindowHandle<Self>> {
        let title_height = ui::dialog_window::title_height(true);
        let options = ui::dialog_window::DialogOptions {
            title: title.clone().into(),
            size: size(px(WIDTH), px(HEIGHT + title_height)),
            min_size: Some(size(px(300.0), px(240.0))),
            activate,
            themed_title: true,
            modal: true,
            key: None,
        };
        let owner_window = window.window_handle();
        ui::dialog_window::open(window, options, cx, move |window, cx| {
            cx.new(|cx| Self::new(title.into(), classes, owner, owner_window, window, cx))
        })
    }

    fn new(
        title: SharedString,
        classes: Vec<String>,
        owner: WeakEntity<BreakpointsDialog>,
        owner_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = editor::field::new("", None, true, window, cx);
        window.focus(&query.focus_handle(cx));
        let subscriptions = vec![cx.subscribe(&query, |this, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                this.filter(cx);
            }
        })];
        let mut chooser = Self {
            title,
            classes,
            query,
            rows: Vec::new(),
            selected: None,
            scroll: ScrollHandle::new(),
            owner,
            owner_window,
            _subscriptions: subscriptions,
        };
        chooser.filter(cx);
        chooser
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).text();
        self.rows = matching(&self.classes, query.trim());
        self.selected = (!self.rows.is_empty()).then_some(0);
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    /// The class Enter takes: the selected row, else a dotted name typed.
    fn chosen(&self, cx: &App) -> Option<String> {
        if let Some(row) = self.selected.and_then(|i| self.rows.get(i)) {
            return Some(row.clone());
        }
        let typed = self.query.read(cx).text();
        let typed = typed.trim();
        is_dotted_name(typed).then(|| typed.to_owned())
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        let at = self
            .selected
            .map_or(0, |s| (s as isize + by).clamp(0, last)) as usize;
        self.selected = Some(at);
        self.scroll.scroll_to_item(at);
        cx.notify();
    }

    fn on_previous(&mut self, _: &ChooserPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    fn on_next(&mut self, _: &ChooserNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn on_ok(&mut self, _: &ChooserOk, window: &mut Window, cx: &mut Context<Self>) {
        let Some(class) = self.chosen(cx) else {
            return;
        };
        let (owner, owner_window) = (self.owner.clone(), self.owner_window);
        Self::close(window, cx);
        // After this update: the Breakpoints dialog selects the new row in
        // its own window.
        cx.defer(move |cx| {
            let _ = owner_window.update(cx, |_, window, cx| {
                let _ = owner.update(cx, |d, cx| d.add_exception(&class, window, cx));
            });
        });
    }

    fn on_set_query(&mut self, a: &ChooserSetQuery, _: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |q, cx| q.set_text_selected(&a.text, cx));
        self.filter(cx);
    }

    fn on_cancel(&mut self, _: &ChooserCancel, window: &mut Window, cx: &mut Context<Self>) {
        Self::close(window, cx);
    }

    /// Closes the chooser from one of its own handlers: its window is the
    /// one being updated, so it is removed here.
    fn close(window: &mut Window, cx: &mut App) {
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    /// The state `debug.state` reports in the Breakpoints dialog's.
    pub(crate) fn state_json(&self, cx: &App) -> Json {
        json!({
            "title": self.title.as_ref(),
            "query": self.query.read(cx).text(),
            "rows": self.rows,
            "selected": self.selected,
        })
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let theme = cx.theme();
        let selection = theme.color(UiRole::Selection);
        let hover = theme.color(UiRole::Hover);
        self.rows
            .iter()
            .enumerate()
            .map(|(i, class)| {
                let selected = self.selected == Some(i);
                div()
                    .id(("class-chooser-row", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(ROW))
                    .px(px(8.0))
                    .flex_none()
                    .when(selected, |d| d.bg(selection))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(Icon::new(IconName::NODE_CLASS))
                    .child(class.clone())
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.selected = Some(i);
                        cx.notify();
                        if e.click_count() > 1 {
                            this.on_ok(&ChooserOk, window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

/// The classes that contain `query` (ignoring case), those that start
/// with it first; all of them for an empty query.
fn matching(classes: &[String], query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    let mut prefix = Vec::new();
    let mut inner = Vec::new();
    for class in classes {
        let lower = class.to_lowercase();
        if lower.starts_with(&query) {
            prefix.push(class.clone());
        } else if lower.contains(&query) {
            inner.push(class.clone());
        }
    }
    prefix.extend(inner);
    prefix
}

/// `pkg.module.Error`: identifiers joined by dots.
fn is_dotted_name(text: &str) -> bool {
    !text.is_empty()
        && text.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
                && chars.all(|c| c == '_' || c.is_alphanumeric())
        })
}

impl Render for ClassChooser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.render_rows(cx);
        let ok_enabled = self.chosen(cx).is_some();
        let field = editor::field::frame(&self.query, true, false, window, cx);
        let theme = cx.theme();
        let bar = BottomBar::new()
            .button(BarButton::new(
                "class-chooser-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(ChooserCancel),
            ))
            .button(
                BarButton::new(
                    "class-chooser-ok",
                    "OK",
                    ButtonRole::Ok,
                    Box::new(ChooserOk),
                )
                .enabled(ok_enabled),
            )
            .render(cx);
        div()
            .id("class-chooser")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_ok))
            .on_action(cx.listener(Self::on_cancel))
            .on_action(cx.listener(Self::on_set_query))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .children(ui::dialog_window::title_bar(self.title.clone(), true, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(8.0))
                    .p(px(INSET))
                    .child(field)
                    .child(
                        div()
                            .id("class-chooser-list")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .border_1()
                            .border_color(theme.color(UiRole::Border))
                            .children(rows),
                    ),
            )
            .child(bar)
            .child(ui::pointer::frame())
    }
}

impl gpui::Focusable for ClassChooser {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes() -> Vec<String> {
        [
            "KeyError",
            "ValueError",
            "UnicodeError",
            "ZeroDivisionError",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn prefix_matches_come_first() {
        assert_eq!(matching(&classes(), ""), classes());
        assert_eq!(
            matching(&classes(), "e"),
            [
                "KeyError",
                "ValueError",
                "UnicodeError",
                "ZeroDivisionError"
            ]
        );
        assert_eq!(matching(&classes(), "u"), ["UnicodeError", "ValueError"]);
        assert_eq!(matching(&classes(), "VAL"), ["ValueError"]);
        assert!(matching(&classes(), "nothing").is_empty());
    }

    #[test]
    fn a_typed_class_is_a_dotted_name() {
        assert!(is_dotted_name("ValueError"));
        assert!(is_dotted_name("pkg.errors.MyError"));
        assert!(is_dotted_name("_Private"));
        assert!(!is_dotted_name(""));
        assert!(!is_dotted_name("pkg..Error"));
        assert!(!is_dotted_name("1Error"));
        assert!(!is_dotted_name("my error"));
    }
}
