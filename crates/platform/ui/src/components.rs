//! Small building blocks shared by the workbench and the editor. Every
//! colour and size comes from the theme.

use gpui::{
    AnyView, App, Context, Div, ElementId, FontWeight, IntoElement, PathBuilder, Render,
    SharedString, Stateful, Window, canvas, div, point, prelude::*, px,
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

/// Size of the small buttons inside bars and fields (find bar arrows,
/// option toggles) and of their icons.
pub const SMALL_BUTTON: f32 = 22.0;
pub const SMALL_ICON: f32 = 16.0;
/// Opacity of a disabled button.
pub const DISABLED_ALPHA: f32 = 0.4;

/// A small icon button for actions inside bars and fields (the find bar's
/// previous and next arrows, its close button). A disabled button is
/// dimmed and has no hover background; the caller adds the click handler
/// only when it is enabled.
pub fn small_icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    enabled: bool,
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
        .size(px(SMALL_BUTTON))
        .rounded(theme.metric(Metric::CornerRadius))
        .when(enabled, |d| {
            d.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed))
        })
        .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
        .child(Icon::new(icon).size(px(SMALL_ICON)))
}

/// An on/off option shown as an icon (Match Case, Words, Regex in a search
/// field): a rounded background while it is on, a lighter one on hover.
pub fn toggle_button(
    id: impl Into<ElementId>,
    icon: IconName,
    on: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    let selected = theme.color(UiRole::SearchOptionSelected);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(SMALL_BUTTON))
        .rounded(theme.metric(Metric::CornerRadius))
        .when(on, |d| d.bg(selected))
        .when(!on, |d| d.hover(move |s| s.bg(hover)))
        .child(Icon::new(icon).size(px(SMALL_ICON)))
}

/// A push button with a text label (Replace, Replace All): a rounded
/// outline, a hover background; dimmed when disabled.
pub fn text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
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
        .h(px(SMALL_BUTTON + 2.0))
        .px(px(10.0))
        .rounded(theme.metric(Metric::CornerRadius))
        .border_1()
        .border_color(theme.color(UiRole::BorderInput))
        .whitespace_nowrap()
        .when(enabled, |d| {
            d.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed))
        })
        .when(!enabled, |d| {
            d.opacity(DISABLED_ALPHA)
                .text_color(theme.color(UiRole::TextDisabled))
        })
        .child(label.into())
}

/// Size of a check box.
pub const CHECKBOX: f32 = 16.0;

/// A check box: a rounded square with the input outline, filled with the
/// accent colour and a check mark while it is on. The caller adds the click
/// handler (usually on the check box and its label together).
pub fn checkbox(id: impl Into<ElementId>, checked: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let mark = theme.color(UiRole::OnAccent);
    div()
        .id(id)
        .flex_none()
        .size(px(CHECKBOX))
        .rounded(theme.metric(Metric::CornerRadiusSmall))
        .when(checked, |d| {
            d.bg(theme.color(UiRole::Accent)).child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let o = bounds.origin;
                        let at = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
                        let mut path = PathBuilder::stroke(px(1.75));
                        path.move_to(at(4.0, 8.0));
                        path.line_to(at(7.0, 11.0));
                        path.line_to(at(12.0, 5.0));
                        if let Ok(path) = path.build() {
                            window.paint_path(path, mark);
                        }
                    },
                )
                .size_full(),
            )
        })
        .when(!checked, |d| {
            d.border_1().border_color(theme.color(UiRole::BorderInput))
        })
}

/// The frame of a one-line text input: rounded, with the input outline;
/// while `focused` the theme's focus ring is drawn over the outline, and
/// with `error` the outline takes the danger colour. Children go in a row:
/// leading icons, the text field (an editor in one-line mode), trailing
/// buttons.
pub fn input_frame(focused: bool, error: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let radius = theme.metric(Metric::CornerRadius) * 1.5;
    let border = if error {
        theme.color(UiRole::Danger)
    } else {
        theme.color(UiRole::BorderInput)
    };
    let ring = theme.color(if error { UiRole::Danger } else { UiRole::Focus });
    let ring_width = theme.metric(Metric::FocusWidth);
    // Light themes fill the field; dark ones let the surface show through
    // (as the reference IDE's fields in the islands themes).
    let fill = (!theme.is_dark()).then(|| theme.color(UiRole::Input));
    div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .min_w(px(0.0))
        .rounded(radius)
        .border_1()
        .border_color(border)
        .when_some(fill, |d, fill| d.bg(fill))
        .when(focused, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(-1.0))
                    .left(px(-1.0))
                    .right(px(-1.0))
                    .bottom(px(-1.0))
                    .rounded(radius)
                    .border(ring_width)
                    .border_color(ring),
            )
        })
}

/// Status text next to a search field ("3/12", "0 results"): muted, or the
/// danger colour when nothing was found or the pattern is wrong.
pub fn status_text(text: impl Into<SharedString>, error: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let color = if error {
        theme.color(UiRole::Danger)
    } else {
        theme.color(UiRole::TextMuted)
    };
    div()
        .flex_none()
        .whitespace_nowrap()
        .text_color(color)
        .child(text.into())
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
