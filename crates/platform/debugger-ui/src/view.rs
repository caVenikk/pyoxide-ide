//! The window's controller of the project's debugger (the reference's
//! `XDebuggerManager` UI side): it opens the [`Debugger`] of a project off
//! the UI thread, follows its events, keeps the breakpoints of the editor
//! tabs in their gutters ([`crate::breakpoints::gutter`]), shows the
//! properties popup and the Breakpoints dialog, and runs the breakpoint
//! commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use debugger::{Debugger, DebuggerEvent, ExceptionBreakpoint, SuspendPolicy, Toggled};
use editor::Editor;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EntityId, EventEmitter, InteractiveElement,
    Task, Window, WindowHandle,
};
use serde_json::{Value as Json, json};

use crate::breakpoints::commands::{
    AddExceptionBreakpoint, EditBreakpoint, MuteBreakpoints, ToggleBreakpoint,
    ToggleBreakpointEnabled, ToggleTemporaryBreakpoint, ViewBreakpoints,
};
use crate::breakpoints::dialog::BreakpointsDialog;
use crate::breakpoints::gutter::{Gutter, Tooltip};
use crate::breakpoints::popup::OpenPopup;
use crate::breakpoints::{presentation, storage_path};
use crate::hook::DebugHook;
use crate::session::DebugToolWindow;
use crate::session::commands::*;
use crate::session::execution_point::ExecutionPoint;

/// How the debugger UI gets the syntax colours of a file for the
/// Breakpoints dialog's source preview.
pub type Highlighters = Rc<dyn Fn(&Path) -> Option<Box<dyn editor::Highlighter>>>;

/// What the debugger UI asks of the window.
#[derive(Clone)]
pub enum DebuggerViewEvent {
    /// Open a file at a line and a column (from 1; 0: the line's start):
    /// Jump to Source of the Breakpoints dialog and of the variables view.
    OpenAt {
        path: PathBuf,
        line: u32,
        column: u32,
    },
    /// Show a line (from 1) where the program is suspended, without
    /// bringing the window to the front.
    Navigate {
        path: PathBuf,
        line: u32,
    },
    /// Show the Debug tool window (and move the focus into it).
    ShowToolWindow {
        activate: bool,
    },
    HideToolWindow,
    /// The last session's tab was closed: the tool window hides and its
    /// stripe button goes.
    Emptied,
    /// A message for the status bar.
    Message(String),
    /// A balloon.
    Notify(ui::notification::Notification),
    /// A console link was followed.
    OpenLink(console::LinkTarget),
    /// The toolbar's More menu, at a window position or under its button.
    MoreMenu(Option<gpui::Point<gpui::Pixels>>),
    /// A Rerun waits for the window ([`DebuggerView::restart_pending`]).
    Restart,
    /// Show Variable Values in Editor was switched: the window stores the
    /// application's option, and every window follows it
    /// ([`DebuggerView::set_inline_values_shown`]).
    ShowValuesInline(bool),
    /// The editors' own popups under the mouse are off while the program
    /// is suspended, the debugger shows values there
    /// (`EditorMouseHoverPopupControl.disablePopups`), and on again after.
    MouseHoverPopups(bool),
    /// Show the editor's own popup (documentation and problems) at a char
    /// offset, where the debugger has no value to show
    /// (`EditorMouseHoverPopupManager.showInfoTooltip`).
    ShowInfoTooltip {
        editor: Entity<Editor>,
        offset: usize,
    },
}

