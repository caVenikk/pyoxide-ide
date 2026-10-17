//! The run widget of the main toolbar (New UI `RunToolbarMainActionGroup`):
//! the configuration selector ("Current File"), Run, Debug, Stop and More
//! Actions, with the states of `ExecutorAction`, `StopAction` and
//! `RunWidgetButtonLook`. While the selected configuration is debugged,
//! Resume or Pause takes Run's place (`ConfigurationXDebuggerResumeAction`,
//! `filterOutRunIfDebugResumeIsPresent`).

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, FontWeight, IntoElement, Pixels, SharedString, canvas, div,
    prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, spinner};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::commands::*;
use crate::chrome::{toolbar_widget, toolbar_widget_chrome, widget_text};
use crate::main_toolbar::{Cut, Fitted, Measure, Measured};
use debugger_ui::session::commands::{Debug, Pause, Resume};

/// Opacity of a temporary configuration's icon
/// (`ProgramRunnerUtil.getTemporaryIcon`).
pub const TEMPORARY_ICON_ALPHA: f32 = 0.45;

/// The selected run configuration as the widget shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedConfig {
    pub name: String,
    /// The type's icon; `None` for a type Pyoxide does not know.
    pub icon: Option<IconName>,
    /// Drawn at [`TEMPORARY_ICON_ALPHA`].
    pub temporary: bool,
    /// Its type is supported: it can run.
    pub supported: bool,
}

/// What the run widget shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WidgetState {
    /// The selected run configuration; `None` is Current File.
    pub configuration: Option<SelectedConfig>,
    /// The file in the editor (its name with the extension) and whether it
    /// can run; `None` without an editor.
    pub file: Option<(String, bool)>,
    /// What Run runs (the selected configuration, or the current file) is
    /// running: Run shows Rerun on green.
    pub file_running: bool,
    /// A launch of what Run runs is being prepared: a spinner.
    pub launching: bool,
    /// Titles of the running processes and debug sessions (the Stop
    /// button).
    pub running: Vec<String>,
    /// The product can debug what Debug debugs.
    pub debuggable: bool,
    /// What Debug debugs is being debugged: Debug shows Rerun on green.
    pub debug_running: bool,
    /// A debug run of it is being prepared: a spinner.
    pub debug_launching: bool,
    /// The selected configuration's debug session: its tab in the Debug
    /// tool window and whether it is suspended (Resume, else Pause, in
    /// Run's place).
    pub debug_session: Option<(usize, bool)>,
    /// Shortcuts of Run, Debug and Stop where the focus is.
    pub run_keys: Option<SharedString>,
    pub debug_keys: Option<SharedString>,
    pub stop_keys: Option<SharedString>,
}

impl WidgetState {
    /// Run's tooltip: what it runs, or why it cannot
    /// (`ExecutorAction.getRunCurrentFileActionStatus`).
    pub fn run_tooltip(&self) -> String {
        if let Some(config) = &self.configuration {
            return if !config.supported {
                format!(
                    "'{}' cannot run: its type is not supported yet",
                    config.name
                )
            } else if self.file_running {
                format!("Rerun '{}'", config.name)
            } else {
                format!("Run '{}'", config.name)
            };
        }
        match &self.file {
            Some((name, true)) => format!("Run '{name}'"),
            Some((_, false)) => "The file in the editor is not runnable".to_owned(),
            None => "Cannot run the current file: the editor is not in focus".to_owned(),
        }
    }

    pub fn can_run(&self) -> bool {
        let runnable = match &self.configuration {
            Some(config) => config.supported,
            None => matches!(self.file, Some((_, true))),
        };
        runnable && !self.launching
    }

    /// Debug is enabled.
    pub fn can_debug(&self) -> bool {
        let runnable = match &self.configuration {
            Some(config) => config.supported,
            None => matches!(self.file, Some((_, true))),
        };
        self.debuggable && runnable && !self.debug_launching
    }

    /// Debug's tooltip (`ExecutorAction` with the Debug executor): "Restart
    /// Debug" while the configuration is debugged; the current file keeps
    /// its name.
    pub fn debug_tooltip(&self) -> String {
        if let Some(config) = &self.configuration {
            return if !config.supported || !self.debuggable {
                format!(
                    "'{}' cannot be debugged: its type is not supported yet",
                    config.name
                )
            } else if self.debug_running {
                format!("Restart Debug '{}'", config.name)
            } else {
                format!("Debug '{}'", config.name)
            };
        }
        match &self.file {
            Some((name, true)) => format!("Debug '{name}'"),
            Some((_, false)) => "The file in the editor is not runnable".to_owned(),
            None => "Cannot run the current file: the editor is not in focus".to_owned(),
        }
    }

