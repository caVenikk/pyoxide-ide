//! The client side of one adapter's message stream (ADR 0015).
//!
//! A [`Client`] has two threads, as the LSP client (ADR 0010):
//!
//! - the reader cuts the adapter's output into frames
//!   ([`stream_framing`]), parses them, answers pending requests (running
//!   their conversions to [`crate::types`]) and passes events, reverse
//!   requests, the adapter's log and the end of the stream to a
//!   [`Handler`];
//! - the writer writes the queued messages in order.
//!
//! No async runtime: a request is a callback in the pending table, and
//! [`Request`] is a future that sends `cancel` when it is dropped before
//! the answer came (when the adapter supports it). Sequence numbers are
//! taken in the order messages are queued, so they grow along the stream.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use process::{ExitStatus, ProcessHandle};
use serde::Serialize;
use serde_json::{Map, Value, json};
use stream_framing::FrameDecoder;

use crate::protocol;
use crate::transport::{Incoming, Inlet, Outlet, Transport};
use crate::types::{
    Breakpoint, Capabilities, ClientInfo, DapError, EvaluateContext, Evaluated, Event,
    ExceptionFilterSetting, ExceptionInfo, Output, ReverseRequest, RunInTerminal, Scope,
    SourceBreakpoint, StackTrace, Stopped, TerminalKind, Thread, Variable, VariablesFilter,
};

/// What the adapter sends on its own. Called on the reader thread, in the
/// order the adapter sent it; implementations return quickly (anything
/// slow goes to another thread, with the [`Responder`] for a request).
pub trait Handler: Send + Sync + 'static {
    fn event(&self, event: Event);

    /// A request of the adapter; answer it through `responder`, now or
    /// later, from any thread. Dropping it answers with an error.
    fn reverse_request(&self, request: ReverseRequest, responder: Responder);

    /// A chunk of the adapter's standard error.
    fn stderr(&self, chunk: &[u8]) {
        let _ = chunk;
    }

    /// Output that was not a valid message, or a message of an unknown
    /// shape.
    fn protocol_error(&self, error: String) {
        let _ = error;
    }

    /// The stream ended (`None`: not a process, or no exit status).
    /// Pending requests have been answered with [`DapError::Closed`].
    fn closed(&self, status: Option<ExitStatus>);
}

type Callback = Box<dyn FnOnce(Result<Value, DapError>) + Send>;

enum Outgoing {
    Message(Value),
    /// Close the adapter's input after everything queued before.
    Close,
}

struct State {
    next_seq: i64,
    /// Requests are accepted (the stream has not ended).
    open: bool,
    pending: HashMap<i64, Callback>,
}

struct Shared {
    state: Mutex<State>,
    outgoing: flume::Sender<Outgoing>,
    capabilities: Mutex<Capabilities>,
    process: Option<ProcessHandle>,
    /// The stream ended.
    ended: Mutex<bool>,
    ended_changed: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn send_request(&self, command: &str, arguments: Option<Value>, callback: Callback) -> i64 {
        let mut state = lock(&self.state);
        if !state.open {
            drop(state);
            callback(Err(DapError::Closed));
            return 0;
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        let mut message = Map::new();
        message.insert("seq".into(), seq.into());
        message.insert("type".into(), "request".into());
        message.insert("command".into(), command.into());
        if let Some(arguments) = arguments {
            message.insert("arguments".into(), arguments);
        }
        state.pending.insert(seq, callback);
        if self
            .outgoing
            .send(Outgoing::Message(Value::Object(message)))
            .is_err()
        {
            let callback = state.pending.remove(&seq);
            drop(state);
            if let Some(callback) = callback {
                callback(Err(DapError::Closed));
            }
        }
        seq
    }

    fn send_response(&self, request_seq: i64, command: &str, result: Result<Value, String>) {
        let mut state = lock(&self.state);
        let seq = state.next_seq;
        state.next_seq += 1;
        let mut message = Map::new();
        message.insert("seq".into(), seq.into());
        message.insert("type".into(), "response".into());
        message.insert("request_seq".into(), request_seq.into());
        message.insert("command".into(), command.into());
        match result {
            Ok(body) => {
                message.insert("success".into(), true.into());
                if !body.is_null() {
                    message.insert("body".into(), body);
                }
            }
            Err(text) => {
                message.insert("success".into(), false.into());
                message.insert("message".into(), text.clone().into());
                message.insert(
                    "body".into(),
                    json!({"error": {"id": 1, "format": text, "showUser": false}}),
                );
            }
        }
        let _ = self
            .outgoing
            .send(Outgoing::Message(Value::Object(message)));
    }

    fn take(&self, seq: i64) -> Option<Callback> {
        lock(&self.state).pending.remove(&seq)
    }

    /// The request will not be awaited: forget it and tell the adapter
    /// when it was still waiting and the adapter supports `cancel`.
    fn cancel(&self, seq: i64) {
        if self.take(seq).is_some() && lock(&self.capabilities).cancel {
            let arguments = to_value(&protocol::CancelArguments { request_id: seq });
            self.send_request("cancel", Some(arguments), Box::new(|_| {}));
        }
    }

    /// The request got no answer in time: it gets
    /// [`DapError::TimedOut`] (and the adapter is told when it supports
    /// `cancel`), unless it was answered meanwhile.
    fn expire(&self, seq: i64, after: Duration) {
        let Some(callback) = self.take(seq) else {
            return;
        };
        if lock(&self.capabilities).cancel {
            let arguments = to_value(&protocol::CancelArguments { request_id: seq });
            self.send_request("cancel", Some(arguments), Box::new(|_| {}));
        }
        callback(Err(DapError::TimedOut(after)));
    }

    /// No answers will come: every pending request gets
    /// [`DapError::Closed`], new ones get it at once.
    fn close_pending(&self) {
        let callbacks = {
            let mut state = lock(&self.state);
            state.open = false;
            std::mem::take(&mut state.pending)
        };
        for (_, callback) in callbacks {
            callback(Err(DapError::Closed));
        }
    }

    fn mark_ended(&self) {
        *lock(&self.ended) = true;
        self.ended_changed.notify_all();
    }

    /// Waits until the stream ended or `deadline` passed; whether it ended.
    fn wait_ended(&self, deadline: Instant) -> bool {
        let mut ended = lock(&self.ended);
        while !*ended {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            ended = self
                .ended_changed
                .wait_timeout(ended, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

fn to_value(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("protocol messages serialize")
}

/// The client of one adapter. Cheap to clone; the connection closes when
/// every clone is dropped (the adapter reads end of file).
#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
}

impl Client {
    /// Starts the reader and writer threads over `transport`; `name`
    /// names the threads.
    pub fn start(name: &str, transport: Transport, handler: Arc<dyn Handler>) -> io::Result<Self> {
        let Transport {
            outlet,
            inlet,
            process,
        } = transport;
        let (outgoing, queue) = flume::unbounded();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                next_seq: 1,
                open: true,
                pending: HashMap::new(),
            }),
            outgoing,
            capabilities: Mutex::new(Capabilities::default()),
            process,
            ended: Mutex::new(false),
            ended_changed: Condvar::new(),
        });
        thread::Builder::new()
            .name(format!("dap {name} writer"))
            .spawn(move || write_loop(outlet, &queue))?;
        // The reader holds the client weakly: once every handle is gone the
        // writer ends and closes the adapter's input, the adapter exits and
        // the reader ends with it.
        let reader = Arc::downgrade(&shared);
        thread::Builder::new()
            .name(format!("dap {name} reader"))
            .spawn(move || read_loop(&reader, inlet, handler.as_ref()))?;
        Ok(Self { shared })
    }

