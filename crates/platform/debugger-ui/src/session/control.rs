//! The [`DebuggerView`]'s side of debug sessions (`XDebuggerManagerImpl`,
//! `XDebugSessionImpl` on the UI side): starting a session (the product's
//! step on the UI thread, then the debugger off it), its tab and console,
//! following its state (the execution point, the tool window shown on a
//! stop, the Console selected when it ends), the session commands, Stop,
//! Rerun and closing a tab.

use debugger::{DebugError, DebugRequest, Pending, Phase, Session, SessionId};
use editor::Editor;
use futures::StreamExt as _;
use gpui::{AppContext as _, Context, Entity, Task, Window};
use serde_json::{Value as Json, json};
use ui::icons::IconName;
use ui::notification::{Notification, NotificationKind};

use super::commands::*;
use super::execution_point::{self, ExecutionPoint};
use super::tool_window::{Content, DebugToolWindow, DebugToolWindowEvent};
use crate::view::{DebuggerView, DebuggerViewEvent};

/// A Rerun that waits for its session to end.
pub(crate) struct PendingRestart {
    request: DebugRequest,
    icon: Option<IconName>,
    key: Option<String>,
    tab: u64,
}

/// The reference's title of a session that could not connect or whose
/// debugger failed (`PyBundle` `debug.notification.title.connection.failed`
/// for Python), without the language.
const CONNECTION_FAILED: &str = "Connection to the debugger failed";

impl DebuggerView {
    /// The Debug tool window's content.
    pub fn tool_window(&self) -> &Entity<DebugToolWindow> {
        &self.tool_window
    }

    /// A session of the Debug tool window or the debugger is not ended.
    pub fn any_session_alive(&self, cx: &gpui::App) -> bool {
        self.tool_window.read(cx).any_alive()
    }

