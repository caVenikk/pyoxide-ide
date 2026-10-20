//! One debug session (the reference's `XDebugSession`) over a DAP client:
//! its state machine, threads, frames in pages, scopes, variables loaded
//! lazily in pages, evaluation and setting values, breakpoints sent to the
//! adapter with their verification, and the debuggee started through
//! `runInTerminal`.
//!
//! Subprocesses of the program that the adapter debugs come as child
//! sessions (`startDebugging`), each over a connection of its own; they
//! are processes of this session, as the reference's multiprocess
//! debugger keeps them in one session: breakpoints go to every process,
//! the session shows the process that stopped (one that stops while
//! another is shown waits until that one moves on; while the shown one
//! steps, it waits unless it stopped at a breakpoint), and Stop ends them
//! all.
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
//! of the previous stop) are dropped, as is the answer to a resume or a
//! step handled after the next stop (the client runs a callback attached
//! late on the caller's thread, after the events that followed it).

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
    /// The sizes of the items that have one (a container's length, shown
    /// with its type), by item; empty when the product shows none.
    pub shapes: Vec<Option<String>>,
    /// More children can be loaded ([`Session::children`] again).
    pub more: bool,
}

impl Children {
    /// The size of the item at `index`.
    pub fn shape(&self, index: usize) -> Option<&str> {
        self.shapes.get(index).and_then(|s| s.as_deref())
    }
}

/// What a session shows, copied at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub id: SessionId,
    pub name: String,
    pub watches_key: String,
    pub phase: Phase,
    /// Grows whenever the adapter's frames and values may have changed (a
    /// stop, a resume, values invalidated): a frame of another generation
    /// is another stop, even with the same id.
    pub generation: u64,
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
    /// Stops shown so far: the answer to a resume or a step sent at one
    /// stop does not move the program on from a later one. (A callback
    /// attached after its answer came runs at once on the caller's thread,
    /// so it may run after the events that followed the answer, the next
    /// stop among them.)
    stops: u64,
    threads: Vec<Thread>,
    stop: Option<StopInfo>,
    frames: HashMap<i64, Frames>,
    selected_thread: Option<i64>,
    selected_frame: Option<i64>,
    scopes: HashMap<i64, Vec<Scope>>,
    children: HashMap<i64, Children>,
    /// The frame a scope's, a variable's or a value's children belong to
    /// (their sizes are evaluated there).
    reference_frames: HashMap<i64, i64>,
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
    /// The program's command line as its console shows it first.
    console_command: Option<String>,
    /// Sent at the next stop (after Step Into My Code).
    after_step: Option<(String, Value)>,
    /// Run to Cursor: where the program runs to.
    run_to: Option<RunTo>,
    /// The program's subprocesses debugged in the session.
    subprocesses: Vec<Subprocess>,
    next_subprocess: u64,
    /// Stops of processes that came while another process is shown
    /// suspended (or steps), oldest first.
    waiting: Vec<(Process, dap_client::Stopped)>,
    /// A step of the shown process is on its way: until its stop, the stop
    /// of another process waits unless it is at a breakpoint.
    stepping: bool,
    /// The product's preparation of steps was sent to the program.
    steps_prepared: bool,
}

/// A process of the session: the program, or one of its subprocesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Process {
    Program,
    Subprocess(u64),
}

/// A subprocess of the program: a child session of the adapter over a
/// connection of its own.
struct Subprocess {
    key: u64,
    client: Client,
    /// `initialized` came: breakpoint changes are sent to it.
    configured: bool,
    /// Its breakpoint ids.
    adapter_ids: HashMap<i64, BreakpointId>,
    /// The product's preparation of steps was sent to it.
    steps_prepared: bool,
}

/// Run to Cursor's target: a breakpoint of the session only, on top of the
/// file's breakpoints, or alone (Force Run to Cursor ignores the others).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunTo {
    file: PathBuf,
    line: u32,
    ignore_breakpoints: bool,
}