    /// What the adapter supports, as far as it said (empty before
    /// `initialize` was answered).
    pub fn capabilities(&self) -> Capabilities {
        lock(&self.shared.capabilities).clone()
    }

    /// The adapter's process, when it is one.
    pub fn process(&self) -> Option<&ProcessHandle> {
        self.shared.process.as_ref()
    }

    /// The stream ended: the adapter exited or closed it.
    pub fn is_closed(&self) -> bool {
        *lock(&self.shared.ended)
    }

    /// Requests waiting for an answer.
    pub fn pending_count(&self) -> usize {
        lock(&self.shared.state).pending.len()
    }

    /// Sends a request whose answer's body `convert` turns into a `T` on
    /// the reader thread.
    fn request<T: Send + 'static>(
        &self,
        command: &str,
        arguments: Option<Value>,
        convert: impl FnOnce(Value) -> Result<T, DapError> + Send + 'static,
    ) -> Request<T> {
        let slot = Arc::new(Slot {
            state: Mutex::new(SlotState::Waiting {
                waker: None,
                then: None,
            }),
        });
        let filled = slot.clone();
        let callback: Callback = Box::new(move |result| filled.fill(result.and_then(convert)));
        let seq = self.shared.send_request(command, arguments, callback);
        Request {
            slot,
            seq,
            shared: Some(self.shared.clone()),
        }
    }

    fn typed<T, B>(
        &self,
        command: &str,
        arguments: Option<Value>,
        convert: impl FnOnce(B) -> T + Send + 'static,
    ) -> Request<T>
    where
        T: Send + 'static,
        B: for<'de> serde::Deserialize<'de> + Default,
    {
        self.request(command, arguments, move |body| {
            protocol::body::<B>(body)
                .map(convert)
                .map_err(DapError::Malformed)
        })
    }

    fn unit(&self, command: &str, arguments: Option<Value>) -> Request<()> {
        self.request(command, arguments, |_| Ok(()))
    }

    /// `initialize`: who the client is and what it supports; the answer is
    /// the adapter's capabilities, also kept for
    /// [`Client::capabilities`].
    pub fn initialize(&self, info: &ClientInfo) -> Request<Capabilities> {
        let arguments = to_value(&protocol::InitializeArguments {
            client_id: &info.client_id,
            client_name: &info.client_name,
            adapter_id: &info.adapter_id,
            locale: &info.locale,
            lines_start_at1: true,
            columns_start_at1: true,
            path_format: "path",
            supports_variable_type: true,
            supports_variable_paging: true,
            supports_run_in_terminal_request: info.run_in_terminal,
            supports_memory_references: false,
            supports_progress_reporting: false,
            supports_invalidated_event: true,
            supports_args_can_be_interpreted_by_shell: false,
            supports_start_debugging_request: info.start_debugging,
        });
        let shared = Arc::downgrade(&self.shared);
        self.typed(
            "initialize",
            Some(arguments),
            move |body: protocol::Capabilities| {
                let mut capabilities = Capabilities::default();
                body.merge_into(&mut capabilities);
                if let Some(shared) = shared.upgrade() {
                    *lock(&shared.capabilities) = capabilities.clone();
                }
                capabilities
            },
        )
    }

    /// `launch` with the adapter's own arguments. Adapters answer it after
    /// `configurationDone`: do not wait for the answer before configuring.
    pub fn launch(&self, arguments: Value) -> Request<()> {
        self.unit("launch", Some(arguments))
    }

    /// `attach` with the adapter's own arguments.
    pub fn attach(&self, arguments: Value) -> Request<()> {
        self.unit("attach", Some(arguments))
    }

    pub fn configuration_done(&self) -> Request<()> {
        self.unit("configurationDone", None)
    }

    /// Replaces the breakpoints of a file; the answer has one breakpoint
    /// per sent one, in order.
    pub fn set_breakpoints(
        &self,
        file: &Path,
        breakpoints: &[SourceBreakpoint],
    ) -> Request<Vec<Breakpoint>> {
        let arguments = to_value(&protocol::SetBreakpointsArguments {
            source: protocol::Source {
                name: file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                path: Some(file.to_string_lossy().into_owned()),
            },
            breakpoints: breakpoints
                .iter()
                .map(|b| protocol::SourceBreakpoint {
                    line: b.line,
                    column: b.column,
                    condition: b.condition.as_deref(),
                    hit_condition: b.hit_condition.as_deref(),
                    log_message: b.log_message.as_deref(),
                })
                .collect(),
            lines: breakpoints.iter().map(|b| b.line).collect(),
            source_modified: false,
        });
        self.typed(
            "setBreakpoints",
            Some(arguments),
            |body: protocol::SetBreakpointsBody| {
                body.breakpoints.into_iter().map(Into::into).collect()
            },
        )
    }

    /// Turns on these exception filters and turns off the others.
    /// Conditions are sent when the adapter supports filter options.
    pub fn set_exception_breakpoints(&self, filters: &[ExceptionFilterSetting]) -> Request<()> {
        let options = lock(&self.shared.capabilities).exception_filter_options;
        let with_conditions = options && filters.iter().any(|f| f.condition.is_some());
        let arguments = to_value(&protocol::SetExceptionBreakpointsArguments {
            filters: if with_conditions {
                Vec::new()
            } else {
                filters.iter().map(|f| f.id.as_str()).collect()
            },
            filter_options: if with_conditions {
                filters
                    .iter()
                    .map(|f| protocol::ExceptionFilterOptions {
                        filter_id: &f.id,
                        condition: f.condition.as_deref(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        });
        self.unit("setExceptionBreakpoints", Some(arguments))
    }

    pub fn threads(&self) -> Request<Vec<Thread>> {
        self.typed("threads", None, |body: protocol::ThreadsBody| {
            body.threads
                .into_iter()
                .map(|t| Thread {
                    id: t.id,
                    name: t.name,
                })
                .collect()
        })
    }

    /// `levels` frames of a thread from `start` (0 is the top frame).
    pub fn stack_trace(&self, thread_id: i64, start: usize, levels: usize) -> Request<StackTrace> {
        let arguments = to_value(&protocol::StackTraceArguments {
            thread_id,
            start_frame: start,
            levels,
        });
        self.typed(
            "stackTrace",
            Some(arguments),
            |body: protocol::StackTraceBody| StackTrace {
                frames: body.stack_frames.into_iter().map(Into::into).collect(),
                total: body.total_frames,
            },
        )
    }

    pub fn scopes(&self, frame_id: i64) -> Request<Vec<Scope>> {
        self.typed(
            "scopes",
            Some(json!({"frameId": frame_id})),
            |body: protocol::ScopesBody| body.scopes.into_iter().map(Into::into).collect(),
        )
    }

    /// The children of a variable, a scope or a value; `page` is the first
    /// child and the number of children (all of them with `None`).
    pub fn variables(
        &self,
        reference: i64,
        filter: VariablesFilter,
        page: Option<(usize, usize)>,
    ) -> Request<Vec<Variable>> {
        let arguments = to_value(&protocol::VariablesArguments {
            variables_reference: reference,
            filter: match filter {
                VariablesFilter::All => None,
                VariablesFilter::Indexed => Some("indexed"),
                VariablesFilter::Named => Some("named"),
            },
            start: page.map(|(start, _)| start),
            count: page.map(|(_, count)| count),
        });
        self.typed(
            "variables",
            Some(arguments),
            |body: protocol::VariablesBody| body.variables.into_iter().map(Into::into).collect(),
        )
    }

    /// Evaluates an expression in a frame (the global scope with `None`).
    pub fn evaluate(
        &self,
        expression: &str,
        frame_id: Option<i64>,
        context: EvaluateContext,
    ) -> Request<Evaluated> {
        let arguments = to_value(&protocol::EvaluateArguments {
            expression,
            frame_id,
            context: context.as_str(),
        });
        self.typed("evaluate", Some(arguments), |body: protocol::ValueBody| {
            body.into()
        })
    }

    /// Sets a child of `reference` (a scope or a variable) to the value of
    /// an expression.
    pub fn set_variable(&self, reference: i64, name: &str, value: &str) -> Request<Evaluated> {
        if !lock(&self.shared.capabilities).set_variable {
            return Request::failed(DapError::Unsupported("setting values"));
        }
        let arguments = to_value(&protocol::SetVariableArguments {
            variables_reference: reference,
            name,
            value,
        });
        self.typed(
            "setVariable",
            Some(arguments),
            |body: protocol::ValueBody| body.into(),
        )
    }

    /// Assigns the value of an expression to an assignable expression.
    pub fn set_expression(
        &self,
        expression: &str,
        value: &str,
        frame_id: Option<i64>,
    ) -> Request<Evaluated> {
        if !lock(&self.shared.capabilities).set_expression {
            return Request::failed(DapError::Unsupported("setting expressions"));
        }
        let arguments = to_value(&protocol::SetExpressionArguments {
            expression,
            value,
            frame_id,
        });
        self.typed(
            "setExpression",
            Some(arguments),
            |body: protocol::ValueBody| body.into(),
        )
    }

    /// Resumes; the answer says whether every thread continued.
    pub fn resume(&self, thread_id: i64) -> Request<bool> {
        let arguments = to_value(&protocol::ThreadArguments { thread_id });
        self.typed(
            "continue",
            Some(arguments),
            |body: protocol::ContinueBody| body.all_threads_continued.unwrap_or(true),
        )
    }

    pub fn step_over(&self, thread_id: i64) -> Request<()> {
        self.unit(
            "next",
            Some(to_value(&protocol::ThreadArguments { thread_id })),
        )
    }

    pub fn step_into(&self, thread_id: i64) -> Request<()> {
        self.unit(
            "stepIn",
            Some(to_value(&protocol::ThreadArguments { thread_id })),
        )
    }

    pub fn step_out(&self, thread_id: i64) -> Request<()> {
        self.unit(
            "stepOut",
            Some(to_value(&protocol::ThreadArguments { thread_id })),
        )
    }

    pub fn pause(&self, thread_id: i64) -> Request<()> {
        self.unit(
            "pause",
            Some(to_value(&protocol::ThreadArguments { thread_id })),
        )
    }

    /// The exception a thread stopped on.
    pub fn exception_info(&self, thread_id: i64) -> Request<ExceptionInfo> {
        if !lock(&self.shared.capabilities).exception_info {
            return Request::failed(DapError::Unsupported("exception details"));
        }
        let arguments = to_value(&protocol::ThreadArguments { thread_id });
        self.typed(
            "exceptionInfo",
            Some(arguments),
            |body: protocol::ExceptionInfoBody| body.into(),
        )
    }

    /// Asks the debuggee to end gracefully.
    pub fn terminate(&self) -> Request<()> {
        if !lock(&self.shared.capabilities).terminate {
            return Request::failed(DapError::Unsupported("terminating the program"));
        }
        self.unit("terminate", Some(json!({"restart": false})))
    }

    /// Ends the session; the debuggee ends too with `terminate_debuggee`.
    pub fn disconnect(&self, terminate_debuggee: bool) -> Request<()> {
        let arguments = to_value(&protocol::DisconnectArguments {
            restart: false,
            terminate_debuggee,
        });
        self.unit("disconnect", Some(arguments))
    }

    /// A request of the adapter's own (`setDebuggerProperty`), its answer's
    /// body as it is.
    pub fn custom(&self, command: &str, arguments: Value) -> Request<Value> {
        self.request(command, Some(arguments), Ok)
    }

    /// Closes the adapter's input after the messages queued so far.
    pub fn close_input(&self) {
        let _ = self.shared.outgoing.send(Outgoing::Close);
    }

    /// Ends the adapter on a background thread and returns at once:
    /// `terminate` (when the adapter supports it and `terminate_debuggee`),
    /// then `disconnect`, then its input is closed; whatever still runs
    /// after `timeout` is killed with its process tree. The receiver gets a
    /// message when the adapter is gone.
    pub fn shutdown(&self, terminate_debuggee: bool, timeout: Duration) -> flume::Receiver<()> {
        let (done, finished) = flume::bounded(1);
        let client = self.clone();
        let spawned = thread::Builder::new()
            .name("dap shutdown".to_owned())
            .spawn(move || {
                client.shutdown_now(terminate_debuggee, timeout);
                let _ = done.send(());
            });
        if spawned.is_err() {
            self.kill();
        }
        finished
    }

    fn shutdown_now(&self, terminate_debuggee: bool, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let wait = |request: Request<()>| {
            let (tx, rx) = flume::bounded(1);
            request.on_done(move |result| {
                let _ = tx.send(result);
            });
            let _ = rx.recv_deadline(deadline);
        };
        if !self.is_closed() {
            if terminate_debuggee && self.capabilities().terminate {
                wait(self.terminate());
            }
            wait(self.disconnect(terminate_debuggee));
        }
        self.close_input();
        if !self.shared.wait_ended(deadline) {
            self.kill();
            self.shared
                .wait_ended(Instant::now() + Duration::from_secs(5));
        }
    }

    /// Kills the adapter's process tree at once.
    pub fn kill(&self) {
        if let Some(process) = &self.shared.process {
            let _ = process.kill();
        }
    }

    #[cfg(test)]
    pub(crate) fn respond_for_test(&self, seq: i64, command: &str, result: Result<Value, String>) {
        self.shared.send_response(seq, command, result);
    }
}

/// The answer to a request: a future, or a callback with
/// [`Request::on_done`]. Dropping it before the answer came cancels the
/// request.
pub struct Request<T> {
    slot: Arc<Slot<T>>,
    seq: i64,
    /// Present while the request may still be cancelled.
    shared: Option<Arc<Shared>>,
}

type Then<T> = Box<dyn FnOnce(Result<T, DapError>) + Send>;

struct Slot<T> {
    state: Mutex<SlotState<T>>,
}

enum SlotState<T> {
    Waiting {
        waker: Option<Waker>,
        then: Option<Then<T>>,
    },
    Ready(Result<T, DapError>),
    Taken,
}

impl<T> Slot<T> {
    fn fill(&self, result: Result<T, DapError>) {
        let mut state = lock(&self.state);
        match std::mem::replace(&mut *state, SlotState::Taken) {
            SlotState::Waiting {
                then: Some(then), ..
            } => {
                drop(state);
                then(result);
            }
            SlotState::Waiting { waker, then: None } => {
                *state = SlotState::Ready(result);
                drop(state);
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
            other => *state = other,
        }
    }
}

impl<T> Request<T> {
    /// A request that failed before it was sent.
    fn failed(error: DapError) -> Self {
        Self {
            slot: Arc::new(Slot {
                state: Mutex::new(SlotState::Ready(Err(error))),
            }),
            seq: 0,
            shared: None,
        }
    }

    /// The sequence number of the request (0 when it was not sent).
    pub fn seq(&self) -> i64 {
        self.seq
    }

    /// Runs `then` with the answer instead of awaiting it: on the reader
    /// thread when it comes, at once when it came already. The request is
    /// no longer cancelled.
    pub fn on_done(mut self, then: impl FnOnce(Result<T, DapError>) + Send + 'static) {
        self.shared = None;
        let mut state = lock(&self.slot.state);
        match std::mem::replace(&mut *state, SlotState::Taken) {
            SlotState::Ready(result) => {
                drop(state);
                then(result);
            }
            SlotState::Waiting { waker, .. } => {
                *state = SlotState::Waiting {
                    waker,
                    then: Some(Box::new(then)),
                };
            }
            SlotState::Taken => {}
        }
    }

    /// Keeps the request without waiting for its answer.
    pub fn detach(self) {
        self.on_done(|_| {});
    }

    /// Gives the request `after` to be answered: later it fails with
    /// [`DapError::TimedOut`] and the adapter is told to cancel it (when
    /// it supports that). An answer that comes after is dropped.
    #[must_use]
    pub fn timeout(self, after: Duration) -> Self {
        if let Some(shared) = &self.shared
            && self.seq != 0
        {
            timers::add(
                Instant::now() + after,
                Arc::downgrade(shared),
                self.seq,
                after,
            );
        }
        self
    }
}

impl<T> Future for Request<T> {
    type Output = Result<T, DapError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = lock(&self.slot.state);
        match std::mem::replace(&mut *state, SlotState::Taken) {
            SlotState::Ready(result) => {
                drop(state);
                self.shared = None;
                Poll::Ready(result)
            }
            SlotState::Waiting { then, .. } => {
                *state = SlotState::Waiting {
                    waker: Some(cx.waker().clone()),
                    then,
                };
                Poll::Pending
            }
            SlotState::Taken => Poll::Ready(Err(DapError::Closed)),
        }
    }
}

impl<T> Unpin for Request<T> {}

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.cancel(self.seq);
        }
    }
}