/// The debugger UI of a window.
pub struct DebuggerView {
    pub(crate) hook: Option<DebugHook>,
    pub(crate) debugger: Option<Debugger>,
    /// Changes with every project, so late results of the previous one are
    /// dropped.
    generation: u64,
    _load: Option<Task<()>>,
    _events: Option<Task<()>>,
    /// The gutters of the editor tabs, by editor.
    pub(crate) gutters: HashMap<EntityId, Gutter>,
    pub(crate) popup: Option<OpenPopup>,
    pub(crate) tooltip: Option<Tooltip>,
    pub(crate) dialog: Option<WindowHandle<BreakpointsDialog>>,
    /// Conditions entered before, the latest first (`breakpointCondition`
    /// history).
    pub(crate) history: Vec<String>,
    pub(crate) highlighters: Option<Highlighters>,
    /// How long the mouse stays over a line number before its tooltip
    /// shows.
    pub(crate) tooltip_delay: Duration,
    /// Windows and popups take the focus when they open (off for windows
    /// opened in the background).
    pub(crate) activate: bool,
    /// The Debug tool window's content.
    pub(crate) tool_window: Entity<DebugToolWindow>,
    /// Where the selected session is suspended, as the editors show it.
    pub(crate) execution: Option<ExecutionPoint>,
    /// The stop the editor was taken to last.
    pub(crate) navigated: Option<ExecutionPoint>,
    /// Debug runs being prepared, by what the window started.
    pub(crate) starting: Vec<Option<String>>,
    /// A Rerun to start once its session ended.
    pub(crate) pending_restart: Option<crate::session::control::PendingRestart>,
    /// The selected frame's values after the lines.
    pub(crate) inline: crate::inline::InlineValues,
    /// Values under the mouse and their popups.
    pub(crate) hints: crate::value_hint::ValueHints,
    _tool_window_events: gpui::Subscription,
}

impl EventEmitter<DebuggerViewEvent> for DebuggerView {}

/// The reference's initial tooltip delay (`ide.tooltip.initialDelay`).
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

impl DebuggerView {
    /// The debugger UI of a product; without a hook there are no
    /// breakpoints.
    pub fn new(hook: Option<DebugHook>, cx: &mut Context<Self>) -> Self {
        // The changes made last are written before the application quits.
        cx.on_app_quit(|this, _| {
            let debugger = this.debugger.clone();
            async move {
                if let Some(debugger) = debugger {
                    debugger.flush();
                }
            }
        })
        .detach();
        let tool_window = cx.new(DebugToolWindow::new);
        let tool_window_events = cx.subscribe(&tool_window, |this, _, event, cx| {
            this.tool_window_event(event, cx)
        });
        Self {
            hook,
            debugger: None,
            generation: 0,
            _load: None,
            _events: None,
            gutters: HashMap::new(),
            popup: None,
            tooltip: None,
            dialog: None,
            history: Vec::new(),
            highlighters: None,
            tooltip_delay: TOOLTIP_DELAY,
            activate: true,
            tool_window,
            execution: None,
            navigated: None,
            starting: Vec::new(),
            pending_restart: None,
            inline: Default::default(),
            hints: Default::default(),
            _tool_window_events: tool_window_events,
        }
    }

    /// Where the source preview of the Breakpoints dialog gets its syntax
    /// colours.
    pub fn set_highlighters(&mut self, highlighters: Highlighters) {
        self.highlighters = Some(highlighters);
    }

    /// Whether the Breakpoints dialog and the popups of lists take the
    /// focus when they open (`PYOXIDE_BACKGROUND` turns it off).
    pub fn set_activate(&mut self, activate: bool) {
        self.activate = activate;
    }

    /// The tooltip delay (tests show tooltips at once).
    pub fn set_tooltip_delay(&mut self, delay: Duration) {
        self.tooltip_delay = delay;
    }

    /// The project's debugger, once it is open.
    pub fn debugger(&self) -> Option<&Debugger> {
        self.debugger.as_ref()
    }

