//! The Run tool window: a tab per run with its console and toolbar.

use gpui::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, Window, div, prelude::*, px,
};
use themes::Metric;
use ui::components::{Tooltip, icon_button};
use ui::icons::IconName;
use ui::theme::ThemeExt;

/// What the Run tool window asks of the workbench.
#[derive(Debug, Clone, PartialEq)]
pub enum RunToolWindowEvent {
    /// The Hide button was pressed.
    Hide,
}

/// The Run tool window's content.
pub struct RunToolWindow {
    focus: FocusHandle,
}

impl EventEmitter<RunToolWindowEvent> for RunToolWindow {}

impl RunToolWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
        }
    }

    /// The state reported by `run.state`.
    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({ "tabs": [] })
    }
}

impl Focusable for RunToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RunToolWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header_h = theme.metric(Metric::ToolWindowHeaderHeight);
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(header_h)
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child("Run"),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "run-options",
                    IconName::MORE_VERTICAL,
                    px(24.0),
                    px(16.0),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Options", None)),
            )
            .child(
                icon_button("run-hide", IconName::HIDE, px(24.0), px(16.0), false, cx)
                    .tooltip(Tooltip::text("Hide", None))
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(RunToolWindowEvent::Hide)),
                    ),
            );
        div()
            .id("run-tool-window-content")
            .key_context("RunToolWindow")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(div().flex_1().min_h(px(0.0)))
    }
}