/// The answer to a request of the adapter. Dropping it unanswered answers
/// with an error, so the adapter never waits forever.
pub struct Responder {
    shared: Option<Arc<Shared>>,
    request_seq: i64,
    command: String,
}

impl Responder {
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Answers with a body, or fails with a message for the user.
    pub fn respond(mut self, result: Result<Value, String>) {
        if let Some(shared) = self.shared.take() {
            shared.send_response(self.request_seq, &self.command, result);
        }
    }

    /// Answers `runInTerminal`: the started process's id (and its shell's,
    /// when a shell started it), or why it could not start.
    pub fn run_in_terminal(self, result: Result<(Option<u32>, Option<u32>), String>) {
        let body = result.map(|(process_id, shell_process_id)| {
            to_value(&protocol::RunInTerminalResponse {
                process_id,
                shell_process_id,
            })
        });
        self.respond(body);
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.send_response(
                self.request_seq,
                &self.command,
                Err(format!("Pyoxide does not handle '{}'", self.command)),
            );
        }
    }
}

/// The clock of [`Request::timeout`]: one thread of the process, asleep
/// until the earliest deadline, holds the requests' deadlines weakly (a
/// closed client has nothing left to expire).
mod timers {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    use std::sync::{Condvar, Mutex, OnceLock, Weak};
    use std::time::{Duration, Instant};

