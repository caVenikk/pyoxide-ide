//! A development command that shows one terminal session in the Terminal
//! tool window's place (the bottom area), for screenshots and measurements
//! of the terminal view before the Terminal tool window with its tabs
//! exists. The tool window replaces it.

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Window, div, px,
};
use terminal_ui::{SessionView, SessionViewEvent, StartRequest};
use themes::Metric;
use ui::icons::IconName;
use ui::theme::ThemeExt;

use super::Workbench;
use crate::tool_windows::{Area, ToolWindow, ToolWindowContent, ToolWindowSpec, ToolWindowView};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Show a terminal session (development).
    OpenDevTerminal { input: Option<String>, shell: Option<String> } => "terminal.dev-session",
        "Terminal Session (Development)",
        "Shows a terminal session in the project root in the bottom area and focuses it: the default shell, or `shell` (a path) for a new session; `input` is typed into it once it starts. A stand-in until the Terminal tool window has tabs.",
        r#"{"input": "ls\r", "shell": "/bin/sh"}"#;
    /// The shown session's state.
    DevTerminalState => "terminal.dev-state", "Terminal Session State (Development)",
        "Returns the state of the development terminal session as JSON (size, visible lines, cursor, modes, selection, scrolling, title, exit).";
}

/// The Terminal tool window's title and one session.
pub struct DevTerminal {
    session: Entity<SessionView>,
    _redraw: Subscription,
}

impl Focusable for DevTerminal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.session.focus_handle(cx)
    }
}

impl ToolWindowView for DevTerminal {}

impl Render for DevTerminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(theme.metric(Metric::ToolWindowHeaderHeight))
                    .pl(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Terminal"),
            )
            .child(div().flex_1().min_h(px(0.0)).child(self.session.clone()))
    }
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register_query(
        cx,
        "terminal.dev-state",
        std::rc::Rc::new(|_, window, cx| {
            let workbench = window.root::<Workbench>().flatten().ok_or_else(|| {
                ui::command::CommandError::new("not_available", "not a main window")
            })?;
            let session = workbench.read(cx).dev_terminal_session(cx).ok_or_else(|| {
                ui::command::CommandError::new("not_available", "no terminal session is shown")
            })?;
            Ok(session.read(cx).state())
        }),
    );
}

const TERMINAL: ToolWindow = ToolWindow("terminal");

impl Workbench {
    /// The session of the development terminal, when it is shown.
    pub fn dev_terminal_session(&self, cx: &App) -> Option<Entity<SessionView>> {
        let content = self.tool_windows.get(TERMINAL)?.content.as_ref()?;
        let view = content.view().downcast::<DevTerminal>().ok()?;
        Some(view.read(cx).session.clone())
    }

    pub(crate) fn open_dev_terminal(
        &mut self,
        action: &OpenDevTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = match self.dev_terminal_session(cx) {
            Some(session) => session,
            None => {
                let request = StartRequest {
                    shell_path: action.shell.clone(),
                    project_root: self.project_root(cx),
                    ..StartRequest::default()
                };
                let session = cx.new(|cx| SessionView::local(&request, window, cx));
                let view = cx.new(|cx| DevTerminal {
                    session: session.clone(),
                    // The echo of a key is drawn at once, as typed text in
                    // the editor is (`app.rs`, `observe_keystrokes`).
                    _redraw: cx.subscribe_in(&session, window, |_, _, event, window, _| {
                        if *event == SessionViewEvent::Redraw {
                            crate::shot::draw_now(window);
                        }
                    }),
                });
                self.register_tool_window(
                    ToolWindowSpec {
                        id: TERMINAL,
                        area: Area::Bottom,
                        order: 50,
                        icon: IconName::TOOL_TERMINAL,
                        title: "Terminal",
                        command: None,
                        registered: true,
                    },
                    Some(ToolWindowContent::new(view)),
                    cx,
                );
                session
            }
        };
        self.activate_tool_window(TERMINAL, window, cx);
        if let Some(input) = &action.input {
            session.update(cx, |s, cx| s.type_text(input, cx));
        }
    }
}
