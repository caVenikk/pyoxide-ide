//! One debug session (the reference's `XDebugSession`) over a DAP client:
//! its state machine, threads, frames in pages, scopes, variables loaded
//! lazily in pages, evaluation and setting values, breakpoints sent to the
//! adapter with their verification, and the debuggee started through
//! `runInTerminal`.
//!
//! ```text
//! Starting --initialized, configurationDone, launch answered--> Running
//! Running --stopped--> Suspended --resume / step answered, continued--> Running
//! any --stop, terminated--> Stopping --the adapter is gone--> Ended
//! ```
//!
//! Everything runs on the client's reader thread or the caller's thread
//! and returns at once; queries return futures the UI awaits on its
//! executor. Answers that come after the program moved on (a stack trace
//! of the previous stop) are dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use dap_client::{
    Client, ClientInfo, DapError, EvaluateContext, Evaluated, Event, ExceptionInfo, Handler,
    Request, Responder, ReverseRequest, RunInTerminal, Scope, SourceBreakpoint, StackFrame, Thread,
    Variable, VariablesFilter,
};
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use process::{CommandLine, ExitStatus, ProcessEvent, ProcessHandle};
use serde_json::Value;

use crate::breakpoints::{BreakpointId, LineBreakpoint, SuspendPolicy};
use crate::debugger::{DebuggerEvent, Inner as DebuggerInner};
use crate::provider::{AdapterLaunch, LaunchKind, StepIntoMyCode};

/// Frames asked for at a time.
pub const FRAME_PAGE: usize = 20;
/// Children of a variable asked for at a time (the reference shows 100
/// children, then "more", `XCompositeNode.MAX_CHILDREN_TO_SHOW`).
pub const VARIABLES_PAGE: usize = 100;
/// How long Stop waits for the adapter and the program to end before it
/// kills them.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(3);
/// How much of the adapter's log is kept for an error message.
const LOG_TAIL: usize = 4096;

/// A session's identity while the project is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The adapter starts and takes the configuration.
    Starting,
    Running,
    Suspended,
    /// Stop was requested or the program ended; the adapter is ending.
    Stopping,
    Ended,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Starting => "starting",
            Phase::Running => "running",
            Phase::Suspended => "suspended",
            Phase::Stopping => "stopping",
            Phase::Ended => "ended",
        }
    }
}

/// Why a request of the session has no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugError {
    /// The request needs a suspended program.
    NotSuspended,
    /// The session ended.
    Ended,
    Unsupported(String),
    /// The adapter's message.
    Adapter(String),
}

impl std::fmt::Display for DebugError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DebugError::NotSuspended => f.write_str("The program is not suspended"),
            DebugError::Ended => f.write_str("The debug session has ended"),
            DebugError::Unsupported(what) => write!(f, "The debugger does not support {what}"),
            DebugError::Adapter(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DebugError {}

impl From<DapError> for DebugError {
    fn from(error: DapError) -> Self {
        match error {
            DapError::Adapter(message) | DapError::Malformed(message) => {
                DebugError::Adapter(message)
            }
            DapError::Cancelled => DebugError::Adapter("cancelled".to_owned()),
            DapError::Closed => DebugError::Ended,
            DapError::Unsupported(what) => DebugError::Unsupported(what.to_owned()),
        }
    }
}

/// An answer the UI awaits; dropping it does not stop the request.
pub type Pending<T> = BoxFuture<'static, Result<T, DebugError>>;

fn ready<T: Send + 'static>(result: Result<T, DebugError>) -> Pending<T> {
    futures::future::ready(result).boxed()
}

/// What the adapter says of a breakpoint in this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakpointStatus {
    pub verified: bool,
    pub message: Option<String>,
    /// The line the adapter set it on (another one when it moved it).
    pub line: Option<u32>,
}

/// Why the program is suspended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopInfo {
    /// `breakpoint`, `step`, `exception`, `pause`, `entry`, ...
    pub reason: String,
    pub thread_id: Option<i64>,
    pub description: Option<String>,
    pub text: Option<String>,
    pub all_threads: bool,
    /// The exception, when it stopped on one (and the adapter tells).
    pub exception: Option<ExceptionInfo>,
}

/// The loaded children of a variable or a scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Children {
    pub items: Vec<Variable>,
    /// More children can be loaded ([`Session::children`] again).
    pub more: bool,
}

/// What a session shows, copied at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub id: SessionId,
    pub name: String,
    pub watches_key: String,
    pub phase: Phase,
    pub threads: Vec<Thread>,
    pub selected_thread: Option<i64>,
    /// The loaded frames of the selected thread.
    pub frames: Vec<StackFrame>,
    /// All frames of the selected thread, when the adapter said.
    pub frames_total: Option<usize>,
    pub selected_frame: Option<i64>,
    pub stop: Option<StopInfo>,
    pub exit_code: Option<i64>,
    /// Why the session failed.
    pub error: Option<String>,
    pub adapter_pid: Option<u32>,
    pub debuggee_pid: Option<u32>,
}

#[derive(Default)]
struct Frames {
    frames: Vec<StackFrame>,
    total: Option<usize>,
}

