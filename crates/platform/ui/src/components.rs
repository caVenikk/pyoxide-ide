//! Small building blocks shared by the workbench and the editor. Every
//! colour and size comes from the theme.

use gpui::{
    AnyView, App, Context, Div, ElementId, FontWeight, IntoElement, Render, SharedString, Stateful,
    Window, div, prelude::*, px,
};
use themes::{Metric, UiRole};

use crate::icons::{Icon, IconName};
use crate::theme::ThemeExt;

/// An island: a rounded panel of the main window.
pub fn island(cx: &App, background: UiRole) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(theme.metric(Metric::IslandRadius))
        .bg(theme.color(background))
}

/// A square icon button with a rounded hover background, as in the main
/// toolbar and the tool window stripes.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    button_size: gpui::Pixels,
    icon_size: gpui::Pixels,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    let pressed = theme.color(UiRole::Pressed);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(button_size)
        .rounded(theme.metric(Metric::ButtonRadius))
        .when(selected, |d| d.bg(pressed))
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .active(move |s| s.bg(pressed))
        .child(Icon::new(icon).size(icon_size))
}

/// A tooltip with a title and an optional shortcut in muted text.
pub struct Tooltip {
    title: SharedString,
    shortcut: Option<SharedString>,
}

impl Tooltip {
    /// A tooltip view for `.tooltip(...)` of a stateful element.
    pub fn text(
        title: impl Into<SharedString>,
        shortcut: Option<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let title = title.into();
        move |_, cx| {
            cx.new(|_| Tooltip {
                title: title.clone(),
                shortcut: shortcut.clone(),
            })
            .into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(theme.metric(Metric::CornerRadius) * 2.0)
            .border_1()
            .border_color(theme.color(UiRole::Border))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow_md()
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .child(self.title.clone())
            .children(
                self.shortcut
                    .clone()
                    .map(|s| div().text_color(theme.color(UiRole::TextMuted)).child(s)),
            )
    }
}

/// Default text style of a window's root element: the UI font, size and
/// colour of the theme.
pub fn root(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .size_full()
        .font_family(theme.ui_family())
        .text_size(theme.ui_size())
        .text_color(theme.color(UiRole::Text))
}

/// Weight of titles such as the tool window name.
pub const TITLE_WEIGHT: FontWeight = FontWeight::SEMIBOLD;
