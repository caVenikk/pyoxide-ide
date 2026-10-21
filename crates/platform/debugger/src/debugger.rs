//! The debugger of one project (the reference's `XDebuggerManager`): its
//! breakpoints, watches and mute state, kept in the personal project state
//! and imported once from the reference, and its debug sessions.
//!
//! Changes apply at once and reach the running sessions; the file is
//! written on a thread of its own. Listeners get [`DebuggerEvent`]s.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use serde_json::{Value, json};

use crate::breakpoints::{
    BreakpointId, Breakpoints, ExceptionBreakpoint, LineBreakpoint, SuspendPolicy, Toggled, Watches,
};
use crate::provider::{
    DebugAdapterProvider, DebugRequest, ExceptionFilter, ExceptionFilterSetting,
};
use crate::session::{BreakpointStatus, Phase, Session, SessionId};
use crate::storage::{self, Stored};

/// How many expressions the history keeps (the reference's
/// `debugger.max.recent.expressions`).
pub const HISTORY: usize = 50;

/// What changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebuggerEvent {
    /// Breakpoints were added, removed or edited, or their verification
    /// changed.
    BreakpointsChanged,
    WatchesChanged,
    MutedChanged(bool),
    SessionStarted(SessionId),
    /// A session's state, threads, frames or values changed.
    SessionChanged(SessionId),
    /// A session started its program: [`Session::take_console`] has it.
    Console(SessionId),
    /// Output of the adapter (`console`, `important`) or of the program
    /// when the adapter forwards it (`stdout`, `stderr`).
    Output {
        session: SessionId,
        category: String,
        text: String,
    },
    SessionEnded(SessionId),
}

enum Save {
    Now,
    Flush(flume::Sender<()>),
}

struct State {
    stored: Stored,
    sessions: Vec<Session>,
}

pub(crate) struct Inner {
    root: PathBuf,
    provider: Arc<dyn DebugAdapterProvider>,
    state: Mutex<State>,
    listeners: Mutex<Vec<flume::Sender<DebuggerEvent>>>,
    saver: flume::Sender<Save>,
    next_session: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    pub(crate) fn provider(&self) -> &Arc<dyn DebugAdapterProvider> {
        &self.provider
    }

