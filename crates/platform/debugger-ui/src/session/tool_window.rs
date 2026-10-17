//! The Debug tool window (`XDebugSessionTabNewUI`): a tab per session in
//! the header ("Debug", then `main ×`), and for the selected session the
//! toolbar before its tabs (Rerun, Stop | Resume, Pause, Step Over, Step
//! Into, Step Into My Code, Step Out | View Breakpoints, Mute | More), the
//! "Threads & Variables" and "Console" tabs and the layout button at the
//! right, then the selected tab's content.

use std::cell::Cell;
use std::rc::Rc;

use console::LinkTarget;
use console_ui::{ConsoleEvent, ConsoleView};
use debugger::{DebugRequest, Phase, Session, SessionId, SessionSnapshot};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, IntoElement, MouseButton, MouseDownEvent, Pixels, Point,
    Render, SharedString, Subscription, Task, Window, canvas, div, prelude::*, px,
};
use process::ExitStatus;
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::{DISABLED_ALPHA, Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::commands::*;
use crate::breakpoints::commands::{MuteBreakpoints, ViewBreakpoints};
use crate::icons;

/// Size of the toolbar's buttons and of their icons.
const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;

/// A stepping button of the toolbar: its id, icon, tooltip, action and the
/// key of the state that enables it.
type ToolbarStep = (
    &'static str,
    IconName,
    &'static str,
    Box<dyn gpui::Action>,
    &'static str,
);

/// A session's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    ThreadsVariables,
    Console,
}

impl Content {
    pub fn id(self) -> &'static str {
        match self {
            Content::ThreadsVariables => "threads-variables",
            Content::Console => "console",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "threads-variables" => Some(Content::ThreadsVariables),
            "console" => Some(Content::Console),
            _ => None,
        }
    }

    /// The tab's title (`XDebuggerBundle` `xdebugger.threads.vars.tab.title`,
    /// `debugger.session.tab.console.content.name`).
    pub fn title(self) -> &'static str {
        match self {
            Content::ThreadsVariables => "Threads & Variables",
            Content::Console => "Console",
        }
    }
}

/// One debug session's tab.
pub(crate) struct SessionTab {
    pub id: u64,
    pub session: Session,
    /// What was debugged: Rerun debugs it again.
    pub request: DebugRequest,
    pub title: String,
    pub icon: Option<IconName>,
    /// What the window started: a run configuration or a file, in the
    /// window's own terms (the run widget and the "Is Running" check
    /// compare it).
    pub key: Option<String>,
    pub console: Entity<ConsoleView>,
    pub content: Content,
    /// The session as it was last shown.
    pub snapshot: SessionSnapshot,
    /// The program's exit, as its console printed it.
    pub exit: Option<ExitStatus>,
    /// The console shows the program's output.
    pub attached: bool,
    /// Debug again in this tab once the session ended (Rerun).
    pub restart: bool,
    pub _watch: Option<Task<()>>,
    _console_events: Subscription,
}

impl SessionTab {
    pub fn is_alive(&self) -> bool {
        self.snapshot.phase != Phase::Ended
    }
}

/// What the Debug tool window asks of the window.
#[derive(Debug, Clone, PartialEq)]
pub enum DebugToolWindowEvent {
    Hide,
    OpenLink(LinkTarget),
    MoreMenu(Point<Pixels>),
}

/// The Debug tool window's content.
pub struct DebugToolWindow {
    pub(crate) tabs: Vec<SessionTab>,
    pub(crate) selected: Option<usize>,
    /// Breakpoints are muted (the Mute button is pressed).
    pub(crate) muted: bool,
    /// The toolbar's More button where it was last laid out: its menu
    /// opens below it.
    more_anchor: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    next_id: u64,
}

impl EventEmitter<DebugToolWindowEvent> for DebugToolWindow {}

