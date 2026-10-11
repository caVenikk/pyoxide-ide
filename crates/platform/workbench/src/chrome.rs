//! The frame of the main window: main toolbar, tool window stripes and
//! status bar. These are drawn on the frame background around the islands.

use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, IntoElement, Pixels, SharedString, Stateful, div,
    prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

/// Space kept free for the macOS window buttons at the left of the toolbar.
pub const TRAFFIC_LIGHTS_INSET: f32 = 80.0;

/// Initials for the project badge: the first letters of the first two
/// words of the name (words split at `-`, `_`, `.` and spaces), upper case.
pub fn initials(name: &str) -> String {
    let mut words = name
        .split(|c: char| c == '-' || c == '_' || c == '.' || c.is_whitespace())
        .filter(|w| !w.is_empty());
    let mut out = String::new();
    if let Some(first) = words.next() {
        let mut chars = first.chars();
        out.extend(chars.next().map(|c| c.to_uppercase()).into_iter().flatten());
        match words.next() {
            Some(second) => out.extend(
                second
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase())
                    .into_iter()
                    .flatten(),
            ),
            None => out.extend(chars.next().map(|c| c.to_uppercase()).into_iter().flatten()),
        }
    }
    out
}

/// A dropdown-like widget of the main toolbar: rounded hover background,
/// content in a row.
pub fn toolbar_widget(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    let pressed = theme.color(UiRole::Pressed);
    div()
        .id(id)
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(theme.metric(Metric::ToolbarButtonSize))
        .px(px(8.0))
        .rounded(theme.metric(Metric::ButtonRadius))
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
}

/// The project widget: coloured badge with initials, name, chevron.
pub fn project_widget(name: Option<&str>, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let label: SharedString = name.unwrap_or("No project").to_owned().into();
    let badge = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(20.0))
        .rounded(px(4.0))
        .bg(theme.color(UiRole::Accent))
        .text_color(theme.color(UiRole::OnAccent))
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials(name.unwrap_or("?")));
    toolbar_widget("project-widget", cx)
        .debug_selector(|| "project-widget".into())
        .gap(px(6.0))
        .when(name.is_some(), |d| d.child(badge))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .child(label),
        )
        .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0)))
}

/// The run configuration widget and the run buttons. Run configurations
/// arrive with the run area; until then the widget is a placeholder.
pub fn run_widgets(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let button = theme.metric(Metric::ToolbarButtonSize);
    let muted = theme.color(UiRole::TextMuted);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .child(
            toolbar_widget("run-configuration", cx)
                .gap(px(4.0))
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_color(muted)
                        .child("Current File"),
                )
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0))),
        )
        .child(
            icon_button("run", IconName::RUN, button, px(16.0), false, cx)
                .tooltip(Tooltip::text("Run", None)),
        )
        .child(
            icon_button("debug", IconName::DEBUG, button, px(16.0), false, cx)
                .tooltip(Tooltip::text("Debug", None)),
        )
        .child(
            icon_button(
                "run-more",
                IconName::MORE_VERTICAL,
                button,
                px(16.0),
                false,
                cx,
            )
            .tooltip(Tooltip::text("More Actions", None)),
        )
}

/// A tool window stripe button.
pub fn stripe_button(
    id: impl Into<ElementId>,
    icon: IconName,
    title: &'static str,
    shortcut: Option<SharedString>,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    icon_button(
        id,
        icon,
        theme.metric(Metric::StripeButtonSize),
        px(20.0),
        selected,
        cx,
    )
    .tooltip(Tooltip::text(title, shortcut))
}

/// A vertical stripe with a top and a bottom group of buttons.
pub fn stripe(
    id: &'static str,
    top: Vec<AnyElement>,
    bottom: Vec<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let width = theme.metric(Metric::StripeWidth);
    let button = theme.metric(Metric::StripeButtonSize);
    let inset = (width - button) / 2.0;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .justify_between()
        .w(width)
        .h_full()
        .pt(px(2.0))
        .pb(inset)
        .bg(theme.color(UiRole::ToolWindowStripe))
        .child(div().flex().flex_col().gap(px(4.0)).children(top))
        .child(div().flex().flex_col().gap(px(4.0)).children(bottom))
}

/// One text widget of the status bar.
pub fn status_widget(id: impl Into<ElementId>, text: SharedString, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(22.0))
        .px(px(6.0))
        .rounded(px(4.0))
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover))
        .child(text)
}

/// Height of the status bar and its text size.
pub fn status_metrics(cx: &App) -> (Pixels, Pixels) {
    let theme = cx.theme();
    (
        theme.metric(Metric::StatusBarHeight),
        theme.ui_size() - px(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_project_names() {
        assert_eq!(initials("pyoxide-ide"), "PI");
        assert_eq!(initials("flask"), "FL");
        assert_eq!(initials("my_project.app"), "MP");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials("ёлка дом"), "ЁД");
        assert_eq!(initials(""), "");
    }
}