struct Data {
    phase: Phase,
    /// `initialized` came: breakpoint changes are sent at once.
    configured: bool,
    launched: bool,
    configuration_done: bool,
    stopping: bool,
    /// Grows whenever the program moves on: answers of an older one are
    /// dropped.
    generation: u64,
    threads: Vec<Thread>,
    stop: Option<StopInfo>,
    frames: HashMap<i64, Frames>,
    selected_thread: Option<i64>,
    selected_frame: Option<i64>,
    scopes: HashMap<i64, Vec<Scope>>,
    children: HashMap<i64, Children>,
    statuses: HashMap<BreakpointId, BreakpointStatus>,
    /// The breakpoints last sent per file, in the order sent.
    sent: HashMap<PathBuf, Vec<BreakpointId>>,
    /// The adapter's breakpoint ids.
    adapter_ids: HashMap<i64, BreakpointId>,
    exit_code: Option<i64>,
    error: Option<String>,
    log_tail: String,
    adapter_pid: Option<u32>,
    debuggee: Option<ProcessHandle>,
    debuggee_pid: Option<u32>,
    console: Option<flume::Receiver<ProcessEvent>>,
    /// Sent at the next stop (after Step Into My Code).
    after_step: Option<(String, Value)>,
}

pub(crate) struct SessionInner {
    id: SessionId,
    name: String,
    watches_key: String,
    debugger: Weak<DebuggerInner>,
    client: OnceLock<Client>,
    step_into_my_code: Option<StepIntoMyCode>,
    data: Mutex<Data>,
}

