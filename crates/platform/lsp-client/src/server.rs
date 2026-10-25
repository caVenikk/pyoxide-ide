//! One language server: its process, the `initialize` handshake, its state
//! for the Language Services widget, what it asks the client, and stopping
//! it.
//!
//! As in the reference (`LspClientImpl`, `LspClientManagerImpl`,
//! `Lsp4jServerConnector` of IntelliJ Platform):
//!
//! - a server starts on a background thread and is Initializing until it
//!   answers `initialize` (at most [`Timeouts::initialize`]), then Running;
//! - a server that exits on its own, fails to start or does not answer
//!   `initialize` is Terminated with the end of its standard error, and is
//!   not restarted automatically;
//! - stopping sends `shutdown` (waiting at most [`Timeouts::shutdown`]) and
//!   `exit`, then interrupts and finally kills the process when it does not
//!   exit within [`Timeouts::exit`] each.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use code_intel::{
    DocumentId, EditReply, EventHub, IntelEvent, MessageLevel, ServerId, ServerState, ServerStatus,
};
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use process::{ExitStatus, ProcessHandle};
use serde_json::{Value, json};

use crate::capabilities::{self, ServerInfo};
use crate::convert;
use crate::debounce::Debouncer;
use crate::diagnostics::Diagnostics;
use crate::documents::Documents;
use crate::encoding::{PositionEncoding, Text};
use crate::rpc::{self, Connection, Handler, Params, Request, Responder, RpcError};
use crate::spec::ServerSpec;
use crate::texts::FileTexts;
use crate::uri;

mod sync;

pub(crate) use sync::{PULL_DELAY, Synced};

/// How long the client waits for a server at each step of its life. The
/// defaults are the reference's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// For the answer to `initialize` (the reference's
    /// `lsp.server.connect.timeout`, 100 s); a server that takes longer is
    /// Terminated.
    pub initialize: Duration,
    /// For the answer to `shutdown` before `exit` is sent anyway (10 s).
    pub shutdown: Duration,
    /// For the process to exit after `exit`, and again after an interrupt,
    /// before it is killed (1 s).
    pub exit: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            initialize: Duration::from_secs(100),
            shutdown: Duration::from_secs(10),
            exit: Duration::from_secs(1),
        }
    }
}

/// The most standard error kept per server (the reference keeps a
/// megabyte).
pub(crate) const MAX_ERROR_OUTPUT: usize = 1024 * 1024;

/// What the servers of one [`crate::LspCodeIntel`] share.
pub(crate) struct Context {
    pub hub: EventHub,
    pub timeouts: Timeouts,
    pub documents: Documents,
    pub diagnostics: Diagnostics,
    pub files: FileTexts,
    /// Pulls of diagnostics, by server and document.
    pub pulls: Debouncer<(ServerId, DocumentId), Weak<LanguageServer>>,
}

impl Context {
    pub fn new(timeouts: Timeouts) -> Arc<Self> {
        Arc::new(Self {
            hub: EventHub::new(),
            timeouts,
            documents: Documents::default(),
            diagnostics: Diagnostics::default(),
            files: FileTexts::default(),
            pulls: Debouncer::new(
                "lsp diagnostics",
                |(_, doc), server: Weak<LanguageServer>| {
                    if let Some(server) = server.upgrade() {
                        server.pull(&doc);
                    }
                },
            ),
        })
    }

    /// The text of a file for converting positions: the open document, or
    /// the file on disk.
    pub fn text_of(&self, path: &Path) -> Result<Text, String> {
        match self.documents.get(&DocumentId::from(path)) {
            Some(snapshot) => Ok(Text::new(snapshot.text)),
            None => self.files.text(path),
        }
    }
}

/// One server of a project and its current process, if any.
pub(crate) struct LanguageServer {
    id: ServerId,
    cx: Arc<Context>,
    me: Weak<LanguageServer>,
    slot: Mutex<Slot>,
}

struct Slot {
    /// Replaced by [`LanguageServer::update`] when only what needs no
    /// restart changed.
    spec: Arc<ServerSpec>,
    status: ServerStatus,
    /// The name in the status is the one the server reported.
    reported_name: bool,
    /// Increases with every start and stop: callbacks of an older process
    /// see a different number and change nothing.
    generation: u64,
    session: Option<Arc<Session>>,
    /// Earlier processes being stopped, done when they have exited.
    retiring: Vec<Shared<oneshot::Receiver<()>>>,
}