    /// The Resume or Pause button in Run's place: its tooltip
    /// (`description.text.resume`, `description.text.pause`).
    pub fn resume_tooltip(&self) -> Option<String> {
        let (_, suspended) = self.debug_session?;
        let name = &self.configuration.as_ref()?.name;
        Some(if suspended {
            format!("Resume '{name}'")
        } else {
            format!("Pause '{name}'")
        })
    }

    /// The selector's text: the configuration's name cut in the middle,
    /// or "Current File".
    pub fn selector_text(&self) -> String {
        match &self.configuration {
            Some(config) => super::dropdown::trim_name(&config.name),
            None => "Current File".to_owned(),
        }
    }

    /// The selector's tooltip ("Run / Debug Configuration: {name}" when the
    /// name is cut).
    pub fn selector_tooltip(&self) -> String {
        match &self.configuration {
            Some(config) if super::dropdown::trim_name(&config.name) != config.name => {
                self.cut_selector_tooltip()
            }
            _ => "Run / Debug Configurations".to_owned(),
        }
    }

    /// The selector's tooltip when its text is cut, by length or because
    /// the toolbar is narrow.
    fn cut_selector_tooltip(&self) -> String {
        format!("Run / Debug Configuration: {}", self.selector_full_text())
    }

    /// The selector's text before any cut: the configuration's name, or
    /// "Current File".
    pub fn selector_full_text(&self) -> &str {
        match &self.configuration {
            Some(config) => &config.name,
            None => "Current File",
        }
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
            "configuration": self.selector_text(),
            "configuration_icon": self.configuration.as_ref().and_then(|c| c.icon).map(|i| i.0),
            "configuration_temporary": self.configuration.as_ref().is_some_and(|c| c.temporary),
            "selector_tooltip": self.selector_tooltip(),
            "run_enabled": self.can_run(),
            "run_tooltip": self.run_tooltip(),
            "run_icon": if self.file_running { "rerun" } else { "run" },
            "launching": self.launching,
            "run_visible": self.debug_session.is_none(),
            "debug_enabled": self.can_debug(),
            "debug_tooltip": self.debug_tooltip(),
            "debug_icon": if self.debug_running { "rerun" } else { "debug" },
            "debug_launching": self.debug_launching,
            "resume": self.debug_session.map(|(tab, suspended)| serde_json::json!({
                "action": if suspended { "resume" } else { "pause" },
                "tooltip": self.resume_tooltip(),
                "tab": tab,
            })),
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

/// A button of the widget: an icon on an optional coloured background;
/// a disabled one shows its inactive icon (see [`inactive`]).
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
        .child(icon)
        .when_some(tooltip, |d, (title, keys)| {
            d.tooltip(Tooltip::text(title, keys))
        })
        .when(enabled, |d| {
            d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        })
}

/// The outline of a disabled button in the disabled colour (the
/// reference's run widget paints stroke icons and greys them out when the
/// action is disabled), not the coloured icon dimmed.
fn inactive(name: IconName, cx: &App) -> AnyElement {
    Icon::new(name)
        .size(px(16.0))
        .tint(cx.theme().color(UiRole::TextDisabled))
        .into_any_element()
}

/// Between the parts of the selector and between the widget's parts.
const GAP: f32 = 4.0;
/// Size of the icons of the selector.
const SELECTOR_ICON: f32 = 16.0;
/// Button slots after the selector: Run, Debug, Stop or its empty slot,
/// More.
const BUTTONS: f32 = 4.0;

/// The selector's text as measured for the main toolbar: the name cut in
/// the middle when the toolbar is narrow, never longer than the name the
/// widget shows ([`super::dropdown::trim_name`]).
pub fn selector_text(state: &WidgetState, measure: &Measure) -> Measured {
    measure
        .text(state.selector_full_text(), FontWeight::NORMAL, Cut::Middle)
        .max_chars(super::dropdown::NAME_MAX_CHARS)
}

/// Width of the selector without its text: padding, icon and chevron.
pub fn selector_chrome(state: &WidgetState) -> f32 {
    let icon = if state.configuration.is_some() {
        SELECTOR_ICON + GAP
    } else {
        0.0
    };
    toolbar_widget_chrome(GAP) + icon
}

/// Width of the buttons after the selector, with the gap before them.
pub fn buttons_width(cx: &App) -> f32 {
    let button = f32::from(cx.theme().metric(Metric::ToolbarButtonSize));
    BUTTONS * (button + GAP)
}

/// The configuration selector for `state` with its text as the toolbar
/// fitted it (see [`selector_text`]). Of the widget only the selector gives
/// way when the toolbar is narrow; the buttons ([`render_buttons`]) keep
/// their width (`CompressingLayoutStrategy`).
pub fn render_selector(
    state: &WidgetState,
    anchors: &Anchors,
    text: &Fitted,
    cx: &App,
) -> impl IntoElement {
    // A configuration shows its type's icon (temporary ones at 45 %);
    // Current File shows none.
    let config_icon = state.configuration.as_ref().map(|c| {
        div()
            .flex_none()
            .when(c.temporary, |d| d.opacity(TEMPORARY_ICON_ALPHA))
            .child(Icon::new(c.icon.unwrap_or(IconName::FILE_UNKNOWN)).size(px(SELECTOR_ICON)))
    });
    // A name cut short shows in full in the tooltip (`getToolTipText`).
    let tooltip = if text.cut {
        state.cut_selector_tooltip()
    } else {
        state.selector_tooltip()
    };
    toolbar_widget("run-configuration", cx)
        .debug_selector(|| "run-configuration".into())
        .relative()
        .gap(px(GAP))
        .children(config_icon)
        // Left-aligned (`setHorizontalTextAlignment(LEFT)`).
        .child(
            widget_text(text)
                .debug_selector(|| "run-configuration-text".into())
                .text_left(),
        )
        .child(
            div()
                .flex_none()
                .child(Icon::new(IconName::CHEVRON_DOWN).size(px(SELECTOR_ICON))),
        )
        .child(anchor(anchors.selector.clone()))
        .tooltip(Tooltip::text(tooltip, None))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ChooseConfiguration), cx))
}