pub(crate) struct SessionInner {
    id: SessionId,
    name: String,
    watches_key: String,
    debugger: Weak<DebuggerInner>,
    /// The program's connection.
    client: OnceLock<Client>,
    /// The adapter's type, for the subprocesses' `initialize`.
    adapter_id: OnceLock<String>,
    /// The subprocess the session shows (its key and connection), `None`
    /// for the program: requests about threads, frames and values and the
    /// stepping actions go to its connection. Locked after `data`, never
    /// before it.
    focus: Mutex<Option<(u64, Client)>>,
    step_into_my_code: Option<StepIntoMyCode>,
    /// Sent to each process before its first step.
    prepare_steps: Option<(String, Value)>,
    /// "Redirect input from": the file the program reads.
    input_file: Option<PathBuf>,
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
        prepare_steps: Option<(String, Value)>,
        input_file: Option<PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(SessionInner {
                id,
                name,
                watches_key,
                debugger,
                client: OnceLock::new(),
                adapter_id: OnceLock::new(),
                focus: Mutex::new(None),
                step_into_my_code,
                prepare_steps,
                input_file,
                data: Mutex::new(Data {
                    phase: Phase::Starting,
                    configured: false,
                    launched: false,
                    configuration_done: false,
                    stopping: false,
                    generation: 0,
                    stops: 0,
                    threads: Vec::new(),
                    stop: None,
                    frames: HashMap::new(),
                    selected_thread: None,
                    selected_frame: None,
                    scopes: HashMap::new(),
                    children: HashMap::new(),
                    reference_frames: HashMap::new(),
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
                    console_command: None,
                    after_step: None,
                    run_to: None,
                    subprocesses: Vec::new(),
                    next_subprocess: 0,
                    waiting: Vec::new(),
                    stepping: false,
                    steps_prepared: false,
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
        let handler = Arc::new(SessionHandler {
            session: Arc::downgrade(&self.inner),
            process: Process::Program,
        });
        let client = Client::start(&self.inner.name, transport, handler)
            .map_err(|e| format!("Cannot start the debugger: {e}"))?;
        let _ = self.inner.client.set(client.clone());
        let _ = self.inner.adapter_id.set(launch.adapter_id.clone());
        let weak = Arc::downgrade(&self.inner);
        let kind = launch.kind;
        let arguments = launch.arguments;
        client
            .initialize(&self.client_info())
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
            generation: data.generation,
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

    /// The command line the program was started with (`runInTerminal`), as
    /// its console shows it first.
    pub fn console_command(&self) -> Option<String> {
        lock(&self.inner.data).console_command.clone()
    }

    /// "Redirect input from": the file the program's standard input comes
    /// from.
    pub fn input_file(&self) -> Option<&Path> {
        self.inner.input_file.as_deref()
    }

    /// The program's process, while it runs.
    pub fn debuggee(&self) -> Option<ProcessHandle> {
        lock(&self.inner.data).debuggee.clone()
    }

    /// The connection of the process the session shows.
    fn client(&self) -> Option<Client> {
        match &*lock(&self.inner.focus) {
            Some((_, client)) => Some(client.clone()),
            None => self.program_client(),
        }
    }

    /// The program's connection.
    fn program_client(&self) -> Option<Client> {
        self.inner.client.get().cloned()
    }

    /// The process the session shows.
    fn focus(&self) -> Process {
        match &*lock(&self.inner.focus) {
            Some((key, _)) => Process::Subprocess(*key),
            None => Process::Program,
        }
    }

    /// Shows a process (its connection is in `data`, which the caller has
    /// locked).
    fn set_focus(&self, data: &Data, process: Process) {
        let focus = match process {
            Process::Program => None,
            Process::Subprocess(key) => data
                .subprocesses
                .iter()
                .find(|s| s.key == key)
                .map(|s| (key, s.client.clone())),
        };
        *lock(&self.inner.focus) = focus;
    }

    /// The configured processes and their connections, the program first.
    fn targets(&self) -> Vec<(Process, Client)> {
        let data = lock(&self.inner.data);
        let program = self
            .program_client()
            .filter(|_| data.configured)
            .map(|c| (Process::Program, c));
        program
            .into_iter()
            .chain(
                data.subprocesses
                    .iter()
                    .filter(|s| s.configured)
                    .map(|s| (Process::Subprocess(s.key), s.client.clone())),
            )
            .collect()
    }

    /// What the client says of itself at `initialize`: child sessions
    /// when the product debugs them.
    fn client_info(&self) -> ClientInfo {
        let mut info = ClientInfo::new(self.inner.adapter_id.get().cloned().unwrap_or_default());
        info.start_debugging = self
            .inner
            .debugger
            .upgrade()
            .is_some_and(|d| d.provider().child_sessions());
        info
    }

    /// The number of subprocesses debugged in the session now.
    pub fn subprocesses(&self) -> usize {
        lock(&self.inner.data).subprocesses.len()
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

    /// Sends the breakpoints of a file to every process.
    fn send_breakpoints(&self, file: &Path) {
        let Some(ids) = self.breakpoints_to_send(file).map(|(ids, _)| ids) else {
            return;
        };
        {
            let mut data = lock(&self.inner.data);
            if let Some(old) = data.sent.insert(file.to_owned(), ids.clone()) {
                for id in old {
                    data.statuses.remove(&id);
                }
            }
            data.adapter_ids.retain(|_, id| !ids.contains(id));
            for subprocess in &mut data.subprocesses {
                subprocess.adapter_ids.retain(|_, id| !ids.contains(id));
            }
        }
        for (process, client) in self.targets() {
            self.send_breakpoints_to(process, &client, file);
        }
    }

    /// The breakpoints of a file as the adapter takes them, with their ids.
    fn breakpoints_to_send(
        &self,
        file: &Path,
    ) -> Option<(Vec<BreakpointId>, Vec<SourceBreakpoint>)> {
        let (Some(client), Some(debugger)) = (self.program_client(), self.inner.debugger.upgrade())
        else {
            return None;
        };
        let (muted, breakpoints) = debugger.breakpoints_of(file);
        let ignored = lock(&self.inner.data)
            .run_to
            .as_ref()
            .is_some_and(|r| r.ignore_breakpoints);
        let capabilities = client.capabilities();
        let sent: Vec<(BreakpointId, SourceBreakpoint)> = if muted || ignored {
            Vec::new()
        } else {
            breakpoints
                .iter()
                .filter_map(|b| source_breakpoint(b, capabilities.log_points).map(|s| (b.id, s)))
                .collect()
        };
        Some(sent.into_iter().unzip())
    }

    /// Sends the breakpoints of a file to one process; its answer is what
    /// the program's says, or what a subprocess says when it verified one
    /// (or nothing was said yet).
    fn send_breakpoints_to(&self, process: Process, client: &Client, file: &Path) {
        let Some((ids, mut list)) = self.breakpoints_to_send(file) else {
            return;
        };
        let run_to = lock(&self.inner.data).run_to.clone();
        // Run to Cursor's target suspends whatever a breakpoint of its line
        // says until the next stop; one of its own goes last, where its
        // answer has no breakpoint of the project to go to. Only the
        // process shown runs to it.
        if let Some(run_to) = run_to.filter(|r| r.file == file && self.focus() == process) {
            match list.iter_mut().find(|b| b.line == run_to.line) {
                Some(b) => *b = SourceBreakpoint::at(run_to.line),
                None => list.push(SourceBreakpoint::at(run_to.line)),
            }
        }
        let weak = Arc::downgrade(&self.inner);
        client.set_breakpoints(file, &list).on_done(move |result| {
            let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                return;
            };
            {
                let mut data = lock(&session.inner.data);
                let data = &mut *data;
                let statuses: Vec<(BreakpointId, Option<i64>, BreakpointStatus)> = match result {
                    Ok(answers) => ids
                        .iter()
                        .zip(answers)
                        .map(|(id, answer)| {
                            let status = BreakpointStatus {
                                verified: answer.verified,
                                message: answer.message,
                                line: answer.line,
                            };
                            (*id, answer.id, status)
                        })
                        .collect(),
                    Err(error) => ids
                        .iter()
                        .map(|id| {
                            let status = BreakpointStatus {
                                verified: false,
                                message: Some(error.to_string()),
                                line: None,
                            };
                            (*id, None, status)
                        })
                        .collect(),
                };
                let adapter_ids = match process {
                    Process::Program => Some(&mut data.adapter_ids),
                    Process::Subprocess(key) => data
                        .subprocesses
                        .iter_mut()
                        .find(|s| s.key == key)
                        .map(|s| &mut s.adapter_ids),
                };
                let Some(adapter_ids) = adapter_ids else {
                    return;
                };
                for (id, adapter_id, status) in statuses {
                    if let Some(adapter_id) = adapter_id {
                        adapter_ids.insert(adapter_id, id);
                    }
                    let keep = process != Process::Program
                        && !status.verified
                        && data.statuses.contains_key(&id);
                    if !keep {
                        data.statuses.insert(id, status);
                    }
                }
            }
            session.emit(DebuggerEvent::BreakpointsChanged);
        });
    }

    /// Sends the exception breakpoints to every process.
    fn send_exceptions(&self) {
        for (_, client) in self.targets() {
            self.send_exceptions_to(&client);
        }
    }

    fn send_exceptions_to(&self, client: &Client) {
        let Some(debugger) = self.inner.debugger.upgrade() else {
            return;
        };
        let ignored = lock(&self.inner.data)
            .run_to
            .as_ref()
            .is_some_and(|r| r.ignore_breakpoints);
        let filters = if ignored {
            Vec::new()
        } else {
            debugger.exception_filters(&client.capabilities().exception_filters)
        };
        client.set_exception_breakpoints(&filters).detach();
    }

    /// `initialized`: the breakpoints, the exception filters, then
    /// `configurationDone`.
    fn configure(&self) {
        let Some(client) = self.program_client() else {
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

    /// A process stopped: the session shows it, unless another process is
    /// shown suspended (then it waits for that one to move on) or steps
    /// (then it waits unless it stopped at a breakpoint, as the reference
    /// switches processes only for a breakpoint hit).
    fn stopped(&self, process: Process, stopped: dap_client::Stopped) {
        // Run to Cursor is over at the stop the session shows: the
        // breakpoints as they are go to the adapter before the stop shows,
        // so that what is done at the stop (another Run to Cursor) goes
        // after them.
        let run_to = {
            let mut data = lock(&self.inner.data);
            if matches!(data.phase, Phase::Stopping | Phase::Ended) {
                return;
            }
            if self.stop_waits(&data, process, &stopped) {
                None
            } else {
                data.run_to.take()
            }
        };
        if let Some(run_to) = run_to {
            self.restore_breakpoints(run_to);
        }
        let (generation, thread, after_step, hits, run_to) = {
            let mut data = lock(&self.inner.data);
            if matches!(data.phase, Phase::Stopping | Phase::Ended) {
                return;
            }
            let shown = self.focus();
            if self.stop_waits(&data, process, &stopped) {
                data.waiting.retain(|(p, _)| *p != process);
                data.waiting.push((process, stopped));
                return;
            }
            data.stepping = false;
            if shown != process {
                self.set_focus(&data, process);
                data.threads.clear();
                data.selected_thread = None;
            }
            data.generation += 1;
            data.stops += 1;
            data.phase = Phase::Suspended;
            data.frames.clear();
            data.scopes.clear();
            data.children.clear();
            data.reference_frames.clear();
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
            let adapter_ids = match process {
                Process::Program => Some(&data.adapter_ids),
                Process::Subprocess(key) => data
                    .subprocesses
                    .iter()
                    .find(|s| s.key == key)
                    .map(|s| &s.adapter_ids),
            };
            let hits: Vec<BreakpointId> = stopped
                .hit_breakpoint_ids
                .iter()
                .filter_map(|id| adapter_ids.and_then(|ids| ids.get(id)).copied())
                .collect();
            (
                data.generation,
                data.selected_thread,
                data.after_step.take(),
                hits,
                data.run_to.take(),
            )
        };
        self.changed();
        let Some(client) = self.client() else {
            return;
        };
        // A Run to Cursor that came meanwhile is over too.
        if let Some(run_to) = run_to {
            self.restore_breakpoints(run_to);
        }
        if let Some(debugger) = self.inner.debugger.upgrade() {
            debugger.remove_temporary(&hits);
        }
        let reason = stopped.reason;
        match after_step {
            // The frames are asked for once the product's request after
            // the step is answered (it may change what they show).
            Some((command, arguments)) => {
                let weak = Arc::downgrade(&self.inner);
                client.custom(&command, arguments).on_done(move |_| {
                    if let Some(inner) = weak.upgrade() {
                        Session { inner }.load_stop(&reason, thread, generation);
                    }
                });
            }
            None => self.load_stop(&reason, thread, generation),
        }
    }

    /// Whether a stop of `process` waits for the process shown (see
    /// [`Self::stopped`]).
    fn stop_waits(&self, data: &Data, process: Process, stopped: &dap_client::Stopped) -> bool {
        let waits =
            data.phase == Phase::Suspended || (data.stepping && !at_breakpoint(&stopped.reason));
        self.focus() != process && waits
    }

    /// Run to Cursor is over: the breakpoints as they are again.
    fn restore_breakpoints(&self, run_to: RunTo) {
        if run_to.ignore_breakpoints {
            let mut files = self
                .inner
                .debugger
                .upgrade()
                .map(|d| d.breakpoint_files())
                .unwrap_or_default();
            if !files.contains(&run_to.file) {
                files.push(run_to.file);
            }
            for file in &files {
                self.send_breakpoints(file);
            }
            self.send_exceptions();
        } else {
            self.send_breakpoints(&run_to.file);
        }
    }

    /// What a stop shows: the exception, the threads, then the frames of
    /// the selected thread.
    fn load_stop(&self, reason: &str, thread: Option<i64>, generation: u64) {
        let Some(client) = self.client() else {
            return;
        };
        if reason == "exception"
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

    /// The process shown moved on; a process that stopped meanwhile is
    /// shown next, unless the shown one steps (it is shown at the step's
    /// stop). `at`: the stop a resume or a step was sent at, when this is
    /// its answer (it is late when the program stopped again).
    fn resumed(&self, at: Option<u64>) {
        let next = {
            let mut data = lock(&self.inner.data);
            if data.phase != Phase::Suspended || at.is_some_and(|stop| stop != data.stops) {
                return;
            }
            data.generation += 1;
            data.phase = Phase::Running;
            data.stop = None;
            data.frames.clear();
            data.scopes.clear();
            data.children.clear();
            data.reference_frames.clear();
            data.selected_frame = None;
            (!data.stepping && !data.waiting.is_empty()).then(|| data.waiting.remove(0))
        };
        self.changed();
        if let Some((process, stopped)) = next {
            self.stopped(process, stopped);
        }
    }

    /// An event of a process: the program's, or a subprocess's.
    fn process_event(&self, process: Process, event: Event) {
        let shown = self.focus() == process;
        match event {
            Event::Stopped(stopped) => return self.stopped(process, stopped),
            // A waiting stop is over.
            Event::Continued {
                thread_id,
                all_threads,
            } if !shown => {
                lock(&self.inner.data).waiting.retain(|(p, s)| {
                    *p != process || !(all_threads || s.thread_id.is_none_or(|t| t == thread_id))
                });
                return;
            }
            // The threads of a process that is not shown.
            Event::ThreadStarted(_) | Event::ThreadExited(_) | Event::Invalidated if !shown => {
                return;
            }
            _ => {}
        }
        let Process::Subprocess(key) = process else {
            self.handle_event(event);
            return;
        };
        match event {
            Event::Initialized => self.configure_subprocess(key),
            Event::Continued { .. }
            | Event::ThreadStarted(_)
            | Event::ThreadExited(_)
            | Event::Invalidated
            | Event::Output(_) => self.handle_event(event),
            Event::Breakpoint { breakpoint, .. } => {
                let found = {
                    let mut data = lock(&self.inner.data);
                    let id = data
                        .subprocesses
                        .iter()
                        .find(|s| s.key == key)
                        .and_then(|s| breakpoint.id.and_then(|a| s.adapter_ids.get(&a)))
                        .copied();
                    if let Some(id) = id
                        && breakpoint.verified
                    {
                        data.statuses.insert(
                            id,
                            BreakpointStatus {
                                verified: true,
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
            // The subprocess ended: its connection is closed.
            Event::Terminated => self.end_subprocess(key),
            Event::Stopped(_)
            | Event::Exited { .. }
            | Event::Process { .. }
            | Event::Capabilities(_)
            | Event::Other { .. } => {}
        }
    }

    fn handle_event(&self, event: Event) {
        match event {
            Event::Initialized => self.configure(),
            Event::Stopped(stopped) => self.stopped(Process::Program, stopped),
            Event::Continued {
                thread_id,
                all_threads,
            } => {
                let stopped_thread = lock(&self.inner.data)
                    .stop
                    .as_ref()
                    .and_then(|s| s.thread_id);
                if all_threads || stopped_thread.is_none_or(|t| t == thread_id) {
                    self.resumed(None);
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
                        data.reference_frames.clear();
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
                    data.console_command = Some(command.display());
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

    // ---- subprocesses ------------------------------------------------------------

    /// `startDebugging`: a subprocess of the program waits for a child
    /// session. The product says how to reach the adapter for it (on a
    /// thread of its own: it connects a socket); the request is answered,
    /// then the child session is started and attached to this one.
    fn start_subprocess(&self, kind: String, configuration: Value, responder: Responder) {
        let Some(debugger) = self.inner.debugger.upgrade() else {
            return;
        };
        if lock(&self.inner.data).stopping {
            responder.respond(Err("The debug session is ending".to_owned()));
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        // When no thread can start, the closure and the responder in it
        // are dropped: the adapter gets an error answer.
        let _ = thread::Builder::new()
            .name("debug subprocess".to_owned())
            .spawn(move || {
                let adapter = debugger.provider().child_adapter(&configuration);
                drop(debugger);
                let transport = match adapter {
                    Ok(crate::provider::Adapter::Transport(transport)) => transport,
                    Ok(crate::provider::Adapter::Command(command)) => {
                        match dap_client::Transport::stdio(&command) {
                            Ok(transport) => transport,
                            Err(error) => {
                                responder
                                    .respond(Err(format!("Cannot start the debugger: {error}")));
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        responder.respond(Err(error));
                        return;
                    }
                };
                let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                    responder.respond(Err("The debug session has ended".to_owned()));
                    return;
                };
                responder.respond(Ok(Value::Null));
                session.connect_subprocess(transport, &kind, configuration);
            });
    }

    /// Starts the child session of a subprocess over `transport`:
    /// `initialize`, then `attach` (or `launch`) with its configuration;
    /// the breakpoints follow its `initialized`.
    fn connect_subprocess(
        &self,
        transport: dap_client::Transport,
        kind: &str,
        configuration: Value,
    ) {
        let key = {
            let mut data = lock(&self.inner.data);
            data.next_subprocess += 1;
            data.next_subprocess
        };
        let handler = Arc::new(SessionHandler {
            session: Arc::downgrade(&self.inner),
            process: Process::Subprocess(key),
        });
        let name = format!("{} {key}", self.inner.name);
        let Ok(client) = Client::start(&name, transport, handler) else {
            return;
        };
        let stopping = {
            let mut data = lock(&self.inner.data);
            data.subprocesses.push(Subprocess {
                key,
                client: client.clone(),
                configured: false,
                adapter_ids: HashMap::new(),
                steps_prepared: false,
            });
            data.stopping
        };
        if stopping {
            self.end_subprocess(key);
            return;
        }
        self.changed();
        let weak = Arc::downgrade(&self.inner);
        let launch = kind == "launch";
        let started = client.clone();
        client
            .initialize(&self.client_info())
            .on_done(move |result| {
                let Some(session) = weak.upgrade().map(|inner| Session { inner }) else {
                    return;
                };
                if result.is_err() {
                    session.end_subprocess(key);
                    return;
                }
                let request = if launch {
                    started.launch(configuration)
                } else {
                    started.attach(configuration)
                };
                let weak = Arc::downgrade(&session.inner);
                request.on_done(move |result| {
                    if let (Err(error), Some(inner)) = (result, weak.upgrade())
                        && error != DapError::Closed
                    {
                        Session { inner }.end_subprocess(key);
                    }
                });
            });
    }

    /// A subprocess's `initialized`: its breakpoints, its exception
    /// filters, then `configurationDone` (the subprocess runs on).
    fn configure_subprocess(&self, key: u64) {
        let client = {
            let mut data = lock(&self.inner.data);
            let Some(subprocess) = data.subprocesses.iter_mut().find(|s| s.key == key) else {
                return;
            };
            subprocess.configured = true;
            subprocess.client.clone()
        };
        let files = self
            .inner
            .debugger
            .upgrade()
            .map(|d| d.breakpoint_files())
            .unwrap_or_default();
        let process = Process::Subprocess(key);
        for file in &files {
            self.send_breakpoints_to(process, &client, file);
        }
        self.send_exceptions_to(&client);
        if client.capabilities().configuration_done {
            let weak = Arc::downgrade(&self.inner);
            client.configuration_done().on_done(move |result| {
                if let (Err(error), Some(inner)) = (result, weak.upgrade())
                    && error != DapError::Closed
                {
                    Session { inner }.end_subprocess(key);
                }
            });
        }
    }

    /// Ends a subprocess's child session (`disconnect`, leaving the
    /// process to the adapter); its connection then closes.
    fn end_subprocess(&self, key: u64) {
        let client = lock(&self.inner.data)
            .subprocesses
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.client.clone());
        if let Some(client) = client {
            // Ends on a thread of its own; nobody waits for it.
            drop(client.shutdown(false, STOP_TIMEOUT));
        }
    }

    /// A subprocess's connection closed: it is no longer a process of the
    /// session; when it was shown suspended, the session moves on.
    fn subprocess_closed(&self, key: u64) {
        let process = Process::Subprocess(key);
        let shown = {
            let mut data = lock(&self.inner.data);
            data.subprocesses.retain(|s| s.key != key);
            data.waiting.retain(|(p, _)| *p != process);
            let shown = self.focus() == process;
            if shown {
                self.set_focus(&data, Process::Program);
                data.threads.clear();
                data.selected_thread = None;
                // Its step is over: a waiting stop is shown.
                data.stepping = false;
            }
            shown
        };
        if shown {
            self.resumed(None);
        }
        self.changed();
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

    /// A step of the thread the program stopped in (or the selected one),
    /// sent once the product's preparation of steps is answered.
    fn step(
        &self,
        request: impl FnOnce(&Client, i64) -> Request<()> + Send + 'static,
    ) -> Pending<()> {
        let (client, thread) = match self.thread() {
            Ok(found) => found,
            Err(error) => return ready(Err(error)),
        };
        let at = self.set_stepping(true);
        let weak = Arc::downgrade(&self.inner);
        let (tx, rx) = oneshot::channel();
        let stepping = client.clone();
        self.prepared(&client, move || {
            request(&stepping, thread).on_done(move |result| {
                if let Some(inner) = weak.upgrade() {
                    Session { inner }.stepped(result.is_ok(), at);
                }
                let _ = tx.send(result.map_err(DebugError::from));
            });
        });
        rx.map(|r| r.unwrap_or(Err(DebugError::Ended))).boxed()
    }

    /// Runs `then` once the shown process answered the product's
    /// preparation of steps, sent before its first step only.
    fn prepared(&self, client: &Client, then: impl FnOnce() + Send + 'static) {
        match self.steps_preparation() {
            Some((command, arguments)) => client.custom(&command, arguments).on_done(|_| then()),
            None => then(),
        }
    }

    /// The product's preparation of steps, when the shown process has not
    /// had it yet.
    fn steps_preparation(&self) -> Option<(String, Value)> {
        let request = self.inner.prepare_steps.clone()?;
        let mut data = lock(&self.inner.data);
        let prepared = match self.focus() {
            Process::Program => &mut data.steps_prepared,
            Process::Subprocess(key) => {
                &mut data
                    .subprocesses
                    .iter_mut()
                    .find(|s| s.key == key)?
                    .steps_prepared
            }
        };
        (!std::mem::replace(prepared, true)).then_some(request)
    }

    /// Whether a step of the shown process is on its way; the stop it is
    /// sent at.
    fn set_stepping(&self, stepping: bool) -> u64 {
        let mut data = lock(&self.inner.data);
        data.stepping = stepping;
        data.stops
    }

    /// A step sent at stop `at` was answered: the program moves on, or the
    /// step failed.
    fn stepped(&self, ok: bool, at: u64) {
        if ok {
            self.resumed(Some(at));
        } else {
            let mut data = lock(&self.inner.data);
            if data.stops == at {
                data.stepping = false;
            }
        }
    }

    /// Resume Program.
    pub fn resume(&self) -> Pending<()> {
        let (client, thread) = match self.thread() {
            Ok(found) => found,
            Err(error) => return ready(Err(error)),
        };
        let at = self.set_stepping(false);
        let weak = Arc::downgrade(&self.inner);
        track(client.resume(thread), move |result| {
            if result.is_ok()
                && let Some(inner) = weak.upgrade()
            {
                Session { inner }.resumed(Some(at));
            }
        })
        .map(|r| r.map(|_| ()))
        .boxed()
    }

    pub fn step_over(&self) -> Pending<()> {
        self.step(|client, thread| client.step_over(thread))
    }

    pub fn step_into(&self) -> Pending<()> {
        self.step(|client, thread| client.step_into(thread))
    }

    /// Step Into My Code: the product's request, answered before the step
    /// is sent, and its request after the next stop; a plain Step Into
    /// when the product has none.
    pub fn step_into_my_code(&self) -> Pending<()> {
        let Some(step) = self.inner.step_into_my_code.clone() else {
            return self.step_into();
        };
        let (client, thread) = match self.thread() {
            Ok(found) => found,
            Err(error) => return ready(Err(error)),
        };
        let at = self.set_stepping(true);
        let weak = Arc::downgrade(&self.inner);
        let (tx, rx) = oneshot::channel();
        let (command, arguments) = step.before;
        let stepping = client.clone();
        self.prepared(&client, move || {
            let before = stepping.clone();
            before.custom(&command, arguments).on_done(move |before| {
                let Some(inner) = weak.upgrade() else {
                    let _ = tx.send(Err(DebugError::Ended));
                    return;
                };
                // When the product's request failed, the step is a plain one.
                if before.is_ok() {
                    lock(&inner.data).after_step = Some(step.after);
                }
                let weak = Arc::downgrade(&inner);
                stepping.step_into(thread).on_done(move |result| {
                    if let Some(inner) = weak.upgrade() {
                        Session { inner }.stepped(result.is_ok(), at);
                    }
                    let _ = tx.send(result.map_err(DebugError::from));
                });
            });
        });
        rx.map(|r| r.unwrap_or(Err(DebugError::Ended))).boxed()
    }

    /// Run to Cursor: the program runs on to a line and stops there, at a
    /// breakpoint of the session only (removed at the next stop); with
    /// `ignore_breakpoints` (Force Run to Cursor) the project's breakpoints
    /// do not suspend it on the way.
    pub fn run_to_position(&self, file: &Path, line: u32, ignore_breakpoints: bool) -> Pending<()> {
        if let Err(error) = self.thread() {
            return ready(Err(error));
        }
        let files = {
            let mut data = lock(&self.inner.data);
            data.run_to = Some(RunTo {
                file: file.to_owned(),
                line: line.max(1),
                ignore_breakpoints,
            });
            let mut files = vec![file.to_owned()];
            if ignore_breakpoints {
                files.extend(data.sent.keys().cloned());
            }
            files.sort();
            files.dedup();
            files
        };
        for file in &files {
            self.send_breakpoints(file);
        }
        if ignore_breakpoints {
            self.send_exceptions();
        }
        // `continue` is sent after the breakpoints: the adapter takes the
        // requests in order.
        self.resume()
    }

    pub fn step_out(&self) -> Pending<()> {
        self.step(|client, thread| client.step_out(thread))
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
        // The reference suspends every process of the session.
        let shown = self.focus();
        let others: Vec<Client> = {
            let data = lock(&self.inner.data);
            let program = self.program_client().filter(|_| shown != Process::Program);
            program
                .into_iter()
                .chain(
                    data.subprocesses
                        .iter()
                        .filter(|s| Process::Subprocess(s.key) != shown)
                        .map(|s| s.client.clone()),
                )
                .collect()
        };
        for other in others {
            let paused = other.clone();
            other.threads().on_done(move |result| {
                if let Some(thread) = result.ok().and_then(|t| t.first().map(|t| t.id)) {
                    paused.pause(thread).detach();
                }
            });
        }
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
        let (debuggee, first, subprocesses) = {
            let mut data = lock(&self.inner.data);
            let first = !data.stopping;
            data.stopping = true;
            if data.phase != Phase::Ended {
                data.phase = Phase::Stopping;
            }
            let subprocesses: Vec<Client> =
                data.subprocesses.iter().map(|s| s.client.clone()).collect();
            (data.debuggee.clone(), first, subprocesses)
        };
        let client = self.program_client();
        if first {
            self.changed();
        }
        let spawned = thread::Builder::new()
            .name("debug stop".to_owned())
            .spawn(move || {
                // The program's session first: its adapter then ends the
                // program and the subprocesses it debugs.
                if let Some(client) = &client {
                    let _ = client.shutdown(true, STOP_TIMEOUT).recv();
                }
                let ending: Vec<_> = subprocesses
                    .iter()
                    .map(|client| client.shutdown(true, STOP_TIMEOUT))
                    .collect();
                for ended in ending {
                    let _ = ended.recv();
                }
                if let Some(debuggee) = debuggee {
                    wait_exit(&debuggee, Duration::from_millis(500));
                    let _ = debuggee.kill();
                    wait_exit(&debuggee, STOP_TIMEOUT);
                }
                let _ = done.send(());
            });
        if spawned.is_err()
            && let Some(client) = self.program_client()
        {
            client.kill();
        }
        finished
    }

    /// Kills the program's and the adapter's process trees at once (the
    /// application quits); the subprocesses' connections close with the
    /// adapter.
    pub fn kill(&self) {
        let debuggee = lock(&self.inner.data).debuggee.clone();
        if let Some(debuggee) = debuggee {
            let _ = debuggee.kill();
        }
        if let Some(client) = self.program_client() {
            client.kill();
        }
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
                    for scope in &scopes {
                        data.reference_frames.insert(scope.reference, frame);
                    }
                    data.scopes.insert(frame, scopes.clone());
                }
            }
        })
    }

    /// The loaded scopes of a frame.
    pub fn loaded_scopes(&self, frame: i64) -> Option<Vec<Scope>> {
        lock(&self.inner.data).scopes.get(&frame).cloned()
    }

    /// The loaded children of a scope or a variable (`reference`).
    pub fn loaded_children(&self, reference: i64) -> Option<Children> {
        lock(&self.inner.data).children.get(&reference).cloned()
    }

    /// Loads the children of a scope or a variable (all of them: debugpy
    /// pages large collections with children of its own) with the sizes
    /// of the containers among them, and returns them; loaded children are
    /// kept until the program moves on.
    pub fn children(&self, reference: i64) -> Pending<Children> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let generation = {
            let data = lock(&self.inner.data);
            if let Some(loaded) = data.children.get(&reference) {
                return ready(Ok(loaded.clone()));
            }
            if data.phase != Phase::Suspended {
                return ready(Err(DebugError::NotSuspended));
            }
            data.generation
        };
        let session = self.clone();
        let (tx, rx) = oneshot::channel();
        client
            .variables(reference, VariablesFilter::All, None)
            .on_done(move |result| match result {
                Ok(items) => session.children_loaded(reference, items, generation, tx),
                Err(error) => {
                    let _ = tx.send(Err(error.into()));
                }
            });
        rx.map(|r| r.unwrap_or(Err(DebugError::Ended))).boxed()
    }

    /// Children came: their sizes are asked for, then they are kept.
    fn children_loaded(
        &self,
        reference: i64,
        items: Vec<Variable>,
        generation: u64,
        tx: oneshot::Sender<Result<Children, DebugError>>,
    ) {
        let frame = {
            let mut data = lock(&self.inner.data);
            let frame = data.reference_frames.get(&reference).copied();
            if let (Some(frame), true) = (frame, data.generation == generation) {
                for item in items.iter().filter(|v| v.reference > 0) {
                    data.reference_frames.insert(item.reference, frame);
                }
            }
            frame
        };
        // Values with children have a size; groups and the like do not.
        let provider = self.inner.debugger.upgrade().map(|d| d.provider().clone());
        let sized: Vec<(usize, String)> = items
            .iter()
            .enumerate()
            .filter(|(_, v)| v.reference > 0)
            .filter(|(_, v)| {
                provider.as_ref().is_none_or(|p| {
                    matches!(p.present_value(v).role, crate::values::ValueRole::Value(_))
                })
            })
            .filter_map(|(i, v)| Some((i, v.evaluate_name.clone()?)))
            .collect();
        let weak = Arc::downgrade(&self.inner);
        let store = move |items: Vec<Variable>, shapes: Vec<Option<String>>| {
            let children = Children {
                items,
                shapes,
                more: false,
            };
            if let Some(inner) = weak.upgrade() {
                let mut data = lock(&inner.data);
                if data.generation == generation {
                    data.children
                        .entry(reference)
                        .or_insert_with(|| children.clone());
                }
            }
            let _ = tx.send(Ok(children));
        };
        let expressions: Vec<String> = sized.iter().map(|(_, e)| e.clone()).collect();
        match (frame, sized.is_empty()) {
            (Some(frame), false) => {
                let count = items.len();
                self.sizes_then(frame, &expressions, move |sizes| {
                    let mut shapes = vec![None; count];
                    for ((index, _), size) in sized.iter().zip(sizes) {
                        shapes[*index] = size;
                    }
                    store(items, shapes);
                });
            }
            _ => store(items, Vec::new()),
        }
    }

    /// The sizes of the values of `expressions` in a frame (the product's
    /// [`crate::DebugAdapterProvider::sizes_expression`]), one per
    /// expression; all `None` when the product shows no sizes.
    pub fn sizes(&self, frame: i64, expressions: Vec<String>) -> Pending<Vec<Option<String>>> {
        let (tx, rx) = oneshot::channel();
        self.sizes_then(frame, &expressions, move |sizes| {
            let _ = tx.send(sizes);
        });
        rx.map(|r| Ok(r.unwrap_or_default())).boxed()
    }

    /// [`Self::sizes`] with a callback (on the reader thread, or at once).
    fn sizes_then(
        &self,
        frame: i64,
        expressions: &[String],
        then: impl FnOnce(Vec<Option<String>>) + Send + 'static,
    ) {
        let count = expressions.len();
        let provider = self.inner.debugger.upgrade().map(|d| d.provider().clone());
        let names: Vec<&str> = expressions.iter().map(String::as_str).collect();
        let request = match (provider, self.client()) {
            (Some(provider), Some(client)) => provider
                .sizes_expression(&names)
                .map(|expression| (provider, client, expression)),
            _ => None,
        };
        let Some((provider, client, expression)) = request else {
            then(vec![None; count]);
            return;
        };
        client
            .evaluate(&expression, Some(frame), EvaluateContext::Watch)
            .on_done(move |result| {
                then(match result {
                    Ok(value) => {
                        let mut sizes = provider.parse_sizes(&value.value, count);
                        sizes.resize(count, None);
                        sizes
                    }
                    Err(_) => vec![None; count],
                })
            });
    }

    /// Evaluates an expression in the selected frame. Dropping the future
    /// cancels the evaluation.
    pub fn evaluate(&self, expression: &str, context: EvaluateContext) -> Pending<Evaluated> {
        let frame = lock(&self.inner.data).selected_frame;
        self.evaluate_in(expression, frame, context)
    }

    /// Evaluates an expression in a frame (globally without one); the
    /// children of its value belong to the frame. Dropping the future
    /// cancels the evaluation.
    pub fn evaluate_in(
        &self,
        expression: &str,
        frame: Option<i64>,
        context: EvaluateContext,
    ) -> Pending<Evaluated> {
        let Some(client) = self.client() else {
            return ready(Err(DebugError::Ended));
        };
        let generation = lock(&self.inner.data).generation;
        let weak = Arc::downgrade(&self.inner);
        client
            .evaluate(expression, frame, context)
            .map(move |result| {
                if let (Ok(value), Some(inner), Some(frame)) = (&result, weak.upgrade(), frame)
                    && value.reference > 0
                {
                    let mut data = lock(&inner.data);
                    if data.generation == generation {
                        data.reference_frames.insert(value.reference, frame);
                    }
                }
                result.map_err(DebugError::from)
            })
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
                .enumerate()
                .map(|(i, v)| {
                    let mut item = serde_json::json!({
                        "name": v.name,
                        "value": v.value,
                        "type": v.type_name,
                        "shape": loaded.shape(i),
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

/// A stop of this reason is a breakpoint hit (a line, function, data,
/// instruction or exception breakpoint).
fn at_breakpoint(reason: &str) -> bool {
    matches!(
        reason,
        "breakpoint"
            | "function breakpoint"
            | "data breakpoint"
            | "instruction breakpoint"
            | "exception"
    )
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

/// What one connection of the session (the program's or a subprocess's)
/// receives.
struct SessionHandler {
    session: Weak<SessionInner>,
    process: Process,
}

impl SessionHandler {
    fn session(&self) -> Option<Session> {
        self.session.upgrade().map(|inner| Session { inner })
    }
}

impl Handler for SessionHandler {
    fn event(&self, event: Event) {
        if let Some(session) = self.session() {
            session.process_event(self.process, event);
        }
    }

    fn reverse_request(&self, request: ReverseRequest, responder: Responder) {
        let Some(session) = self.session() else {
            return;
        };
        match request {
            ReverseRequest::RunInTerminal(request) if self.process == Process::Program => {
                session.run_in_terminal(request, responder);
            }
            ReverseRequest::StartDebugging {
                request,
                configuration,
            } => session.start_subprocess(request, configuration, responder),
            // The adapter gets an error answer.
            ReverseRequest::RunInTerminal(_) | ReverseRequest::Other { .. } => drop(responder),
        }
    }

    fn stderr(&self, chunk: &[u8]) {
        if self.process != Process::Program {
            return;
        }
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
            match self.process {
                Process::Program => session.closed(status),
                Process::Subprocess(key) => session.subprocess_closed(key),
            }
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