/// One run of a server process.
pub(crate) struct Session {
    pub generation: u64,
    pub connection: Connection,
    process: ProcessHandle,
    pub state: Arc<SessionState>,
    /// The documents open on the server.
    docs: Mutex<HashMap<DocumentId, Synced>>,
    /// The diagnostics pull in flight per document (replacing one cancels
    /// it).
    pulls: Mutex<HashMap<DocumentId, Request<()>>>,
    /// The `resultId` of each document's last pulled report.
    result_ids: Mutex<HashMap<DocumentId, String>>,
}

/// What a session learns while it runs; shared with its reader thread.
#[derive(Default)]
pub(crate) struct SessionState {
    info: OnceLock<ServerInfo>,
    registrations: Mutex<Vec<Registration>>,
    progress: Mutex<HashMap<String, Progress>>,
    stderr: Mutex<Tail>,
    exit: Mutex<Option<Option<ExitStatus>>>,
    exited: Condvar,
}

/// A capability the server registered (`client/registerCapability`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Registration {
    pub id: String,
    pub method: String,
    pub options: Value,
}

#[derive(Debug, Clone)]
struct Progress {
    title: String,
    message: Option<String>,
    percentage: Option<u32>,
}

impl LanguageServer {
    pub fn new(spec: ServerSpec, cx: Arc<Context>) -> Arc<Self> {
        let mut status = ServerStatus::new(spec.id.clone(), spec.name.clone());
        status.roots = vec![spec.root.clone()];
        status.icon = spec.icon.clone();
        status.features = spec.features;
        Arc::new_cyclic(|me| Self {
            id: spec.id.clone(),
            cx,
            me: me.clone(),
            slot: Mutex::new(Slot {
                spec: Arc::new(spec),
                status,
                reported_name: false,
                generation: 0,
                session: None,
                retiring: Vec::new(),
            }),
        })
    }

    pub fn spec(&self) -> Arc<ServerSpec> {
        self.slot().spec.clone()
    }

    pub fn id(&self) -> &ServerId {
        &self.id
    }

    /// A new spec of the same server needs a new process: how it starts,
    /// its folder, or what it is told at start or asked for later changed.
    pub fn needs_restart_for(&self, spec: &ServerSpec) -> bool {
        let current = self.spec();
        current.command != spec.command
            || current.root != spec.root
            || current.initialization_options != spec.initialization_options
            || current.settings != spec.settings
            || current.empty_configuration_change != spec.empty_configuration_change
    }

    /// Takes a new spec that needs no restart (see
    /// [`LanguageServer::needs_restart_for`]): languages, features, name,
    /// icon, customizer.
    pub fn update(&self, spec: ServerSpec) {
        let mut slot = self.slot();
        let mut status = slot.status.clone();
        if !slot.reported_name {
            status.name = spec.name.clone();
        }
        status.icon = spec.icon.clone();
        status.features = spec.features;
        slot.spec = Arc::new(spec);
        if status != slot.status {
            slot.status = status;
            self.emit(&slot);
        }
    }

    /// A process of the server is still being stopped.
    pub fn is_stopping(&self) -> bool {
        self.slot().retiring.iter().any(|r| r.peek().is_none())
    }

    pub fn status(&self) -> ServerStatus {
        self.slot().status.clone()
    }

    /// The running process, once it answered `initialize`.
    pub fn running_session(&self) -> Option<Arc<Session>> {
        let slot = self.slot();
        if slot.status.state != ServerState::Running {
            return None;
        }
        slot.session.clone()
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, slot: &Slot) {
        self.cx.hub.emit(IntelEvent::Status(slot.status.clone()));
    }

    /// Starts the server when it is stopped (not when it terminated: that
    /// takes an explicit restart, as in the reference).
    pub fn ensure_started(&self) {
        self.start_if(|state| *state == ServerState::Stopped);
    }

    /// Stops the current process, if any, and starts a new one.
    pub fn restart(&self) {
        self.stop();
        self.start();
    }

    fn start(&self) {
        self.start_if(|_| true);
    }

