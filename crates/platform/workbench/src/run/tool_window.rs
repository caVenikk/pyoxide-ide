//! The Run tool window: a tab per run with its process and console, the
//! run toolbar (Rerun, Stop, More) and the console toolbar, as the
//! reference IDE's `RunContentManagerImpl`, `RunContentBuilder` and
//! `ConsoleViewImpl` show them in the New UI.

use std::sync::atomic::Ordering;

use console::{ContentKind, LinkTarget};
use futures::StreamExt as _;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render,
    SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use process::{ExitStatus, ProcessEvent, ProcessHandle};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::commands::*;
use super::configurations::{RunConfigs, RunConfigsEvent};
use super::console_view::{ConsoleEvent, ConsoleView};
use super::{LaunchKey, RunHook, RunRequest, RunSpec};
use crate::chrome;

/// Size of the buttons of the run and console toolbars and of their icons.
const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;

/// What the Run tool window asks of the workbench.
#[derive(Debug, Clone, PartialEq)]
pub enum RunToolWindowEvent {
    /// The Hide button was pressed.
    Hide,
    /// The last tab was closed: the tool window hides and its stripe
    /// button goes away (`setToHideOnEmptyContent`).
    Emptied,
    /// A console link was followed.
    OpenLink(LinkTarget),
    /// A message for the status bar.
    Message(String),
    /// A notice for the status bar.
    Notice(String),
    /// A balloon (the import of run configurations).
    Notify(ui::notification::Notification),
    /// The run configurations' files were written or removed.
    FilesWritten(Vec<std::path::PathBuf>),
    /// Closing a tab whose process runs: the workbench asks first.
    ConfirmClose(usize),
    /// The tab context menu was asked for at a window position.
    TabMenu(usize, Point<Pixels>),
    /// The run toolbar's More menu was asked for at a window position.
    MoreMenu(Point<Pixels>),
}

/// Where a tab's process is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStatus {
    /// The command line is being prepared.
    Starting,
    Running,
    /// The process ended (or did not start: `None`).
    Ended(Option<ExitStatus>),
}

/// Stops a process off the UI thread (signalling the process tree walks
/// the process table) and redraws the window once it is sent, so that the
/// Stop button turns into Kill.
fn stop_process(process: ProcessHandle, cx: &mut Context<RunToolWindow>) {
    let stop = cx.background_spawn(async move {
        let _ = process.stop();
    });
    cx.spawn(async move |this, cx| {
        stop.await;
        let _ = this.update(cx, |_, cx| cx.notify());
    })
    .detach();
}

/// Writes a file to a process's standard input and closes it, off the UI
/// thread.
fn feed_input(process: ProcessHandle, file: std::path::PathBuf, cx: &mut Context<RunToolWindow>) {
    cx.background_spawn(async move {
        if let Ok(bytes) = std::fs::read(&file) {
            let _ = process.write_stdin(&bytes);
        }
        process.close_stdin();
    })
    .detach();
}

/// One run.
pub struct RunTab {
    id: u64,
    title: String,
    icon: IconName,
    /// What was run: Rerun runs it again.
    request: RunRequest,
    console: Entity<ConsoleView>,
    process: Option<ProcessHandle>,
    status: TabStatus,
    pinned: bool,
    /// Run again in this tab once the process ended (Rerun, Stop and
    /// Rerun).
    restart: bool,
    _watch: Option<Task<()>>,
    _subscription: Subscription,
}

impl RunTab {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn request(&self) -> &RunRequest {
        &self.request
    }

    pub fn status(&self) -> TabStatus {
        self.status
    }

    pub fn console(&self) -> &Entity<ConsoleView> {
        &self.console
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// The process runs.
    pub fn is_alive(&self) -> bool {
        self.status == TabStatus::Running && self.process.as_ref().is_some_and(|p| p.is_running())
    }

    /// The first Stop was sent and the process still runs: the next one
    /// kills it.
    pub fn is_stopping(&self) -> bool {
        self.is_alive() && self.process.as_ref().is_some_and(|p| p.is_stopping())
    }

    fn is_terminated(&self) -> bool {
        matches!(self.status, TabStatus::Ended(_))
    }

    /// The process id while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.is_alive()
            .then(|| self.process.as_ref().map(|p| p.pid()))
            .flatten()
    }
}