/// A debug session. Cheap to clone.
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.inner.id)
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Session {
    pub(crate) fn new(
        id: SessionId,
        name: String,
        watches_key: String,
        debugger: Weak<DebuggerInner>,
        step_into_my_code: Option<StepIntoMyCode>,
    ) -> Self {
        Self {
            inner: Arc::new(SessionInner {
                id,
                name,
                watches_key,
                debugger,
                client: OnceLock::new(),
                step_into_my_code,
                data: Mutex::new(Data {
                    phase: Phase::Starting,
                    configured: false,
                    launched: false,
                    configuration_done: false,
                    stopping: false,
                    generation: 0,
                    threads: Vec::new(),
                    stop: None,
                    frames: HashMap::new(),
                    selected_thread: None,
                    selected_frame: None,
                    scopes: HashMap::new(),
                    children: HashMap::new(),
                    statuses: HashMap::new(),
                    sent: HashMap::new(),
                    adapter_ids: HashMap::new(),
                    exit_code: None,
                    error: None,
                    log_tail: String::new(),
                    adapter_pid: None,
                    debuggee: None,
                    debuggee_pid: None,
                    console: None,
                    after_step: None,
                }),
            }),
        }
    }

    /// Connects to the adapter and starts the program: `initialize`, then
    /// `launch` (or `attach`); the configuration follows `initialized`.
    pub(crate) fn connect(&self, launch: AdapterLaunch) -> Result<(), String> {
        let transport = match launch.adapter {
            crate::provider::Adapter::Command(command) => dap_client::Transport::stdio(&command)
                .map_err(|e| format!("Cannot start the debugger: {e}"))?,
            crate::provider::Adapter::Transport(transport) => transport,
        };
        lock(&self.inner.data).adapter_pid = transport.process().map(ProcessHandle::pid);
        let handler = Arc::new(SessionHandler(Arc::downgrade(&self.inner)));
        let client = Client::start(&self.inner.name, transport, handler)
            .map_err(|e| format!("Cannot start the debugger: {e}"))?;
        let _ = self.inner.client.set(client.clone());
        let weak = Arc::downgrade(&self.inner);
        let kind = launch.kind;
        let arguments = launch.arguments;
        client
            .initialize(&ClientInfo::new(launch.adapter_id))
            .on_done(move |result| {
                let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                    return;
                };
                match result {
                    Ok(_) => {}
                    // The adapter went away: the exit reports why.
                    Err(DapError::Closed) => return,
                    Err(error) => {
                        session.fail(format!("Connection to the debugger failed: {error}"));
                        return;
                    }
                }
                let request = match kind {
                    LaunchKind::Launch => client.launch(arguments),
                    LaunchKind::Attach => client.attach(arguments),
                };
                request.on_done(move |result| match result {
                    Ok(()) => {
                        let started = {
                            let mut data = lock(&session.inner.data);
                            data.launched = true;
                            data.phase == Phase::Starting && data.configuration_done
                        };
                        if started {
                            session.set_phase_if(Phase::Starting, Phase::Running);
                        }
                    }
                    Err(DapError::Closed) => {}
                    Err(error) => session.fail(format!("The debugger could not start: {error}")),
                });
            });
        Ok(())
    }

    pub fn id(&self) -> SessionId {
        self.inner.id
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// The key of the watches the session shows.
    pub fn watches_key(&self) -> &str {
        &self.inner.watches_key
    }

    pub fn phase(&self) -> Phase {
        lock(&self.inner.data).phase
    }

    pub fn is_ended(&self) -> bool {
        self.phase() == Phase::Ended
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let data = lock(&self.inner.data);
        let frames = data.selected_thread.and_then(|t| data.frames.get(&t));
        SessionSnapshot {
            id: self.inner.id,
            name: self.inner.name.clone(),
            watches_key: self.inner.watches_key.clone(),
            phase: data.phase,
            threads: data.threads.clone(),
            selected_thread: data.selected_thread,
            frames: frames.map(|f| f.frames.clone()).unwrap_or_default(),
            frames_total: frames.and_then(|f| f.total),
            selected_frame: data.selected_frame,
            stop: data.stop.clone(),
            exit_code: data.exit_code,
            error: data.error.clone(),
            adapter_pid: data.adapter_pid,
            debuggee_pid: data.debuggee_pid,
        }
    }

    /// What the adapter said of a breakpoint.
    pub fn breakpoint_status(&self, id: BreakpointId) -> Option<BreakpointStatus> {
        lock(&self.inner.data).statuses.get(&id).cloned()
    }

    /// The debuggee's process and output, once (`runInTerminal`): the Debug
    /// console shows them as the Run console does.
    pub fn take_console(&self) -> Option<(ProcessHandle, flume::Receiver<ProcessEvent>)> {
        let mut data = lock(&self.inner.data);
        let events = data.console.take()?;
        Some((data.debuggee.clone()?, events))
    }

    fn client(&self) -> Option<Client> {
        self.inner.client.get().cloned()
    }

    fn emit(&self, event: DebuggerEvent) {
        if let Some(debugger) = self.inner.debugger.upgrade() {
            debugger.emit(event);
        }
    }

    fn changed(&self) {
        self.emit(DebuggerEvent::SessionChanged(self.inner.id));
    }

    fn set_phase_if(&self, from: Phase, to: Phase) {
        let changed = {
            let mut data = lock(&self.inner.data);
            let ok = data.phase == from;
            if ok {
                data.phase = to;
            }
            ok
        };
        if changed {
            self.changed();
        }
    }

    /// The session failed: the message is kept and the adapter stopped.
    fn fail(&self, message: String) {
        {
            let mut data = lock(&self.inner.data);
            if data.error.is_none() {
                data.error = Some(message);
            }
        }
        self.stop();
    }

    // ---- breakpoints -------------------------------------------------------

    /// Sends the breakpoints of these files again (they changed).
    pub(crate) fn sync_breakpoints(&self, files: &[PathBuf]) {
        if !lock(&self.inner.data).configured {
            return;
        }
        for file in files {
            self.send_breakpoints(file);
        }
    }

    /// Sends the exception breakpoints again.
    pub(crate) fn sync_exceptions(&self) {
        if lock(&self.inner.data).configured {
            self.send_exceptions();
        }
    }

    fn send_breakpoints(&self, file: &Path) {
        let (Some(client), Some(debugger)) = (self.client(), self.inner.debugger.upgrade()) else {
            return;
        };
        let (muted, breakpoints) = debugger.breakpoints_of(file);
        let capabilities = client.capabilities();
        let sent: Vec<(BreakpointId, SourceBreakpoint)> = if muted {
            Vec::new()
        } else {
            breakpoints
                .iter()
                .filter_map(|b| source_breakpoint(b, capabilities.log_points).map(|s| (b.id, s)))
                .collect()
        };
        let ids: Vec<BreakpointId> = sent.iter().map(|(id, _)| *id).collect();
        {
            let mut data = lock(&self.inner.data);
            if let Some(old) = data.sent.insert(file.to_owned(), ids.clone()) {
                for id in old {
                    data.statuses.remove(&id);
                }
            }
            data.adapter_ids.retain(|_, id| !ids.contains(id));
        }
        let list: Vec<SourceBreakpoint> = sent.into_iter().map(|(_, s)| s).collect();
        let weak = Arc::downgrade(&self.inner);
        client.set_breakpoints(file, &list).on_done(move |result| {
            let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                return;
            };
            {
                let mut data = lock(&session.inner.data);
                match result {
                    Ok(answers) => {
                        for (id, answer) in ids.iter().zip(answers) {
                            if let Some(adapter_id) = answer.id {
                                data.adapter_ids.insert(adapter_id, *id);
                            }
                            data.statuses.insert(
                                *id,
                                BreakpointStatus {
                                    verified: answer.verified,
                                    message: answer.message,
                                    line: answer.line,
                                },
                            );
                        }
                    }
                    Err(error) => {
                        for id in &ids {
                            data.statuses.insert(
                                *id,
                                BreakpointStatus {
                                    verified: false,
                                    message: Some(error.to_string()),
                                    line: None,
                                },
                            );
                        }
                    }
                }
            }
            session.emit(DebuggerEvent::BreakpointsChanged);
        });
    }

    fn send_exceptions(&self) {
        let (Some(client), Some(debugger)) = (self.client(), self.inner.debugger.upgrade()) else {
            return;
        };
        let filters = debugger.exception_filters(&client.capabilities().exception_filters);
        client.set_exception_breakpoints(&filters).detach();
    }

    /// `initialized`: the breakpoints, the exception filters, then
    /// `configurationDone`.
    fn configure(&self) {
        let Some(client) = self.client() else {
            return;
        };
        lock(&self.inner.data).configured = true;
        let files = self
            .inner
            .debugger
            .upgrade()
            .map(|d| d.breakpoint_files())
            .unwrap_or_default();
        for file in &files {
            self.send_breakpoints(file);
        }
        self.send_exceptions();
        let weak = Arc::downgrade(&self.inner);
        let done = move |result: Result<(), DapError>| {
            let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                return;
            };
            if let Err(error) = result {
                if error != DapError::Closed {
                    session.fail(format!("The debugger could not start: {error}"));
                }
                return;
            }
            let started = {
                let mut data = lock(&session.inner.data);
                data.configuration_done = true;
                data.launched
            };
            if started {
                session.set_phase_if(Phase::Starting, Phase::Running);
            }
        };
        if client.capabilities().configuration_done {
            client.configuration_done().on_done(done);
        } else {
            done(Ok(()));
        }
    }

    // ---- events ----------------------------------------------------------------

    fn stopped(&self, stopped: dap_client::Stopped) {
        let (generation, thread, after_step, hits) = {
            let mut data = lock(&self.inner.data);
            if matches!(data.phase, Phase::Stopping | Phase::Ended) {
                return;
            }
            data.generation += 1;
            data.phase = Phase::Suspended;
            data.frames.clear();
            data.scopes.clear();
            data.children.clear();
            data.selected_frame = None;
            if stopped.thread_id.is_some() {
                data.selected_thread = stopped.thread_id;
            }
            data.stop = Some(StopInfo {
                reason: stopped.reason.clone(),
                thread_id: stopped.thread_id,
                description: stopped.description.clone(),
                text: stopped.text.clone(),
                all_threads: stopped.all_threads_stopped,
                exception: None,
            });
            let hits: Vec<BreakpointId> = stopped
                .hit_breakpoint_ids
                .iter()
                .filter_map(|id| data.adapter_ids.get(id).copied())
                .collect();
            (
                data.generation,
                data.selected_thread,
                data.after_step.take(),
                hits,
            )
        };
        self.changed();
        let Some(client) = self.client() else {
            return;
        };
        if let Some((command, arguments)) = after_step {
            client.custom(&command, arguments).detach();
        }
        if let Some(debugger) = self.inner.debugger.upgrade() {
            debugger.remove_temporary(&hits);
        }
        if stopped.reason == "exception"
            && let Some(thread) = thread
            && client.capabilities().exception_info
        {
            let weak = Arc::downgrade(&self.inner);
            client.exception_info(thread).on_done(move |result| {
                let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                    return;
                };
                if let Ok(info) = result {
                    let mut data = lock(&session.inner.data);
                    if data.generation == generation
                        && let Some(stop) = &mut data.stop
                    {
                        stop.exception = Some(info);
                    }
                    drop(data);
                    session.changed();
                }
            });
        }
        self.refresh_threads(generation, true);
    }

    /// Asks for the threads; with `frames`, then for the frames of the
    /// selected thread.
    fn refresh_threads(&self, generation: u64, frames: bool) {
        let Some(client) = self.client() else {
            return;
        };
        let weak = Arc::downgrade(&self.inner);
        client.threads().on_done(move |result| {
            let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                return;
            };
            let Ok(threads) = result else {
                return;
            };
            let thread = {
                let mut data = lock(&session.inner.data);
                if data.generation != generation {
                    return;
                }
                if data
                    .selected_thread
                    .is_none_or(|t| !threads.iter().any(|x| x.id == t))
                {
                    data.selected_thread = threads.first().map(|t| t.id);
                }
                data.threads = threads;
                data.selected_thread
            };
            session.changed();
            if frames
                && let Some(thread) = thread
                && session.phase() == Phase::Suspended
            {
                session.load_frames(thread, generation).detach_now();
            }
        });
    }

    fn load_frames(&self, thread: i64, generation: u64) -> Pending<()> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let start = lock(&self.inner.data)
            .frames
            .get(&thread)
            .map_or(0, |f| f.frames.len());
        let weak = Arc::downgrade(&self.inner);
        track(
            client.stack_trace(thread, start, FRAME_PAGE),
            move |result| {
                let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                    return;
                };
                let Ok(trace) = result else {
                    return;
                };
                {
                    let mut data = lock(&session.inner.data);
                    if data.generation != generation {
                        return;
                    }
                    let select_top = data.selected_frame.is_none()
                        && data.selected_thread == Some(thread)
                        && start == 0;
                    if select_top {
                        data.selected_frame = trace.frames.first().map(|f| f.id);
                    }
                    let entry = data.frames.entry(thread).or_default();
                    if entry.frames.len() == start {
                        let got = trace.frames.len();
                        entry.frames.extend(trace.frames.iter().cloned());
                        entry.total = trace.total.or(if got < FRAME_PAGE {
                            Some(entry.frames.len())
                        } else {
                            None
                        });
                    }
                }
                session.changed();
            },
        )
        .map(|r| r.map(|_| ()))
        .boxed()
    }

    fn resumed(&self) {
        {
            let mut data = lock(&self.inner.data);
            if data.phase != Phase::Suspended {
                return;
            }
            data.generation += 1;
            data.phase = Phase::Running;
            data.stop = None;
            data.frames.clear();
            data.scopes.clear();
            data.children.clear();
            data.selected_frame = None;
        }
        self.changed();
    }

    fn handle_event(&self, event: Event) {
        match event {
            Event::Initialized => self.configure(),
            Event::Stopped(stopped) => self.stopped(stopped),
            Event::Continued {
                thread_id,
                all_threads,
            } => {
                let stopped_thread = lock(&self.inner.data)
                    .stop
                    .as_ref()
                    .and_then(|s| s.thread_id);
                if all_threads || stopped_thread.is_none_or(|t| t == thread_id) {
                    self.resumed();
                }
            }
            Event::Exited { exit_code } => {
                lock(&self.inner.data).exit_code = Some(exit_code);
                self.changed();
            }
            Event::Terminated => {
                self.stop();
            }
            Event::ThreadStarted(_) | Event::ThreadExited(_) => {
                let generation = lock(&self.inner.data).generation;
                self.refresh_threads(generation, false);
            }
            Event::Output(output) => {
                if output.category != "telemetry" {
                    self.emit(DebuggerEvent::Output {
                        session: self.inner.id,
                        category: output.category,
                        text: output.text,
                    });
                }
            }
            Event::Breakpoint { breakpoint, .. } => {
                let found = {
                    let mut data = lock(&self.inner.data);
                    let id = breakpoint
                        .id
                        .and_then(|adapter_id| data.adapter_ids.get(&adapter_id).copied());
                    if let Some(id) = id {
                        data.statuses.insert(
                            id,
                            BreakpointStatus {
                                verified: breakpoint.verified,
                                message: breakpoint.message,
                                line: breakpoint.line,
                            },
                        );
                    }
                    id.is_some()
                };
                if found {
                    self.emit(DebuggerEvent::BreakpointsChanged);
                }
            }
            Event::Process { pid, .. } => {
                let mut data = lock(&self.inner.data);
                if data.debuggee_pid.is_none() {
                    data.debuggee_pid = pid;
                }
            }
            Event::Invalidated => {
                let (suspended, generation) = {
                    let mut data = lock(&self.inner.data);
                    let suspended = data.phase == Phase::Suspended;
                    if suspended {
                        data.generation += 1;
                        data.frames.clear();
                        data.scopes.clear();
                        data.children.clear();
                    }
                    (suspended, data.generation)
                };
                if suspended {
                    self.refresh_threads(generation, true);
                }
            }
            Event::Capabilities(_) | Event::Other { .. } => {}
        }
    }

    fn run_in_terminal(&self, request: RunInTerminal, responder: Responder) {
        let Some((program, args)) = request.args.split_first() else {
            responder.run_in_terminal(Err("No command line to run".to_owned()));
            return;
        };
        let mut command = CommandLine::new(program).args(args);
        if !request.cwd.as_os_str().is_empty() {
            command = command.current_dir(&request.cwd);
        }
        for (name, value) in &request.env {
            command = match value {
                Some(value) => command.with_env(name, value),
                None => command.without_env(name),
            };
        }
        match process::spawn(&command) {
            Ok((handle, events)) => {
                let pid = handle.pid();
                {
                    let mut data = lock(&self.inner.data);
                    data.debuggee = Some(handle);
                    data.debuggee_pid = Some(pid);
                    data.console = Some(events);
                }
                responder.run_in_terminal(Ok((Some(pid), None)));
                self.emit(DebuggerEvent::Console(self.inner.id));
            }
            Err(error) => {
                responder.run_in_terminal(Err(format!("Cannot run {program}: {error}")));
            }
        }
    }

    fn closed(&self, status: Option<ExitStatus>) {
        let debuggee = {
            let mut data = lock(&self.inner.data);
            if data.phase == Phase::Ended {
                return;
            }
            let crashed = !data.stopping && status.is_some_and(|s| !s.success());
            if crashed && data.error.is_none() {
                let mut message = match status {
                    Some(status) => format!("The debugger {}", exit_text(&status)),
                    None => "The debugger ended".to_owned(),
                };
                let tail = data.log_tail.trim();
                if !tail.is_empty() {
                    message.push_str(":\n");
                    message.push_str(tail);
                }
                data.error = Some(message);
            }
            data.phase = Phase::Ended;
            data.stop = None;
            data.debuggee.clone()
        };
        // The program never outlives its adapter.
        if let Some(debuggee) = debuggee
            && debuggee.is_running()
        {
            thread::Builder::new()
                .name("debug cleanup".to_owned())
                .spawn(move || {
                    wait_exit(&debuggee, STOP_TIMEOUT);
                    let _ = debuggee.kill();
                })
                .ok();
        }
        self.emit(DebuggerEvent::SessionEnded(self.inner.id));
    }

    // ---- actions -----------------------------------------------------------------

    /// The thread the program stopped in, or the selected one.
    fn thread(&self) -> Result<(Client, i64), DebugError> {
        let client = self.client().ok_or(DebugError::Ended)?;
        let data = lock(&self.inner.data);
        if data.phase != Phase::Suspended {
            return Err(DebugError::NotSuspended);
        }
        let thread = data
            .selected_thread
            .or_else(|| data.stop.as_ref().and_then(|s| s.thread_id))
            .or_else(|| data.threads.first().map(|t| t.id))
            .ok_or(DebugError::NotSuspended)?;
        Ok((client, thread))
    }

    fn run(&self, request: impl FnOnce(&Client, i64) -> Request<()>) -> Pending<()> {
        let (client, thread) = match self.thread() {
            Ok(found) => found,
            Err(error) => return ready(Err(error)),
        };
        let weak = Arc::downgrade(&self.inner);
        track(request(&client, thread), move |result| {
            if result.is_ok()
                && let Some(inner) = weak.upgrade()
            {
                Session { inner }.resumed();
            }
        })
    }

    /// Resume Program.
    pub fn resume(&self) -> Pending<()> {
        let (client, thread) = match self.thread() {
            Ok(found) => found,
            Err(error) => return ready(Err(error)),
        };
        let weak = Arc::downgrade(&self.inner);
        track(client.resume(thread), move |result| {
            if result.is_ok()
                && let Some(inner) = weak.upgrade()
            {
                Session { inner }.resumed();
            }
        })
        .map(|r| r.map(|_| ()))
        .boxed()
    }

    pub fn step_over(&self) -> Pending<()> {
        self.run(|client, thread| client.step_over(thread))
    }

    pub fn step_into(&self) -> Pending<()> {
        self.run(|client, thread| client.step_into(thread))
    }

    /// Step Into My Code: the product's request before the step and after
    /// the next stop, or a plain Step Into.
    pub fn step_into_my_code(&self) -> Pending<()> {
        if let Some(step) = &self.inner.step_into_my_code
            && let Ok((client, _)) = self.thread()
        {
            client
                .custom(&step.before.0, step.before.1.clone())
                .detach();
            lock(&self.inner.data).after_step = Some(step.after.clone());
        }
        self.step_into()
    }

    pub fn step_out(&self) -> Pending<()> {
        self.run(|client, thread| client.step_out(thread))
    }

    /// Pause Program: suspends the selected thread (the adapter suspends
    /// all of them).
    pub fn pause(&self) -> Pending<()> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let thread = {
            let data = lock(&self.inner.data);
            if data.phase != Phase::Running {
                return ready(Err(DebugError::Unsupported(
                    "pausing a program that is not running".to_owned(),
                )));
            }
            data.selected_thread
                .or_else(|| data.threads.first().map(|t| t.id))
        };
        match thread {
            Some(thread) => track(client.pause(thread), |_| {}),
            None => {
                // Threads are not known yet: ask, then pause the first.
                let (tx, rx) = oneshot::channel();
                let paused = client.clone();
                client.threads().on_done(move |result| match result {
                    Ok(threads) => match threads.first() {
                        Some(thread) => paused.pause(thread.id).on_done(move |r| {
                            let _ = tx.send(r.map_err(DebugError::from));
                        }),
                        None => {
                            let _ = tx.send(Err(DebugError::NotSuspended));
                        }
                    },
                    Err(error) => {
                        let _ = tx.send(Err(error.into()));
                    }
                });
                rx.map(|r| r.unwrap_or(Err(DebugError::Ended))).boxed()
            }
        }
    }

    /// Stop: ends the program and the adapter (`terminate`, then
    /// `disconnect`); whatever still runs after [`STOP_TIMEOUT`] is
    /// killed. The receiver gets a message when every process is gone.
    pub fn stop(&self) -> flume::Receiver<()> {
        let (done, finished) = flume::bounded(1);
        let (client, debuggee, first) = {
            let mut data = lock(&self.inner.data);
            let first = !data.stopping;
            data.stopping = true;
            if data.phase != Phase::Ended {
                data.phase = Phase::Stopping;
            }
            (self.client(), data.debuggee.clone(), first)
        };
        if first {
            self.changed();
        }
        let spawned = thread::Builder::new()
            .name("debug stop".to_owned())
            .spawn(move || {
                if let Some(client) = &client {
                    let _ = client.shutdown(true, STOP_TIMEOUT).recv();
                }
                if let Some(debuggee) = debuggee {
                    wait_exit(&debuggee, Duration::from_millis(500));
                    let _ = debuggee.kill();
                    wait_exit(&debuggee, STOP_TIMEOUT);
                }
                let _ = done.send(());
            });
        if spawned.is_err()
            && let Some(client) = self.client()
        {
            client.kill();
        }
        finished
    }

    pub fn select_thread(&self, thread: i64) {
        let generation = {
            let mut data = lock(&self.inner.data);
            if data.selected_thread == Some(thread) {
                return;
            }
            data.selected_thread = Some(thread);
            data.selected_frame = data
                .frames
                .get(&thread)
                .and_then(|f| f.frames.first())
                .map(|f| f.id);
            if data.frames.contains_key(&thread) || data.phase != Phase::Suspended {
                None
            } else {
                Some(data.generation)
            }
        };
        self.changed();
        if let Some(generation) = generation {
            self.load_frames(thread, generation).detach_now();
        }
    }

    pub fn select_frame(&self, frame: i64) {
        lock(&self.inner.data).selected_frame = Some(frame);
        self.changed();
    }

    /// Loads the next page of the selected thread's frames.
    pub fn load_more_frames(&self) -> Pending<()> {
        let (thread, generation) = {
            let data = lock(&self.inner.data);
            match data.selected_thread {
                Some(thread) if data.phase == Phase::Suspended => (thread, data.generation),
                _ => return ready(Err(DebugError::NotSuspended)),
            }
        };
        self.load_frames(thread, generation)
    }

    /// The scopes of a frame (kept until the program moves on).
    pub fn scopes(&self, frame: i64) -> Pending<Vec<Scope>> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let generation = {
            let data = lock(&self.inner.data);
            if let Some(scopes) = data.scopes.get(&frame) {
                return ready(Ok(scopes.clone()));
            }
            if data.phase != Phase::Suspended {
                return ready(Err(DebugError::NotSuspended));
            }
            data.generation
        };
        let weak = Arc::downgrade(&self.inner);
        track(client.scopes(frame), move |result| {
            if let (Ok(scopes), Some(inner)) = (result, weak.upgrade()) {
                let mut data = lock(&inner.data);
                if data.generation == generation {
                    data.scopes.insert(frame, scopes.clone());
                }
            }
        })
    }

    /// The loaded children of a scope or a variable (`reference`).
    pub fn loaded_children(&self, reference: i64) -> Option<Children> {
        lock(&self.inner.data).children.get(&reference).cloned()
    }

    /// Loads the next page of children of a scope or a variable and
    /// returns all loaded so far.
    pub fn children(&self, reference: i64) -> Pending<Children> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let (start, generation) = {
            let data = lock(&self.inner.data);
            let loaded = data.children.get(&reference);
            if let Some(loaded) = loaded
                && !loaded.more
            {
                return ready(Ok(loaded.clone()));
            }
            if data.phase != Phase::Suspended {
                return ready(Err(DebugError::NotSuspended));
            }
            (loaded.map_or(0, |c| c.items.len()), data.generation)
        };
        let weak = Arc::downgrade(&self.inner);
        let (tx, rx) = oneshot::channel();
        client
            .variables(
                reference,
                VariablesFilter::All,
                Some((start, VARIABLES_PAGE)),
            )
            .on_done(move |result| {
                let result = result.map_err(DebugError::from).map(|items| {
                    let more = items.len() >= VARIABLES_PAGE;
                    let Some(inner) = weak.upgrade() else {
                        return Children { items, more };
                    };
                    let mut data = lock(&inner.data);
                    if data.generation != generation {
                        return Children { items, more };
                    }
                    let entry = data.children.entry(reference).or_default();
                    if entry.items.len() == start {
                        entry.items.extend(items);
                    }
                    entry.more = more;
                    entry.clone()
                });
                let _ = tx.send(result);
            });
        rx.map(|r| r.unwrap_or(Err(DebugError::Ended))).boxed()
    }

    /// Evaluates an expression in the selected frame. Dropping the future
    /// cancels the evaluation.
    pub fn evaluate(&self, expression: &str, context: EvaluateContext) -> Pending<Evaluated> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let frame = lock(&self.inner.data).selected_frame;
        client
            .evaluate(expression, frame, context)
            .map(|r| r.map_err(DebugError::from))
            .boxed()
    }

    /// Set Value: a child `variable` of `parent` (a scope or a variable)
    /// gets the value of an expression. Loaded values are dropped after it
    /// (others may have changed).
    pub fn set_value(&self, parent: i64, variable: &Variable, value: &str) -> Pending<Evaluated> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let capabilities = client.capabilities();
        let frame = lock(&self.inner.data).selected_frame;
        let request = if capabilities.set_variable && parent > 0 {
            client.set_variable(parent, &variable.name, value)
        } else if let (true, Some(name)) = (capabilities.set_expression, &variable.evaluate_name) {
            client.set_expression(name, value, frame)
        } else {
            return ready(Err(DebugError::Unsupported("setting values".to_owned())));
        };
        let weak = Arc::downgrade(&self.inner);
        track(request, move |result| {
            if result.is_ok()
                && let Some(inner) = weak.upgrade()
            {
                {
                    let mut data = lock(&inner.data);
                    data.children.clear();
                }
                Session { inner }.changed();
            }
        })
    }

    /// The variable at a path of names from a scope of the selected frame
    /// (`["items", "0"]`), with the reference of its parent; children are
    /// loaded on the way.
    pub fn find_variable(&self, path: Vec<String>) -> Pending<(i64, Variable)> {
        let session = self.clone();
        async move {
            let frame = lock(&session.inner.data)
                .selected_frame
                .ok_or(DebugError::NotSuspended)?;
            let Some((first, rest)) = path.split_first() else {
                return Err(DebugError::Adapter("an empty variable path".to_owned()));
            };
            let mut found = None;
            for scope in session.scopes(frame).await? {
                if let Some(variable) = session.child_named(scope.reference, first).await? {
                    found = Some((scope.reference, variable));
                    break;
                }
            }
            let (mut parent, mut variable) =
                found.ok_or_else(|| DebugError::Adapter(format!("No variable {first}")))?;
            for name in rest {
                let next = session
                    .child_named(variable.reference, name)
                    .await?
                    .ok_or_else(|| DebugError::Adapter(format!("No variable {name}")))?;
                parent = variable.reference;
                variable = next;
            }
            Ok((parent, variable))
        }
        .boxed()
    }

    async fn child_named(
        &self,
        reference: i64,
        name: &str,
    ) -> Result<Option<Variable>, DebugError> {
        if reference <= 0 {
            return Ok(None);
        }
        loop {
            let children = self.children(reference).await?;
            if let Some(found) = children.items.iter().find(|v| v.name == name) {
                return Ok(Some(found.clone()));
            }
            if !children.more {
                return Ok(None);
            }
        }
    }

    /// The loaded scopes of the selected frame with their loaded children,
    /// as JSON (`debug.state`).
    pub fn variables_json(&self) -> Value {
        let data = lock(&self.inner.data);
        let Some(scopes) = data.selected_frame.and_then(|f| data.scopes.get(&f)) else {
            return Value::Array(Vec::new());
        };
        fn children(data: &Data, reference: i64, depth: usize) -> Option<Value> {
            let loaded = data.children.get(&reference)?;
            if depth > 16 {
                return None;
            }
            let items: Vec<Value> = loaded
                .items
                .iter()
                .map(|v| {
                    let mut item = serde_json::json!({
                        "name": v.name,
                        "value": v.value,
                        "type": v.type_name,
                        "has_children": v.reference > 0,
                    });
                    if let Some(c) = children(data, v.reference, depth + 1) {
                        item["children"] = c;
                    }
                    item
                })
                .collect();
            Some(Value::Array(items))
        }
        Value::Array(
            scopes
                .iter()
                .map(|s| {
                    let mut scope = serde_json::json!({"scope": s.name});
                    if let Some(c) = children(&data, s.reference, 0) {
                        scope["children"] = c;
                    }
                    scope
                })
                .collect(),
        )
    }
}