    /// Starts a new process when the state is one `when` accepts (checked
    /// under the same lock that marks it Initializing).
    fn start_if(&self, when: impl FnOnce(&ServerState) -> bool) {
        let generation = {
            let mut slot = self.slot();
            if !when(&slot.status.state) {
                return;
            }
            slot.generation += 1;
            slot.status.state = ServerState::Initializing;
            self.emit(&slot);
            slot.generation
        };
        let Some(server) = self.me.upgrade() else {
            return;
        };
        let spawned = thread::Builder::new()
            .name(format!("lsp {} start", self.id))
            .spawn(move || server.run(generation));
        if let Err(error) = spawned {
            self.terminate(
                generation,
                &format!("Cannot start the server: {error}"),
                None,
            );
        }
    }

    /// Starts the process and waits for its answer to `initialize`; runs
    /// on its own thread.
    fn run(self: Arc<Self>, generation: u64) {
        let spec = self.spec();
        let (process, events) = match process::spawn(&spec.command) {
            Ok(spawned) => spawned,
            Err(error) => {
                let message = format!(
                    "Cannot run program \"{}\": {error}",
                    spec.command.program.display()
                );
                self.terminate(generation, &message, None);
                return;
            }
        };
        let state = Arc::new(SessionState::default());
        let handler = Arc::new(SessionHandler {
            server: self.me.clone(),
            generation,
            state: state.clone(),
        });
        let connection = match Connection::start(self.id.as_str(), process.clone(), events, handler)
        {
            Ok(connection) => connection,
            Err(error) => {
                let _ = process.kill();
                self.terminate(
                    generation,
                    &format!("Cannot start the server: {error}"),
                    None,
                );
                return;
            }
        };
        let session = Arc::new(Session {
            generation,
            connection,
            process,
            state,
            docs: Mutex::new(HashMap::new()),
            pulls: Mutex::new(HashMap::new()),
            result_ids: Mutex::new(HashMap::new()),
        });
        {
            let mut slot = self.slot();
            if slot.generation != generation || slot.status.state != ServerState::Initializing {
                // Stopped, restarted or terminated meanwhile.
                self.retire(&mut slot, session, false);
                return;
            }
            slot.session = Some(session.clone());
        }
        let (answer, answered) = flume::bounded(1);
        session.connection.request_with(
            "initialize",
            Params::Lazy(Box::new(move || capabilities::initialize_params(&spec))),
            Box::new(move |result| {
                let _ = answer.send(result);
            }),
        );
        match answered.recv_timeout(self.cx.timeouts.initialize) {
            Ok(Ok(result)) => self.initialized(&session, &result),
            // The process exited: its exit makes it Terminated.
            Ok(Err(RpcError::Closed)) | Err(flume::RecvTimeoutError::Disconnected) => {}
            Ok(Err(error)) => self.fail(
                &session,
                &format!("The server failed to initialize: {error}"),
            ),
            Err(flume::RecvTimeoutError::Timeout) => self.fail(
                &session,
                &format!(
                    "The server did not answer initialize in {} s",
                    self.cx.timeouts.initialize.as_secs_f64()
                ),
            ),
        }
    }

    fn initialized(&self, session: &Arc<Session>, result: &Value) {
        let info = ServerInfo::from_initialize_result(result);
        let _ = session.state.info.set(info.clone());
        let (status, spec) = {
            let mut slot = self.slot();
            if slot.generation != session.generation
                || slot.status.state != ServerState::Initializing
            {
                return;
            }
            if let Some(name) = info.name {
                slot.status.name = name;
                slot.reported_name = true;
            }
            if info.version.is_some() {
                slot.status.version = info.version;
            }
            slot.status.state = ServerState::Running;
            self.emit(&slot);
            // Queued under the lock that makes it Running: nothing about
            // documents goes out before it, and an exit right after it is
            // seen once it runs. Queued after the status event: whatever
            // the server sends in answer to it must not be reported
            // before the server is Running.
            session.connection.notify("initialized", json!({}));
            (slot.status.clone(), slot.spec.clone())
        };
        if spec.empty_configuration_change {
            // The pyright family reads its settings only after this (the
            // reference's `PyrightEmptyConfigurationSender`).
            session
                .connection
                .notify("workspace/didChangeConfiguration", json!({"settings": {}}));
        }
        if let Some(customizer) = &spec.customizer {
            customizer.initialized(&status);
        }
        // The documents opened before (the reference's
        // `openForOpenedOrUnsavedFiles`).
        self.open_all(session);
    }