impl DebugToolWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tabs: Vec::new(),
            selected: None,
            muted: false,
            more_anchor: Rc::default(),
            focus: cx.focus_handle(),
            next_id: 0,
        }
    }

    /// Where the More menu opens: below the toolbar's More button, `None`
    /// before the toolbar was laid out.
    pub fn more_menu_position(&self) -> Option<Point<Pixels>> {
        let bounds = self.more_anchor.get();
        (bounds.size.width > px(0.0)).then(|| bounds.bottom_left())
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected.filter(|i| *i < self.tabs.len())
    }

    pub(crate) fn selected_tab(&self) -> Option<&SessionTab> {
        self.selected().map(|i| &self.tabs[i])
    }

    /// The session of the selected tab.
    pub fn selected_session(&self) -> Option<Session> {
        self.selected_tab().map(|t| t.session.clone())
    }

    pub(crate) fn index_of_session(&self, id: SessionId) -> Option<usize> {
        self.tabs.iter().position(|t| t.session.id() == id)
    }

    pub(crate) fn index_of(&self, tab: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == tab)
    }

    /// A session has not ended: the stripe button shows the live badge.
    pub fn any_alive(&self) -> bool {
        self.tabs.iter().any(SessionTab::is_alive)
    }

    /// The titles of the sessions that have not ended (the run widget's
    /// Stop).
    pub fn alive_titles(&self) -> Vec<String> {
        self.tabs
            .iter()
            .filter(|t| t.is_alive())
            .map(|t| t.title.clone())
            .collect()
    }

    /// The tabs' sessions and titles.
    pub fn sessions(&self) -> Vec<(Session, String)> {
        self.tabs
            .iter()
            .map(|t| (t.session.clone(), t.title.clone()))
            .collect()
    }

    /// The tabs whose sessions have not ended.
    pub fn alive_tabs(&self) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|i| self.tabs[*i].is_alive())
            .collect()
    }

    /// A tab's title.
    pub fn title(&self, index: usize) -> Option<&str> {
        self.tabs.get(index).map(|t| t.title.as_str())
    }

    /// A tab's session's phase, as the tab shows it.
    pub fn phase(&self, index: usize) -> Option<Phase> {
        self.tabs.get(index).map(|t| t.snapshot.phase)
    }

    /// The tab of a session started as `key` that has not ended.
    pub fn alive_with_key(&self, key: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.is_alive() && t.key.as_deref() == Some(key))
    }

    /// Where activating the tool window moves the focus: the console when
    /// it shows, else the tool window.
    pub fn content_focus(&self, cx: &App) -> FocusHandle {
        match self.selected_tab() {
            Some(tab) if tab.content == Content::Console => tab.console.focus_handle(cx),
            _ => self.focus.clone(),
        }
    }

    /// A new tab for a session, selected; its console shows the program
    /// once it starts.
    pub(crate) fn add(
        &mut self,
        session: Session,
        request: DebugRequest,
        icon: Option<IconName>,
        key: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let console = cx.new(|cx| {
            let mut console = ConsoleView::new(window, cx);
            console.set_key_context("DebugConsole");
            console
        });
        let events = cx.subscribe(&console, |_, _, event: &ConsoleEvent, cx| match event {
            ConsoleEvent::OpenLink(target) => {
                cx.emit(DebugToolWindowEvent::OpenLink(target.clone()))
            }
        });
        self.next_id += 1;
        self.tabs.push(SessionTab {
            id: self.next_id,
            title: session.name().to_owned(),
            snapshot: session.snapshot(),
            session,
            request,
            icon,
            key,
            console,
            content: Content::Console,
            exit: None,
            attached: false,
            restart: false,
            _watch: None,
            _console_events: events,
        });
        let index = self.tabs.len() - 1;
        self.selected = Some(index);
        cx.notify();
        index
    }

    /// Rerun: a new session in an existing tab, its console cleared.
    pub(crate) fn replace(
        &mut self,
        index: usize,
        session: Session,
        request: DebugRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.title = session.name().to_owned();
        tab.snapshot = session.snapshot();
        tab.session = session;
        tab.request = request;
        tab.content = Content::Console;
        tab.exit = None;
        tab.attached = false;
        tab.restart = false;
        tab._watch = None;
        tab.console.update(cx, |c, cx| {
            c.set_input(None);
            c.reset(Vec::new(), cx);
        });
        self.selected = Some(index);
        cx.notify();
    }

    pub(crate) fn remove(&mut self, index: usize, cx: &mut Context<Self>) -> Option<SessionTab> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        self.selected = match self.selected {
            _ if self.tabs.is_empty() => None,
            Some(s) if s > index => Some(s - 1),
            Some(s) if s == index => Some(index.saturating_sub(1).min(self.tabs.len() - 1)),
            other => other,
        };
        cx.notify();
        Some(tab)
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.selected = Some(index);
            cx.notify();
        }
    }

    /// Shows a tab of the selected session.
    pub fn select_content(&mut self, content: Content, cx: &mut Context<Self>) {
        if let Some(i) = self.selected() {
            self.tabs[i].content = content;
            cx.notify();
        }
    }

    pub(crate) fn set_content(&mut self, index: usize, content: Content, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.content = content;
            cx.notify();
        }
    }

    /// The tool window as `debug.state` reports it.
    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "selected": self.selected(),
            "muted": self.muted,
            "tabs": self.tabs.iter().enumerate().map(|(i, t)| {
                let s = &t.snapshot;
                json!({
                    "title": t.title,
                    "icon": t.icon.map(|i| i.0),
                    "key": t.key,
                    "session": s.id.0,
                    "state": s.phase.as_str(),
                    "content": t.content.id(),
                    "alive": t.is_alive(),
                    "exit_code": t.exit.map(|e| e.code),
                    "toolbar": toolbar_state(s.phase),
                    "frames": s.frames.iter().map(frame_text).collect::<Vec<_>>(),
                    "selected": self.selected() == Some(i),
                    "console": t.console.read(cx).state_json(cx),
                })
            }).collect::<Vec<_>>(),
        })
    }

    // ---- drawing ---------------------------------------------------------------

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let theme = cx.theme();
        let selected = self.selected() == Some(index);
        let hover = theme.color(UiRole::TabHover);
        let text = if selected {
            theme.color(UiRole::TabActiveText)
        } else {
            theme.color(UiRole::TabInactiveText)
        };
        let group: SharedString = format!("debug-tab-{index}").into();
        let icon = div()
            .flex_none()
            .when(!tab.is_alive(), |d| d.opacity(0.5))
            .child(Icon::new(tab.icon.unwrap_or(IconName::DEBUG)).size(px(ICON)));
        let close = div()
            .id(("debug-tab-close", index))
            .debug_selector(move || format!("debug-tab-close-{index}"))
            .flex_none()
            .size(px(16.0))
            .rounded(px(3.0))
            .hover(move |s| s.bg(hover))
            .child(Icon::new(IconName::CLOSE_SMALL))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(CloseSession { index: Some(index) }), cx);
            }));
        div()
            .id(("debug-tab", index))
            .debug_selector(move || format!("debug-tab-{index}"))
            .group(group)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(26.0))
            .pl(px(6.0))
            .pr(px(4.0))
            .gap(px(4.0))
            .rounded(theme.metric(Metric::ButtonRadius))
            .border_1()
            .when(selected, |d| {
                d.bg(theme.color(UiRole::TabActive))
                    .border_color(theme.color(UiRole::Border))
            })
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .hover(move |s| s.bg(hover))
            })
            .child(icon)
            .child(
                div()
                    .whitespace_nowrap()
                    .text_color(text)
                    .child(tab.title.clone()),
            )
            .child(close)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select(index, cx);
                window.focus(&this.content_focus(cx));
            }))
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<AnyElement> = (0..self.tabs.len())
            .map(|i| self.render_tab(i, cx))
            .collect();
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(theme.metric(Metric::ToolWindowHeaderHeight))
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .mr(px(8.0))
                    .child("Debug"),
            )
            .child(
                div()
                    .id("debug-tabs")
                    .debug_selector(|| "debug-tabs".into())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .min_w(px(0.0))
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "debug-options",
                    IconName::MORE_VERTICAL,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Options", None)),
            )
            .child(
                icon_button(
                    "debug-hide",
                    IconName::HIDE,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Hide", None))
                .on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(DebugToolWindowEvent::Hide)),
                ),
            )
    }

    /// A toolbar button that dispatches a command; dimmed when disabled,
    /// pressed when `toggled`.
    #[allow(clippy::too_many_arguments)]
    fn tool_button(
        &self,
        id: &'static str,
        icon: IconName,
        title: SharedString,
        action: Box<dyn gpui::Action>,
        enabled: bool,
        toggled: bool,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let keys = ui::keys::shortcut_in(action.as_ref(), &self.focus, window).map(Into::into);
        icon_button(id, icon, px(BUTTON), px(ICON), toggled, cx)
            .debug_selector(move || id.into())
            .tooltip(Tooltip::text(title, keys))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .when(enabled, |d| {
                d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
            .into_any_element()
    }

    fn separator(cx: &App) -> AnyElement {
        div()
            .flex_none()
            .w(px(1.0))
            .h(px(16.0))
            .mx(px(3.0))
            .bg(cx.theme().color(UiRole::Border))
            .into_any_element()
    }

    fn render_toolbar(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let state = toolbar_state(tab.snapshot.phase);
        let on = |key: &str| state[key].as_bool().unwrap_or(false);
        let title = tab.title.clone();
        let mut buttons = vec![
            self.tool_button(
                "debug-rerun",
                icons::RERUN_DEBUG,
                format!("Rerun '{title}'").into(),
                Box::new(RerunSession { index: None }),
                !tab.restart,
                false,
                window,
                cx,
            ),
            self.tool_button(
                "debug-stop",
                IconName::STOP,
                format!("Stop '{title}'").into(),
                Box::new(StopSession { index: None }),
                on("stop"),
                false,
                window,
                cx,
            ),
            Self::separator(cx),
        ];
        let steps: [ToolbarStep; 6] = [
            (
                "debug-resume",
                icons::RESUME,
                "Resume Program",
                Box::new(Resume { index: None }),
                "resume",
            ),
            (
                "debug-pause",
                icons::PAUSE,
                "Pause Program",
                Box::new(Pause { index: None }),
                "pause",
            ),
            (
                "debug-step-over",
                icons::STEP_OVER,
                "Step Over",
                Box::new(StepOver),
                "step",
            ),
            (
                "debug-step-into",
                icons::STEP_INTO,
                "Step Into",
                Box::new(StepInto),
                "step",
            ),
            (
                "debug-step-into-my-code",
                icons::STEP_INTO_MY_CODE,
                "Step Into My Code",
                Box::new(StepIntoMyCode),
                "step",
            ),
            (
                "debug-step-out",
                icons::STEP_OUT,
                "Step Out",
                Box::new(StepOut),
                "step",
            ),
        ];
        for (id, icon, text, action, key) in steps {
            buttons.push(self.tool_button(
                id,
                icon,
                text.into(),
                action,
                on(key),
                false,
                window,
                cx,
            ));
        }
        buttons.push(Self::separator(cx));
        buttons.push(self.tool_button(
            "debug-view-breakpoints",
            icons::VIEW_BREAKPOINTS,
            "View Breakpoints\u{2026}".into(),
            Box::new(ViewBreakpoints),
            true,
            false,
            window,
            cx,
        ));
        buttons.push(self.tool_button(
            "debug-mute",
            icons::MUTE_BREAKPOINTS,
            "Mute Breakpoints".into(),
            Box::new(MuteBreakpoints { muted: None }),
            true,
            self.muted,
            window,
            cx,
        ));
        buttons.push(
            icon_button(
                "debug-more",
                IconName::MORE_VERTICAL,
                px(BUTTON),
                px(ICON),
                false,
                cx,
            )
            .debug_selector(|| "debug-more".into())
            .tooltip(Tooltip::text("More", None))
            .relative()
            .child({
                let anchor = self.more_anchor.clone();
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    let position = this.more_menu_position().unwrap_or(e.position);
                    cx.emit(DebugToolWindowEvent::MoreMenu(position))
                }),
            )
            .into_any_element(),
        );
        let theme = cx.theme();
        let content_tab = |content: Content| {
            let selected = tab.content == content;
            let hover = theme.color(UiRole::TabHover);
            div()
                .id(content.id())
                .debug_selector(move || format!("debug-content-{}", content.id()))
                .flex_none()
                .h(px(26.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(theme.metric(Metric::ButtonRadius))
                .whitespace_nowrap()
                .when(selected, |d| {
                    d.bg(theme.color(UiRole::TabActive))
                        .text_color(theme.color(UiRole::TabActiveText))
                })
                .when(!selected, |d| {
                    d.text_color(theme.color(UiRole::TabInactiveText))
                        .hover(move |s| s.bg(hover))
                })
                .child(content.title())
                .on_click(move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(SelectContent {
                            content: content.id().to_owned(),
                        }),
                        cx,
                    )
                })
        };
        div()
            .id("debug-toolbar")
            .debug_selector(|| "debug-toolbar".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(36.0))
            .px(px(6.0))
            .gap(px(2.0))
            .children(buttons)
            .child(div().w(px(12.0)))
            .child(content_tab(Content::ThreadsVariables))
            .child(content_tab(Content::Console))
            .child(div().flex_1())
            .child(
                icon_button(
                    "debug-layout",
                    icons::LAYOUT,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .debug_selector(|| "debug-layout".into())
                .tooltip(Tooltip::text("Layout Settings", None)),
            )
            .into_any_element()
    }

    /// Threads & Variables: the selected thread's frames and a pane for
    /// the variables. A plain list until the frames and variables views
    /// come (#166).
    fn render_threads_variables(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let s = &tab.snapshot;
        let message = |text: &'static str| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        let frames: AnyElement = match s.phase {
            Phase::Suspended if !s.frames.is_empty() => {
                let selection = theme.color(UiRole::SelectionInactive);
                let thread = s
                    .threads
                    .iter()
                    .find(|t| Some(t.id) == s.selected_thread)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                div()
                    .id("debug-frames")
                    .debug_selector(|| "debug-frames".into())
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .h(px(28.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .child(thread),
                    )
                    .children(s.frames.iter().map(|f| {
                        let selected = s.selected_frame == Some(f.id);
                        div()
                            .h(px(24.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .when(selected, |d| d.bg(selection))
                            .child(frame_text(f))
                    }))
                    .into_any_element()
            }
            Phase::Suspended => message("Collecting data\u{2026}"),
            Phase::Running | Phase::Starting => message("The application is running"),
            Phase::Stopping | Phase::Ended => message("Frames are not available"),
        };
        let variables = match s.phase {
            Phase::Suspended => message("Variables are not available"),
            _ => message("The application is running"),
        };
        let variables = if s.phase == Phase::Suspended {
            variables
        } else {
            div().flex_1().into_any_element()
        };
        div()
            .id("debug-threads-variables")
            .debug_selector(|| "debug-threads-variables".into())
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .border_t_1()
            .border_color(theme.color(UiRole::Border))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(380.0))
                    .h_full()
                    .border_r_1()
                    .border_color(theme.color(UiRole::Border))
                    .child(frames),
            )
            .child(div().flex().flex_1().h_full().child(variables))
            .into_any_element()
    }
}

/// A frame as the frames list shows it (`describe, main.py:7`).
fn frame_text(frame: &debugger::StackFrame) -> String {
    let file = frame
        .source
        .as_ref()
        .and_then(|s| {
            s.path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .or_else(|| s.name.clone())
        })
        .unwrap_or_default();
    if file.is_empty() {
        frame.name.clone()
    } else {
        format!("{}, {file}:{}", frame.name, frame.line)
    }
}

/// Which toolbar actions are enabled in a session's phase.
pub(crate) fn toolbar_state(phase: Phase) -> Json {
    let suspended = phase == Phase::Suspended;
    json!({
        "stop": matches!(phase, Phase::Starting | Phase::Running | Phase::Suspended),
        "resume": suspended,
        "pause": phase == Phase::Running,
        "step": suspended,
    })
}

impl Focusable for DebugToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DebugToolWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx).into_any_element();
        let body = match self.selected() {
            Some(index) => {
                let toolbar = self.render_toolbar(index, window, cx);
                let content = match self.tabs[index].content {
                    Content::ThreadsVariables => self.render_threads_variables(index, cx),
                    Content::Console => div()
                        .id("debug-console")
                        .debug_selector(|| "debug-console".into())
                        .flex_1()
                        .min_h(px(0.0))
                        .child(self.tabs[index].console.clone())
                        .into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(toolbar)
                    .child(content)
                    .into_any_element()
            }
            None => div().flex_1().into_any_element(),
        };
        div()
            .id("debug-tool-window-content")
            .key_context("DebugToolWindow")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(body)
    }
}