/// A line breakpoint as the adapter takes it; `None` for one that is
/// disabled or does nothing the adapter can do (a breakpoint that neither
/// suspends nor logs).
fn source_breakpoint(b: &LineBreakpoint, log_points: bool) -> Option<SourceBreakpoint> {
    if !b.enabled || (b.suspend == SuspendPolicy::None && !b.logs_only()) {
        return None;
    }
    let mut out = SourceBreakpoint::at(b.line);
    out.condition = b.condition.clone();
    if b.logs_only() {
        if !log_points {
            return None;
        }
        let mut message = Vec::new();
        if b.log_message {
            let name = b
                .file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            message.push(format!("Breakpoint reached at {name}:{}", b.line));
        }
        if let Some(expression) = &b.log_expression {
            message.push(format!("{{{expression}}}"));
        }
        if message.is_empty() {
            return None;
        }
        out.log_message = Some(message.join("\n"));
    }
    Some(out)
}

/// The request as a future, with `then` run on the reader thread first.
fn track<T: Clone + Send + 'static>(
    request: Request<T>,
    then: impl FnOnce(Result<T, DapError>) + Send + 'static,
) -> Pending<T> {
    let (tx, rx) = oneshot::channel();
    request.on_done(move |result| {
        then(result.clone());
        let _ = tx.send(result.map_err(DebugError::from));
    });
    rx.map(|r| r.unwrap_or(Err(DebugError::Ended))).boxed()
}

