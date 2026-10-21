//! The Terminal tool window of the main window (#158): registered in the
//! bottom area at start, its sessions planned with the product's
//! customizers (`crate::terminal::plan`), the confirmation before a tab with
//! a running command closes, and its commands (`workbench.toggle-terminal`,
//! `terminal.open`, `terminal.type`, `terminal.state`).

use gpui::{App, AppContext as _, Context, Entity, Focusable as _, Subscription, Window};
use terminal_ui::{SessionView, TerminalToolWindow, TerminalToolWindowEvent};
use ui::icons::IconName;

use super::Workbench;
use super::run_actions::{CloseRunning, PendingDialog};
use crate::dialog::{Dialog, DialogButton};
use crate::tool_windows::{
    Area, ToolWindow, ToolWindowContent, ToolWindowSpec, ToolWindowView, ToolWindows,
};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Show, focus or hide the Terminal tool window.
    ToggleTerminal => "workbench.toggle-terminal", "Terminal",
        "Shows and focuses the Terminal tool window, opening a tab when it has none; hides it when it has the focus.";
    /// Show the Terminal tool window with a session.
    OpenTerminal { input: Option<String>, shell: Option<String> } => "terminal.open",
        "Open Terminal",
        "Shows and focuses the Terminal tool window, opening a tab when it has none. `shell` (a path) is the shell of the tabs opened from now on in this window (default: the Terminal settings); `input` is typed into the selected session once it starts.",
        r#"{"input": "ls\r", "shell": "/bin/sh"}"#;
    /// Type text into the selected session.
    TypeText { text: String } => "terminal.type", "Type into Terminal",
        "Types text into the selected session of the Terminal tool window (kept until the session starts).",
        r#"{"text": "echo hi\r"}"#;
    /// The Terminal tool window's state.
    State => "terminal.state", "Terminal State",
        "Returns the state of the Terminal tool window as JSON: the tabs (name, title, working directory, exit) and the selected one, the open list of tabs (a menu's state, `null` while closed), and the selected session's size, visible lines, cursor, modes, selection, scrolling, title and exit.";
}

/// The Terminal tool window's id.
pub const TERMINAL: ToolWindow = ToolWindow("terminal");

impl ToolWindowView for TerminalToolWindow {
    fn content_focus(&self, cx: &App) -> gpui::FocusHandle {
        TerminalToolWindow::content_focus(self, cx)
    }
}

/// The Terminal tool window of a main window.
pub(super) struct TerminalUi {
    pub(super) view: Entity<TerminalToolWindow>,
    /// The shell of new tabs (`terminal.open {shell}`), instead of the
    /// Terminal settings' one.
    shell: Option<String>,
    /// A new tab's session is being planned.
    starting: bool,
    /// The tabs (by id) whose closing is being checked or confirmed.
    closing: Vec<u64>,
    _subscription: Subscription,
}