    pub(crate) fn emit(&self, event: DebuggerEvent) {
        let mut listeners = lock(&self.listeners);
        listeners.retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Whether breakpoints are muted, and the breakpoints of a file.
    pub(crate) fn breakpoints_of(&self, file: &Path) -> (bool, Vec<LineBreakpoint>) {
        let state = self.state();
        let list = state
            .stored
            .breakpoints
            .in_file(file)
            .into_iter()
            .cloned()
            .collect();
        (state.stored.muted, list)
    }

    pub(crate) fn breakpoint_files(&self) -> Vec<PathBuf> {
        self.state().stored.breakpoints.files()
    }

    /// The adapter's filters for the enabled exception breakpoints (none
    /// while muted).
    pub(crate) fn exception_filters(
        &self,
        available: &[ExceptionFilter],
    ) -> Vec<ExceptionFilterSetting> {
        let enabled: Vec<ExceptionBreakpoint> = {
            let state = self.state();
            if state.stored.muted {
                return Vec::new();
            }
            state
                .stored
                .breakpoints
                .exception()
                .iter()
                .filter(|b| b.enabled)
                .cloned()
                .collect()
        };
        let enabled: Vec<&ExceptionBreakpoint> = enabled.iter().collect();
        self.provider.exception_filters(&enabled, available)
    }

    /// Temporary breakpoints that were hit are removed.
    pub(crate) fn remove_temporary(self: &Arc<Self>, hits: &[BreakpointId]) {
        let files: Vec<PathBuf> = {
            let mut state = self.state();
            let temporary: Vec<(BreakpointId, PathBuf)> = hits
                .iter()
                .filter_map(|id| state.stored.breakpoints.get(*id))
                .filter(|b| b.temporary)
                .map(|b| (b.id, b.file.clone()))
                .collect();
            for (id, _) in &temporary {
                state.stored.breakpoints.remove(*id);
            }
            temporary.into_iter().map(|(_, file)| file).collect()
        };
        if !files.is_empty() {
            Debugger {
                inner: self.clone(),
            }
            .breakpoints_changed(files, false);
        }
    }
}

/// The debugger of a project. Cheap to clone.
#[derive(Clone)]
pub struct Debugger {
    inner: Arc<Inner>,
}

impl Debugger {
    /// Opens the debugger state of a project: what is stored, or once the
    /// reference's breakpoints and watches, or the product's defaults.
    /// Blocking (reads files): called off the UI thread. A stored state
    /// that cannot be read is left alone (and not overwritten).
    pub fn open(root: &Path, provider: Arc<dyn DebugAdapterProvider>) -> Self {
        let mut writable = true;
        let mut stored = match storage::load(root) {
            Ok(Some(stored)) => stored,
            Ok(None) => Stored::default(),
            Err(_) => {
                writable = false;
                Stored::default()
            }
        };
        let mut import_done = false;
        if writable
            && !stored.imported
            && let Some(imported) = crate::idea::import(root, &provider.idea_types())
        {
            for b in imported.breakpoints.line() {
                stored.breakpoints.add_line(b.clone());
            }
            for b in imported.breakpoints.exception() {
                stored.breakpoints.add_exception(b.clone());
            }
            stored.has_exception_breakpoints |= imported.has_exception_breakpoints;
            for (key, list) in imported.watches.keys() {
                if stored.watches.get(key).is_empty() {
                    stored.watches.set(key, list.clone());
                }
            }
            stored.imported = true;
            import_done = true;
        }
        // The product's default breakpoints ("Any exception") are there
        // unless the stored ones have them.
        for default in provider.default_exception_breakpoints() {
            let present = stored
                .breakpoints
                .exception()
                .iter()
                .any(|b| b.kind == default.kind && b.class == default.class);
            if !present {
                let mut default = default;
                default.default = true;
                stored.breakpoints.add_exception(default);
            }
        }
        let (saver, requests) = flume::unbounded();
        let inner = Arc::new(Inner {
            root: root.to_owned(),
            provider,
            state: Mutex::new(State {
                stored,
                sessions: Vec::new(),
            }),
            listeners: Mutex::new(Vec::new()),
            saver,
            next_session: AtomicU64::new(1),
        });
        let weak = Arc::downgrade(&inner);
        let _ = thread::Builder::new()
            .name("debugger store".to_owned())
            .spawn(move || {
                while let Ok(first) = requests.recv() {
                    let mut flushes = Vec::new();
                    for request in std::iter::once(first).chain(requests.try_iter()) {
                        if let Save::Flush(done) = request {
                            flushes.push(done);
                        }
                    }
                    if let Some(inner) = weak.upgrade() {
                        let snapshot = inner.state().stored.clone();
                        if writable {
                            let _ = storage::save(&inner.root, &snapshot);
                        }
                    }
                    for done in flushes {
                        let _ = done.send(());
                    }
                }
            });
        let debugger = Self { inner };
        if import_done {
            debugger.save();
        }
        debugger
    }