    /// A project was opened (its window's tabs were closed): its debugger
    /// state is read off the UI thread (and imported once from the
    /// reference), then the gutters of the tabs opened show its
    /// breakpoints.
    pub fn project_opened(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.close_popup(false, window, cx);
        self.close_dialog(cx);
        self.tooltip = None;
        self.gutters.clear();
        // The previous project's sessions end with it.
        while self.tool_window.read(cx).tab_count() > 0 {
            self.close_session(Some(0), cx);
        }
        self.execution = None;
        self.navigated = None;
        if let Some(debugger) = self.debugger.take() {
            drop(debugger.stop_all());
            debugger.flush();
        }
        self._events = None;
        self.refresh_all(cx);
        let Some(hook) = self.hook.clone() else {
            return;
        };
        let generation = self.generation;
        let root = root.to_owned();
        let open = cx.background_spawn(async move { Debugger::open(&root, hook.provider) });
        self._load = Some(cx.spawn(async move |this, cx| {
            let debugger = open.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.loaded(debugger, cx);
                }
            });
        }));
    }

    fn loaded(&mut self, debugger: Debugger, cx: &mut Context<Self>) {
        let events = debugger.subscribe();
        self._events = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv_async().await {
                let mut batch = vec![first];
                batch.extend(events.try_iter());
                if this
                    .update(cx, |this, cx| this.debugger_events(&batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        let (root, provider) = (debugger.root().to_owned(), debugger.provider().clone());
        let highlighters = self.highlighters.clone();
        self.tool_window.update(cx, |w, _| {
            w.root = Some(root);
            w.provider = Some(provider);
            w.highlighters = highlighters;
            w.view_state_loaded(&debugger);
        });
        self.debugger = Some(debugger);
        self.sync_muted(cx);
        self.refresh_all(cx);
        cx.notify();
    }

    /// The Mute button shows whether breakpoints are muted.
    fn sync_muted(&mut self, cx: &mut Context<Self>) {
        let muted = self.debugger.as_ref().is_some_and(Debugger::is_muted);
        self.tool_window.update(cx, |w, cx| {
            if w.muted != muted {
                w.muted = muted;
                cx.notify();
            }
        });
    }

    /// Breakpoints, their verification or the mute state changed: the
    /// gutters, the popup and the dialog show them again; sessions changed:
    /// their tabs, consoles and the execution point follow.
    fn debugger_events(&mut self, events: &[DebuggerEvent], cx: &mut Context<Self>) {
        let mut changed: Vec<debugger::SessionId> = Vec::new();
        for event in events {
            match event {
                DebuggerEvent::SessionChanged(id) | DebuggerEvent::SessionEnded(id) => {
                    if !changed.contains(id) {
                        changed.push(*id);
                    }
                }
                DebuggerEvent::Console(id) => self.attach_console(*id, cx),
                DebuggerEvent::WatchesChanged => self.watches_changed(cx),
                DebuggerEvent::Output {
                    session,
                    category,
                    text,
                } => self.session_output(*session, category, text, cx),
                _ => {}
            }
        }
        for id in changed {
            self.refresh_session(id, cx);
        }
        let shown = events.iter().any(|e| {
            matches!(
                e,
                DebuggerEvent::BreakpointsChanged
                    | DebuggerEvent::MutedChanged(_)
                    | DebuggerEvent::SessionStarted(_)
                    | DebuggerEvent::SessionEnded(_)
            )
        });
        if !shown {
            return;
        }
        self.sync_muted(cx);
        self.refresh_all(cx);
        // A removed breakpoint takes its popup with it.
        if let (Some(p), Some(d)) = (&self.popup, &self.debugger)
            && d.breakpoints().get(p.id).is_none()
        {
            self.popup = None;
        }
        cx.notify();
    }

    // ---- editors --------------------------------------------------------------

    /// An editor tab was opened: its gutter shows the file's breakpoints
    /// and takes clicks on the line numbers.
    pub fn editor_opened(
        &mut self,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = cx.subscribe_in(editor, window, |this, editor, event, window, cx| {
            this.editor_event(editor, event, window, cx)
        });
        self.gutters
            .insert(editor.entity_id(), Gutter::new(editor.clone(), events));
        self.refresh_editor(editor.entity_id(), cx);
        self.show_execution_point_in(editor, cx);
        self.show_inline_values_in(editor, cx);
    }

    /// A tab was closed.
    pub fn editor_closed(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        self.gutters.remove(&editor.entity_id());
        if self
            .hints
            .shown
            .as_ref()
            .is_some_and(|s| s.editor == *editor)
        {
            self.close_value_hint(cx);
        }
        if self.popup.as_ref().is_some_and(|p| p.editor == *editor) {
            self.popup = None;
        }
        if self.tooltip.as_ref().is_some_and(|t| t.editor == *editor) {
            self.tooltip = None;
        }
        cx.notify();
    }

    /// Files or folders were renamed or moved: their breakpoints follow.
    pub fn files_moved(&mut self, moves: &[(PathBuf, PathBuf)], cx: &mut Context<Self>) {
        if let Some(debugger) = &self.debugger {
            for (from, to) in moves {
                debugger.file_moved(from, to);
            }
        }
        self.refresh_all(cx);
    }

    /// The editor of an open file.
    pub(crate) fn editor_of(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        self.gutters
            .values()
            .map(|g| &g.editor)
            .find(|e| e.read(cx).path().is_some_and(|p| p == path))
            .cloned()
    }

    // ---- commands -------------------------------------------------------------

    /// The file and line (from 1) a command acts on: the ones given, or the
    /// caret's in the active editor; with the editor of the file when it is
    /// open.
    pub(crate) fn target(
        &self,
        file: Option<&str>,
        line: Option<u32>,
        active: Option<&Entity<Editor>>,
        cx: &App,
    ) -> Option<(PathBuf, u32, Option<Entity<Editor>>)> {
        match file {
            Some(file) => {
                let debugger = self.debugger.as_ref()?;
                let path = debugger::storage::resolve_path(file, debugger.root())?;
                let editor = self.editor_of(&path, cx);
                let line = match (line, &editor) {
                    (Some(line), _) => line.max(1),
                    (None, Some(editor)) => caret_line(editor, cx),
                    (None, None) => return None,
                };
                Some((path, line, editor))
            }
            None => {
                let editor = active?.clone();
                let path = editor.read(cx).path()?.clone();
                let line = line.unwrap_or_else(|| caret_line(&editor, cx)).max(1);
                Some((path, line, Some(editor)))
            }
        }
    }

    fn toggle(
        &mut self,
        file: Option<&str>,
        line: Option<u32>,
        temporary: bool,
        active: Option<&Entity<Editor>>,
        cx: &mut Context<Self>,
    ) {
        let Some((path, line, editor)) = self.target(file, line, active, cx) else {
            return;
        };
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        // A line of an open file takes a breakpoint where the product's
        // type can be put (`ToggleLineBreakpointAction` is enabled there).
        let existing = debugger.breakpoints().at(&path, line).is_some();
        if !existing
            && let Some(editor) = &editor
            && !self.can_put(editor.entity_id(), line, cx)
        {
            return;
        }
        debugger.toggle_breakpoint(&path, line, temporary);
        self.refresh_path(&path, cx);
    }

    pub(crate) fn toggle_action(
        &mut self,
        a: &ToggleBreakpoint,
        active: Option<&Entity<Editor>>,
        cx: &mut Context<Self>,
    ) {
        let temporary = a.temporary.unwrap_or(false);
        self.toggle(a.file.as_deref(), a.line, temporary, active, cx);
    }

    pub(crate) fn toggle_temporary_action(
        &mut self,
        a: &ToggleTemporaryBreakpoint,
        active: Option<&Entity<Editor>>,
        cx: &mut Context<Self>,
    ) {
        self.toggle(a.file.as_deref(), a.line, true, active, cx);
    }

    pub(crate) fn toggle_enabled_action(
        &mut self,
        a: &ToggleBreakpointEnabled,
        active: Option<&Entity<Editor>>,
        cx: &mut Context<Self>,
    ) {
        let Some((path, line, _)) = self.target(a.file.as_deref(), a.line, active, cx) else {
            return;
        };
        self.toggle_enabled_at(&path, line, cx);
    }

    pub(crate) fn toggle_enabled_at(&mut self, path: &Path, line: u32, cx: &mut Context<Self>) {
        let Some(debugger) = &self.debugger else {
            return;
        };
        if let Some(id) = debugger.breakpoints().at(path, line).map(|b| b.id) {
            debugger.update_breakpoint(id, |b| b.enabled = !b.enabled);
            self.refresh_path(path, cx);
        }
    }

    pub(crate) fn edit_action(
        &mut self,
        a: &EditBreakpoint,
        active: Option<&Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, line, editor)) = self.target(a.file.as_deref(), a.line, active, cx) else {
            if a.file.is_none() {
                self.open_dialog(None, window, cx);
            }
            return;
        };
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        let Some(id) = debugger.breakpoints().at(&path, line).map(|b| b.id) else {
            // No breakpoint at the caret: the dialog (the shortcut is
            // View Breakpoints' too).
            if a.file.is_none() {
                self.open_dialog(None, window, cx);
            }
            return;
        };
        let changes = a.enabled.is_some()
            || a.suspend.is_some()
            || a.condition.is_some()
            || a.log_message.is_some()
            || a.log_stack.is_some()
            || a.log_expression.is_some();
        if !changes {
            if let Some(editor) = editor {
                self.open_popup(&editor, id, window, cx);
            }
            return;
        }
        let suspend = a.suspend.as_deref().and_then(SuspendPolicy::parse);
        let text = |t: &Option<String>| t.clone().filter(|t| !t.trim().is_empty());
        debugger.update_breakpoint(id, |b| {
            if let Some(enabled) = a.enabled {
                b.enabled = enabled;
            }
            if let Some(suspend) = suspend {
                b.suspend = suspend;
            }
            if a.condition.is_some() {
                b.condition = text(&a.condition);
            }
            if let Some(log) = a.log_message {
                b.log_message = log;
            }
            if let Some(log) = a.log_stack {
                b.log_stack = log;
            }
            if a.log_expression.is_some() {
                b.log_expression = text(&a.log_expression);
            }
        });
        if let Some(condition) = text(&a.condition) {
            self.remember_condition(&condition);
        }
        self.refresh_path(&path, cx);
    }

    pub(crate) fn mute_action(&mut self, a: &MuteBreakpoints, cx: &mut Context<Self>) {
        if let Some(debugger) = &self.debugger {
            debugger.set_muted(a.muted.unwrap_or(!debugger.is_muted()));
            self.refresh_all(cx);
        }
    }

    pub(crate) fn add_exception_action(
        &mut self,
        a: &AddExceptionBreakpoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match a.class.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(class) => {
                self.add_exception_breakpoint(class, cx);
            }
            None => {
                self.open_dialog(None, window, cx);
                if let Some(dialog) = &self.dialog {
                    let _ = dialog.update(cx, |d, window, cx| d.choose_class(window, cx));
                }
            }
        }
    }

    /// Adds an exception breakpoint of the product's type for a class.
    pub(crate) fn add_exception_breakpoint(
        &mut self,
        class: &str,
        cx: &mut Context<Self>,
    ) -> Option<debugger::BreakpointId> {
        let debugger = self.debugger.as_ref()?;
        let kind = debugger.provider().exception_breakpoint_type()?;
        let id = debugger.add_exception_breakpoint(ExceptionBreakpoint::new(kind.id, class));
        cx.notify();
        Some(id)
    }

    pub(crate) fn view_action(
        &mut self,
        _: &ViewBreakpoints,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_dialog(None, window, cx);
    }

    /// Toggle Line Breakpoint at a gutter line (zero-based) of an editor:
    /// a new breakpoint, temporary with Alt, or a non-suspending logging
    /// one with Shift.
    pub(crate) fn toggle_from_gutter(
        &mut self,
        editor: &Entity<Editor>,
        line: usize,
        temporary: bool,
        logging: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        let Some(path) = editor.read(cx).path().cloned() else {
            return;
        };
        let line = line as u32 + 1;
        if debugger.breakpoints().at(&path, line).is_some() {
            return;
        }
        if !self.can_put(editor.entity_id(), line, cx) {
            return;
        }
        let Toggled::Added(id) = debugger.toggle_breakpoint(&path, line, temporary) else {
            return;
        };
        if logging {
            // The selection's text is logged, else "Breakpoint hit"
            // (`XBreakpointUtil.toggleLineBreakpoint` with `isLogging`).
            let selected = {
                let e = editor.read(cx);
                let range = e.selections().primary().range();
                (!range.is_empty())
                    .then(|| e.buffer().slice(range))
                    .filter(|t| !t.trim().is_empty() && !t.contains('\n'))
            };
            debugger.update_breakpoint(id, |b| {
                b.suspend = SuspendPolicy::None;
                match selected {
                    Some(expression) => b.log_expression = Some(expression),
                    None => b.log_message = true,
                }
            });
            self.refresh_path(&path, cx);
            self.open_popup(editor, id, window, cx);
            return;
        }
        self.refresh_path(&path, cx);
    }

    pub(crate) fn remember_condition(&mut self, condition: &str) {
        let condition = condition.trim();
        if condition.is_empty() {
            return;
        }
        self.history.retain(|c| c != condition);
        self.history.insert(0, condition.to_owned());
        self.history.truncate(HISTORY);
    }

    // ---- the dialog -----------------------------------------------------------

    /// Opens the Breakpoints dialog (or brings it to the front), with a
    /// breakpoint selected.
    pub fn open_dialog(
        &mut self,
        select: Option<debugger::BreakpointId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(debugger) = self.debugger.clone() else {
            return;
        };
        self.close_popup(true, window, cx);
        if let Some(dialog) = &self.dialog
            && dialog
                .update(cx, |d, window, cx| {
                    window.activate_window();
                    if let Some(id) = select {
                        d.select_breakpoint(id, window, cx);
                    }
                })
                .is_ok()
        {
            return;
        }
        let view = cx.entity();
        self.dialog = BreakpointsDialog::open(view, debugger, select, self.activate, window, cx);
        cx.notify();
    }

    pub(crate) fn close_dialog(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.dialog.take() {
            ui::dialog_window::close(dialog.into(), cx);
        }
    }

    /// A dialog was closed (by its title bar too): forgotten unless another
    /// one is open by now.
    pub(crate) fn dialog_closed(&mut self, cx: &mut Context<Self>) {
        if self.dialog.as_ref().is_some_and(|d| d.read(cx).is_err()) {
            self.dialog = None;
            cx.notify();
        }
    }

    /// Jump to Source: the window opens the file at the line.
    pub(crate) fn navigate(&mut self, path: PathBuf, line: u32, cx: &mut Context<Self>) {
        cx.emit(DebuggerViewEvent::OpenAt {
            path,
            line,
            column: 0,
        });
    }

    // ---- drawing and state ----------------------------------------------------

    /// The popup and the gutter tooltip over the active editor.
    pub fn render_overlay(
        &mut self,
        active: Option<&Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(element) = self.render_value_hint(active, window, cx) {
            out.push(element);
        }
        if let Some(element) = self.render_tooltip(active, window, cx) {
            out.push(element);
        }
        if let Some(element) = self.render_popup(active, window, cx) {
            out.push(element);
        }
        out
    }

    /// The debugger as JSON (`debug.state`): the model's state, the icon
    /// and tooltip of each line breakpoint, the popup, the tooltip and the
    /// dialog.
    pub fn state_json(&self, cx: &App) -> Json {
        let Some(debugger) = &self.debugger else {
            return json!({
                "loaded": false,
                "available": self.hook.is_some(),
                "show_values_inline": self.inline_values_shown(),
            });
        };
        let mut state = debugger.state_json();
        let breakpoints = debugger.breakpoints();
        let muted = debugger.is_muted();
        let (default_suspend, suspend_thread) = line_defaults(debugger);
        if let Some(list) = state["breakpoints"].as_array_mut() {
            for item in list {
                let file = item["file"].as_str().unwrap_or_default();
                let line = item["line"].as_u64().unwrap_or_default() as u32;
                let Some(path) = debugger::storage::resolve_path(file, debugger.root()) else {
                    continue;
                };
                let Some(b) = breakpoints.at(&path, line) else {
                    continue;
                };
                let status = debugger.breakpoint_status(b.id);
                item["icon"] = json!(presentation::line_icon(b, muted, status.as_ref()).0);
                item["badge"] = json!(presentation::badge(b.condition.as_deref()).map(|i| i.0));
                item["tooltip"] = json!(presentation::tooltip(
                    b,
                    status.as_ref(),
                    default_suspend,
                    suspend_thread
                ));
            }
        }
        let kind = debugger.provider().exception_breakpoint_type();
        if let Some(list) = state["exception_breakpoints"].as_array_mut() {
            for (item, b) in list.iter_mut().zip(breakpoints.exception()) {
                item["name"] = json!(match &kind {
                    Some(kind) => kind.display_name(b).to_owned(),
                    None => b.class.clone(),
                });
                item["icon"] = json!(presentation::exception_icon(b, muted).0);
            }
        }
        state["loaded"] = json!(true);
        state["default_suspend"] = json!(debugger.default_suspend().as_str());
        state["popup"] = self.popup_json(cx);
        state["tooltip"] = match &self.tooltip {
            Some(t) if t.shown => json!({"line": t.line, "lines": t.lines}),
            _ => Json::Null,
        };
        state["dialog"] = self
            .dialog
            .as_ref()
            .and_then(|d| d.read(cx).ok())
            .map(|d| d.state_json(cx))
            .unwrap_or(Json::Null);
        let (tool_window, execution_point) = self.sessions_json(cx);
        state["tool_window"] = tool_window;
        state["execution_point"] = execution_point;
        state["show_values_inline"] = json!(self.inline_values_shown());
        state["inline_values"] = self.inline_values_json();
        state["value_hint"] = self.value_hint_json(cx);
        state["starting"] = json!(self.starting.len());
        state
    }

    /// Paths as `debug.state` shows them.
    pub(crate) fn shown_path(&self, path: &Path) -> String {
        match &self.debugger {
            Some(d) => storage_path(path, d.root()),
            None => path.display().to_string(),
        }
    }
}