    /// The session cannot go on (no or a failed answer to `initialize`):
    /// Terminated, and its process is stopped without the handshake.
    fn fail(&self, session: &Arc<Session>, reason: &str) {
        let mut slot = self.slot();
        self.terminate_locked(&mut slot, session.generation, reason, Some(&session.state));
        self.retire(&mut slot, session.clone(), false);
    }

    /// Makes the server Terminated when `generation` is still its current
    /// run and it was starting or running.
    fn terminate(&self, generation: u64, reason: &str, state: Option<&SessionState>) {
        self.terminate_locked(&mut self.slot(), generation, reason, state);
    }

    fn terminate_locked(
        &self,
        slot: &mut Slot,
        generation: u64,
        reason: &str,
        state: Option<&SessionState>,
    ) {
        if slot.generation != generation || !slot.status.state.is_running() {
            return;
        }
        let error_output = match state {
            Some(state) => state.error_output(reason),
            None => reason.to_owned(),
        };
        slot.session = None;
        slot.status.state = ServerState::Terminated { error_output };
        slot.status.documents.clear();
        self.emit(slot);
        self.forget_diagnostics();
    }

    /// The server's diagnostics go with it.
    fn forget_diagnostics(&self) {
        let cx = &self.cx;
        cx.diagnostics
            .clear_server(&cx.documents, &cx.hub, &self.id);
    }

    /// Ends a session's process on a thread of its own: gracefully
    /// (`shutdown`, `exit`) or not. [`LanguageServer::stopped`] waits for
    /// it.
    fn retire(&self, slot: &mut Slot, session: Arc<Session>, graceful: bool) {
        let (done, stopped) = oneshot::channel();
        slot.retiring.retain(|r| r.peek().is_none());
        slot.retiring.push(stopped.shared());
        let timeouts = self.cx.timeouts;
        let process = session.process.clone();
        let spawned = thread::Builder::new()
            .name(format!("lsp {} stop", self.id))
            .spawn(move || {
                if graceful {
                    session.shut_down(timeouts);
                } else {
                    session.stop_process(timeouts);
                }
                let _ = done.send(());
            });
        if spawned.is_err() {
            let _ = process.kill();
        }
    }

    /// Completes when every process of the server that is being stopped
    /// has exited or was killed.
    pub fn stopped(&self) -> impl Future<Output = ()> + Send + 'static {
        let retiring = self.slot().retiring.clone();
        async move {
            futures::future::join_all(retiring).await;
        }
    }

    /// The process of a session exited.
    fn exited(&self, generation: u64, status: Option<ExitStatus>, state: &SessionState) {
        let reason = match status {
            Some(status) => status.finished_message(),
            None => "The server's output ended".to_owned(),
        };
        self.terminate(generation, &reason, Some(state));
    }

    /// Stops the server: Stopped at once, the process shut down in the
    /// background ([`LanguageServer::stopped`] waits for it).
    pub fn stop(&self) {
        let mut slot = self.slot();
        slot.generation += 1;
        if slot.status.state != ServerState::Stopped {
            slot.status.state = ServerState::Stopped;
            slot.status.documents.clear();
            self.emit(&slot);
        }
        if let Some(session) = slot.session.take() {
            self.forget_diagnostics();
            self.retire(&mut slot, session, true);
        }
    }

    /// The text of a file for converting positions: the open document, or
    /// the file on disk.
    pub fn text_of(&self, path: &Path) -> Result<Text, String> {
        self.cx.text_of(path)
    }

    /// The current session if it is run `generation`.
    fn session_of(&self, generation: u64) -> Option<Arc<Session>> {
        let slot = self.slot();
        if slot.generation != generation {
            return None;
        }
        slot.session.clone()
    }

    fn registered(&self, generation: u64, methods: &[String]) {
        // A new pull registration makes earlier pulled results stale (the
        // reference's `restartHighlightingIfNeeded`).
        if methods.iter().any(|m| m == "textDocument/diagnostic") {
            self.refresh_diagnostics(generation);
        }
    }
}