    pub(crate) fn tool_window_event(
        &mut self,
        event: &DebugToolWindowEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            DebugToolWindowEvent::Hide => cx.emit(DebuggerViewEvent::HideToolWindow),
            DebugToolWindowEvent::OpenLink(target) => {
                cx.emit(DebuggerViewEvent::OpenLink(target.clone()))
            }
            DebugToolWindowEvent::MoreMenu(position) => {
                cx.emit(DebuggerViewEvent::MoreMenu(Some(*position)))
            }
        }
    }

    // ---- starting ---------------------------------------------------------------

    /// Debugs a run configuration: the product resolves what it needs on
    /// the UI thread, the debugger starts the adapter off it, then a tab
    /// shows the session. `icon` is the tab's (the configuration type's);
    /// `key` is what the window started, in its own terms (see
    /// [`DebugToolWindow::alive_with_key`]).
    pub fn start(
        &mut self,
        request: DebugRequest,
        icon: Option<IconName>,
        key: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_in(request, icon, key, None, window, cx);
    }

    /// A debug run is being prepared.
    pub fn is_starting(&self) -> bool {
        !self.starting.is_empty()
    }

    /// A debug run started as `key` is being prepared.
    pub fn is_starting_key(&self, key: &str) -> bool {
        self.starting.iter().any(|k| k.as_deref() == Some(key))
    }

    /// The product can debug a run configuration (its type has a debug
    /// launch).
    pub fn can_debug(&self, configuration: &run_config::RunConfiguration) -> bool {
        self.hook
            .as_ref()
            .is_some_and(|h| h.provider.can_debug(configuration))
    }

    fn start_in(
        &mut self,
        request: DebugRequest,
        icon: Option<IconName>,
        key: Option<String>,
        tab: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = request.configuration.name.clone();
        let Some(debugger) = self.debugger.clone() else {
            cx.emit(DebuggerViewEvent::Message(format!(
                "Error running '{name}': the debugger is not available"
            )));
            return;
        };
        let resolve = match self.hook.as_ref().and_then(|h| h.resolve.clone()) {
            Some(resolve) => resolve(request, cx),
            None => Task::ready(Ok(request)),
        };
        self.starting.push(key.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let started = match resolve.await {
                Ok(request) => {
                    let started = request.clone();
                    cx.background_spawn(async move { debugger.start(started) })
                        .await
                        .map(|session| (session, request))
                }
                Err(message) => Err(message),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(i) = this.starting.iter().position(|k| *k == key) {
                    this.starting.remove(i);
                }
                this.started(started, &name, icon, key, tab, window, cx);
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn started(
        &mut self,
        started: Result<(Session, DebugRequest), String>,
        name: &str,
        icon: Option<IconName>,
        key: Option<String>,
        tab: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (session, request) = match started {
            Ok(started) => started,
            Err(message) => {
                cx.emit(DebuggerViewEvent::Message(format!(
                    "Error running '{name}': {message}"
                )));
                cx.emit(DebuggerViewEvent::Notify(Notification::new(
                    NotificationKind::Error,
                    format!("Error running '{name}'"),
                    message,
                )));
                cx.notify();
                return;
            }
        };
        let id = session.id();
        self.tool_window
            .update(cx, |w, cx| match tab.and_then(|t| w.index_of(t)) {
                Some(index) => w.replace(index, session, request, cx),
                None => {
                    w.add(session, request, icon, key, window, cx);
                }
            });
        // `runInTerminal` may have come before the tab.
        self.attach_console(id, cx);
        self.refresh_session(id, cx);
        cx.emit(DebuggerViewEvent::ShowToolWindow { activate: false });
        cx.notify();
    }

    /// The program of a session started: its console shows the command
    /// line, then the output; typed input goes to it, or a file does.
    pub(crate) fn attach_console(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let found = {
            let w = self.tool_window.read(cx);
            w.index_of_session(id)
                .map(|i| &w.tabs[i])
                .filter(|t| !t.attached)
                .map(|t| (t.id, t.session.clone(), t.console.clone()))
        };
        let Some((tab_id, session, console)) = found else {
            return;
        };
        let Some((process, events)) = session.take_console() else {
            return;
        };
        let command = session.console_command().unwrap_or_default();
        console.update(cx, |c, cx| {
            c.print_system(&format!("{command}\n"), cx);
            match session.input_file() {
                // "Redirect input from", as a run prints it.
                Some(file) => {
                    c.print_system(
                        &format!("Input is being redirected from {}\n", file.display()),
                        cx,
                    );
                    console_ui::feed_input(process.clone(), file.to_owned(), cx);
                }
                None => c.set_input(Some(process.clone())),
            }
        });
        let mut feed = console_ui::feed(&console, events, cx);
        let watch = cx.spawn(async move |this, cx| {
            while let Some(message) = feed.next().await {
                match message {
                    None => {
                        let _ = console.update(cx, |c, cx| c.request_flush(cx));
                    }
                    Some(status) => {
                        let _ = this.update(cx, |this, cx| this.program_exited(tab_id, status, cx));
                        break;
                    }
                }
            }
        });
        self.tool_window.update(cx, |w, _| {
            if let Some(i) = w.index_of(tab_id) {
                w.tabs[i].attached = true;
                w.tabs[i]._watch = Some(watch);
            }
        });
    }

    fn program_exited(&mut self, tab: u64, status: process::ExitStatus, cx: &mut Context<Self>) {
        self.tool_window.update(cx, |w, cx| {
            if let Some(i) = w.index_of(tab) {
                w.tabs[i].exit = Some(status);
                w.tabs[i].console.update(cx, |c, cx| {
                    c.set_input(None);
                    c.flush(cx);
                });
                cx.notify();
            }
        });
    }

    // ---- following sessions --------------------------------------------------------

    /// A session's state changed: its tab, the execution point, the
    /// editor on a new stop, the Console when it ended.
    pub(crate) fn refresh_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let Some(index) = self.tool_window.read(cx).index_of_session(id) else {
            return;
        };
        let (snapshot, was) = self.tool_window.update(cx, |w, cx| {
            let snapshot = w.tabs[index].session.snapshot();
            let was = w.update_snapshot(index, snapshot.clone(), cx);
            (snapshot, was)
        });
        // A new stop: its tab is selected, the editor shows the line and
        // the tool window shows (`isShowDebuggerOnBreakpoint`) unless it is
        // the end of a step.
        if let Some(point) = ExecutionPoint::of(&snapshot)
            && !self.navigated.as_ref().is_some_and(|n| n.same_stop(&point))
        {
            let step = snapshot.stop.as_ref().is_some_and(|s| s.reason == "step");
            self.tool_window.update(cx, |w, cx| {
                w.select(index, cx);
                w.set_content(index, Content::ThreadsVariables, cx);
            });
            self.navigated = Some(point.clone());
            cx.emit(DebuggerViewEvent::Navigate {
                path: point.path.clone(),
                line: point.line,
            });
            if !step {
                cx.emit(DebuggerViewEvent::ShowToolWindow { activate: false });
            }
        }
        if snapshot.phase == Phase::Ended && was != Phase::Ended {
            self.session_ended(index, cx);
        }
        self.update_execution_point(cx);
    }

    /// Output of the adapter (its messages, or the program's when it
    /// forwards them) goes to the session's console.
    pub(crate) fn session_output(
        &mut self,
        id: SessionId,
        category: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let w = self.tool_window.read(cx);
        let Some(index) = w.index_of_session(id) else {
            return;
        };
        let console = w.tabs[index].console.clone();
        let kind = match category {
            "stderr" => console::ContentKind::Error,
            "stdout" => console::ContentKind::Normal,
            _ => console::ContentKind::System,
        };
        console.update(cx, |c, cx| {
            c.model()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_str(kind, text);
            c.request_flush(cx);
        });
    }

    /// A session ended: its error is shown, the Console is selected; Rerun
    /// debugs again in the tab.
    fn session_ended(&mut self, index: usize, cx: &mut Context<Self>) {
        let (tab_id, error, restart, request, icon, key, console) = {
            let tab = &self.tool_window.read(cx).tabs[index];
            (
                tab.id,
                tab.snapshot.error.clone(),
                tab.restart,
                tab.request.clone(),
                tab.icon,
                tab.key.clone(),
                tab.console.clone(),
            )
        };
        if let Some(error) = &error {
            console.update(cx, |c, cx| {
                c.model()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_str(console::ContentKind::Error, &format!("{error}\n"));
                c.flush(cx);
            });
            let title = self
                .debugger
                .as_ref()
                .and_then(|d| d.provider().connection_failed_title())
                .unwrap_or_else(|| CONNECTION_FAILED.to_owned());
            cx.emit(DebuggerViewEvent::Notify(Notification::new(
                NotificationKind::Error,
                title,
                error.clone(),
            )));
        }
        self.tool_window
            .update(cx, |w, cx| w.set_content(index, Content::Console, cx));
        if restart {
            self.pending_restart = Some(PendingRestart {
                request,
                icon,
                key,
                tab: tab_id,
            });
            cx.emit(DebuggerViewEvent::Restart);
        }
        cx.notify();
    }

    /// Runs a Rerun that waits for its session to end (the workbench calls
    /// it with the window).
    pub fn restart_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.pending_restart.take() {
            self.start_in(p.request, p.icon, p.key, Some(p.tab), window, cx);
        }
    }

    /// The execution point of the selected session, shown in the editors.
    pub(crate) fn update_execution_point(&mut self, cx: &mut Context<Self>) {
        let point = self
            .tool_window
            .read(cx)
            .selected_tab()
            .and_then(|t| ExecutionPoint::of(&t.snapshot));
        if point == self.execution {
            return;
        }
        self.execution = point;
        let editors: Vec<Entity<Editor>> =
            self.gutters.values().map(|g| g.editor.clone()).collect();
        for editor in editors {
            execution_point::show(&editor, self.execution.as_ref(), cx);
        }
        cx.notify();
    }

    /// An editor tab opened: it shows the execution point when it is in
    /// its file.
    pub(crate) fn show_execution_point_in(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        if self.execution.is_some() {
            execution_point::show(editor, self.execution.as_ref(), cx);
        }
    }

    // ---- commands --------------------------------------------------------------

    /// The session a command acts on: the selected tab's, else the last one
    /// started that has not ended.
    fn command_session(&self, cx: &gpui::App) -> Option<Session> {
        self.tool_window
            .read(cx)
            .selected_session()
            .filter(|s| !s.is_ended())
            .or_else(|| self.debugger.as_ref()?.current_session())
    }

    /// The session of the tab at `index`, or the one a command acts on.
    fn session_at(&self, index: Option<usize>, cx: &gpui::App) -> Option<Session> {
        match index {
            Some(i) => self
                .tool_window
                .read(cx)
                .tabs
                .get(i)
                .map(|t| t.session.clone()),
            None => self.command_session(cx),
        }
    }

    /// Runs a session request; a failure shows in the status bar.
    fn run_request(&mut self, request: Pending<()>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            if let Err(error) = request.await
                && error != DebugError::Ended
            {
                let _ = this.update(cx, |_, cx| {
                    cx.emit(DebuggerViewEvent::Message(error.to_string()))
                });
            }
        })
        .detach();
    }

    pub(crate) fn resume_action(&mut self, a: &Resume, cx: &mut Context<Self>) {
        if let Some(s) = self.session_at(a.index, cx) {
            self.run_request(s.resume(), cx);
        }
    }

    pub(crate) fn pause_action(&mut self, a: &Pause, cx: &mut Context<Self>) {
        if let Some(s) = self.session_at(a.index, cx) {
            self.run_request(s.pause(), cx);
        }
    }

    pub(crate) fn step_over_action(&mut self, _: &StepOver, cx: &mut Context<Self>) {
        if let Some(s) = self.command_session(cx) {
            self.run_request(s.step_over(), cx);
        }
    }

    /// Steps into; `false` without a session (the key is not used: F7 is
    /// also Next Difference).
    pub(crate) fn step_into_action(&mut self, _: &StepInto, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.command_session(cx) else {
            return false;
        };
        self.run_request(s.step_into(), cx);
        true
    }

    pub(crate) fn step_into_my_code_action(&mut self, _: &StepIntoMyCode, cx: &mut Context<Self>) {
        if let Some(s) = self.command_session(cx) {
            self.run_request(s.step_into_my_code(), cx);
        }
    }

    pub(crate) fn step_out_action(&mut self, _: &StepOut, cx: &mut Context<Self>) {
        if let Some(s) = self.command_session(cx) {
            self.run_request(s.step_out(), cx);
        }
    }

    /// Run to Cursor and Force Run to Cursor: the line given, or the
    /// caret's in the active editor.
    pub(crate) fn run_to_cursor(
        &mut self,
        file: Option<&str>,
        line: Option<u32>,
        ignore_breakpoints: bool,
        active: Option<&Entity<Editor>>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.command_session(cx) else {
            return;
        };
        let Some((path, line, _)) = self.target(file, line, active, cx) else {
            return;
        };
        self.run_request(session.run_to_position(&path, line, ignore_breakpoints), cx);
    }

    /// Show Execution Point: the editor shows the line again.
    pub(crate) fn show_execution_point_action(
        &mut self,
        _: &ShowExecutionPoint,
        cx: &mut Context<Self>,
    ) {
        if let Some(point) = &self.execution {
            cx.emit(DebuggerViewEvent::Navigate {
                path: point.path.clone(),
                line: point.line,
            });
        }
    }

    fn tab_index(&self, index: Option<usize>, cx: &gpui::App) -> Option<usize> {
        let w = self.tool_window.read(cx);
        index.or(w.selected()).filter(|i| *i < w.tab_count())
    }

    /// Stop: ends the program and its debugger.
    pub fn stop_session(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(index, cx) else {
            return;
        };
        let session = self.tool_window.read(cx).tabs[index].session.clone();
        // Every process is gone when the receiver gets its message.
        drop(session.stop());
        cx.notify();
    }

    /// Kills every session's processes at once (the application quits).
    pub fn kill_all(&self) {
        if let Some(debugger) = &self.debugger {
            debugger.kill_all();
        }
    }

    /// Rerun: the session is stopped and its configuration debugged again
    /// in the same tab once it ended.
    pub fn rerun_session(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(index, cx) else {
            return;
        };
        let (alive, session, request, icon, key, tab) = {
            let t = &self.tool_window.read(cx).tabs[index];
            (
                !t.session.is_ended(),
                t.session.clone(),
                t.request.clone(),
                t.icon,
                t.key.clone(),
                t.id,
            )
        };
        if alive {
            self.tool_window
                .update(cx, |w, _| w.tabs[index].restart = true);
            drop(session.stop());
            cx.notify();
        } else {
            self.start_in(request, icon, key, Some(tab), window, cx);
        }
    }

    /// Closes a session's tab; a running session is stopped.
    pub fn close_session(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(index, cx) else {
            return;
        };
        let Some(tab) = self.tool_window.update(cx, |w, cx| w.remove(index, cx)) else {
            return;
        };
        if let Some(debugger) = &self.debugger {
            drop(debugger.close_session(tab.session.id()));
        } else {
            drop(tab.session.stop());
        }
        if self.tool_window.read(cx).tab_count() == 0 {
            cx.emit(DebuggerViewEvent::Emptied);
        }
        self.update_execution_point(cx);
        cx.notify();
    }

    pub(crate) fn select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tool_window.update(cx, |w, cx| w.select(index, cx));
        self.update_execution_point(cx);
    }

    pub(crate) fn select_content(&mut self, a: &SelectContent, cx: &mut Context<Self>) {
        if let Some(content) = Content::parse(&a.content) {
            self.tool_window
                .update(cx, |w, cx| w.select_content(content, cx));
        }
    }

    pub(crate) fn type_input(&mut self, a: &TypeInput, cx: &mut Context<Self>) {
        let console = self
            .tool_window
            .read(cx)
            .selected_tab()
            .map(|t| t.console.clone());
        if let Some(console) = console {
            console.update(cx, |c, cx| c.type_text(&a.text, cx));
        }
    }

    // ---- frames and variables ------------------------------------------------

    /// The selected tab's session when it is suspended.
    fn suspended_session(&self, cx: &gpui::App) -> Option<Session> {
        self.tool_window
            .read(cx)
            .selected_session()
            .filter(|s| s.phase() == Phase::Suspended)
    }

    pub(crate) fn show_threads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tool_window
            .update(cx, |w, cx| w.show_threads(window, cx));
    }

    pub(crate) fn select_thread(&mut self, a: &SelectThread, cx: &mut Context<Self>) {
        if let Some(session) = self.suspended_session(cx) {
            session.select_thread(a.id);
        }
    }

    /// Selects a frame of the selected thread: the editor shows its line
    /// (the session's change does), the variables view its values.
    pub(crate) fn select_frame(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.suspended_session(cx) else {
            return;
        };
        if let Some(frame) = session.snapshot().frames.get(index) {
            session.select_frame(frame.id);
        }
    }

    /// Previous Frame and Next Frame.
    pub(crate) fn step_frame(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(session) = self.suspended_session(cx) else {
            return;
        };
        if let Some(index) = crate::frames::frame_in_direction(&session.snapshot(), delta) {
            self.select_frame(index, cx);
        }
    }

    pub(crate) fn close_frames_hint(&mut self, cx: &mut Context<Self>) {
        self.tool_window.update(cx, |w, cx| {
            w.frames_hint_closed = true;
            cx.notify();
        });
    }

    /// Runs `f` on the selected tab's variables view.
    pub(crate) fn with_variables(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut crate::variables::VariablesView, &mut Context<crate::variables::VariablesView>),
    ) {
        if let Some(variables) = self.tool_window.read(cx).selected_variables() {
            variables.update(cx, f);
        }
    }

    /// The sessions as `debug.state` shows them in the window: the tool
    /// window and the execution point.
    pub(crate) fn sessions_json(&self, cx: &gpui::App) -> (Json, Json) {
        let point = self.execution.as_ref().map(|p| {
            json!({
                "file": self.shown_path(&p.path),
                "line": p.line,
                "session": p.session.0,
            })
        });
        (
            self.tool_window.read(cx).state_json(cx),
            point.unwrap_or(Json::Null),
        )
    }
}