/// How many conditions the history keeps.
const HISTORY: usize = 20;

/// The caret's line in an editor, from 1.
pub(crate) fn caret_line(editor: &Entity<Editor>, cx: &App) -> u32 {
    let e = editor.read(cx);
    let head = e.selections().primary().head;
    e.buffer().char_to_line(head) as u32 + 1
}

/// What new line breakpoints suspend and whether All or Thread can be
/// chosen.
pub(crate) fn line_defaults(debugger: &Debugger) -> (SuspendPolicy, bool) {
    let thread = debugger
        .provider()
        .line_breakpoint_type()
        .is_some_and(|t| t.suspend_thread);
    (debugger.default_suspend(), thread)
}

/// Adds handlers to `el` (the workbench root) that run the breakpoint
/// commands with `active`, the active editor, wherever the focus is in the
/// window.
pub fn forward_actions<E: InteractiveElement>(
    el: E,
    view: Entity<DebuggerView>,
    active: Option<Entity<Editor>>,
) -> E {
    let v = view.clone();
    let editor = active.clone();
    let el = el.on_action(move |a: &ToggleBreakpoint, _, cx| {
        v.update(cx, |v, cx| v.toggle_action(a, editor.as_ref(), cx))
    });
    let v = view.clone();
    let editor = active.clone();
    let el = el.on_action(move |a: &ToggleTemporaryBreakpoint, _, cx| {
        v.update(cx, |v, cx| {
            v.toggle_temporary_action(a, editor.as_ref(), cx)
        })
    });
    let v = view.clone();
    let editor = active.clone();
    let el = el.on_action(move |a: &ToggleBreakpointEnabled, _, cx| {
        v.update(cx, |v, cx| v.toggle_enabled_action(a, editor.as_ref(), cx))
    });
    let v = view.clone();
    let editor = active.clone();
    let el = el.on_action(move |a: &EditBreakpoint, window, cx| {
        v.update(cx, |v, cx| v.edit_action(a, editor.as_ref(), window, cx))
    });
    let v = view.clone();
    let el = el.on_action(move |a: &ViewBreakpoints, window, cx| {
        v.update(cx, |v, cx| v.view_action(a, window, cx))
    });
    let v = view.clone();
    let el =
        el.on_action(move |a: &MuteBreakpoints, _, cx| v.update(cx, |v, cx| v.mute_action(a, cx)));
    let v = view.clone();
    let el = el.on_action(move |a: &AddExceptionBreakpoint, window, cx| {
        v.update(cx, |v, cx| v.add_exception_action(a, window, cx))
    });
    let el = crate::value_hint::forward_actions(el, view.clone(), active.clone());
    forward_session_actions(el, view, active)
}