impl Session {
    pub fn info(&self) -> Option<&ServerInfo> {
        self.state.info.get()
    }

    pub fn encoding(&self) -> PositionEncoding {
        self.state.encoding()
    }

    /// The graceful end: `shutdown`, `exit`, then interrupt and kill when
    /// the process does not go.
    fn shut_down(&self, timeouts: Timeouts) {
        if self.state.wait_exit(Duration::ZERO) {
            return;
        }
        let (answer, answered) = flume::bounded(1);
        self.connection.request_with(
            "shutdown",
            Params::None,
            Box::new(move |_| {
                let _ = answer.send(());
            }),
        );
        let _ = answered.recv_timeout(timeouts.shutdown);
        self.connection.notify("exit", Params::None);
        self.stop_process(timeouts);
    }

    /// Closes the input and waits for the exit; interrupts, then kills the
    /// process tree when it takes too long.
    fn stop_process(&self, timeouts: Timeouts) {
        self.connection.close_input();
        if self.state.wait_exit(timeouts.exit) {
            return;
        }
        let _ = self.process.stop();
        if self.state.wait_exit(timeouts.exit) {
            return;
        }
        let _ = self.process.kill();
        self.state.wait_exit(timeouts.exit);
    }
}

impl SessionState {
    fn encoding(&self) -> PositionEncoding {
        self.info
            .get()
            .map_or(PositionEncoding::Utf16, |info| info.encoding)
    }

    fn registrations(&self) -> MutexGuard<'_, Vec<Registration>> {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn stderr(&self) -> MutexGuard<'_, Tail> {
        self.stderr.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The end of the standard error, then `reason`.
    fn error_output(&self, reason: &str) -> String {
        let mut text = self.stderr().text().trim_end().to_owned();
        if !reason.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(reason);
        }
        text
    }

    fn set_exit(&self, status: Option<ExitStatus>) {
        let mut exit = self.exit.lock().unwrap_or_else(PoisonError::into_inner);
        *exit = Some(status);
        self.exited.notify_all();
    }

