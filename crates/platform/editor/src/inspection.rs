//! The inspection widget in the top-right corner of an editor and the
//! tooltip of the error stripe, as the reference draws them
//! (`EditorMarkupModelImpl`, `TrafficLightRenderer`): an icon and a count
//! per problem level present, then previous / next problem arrows; a check
//! mark when the text is clean; translucent while the analysis runs.

use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, popup_shadow, small_icon_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::diagnostics::{Analysis, MarkSeverity};
use crate::editor::Editor;

/// Border of the widget's toolbar.
const TOOLBAR_BORDER: f32 = 2.0;
/// Smallest height and width of the traffic light button's content.
const STATUS_ICON: f32 = 18.0;
/// Insets of the traffic light button.
const BUTTON_INSET: f32 = 2.0;
/// Space before the first and after the last count.
const LEFT_RIGHT_INDENT: f32 = 5.0;
/// Space between two counts.
const INTER_GROUP_OFFSET: f32 = 6.0;
/// Opacity of the widget while the analysis runs.
const TRANSLUCENT: f32 = 0.5;

/// The icon of a level and its colour: the white severity shapes in the
/// level's error stripe colour.
fn level_icon(severity: MarkSeverity, cx: &gpui::App) -> (IconName, Hsla) {
    let theme = cx.theme();
    match severity {
        MarkSeverity::Error => (
            IconName::INSPECTIONS_ERROR,
            theme.color(UiRole::DiagnosticErrorStripe),
        ),
        MarkSeverity::Warning => (
            IconName::INSPECTIONS_WARNING,
            theme.color(UiRole::DiagnosticWarningStripe),
        ),
        _ => (
            IconName::INSPECTIONS_WARNING,
            theme.color(UiRole::DiagnosticWeakWarningStripe),
        ),
    }
}

/// The inspection widget of an editor whose text is analysed.
pub(crate) fn widget(editor: &Editor, cx: &mut Context<Editor>) -> Option<AnyElement> {
    if editor.analysis == Analysis::Off || editor.single_line || editor.is_console() {
        return None;
    }
    let theme = cx.theme();
    let counts = editor.problem_counts();
    let running = editor.analysis == Analysis::Running;
    let items: Vec<(IconName, Hsla, usize)> = MarkSeverity::COUNTED
        .iter()
        .filter(|&&s| counts.get(s) > 0)
        .map(|&s| {
            let (icon, color) = level_icon(s, cx);
            (icon, color, counts.get(s))
        })
        .collect();
    let navigation = !items.is_empty();
    let hover = theme.color(UiRole::Hover);
    let pressed = theme.color(UiRole::Pressed);
    let text_color = theme.color(UiRole::TextMuted);
    let text_size = theme.ui_size() - px(2.0);
    let radius = theme.metric(Metric::CornerRadius);
    let stripe = theme.metric(Metric::ScrollbarWidth);

    let content: Vec<AnyElement> = if items.is_empty() {
        // Clean (or nothing found yet): the check mark alone.
        vec![Icon::new(IconName::INSPECTIONS_OK).into_any_element()]
    } else {
        items
            .into_iter()
            .enumerate()
            .map(|(i, (icon, color, count))| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(1.0))
                    .when(i > 0, |d| d.ml(px(INTER_GROUP_OFFSET)))
                    .child(Icon::new(icon).tint(color))
                    .child(SharedString::from(count.to_string()))
                    .into_any_element()
            })
            .collect()
    };
    let light = div()
        .id("inspection-widget")
        .debug_selector(|| "inspection-widget".into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(px(STATUS_ICON + 2.0 * BUTTON_INSET))
        .h(px(STATUS_ICON + 2.0 * BUTTON_INSET))
        .when(navigation, |d| {
            d.pl(px(BUTTON_INSET + LEFT_RIGHT_INDENT))
                .pr(px(LEFT_RIGHT_INDENT))
        })
        .rounded(radius)
        .text_size(text_size)
        .text_color(text_color)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .child(
            div()
                .flex()
                .items_center()
                .when(running, |d| d.opacity(TRANSLUCENT))
                .children(content),
        )
        .on_click(cx.listener(|this, _, window, cx| {
            window.focus(&this.focus_handle_ref().clone());
            this.go_to_first_problem(cx);
        }));
    let arrows = navigation.then(|| {
        [
            small_icon_button("inspection-previous", IconName::CHEVRON_UP, true, cx)
                .debug_selector(|| "inspection-previous".into())
                .tooltip(Tooltip::text("Previous Highlighted Error", None))
                .on_click(cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus_handle_ref().clone());
                    this.go_to_problem(false, cx);
                })),
            small_icon_button("inspection-next", IconName::CHEVRON_DOWN, true, cx)
                .debug_selector(|| "inspection-next".into())
                .tooltip(Tooltip::text("Next Highlighted Error", None))
                .on_click(cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus_handle_ref().clone());
                    this.go_to_problem(true, cx);
                })),
        ]
    });
    Some(
        div()
            .id("inspection-toolbar")
            .absolute()
            .top(px(TOOLBAR_BORDER))
            .right(stripe + px(TOOLBAR_BORDER))
            .flex()
            .items_center()
            .occlude()
            .child(light)
            .children(arrows.into_iter().flatten())
            .into_any_element(),
    )
}

/// The messages of the problems under the mouse on the error stripe, left
/// of the stripe at the nearest mark: each message once, its source and
/// rule in grey after it.
pub(crate) fn stripe_tooltip(editor: &Editor, cx: &mut Context<Editor>) -> Option<AnyElement> {
    let hover = editor.stripe_hover.as_ref()?;
    let layout = editor.layout.as_ref()?;
    let stripe = layout.stripe?;
    let theme = cx.theme();
    let mut rows: Vec<(SharedString, Option<SharedString>)> = Vec::new();
    for &i in &hover.marks {
        let Some(mark) = editor.diagnostics().get(i) else {
            continue;
        };
        for msg in &mark.messages {
            let suffix = match (&msg.source, &msg.code) {
                (Some(s), Some(c)) => Some(format!("{s}({c})").into()),
                (Some(s), None) => Some(s.clone()),
                (None, Some(c)) => Some(c.clone()),
                (None, None) => None,
            };
            let row = (msg.text.clone(), suffix);
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    if rows.is_empty() {
        return None;
    }
    let muted = theme.color(UiRole::TextMuted);
    let line = layout.line_height;
    Some(
        div()
            .debug_selector(|| "stripe-tooltip".into())
            .absolute()
            .right(layout.bounds.right() - stripe.left() + px(4.0))
            .top((hover.y - layout.bounds.top() - line / 2.0).max(px(0.0)))
            .max_w(px(600.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
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
            .children(rows.into_iter().map(|(text, suffix)| {
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(text)
                    .children(suffix.map(|s| div().text_color(muted).child(s)))
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests;