    use super::{Shared, lock};

    struct Deadline {
        at: Instant,
        seq: i64,
        shared: Weak<Shared>,
        after: Duration,
    }

    impl PartialEq for Deadline {
        fn eq(&self, other: &Self) -> bool {
            (self.at, self.seq) == (other.at, other.seq)
        }
    }

    impl Eq for Deadline {}

    impl PartialOrd for Deadline {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Deadline {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            (self.at, self.seq).cmp(&(other.at, other.seq))
        }
    }

    struct Timers {
        queue: Mutex<BinaryHeap<Reverse<Deadline>>>,
        changed: Condvar,
    }

    fn timers() -> &'static Timers {
        static TIMERS: OnceLock<&'static Timers> = OnceLock::new();
        TIMERS.get_or_init(|| {
            let timers: &'static Timers = Box::leak(Box::new(Timers {
                queue: Mutex::new(BinaryHeap::new()),
                changed: Condvar::new(),
            }));
            std::thread::Builder::new()
                .name("dap timeouts".to_owned())
                .spawn(move || run(timers))
                .expect("a thread for the request timeouts");
            timers
        })
    }

    pub(super) fn add(at: Instant, shared: Weak<Shared>, seq: i64, after: Duration) {
        let timers = timers();
        lock(&timers.queue).push(Reverse(Deadline {
            at,
            seq,
            shared,
            after,
        }));
        timers.changed.notify_one();
    }

    fn run(timers: &Timers) {
        let mut queue = lock(&timers.queue);
        loop {
            let now = Instant::now();
            match queue.peek() {
                None => {
                    queue = timers
                        .changed
                        .wait(queue)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                Some(Reverse(next)) if next.at > now => {
                    let wait = next.at - now;
                    queue = timers
                        .changed
                        .wait_timeout(queue, wait)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                }
                Some(_) => {
                    let Some(Reverse(due)) = queue.pop() else {
                        continue;
                    };
                    // The callback runs without the queue locked.
                    drop(queue);
                    if let Some(shared) = due.shared.upgrade() {
                        shared.expire(due.seq, due.after);
                    }
                    queue = lock(&timers.queue);
                }
            }
        }
    }
}