    /// Waits until the process exited, at most `timeout`; whether it has.
    fn wait_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut exit = self.exit.lock().unwrap_or_else(PoisonError::into_inner);
        while exit.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            exit = self
                .exited
                .wait_timeout(exit, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// The end of a stream, at most `limit` bytes.
#[derive(Debug)]
pub(crate) struct Tail {
    bytes: VecDeque<u8>,
    limit: usize,
}

impl Default for Tail {
    fn default() -> Self {
        Self::new(MAX_ERROR_OUTPUT)
    }
}

impl Tail {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            limit,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        let chunk = &chunk[chunk.len().saturating_sub(self.limit)..];
        self.bytes.extend(chunk);
        if self.bytes.len() > self.limit {
            let excess = self.bytes.len() - self.limit;
            self.bytes.drain(..excess);
        }
    }

    /// The kept bytes as text; a character cut at the start is left out.
    pub fn text(&self) -> String {
        let skip = self
            .bytes
            .iter()
            .take(3)
            .take_while(|&&b| b & 0xC0 == 0x80)
            .count();
        let bytes: Vec<u8> = self.bytes.iter().skip(skip).copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Handles what one session's server sends, on its reader thread.
struct SessionHandler {
    server: Weak<LanguageServer>,
    generation: u64,
    state: Arc<SessionState>,
}

impl SessionHandler {
    fn emit(&self, server: &LanguageServer, event: IntelEvent) {
        server.cx.hub.emit(event);
    }

    fn configuration(&self, server: &LanguageServer, params: &Value) -> Value {
        let items = params
            .get("items")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let spec = server.spec();
        let answers = items
            .iter()
            .map(|item| {
                let section = item.get("section").and_then(Value::as_str);
                let scope = item
                    .get("scopeUri")
                    .and_then(Value::as_str)
                    .and_then(uri::str_to_path);
                match &spec.customizer {
                    Some(customizer) => {
                        customizer.configuration(section, scope.as_deref(), &spec.settings)
                    }
                    None => crate::customizer::lookup_section(&spec.settings, section),
                }
            })
            .collect();
        Value::Array(answers)
    }

    fn register(&self, params: &Value) -> Vec<String> {
        let mut methods = Vec::new();
        let mut registrations = self.state.registrations();
        for registration in params
            .get("registrations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(method)) = (
                registration.get("id").and_then(Value::as_str),
                registration.get("method").and_then(Value::as_str),
            ) else {
                continue;
            };
            registrations.retain(|r| r.id != id);
            registrations.push(Registration {
                id: id.to_owned(),
                method: method.to_owned(),
                options: registration
                    .get("registerOptions")
                    .cloned()
                    .unwrap_or(Value::Null),
            });
            methods.push(method.to_owned());
        }
        methods
    }

    fn unregister(&self, params: &Value) {
        // The protocol spells the member `unregisterations`.
        let list = params
            .get("unregisterations")
            .or_else(|| params.get("unregistrations"))
            .and_then(Value::as_array);
        let mut registrations = self.state.registrations();
        for item in list.into_iter().flatten() {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                registrations.retain(|r| r.id != id);
            }
        }
    }

    fn progress(&self, server: &LanguageServer, params: &Value) {
        let token = match params.get("token") {
            Some(Value::String(token)) => token.clone(),
            Some(Value::Number(token)) => token.to_string(),
            _ => return,
        };
        let Some(value) = params.get("value") else {
            return;
        };
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let percentage = value
            .get("percentage")
            .and_then(Value::as_u64)
            .map(|p| p.min(100) as u32);
        let mut progress = self
            .state
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (current, done) = match value.get("kind").and_then(Value::as_str) {
            Some("begin") => {
                let current = Progress {
                    title: text("title").unwrap_or_default(),
                    message: text("message"),
                    percentage,
                };
                progress.insert(token, current.clone());
                (current, false)
            }
            Some("report") => {
                let Some(current) = progress.get_mut(&token) else {
                    return;
                };
                if let Some(message) = text("message") {
                    current.message = Some(message);
                }
                if percentage.is_some() {
                    current.percentage = percentage;
                }
                (current.clone(), false)
            }
            Some("end") => {
                let Some(mut current) = progress.remove(&token) else {
                    return;
                };
                if let Some(message) = text("message") {
                    current.message = Some(message);
                }
                (current, true)
            }
            // Partial results and anything else.
            _ => return,
        };
        drop(progress);
        self.emit(
            server,
            IntelEvent::Progress {
                server: server.id.clone(),
                title: current.title,
                message: current.message,
                percentage: current.percentage,
                done,
            },
        );
    }

    fn message(&self, server: &LanguageServer, params: &Value, log: bool) {
        let text = params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        // The reference shows Log and Debug messages as information.
        let level = if log {
            MessageLevel::Log
        } else {
            match params.get("type").and_then(Value::as_u64) {
                Some(1) => MessageLevel::Error,
                Some(2) => MessageLevel::Warning,
                _ => MessageLevel::Info,
            }
        };
        self.emit(
            server,
            IntelEvent::Message {
                server: server.id.clone(),
                level,
                text,
            },
        );
    }

    /// `workspace/applyEdit`: converted and handed to the application on
    /// a thread of its own (files may be read, the user may take a while).
    fn apply_edit(&self, server: Arc<LanguageServer>, params: Value, responder: Responder) {
        let encoding = self.state.encoding();
        let spawned = thread::Builder::new()
            .name(format!("lsp {} apply edit", server.id))
            .spawn(move || {
                let label = params
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let edit = params.get("edit").cloned().unwrap_or(Value::Null);
                let converted =
                    convert::workspace_edit(&edit, encoding, &|path| server.text_of(path));
                let edit = match converted {
                    Ok(edit) => edit,
                    Err(reason) => {
                        responder.respond(Ok(json!({"applied": false, "failureReason": reason})));
                        return;
                    }
                };
                let (reply, answer) = EditReply::new();
                server.cx.hub.emit(IntelEvent::ApplyEdit {
                    server: server.id.clone(),
                    label,
                    edit,
                    reply,
                });
                let applied = futures::executor::block_on(answer).unwrap_or(false);
                responder.respond(Ok(json!({ "applied": applied })));
            });
        // Without a thread the responder was dropped: an error answer.
        drop(spawned);
    }
}

impl Handler for SessionHandler {
    fn notification(&self, method: &str, params: Value) {
        let Some(server) = self.server.upgrade() else {
            return;
        };
        match method {
            "window/showMessage" => self.message(&server, &params, false),
            "window/logMessage" => self.message(&server, &params, true),
            "$/progress" => self.progress(&server, &params),
            "textDocument/publishDiagnostics" => {
                server.published_diagnostics(self.generation, params);
            }
            // `$/logTrace`, `telemetry/event`, a cancelled server request.
            _ => {}
        }
    }

