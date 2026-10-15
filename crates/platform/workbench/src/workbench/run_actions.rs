//! Running files and run configurations from the main window: the run
//! commands, the run widget's state and popups (the configuration
//! dropdown), the Run tool window's events and the dialogs that confirm
//! stopping a process (the reference's `ExecutorAction`, `StopAction`,
//! `RunToolbarPopup`, `RunContentManagerImpl` and `ExecutionManagerImpl`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use console::LinkTarget;
use gpui::{
    AnyElement, App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable as _,
    Pixels, Point, Subscription, Task, Window, point, px,
};
use serde_json::json;
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};

use super::{ToolWindow, Workbench};
use crate::dialog::{Dialog, DialogButton, DialogEvent};
use crate::run::commands::*;
use crate::run::dropdown::{self, Row};
use crate::run::widget::{SelectedConfig, WidgetState};
use crate::run::{LaunchKey, RunConfigs, RunRequest, RunToolWindowEvent};

/// What closing an open dialog leads to.
pub(super) enum PendingDialog {
    /// "Process 'x' Is Running": Stop and Rerun the tab.
    Rerun(usize),
    /// Closing a tab whose process runs: Terminate or Disconnect.
    CloseRunning(usize),
    /// A dialog about a file changed on disk (`file_sync`).
    FileSync(super::file_sync::SyncDialog),
}

/// What closing a tab with a running process does
/// (`GeneralSettings.processCloseConfirmation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseRunning {
    Ask,
    Terminate,
    Disconnect,
}

/// The run settings of this session.
#[derive(Debug, Clone, Copy)]
pub(super) struct RunSettings {
    /// Ask before stopping a running file to run it again
    /// (`confirm.rerun.with.termination`).
    pub confirm_rerun: bool,
    pub close_running: CloseRunning,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            confirm_rerun: true,
            close_running: CloseRunning::Ask,
        }
    }
}

/// An open dialog: the entity, what follows it, where the focus returns.
pub(super) struct OpenDialog {
    pub dialog: Entity<Dialog>,
    pending: PendingDialog,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// An open popup menu of the run widget or a Run tab and where it is.
pub(super) struct OpenMenu {
    pub menu: Entity<ContextMenu>,
    pub position: Point<Pixels>,
    _subscription: Subscription,
}

impl Workbench {
    /// The file Run runs and whether the product can run it: the active
    /// editor's file (Pyoxide shows one editor at a time, so it is the
    /// reference's single visible editor even without the focus).
    fn current_run_file(&self, cx: &App) -> Option<(PathBuf, bool)> {
        let file = self.active_file(cx)?;
        let runnable = self
            .product
            .run
            .as_ref()
            .is_some_and(|r| r.0.can_run(&file, cx));
        Some((file, runnable))
    }

    /// The project's run configurations.
    fn run_configs(&self, cx: &App) -> Entity<RunConfigs> {
        self.run_window.read(cx).configs().clone()
    }

    /// A project was opened: its run configurations load (and the
    /// reference's are imported once), off the UI thread.
    pub(super) fn run_project_opened(&mut self, root: &Path, cx: &mut Context<Self>) {
        self.run_window
            .update(cx, |w, cx| w.project_opened(root, cx));
    }

    /// What the run widget shows.
    pub fn run_widget_state(&self, window: &Window, cx: &App) -> WidgetState {
        let run = self.run_window.read(cx);
        let configs = run.configs().read(cx);
        let file = self.current_run_file(cx);
        let path = file.as_ref().map(|(f, _)| f.clone());
        let selected = configs.list().selected_config();
        // What Run runs: the selected configuration, or the current file.
        let key = match selected {
            Some(c) => Some(LaunchKey::Configuration(c.key())),
            None => path.clone().map(LaunchKey::File),
        };
        WidgetState {
            configuration: selected.map(|c| SelectedConfig {
                name: c.name.clone(),
                icon: configs.types().icon_of(c).map(IconName),
                temporary: c.temporary,
                supported: !c.is_unsupported(),
            }),
            file: file.map(|(f, runnable)| {
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (name, runnable)
            }),
            file_running: key.as_ref().is_some_and(|k| run.running(k).is_some()),
            launching: key.as_ref().is_some_and(|k| run.is_launching_key(k)),
            running: run
                .alive_tabs()
                .into_iter()
                .map(|i| run.tabs()[i].title().to_owned())
                .collect(),
            run_keys: ui::keys::shortcut_in(&Run, &self.focus, window).map(Into::into),
            stop_keys: ui::keys::shortcut_in(&Stop { tab: None }, &self.focus, window)
                .map(Into::into),
        }
    }