fn write_loop(mut outlet: Box<dyn Outlet>, queue: &flume::Receiver<Outgoing>) {
    let mut broken = false;
    for outgoing in queue.iter() {
        let Outgoing::Message(message) = outgoing else {
            break;
        };
        if broken {
            continue;
        }
        let Ok(body) = serde_json::to_vec(&message) else {
            continue;
        };
        if outlet.write(&stream_framing::encode(&body)).is_err() {
            // The adapter does not read any more; the reader sees it end.
            broken = true;
        }
    }
    outlet.close();
}

fn read_loop(shared: &Weak<Shared>, mut inlet: Box<dyn Inlet>, handler: &dyn Handler) {
    let mut decoder = FrameDecoder::new();
    let status = loop {
        match inlet.recv() {
            Incoming::Data(chunk) => {
                decoder.push(&chunk);
                while let Some(frame) = decoder.next_frame() {
                    match frame {
                        Ok(message) => {
                            let dispatched = catch_unwind(AssertUnwindSafe(|| {
                                dispatch(shared, handler, message);
                            }));
                            if let Err(panic) = dispatched {
                                let reason = panic
                                    .downcast_ref::<String>()
                                    .map(String::as_str)
                                    .or_else(|| panic.downcast_ref::<&str>().copied())
                                    .unwrap_or("a panic");
                                handler.protocol_error(format!(
                                    "a message could not be handled: {reason}"
                                ));
                            }
                        }
                        Err(error) => handler.protocol_error(error.to_string()),
                    }
                }
            }
            Incoming::Stderr(chunk) => handler.stderr(&chunk),
            Incoming::Closed(status) => break status,
        }
    };
    if let Some(shared) = shared.upgrade() {
        shared.close_pending();
        shared.mark_ended();
    }
    handler.closed(status);
}