/// The session commands of the toolbar, the Run menu and the keys.
fn forward_session_actions<E: InteractiveElement>(
    el: E,
    view: Entity<DebuggerView>,
    active: Option<Entity<Editor>>,
) -> E {
    let v = view.clone();
    let el = el.on_action(move |a: &Resume, _, cx| v.update(cx, |v, cx| v.resume_action(a, cx)));
    let v = view.clone();
    let el = el.on_action(move |a: &Pause, _, cx| v.update(cx, |v, cx| v.pause_action(a, cx)));
    let v = view.clone();
    let el =
        el.on_action(move |a: &StepOver, _, cx| v.update(cx, |v, cx| v.step_over_action(a, cx)));
    let v = view.clone();
    let el = el.on_action(move |a: &StepInto, _, cx| {
        // Without a session the key goes on to the next binding.
        if !v.update(cx, |v, cx| v.step_into_action(a, cx)) {
            cx.propagate();
        }
    });
    let v = view.clone();
    let el = el.on_action(move |a: &StepIntoMyCode, _, cx| {
        v.update(cx, |v, cx| v.step_into_my_code_action(a, cx))
    });
    let v = view.clone();
    let el = el.on_action(move |a: &StepOut, _, cx| v.update(cx, |v, cx| v.step_out_action(a, cx)));
    let v = view.clone();
    let editor = active.clone();
    let el = el.on_action(move |a: &RunToCursor, _, cx| {
        v.update(cx, |v, cx| {
            v.run_to_cursor(a.file.as_deref(), a.line, false, editor.as_ref(), cx)
        })
    });
    let v = view.clone();
    let editor = active;
    let el = el.on_action(move |a: &ForceRunToCursor, _, cx| {
        v.update(cx, |v, cx| {
            v.run_to_cursor(a.file.as_deref(), a.line, true, editor.as_ref(), cx)
        })
    });
    let v = view.clone();
    let el = el.on_action(move |a: &ShowExecutionPoint, _, cx| {
        v.update(cx, |v, cx| v.show_execution_point_action(a, cx))
    });
    let v = view.clone();
    let el = el
        .on_action(move |a: &StopSession, _, cx| v.update(cx, |v, cx| v.stop_session(a.index, cx)));
    let v = view.clone();
    let el = el.on_action(move |a: &RerunSession, window, cx| {
        v.update(cx, |v, cx| v.rerun_session(a.index, window, cx))
    });
    let v = view.clone();
    let el = el.on_action(move |a: &CloseSession, _, cx| {
        v.update(cx, |v, cx| v.close_session(a.index, cx))
    });
    let v = view.clone();
    let el = el.on_action(move |a: &SelectSession, _, cx| {
        v.update(cx, |v, cx| v.select_session(a.index, cx))
    });
    let v = view.clone();
    let el =
        el.on_action(move |a: &SelectContent, _, cx| v.update(cx, |v, cx| v.select_content(a, cx)));
    let v = view.clone();
    let el = el.on_action(move |_: &MoreActions, _, cx| {
        v.update(cx, |v, cx| {
            let position = v.tool_window().read(cx).more_menu_position();
            cx.emit(DebuggerViewEvent::MoreMenu(position))
        })
    });
    let v = view.clone();
    let el = el.on_action(move |a: &TypeInput, _, cx| v.update(cx, |v, cx| v.type_input(a, cx)));
    let v = view.clone();
    let el = el.on_action(move |a: &ToggleInlineValues, _, cx| {
        v.update(cx, |v, cx| v.toggle_inline_values(a.on, cx))
    });
    forward_frame_actions(el, view)
}

