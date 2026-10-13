//! The run widget of the main toolbar (New UI `RunToolbarMainActionGroup`):
//! the configuration selector ("Current File"), Run, Debug, Stop and More
//! Actions, with the states of `ExecutorAction`, `StopAction` and
//! `RunWidgetButtonLook`.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, IntoElement, Pixels, SharedString, canvas, div, prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip, spinner};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::commands::*;
use crate::chrome::toolbar_widget;

/// What the run widget shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WidgetState {
    /// The file in the editor (its name with the extension) and whether it
    /// can run; `None` without an editor.
    pub file: Option<(String, bool)>,
    /// The current file's process runs: Run shows Rerun on green.
    pub file_running: bool,
    /// A launch of the current file is being prepared: a spinner.
    pub launching: bool,
    /// Titles of the running processes (the Stop button).
    pub running: Vec<String>,
    /// Shortcuts of Run and Stop where the focus is.
    pub run_keys: Option<SharedString>,
    pub stop_keys: Option<SharedString>,
}

impl WidgetState {
    /// Run's tooltip: what it runs, or why it cannot
    /// (`ExecutorAction.getRunCurrentFileActionStatus`).
    pub fn run_tooltip(&self) -> String {
        match &self.file {
            Some((name, true)) => format!("Run '{name}'"),
            Some((_, false)) => "The file in the editor is not runnable".to_owned(),
            None => "Cannot run the current file: the editor is not in focus".to_owned(),
        }
    }

    pub fn can_run(&self) -> bool {
        matches!(self.file, Some((_, true))) && !self.launching
    }

    /// Stop's tooltip: the one process, or "Stop..." for several.
    pub fn stop_tooltip(&self) -> Option<String> {
        match self.running.as_slice() {
            [] => None,
            [one] => Some(format!("Stop '{one}'")),
            _ => Some("Stop...".to_owned()),
        }
    }

    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "configuration": "Current File",
            "run_enabled": self.can_run(),
            "run_tooltip": self.run_tooltip(),
            "run_icon": if self.file_running { "rerun" } else { "run" },
            "launching": self.launching,
            "debug_enabled": false,
            "stop_visible": !self.running.is_empty(),
            "stop_tooltip": self.stop_tooltip(),
            "stop_count": self.running.len(),
        })
    }
}

/// Where the popups of the widget open: below the selector and below More.
#[derive(Clone, Default)]
pub struct Anchors {
    pub selector: Rc<Cell<Bounds<Pixels>>>,
    pub more: Rc<Cell<Bounds<Pixels>>>,
    pub stop: Rc<Cell<Bounds<Pixels>>>,
}

/// Records an element's bounds when it is laid out (a popup's anchor).
fn anchor(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// A button of the widget: an icon on an optional coloured background,
/// dimmed when disabled.
fn button(
    id: &'static str,
    icon: AnyElement,
    background: Option<gpui::Hsla>,
    enabled: bool,
    tooltip: Option<(String, Option<SharedString>)>,
    action: Box<dyn gpui::Action>,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    let size = theme.metric(Metric::ToolbarButtonSize);
    let hover = theme.color(UiRole::Hover);
    let pressed = theme.color(UiRole::Pressed);
    div()
        .id(id)
        .debug_selector(move || id.into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(theme.metric(Metric::ButtonRadius))
        .map(|d| match background {
            Some(bg) => d.bg(bg),
            None if enabled => d.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed)),
            None => d,
        })
        .child(
            div()
                .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                .child(icon),
        )
        .when_some(tooltip, |d, (title, keys)| {
            d.tooltip(Tooltip::text(title, keys))
        })
        .when(enabled, |d| {
            d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        })
}

/// The widget for `state`.
pub fn render(state: &WidgetState, anchors: &Anchors, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let icon = |name: IconName| Icon::new(name).size(px(16.0)).into_any_element();
    let selector = toolbar_widget("run-configuration", cx)
        .debug_selector(|| "run-configuration".into())
        .relative()
        .gap(px(4.0))
        .child(div().whitespace_nowrap().child("Current File"))
        .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0)))
        .child(anchor(anchors.selector.clone()))
        .tooltip(Tooltip::text("Run / Debug Configurations", None))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ChooseConfiguration), cx));
    let run_icon = if state.launching {
        spinner("run-launching", cx).into_any_element()
    } else if state.file_running {
        icon(IconName::RERUN_STROKE)
    } else {
        icon(IconName::RUN)
    };
    let run = button(
        "run",
        run_icon,
        state
            .file_running
            .then(|| theme.color(UiRole::RunWidgetRunning)),
        state.can_run(),
        Some((state.run_tooltip(), state.run_keys.clone())),
        Box::new(Run),
        cx,
    );
    // Debugging arrives with the debugger.
    let debug = button(
        "debug",
        icon(IconName::DEBUG),
        None,
        false,
        Some(("Debug".to_owned(), None)),
        Box::new(Run),
        cx,
    );
    // Hidden while nothing runs; an empty slot keeps its place
    // (`InactiveStopActionPlaceholder`).
    let size = theme.metric(Metric::ToolbarButtonSize);
    let stop = if state.running.is_empty() {
        div().flex_none().size(size).into_any_element()
    } else {
        let count = state.running.len();
        let stop_icon = div()
            .relative()
            .child(icon(IconName::STOP_STROKE))
            .when(count > 1, |d| {
                let text = if count > 9 {
                    "9+".to_owned()
                } else {
                    count.to_string()
                };
                d.child(
                    div()
                        .absolute()
                        .top(px(-5.0))
                        .right(px(-7.0))
                        .px(px(3.0))
                        .rounded_full()
                        .bg(theme.color(UiRole::RunWidgetStop))
                        .text_color(theme.color(UiRole::OnAccent))
                        .text_size(px(9.0))
                        .child(text),
                )
            })
            .into_any_element();
        button(
            "run-stop",
            stop_icon,
            Some(theme.color(UiRole::RunWidgetStop)),
            true,
            state.stop_tooltip().map(|t| (t, state.stop_keys.clone())),
            Box::new(Stop { tab: None }),
            cx,
        )
        .child(anchor(anchors.stop.clone()))
        .into_any_element()
    };
    let more = button(
        "run-more",
        icon(IconName::MORE_VERTICAL),
        None,
        true,
        Some(("More Actions".to_owned(), None)),
        Box::new(MoreActions),
        cx,
    )
    .child(anchor(anchors.more.clone()));
    div()
        .id("run-widget")
        .debug_selector(|| "run-widget".into())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .child(selector)
        .child(run)
        .child(debug)
        .child(stop)
        .child(more)
}