    pub(crate) fn from_inner(inner: Arc<Inner>) -> Self {
        Self { inner }
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// The product's hook (its breakpoint types).
    pub fn provider(&self) -> &Arc<dyn DebugAdapterProvider> {
        &self.inner.provider
    }

    /// Events from now on.
    pub fn subscribe(&self) -> flume::Receiver<DebuggerEvent> {
        let (tx, rx) = flume::unbounded();
        lock(&self.inner.listeners).push(tx);
        rx
    }

    fn save(&self) {
        let _ = self.inner.saver.send(Save::Now);
    }

    /// Waits until the changes so far are written.
    pub fn flush(&self) {
        let (tx, rx) = flume::bounded(1);
        if self.inner.saver.send(Save::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    // ---- breakpoints -----------------------------------------------------------

    pub fn breakpoints(&self) -> Breakpoints {
        self.inner.state().stored.breakpoints.clone()
    }

    fn sessions_live(&self) -> Vec<Session> {
        self.inner
            .state()
            .sessions
            .iter()
            .filter(|s| !s.is_ended())
            .cloned()
            .collect()
    }

    fn breakpoints_changed(&self, files: Vec<PathBuf>, exceptions: bool) {
        self.save();
        self.inner.emit(DebuggerEvent::BreakpointsChanged);
        for session in self.sessions_live() {
            session.sync_breakpoints(&files);
            if exceptions {
                session.sync_exceptions();
            }
        }
    }

    fn edit<T>(&self, f: impl FnOnce(&mut Breakpoints) -> T) -> T {
        f(&mut self.inner.state().stored.breakpoints)
    }

    /// Toggle Line Breakpoint (Temporary with `temporary`): a new
    /// breakpoint suspends what new breakpoints of the line type suspend
    /// ([`Self::default_suspend`]).
    pub fn toggle_breakpoint(&self, file: &Path, line: u32, temporary: bool) -> Toggled {
        let suspend = self.default_suspend();
        let toggled = self.edit(|b| {
            let toggled = b.toggle(file, line, temporary);
            if let Toggled::Added(id) = toggled
                && let Some(added) = b.get_mut(id)
            {
                added.suspend = suspend;
            }
            toggled
        });
        self.breakpoints_changed(vec![file.to_owned()], false);
        toggled
    }

    /// What a new line breakpoint suspends: the policy Make Default chose
    /// for the product's line type, else the type's default
    /// (`getBreakpointDefaults(type).getSuspendPolicy()`).
    pub fn default_suspend(&self) -> SuspendPolicy {
        let Some(kind) = self.inner.provider.line_breakpoint_type() else {
            return SuspendPolicy::All;
        };
        let state = self.inner.state();
        state
            .stored
            .suspend_defaults
            .get(&kind.id)
            .copied()
            .unwrap_or(kind.default_suspend)
    }

    /// Make Default: new line breakpoints suspend `policy`.
    pub fn set_default_suspend(&self, policy: SuspendPolicy) {
        let Some(kind) = self.inner.provider.line_breakpoint_type() else {
            return;
        };
        {
            let mut state = self.inner.state();
            if policy == kind.default_suspend {
                state.stored.suspend_defaults.remove(&kind.id);
            } else {
                state.stored.suspend_defaults.insert(kind.id, policy);
            }
        }
        self.save();
        self.inner.emit(DebuggerEvent::BreakpointsChanged);
    }

    /// The editor moved the lines of a file's breakpoints (they follow
    /// its edits): each to its new line, or removed with `None` (its line
    /// was deleted). Two on one line: the first one stays.
    pub fn set_lines(&self, file: &Path, lines: &[(BreakpointId, Option<u32>)]) {
        let changed = self.edit(|b| {
            let mut changed = false;
            for (id, line) in lines {
                match line {
                    Some(line) => {
                        if let Some(bp) = b.get_mut(*id)
                            && bp.line != (*line).max(1)
                        {
                            bp.line = (*line).max(1);
                            changed = true;
                        }
                    }
                    None => changed |= b.remove(*id),
                }
            }
            changed | b.dedup_lines()
        });
        if changed {
            self.breakpoints_changed(vec![file.to_owned()], false);
        }
    }

    /// Adds a line breakpoint (replacing one on its line).
    pub fn add_breakpoint(&self, breakpoint: LineBreakpoint) -> BreakpointId {
        let file = breakpoint.file.clone();
        let id = self.edit(|b| b.add_line(breakpoint));
        self.breakpoints_changed(vec![file], false);
        id
    }

    /// Changes a line breakpoint (enabled, policy, condition, logging).
    pub fn update_breakpoint(&self, id: BreakpointId, f: impl FnOnce(&mut LineBreakpoint)) -> bool {
        let file = self.edit(|b| {
            b.get_mut(id).map(|bp| {
                f(bp);
                bp.line = bp.line.max(1);
                bp.file.clone()
            })
        });
        match file {
            Some(file) => {
                self.breakpoints_changed(vec![file], false);
                true
            }
            None => false,
        }
    }

    /// Removes a line breakpoint or an exception breakpoint that is not a
    /// default one.
    pub fn remove_breakpoint(&self, id: BreakpointId) -> bool {
        let (file, removed) = self.edit(|b| {
            let file = b.get(id).map(|bp| bp.file.clone());
            (file, b.remove(id))
        });
        if removed {
            let exceptions = file.is_none();
            self.breakpoints_changed(file.into_iter().collect(), exceptions);
        }
        removed
    }

    /// Lines of a file were edited (see [`Breakpoints::lines_changed`]).
    pub fn lines_changed(&self, file: &Path, first: u32, old_count: u32, new_count: u32) {
        if self.edit(|b| b.lines_changed(file, first, old_count, new_count)) {
            self.breakpoints_changed(vec![file.to_owned()], false);
        }
    }

    /// A file or folder was renamed or moved.
    pub fn file_moved(&self, from: &Path, to: &Path) {
        let before = self.breakpoints().files();
        if self.edit(|b| b.file_moved(from, to)) {
            let mut files = before;
            files.extend(self.breakpoints().files());
            files.sort();
            files.dedup();
            self.breakpoints_changed(files, false);
        }
    }

    /// Adds an exception breakpoint (one of the same type and class is
    /// replaced).
    pub fn add_exception_breakpoint(&self, breakpoint: ExceptionBreakpoint) -> BreakpointId {
        let id = {
            let mut state = self.inner.state();
            state.stored.has_exception_breakpoints = true;
            state.stored.breakpoints.add_exception(breakpoint)
        };
        self.breakpoints_changed(Vec::new(), true);
        id
    }

    pub fn update_exception_breakpoint(
        &self,
        id: BreakpointId,
        f: impl FnOnce(&mut ExceptionBreakpoint),
    ) -> bool {
        let found = {
            let mut state = self.inner.state();
            state.stored.has_exception_breakpoints = true;
            state.stored.breakpoints.exception_mut(id).map(f).is_some()
        };
        if found {
            self.breakpoints_changed(Vec::new(), true);
        }
        found
    }

    /// What the running sessions say of a breakpoint: verified when one
    /// of them verified it.
    pub fn breakpoint_status(&self, id: BreakpointId) -> Option<BreakpointStatus> {
        let mut found: Option<BreakpointStatus> = None;
        for session in self.sessions_live() {
            if let Some(status) = session.breakpoint_status(id)
                && found.as_ref().is_none_or(|f| !f.verified)
            {
                found = Some(status);
            }
        }
        found
    }

    pub fn is_muted(&self) -> bool {
        self.inner.state().stored.muted
    }

    /// Mute Breakpoints: no breakpoint suspends while muted.
    pub fn set_muted(&self, muted: bool) {
        let files = {
            let mut state = self.inner.state();
            if state.stored.muted == muted {
                return;
            }
            state.stored.muted = muted;
            state.stored.breakpoints.files()
        };
        self.save();
        self.inner.emit(DebuggerEvent::MutedChanged(muted));
        for session in self.sessions_live() {
            session.sync_breakpoints(&files);
            session.sync_exceptions();
        }
    }

    // ---- watches ---------------------------------------------------------------

    pub fn watches(&self, key: &str) -> Vec<String> {
        self.inner.state().stored.watches.get(key).to_vec()
    }

    fn edit_watches(&self, f: impl FnOnce(&mut Watches) -> bool) -> bool {
        let changed = f(&mut self.inner.state().stored.watches);
        if changed {
            self.save();
            self.inner.emit(DebuggerEvent::WatchesChanged);
        }
        changed
    }

    /// New Watch: at `index`, or at the end.
    pub fn add_watch(&self, key: &str, expression: &str, index: Option<usize>) -> bool {
        self.edit_watches(|w| w.add(key, expression, index))
    }

    pub fn remove_watch(&self, key: &str, index: usize) -> bool {
        self.edit_watches(|w| w.remove(key, index).is_some())
    }

    pub fn edit_watch(&self, key: &str, index: usize, expression: &str) -> bool {
        self.edit_watches(|w| w.edit(key, index, expression))
    }

    // ---- Threads & Variables -----------------------------------------------------

    /// The expressions evaluated last, the latest first
    /// (`XDebuggerHistoryManager`).
    pub fn expression_history(&self) -> Vec<String> {
        self.inner.state().stored.view.history.clone()
    }

    /// An expression was evaluated or watched: it goes first in the
    /// history, which keeps [`HISTORY`] of them.
    pub fn remember_expression(&self, expression: &str) {
        let expression = expression.trim();
        if expression.is_empty() || expression.len() > 100_000 {
            return;
        }
        {
            let mut state = self.inner.state();
            let history = &mut state.stored.view.history;
            if history.first().is_some_and(|e| e == expression) {
                return;
            }
            history.retain(|e| e != expression);
            history.insert(0, expression.to_owned());
            history.truncate(HISTORY);
        }
        self.save();
    }

    /// The frames' share of the width of Threads & Variables, once it was
    /// dragged.
    pub fn frames_split(&self) -> Option<f32> {
        self.inner.state().stored.view.frames_split
    }

    pub fn set_frames_split(&self, split: f32) {
        let split = split.clamp(0.05, 0.95);
        let changed = {
            let mut state = self.inner.state();
            let changed = state.stored.view.frames_split != Some(split);
            state.stored.view.frames_split = Some(split);
            changed
        };
        if changed {
            self.save();
        }
    }

    /// The hint below the frames was closed.
    pub fn frames_hint_closed(&self) -> bool {
        self.inner.state().stored.view.frames_hint_closed
    }

    pub fn close_frames_hint(&self) {
        let changed = {
            let mut state = self.inner.state();
            !std::mem::replace(&mut state.stored.view.frames_hint_closed, true)
        };
        if changed {
            self.save();
        }
    }

    // ---- sessions --------------------------------------------------------------

    /// Starts a debug session for a run configuration. Blocking (the
    /// product prepares the launch): called off the UI thread. An error is
    /// a message for the user.
    pub fn start(&self, request: DebugRequest) -> Result<Session, String> {
        let launch = self.inner.provider.prepare(&request)?;
        let id = SessionId(self.inner.next_session.fetch_add(1, Ordering::Relaxed));
        let session = Session::new(
            id,
            launch.title.clone(),
            request.watches_key().to_owned(),
            Arc::downgrade(&self.inner),
            self.inner.provider.step_into_my_code(),
            self.inner.provider.prepare_steps(),
            &launch,
        );
        self.inner.state().sessions.push(session.clone());
        self.inner.emit(DebuggerEvent::SessionStarted(id));
        if let Err(error) = session.connect(launch) {
            self.inner.state().sessions.retain(|s| s.id() != id);
            self.inner.emit(DebuggerEvent::SessionEnded(id));
            return Err(error);
        }
        Ok(session)
    }

    /// Every session, the ended ones too (their tabs are open).
    pub fn sessions(&self) -> Vec<Session> {
        self.inner.state().sessions.clone()
    }

    pub fn session(&self, id: SessionId) -> Option<Session> {
        self.inner
            .state()
            .sessions
            .iter()
            .find(|s| s.id() == id)
            .cloned()
    }

    /// The last session started that has not ended.
    pub fn current_session(&self) -> Option<Session> {
        self.sessions_live().pop()
    }

    /// Forgets an ended session (its tab was closed); one still running is
    /// stopped first.
    pub fn close_session(&self, id: SessionId) -> Option<flume::Receiver<()>> {
        let session = self.session(id)?;
        let done = session.stop();
        self.inner.state().sessions.retain(|s| s.id() != id);
        Some(done)
    }

    /// Stops every session (the project closes); the receiver gets a
    /// message when all processes are gone.
    pub fn stop_all(&self) -> flume::Receiver<()> {
        let waits: Vec<flume::Receiver<()>> =
            self.sessions_live().iter().map(Session::stop).collect();
        let (tx, rx) = flume::bounded(1);
        let _ = thread::Builder::new()
            .name("debug stop all".to_owned())
            .spawn(move || {
                for wait in waits {
                    let _ = wait.recv();
                }
                let _ = tx.send(());
            });
        rx
    }

    /// Kills every session's processes at once (the application quits and
    /// cannot wait for Stop).
    pub fn kill_all(&self) {
        for session in self.sessions_live() {
            session.kill();
        }
    }

    /// The debugger's state as JSON (`debug.state`): breakpoints with
    /// their verification, exception breakpoints, watches, mute and every
    /// session with its threads, frames and loaded variables.
    pub fn state_json(&self) -> Value {
        let (stored, sessions) = {
            let state = self.inner.state();
            (state.stored.clone(), state.sessions.clone())
        };
        let root = &self.inner.root;
        let breakpoints: Vec<Value> = {
            let mut list: Vec<&LineBreakpoint> = stored.breakpoints.line().iter().collect();
            list.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
            list.into_iter()
                .map(|b| {
                    let status = self.breakpoint_status(b.id);
                    json!({
                        "file": storage::stored_path(&b.file, root),
                        "line": b.line,
                        "enabled": b.enabled,
                        "suspend": b.suspend.as_str(),
                        "condition": b.condition,
                        "log_message": b.log_message,
                        "log_stack": b.log_stack,
                        "log_expression": b.log_expression,
                        "temporary": b.temporary,
                        "verified": status.as_ref().map(|s| s.verified),
                        "message": status.as_ref().and_then(|s| s.message.clone()),
                        "adapter_line": status.as_ref().and_then(|s| s.line),
                    })
                })
                .collect()
        };
        let exceptions: Vec<Value> = stored
            .breakpoints
            .exception()
            .iter()
            .map(|b| {
                json!({
                    "type": b.kind,
                    "class": b.class,
                    "default": b.default,
                    "enabled": b.enabled,
                    "suspend": b.suspend.as_str(),
                    "condition": b.condition,
                    "properties": b.properties,
                })
            })
            .collect();
        let watches: serde_json::Map<String, Value> = stored
            .watches
            .keys()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let sessions: Vec<Value> = sessions
            .iter()
            .map(|session| {
                let s = session.snapshot();
                let frames: Vec<Value> = s
                    .frames
                    .iter()
                    .map(|f| {
                        let file = f
                            .source
                            .as_ref()
                            .and_then(|src| src.path.as_ref())
                            .map(|p| storage::stored_path(p, root));
                        json!({"id": f.id, "name": f.name, "file": file, "line": f.line, "column": f.column})
                    })
                    .collect();
                json!({
                    "id": s.id.0,
                    "name": s.name,
                    "watches_key": s.watches_key,
                    "state": s.phase.as_str(),
                    "subprocesses": session.subprocesses(),
                    "stop": s.stop.as_ref().map(|stop| json!({
                        "reason": stop.reason,
                        "thread": stop.thread_id,
                        "description": stop.description,
                        "text": stop.text,
                        "exception": stop.exception.as_ref().map(|e| json!({
                            "id": e.id,
                            "description": e.description,
                            "break_mode": e.break_mode,
                            "type": e.type_name,
                            "message": e.message,
                        })),
                    })),
                    "threads": s.threads.iter().map(|t| json!({"id": t.id, "name": t.name})).collect::<Vec<_>>(),
                    "selected_thread": s.selected_thread,
                    "frames": frames,
                    "frames_total": s.frames_total,
                    "selected_frame": s.selected_frame,
                    "variables": session.variables_json(),
                    "watches": stored.watches.get(&s.watches_key),
                    "exit_code": s.exit_code,
                    "error": s.error,
                    "processes": {"adapter": s.adapter_pid, "debuggee": s.debuggee_pid},
                })
            })
            .collect();
        json!({
            "muted": stored.muted,
            "breakpoints": breakpoints,
            "exception_breakpoints": exceptions,
            "watches": watches,
            "sessions": sessions,
            "running": sessions.iter().any(|s| s["state"] != Phase::Ended.as_str()),
        })
    }
}