fn dispatch(shared: &Weak<Shared>, handler: &dyn Handler, message: Value) {
    let Some(shared) = shared.upgrade() else {
        // Nobody listens any more.
        return;
    };
    let Value::Object(mut message) = message else {
        handler.protocol_error(format!("a message that is not an object: {message}"));
        return;
    };
    let text = |message: &mut Map<String, Value>, key: &str| match message.remove(key) {
        Some(Value::String(text)) => Some(text),
        _ => None,
    };
    match message.get("type").and_then(Value::as_str) {
        Some("response") => {
            let Some(request_seq) = message.get("request_seq").and_then(Value::as_i64) else {
                handler.protocol_error(format!(
                    "an answer without request_seq: {}",
                    Value::Object(message)
                ));
                return;
            };
            // An answer to a cancelled request is dropped.
            let Some(callback) = shared.take(request_seq) else {
                return;
            };
            let success = message
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let body = message.remove("body").unwrap_or(Value::Null);
            if success {
                callback(Ok(body));
            } else {
                let short = text(&mut message, "message");
                let error = protocol::body::<protocol::ErrorBody>(body)
                    .ok()
                    .and_then(|b| b.error)
                    .map(|e| e.text())
                    .filter(|t| !t.is_empty());
                callback(Err(match (short.as_deref(), error) {
                    (Some("cancelled"), _) => DapError::Cancelled,
                    (_, Some(error)) => DapError::Adapter(error),
                    (Some(short), None) if !short.is_empty() => DapError::Adapter(short.to_owned()),
                    _ => DapError::Adapter("the request failed".to_owned()),
                }));
            }
        }
        Some("event") => {
            let Some(name) = text(&mut message, "event") else {
                handler.protocol_error("an event without a name".to_owned());
                return;
            };
            let body = message.remove("body").unwrap_or(Value::Null);
            match event(&shared, &name, body.clone()) {
                Ok(event) => handler.event(event),
                Err(error) => {
                    handler.protocol_error(format!("a malformed {name} event: {error}"));
                    handler.event(Event::Other { name, body });
                }
            }
        }
        Some("request") => {
            let seq = message.get("seq").and_then(Value::as_i64).unwrap_or(0);
            let Some(command) = text(&mut message, "command") else {
                handler.protocol_error("a request without a command".to_owned());
                return;
            };
            let arguments = message.remove("arguments").unwrap_or(Value::Null);
            let responder = Responder {
                shared: Some(shared.clone()),
                request_seq: seq,
                command: command.clone(),
            };
            match reverse_request(&command, arguments) {
                Ok(request) => handler.reverse_request(request, responder),
                Err(error) => {
                    handler.protocol_error(format!("a malformed {command} request: {error}"));
                    responder.respond(Err(format!("malformed arguments: {error}")));
                }
            }
        }
        _ => handler.protocol_error(format!(
            "a message of an unknown type: {}",
            Value::Object(message)
        )),
    }
}

fn event(shared: &Shared, name: &str, body: Value) -> Result<Event, String> {
    use protocol::body as read;
    Ok(match name {
        "initialized" => Event::Initialized,
        "stopped" => {
            let b: protocol::StoppedBody = read(body)?;
            Event::Stopped(Stopped {
                reason: b.reason,
                description: b.description,
                thread_id: b.thread_id,
                preserve_focus: b.preserve_focus_hint,
                text: b.text,
                all_threads_stopped: b.all_threads_stopped,
                hit_breakpoint_ids: b.hit_breakpoint_ids,
            })
        }
        "continued" => {
            let b: protocol::ContinuedBody = read(body)?;
            Event::Continued {
                thread_id: b.thread_id,
                all_threads: b.all_threads_continued.unwrap_or(true),
            }
        }
        "exited" => Event::Exited {
            exit_code: read::<protocol::ExitedBody>(body)?.exit_code,
        },
        "terminated" => Event::Terminated,
        "thread" => {
            let b: protocol::ThreadBody = read(body)?;
            match b.reason.as_str() {
                "exited" => Event::ThreadExited(b.thread_id),
                _ => Event::ThreadStarted(b.thread_id),
            }
        }
        "output" => {
            let b: protocol::OutputBody = read(body)?;
            Event::Output(Output {
                category: b.category.unwrap_or_else(|| "console".to_owned()),
                text: b.output,
                source: b.source.map(Into::into),
                line: b.line,
            })
        }
        "breakpoint" => {
            let b: protocol::BreakpointBody = read(body)?;
            Event::Breakpoint {
                reason: b.reason,
                breakpoint: b.breakpoint.into(),
            }
        }
        "process" => {
            let b: protocol::ProcessBody = read(body)?;
            Event::Process {
                name: b.name,
                pid: b.system_process_id,
            }
        }
        "capabilities" => {
            let b: protocol::CapabilitiesBody = read(body)?;
            let mut capabilities = lock(&shared.capabilities);
            b.capabilities.merge_into(&mut capabilities);
            Event::Capabilities(capabilities.clone())
        }
        "invalidated" => Event::Invalidated,
        _ => Event::Other {
            name: name.to_owned(),
            body,
        },
    })
}

