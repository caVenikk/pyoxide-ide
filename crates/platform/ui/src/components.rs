//! Small building blocks shared by the workbench and the editor. Every
//! colour and size comes from the theme.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyView, App, BoxShadow, Context, Div, ElementId, FontWeight,
    IntoElement, PathBuilder, Render, SharedString, Stateful, Window, canvas, div, point,
    prelude::*, px,
};
use themes::{Controls, Metric, UiRole};

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

/// Popup shadow: `Ide.Shadow` of the Islands themes, 16 px of `#00000020`
/// in both the dark and the light theme.
const SHADOW_BLUR: f32 = 16.0;
const SHADOW_ALPHA: f32 = 32.0 / 255.0;

/// The shadow of popups, menus and tooltips.
pub fn popup_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: gpui::hsla(0.0, 0.0, 0.0, SHADOW_ALPHA),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(SHADOW_BLUR),
        spread_radius: px(0.0),
    }]
}

/// Size of a check box with the New UI's controls (the box of the 24 px
/// check box icons of the New UI themes); classic controls have
/// [`CLASSIC_CHECKBOX`].
pub const CHECKBOX: f32 = 16.0;
/// Size of a check box with classic controls (the 14 px box of the
/// Darcula check box icons).
pub const CLASSIC_CHECKBOX: f32 = 14.0;

/// The size of a check box with the theme's controls.
pub fn checkbox_size(cx: &App) -> f32 {
    match cx.theme().controls() {
        Controls::New => CHECKBOX,
        Controls::Classic => CLASSIC_CHECKBOX,
    }
}

/// A check box as the reference draws its check box icons: a rounded
/// square with an outline and a fill, a check mark while it is on, in the
/// theme's check box colours (`checkbox-*`, the `Checkbox.*` keys of the
/// icon palette); 16 px with round strokes for the New UI's controls, 14 px
/// for classic ones. The caller adds the click handler (usually on the
/// check box and its label together).
pub fn checkbox(id: impl Into<ElementId>, checked: bool, cx: &App) -> Stateful<Div> {
    checkbox_with(id, checked, true, cx)
}

/// A check box that may be disabled ([`checkbox`]).
pub fn checkbox_with(
    id: impl Into<ElementId>,
    checked: bool,
    enabled: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let (background, border, mark) = match (checked, enabled) {
        (_, false) => (
            UiRole::CheckboxBackgroundDisabled,
            UiRole::CheckboxBorderDisabled,
            UiRole::CheckboxCheckDisabled,
        ),
        (true, true) => (
            UiRole::CheckboxBackgroundSelected,
            UiRole::CheckboxBorderSelected,
            UiRole::CheckboxCheck,
        ),
        (false, true) => (
            UiRole::CheckboxBackground,
            UiRole::CheckboxBorder,
            UiRole::CheckboxCheck,
        ),
    };
    let mark = theme.color(mark);
    let classic = theme.controls() == Controls::Classic;
    // The check mark's points and width in the box: the New UI's
    // `M8 12.5L11 15.5L16.5 9` (width 2, round) in its 24 px icon, and the
    // middle line of the Darcula mark in its 19 px icon, each less the
    // box's place in the icon.
    let (size, radius, points, width) = if classic {
        (
            CLASSIC_CHECKBOX,
            2.0,
            [(3.1, 7.0), (5.6, 10.3), (10.4, 3.3)],
            1.75,
        )
    } else {
        (CHECKBOX, 3.0, [(4.0, 8.5), (7.0, 11.5), (12.5, 5.0)], 2.0)
    };
    div()
        .id(id)
        .flex_none()
        .size(px(size))
        .rounded(px(radius))
        .bg(theme.color(background))
        .border_1()
        .border_color(theme.color(border))
        .when(checked, |d| {
            d.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        // The canvas fills the box inside its outline.
                        let o = bounds.origin - point(px(1.0), px(1.0));
                        let at = |(x, y): (f32, f32)| point(o.x + px(x), o.y + px(y));
                        let mut path = PathBuilder::stroke(px(width));
                        path.move_to(at(points[0]));
                        path.line_to(at(points[1]));
                        path.line_to(at(points[2]));
                        if let Ok(path) = path.build() {
                            window.paint_path(path, mark);
                        }
                        // Round caps and join.
                        for p in points {
                            let r = width / 2.0;
                            let dot = gpui::Bounds::new(
                                at((p.0 - r, p.1 - r)),
                                gpui::size(px(width), px(width)),
                            );
                            window.paint_quad(gpui::fill(dot, mark).corner_radii(px(r)));
                        }
                    },
                )
                .size_full(),
            )
        })
}

/// The expand arrow of a tree row or a collapsible group in a 16 px slot:
/// the New UI's chevrons, or filled triangles in the theme's grey icon
/// colour for classic controls (as the reference shows its trees with
/// classic theme plugins).
pub fn tree_arrow(expanded: bool, cx: &App) -> gpui::AnyElement {
    let theme = cx.theme();
    if theme.controls() == Controls::New {
        let icon = if expanded {
            IconName::CHEVRON_DOWN
        } else {
            IconName::CHEVRON_RIGHT
        };
        return Icon::new(icon).into_any_element();
    }
    let color = theme.color(UiRole::IconGrayStroke);
    // A triangle 7 across its base and 4.75 high, centred in the slot.
    let points: [(f32, f32); 3] = if expanded {
        [(4.5, 5.6), (11.5, 5.6), (8.0, 10.35)]
    } else {
        [(5.6, 4.5), (5.6, 11.5), (10.35, 8.0)]
    };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let at = |(x, y): (f32, f32)| point(o.x + px(x), o.y + px(y));
            let mut path = PathBuilder::fill();
            path.move_to(at(points[0]));
            path.line_to(at(points[1]));
            path.line_to(at(points[2]));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .flex_none()
    .size(px(16.0))
    .into_any_element()
}

/// Size of the loading indicator.
pub const SPINNER: f32 = 16.0;

/// The loading indicator: eight dots in a circle, the bright one going
/// round, as the reference IDE's animated progress icon.
pub fn spinner(id: impl Into<ElementId>, cx: &App) -> impl IntoElement {
    let color = cx.theme().color(UiRole::TextMuted);
    div()
        .relative()
        .flex_none()
        .size(px(SPINNER))
        .with_animation(
            id,
            Animation::new(Duration::from_millis(800)).repeat(),
            move |d, delta| {
                let step = (delta * 8.0) as usize % 8;
                d.children((0..8).map(|i| {
                    let angle = std::f32::consts::TAU * i as f32 / 8.0;
                    let (r, dot) = (5.5, 3.0);
                    let x = SPINNER / 2.0 + r * angle.sin() - dot / 2.0;
                    let y = SPINNER / 2.0 - r * angle.cos() - dot / 2.0;
                    let age = (step + 8 - i) % 8;
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .size(px(dot))
                        .rounded_full()
                        .bg(color)
                        .opacity(1.0 - age as f32 * 0.11)
                }))
            },
        )
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
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
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