trait DetachNow {
    /// Lets the request run without waiting for it.
    fn detach_now(self);
}

impl<T> DetachNow for Pending<T> {
    fn detach_now(self) {
        // The work is done in the request's callback; the future only
        // carries the answer.
        drop(self);
    }
}

fn exit_text(status: &ExitStatus) -> String {
    match (status.code, status.signal) {
        (Some(code), _) => format!("exited with code {code}"),
        (None, Some(signal)) => format!("was killed by signal {signal}"),
        (None, None) => "ended".to_owned(),
    }
}

fn wait_exit(process: &ProcessHandle, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while process.is_running() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

struct SessionHandler(Weak<SessionInner>);

impl SessionHandler {
    fn session(&self) -> Option<Session> {
        self.0.upgrade().map(|inner| Session { inner })
    }
}

impl Handler for SessionHandler {
    fn event(&self, event: Event) {
        if let Some(session) = self.session() {
            session.handle_event(event);
        }
    }

    fn reverse_request(&self, request: ReverseRequest, responder: Responder) {
        let Some(session) = self.session() else {
            return;
        };
        match request {
            ReverseRequest::RunInTerminal(request) => session.run_in_terminal(request, responder),
            // Subprocesses come later; the adapter gets an error answer.
            ReverseRequest::StartDebugging { .. } | ReverseRequest::Other { .. } => drop(responder),
        }
    }

    fn stderr(&self, chunk: &[u8]) {
        if let Some(session) = self.session() {
            let mut data = lock(&session.inner.data);
            data.log_tail.push_str(&String::from_utf8_lossy(chunk));
            if data.log_tail.len() > LOG_TAIL {
                let mut cut = data.log_tail.len() - LOG_TAIL;
                while !data.log_tail.is_char_boundary(cut) {
                    cut += 1;
                }
                data.log_tail.drain(..cut);
            }
        }
    }

    fn closed(&self, status: Option<ExitStatus>) {
        if let Some(session) = self.session() {
            session.closed(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A breakpoint that neither suspends nor logs does nothing: it is not
    /// sent (the program runs past it); a logging one is a log point.
    #[test]
    fn breakpoints_that_do_nothing_are_not_sent() {
        let mut b = LineBreakpoint::new("/p/main.py", 3);
        assert!(source_breakpoint(&b, true).is_some());
        b.suspend = SuspendPolicy::None;
        assert!(source_breakpoint(&b, true).is_none());
        b.log_message = true;
        let sent = source_breakpoint(&b, true).unwrap();
        assert_eq!(
            sent.log_message.as_deref(),
            Some("Breakpoint reached at main.py:3")
        );
        assert!(source_breakpoint(&b, false).is_none());
        b.enabled = false;
        assert!(source_breakpoint(&b, true).is_none());
    }
}