/// Run, Debug, Stop and More Actions of the widget for `state`, after the
/// selector.
pub fn render_buttons(state: &WidgetState, anchors: &Anchors, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let icon = |name: IconName| Icon::new(name).size(px(16.0)).into_any_element();
    let run_icon = if state.launching {
        spinner("run-launching", cx).into_any_element()
    } else if state.file_running {
        icon(IconName::RERUN_STROKE)
    } else if state.can_run() {
        icon(IconName::RUN)
    } else {
        inactive(IconName::RUN_STROKE, cx)
    };
    let run = match state.debug_session {
        // Resume or Pause of the configuration's debug session.
        Some((tab, suspended)) => {
            let (name, action): (IconName, Box<dyn gpui::Action>) = if suspended {
                (
                    debugger_ui::icons::RESUME_STROKE,
                    Box::new(Resume { index: Some(tab) }),
                )
            } else {
                (
                    debugger_ui::icons::PAUSE,
                    Box::new(Pause { index: Some(tab) }),
                )
            };
            let icon = Icon::new(name)
                .size(px(16.0))
                .tint(theme.color(UiRole::Text))
                .into_any_element();
            button(
                "run-resume",
                icon,
                None,
                true,
                state.resume_tooltip().map(|t| (t, None)),
                action,
                cx,
            )
        }
        None => button(
            "run",
            run_icon,
            state
                .file_running
                .then(|| theme.color(UiRole::RunWidgetRunning)),
            state.can_run(),
            Some((state.run_tooltip(), state.run_keys.clone())),
            Box::new(Run),
            cx,
        ),
    };
    let debug_icon = if state.debug_launching {
        spinner("debug-launching", cx).into_any_element()
    } else if state.debug_running {
        icon(debugger_ui::icons::RERUN_DEBUG_STROKE)
    } else if state.can_debug() {
        icon(IconName::DEBUG)
    } else {
        inactive(IconName::DEBUG_STROKE, cx)
    };
    let debug = button(
        "debug",
        debug_icon,
        state
            .debug_running
            .then(|| theme.color(UiRole::RunWidgetRunning)),
        state.can_debug(),
        Some((state.debug_tooltip(), state.debug_keys.clone())),
        Box::new(Debug),
        cx,
    );
    // Hidden while nothing runs; an empty slot after More keeps the
    // widget's width (`InactiveStopActionPlaceholder`, the last of
    // `RunToolbarMainActionGroup`).
    let size = theme.metric(Metric::ToolbarButtonSize);
    let placeholder = state
        .running
        .is_empty()
        .then(|| div().flex_none().size(size).into_any_element());
    let stop = if state.running.is_empty() {
        None
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
        let stop = button(
            "run-stop",
            stop_icon,
            Some(theme.color(UiRole::RunWidgetStop)),
            true,
            state.stop_tooltip().map(|t| (t, state.stop_keys.clone())),
            Box::new(Stop { tab: None }),
            cx,
        )
        .child(anchor(anchors.stop.clone()))
        .into_any_element();
        Some(stop)
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
        .id("run-buttons")
        .debug_selector(|| "run-buttons".into())
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        // Buttons of 30 with 2 on both sides (`RunWidget.actionButtonWidth`,
        // `toolbarBorderDirectionalGap`).
        .pl(px(GAP))
        .gap(px(GAP))
        .child(run)
        .child(debug)
        .children(stop)
        .child(more)
        .children(placeholder)
}