/// The Run tool window's content.
pub struct RunToolWindow {
    focus: FocusHandle,
    provider: Option<RunHook>,
    tabs: Vec<RunTab>,
    selected: Option<usize>,
    /// Launches whose command lines are being prepared.
    launching: Vec<LaunchKey>,
    /// The project's run configurations.
    configs: Entity<RunConfigs>,
    /// The Run/Debug Configurations dialog while it is open, its window
    /// and the workbench's subscription.
    edit_dialog: Option<(
        Entity<super::edit_dialog::EditDialog>,
        gpui::AnyWindowHandle,
        Subscription,
    )>,
    next_id: u64,
    /// The main window (a rerun after the process ended starts there).
    window_handle: gpui::AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RunToolWindowEvent> for RunToolWindow {}

impl RunToolWindow {
    pub fn new(provider: Option<RunHook>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let configs = cx.new(|_| RunConfigs::new(provider.as_ref()));
        let subscriptions = vec![
            cx.observe(&configs, |_, _, cx| cx.notify()),
            cx.subscribe(&configs, |_, _, event: &RunConfigsEvent, cx| match event {
                RunConfigsEvent::Notice(text) => cx.emit(RunToolWindowEvent::Notice(text.clone())),
                RunConfigsEvent::Notify(n) => cx.emit(RunToolWindowEvent::Notify(n.clone())),
                RunConfigsEvent::FilesWritten(paths) => {
                    cx.emit(RunToolWindowEvent::FilesWritten(paths.clone()))
                }
            }),
        ];
        Self {
            focus: cx.focus_handle(),
            provider,
            tabs: Vec::new(),
            selected: None,
            launching: Vec::new(),
            configs,
            edit_dialog: None,
            next_id: 0,
            window_handle: window.window_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The project's run configurations.
    pub fn configs(&self) -> &Entity<RunConfigs> {
        &self.configs
    }

    /// The Run/Debug Configurations dialog while it is open.
    pub fn edit_dialog(&self) -> Option<&Entity<super::edit_dialog::EditDialog>> {
        self.edit_dialog.as_ref().map(|(d, _, _)| d)
    }

    /// The window of the open dialog.
    pub fn edit_dialog_window(&self) -> Option<gpui::AnyWindowHandle> {
        self.edit_dialog.as_ref().map(|(_, w, _)| *w)
    }

    /// Keeps the open dialog (`None` forgets it); returns the window of the
    /// one it had.
    pub fn set_edit_dialog(
        &mut self,
        dialog: Option<(
            Entity<super::edit_dialog::EditDialog>,
            gpui::AnyWindowHandle,
            Subscription,
        )>,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyWindowHandle> {
        let old = std::mem::replace(&mut self.edit_dialog, dialog);
        cx.notify();
        old.map(|(_, window, _)| window)
    }

    /// A project was opened: its run configurations load.
    pub fn project_opened(&mut self, root: &std::path::Path, cx: &mut Context<Self>) {
        let root = root.to_owned();
        self.configs.update(cx, |c, cx| c.open(root, cx));
    }

    pub fn provider(&self) -> Option<&RunHook> {
        self.provider.as_ref()
    }

    pub fn tabs(&self) -> &[RunTab] {
        &self.tabs
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_tab(&self) -> Option<&RunTab> {
        self.selected.and_then(|i| self.tabs.get(i))
    }

    /// A process of the window runs (the stripe button's live dot).
    pub fn any_alive(&self) -> bool {
        self.tabs.iter().any(|t| t.is_alive())
    }

    /// Tabs whose process runs, in order.
    pub fn alive_tabs(&self) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|i| self.tabs[*i].is_alive())
            .collect()
    }

    /// The tab whose process runs `file` (a Current File run).
    pub fn running_file(&self, file: &std::path::Path) -> Option<usize> {
        self.running(&LaunchKey::File(file.to_owned()))
    }

    /// The tab whose process runs what `key` names.
    pub fn running(&self, key: &LaunchKey) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.is_alive() && t.request.launch_key() == *key)
    }

    /// A launch of `file` is being prepared.
    pub fn is_launching(&self, file: &std::path::Path) -> bool {
        self.is_launching_key(&LaunchKey::File(file.to_owned()))
    }