    fn request(&self, method: &str, params: Value, responder: Responder) {
        let Some(server) = self.server.upgrade() else {
            responder.respond(Err(RpcError::server(
                rpc::code::INTERNAL_ERROR,
                "the client is closing",
            )));
            return;
        };
        let result = match method {
            "workspace/configuration" => Ok(self.configuration(&server, &params)),
            "client/registerCapability" => {
                let methods = self.register(&params);
                responder.respond(Ok(Value::Null));
                server.registered(self.generation, &methods);
                return;
            }
            "client/unregisterCapability" => {
                self.unregister(&params);
                Ok(Value::Null)
            }
            "window/workDoneProgress/create" => Ok(Value::Null),
            "workspace/applyEdit" => {
                self.apply_edit(server, params, responder);
                return;
            }
            "workspace/diagnostic/refresh" => {
                responder.respond(Ok(Value::Null));
                server.refresh_diagnostics(self.generation);
                return;
            }
            "workspace/workspaceFolders" => {
                Ok(json!([capabilities::workspace_folder(&server.spec().root)]))
            }
            "window/showMessageRequest" => {
                // No actions to offer: the message is shown, nothing chosen.
                self.message(&server, &params, false);
                Ok(Value::Null)
            }
            "window/showDocument" => Ok(json!({ "success": false })),
            "workspace/semanticTokens/refresh"
            | "workspace/inlayHint/refresh"
            | "workspace/inlineValue/refresh"
            | "workspace/codeLens/refresh"
            | "workspace/foldingRange/refresh" => Ok(Value::Null),
            _ => Err(RpcError::server(
                rpc::code::METHOD_NOT_FOUND,
                format!("unhandled method {method}"),
            )),
        };
        responder.respond(result);
    }

    fn stderr(&self, chunk: &[u8]) {
        self.state.stderr().push(chunk);
    }

    fn protocol_error(&self, error: String) {
        self.state
            .stderr()
            .push(format!("\nInvalid server output: {error}\n").as_bytes());
    }

    fn exited(&self, status: Option<ExitStatus>) {
        self.state.set_exit(status);
        if let Some(server) = self.server.upgrade() {
            server.exited(self.generation, status, &self.state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_keeps_the_end() {
        let mut tail = Tail::new(10);
        tail.push(b"0123456789abc");
        assert_eq!(tail.text(), "3456789abc");
        tail.push(b"XY");
        assert_eq!(tail.text(), "56789abcXY");
        // A character cut at the start is left out.
        let mut tail = Tail::new(5);
        tail.push("aé🦀".as_bytes());
        assert_eq!(tail.text(), "🦀");
        tail.push("中".as_bytes());
        assert_eq!(tail.text(), "中");
        assert_eq!(Tail::default().limit, MAX_ERROR_OUTPUT);
    }

    #[test]
    fn exits_are_waited_for() {
        let state = Arc::new(SessionState::default());
        assert!(!state.wait_exit(Duration::from_millis(10)));
        let waiting = state.clone();
        let waiter = thread::spawn(move || waiting.wait_exit(Duration::from_secs(10)));
        thread::sleep(Duration::from_millis(20));
        state.set_exit(None);
        assert!(waiter.join().unwrap());
        assert!(state.wait_exit(Duration::ZERO));
    }

    #[test]
    fn error_output_is_the_tail_then_the_reason() {
        let state = SessionState::default();
        assert_eq!(state.error_output("exit 1"), "exit 1");
        state.stderr().push(b"panic: boom\n\n");
        assert_eq!(state.error_output("exit 1"), "panic: boom\nexit 1");
        assert_eq!(state.error_output(""), "panic: boom");
    }
}
