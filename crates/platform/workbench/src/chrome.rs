//! The frame of the main window: main toolbar, tool window stripes and
//! status bar. These are drawn on the frame background around the islands.

use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, Hsla, IntoElement, Pixels, SharedString, Stateful,
    div, prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button};

use crate::main_toolbar::Fitted;
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

/// Horizontal padding of a dropdown-like widget of the main toolbar.
const WIDGET_PADDING: f32 = 8.0;
/// Size of the chevron after a widget's text.
const CHEVRON: f32 = 16.0;
/// Size of the project badge and the gap between the project widget's
/// parts.
const BADGE: f32 = 20.0;
const PROJECT_GAP: f32 = 6.0;
/// Size of the file widget's icon, its gap to the text and the text's gap
/// to the chevron.
const FILE_ICON: f32 = 16.0;
const FILE_ICON_GAP: f32 = 6.0;
const FILE_CHEVRON_GAP: f32 = 4.0;

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
        .px(px(WIDGET_PADDING))
        .rounded(theme.metric(Metric::ButtonRadius))
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
}

/// Width of a toolbar widget's padding and chevron, with `gap` before the
/// chevron.
pub fn toolbar_widget_chrome(gap: f32) -> f32 {
    2.0 * WIDGET_PADDING + gap + CHEVRON
}

/// A widget's text in the box the main toolbar gave it, cut short
/// already; centred when the cut leaves room (`AbstractToolbarComboUI`).
pub fn widget_text(text: &Fitted) -> Div {
    div()
        .flex_none()
        .w(text.width)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_center()
        .child(text.text.clone())
}

/// The project widget's label: the name, or "No project".
pub fn project_label(name: Option<&str>) -> &str {
    name.unwrap_or("No project")
}

/// Width of the project widget without its label.
pub fn project_widget_chrome(has_project: bool) -> f32 {
    let badge = if has_project {
        BADGE + PROJECT_GAP
    } else {
        0.0
    };
    toolbar_widget_chrome(PROJECT_GAP) + badge
}

/// The project widget: coloured badge with initials, name, chevron; the
/// name as the toolbar fitted it (see [`project_label`]).
pub fn project_widget(name: Option<&str>, label: &Fitted, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let badge = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(BADGE))
        .rounded(px(4.0))
        .bg(theme.color(UiRole::Accent))
        .text_color(theme.color(UiRole::OnAccent))
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials(name.unwrap_or("?")));
    toolbar_widget("project-widget", cx)
        .debug_selector(|| "project-widget".into())
        .gap(px(PROJECT_GAP))
        .when(name.is_some(), |d| d.child(badge))
        .child(
            widget_text(label)
                .debug_selector(|| "project-widget-label".into())
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(
            div()
                .flex_none()
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(CHEVRON))),
        )
}

/// Width of the file widget without its text.
pub fn file_path_widget_chrome() -> f32 {
    toolbar_widget_chrome(FILE_CHEVRON_GAP) + FILE_ICON + FILE_ICON_GAP
}

/// The active file in the middle of the main toolbar: file type icon, the
/// path relative to the project with the full path in brackets, chevron;
/// the text as the toolbar fitted it.
pub fn file_path_widget(icon: IconName, text: &Fitted, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    toolbar_widget("file-path-widget", cx)
        .debug_selector(|| "file-path-widget".into())
        .text_color(theme.color(UiRole::Text))
        .child(div().flex_none().child(Icon::new(icon).size(px(FILE_ICON))))
        .child(
            widget_text(text)
                .debug_selector(|| "file-path-widget-text".into())
                .ml(px(FILE_ICON_GAP)),
        )
        .child(
            div()
                .flex_none()
                .ml(px(FILE_CHEVRON_GAP))
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(CHEVRON))),
        )
}

/// Room between the project widget and the VCS widget, and the VCS
/// widget's gaps: icon to text, text to the incoming and outgoing arrows
/// (12 px each), arrows to the chevron.
pub const VCS_WIDGET_SPACE: f32 = 16.0;
const VCS_ICON_GAP: f32 = 6.0;
const VCS_ARROWS_GAP: f32 = 6.0;
const VCS_ARROW: f32 = 12.0;

/// Width of the VCS widget without its text.
pub fn vcs_widget_chrome(icon: bool, arrows: usize) -> f32 {
    let icon = if icon { FILE_ICON + VCS_ICON_GAP } else { 0.0 };
    let arrows = if arrows > 0 {
        VCS_ARROWS_GAP + arrows as f32 * VCS_ARROW
    } else {
        0.0
    };
    toolbar_widget_chrome(FILE_CHEVRON_GAP) + icon + arrows
}

/// The VCS widget next to the project widget (`GitToolbarWidgetAction`):
/// the branch icon (a tag or a warning when `HEAD` is detached), the
/// branch, the incoming and outgoing arrows, the chevron; "Version
/// Control" without an icon when the project has no repository.
pub fn vcs_widget(
    icon: Option<IconName>,
    text: &Fitted,
    incoming: bool,
    outgoing: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    toolbar_widget("vcs-widget", cx)
        .debug_selector(|| "vcs-widget".into())
        .text_color(theme.color(UiRole::Text))
        .children(icon.map(|icon| {
            div()
                .flex_none()
                .mr(px(VCS_ICON_GAP))
                .child(Icon::new(icon).size(px(FILE_ICON)))
        }))
        .child(widget_text(text).debug_selector(|| "vcs-widget-text".into()))
        .when(incoming || outgoing, |d| {
            d.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .ml(px(VCS_ARROWS_GAP))
                    .when(incoming, |d| {
                        d.child(Icon::new(IconName::INCOMING).size(px(VCS_ARROW)))
                    })
                    .when(outgoing, |d| {
                        d.child(Icon::new(IconName::OUTGOING).size(px(VCS_ARROW)))
                    }),
            )
        })
        .child(
            div()
                .flex_none()
                .ml(px(FILE_CHEVRON_GAP))
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(CHEVRON))),
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

/// A vertical stripe with a top and a bottom group of buttons (debug
/// selectors `{id}-top`, `{id}-bottom`). The groups keep their places at
/// the top and at the bottom; in a low window the top group is drawn over
/// the bottom one and covers as much of it as the window lacks
/// (`ToolWindowToolbar`: the top stripe in the north of a border layout,
/// the bottom stripe in the south, the north one painted over it).
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
    let background = theme.color(UiRole::ToolWindowStripe);
    let group = || div().flex().flex_col().items_center().gap(px(4.0));
    div()
        .id(id)
        .debug_selector(move || id.into())
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(width)
        .h_full()
        .overflow_hidden()
        .bg(background)
        // Drawn first, so that the top group covers it.
        .child(
            group()
                .debug_selector(move || format!("{id}-bottom"))
                .absolute()
                .left_0()
                .right_0()
                .bottom(inset)
                .children(bottom),
        )
        .child(
            group()
                .debug_selector(move || format!("{id}-top"))
                .occlude()
                .pt(px(2.0))
                .bg(background)
                .children(top),
        )
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