    /// A launch of what `key` names is being prepared.
    pub fn is_launching_key(&self, key: &LaunchKey) -> bool {
        self.launching.iter().any(|k| k == key)
    }

    /// Where the focus goes in the tool window: the selected console.
    pub fn content_focus(&self, cx: &App) -> FocusHandle {
        match self.selected_tab() {
            Some(t) => t.console.focus_handle(cx),
            None => self.focus.clone(),
        }
    }

    // ---- running ---------------------------------------------------------------

    /// Prepares the command line of `request` with the product's provider
    /// and runs it: in `tab` (Rerun), or in a tab chosen by the reference's
    /// reuse rules.
    pub fn launch(
        &mut self,
        request: RunRequest,
        tab: Option<usize>,
        saved: Vec<Task<()>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.provider.clone() else {
            saved.into_iter().for_each(Task::detach);
            return;
        };
        let tab_id = tab.and_then(|i| self.tabs.get(i)).map(|t| t.id);
        if let Some(i) = tab {
            self.tabs[i].status = TabStatus::Starting;
        }
        let key = request.launch_key();
        self.launching.push(key.clone());
        let prepare = match &request.configuration {
            Some(c) if c.type_id == run_config::shell::TYPE_ID => {
                super::prepare_shell_script(request.clone(), cx)
            }
            Some(_) => provider.0.prepare_configuration(request.clone(), cx),
            None => provider.0.prepare(request.clone(), cx),
        };
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let spec = prepare.await;
            // The files the run reads are written first.
            for write in saved {
                write.await;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(i) = this.launching.iter().position(|k| *k == key) {
                    this.launching.remove(i);
                }
                match spec {
                    Ok(spec) => {
                        let index = match tab_id.and_then(|id| this.index_of(id)) {
                            Some(i) => i,
                            None => this.tab_for(&spec.title, request.clone(), window, cx),
                        };
                        this.start(index, request, spec, cx);
                    }
                    Err(message) => {
                        if let Some(i) = tab_id.and_then(|id| this.index_of(id)) {
                            this.tabs[i].status = TabStatus::Ended(None);
                        }
                        let name = request.name();
                        cx.emit(RunToolWindowEvent::Message(format!(
                            "Error running '{name}': {message}"
                        )));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The tab a new run of `title` goes to: a terminated, unpinned tab,
    /// the selected one first, one with the same name before others
    /// (`chooseReuseContentForDescriptor`); otherwise a new tab.
    fn tab_for(
        &mut self,
        title: &str,
        request: RunRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let mut order: Vec<usize> = (0..self.tabs.len()).collect();
        if let Some(s) = self.selected {
            order.retain(|i| *i != s);
            order.insert(0, s);
        }
        let reusable = |t: &RunTab| !t.pinned && t.is_terminated();
        let reuse = order
            .iter()
            .copied()
            .find(|i| reusable(&self.tabs[*i]) && self.tabs[*i].title == title)
            .or_else(|| order.iter().copied().find(|i| reusable(&self.tabs[*i])));
        let index = match reuse {
            Some(i) => i,
            None => {
                let console = cx.new(|cx| ConsoleView::new(window, cx));
                let subscription =
                    cx.subscribe(&console, |_, _, event: &ConsoleEvent, cx| match event {
                        ConsoleEvent::OpenLink(target) => {
                            cx.emit(RunToolWindowEvent::OpenLink(target.clone()))
                        }
                    });
                self.next_id += 1;
                self.tabs.push(RunTab {
                    id: self.next_id,
                    title: title.to_owned(),
                    icon: IconName::RUN,
                    request: request.clone(),
                    console,
                    process: None,
                    status: TabStatus::Starting,
                    pinned: false,
                    restart: false,
                    _watch: None,
                    _subscription: subscription,
                });
                self.tabs.len() - 1
            }
        };
        self.selected = Some(index);
        index
    }

    /// Starts `spec` in tab `index`: the command line as the first line,
    /// then the process's output, then the exit line.
    fn start(&mut self, index: usize, request: RunRequest, spec: RunSpec, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[index];
        tab.title = spec.title.clone();
        tab.icon = spec.icon;
        tab.request = request;
        tab.restart = false;
        tab.process = None;
        let console = tab.console.clone();
        console.update(cx, |c, cx| {
            c.reset(spec.link_filters.clone(), cx);
            c.print_system(&format!("{}\n", spec.display), cx);
        });
        if let Some(warning) = &spec.warning {
            cx.emit(RunToolWindowEvent::Message(warning.clone()));
        }
        let (handle, events) = match process::spawn(&spec.command) {
            Ok(started) => started,
            Err(e) => {
                let message = format!(
                    "Cannot run program \"{}\": {e}",
                    spec.command.program.display()
                );
                console.update(cx, |c, cx| {
                    c.model()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_str(ContentKind::Error, &format!("{message}\n"));
                    c.flush(cx);
                });
                let tab = &mut self.tabs[index];
                tab.status = TabStatus::Ended(None);
                cx.emit(RunToolWindowEvent::Message(format!(
                    "Error running '{}': {message}",
                    spec.title
                )));
                cx.notify();
                return;
            }
        };
        match &spec.input_file {
            // "Redirect input from": the file is the process's input
            // (`PythonScriptCommandLineState`, "Input is being redirected
            // from …"); the console takes none.
            Some(file) => {
                console.update(cx, |c, cx| {
                    c.print_system(
                        &format!("Input is being redirected from {}\n", file.display()),
                        cx,
                    )
                });
                feed_input(handle.clone(), file.clone(), cx);
            }
            None => console.update(cx, |c, _| c.set_input(Some(handle.clone()))),
        }
        // The model is fed off the UI thread (link filters may look at the
        // file system); the view takes the changes once per frame.
        let (model, dirty) = {
            let c = console.read(cx);
            (c.model(), c.dirty_flag())
        };
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Option<ExitStatus>>();
        cx.background_spawn(async move {
            while let Ok(event) = events.recv_async().await {
                let mut model = model.lock().unwrap_or_else(|e| e.into_inner());
                match event {
                    ProcessEvent::Stdout(bytes) => model.push(ContentKind::Normal, &bytes),
                    ProcessEvent::Stderr(bytes) => model.push(ContentKind::Error, &bytes),
                    ProcessEvent::Exited(status) => {
                        model.finish();
                        model.push_str(
                            ContentKind::System,
                            &format!("\n{}\n", status.finished_message()),
                        );
                        drop(model);
                        let _ = tx.unbounded_send(Some(status));
                        return;
                    }
                }
                drop(model);
                if !dirty.swap(true, Ordering::AcqRel) {
                    let _ = tx.unbounded_send(None);
                }
            }
        })
        .detach();
        let id = self.tabs[index].id;
        let window_handle = self.window_handle;
        let watch = cx.spawn(async move |this, cx| {
            while let Some(message) = rx.next().await {
                match message {
                    None => {
                        let _ = console.update(cx, |c, cx| c.request_flush(cx));
                    }
                    Some(status) => {
                        let restart = this
                            .update(cx, |this, cx| this.exited(id, status, cx))
                            .ok()
                            .flatten();
                        // Rerun, or Stop and Rerun: the same tab again.
                        if let Some((index, request)) = restart {
                            let _ = cx.update_window(window_handle, |_, window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.launch(request, Some(index), Vec::new(), window, cx)
                                });
                            });
                        }
                        break;
                    }
                }
            }
        });
        let tab = &mut self.tabs[index];
        tab.process = Some(handle);
        tab.status = TabStatus::Running;
        tab._watch = Some(watch);
        cx.notify();
    }

    /// The process of tab `id` ended. Returns what to run again in it
    /// (Rerun, Stop and Rerun).
    fn exited(
        &mut self,
        id: u64,
        status: ExitStatus,
        cx: &mut Context<Self>,
    ) -> Option<(usize, RunRequest)> {
        let index = self.index_of(id)?;
        let tab = &mut self.tabs[index];
        tab.status = TabStatus::Ended(Some(status));
        let console = tab.console.clone();
        let restart = std::mem::take(&mut tab.restart);
        let request = tab.request.clone();
        console.update(cx, |c, cx| {
            c.set_input(None);
            c.flush(cx);
        });
        cx.notify();
        restart.then_some((index, request))
    }

    /// Stops the process of a tab: the first time gracefully, then by
    /// killing it (`StopAction`, `KillableProcessHandler`).
    pub fn stop(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.restart = false;
        if tab.is_alive()
            && let Some(p) = &tab.process
        {
            stop_process(p.clone(), cx);
        }
        cx.notify();
    }

    /// Runs a tab's request again in the same tab, stopping its process
    /// first (`FakeRerunAction`): it starts once the old one ended.
    pub fn rerun(
        &mut self,
        index: usize,
        saved: Vec<Task<()>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Writes that are not waited for still finish.
        let detach = |saved: Vec<Task<()>>| saved.into_iter().for_each(Task::detach);
        let Some(tab) = self.tabs.get_mut(index) else {
            detach(saved);
            return;
        };
        if tab.status == TabStatus::Starting {
            detach(saved);
            return;
        }
        if tab.is_alive() {
            if !tab.is_stopping()
                && let Some(p) = &tab.process
            {
                stop_process(p.clone(), cx);
            }
            tab.restart = true;
            detach(saved);
            cx.notify();
            return;
        }
        let request = tab.request.clone();
        self.selected = Some(index);
        self.launch(request, Some(index), saved, window, cx);
    }

    /// Rerun is available: not while the process starts or terminates.
    pub fn can_rerun(&self, index: usize) -> bool {
        self.tabs
            .get(index)
            .is_some_and(|t| t.status != TabStatus::Starting && !t.is_stopping() && !t.restart)
    }

    /// Asks to close a tab; a running process is confirmed by the
    /// workbench first.
    pub fn request_close(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if tab.is_alive() {
            cx.emit(RunToolWindowEvent::ConfirmClose(index));
        } else {
            self.close(index, false, cx);
        }
    }

    /// Closes a tab; `terminate` stops its process, otherwise a running
    /// process is left alone (Disconnect).
    pub fn close(&mut self, index: usize, terminate: bool, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        if terminate
            && tab.is_alive()
            && let Some(p) = &tab.process
        {
            stop_process(p.clone(), cx);
        }
        self.selected = match self.selected {
            _ if self.tabs.is_empty() => None,
            Some(s) if s > index => Some(s - 1),
            Some(s) if s == index => Some(index.saturating_sub(1).min(self.tabs.len() - 1)),
            other => other,
        };
        if self.tabs.is_empty() {
            cx.emit(RunToolWindowEvent::Emptied);
        }
        cx.notify();
    }

    /// Stops every process (the application quits).
    pub fn kill_all(&mut self) {
        for tab in &self.tabs {
            if let Some(p) = &tab.process
                && p.is_running()
            {
                let _ = p.kill();
            }
        }
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.selected = Some(index);
            cx.notify();
        }
    }