/// The commands of the frames and variables views.
fn forward_frame_actions<E: InteractiveElement>(el: E, view: Entity<DebuggerView>) -> E {
    let v = view.clone();
    let el = el.on_action(move |_: &ShowThreads, window, cx| {
        v.update(cx, |v, cx| v.show_threads(window, cx))
    });
    let v = view.clone();
    let el =
        el.on_action(move |a: &SelectThread, _, cx| v.update(cx, |v, cx| v.select_thread(a, cx)));
    let v = view.clone();
    let el = el
        .on_action(move |a: &SelectFrame, _, cx| v.update(cx, |v, cx| v.select_frame(a.index, cx)));
    let v = view.clone();
    let el =
        el.on_action(move |_: &PreviousFrame, _, cx| v.update(cx, |v, cx| v.step_frame(-1, cx)));
    let v = view.clone();
    let el = el.on_action(move |_: &NextFrame, _, cx| v.update(cx, |v, cx| v.step_frame(1, cx)));
    let v = view.clone();
    let el = el
        .on_action(move |_: &CloseFramesHint, _, cx| v.update(cx, |v, cx| v.close_frames_hint(cx)));
    let v = view.clone();
    let el = el.on_action(move |_: &VariablesUp, _, cx| {
        v.update(cx, |v, cx| {
            v.with_variables(cx, |t, cx| t.move_selection(-1, cx))
        })
    });
    let v = view.clone();
    let el = el.on_action(move |_: &VariablesDown, _, cx| {
        v.update(cx, |v, cx| {
            v.with_variables(cx, |t, cx| t.move_selection(1, cx))
        })
    });
    let v = view.clone();
    let el = el.on_action(move |_: &VariablesLeft, _, cx| {
        v.update(cx, |v, cx| {
            v.with_variables(cx, |t, cx| t.left_right(false, cx))
        })
    });
    let v = view.clone();
    let el = el.on_action(move |_: &VariablesRight, _, cx| {
        v.update(cx, |v, cx| {
            v.with_variables(cx, |t, cx| t.left_right(true, cx))
        })
    });
    let v = view.clone();
    let el = el.on_action(move |a: &SelectVariable, _, cx| {
        v.update(cx, |v, cx| {
            v.with_variables(cx, |t, cx| {
                t.select_names(&a.path, cx);
            })
        })
    });
    let v = view.clone();
    let el = el.on_action(move |a: &ExpandVariable, _, cx| {
        v.update(cx, |v, cx| {
            v.with_variables(cx, |t, cx| {
                t.expand_names(&a.path, a.expanded, cx);
            })
        })
    });
    crate::watches::forward_watch_actions(el, view)
}
