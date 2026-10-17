//! The debugger of the window (ADR 0015): the product's debug hook drives
//! a [`DebuggerView`] that opens the project's debugger, keeps the
//! breakpoints of the editor tabs in their gutters, and shows the
//! breakpoint properties popup and the Breakpoints dialog; Jump to Source
//! of the dialog opens the file here. The Debug executor (`Debug`,
//! `DebugClass`, the run widget's, the dropdown's and the Run/Debug
//! Configurations dialog's Debug) starts sessions of run configurations,
//! which the Debug tool window shows. A configuration that may not run in
//! parallel is stopped first, as Run and Debug share it
//! (`ExecutionManagerImpl.restartRunProfile`).

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use debugger_ui::session::commands::{Debug, DebugConfiguration, DebugCurrentFile, DebugFile};
use debugger_ui::session::tool_window::DebugToolWindow;
use debugger_ui::{DebugRequest, DebuggerView, DebuggerViewEvent};
use editor::Editor;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement,
    Subscription, Window, point,
};
use run_config::RunConfiguration;
use serde_json::Value as Json;
use ui::icons::IconName;
use ui::menu::MenuItem;

use super::Workbench;
use super::run_actions::PendingDialog;
use crate::app::Product;
use crate::dialog::{Dialog, DialogButton};
use crate::file_types::FileTypes;
use crate::run::{LaunchKey, RunRequest};
use crate::tool_windows::{
    Area, ToolWindow, ToolWindowContent, ToolWindowSpec, ToolWindowView, ToolWindows,
};

/// What the Debug tool window knows a debug run by: the configuration or
/// the file, as the run widget and the "Is Running" check compare it.
pub(super) fn launch_tag(key: &LaunchKey) -> String {
    match key {
        LaunchKey::File(path) => format!("file:{}", path.display()),
        LaunchKey::Configuration(k) => format!("configuration:{}:{}", k.type_id, k.name),
    }
}

/// A launch that waits for the configuration's run or debug session to
/// stop ("Stop and Rerun").
pub(crate) enum Launch {
    Run {
        request: RunRequest,
    },
    Debug {
        config: Box<RunConfiguration>,
        key: LaunchKey,
    },
}

impl Launch {
    fn key(&self) -> LaunchKey {
        match self {
            Launch::Run { request } => request.launch_key(),
            Launch::Debug { key, .. } => key.clone(),
        }
    }

    fn name(&self) -> String {
        match self {
            Launch::Run { request } => request.name(),
            Launch::Debug { config, .. } => config.name.clone(),
        }
    }
}

/// How often a launch looks whether what it stopped has ended
/// (`ExecutionManagerImpl.awaitTermination`).
const AWAIT_TERMINATION: Duration = Duration::from_millis(50);

impl ToolWindowView for DebugToolWindow {
    fn content_focus(&self, cx: &App) -> FocusHandle {
        DebugToolWindow::content_focus(self, cx)
    }

    fn is_live(&self, _: &App) -> bool {
        self.any_alive()
    }
}