    pub fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.pinned = !tab.pinned;
            cx.notify();
        }
    }

    fn selected_console(&self) -> Option<Entity<ConsoleView>> {
        self.selected_tab().map(|t| t.console.clone())
    }

    /// Up / Down the Stack Trace in the selected console.
    pub fn go_to_link(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(console) = self.selected_console() {
            console.update(cx, |c, cx| c.go_to_link(forward, cx));
        }
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        if let Some(console) = self.selected_console() {
            console.update(cx, |c, cx| c.scroll_to_end(cx));
        }
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if let Some(console) = self.selected_console() {
            console.update(cx, |c, cx| c.clear(cx));
        }
    }

    pub fn send_eof(&mut self, cx: &mut Context<Self>) {
        if let Some(console) = self.selected_console() {
            console.update(cx, |c, _| c.send_eof());
        }
    }

    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(console) = self.selected_console() {
            console.update(cx, |c, cx| c.type_text(text, cx));
        }
    }

    /// The state reported by `run.state`.
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        serde_json::json!({
            "selected": self.selected,
            "launching": self.launching.iter().map(|k| match k {
                LaunchKey::File(f) => f.display().to_string(),
                LaunchKey::Configuration(c) => c.name.clone(),
            }).collect::<Vec<_>>(),
            "tabs": self.tabs.iter().enumerate().map(|(i, t)| {
                let (status, exit) = match t.status {
                    TabStatus::Starting => ("starting", None),
                    TabStatus::Running if t.is_stopping() => ("stopping", None),
                    TabStatus::Running => ("running", None),
                    TabStatus::Ended(status) => ("terminated", status),
                };
                serde_json::json!({
                    "title": t.title,
                    "file": t.request.file.display().to_string(),
                    "status": status,
                    "exit_code": exit.map(|s| s.exit_code()),
                    "exit_text": exit.map(|s| s.exit_code_text()),
                    "pid": t.pid(),
                    "pinned": t.pinned,
                    "can_rerun": self.can_rerun(i),
                    "console": t.console.read(cx).state_json(cx),
                })
            }).collect::<Vec<_>>(),
        })
    }

    // ---- drawing ---------------------------------------------------------------

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let theme = cx.theme();
        let selected = self.selected == Some(index);
        let hover = theme.color(UiRole::TabHover);
        let text = if selected {
            theme.color(UiRole::TabActiveText)
        } else {
            theme.color(UiRole::TabInactiveText)
        };
        let group: SharedString = format!("run-tab-{index}").into();
        let icon = chrome::badged_icon(
            tab.icon,
            px(ICON),
            tab.is_alive().then(|| theme.color(UiRole::Success)),
            theme.color(UiRole::ToolWindow),
        );
        let icon = div()
            .flex_none()
            .when(tab.is_terminated(), |d| d.opacity(0.5))
            .child(icon);
        let close = div()
            .id(("run-tab-close", index))
            .debug_selector(move || format!("run-tab-close-{index}"))
            .flex_none()
            .size(px(16.0))
            .rounded(px(3.0))
            .when(!selected, |d| {
                d.invisible().group_hover(group.clone(), |s| s.visible())
            })
            .hover(move |s| s.bg(hover))
            .child(Icon::new(IconName::CLOSE_SMALL))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(CloseTab { index: Some(index) }), cx);
            }));
        let tooltip = tab.pid().map(|pid| format!("Process ID: {pid}"));
        div()
            .id(("run-tab", index))
            .debug_selector(move || format!("run-tab-{index}"))
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
                    .border_color(theme.color(UiRole::TabUnderline))
            })
            .when(!selected, |d| d.border_color(gpui::transparent_black()))
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .child(icon)
            .child(
                div()
                    .whitespace_nowrap()
                    .text_color(text)
                    .child(tab.title.clone()),
            )
            .child(close)
            .when_some(tooltip, |d, t| d.tooltip(Tooltip::text(t, None)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select(index, cx);
                window.focus(&this.content_focus(cx));
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.select(index, cx);
                    cx.emit(RunToolWindowEvent::TabMenu(index, e.position));
                }),
            )
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
                    .child("Run"),
            )
            .child(
                div()
                    .id("run-tabs")
                    .debug_selector(|| "run-tabs".into())
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
                    "run-options",
                    IconName::MORE_VERTICAL,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Options", None)),
            )
            .child(
                icon_button("run-hide", IconName::HIDE, px(BUTTON), px(ICON), false, cx)
                    .tooltip(Tooltip::text("Hide", None))
                    .on_click(
                        cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(RunToolWindowEvent::Hide)),
                    ),
            )
    }

    /// A toolbar button that dispatches a command; dimmed when disabled.
    #[allow(clippy::too_many_arguments)]
    fn tool_button(
        &self,
        id: &'static str,
        icon: IconName,
        title: SharedString,
        shortcut: Option<SharedString>,
        enabled: bool,
        action: Box<dyn gpui::Action>,
        cx: &App,
    ) -> AnyElement {
        icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
            .debug_selector(move || id.into())
            .tooltip(Tooltip::text(title, shortcut))
            .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
            .when(enabled, |d| {
                d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
            .into_any_element()
    }

    fn render_toolbar(
        &self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab = &self.tabs[index];
        let keys = |action: &dyn gpui::Action| -> Option<SharedString> {
            ui::keys::shortcut_in(action, &self.content_focus(cx), window).map(Into::into)
        };
        let title = tab.title.clone();
        let stopping = tab.is_stopping();
        let (stop_icon, stop_title) = if stopping {
            (IconName::KILL_PROCESS, SharedString::from("Kill process"))
        } else {
            (IconName::STOP, format!("Stop '{title}'").into())
        };
        let rerun = self.tool_button(
            "run-rerun",
            IconName::RERUN,
            format!("Rerun '{title}'").into(),
            keys(&Rerun),
            self.can_rerun(index),
            Box::new(Rerun),
            cx,
        );
        let stop = self.tool_button(
            "run-stop",
            stop_icon,
            stop_title,
            keys(&Stop { tab: None }),
            tab.is_alive(),
            Box::new(Stop { tab: Some(index) }),
            cx,
        );
        let more = icon_button(
            "run-more-tab",
            IconName::MORE_VERTICAL,
            px(BUTTON),
            px(ICON),
            false,
            cx,
        )
        .tooltip(Tooltip::text("More", None))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, e: &MouseDownEvent, _, cx| {
                cx.emit(RunToolWindowEvent::MoreMenu(e.position))
            }),
        );
        div()
            .id("run-toolbar")
            .debug_selector(|| "run-toolbar".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(32.0))
            .px(px(6.0))
            .gap(px(2.0))
            .child(rerun)
            .child(stop)
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h(px(16.0))
                    .mx(px(3.0))
                    .bg(cx.theme().color(UiRole::Border)),
            )
            .child(more)
    }

    fn render_console_toolbar(
        &self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab = &self.tabs[index];
        let console = tab.console.read(cx);
        let has_content = console.has_content(cx);
        let has_links = console.has_links();
        let keys = |action: &dyn gpui::Action| -> Option<SharedString> {
            ui::keys::shortcut_in(action, &self.content_focus(cx), window).map(Into::into)
        };
        let buttons = vec![
            self.tool_button(
                "run-console-up",
                IconName::ARROW_UP,
                "Up the Stack Trace".into(),
                keys(&PreviousLink),
                has_links,
                Box::new(PreviousLink),
                cx,
            ),
            self.tool_button(
                "run-console-down",
                IconName::ARROW_DOWN,
                "Down the Stack Trace".into(),
                keys(&NextLink),
                has_links,
                Box::new(NextLink),
                cx,
            ),
            // Soft wraps are not drawn by the editor yet.
            self.tool_button(
                "run-console-soft-wrap",
                IconName::SOFT_WRAP,
                "Soft-Wrap".into(),
                None,
                false,
                Box::new(ScrollToEnd),
                cx,
            ),
            self.tool_button(
                "run-console-scroll-to-end",
                IconName::SCROLL_DOWN,
                "Scroll to End".into(),
                keys(&ScrollToEnd),
                true,
                Box::new(ScrollToEnd),
                cx,
            ),
            // Printing is not available.
            self.tool_button(
                "run-console-print",
                IconName::PRINT,
                "Print…".into(),
                None,
                false,
                Box::new(ScrollToEnd),
                cx,
            ),
            self.tool_button(
                "run-console-clear",
                IconName::DELETE,
                "Clear All".into(),
                keys(&Clear),
                has_content,
                Box::new(Clear),
                cx,
            ),
        ];
        div()
            .id("run-console-toolbar")
            .debug_selector(|| "run-console-toolbar".into())
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .w(px(32.0))
            .pt(px(2.0))
            .gap(px(2.0))
            .children(buttons)
    }

    /// The reference's empty state of the Run tool window
    /// (`ActivateRunToolWindowAction.setupEmptyText`), without the help
    /// link.
    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let link = theme.color(UiRole::TextLink);
        div()
            .id("run-empty")
            .debug_selector(|| "run-empty".into())
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .text_color(muted)
                    .child("To run your code, do one of the following:")
                    .child("\u{2014} Click the Run icon in the editor gutter")
                    .child("\u{2014} Select \"Run\u{2026}\" in the editor context menu")
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .child("\u{2014} Launch a\u{00a0}")
                            .child(div().text_color(link).child("run configuration")),
                    ),
            )
    }
}

impl Focusable for RunToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RunToolWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx).into_any_element();
        let body = match self.selected {
            Some(index) if index < self.tabs.len() => {
                let toolbar = self.render_toolbar(index, window, cx).into_any_element();
                let console_toolbar = self
                    .render_console_toolbar(index, window, cx)
                    .into_any_element();
                let console = self.tabs[index].console.clone();
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(toolbar)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_h(px(0.0))
                            .child(console_toolbar)
                            .child(
                                div()
                                    .id("run-console")
                                    .debug_selector(|| "run-console".into())
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h_full()
                                    .child(console),
                            ),
                    )
                    .into_any_element()
            }
            _ => self.render_empty(cx).into_any_element(),
        };
        div()
            .id("run-tool-window-content")
            .key_context("RunToolWindow")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(body)
    }
}
