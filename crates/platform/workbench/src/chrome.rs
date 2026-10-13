//! The frame of the main window: main toolbar, tool window stripes and
//! status bar. These are drawn on the frame background around the islands.

use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, Hsla, IntoElement, Pixels, SharedString, Stateful,
    div, prelude::*, px,
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

/// The active file in the middle of the main toolbar: file type icon, the
/// path relative to the project with the full path in brackets, chevron.
/// The text is cut short with an ellipsis when the widget is narrowed.
pub fn file_path_widget(icon: IconName, text: String, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    toolbar_widget("file-path-widget", cx)
        .debug_selector(|| "file-path-widget".into())
        .flex_shrink()
        .min_w(px(0.0))
        .text_color(theme.color(UiRole::Text))
        .child(Icon::new(icon))
        .child(
            div()
                .ml(px(6.0))
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(text),
        )
        .child(
            div()
                .flex_none()
                .ml(px(4.0))
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0))),
        )
}

/// An icon with a round badge at its top right, cut out of the icon by a
/// ring of the background colour, as the New UI draws a running process
/// (`IconManager.withIconBadge`, `BadgeDotProvider`: a dot of radius 3.5
/// at (16.5, 3.5) of a 20 px icon with a 1.5 ring, scaled to the icon).
pub fn badged_icon(icon: IconName, size: Pixels, badge: Option<Hsla>, bg: Hsla) -> AnyElement {
    let scale = f32::from(size) / 20.0;
    let r = 3.5 * scale;
    let ring = 1.5 * scale;
    let (cx_, cy) = (16.5 * scale, 3.5 * scale);
    div()
        .relative()
        .flex_none()
        .size(size)
        .child(Icon::new(icon).size(size))
        .when_some(badge, |d, badge| {
            let outer = r + ring;
            d.child(
                div()
                    .absolute()
                    .left(px(cx_ - outer))
                    .top(px(cy - outer))
                    .size(px(outer * 2.0))
                    .rounded_full()
                    .bg(bg),
            )
            .child(
                div()
                    .absolute()
                    .left(px(cx_ - r))
                    .top(px(cy - r))
                    .size(px(r * 2.0))
                    .rounded_full()
                    .bg(badge),
            )
        })
        .into_any_element()
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

/// A tool window stripe button whose icon may carry a badge (the Run tool
/// window's live indicator).
pub fn stripe_button_badged(
    id: impl Into<ElementId>,
    icon: IconName,
    title: &'static str,
    shortcut: Option<SharedString>,
    selected: bool,
    badge: Option<Hsla>,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    let pressed = theme.color(UiRole::Pressed);
    let bg = if selected {
        pressed
    } else {
        theme.color(UiRole::ToolWindowStripe)
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(theme.metric(Metric::StripeButtonSize))
        .rounded(theme.metric(Metric::ButtonRadius))
        .when(selected, |d| d.bg(pressed))
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .active(move |s| s.bg(pressed))
        .child(badged_icon(icon, px(20.0), badge, bg))
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