    /// The state reported by `run.state`.
    pub fn run_state_json(&self, window: &Window, cx: &App) -> serde_json::Value {
        let mut state = self.run_window.read(cx).state_json(cx);
        state["visible"] = json!(self.run_visible);
        state["registered"] = json!(self.run_registered);
        state["alive"] = json!(self.run_window.read(cx).any_alive());
        state["widget"] = self.run_widget_state(window, cx).state_json();
        state["configurations"] = self.run_configs(cx).read(cx).state_json();
        state["menu"] = self
            .run_menu
            .as_ref()
            .map_or(json!(null), |m| m.menu.read(cx).state_json());
        state
    }

    fn run_request(&self, file: PathBuf, cx: &App) -> RunRequest {
        RunRequest {
            file,
            configuration: None,
            project_root: self.project_root(cx),
            source_roots: self.project_files.read(cx).source_roots(),
        }
    }

    /// Run: the selected configuration, or the current file.
    fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self
            .run_configs(cx)
            .read(cx)
            .list()
            .selected_config()
            .map(|c| c.key());
        match selected {
            Some(key) => self.run_configuration(&key, window, cx),
            None => self.run_current(window, cx),
        }
    }

    /// Runs a configuration of a supported type.
    fn run_configuration(
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
        let parallel = configs.read(cx).types().allows_parallel(&config);
        let request = RunRequest {
            file: PathBuf::new(),
            configuration: Some(Arc::new(config)),
            project_root: self.project_root(cx),
            source_roots: self.project_files.read(cx).source_roots(),
        };
        self.launch_request(request, parallel, window, cx);
    }