impl Workbench {
    /// The window's debugger UI and its subscriptions.
    pub(super) fn new_debugger_view(
        product: &Product,
        file_types: Rc<FileTypes>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<DebuggerView>, Vec<Subscription>) {
        let view = cx.new(|cx| {
            let mut view = DebuggerView::new(product.debug.clone(), cx);
            view.set_activate(!crate::env::background());
            view.set_highlighters(Rc::new(move |path: &Path| {
                file_types
                    .for_path(path)
                    .and_then(|t| t.highlighter.as_ref())
                    .map(|factory| factory())
            }));
            view
        });
        let tool_window = view.read(cx).tool_window().clone();
        let subscriptions = vec![
            cx.observe(&view, |_, _, cx| cx.notify()),
            cx.observe(&tool_window, |_, _, cx| cx.notify()),
            cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                this.debugger_view_event(event, window, cx)
            }),
        ];
        (view, subscriptions)
    }

    /// The Debug tool window in the bottom area after Run; its first
    /// session registers it (`ToolWindowId.DEBUG`).
    pub(super) fn register_debug_tool_window(
        windows: &mut ToolWindows,
        view: &Entity<DebuggerView>,
        cx: &App,
    ) {
        windows.register(
            ToolWindowSpec {
                id: ToolWindow::Debug,
                area: Area::Bottom,
                order: 10,
                icon: IconName::TOOL_DEBUG,
                title: "Debug",
                command: Some("workbench.toggle-debug"),
                registered: false,
            },
            Some(ToolWindowContent::new(view.read(cx).tool_window().clone())),
        );
    }

    fn debugger_view_event(
        &mut self,
        event: &DebuggerViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DebuggerViewEvent::OpenAt { path, line } => {
                window.activate_window();
                let point = text::Point {
                    line: line.saturating_sub(1) as usize,
                    column: 0,
                };
                self.open_file_at(path.clone(), point, None, window, cx);
            }
            DebuggerViewEvent::Navigate { path, line } => {
                let point = text::Point {
                    line: line.saturating_sub(1) as usize,
                    column: 0,
                };
                // A frame chosen in the frames list or the variables view
                // shows its line and the focus stays there
                // (`XDebuggerUtilImpl.createEditor` focuses the editor only
                // from another editor); a stop brings the focus to the
                // editor.
                let views = self.debugger.read(cx).tool_window().clone();
                let keep = views
                    .read(cx)
                    .frames_or_variables_focused(window, cx)
                    .then(|| window.focused(cx))
                    .flatten();
                let open = self.tabs.iter().any(|t| t.path() == Some(path));
                self.open_file_at(path.clone(), point, None, window, cx);
                match keep {
                    Some(focus) if open => window.focus(&focus),
                    Some(focus) => self.focus_after_open = Some((path.clone(), focus)),
                    None => {}
                }
            }
            DebuggerViewEvent::ShowToolWindow { activate } => {
                if *activate {
                    self.activate_tool_window(ToolWindow::Debug, window, cx);
                } else {
                    self.tool_windows.show(ToolWindow::Debug);
                }
            }
            DebuggerViewEvent::HideToolWindow => self.hide(ToolWindow::Debug, window, cx),
            DebuggerViewEvent::Emptied => {
                // As Run: a tool window that hides on empty content leaves
                // the stripe too.
                if self.is_visible(ToolWindow::Debug) {
                    self.hide(ToolWindow::Debug, window, cx);
                }
                self.tool_windows.set_registered(ToolWindow::Debug, false);
            }
            DebuggerViewEvent::Message(message) => {
                self.message = (!message.is_empty()).then(|| message.clone().into());
            }
            DebuggerViewEvent::Notify(notification) => {
                self.notify(notification.clone(), cx);
            }
            DebuggerViewEvent::OpenLink(target) => self.open_link(target, window, cx),
            DebuggerViewEvent::MoreMenu(position) => {
                let position = position.unwrap_or_else(|| {
                    let size = window.viewport_size();
                    point(size.width / 2.0, size.height / 2.0)
                });
                let items = self.debug_more_items(window, cx);
                self.open_run_menu(items, position, window, cx);
            }
            DebuggerViewEvent::Restart => {
                self.debugger
                    .update(cx, |v, cx| v.restart_pending(window, cx));
            }
        }
        cx.notify();
    }

    /// The Debug toolbar's More menu (`XDebugger.ToolWindow.TopToolbar3.Extra`
    /// without the actions to come, and Debugger Settings).
    fn debug_more_items(&self, window: &Window, cx: &App) -> Vec<MenuItem> {
        use debugger_ui::session::commands::{ForceRunToCursor, RunToCursor, ShowExecutionPoint};
        let keys = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &self.focus, window).unwrap_or_default()
        };
        let suspended = self
            .debugger
            .read(cx)
            .tool_window()
            .read(cx)
            .selected_session()
            .is_some_and(|s| s.phase() == debugger_ui::Phase::Suspended);
        let run_to = RunToCursor {
            file: None,
            line: None,
        };
        let force = ForceRunToCursor {
            file: None,
            line: None,
        };
        vec![
            MenuItem::action("Run to Cursor", run_to.clone())
                .shortcut(keys(&run_to))
                .enabled(suspended),
            MenuItem::action("Force Run to Cursor", force.clone())
                .shortcut(keys(&force))
                .enabled(suspended),
            MenuItem::separator(),
            MenuItem::action("Show Execution Point", ShowExecutionPoint)
                .shortcut(keys(&ShowExecutionPoint))
                .enabled(suspended),
            MenuItem::separator(),
            MenuItem::submenu(
                "Debugger Settings",
                vec![
                    MenuItem::action("Open Files in Preview Tab", ShowExecutionPoint)
                        .enabled(false),
                    MenuItem::action("Show Variable Values in Editor", ShowExecutionPoint)
                        .enabled(false),
                    MenuItem::separator(),
                    MenuItem::action("Sort Variables Alphabetically", ShowExecutionPoint)
                        .enabled(false),
                    MenuItem::separator(),
                    MenuItem::action("Unmute Breakpoints on Session Finish", ShowExecutionPoint)
                        .enabled(false),
                ],
            ),
        ]
    }

    /// The window's debugger UI.
    pub fn debugger_view(&self) -> &Entity<DebuggerView> {
        &self.debugger
    }

    pub(super) fn debug_project_opened(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debugger
            .update(cx, |v, cx| v.project_opened(root, window, cx));
    }

    pub(super) fn debug_tab_opened(
        &mut self,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debugger
            .update(cx, |v, cx| v.editor_opened(editor, window, cx));
    }

    pub(super) fn debug_tab_closed(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        self.debugger
            .update(cx, |v, cx| v.editor_closed(editor, cx));
    }

    /// A file or folder was renamed or moved: its breakpoints follow.
    pub(super) fn debug_files_moved(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let moves = [(from.to_owned(), to.to_owned())];
        self.debugger.update(cx, |v, cx| v.files_moved(&moves, cx));
    }

    /// The breakpoint and session commands, run with the active editor
    /// wherever the focus is in the window, and the Debug executor.
    pub(super) fn debug_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        let root = root
            .on_action(cx.listener(Self::debug_debug))
            .on_action(cx.listener(Self::debug_current_file))
            .on_action(cx.listener(Self::debug_file))
            .on_action(cx.listener(Self::debug_configuration))
            .on_action(cx.listener(Self::toggle_debug));
        debugger_ui::forward_actions(root, self.debugger.clone(), self.active_editor().cloned())
    }

    /// The breakpoint popup and the gutter tooltip over the active editor.
    pub(super) fn debug_overlay(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let active = self.active_editor().cloned();
        self.debugger
            .update(cx, |v, cx| v.render_overlay(active.as_ref(), window, cx))
    }

    /// `debug.state`: the debugger, the Debug tool window's visibility.
    pub fn debug_state(&self, cx: &gpui::App) -> Json {
        let mut state = self.debugger.read(cx).state_json(cx);
        state["visible"] = Json::Bool(self.is_visible(ToolWindow::Debug));
        state["registered"] = Json::Bool(self.tool_windows.is_registered(ToolWindow::Debug));
        state
    }

    /// Kills the sessions' processes (the application quits).
    pub(super) fn kill_debug_sessions(&self, cx: &App) {
        self.debugger.read(cx).kill_all();
    }

    // ---- the Debug executor ------------------------------------------------------

    fn toggle_debug(
        &mut self,
        _: &crate::commands::ToggleDebug,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.focused_tool_window(window, cx) == Some(ToolWindow::Debug);
        if self.is_visible(ToolWindow::Debug) && focused {
            self.hide(ToolWindow::Debug, window, cx);
        } else {
            self.activate_tool_window(ToolWindow::Debug, window, cx);
        }
    }

    /// Debug: the selected configuration, or the current file.
    fn debug_debug(&mut self, _: &Debug, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self
            .run_configs(cx)
            .read(cx)
            .list()
            .selected_config()
            .map(|c| c.key());
        match selected {
            Some(key) => self.debug_configuration_key(&key, window, cx),
            None => self.debug_current(window, cx),
        }
    }

    /// Debug Context Configuration: the file in the editor with its context
    /// configuration (an existing one or a new temporary one, selected).
    fn debug_current_file(
        &mut self,
        _: &DebugCurrentFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((file, true)) = self.current_run_file(cx) else {
            return;
        };
        let key = self
            .run_configs(cx)
            .update(cx, |c, cx| c.context_configuration(&file, cx));
        match key {
            Some(key) => self.debug_configuration_key(&key, window, cx),
            None => self.debug_current(window, cx),
        }
    }

    /// The dropdown's Current File: the file in the editor with its context
    /// configuration, which is not kept.
    fn debug_file(&mut self, _: &DebugFile, window: &mut Window, cx: &mut Context<Self>) {
        self.debug_current(window, cx);
    }

    fn debug_configuration(
        &mut self,
        action: &DebugConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.named_configuration(action.name.as_deref(), action.r#type.as_deref(), cx) {
            Ok(Some(key)) => self.debug_configuration_key(&key, window, cx),
            Ok(None) => self.debug_current(window, cx),
            Err(message) => {
                self.message = Some(message.into());
                cx.notify();
            }
        }
    }

    /// The file in the editor, debugged with the product's context
    /// configuration for it.
    fn debug_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, true)) = self.current_run_file(cx) else {
            return;
        };
        let Some(root) = self.project_root(cx) else {
            return;
        };
        let context = self
            .product
            .run
            .as_ref()
            .and_then(|r| r.0.context_configuration(&file, &root));
        match context {
            Some(config) => self.debug_start(config, LaunchKey::File(file), window, cx),
            None => {
                self.message = Some("The file in the editor cannot be debugged".into());
                cx.notify();
            }
        }
    }

    fn debug_configuration_key(
        &mut self,
        key: &run_config::ConfigKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configs = self.run_configs(cx);
        let Some(config) = configs.read(cx).list().get(key).cloned() else {
            return;
        };
        if config.is_unsupported() {
            self.message = Some(
                format!(
                    "'{}' cannot run: its type is not supported yet",
                    config.name
                )
                .into(),
            );
            cx.notify();
            return;
        }
        self.debug_start(config, LaunchKey::Configuration(key.clone()), window, cx);
    }

    /// The Debug tool window's tab of a session started as `key` that has
    /// not ended.
    pub(super) fn debug_tab_of(&self, key: &LaunchKey, cx: &App) -> Option<usize> {
        self.debugger
            .read(cx)
            .tool_window()
            .read(cx)
            .alive_with_key(&launch_tag(key))
    }

    /// Debugs a configuration (`key` is what it was started as). What runs
    /// or is debugged as `key` already and may not run in parallel is
    /// stopped first, after asking ("Process 'x' Is Running").
    fn debug_start(
        &mut self,
        config: RunConfiguration,
        key: LaunchKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.debugger.read(cx).can_debug(&config) {
            self.message = Some(
                format!(
                    "'{}' cannot be debugged: its type is not supported yet",
                    config.name
                )
                .into(),
            );
            cx.notify();
            return;
        }
        if self.debugger.read(cx).is_starting_key(&launch_tag(&key)) {
            return;
        }
        let parallel = match key {
            LaunchKey::Configuration(_) => self
                .run_configs(cx)
                .read(cx)
                .types()
                .allows_parallel(&config),
            LaunchKey::File(_) => false,
        };
        let running = self.run_window.read(cx).running(&key).is_some();
        if !parallel && (running || self.debug_tab_of(&key, cx).is_some()) {
            self.confirm_stop_and_launch(
                Launch::Debug {
                    config: Box::new(config),
                    key,
                },
                window,
                cx,
            );
            return;
        }
        self.debug_now(config, key, window, cx);
    }

    /// "Process 'x' Is Running" before a launch of something that runs or
    /// is debugged (`userApprovesStopForSameTypeConfigurations`); without
    /// the dialog, Stop and Rerun at once.
    pub(super) fn confirm_stop_and_launch(
        &mut self,
        launch: Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = launch.key();
        // The session of a configuration debugged again restarts in its
        // tab (Rerun), as a run that runs again does.
        let rerun_session = match &launch {
            Launch::Debug { .. } if self.run_window.read(cx).running(&key).is_none() => {
                self.debug_tab_of(&key, cx)
            }
            _ => None,
        };
        if !self.run_settings.confirm_rerun {
            match rerun_session {
                Some(index) => self.debug_rerun_tab(index, window, cx),
                None => self.stop_and_launch(launch, window, cx),
            }
            return;
        }
        let count = usize::from(self.run_window.read(cx).running(&key).is_some())
            + usize::from(self.debug_tab_of(&key, cx).is_some());
        let name = launch.name();
        let stop = if count > 1 {
            format!("{count} running instances")
        } else {
            "the running one".to_owned()
        };
        let dialog = cx.new(|cx| {
            Dialog::new(
                format!("Process '{name}' Is Running"),
                format!(
                    "'{name}' is not allowed to run in parallel.\nWould you like to stop {stop}?"
                ),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "stop-and-rerun",
                        label: "Stop and Rerun".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::QUESTION_DIALOG)
            .with_option("Do not show this dialog in the future")
        });
        self.open_dialog(dialog, PendingDialog::StopAndLaunch(launch), window, cx);
    }

    /// Stop and Rerun: what runs or is debugged as the launch's key is
    /// stopped, and the launch starts once it ended; a debug session
    /// debugged again restarts in its own tab.
    pub(super) fn stop_and_launch(
        &mut self,
        launch: Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = launch.key();
        let run_tab = self.run_window.read(cx).running(&key);
        let debug_tab = self.debug_tab_of(&key, cx);
        if let (Launch::Debug { .. }, None, Some(index)) = (&launch, run_tab, debug_tab) {
            self.debug_rerun_tab(index, window, cx);
            return;
        }
        if let Some(tab) = run_tab {
            self.run_window.update(cx, |w, cx| w.stop(tab, cx));
        }
        if let Some(index) = debug_tab {
            self.debugger
                .update(cx, |v, cx| v.stop_session(Some(index), cx));
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let ended = this.update(cx, |this, cx| {
                    this.run_window.read(cx).running(&key).is_none()
                        && this.debug_tab_of(&key, cx).is_none()
                });
                match ended {
                    Ok(true) => break,
                    Ok(false) => cx.background_executor().timer(AWAIT_TERMINATION).await,
                    Err(_) => return,
                }
            }
            let _ = this.update_in(cx, |this, window, cx| match launch {
                Launch::Run { request } => this.launch_now(request, window, cx),
                Launch::Debug { config, key } => this.debug_now(*config, key, window, cx),
            });
        })
        .detach();
        cx.notify();
    }

    /// Rerun of a debug session's tab.
    fn debug_rerun_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.debugger
            .update(cx, |v, cx| v.rerun_session(Some(index), window, cx));
        cx.notify();
    }

    /// Starts a debug session of a configuration once the modified files
    /// are written.
    fn debug_now(
        &mut self,
        config: RunConfiguration,
        key: LaunchKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.project_root(cx) else {
            return;
        };
        if let LaunchKey::Configuration(key) = &key {
            self.run_configs(cx)
                .update(cx, |c, cx| c.started_with(key, "Debug", cx));
        }
        let icon = self
            .run_configs(cx)
            .read(cx)
            .types()
            .icon_of(&config)
            .map(IconName);
        let mut request = DebugRequest::new(root, config);
        request.source_roots = self.project_files.read(cx).source_roots().to_vec();
        let tag = Some(launch_tag(&key));
        let saved = self.save_all_tasks(cx);
        let view = self.debugger.clone();
        cx.spawn_in(window, async move |_, cx| {
            for write in saved {
                write.await;
            }
            let _ = cx.update(|window, cx| {
                view.update(cx, |v, cx| v.start(request, icon, tag, window, cx));
            });
        })
        .detach();
    }

    /// The Debug tool window's tab debugging a file (Current File).
    pub(super) fn debug_file_running(&self, file: &Path, cx: &App) -> Option<usize> {
        self.debug_tab_of(&LaunchKey::File(file.to_owned()), cx)
    }
}