impl TerminalUi {
    /// The view, registered as the content of the Terminal stripe button.
    pub(super) fn new(
        tool_windows: &mut ToolWindows,
        window: &mut Window,
        cx: &mut Context<Workbench>,
    ) -> Self {
        let view = cx.new(TerminalToolWindow::new);
        tool_windows.register(
            ToolWindowSpec {
                id: TERMINAL,
                area: Area::Bottom,
                order: 50,
                icon: IconName::TOOL_TERMINAL,
                title: "Terminal",
                command: Some("workbench.toggle-terminal"),
                registered: true,
            },
            Some(ToolWindowContent::new(view.clone())),
        );
        let subscription = cx.subscribe_in(&view, window, |this, _, event, window, cx| {
            this.terminal_event(event, window, cx)
        });
        Self {
            view,
            shell: None,
            starting: false,
            closing: Vec::new(),
            _subscription: subscription,
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register_query(
        cx,
        "terminal.state",
        std::rc::Rc::new(|_, window, cx| {
            let workbench = window.root::<Workbench>().flatten().ok_or_else(|| {
                ui::command::CommandError::new("not_available", "not a main window")
            })?;
            let view = workbench.read(cx).terminal_ui.view.clone();
            Ok(view.read(cx).state_json(cx))
        }),
    );
}

impl Workbench {
    /// The Terminal tool window's view.
    pub fn terminal_window(&self) -> &Entity<TerminalToolWindow> {
        &self.terminal_ui.view
    }

    /// The selected session of the Terminal tool window.
    pub fn terminal_session(&self, cx: &App) -> Option<Entity<SessionView>> {
        self.terminal_ui.view.read(cx).selected_session().cloned()
    }

    /// The terminal commands on the window's root.
    pub(super) fn terminal_actions<E: gpui::InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        root.on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::open_terminal))
            .on_action(cx.listener(Self::type_into_terminal))
    }

    fn terminal_event(
        &mut self,
        event: &TerminalToolWindowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The echo of a key is drawn at once, as typed text in the
            // editor is (`app.rs`, `observe_keystrokes`).
            TerminalToolWindowEvent::Redraw => crate::shot::draw_now(window),
            TerminalToolWindowEvent::NewTab => self.new_terminal_tab(None, window, cx),
            TerminalToolWindowEvent::CloseRequested(id) => {
                self.terminal_close_requested(*id, window, cx)
            }
            TerminalToolWindowEvent::Hide | TerminalToolWindowEvent::Emptied => {
                self.hide(TERMINAL, window, cx)
            }
        }
    }

    /// Starts a session in a new tab: planned off the UI thread with the
    /// product's customizers, then added and selected; `input` is typed
    /// into it.
    pub fn new_terminal_tab(
        &mut self,
        input: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut request = crate::terminal::request(self.project_root(cx), cx);
        if let Some(shell) = &self.terminal_ui.shell {
            request.shell_path = Some(shell.clone());
        }
        let settings: crate::terminal::TerminalSettings = settings_ui::store::get(cx);
        let plan = crate::terminal::plan(request, cx);
        let view = self.terminal_ui.view.clone();
        self.terminal_ui.starting = true;
        cx.spawn_in(window, async move |this, cx| {
            let plan = plan.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.terminal_ui.starting = false;
                let session = cx.new(|cx| {
                    let mut session = SessionView::new(plan.options, window, cx);
                    let options = terminal_ui::TerminalOptions {
                        option_as_meta: settings.option_as_meta,
                        ..session.options().clone()
                    };
                    session.set_options(options, cx);
                    session
                });
                if let Some(input) = &input {
                    session.update(cx, |s, cx| s.type_text(input, cx));
                }
                view.update(cx, |v, cx| {
                    v.set_close_on_exit(settings.close_session_on_exit);
                    v.add_tab(session, None, window, cx)
                });
            });
        })
        .detach();
    }

    /// A tab was asked to close (`TerminalTabCloseListener.closeQuery`): at
    /// once when the shell has nothing running, or when "When closing a tool
    /// window with a running process" is not Ask; otherwise
    /// `TerminateRemoteProcessDialog` asks first. The check (does the shell
    /// have a child process?) runs off the UI thread.
    fn terminal_close_requested(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.terminal_ui.view.clone();
        let Some(session) = view.read(cx).session_of(id).cloned() else {
            return;
        };
        if self.terminal_ui.closing.contains(&id) {
            return;
        }
        // Terminate, and Disconnect (a terminal has none), close without
        // asking.
        if self.run_settings.close_running != CloseRunning::Ask {
            view.update(cx, |v, cx| v.close_tab_by_id(id, window, cx));
            return;
        }
        self.terminal_ui.closing.push(id);
        let running = session.read(cx).has_running_commands(cx);
        cx.spawn_in(window, async move |this, cx| {
            let running = running.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.terminal_close_checked(id, running, window, cx)
            });
        })
        .detach();
    }

    /// The check of a closing tab is done: a shell with nothing running
    /// closes with its tab, one with a running command asks ("Process
    /// 'Terminal Local' Is Running", Terminate or Cancel, no Disconnect).
    fn terminal_close_checked(
        &mut self,
        id: u64,
        running: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.terminal_ui.view.clone();
        let tab = view
            .read(cx)
            .index_of(id)
            .and_then(|i| view.read(cx).tabs().get(i))
            .map(|t| t.name().to_string());
        self.terminal_ui.closing.retain(|c| *c != id);
        let Some(name) = tab else {
            return;
        };
        if !running {
            view.update(cx, |v, cx| v.close_tab_by_id(id, window, cx));
            return;
        }
        // Another dialog is open: the request is dropped, as a Cancel.
        if self.dialog.is_some() {
            return;
        }
        self.terminal_ui.closing.push(id);
        let session = format!("Terminal {name}");
        let dialog = cx.new(|cx| {
            Dialog::new(
                format!("Process '{session}' Is Running"),
                format!("Do you want to terminate the process '{session}'?"),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "terminate",
                        label: "&Terminate".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::WARNING_DIALOG)
            .with_option("Don't ask again")
        });
        self.open_dialog(dialog, PendingDialog::CloseTerminal(id), window, cx);
    }

    /// The confirmation of a closing tab was answered: Terminate closes the
    /// tab (its session ends with it), and with "Don't ask again" the choice
    /// is remembered; Cancel keeps it.
    pub(super) fn terminal_close_answered(
        &mut self,
        id: u64,
        button: Option<&'static str>,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.terminal_ui.closing.retain(|c| *c != id);
        if button != Some("terminate") {
            return;
        }
        if remember {
            self.run_settings.close_running = CloseRunning::Terminate;
        }
        let view = self.terminal_ui.view.clone();
        view.update(cx, |v, cx| v.close_tab_by_id(id, window, cx));
    }

    /// The reference's Terminal activation: hidden when it has the focus,
    /// otherwise shown and focused, with a new tab when it has none.
    fn toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .terminal_ui
            .view
            .focus_handle(cx)
            .contains_focused(window, cx);
        if self.is_visible(TERMINAL) && focused {
            self.hide(TERMINAL, window, cx);
        } else {
            self.show_terminal(None, window, cx);
        }
    }

    /// The Terminal tool window was activated (its key, its stripe button,
    /// `workbench.activate-tool-window`): a new tab when it has none.
    pub(super) fn terminal_activated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_session(cx).is_none() && !self.terminal_ui.starting {
            self.new_terminal_tab(None, window, cx);
        }
    }

    /// Shows and focuses the Terminal tool window; a new tab when it has
    /// none, with `input` typed into the selected session.
    fn show_terminal(
        &mut self,
        input: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.terminal_session(cx) {
            Some(session) => {
                if let Some(input) = &input {
                    session.update(cx, |s, cx| s.type_text(input, cx));
                }
            }
            None if !self.terminal_ui.starting => self.new_terminal_tab(input, window, cx),
            None => {}
        }
        self.activate_tool_window(TERMINAL, window, cx);
    }

    pub(crate) fn open_terminal(
        &mut self,
        action: &OpenTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.shell.is_some() {
            self.terminal_ui.shell = action.shell.clone();
        }
        self.show_terminal(action.input.clone(), window, cx);
    }

    fn type_into_terminal(&mut self, action: &TypeText, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.terminal_session(cx) {
            session.update(cx, |s, cx| s.type_text(&action.text, cx));
        }
    }
}