fn reverse_request(command: &str, arguments: Value) -> Result<ReverseRequest, String> {
    Ok(match command {
        "runInTerminal" => {
            let a: protocol::RunInTerminalArguments = protocol::body(arguments)?;
            ReverseRequest::RunInTerminal(RunInTerminal {
                kind: match a.kind.as_deref() {
                    Some("external") => TerminalKind::External,
                    _ => TerminalKind::Integrated,
                },
                title: a.title,
                cwd: a.cwd.into(),
                args: a.args,
                env: a.env.into_iter().collect(),
                args_can_be_interpreted_by_shell: a.args_can_be_interpreted_by_shell,
            })
        }
        "startDebugging" => {
            let a: protocol::StartDebuggingArguments = protocol::body(arguments)?;
            ReverseRequest::StartDebugging {
                request: a.request,
                configuration: a.configuration,
            }
        }
        _ => ReverseRequest::Other {
            command: command.to_owned(),
            arguments,
        },
    })
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use futures::executor::block_on;

    use super::*;
    use crate::transport::MemoryPeer;

    const WAIT: Duration = Duration::from_secs(10);

    #[derive(Default)]
    struct Recorder {
        log: Mutex<Vec<String>>,
        responders: Mutex<Vec<(ReverseRequest, Responder)>>,
    }

    impl Handler for Recorder {
        fn event(&self, event: Event) {
            lock(&self.log).push(format!("{event:?}"));
        }

        fn reverse_request(&self, request: ReverseRequest, responder: Responder) {
            lock(&self.responders).push((request, responder));
        }

        fn protocol_error(&self, error: String) {
            lock(&self.log).push(format!("error {error}"));
        }

        fn closed(&self, status: Option<ExitStatus>) {
            lock(&self.log).push(format!("closed {status:?}"));
        }
    }

    struct Peer {
        inner: MemoryPeer,
        decoder: FrameDecoder,
    }

    impl Peer {
        fn next(&mut self) -> Value {
            loop {
                if let Some(frame) = self.decoder.next_frame() {
                    return frame.unwrap();
                }
                let chunk = self.inner.written.recv_timeout(WAIT).expect("a message");
                self.decoder.push(&chunk);
            }
        }

        fn send(&self, message: Value) {
            let frame = stream_framing::encode(&serde_json::to_vec(&message).unwrap());
            self.inner.to_client.send(Incoming::Data(frame)).unwrap();
        }

        fn respond(&self, request: &Value, body: Value) {
            self.send(
                json!({"seq": 100, "type": "response", "request_seq": request["seq"],
                "command": request["command"], "success": true, "body": body}),
            );
        }
    }

    fn connect() -> (Client, Peer, Arc<Recorder>) {
        let (transport, inner) = Transport::memory();
        let recorder = Arc::new(Recorder::default());
        let client = Client::start("test", transport, recorder.clone()).unwrap();
        let peer = Peer {
            inner,
            decoder: FrameDecoder::new(),
        };
        (client, peer, recorder)
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(start.elapsed() < WAIT, "timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn requests_have_growing_seqs_and_typed_answers() {
        let (client, mut peer, _) = connect();
        let caps = client.initialize(&ClientInfo::new("fake"));
        let initialize = peer.next();
        assert_eq!(initialize["seq"], 1);
        assert_eq!(initialize["type"], "request");
        assert_eq!(initialize["arguments"]["adapterID"], "fake");
        assert_eq!(initialize["arguments"]["linesStartAt1"], true);
        assert_eq!(
            initialize["arguments"]["supportsRunInTerminalRequest"],
            true
        );
        let threads = client.threads();
        let request = peer.next();
        assert_eq!(request["seq"], 2);
        assert!(request.get("arguments").is_none());
        peer.respond(
            &request,
            json!({"threads": [{"id": 1, "name": "MainThread"}]}),
        );
        peer.respond(
            &initialize,
            json!({"supportsCancelRequest": true, "exceptionBreakpointFilters": [
                {"filter": "raised", "label": "Raised Exceptions"},
                {"filter": "uncaught", "label": "Uncaught Exceptions", "default": true}]}),
        );
        assert_eq!(
            block_on(threads).unwrap(),
            [Thread {
                id: 1,
                name: "MainThread".into()
            }]
        );
        let caps = block_on(caps).unwrap();
        assert!(caps.cancel);
        assert_eq!(caps.exception_filters[1].id, "uncaught");
        assert!(caps.exception_filters[1].default);
        assert_eq!(client.capabilities(), caps);
    }

    #[test]
    fn error_answers() {
        let (client, mut peer, _) = connect();
        let failing = client.evaluate("x", Some(1), EvaluateContext::Watch);
        let cancelled = client.scopes(1);
        let malformed = client.threads();
        let requests: Vec<Value> = (0..3).map(|_| peer.next()).collect();
        assert_eq!(requests[0]["arguments"]["context"], "watch");
        peer.send(json!({"seq": 9, "type": "response", "request_seq": requests[0]["seq"],
            "success": false, "message": "evaluateFailed",
            "body": {"error": {"id": 2, "format": "name '{n}' is not defined", "variables": {"n": "x"}}}}));
        peer.send(
            json!({"seq": 10, "type": "response", "request_seq": requests[1]["seq"],
            "success": false, "message": "cancelled"}),
        );
        peer.respond(&requests[2], json!({"threads": 3}));
        assert_eq!(
            block_on(failing),
            Err(DapError::Adapter("name 'x' is not defined".into()))
        );
        assert_eq!(block_on(cancelled), Err(DapError::Cancelled));
        assert!(matches!(block_on(malformed), Err(DapError::Malformed(_))));
    }

    #[test]
    fn a_dropped_request_is_cancelled_when_the_adapter_supports_it() {
        let (client, mut peer, recorder) = connect();
        // Without support: forgotten, nothing sent.
        let request = client.threads();
        let first = peer.next();
        drop(request);
        assert_eq!(client.pending_count(), 0);
        peer.respond(&first, json!({"threads": []}));
        let caps = client.initialize(&ClientInfo::new("fake"));
        let initialize = peer.next();
        assert_eq!(initialize["command"], "initialize");
        peer.respond(&initialize, json!({"supportsCancelRequest": true}));
        block_on(caps).unwrap();
        // With support: `cancel` with the request's seq.
        let request = client.threads();
        let seq = request.seq();
        assert_eq!(peer.next()["seq"], seq);
        drop(request);
        let cancel = peer.next();
        assert_eq!(cancel["command"], "cancel");
        assert_eq!(cancel["arguments"], json!({"requestId": seq}));
        // An answered request is not cancelled when dropped later.
        let mut answered = client.threads();
        let request = peer.next();
        peer.respond(&request, json!({"threads": []}));
        wait_until(|| client.pending_count() <= 1);
        assert_eq!((&mut answered).now_or_never(), Some(Ok(vec![])));
        drop(answered);
        client.close_input();
        assert!(peer.inner.written.recv_timeout(WAIT).is_err());
        assert!(lock(&recorder.log).is_empty(), "{:?}", lock(&recorder.log));
    }

    #[test]
    fn on_done_runs_on_answer_or_at_once() {
        let (client, mut peer, _) = connect();
        let (tx, rx) = flume::unbounded();
        let tx2 = tx.clone();
        client
            .threads()
            .on_done(move |r| tx.send(r.map(|t| t.len())).unwrap());
        let request = peer.next();
        peer.respond(&request, json!({"threads": [{"id": 1, "name": "a"}]}));
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), Ok(1));
        client
            .exception_info(1)
            .on_done(move |r| tx2.send(r.map(|_| 0)).unwrap());
        assert_eq!(
            rx.recv_timeout(WAIT).unwrap(),
            Err(DapError::Unsupported("exception details"))
        );
    }

    #[test]
    fn events_and_reverse_requests_reach_the_handler() {
        let (client, mut peer, recorder) = connect();
        peer.send(json!({"seq": 1, "type": "event", "event": "initialized"}));
        peer.send(json!({"seq": 2, "type": "event", "event": "stopped",
            "body": {"reason": "breakpoint", "threadId": 1, "hitBreakpointIds": [3]}}));
        peer.send(json!({"seq": 3, "type": "event", "event": "output",
            "body": {"category": "stdout", "output": "hi\n"}}));
        peer.send(json!({"seq": 4, "type": "event", "event": "custom", "body": {"a": 1}}));
        peer.send(
            json!({"seq": 5, "type": "request", "command": "runInTerminal",
            "arguments": {"kind": "integrated", "cwd": "/w", "args": ["py", "-m", "x"],
                "env": {"A": "1", "B": null}}}),
        );
        peer.send(json!({"seq": 6, "type": "request", "command": "handshake", "arguments": {}}));
        wait_until(|| lock(&recorder.responders).len() == 2);
        let log = lock(&recorder.log).clone();
        assert_eq!(log[0], "Initialized");
        assert!(log[1].contains("reason: \"breakpoint\"") && log[1].contains("[3]"));
        assert!(log[2].contains("stdout") && log[2].contains("hi\\n"));
        assert!(log[3].starts_with("Other { name: \"custom\""));
        let mut responders = std::mem::take(&mut *lock(&recorder.responders));
        let (other, unhandled) = responders.pop().unwrap();
        assert!(
            matches!(other, ReverseRequest::Other { ref command, .. } if command == "handshake")
        );
        let (request, responder) = responders.pop().unwrap();
        let ReverseRequest::RunInTerminal(run) = request else {
            panic!("{request:?}");
        };
        assert_eq!(run.args, ["py", "-m", "x"]);
        assert_eq!(
            run.env,
            [
                ("A".to_owned(), Some("1".to_owned())),
                ("B".to_owned(), None)
            ]
        );
        responder.run_in_terminal(Ok((Some(42), None)));
        let answer = peer.next();
        assert_eq!(answer["type"], "response");
        assert_eq!(answer["request_seq"], 5);
        assert_eq!(answer["success"], true);
        assert_eq!(answer["body"], json!({"processId": 42}));
        drop(unhandled);
        let answer = peer.next();
        assert_eq!(answer["request_seq"], 6);
        assert_eq!(answer["success"], false);
        assert!(answer["seq"].as_i64() > Some(1));
        client.respond_for_test(77, "x", Ok(Value::Null));
        assert_eq!(peer.next()["request_seq"], 77);
    }

    #[test]
    fn the_end_of_the_stream_fails_pending_and_later_requests() {
        let (client, mut peer, recorder) = connect();
        let pending = client.threads();
        peer.next();
        peer.inner
            .to_client
            .send(Incoming::Closed(Some(ExitStatus {
                code: Some(3),
                signal: None,
            })))
            .unwrap();
        assert_eq!(block_on(pending), Err(DapError::Closed));
        wait_until(|| client.is_closed());
        assert_eq!(block_on(client.threads()), Err(DapError::Closed));
        wait_until(|| lock(&recorder.log).len() == 1);
        assert!(lock(&recorder.log)[0].contains("code: Some(3)"));
        // Shutting down a closed client finishes at once.
        client
            .shutdown(true, Duration::from_secs(5))
            .recv_timeout(WAIT)
            .unwrap();
    }
}