    /// Run Context Configuration: the current file.
    fn run_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, true)) = self.current_run_file(cx) else {
            return;
        };
        let request = self.run_request(file, cx);
        self.launch_request(request, false, window, cx);
    }

    /// Starts a run. What runs already and may not run in parallel is
    /// stopped and run again after asking ("Process 'x' Is Running",
    /// `userApprovesStopForSameTypeConfigurations`).
    fn launch_request(
        &mut self,
        request: RunRequest,
        parallel: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = request.launch_key();
        let run = self.run_window.read(cx);
        if run.is_launching_key(&key) {
            return;
        }
        if let Some(tab) = run.running(&key).filter(|_| !parallel) {
            if self.run_settings.confirm_rerun {
                let name = run.tabs()[tab].title().to_owned();
                let dialog = cx.new(|cx| {
                    Dialog::new(
                        format!("Process '{name}' Is Running"),
                        format!(
                            "'{name}' is not allowed to run in parallel.\nWould you like to stop the running one?"
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
                self.open_dialog(dialog, PendingDialog::Rerun(tab), window, cx);
            } else {
                self.rerun_tab(tab, window, cx);
            }
            return;
        }
        if let LaunchKey::Configuration(key) = &key {
            self.run_configs(cx).update(cx, |c, cx| c.started(key, cx));
        }
        let saved = self.save_all_tasks(cx);
        self.show_run_for_launch(window, cx);
        self.run_window
            .update(cx, |w, cx| w.launch(request, None, saved, window, cx));
        cx.notify();
    }

    /// The Run tool window shows without taking the focus
    /// (`activateToolWindowBeforeRun`, not `focusToolWindowBeforeRun`); it
    /// takes it when nothing has it.
    fn show_run_for_launch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_registered = true;
        self.run_visible = true;
        if window.focused(cx).is_none() {
            let focus = self.run_window.read(cx).content_focus(cx);
            window.focus(&focus);
        }
    }

    fn rerun_tab(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        let saved = self.save_all_tasks(cx);
        self.show_run_for_launch(window, cx);
        self.run_window
            .update(cx, |w, cx| w.rerun(tab, saved, window, cx));
        cx.notify();
    }

    pub(super) fn run_run(&mut self, _: &Run, window: &mut Window, cx: &mut Context<Self>) {
        self.run_selected(window, cx);
    }

    /// The reference creates a temporary configuration for the context;
    /// until run configurations exist this runs the current file.
    pub(super) fn run_current_file(
        &mut self,
        _: &RunCurrentFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_current(window, cx);
    }

    /// Stop: a given tab, the selected tab when the Run tool window has the
    /// focus, the only running process, or a choice of them.
    pub(super) fn run_stop(&mut self, action: &Stop, window: &mut Window, cx: &mut Context<Self>) {
        let run = self.run_window.read(cx);
        let alive = run.alive_tabs();
        let tab = match action.tab {
            Some(tab) => Some(tab),
            None if self.focused_tool_window(window, cx) == Some(ToolWindow::Run) => run
                .selected()
                .filter(|s| alive.contains(s))
                .or_else(|| (alive.len() == 1).then(|| alive[0])),
            None => match alive.as_slice() {
                [] => None,
                [one] => Some(*one),
                _ => {
                    self.open_stop_popup(window, cx);
                    return;
                }
            },
        };
        if let Some(tab) = tab {
            self.run_window.update(cx, |w, cx| w.stop(tab, cx));
        }
        cx.notify();
    }

    /// The Stop button's popup for several processes
    /// (`RunToolbarPopup.createStopPopup`).
    fn open_stop_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let run = self.run_window.read(cx);
        let alive = run.alive_tabs();
        let mut items: Vec<MenuItem> = alive
            .iter()
            .map(|&i| {
                MenuItem::action(
                    format!("Stop \u{2018}{}\u{2019}", run.tabs()[i].title()),
                    Stop { tab: Some(i) },
                )
            })
            .collect();
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            format!("Stop All {}", alive.len()),
            StopAll,
        ));
        let anchor = self.run_anchors.stop.get();
        let position = if anchor.size.width > px(0.0) {
            anchor.bottom_left()
        } else {
            let size = window.viewport_size();
            point(size.width / 2.0, size.height / 3.0)
        };
        self.open_run_menu(items, position, window, cx);
    }

    pub(super) fn run_stop_all(&mut self, _: &StopAll, _: &mut Window, cx: &mut Context<Self>) {
        let alive = self.run_window.read(cx).alive_tabs();
        self.run_window.update(cx, |w, cx| {
            for tab in alive {
                w.stop(tab, cx);
            }
        });
    }

    pub(super) fn run_rerun(&mut self, _: &Rerun, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.run_window.read(cx).selected() else {
            return;
        };
        if self.run_window.read(cx).can_rerun(tab) {
            self.rerun_tab(tab, window, cx);
        }
    }

    pub(super) fn run_close_tab(
        &mut self,
        action: &CloseTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = action.index.or(self.run_window.read(cx).selected()) {
            self.run_window.update(cx, |w, cx| w.request_close(tab, cx));
        }
    }

    pub(super) fn run_close_other_tabs(
        &mut self,
        _: &CloseOtherTabs,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run = self.run_window.read(cx);
        let Some(selected) = run.selected() else {
            return;
        };
        // From the end, so that the indices stay valid; running ones ask.
        let others: Vec<usize> = (0..run.tabs().len())
            .rev()
            .filter(|i| *i != selected)
            .collect();
        for tab in others {
            self.run_window.update(cx, |w, cx| w.request_close(tab, cx));
        }
    }

    pub(super) fn run_close_all_tabs(
        &mut self,
        _: &CloseAllTabs,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.run_window.read(cx).tabs().len();
        for tab in (0..count).rev() {
            self.run_window.update(cx, |w, cx| w.request_close(tab, cx));
        }
    }

    pub(super) fn run_select_tab(
        &mut self,
        action: &SelectTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_window
            .update(cx, |w, cx| w.select(action.index, cx));
        if self.run_visible {
            let focus = self.run_window.read(cx).content_focus(cx);
            window.focus(&focus);
        }
    }

    pub(super) fn run_toggle_pin(
        &mut self,
        action: &TogglePin,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = action.index.or(self.run_window.read(cx).selected()) {
            self.run_window.update(cx, |w, cx| w.toggle_pin(tab, cx));
        }
    }

    pub(super) fn run_previous_link(
        &mut self,
        _: &PreviousLink,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_window.update(cx, |w, cx| w.go_to_link(false, cx));
    }

    pub(super) fn run_next_link(&mut self, _: &NextLink, _: &mut Window, cx: &mut Context<Self>) {
        self.run_window.update(cx, |w, cx| w.go_to_link(true, cx));
    }

    pub(super) fn run_scroll_to_end(
        &mut self,
        _: &ScrollToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_window.update(cx, |w, cx| w.scroll_to_end(cx));
    }

    pub(super) fn run_clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        self.run_window.update(cx, |w, cx| w.clear(cx));
    }

    pub(super) fn run_send_eof(&mut self, _: &SendEof, _: &mut Window, cx: &mut Context<Self>) {
        self.run_window.update(cx, |w, cx| w.send_eof(cx));
    }

    pub(super) fn run_type(&mut self, action: &TypeInput, _: &mut Window, cx: &mut Context<Self>) {
        let text = action.text.clone();
        self.run_window.update(cx, |w, cx| w.type_text(&text, cx));
    }

    /// The run configuration dropdown (`RunToolbarPopup`): pinned and
    /// recent configurations, every configuration by type with folders as
    /// submenus, Current File, Edit Configurations…; the selected row is
    /// preselected and typing filters the rows.
    pub(super) fn run_choose_configuration(
        &mut self,
        _: &ChooseConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configs = self.run_configs(cx);
        let configs = configs.read(cx);
        let list = configs.list();
        let types = configs.types();
        let item = |key: &run_config::ConfigKey| -> Option<MenuItem> {
            let c = list.get(key)?;
            let icon = types
                .icon_of(c)
                .map(IconName)
                .unwrap_or(IconName::FILE_UNKNOWN);
            let mut description = types.name_of(c).to_owned();
            if c.is_unsupported() {
                description.push_str(", not supported yet");
            }
            Some(
                MenuItem::action(
                    c.name.clone(),
                    SelectConfiguration {
                        name: Some(c.name.clone()),
                        r#type: Some(c.type_id.clone()),
                    },
                )
                .icon(icon)
                .description(description),
            )
        };
        let mut items: Vec<MenuItem> = Vec::new();
        let mut preselect = None;
        for row in dropdown::rows(list, types, configs.show_all) {
            match row {
                Row::Header(text) => items.push(MenuItem::unavailable(text)),
                Row::Configuration { key, .. } => {
                    if list.selected.as_ref() == Some(&key) && preselect.is_none() {
                        preselect = Some(items.len());
                    }
                    items.extend(item(&key));
                }
                Row::Folder { name, keys } => {
                    let sub: Vec<MenuItem> = keys.iter().filter_map(&item).collect();
                    items.push(MenuItem::submenu(name, sub).icon(IconName::FOLDER));
                }
                Row::Toggle { count, expanded } => {
                    let toggle =
                        MenuItem::action(dropdown::toggle_text(count), ToggleAllConfigurations);
                    items.push(if expanded {
                        toggle.icon(IconName::CHECKED)
                    } else {
                        toggle
                    });
                }
                Row::Separator => {
                    if items
                        .last()
                        .is_some_and(|i| !matches!(i, MenuItem::Separator))
                    {
                        items.push(MenuItem::separator());
                    }
                }
                Row::CurrentFile => {
                    if list.selected.is_none() {
                        preselect = Some(items.len());
                    }
                    items.push(MenuItem::action("Current File", SelectCurrentFile));
                }
                Row::EditConfigurations => {
                    items.push(MenuItem::unavailable("Edit Configurations\u{2026}"))
                }
            }
        }
        let anchor = self.run_anchors.selector.get();
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                title: None,
                speed_search: true,
                selected: preselect,
            },
            window,
            cx,
        );
        self.open_run_menu_entity(menu, anchor.bottom_left(), cx);
    }

    pub(super) fn run_select_current_file(
        &mut self,
        _: &SelectCurrentFile,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_configs(cx).update(cx, |c, cx| c.select(None, cx));
        cx.notify();
    }

    /// The configuration a command names: by name (and type), or the
    /// selected one.
    fn named_configuration(
        &self,
        name: Option<&str>,
        type_id: Option<&str>,
        cx: &App,
    ) -> Result<Option<run_config::ConfigKey>, String> {
        let configs = self.run_configs(cx);
        let configs = configs.read(cx);
        match name {
            Some(name) => configs.find(name, type_id).map(Some),
            None => Ok(configs.list().selected.clone()),
        }
    }

    pub(super) fn run_select_configuration(
        &mut self,
        action: &SelectConfiguration,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.named_configuration(action.name.as_deref(), action.r#type.as_deref(), cx) {
            Ok(key) => {
                self.run_configs(cx).update(cx, |c, cx| c.select(key, cx));
            }
            Err(message) => self.message = Some(message.into()),
        }
        cx.notify();
    }

    pub(super) fn run_launch_configuration(
        &mut self,
        action: &LaunchConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.named_configuration(action.name.as_deref(), action.r#type.as_deref(), cx) {
            Ok(Some(key)) => self.run_configuration(&key, window, cx),
            Ok(None) => self.run_current(window, cx),
            Err(message) => {
                self.message = Some(message.into());
                cx.notify();
            }
        }
    }

    pub(super) fn run_toggle_all_configurations(
        &mut self,
        _: &ToggleAllConfigurations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_configs(cx).update(cx, |c, cx| {
            c.show_all = !c.show_all;
            cx.notify();
        });
        self.run_choose_configuration(&ChooseConfiguration, window, cx);
    }

    pub(super) fn run_import_reference(
        &mut self,
        _: &ImportReference,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_configs(cx).update(cx, |c, cx| c.import_again(cx));
    }

    /// More Actions of the run widget in Current File mode: Run with
    /// Parameters… (it needs run configurations).
    pub(super) fn run_more_actions(
        &mut self,
        _: &MoreActions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items =
            vec![MenuItem::action("Run with Parameters\u{2026}", SelectCurrentFile).enabled(false)];
        let anchor = self.run_anchors.more.get();
        self.open_run_menu(items, anchor.bottom_left(), window, cx);
    }

    fn open_run_menu(
        &mut self,
        items: Vec<MenuItem>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = ContextMenu::build(items, window, cx);
        self.open_run_menu_entity(menu, position, cx);
    }

    fn open_run_menu_entity(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.run_menu = None;
            cx.notify();
        });
        self.run_menu = Some(OpenMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The tab context menu of the Run tool window.
    fn open_tab_menu(
        &mut self,
        tab: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pinned = self
            .run_window
            .read(cx)
            .tabs()
            .get(tab)
            .is_some_and(|t| t.is_pinned());
        let count = self.run_window.read(cx).tabs().len();
        let items = vec![
            MenuItem::action("Close Tab", CloseTab { index: Some(tab) }),
            MenuItem::action("Close Other Tabs", CloseOtherTabs).enabled(count > 1),
            MenuItem::action("Close All Tabs", CloseAllTabs),
            MenuItem::separator(),
            MenuItem::action(
                if pinned { "Unpin Tab" } else { "Pin Tab" },
                TogglePin { index: Some(tab) },
            ),
        ];
        self.open_run_menu(items, position, window, cx);
    }

    /// The Run tool window asks for something.
    pub(super) fn run_window_event(
        &mut self,
        event: &RunToolWindowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RunToolWindowEvent::Hide => self.hide(ToolWindow::Run, window, cx),
            RunToolWindowEvent::Emptied => {
                // `ToolWindowContentUi.contentRemoved`: a tool window that
                // hides on empty content also leaves the stripe; the next
                // run or ⌘4 brings its button back.
                if self.run_visible {
                    self.hide(ToolWindow::Run, window, cx);
                }
                self.run_registered = false;
                cx.notify();
            }
            RunToolWindowEvent::OpenLink(target) => self.open_link(target, window, cx),
            RunToolWindowEvent::Message(message) => {
                self.message = (!message.is_empty()).then(|| message.clone().into());
            }
            RunToolWindowEvent::Notice(text) => self.show_notice(text.clone().into(), cx),
            RunToolWindowEvent::ConfirmClose(tab) => self.confirm_close_running(*tab, window, cx),
            RunToolWindowEvent::TabMenu(tab, position) => {
                self.open_tab_menu(*tab, *position, window, cx)
            }
            RunToolWindowEvent::MoreMenu(position) => {
                let items = vec![
                    MenuItem::action("Modify Run Configuration\u{2026}", SelectCurrentFile)
                        .enabled(false),
                ];
                self.open_run_menu(items, *position, window, cx);
            }
        }
        cx.notify();
    }

    /// Follows a console link: a file opens at its line, a URL in the
    /// browser.
    fn open_link(&mut self, target: &LinkTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            LinkTarget::File { path, line, column } => match line {
                Some(line) => {
                    let point = text::Point {
                        line: (*line as usize).saturating_sub(1),
                        column: column.map_or(0, |c| (c as usize).saturating_sub(1)),
                    };
                    self.open_file_at(path.clone(), point, None, window, cx)
                }
                None => self.open_file(path.clone(), window, cx),
            },
            LinkTarget::Url(url) => cx.open_url(url),
        }
    }

    /// Closing a tab whose process runs asks first
    /// (`RunContentManagerImpl.CloseListener`, `TerminateRemoteProcessDialog`).
    fn confirm_close_running(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.run_settings.close_running {
            CloseRunning::Terminate => {
                self.run_window.update(cx, |w, cx| w.close(tab, true, cx));
                return;
            }
            CloseRunning::Disconnect => {
                self.run_window.update(cx, |w, cx| w.close(tab, false, cx));
                return;
            }
            CloseRunning::Ask => {}
        }
        let Some(name) = self
            .run_window
            .read(cx)
            .tabs()
            .get(tab)
            .map(|t| t.title().to_owned())
        else {
            return;
        };
        let dialog = cx.new(|cx| {
            Dialog::new(
                format!("Process '{name}' Is Running"),
                format!("Do you want to terminate the process '{name}'?"),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "disconnect",
                        label: "&Disconnect".into(),
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
        self.open_dialog(dialog, PendingDialog::CloseRunning(tab), window, cx);
    }

    pub(super) fn open_dialog(
        &mut self,
        dialog: Entity<Dialog>,
        pending: PendingDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = window.focused(cx);
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            let DialogEvent::Closed { button, checked } = event;
            this.dialog_closed(*button, *checked, window, cx);
        });
        window.focus(&dialog.focus_handle(cx));
        self.dialog = Some(OpenDialog {
            dialog,
            pending,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn dialog_closed(
        &mut self,
        button: Option<&'static str>,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.dialog.take() else {
            return;
        };
        match open.return_focus {
            Some(focus) => window.focus(&focus),
            None => window.focus(&self.focus),
        }
        match (open.pending, button) {
            (PendingDialog::Rerun(tab), Some("stop-and-rerun")) => {
                if checked {
                    self.run_settings.confirm_rerun = false;
                }
                self.rerun_tab(tab, window, cx);
            }
            (PendingDialog::CloseRunning(tab), Some(choice @ ("terminate" | "disconnect"))) => {
                let terminate = choice == "terminate";
                if checked {
                    self.run_settings.close_running = if terminate {
                        CloseRunning::Terminate
                    } else {
                        CloseRunning::Disconnect
                    };
                }
                self.run_window
                    .update(cx, |w, cx| w.close(tab, terminate, cx));
            }
            (PendingDialog::FileSync(pending), button) => {
                self.sync_dialog_closed(pending, button, window, cx)
            }
            _ => {}
        }
        // A file dialog that waited for this one.
        self.show_sync_dialog(window, cx);
        cx.notify();
    }

    /// The open dialog's focus.
    pub fn dialog_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.dialog.as_ref().map(|d| d.dialog.focus_handle(cx))
    }

    /// The open popup menu of the run widget or a Run tab.
    pub(super) fn render_run_menu(&self, window: &Window) -> Option<AnyElement> {
        let open = self.run_menu.as_ref()?;
        Some(ui::menu::overlay(&open.menu, open.position, window))
    }

    /// Stops every process before the application quits.
    pub(super) fn kill_runs(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.run_window.update(cx, |w, _| w.kill_all());
        Task::ready(())
    }
}
