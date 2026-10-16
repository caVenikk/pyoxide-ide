//! JSON-RPC 2.0 over a server's standard streams (ADR 0010).
//!
//! A [`Connection`] has two threads:
//!
//! - the reader takes the process's events, cuts standard output into
//!   frames ([`stream_framing`]), parses them, answers pending requests
//!   (running their callbacks, where answers are converted to the model) and
//!   passes notifications, server requests, standard error and the exit to a
//!   [`Handler`];
//! - the writer serialises outgoing messages in the order they were queued
//!   (parameters may be built there, see [`Params::Lazy`]) and writes them
//!   to the process's standard input.
//!
//! No async runtime: a request is a callback in the pending table, and
//! [`Request`] is a future over a oneshot channel that sends
//! `$/cancelRequest` when it is dropped before the answer came.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};
use std::thread;

use futures::channel::oneshot;
use process::{ExitStatus, ProcessEvent, ProcessHandle};
use serde_json::{Map, Value, json};

use stream_framing::{self as framing, FrameDecoder};

/// JSON-RPC and LSP error codes.
pub(crate) mod code {
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const REQUEST_CANCELLED: i64 = -32800;
    pub const CONTENT_MODIFIED: i64 = -32801;
    pub const SERVER_CANCELLED: i64 = -32802;
}

/// Why a request has no result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RpcError {
    /// The server answered with an error.
    Server { code: i64, message: String },
    /// The connection is closed: the server exited or is being stopped.
    Closed,
    /// The answer came for a text that has changed since.
    Outdated,
    /// The answer did not have the expected form.
    Malformed(String),
}

impl RpcError {
    pub fn server(code: i64, message: impl Into<String>) -> Self {
        RpcError::Server {
            code,
            message: message.into(),
        }
    }

    /// The server gave up on the request because the text changed or it
    /// was asked to (`RequestCancelled`, `ContentModified`,
    /// `ServerCancelled`).
    pub fn is_cancelled(&self) -> bool {
        matches!(
            self,
            RpcError::Server { code, .. }
                if matches!(*code, code::REQUEST_CANCELLED | code::CONTENT_MODIFIED | code::SERVER_CANCELLED)
        )
    }

    fn from_error_object(error: Value) -> Self {
        let code = error.get("code").and_then(Value::as_i64);
        let message = error.get("message").and_then(Value::as_str);
        match (code, message) {
            (Some(code), Some(message)) => RpcError::server(code, message),
            (Some(code), None) => RpcError::server(code, ""),
            _ => RpcError::Malformed(format!("an error answer without a code: {error}")),
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Server { code, message } => write!(f, "{message} ({code})"),
            RpcError::Closed => f.write_str("the connection is closed"),
            RpcError::Outdated => f.write_str("the text changed"),
            RpcError::Malformed(message) => f.write_str(message),
        }
    }
}

/// The parameters of an outgoing message.
pub(crate) enum Params {
    /// No `params` member.
    None,
    Value(Value),
    /// Built on the writer thread, in order with the other messages: for
    /// parameters that are costly to build (the whole text of a large
    /// document), so the caller's thread only clones a rope.
    Lazy(Box<dyn FnOnce() -> Value + Send>),
}

impl From<Value> for Params {
    fn from(value: Value) -> Self {
        Params::Value(value)
    }
}

impl Params {
    fn build(self) -> Option<Value> {
        match self {
            Params::None => None,
            Params::Value(value) => Some(value),
            Params::Lazy(build) => Some(build()),
        }
    }
}

/// What the reader does with an answer.
pub(crate) type Callback = Box<dyn FnOnce(Result<Value, RpcError>) + Send>;

/// Where the writer puts frames: the server's standard input.
pub(crate) trait Outlet: Send + 'static {
    fn write(&mut self, frame: &[u8]) -> io::Result<()>;
    /// No more input: the server reads end of file.
    fn close(&mut self);
}

impl Outlet for ProcessHandle {
    fn write(&mut self, frame: &[u8]) -> io::Result<()> {
        self.write_stdin(frame)
    }

    fn close(&mut self) {
        self.close_stdin();
    }
}

/// What the server sends on its own. Called on the reader thread, in the
/// order the server sent it; implementations return quickly (anything slow
/// goes to another thread with the [`Responder`]).
pub(crate) trait Handler: Send + Sync + 'static {
    fn notification(&self, method: &str, params: Value);
    /// A server request; answer it through `responder` (now or later, from
    /// any thread).
    fn request(&self, method: &str, params: Value, responder: Responder);
    /// A chunk of the server's standard error.
    fn stderr(&self, chunk: &[u8]);
    /// Output that was not a valid message, or a message of an unknown
    /// shape.
    fn protocol_error(&self, error: String);
    /// The process exited (`None`: its events ended without an exit
    /// status). Pending requests have been answered with
    /// [`RpcError::Closed`] already.
    fn exited(&self, status: Option<ExitStatus>);
}

enum Outgoing {
    Request {
        id: i64,
        method: &'static str,
        params: Params,
    },
    Notification {
        method: &'static str,
        params: Params,
    },
    Response {
        id: Value,
        result: Result<Value, RpcError>,
    },
    /// Close standard input after everything queued before.
    Close,
}

impl Outgoing {
    fn into_message(self) -> Option<Value> {
        let mut message = Map::new();
        message.insert("jsonrpc".into(), "2.0".into());
        match self {
            Outgoing::Request { id, method, params } => {
                message.insert("id".into(), id.into());
                message.insert("method".into(), method.into());
                if let Some(params) = params.build() {
                    message.insert("params".into(), params);
                }
            }
            Outgoing::Notification { method, params } => {
                message.insert("method".into(), method.into());
                if let Some(params) = params.build() {
                    message.insert("params".into(), params);
                }
            }
            Outgoing::Response { id, result } => {
                message.insert("id".into(), id);
                match result {
                    Ok(result) => {
                        message.insert("result".into(), result);
                    }
                    Err(error) => {
                        let (code, text) = match error {
                            RpcError::Server { code, message } => (code, message),
                            other => (code::INTERNAL_ERROR, other.to_string()),
                        };
                        message.insert("error".into(), json!({"code": code, "message": text}));
                    }
                }
            }
            Outgoing::Close => return None,
        }
        Some(Value::Object(message))
    }
}

struct Shared {
    next_id: AtomicI64,
    pending: Mutex<Pending>,
    outgoing: flume::Sender<Outgoing>,
}

struct Pending {
    /// Requests are accepted (the connection has not closed).
    open: bool,
    callbacks: HashMap<i64, Callback>,
}

impl Shared {
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the callback of a request that was not answered yet.
    fn take(&self, id: i64) -> Option<Callback> {
        self.pending().callbacks.remove(&id)
    }

    /// The request will not be answered here: forget it and tell the server
    /// when it was still waiting.
    fn cancel(&self, id: i64) {
        if self.take(id).is_some() {
            let _ = self.outgoing.send(Outgoing::Notification {
                method: "$/cancelRequest",
                params: json!({ "id": id }).into(),
            });
        }
    }

    /// No answers will come: every pending request gets
    /// [`RpcError::Closed`], new ones get it at once.
    fn close_pending(&self) {
        let callbacks = {
            let mut pending = self.pending();
            pending.open = false;
            std::mem::take(&mut pending.callbacks)
        };
        for (_, callback) in callbacks {
            callback(Err(RpcError::Closed));
        }
    }
}

/// The client side of one server's JSON-RPC stream. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Connection {
    shared: Arc<Shared>,
}

impl Connection {
    /// Starts the reader and writer threads over a process's events and
    /// its standard input. `name` names the threads.
    pub fn start(
        name: &str,
        outlet: impl Outlet,
        events: flume::Receiver<ProcessEvent>,
        handler: Arc<dyn Handler>,
    ) -> io::Result<Connection> {
        let (outgoing, queue) = flume::unbounded();
        let shared = Arc::new(Shared {
            next_id: AtomicI64::new(1),
            pending: Mutex::new(Pending {
                open: true,
                callbacks: HashMap::new(),
            }),
            outgoing,
        });
        thread::Builder::new()
            .name(format!("lsp {name} writer"))
            .spawn(move || write_loop(outlet, queue))?;
        // The reader holds the connection weakly: once every handle is gone
        // the writer ends and closes the server's input, the server exits
        // and the reader ends with it.
        let reader = Arc::downgrade(&shared);
        thread::Builder::new()
            .name(format!("lsp {name} reader"))
            .spawn(move || read_loop(&reader, &events, handler.as_ref()))?;
        Ok(Connection { shared })
    }

    /// Sends a request; the reader runs `callback` with the answer (or
    /// with [`RpcError::Closed`] when none will come, at once if the
    /// connection is closed already). Returns the request id.
    pub fn request_with(
        &self,
        method: &'static str,
        params: impl Into<Params>,
        callback: Callback,
    ) -> i64 {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let rejected = {
            let mut pending = self.shared.pending();
            if pending.open {
                pending.callbacks.insert(id, callback);
                None
            } else {
                Some(callback)
            }
        };
        match rejected {
            Some(callback) => callback(Err(RpcError::Closed)),
            None => {
                let sent = self.shared.outgoing.send(Outgoing::Request {
                    id,
                    method,
                    params: params.into(),
                });
                if sent.is_err()
                    && let Some(callback) = self.shared.take(id)
                {
                    callback(Err(RpcError::Closed));
                }
            }
        }
        id
    }

    /// Sends a request whose answer `convert` turns into a `T` on the
    /// reader thread. Dropping the returned future before the answer came
    /// cancels the request (`$/cancelRequest`).
    pub fn request<T: Send + 'static>(
        &self,
        method: &'static str,
        params: impl Into<Params>,
        convert: impl FnOnce(Value) -> Result<T, RpcError> + Send + 'static,
    ) -> Request<T> {
        let (tx, rx) = oneshot::channel();
        let callback: Callback = Box::new(move |result| {
            let _ = tx.send(result.and_then(convert));
        });
        let id = self.request_with(method, params, callback);
        Request {
            rx,
            id,
            shared: Some(self.shared.clone()),
        }
    }

    pub fn notify(&self, method: &'static str, params: impl Into<Params>) {
        let _ = self.shared.outgoing.send(Outgoing::Notification {
            method,
            params: params.into(),
        });
    }

    /// Closes the server's standard input after the messages queued so far.
    pub fn close_input(&self) {
        let _ = self.shared.outgoing.send(Outgoing::Close);
    }

    /// Requests waiting for an answer.
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.shared.pending().callbacks.len()
    }
}

/// The answer to a [`Connection::request`]. Dropping it before it is ready
/// cancels the request.
pub(crate) struct Request<T> {
    rx: oneshot::Receiver<Result<T, RpcError>>,
    id: i64,
    /// Present until the answer was taken.
    shared: Option<Arc<Shared>>,
}

impl<T> Request<T> {
    #[cfg(test)]
    pub fn id(&self) -> i64 {
        self.id
    }
}

impl<T> Future for Request<T> {
    type Output = Result<T, RpcError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = match Pin::new(&mut self.rx).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(result)) => result,
            Poll::Ready(Err(oneshot::Canceled)) => Err(RpcError::Closed),
        };
        self.shared = None;
        Poll::Ready(result)
    }
}

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.cancel(self.id);
        }
    }
}

/// The answer to a server request. Dropping it unanswered answers with an
/// internal error, so the server never waits forever.
pub(crate) struct Responder {
    id: Option<Value>,
    outgoing: flume::Sender<Outgoing>,
}

impl Responder {
    pub fn respond(mut self, result: Result<Value, RpcError>) {
        if let Some(id) = self.id.take() {
            let _ = self.outgoing.send(Outgoing::Response { id, result });
        }
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self.outgoing.send(Outgoing::Response {
                id,
                result: Err(RpcError::server(
                    code::INTERNAL_ERROR,
                    "the request was not answered",
                )),
            });
        }
    }
}

fn write_loop(mut outlet: impl Outlet, queue: flume::Receiver<Outgoing>) {
    let mut broken = false;
    for outgoing in queue.iter() {
        let Some(message) = outgoing.into_message() else {
            break;
        };
        if broken {
            continue;
        }
        let Ok(body) = serde_json::to_vec(&message) else {
            continue;
        };
        if outlet.write(&framing::encode(&body)).is_err() {
            // The process does not read any more; the reader sees it exit.
            broken = true;
        }
    }
    outlet.close();
}

fn read_loop(shared: &Weak<Shared>, events: &flume::Receiver<ProcessEvent>, handler: &dyn Handler) {
    let mut decoder = FrameDecoder::new();
    let status = loop {
        match events.recv() {
            Ok(ProcessEvent::Stdout(chunk)) => {
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
            Ok(ProcessEvent::Stderr(chunk)) => handler.stderr(&chunk),
            Ok(ProcessEvent::Exited(status)) => break Some(status),
            Err(_) => break None,
        }
    };
    if let Some(shared) = shared.upgrade() {
        shared.close_pending();
    }
    handler.exited(status);
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
    let id = message.remove("id").filter(|id| !id.is_null());
    match message.remove("method") {
        Some(Value::String(method)) => {
            let params = message.remove("params").unwrap_or(Value::Null);
            match id {
                Some(id) => {
                    let responder = Responder {
                        id: Some(id),
                        outgoing: shared.outgoing.clone(),
                    };
                    handler.request(&method, params, responder);
                }
                None => handler.notification(&method, params),
            }
        }
        Some(method) => handler.protocol_error(format!("a message with method {method}")),
        None => {
            let Some(id) = id.as_ref().and_then(Value::as_i64) else {
                handler.protocol_error(format!(
                    "an answer to an unknown request: {}",
                    id.unwrap_or(Value::Null)
                ));
                return;
            };
            let result = match message.remove("error").filter(|e| !e.is_null()) {
                Some(error) => Err(RpcError::from_error_object(error)),
                None => Ok(message.remove("result").unwrap_or(Value::Null)),
            };
            // An answer to a cancelled request is dropped.
            if let Some(callback) = shared.take(id) {
                callback(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::FutureExt;
    use futures::executor::block_on;

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    /// The other end of a connection in memory: what the client writes
    /// arrives as decoded messages, what the test sends arrives at the
    /// client as standard output chunks.
    struct Peer {
        written: flume::Receiver<Vec<u8>>,
        decoder: FrameDecoder,
        to_client: flume::Sender<ProcessEvent>,
        closed: flume::Receiver<()>,
    }

    struct MemoryOutlet {
        written: flume::Sender<Vec<u8>>,
        closed: flume::Sender<()>,
    }

    impl Outlet for MemoryOutlet {
        fn write(&mut self, frame: &[u8]) -> io::Result<()> {
            self.written
                .send(frame.to_vec())
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn close(&mut self) {
            let _ = self.closed.send(());
        }
    }

    #[derive(Default)]
    struct Recorder {
        log: Mutex<Vec<String>>,
        responders: Mutex<Vec<(String, Value, Responder)>>,
        exited: Mutex<Option<Option<ExitStatus>>>,
    }

    impl Recorder {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Handler for Recorder {
        fn notification(&self, method: &str, params: Value) {
            self.log.lock().unwrap().push(format!("{method} {params}"));
        }

        fn request(&self, method: &str, params: Value, responder: Responder) {
            self.responders
                .lock()
                .unwrap()
                .push((method.to_owned(), params, responder));
        }

        fn stderr(&self, chunk: &[u8]) {
            self.log
                .lock()
                .unwrap()
                .push(format!("stderr {}", String::from_utf8_lossy(chunk)));
        }

        fn protocol_error(&self, error: String) {
            self.log.lock().unwrap().push(format!("error {error}"));
        }

        fn exited(&self, status: Option<ExitStatus>) {
            *self.exited.lock().unwrap() = Some(status);
        }
    }

    fn connect() -> (Connection, Peer, Arc<Recorder>) {
        let (written_tx, written) = flume::unbounded();
        let (closed_tx, closed) = flume::unbounded();
        let (to_client, events) = flume::unbounded();
        let recorder = Arc::new(Recorder::default());
        let connection = Connection::start(
            "test",
            MemoryOutlet {
                written: written_tx,
                closed: closed_tx,
            },
            events,
            recorder.clone(),
        )
        .unwrap();
        let peer = Peer {
            written,
            decoder: FrameDecoder::new(),
            to_client,
            closed,
        };
        (connection, peer, recorder)
    }

    impl Peer {
        fn next(&mut self) -> Value {
            loop {
                if let Some(frame) = self.decoder.next_frame() {
                    return frame.unwrap();
                }
                let chunk = self.written.recv_timeout(WAIT).expect("a message");
                self.decoder.push(&chunk);
            }
        }

        fn send(&self, message: Value) {
            let frame = framing::encode(&serde_json::to_vec(&message).unwrap());
            self.send_raw(&frame);
        }

        fn send_raw(&self, bytes: &[u8]) {
            self.to_client
                .send(ProcessEvent::Stdout(bytes.to_vec()))
                .unwrap();
        }

        fn exit(&self, code: i32) {
            self.to_client
                .send(ProcessEvent::Exited(ExitStatus {
                    code: Some(code),
                    signal: None,
                }))
                .unwrap();
        }
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let start = std::time::Instant::now();
        while !condition() {
            assert!(start.elapsed() < WAIT, "timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn requests_get_their_answers_in_any_order() {
        let (connection, mut peer, _) = connect();
        let a = connection.request("a", json!({"x": 1}), Ok);
        let b = connection.request("b", Params::None, |v| {
            v.as_i64().ok_or(RpcError::Malformed("not a number".into()))
        });
        let first = peer.next();
        assert_eq!(first["method"], "a");
        assert_eq!(first["params"], json!({"x": 1}));
        assert_eq!(first["jsonrpc"], "2.0");
        let second = peer.next();
        assert_eq!(second["method"], "b");
        assert!(second.get("params").is_none());
        peer.send(json!({"jsonrpc": "2.0", "id": second["id"], "result": 42}));
        peer.send(json!({"jsonrpc": "2.0", "id": first["id"], "result": {"ok": true}}));
        assert_eq!(block_on(b), Ok(42));
        assert_eq!(block_on(a), Ok(json!({"ok": true})));
        assert_eq!(connection.pending_count(), 0);
    }

    #[test]
    fn error_answers_and_conversion_failures() {
        let (connection, mut peer, _) = connect();
        let failing = connection.request("x", Params::None, Ok);
        let cancelled = connection.request("y", Params::None, Ok);
        let malformed = connection.request("z", Params::None, |v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or(RpcError::Malformed("not a string".into()))
        });
        let ids: Vec<Value> = (0..3).map(|_| peer.next()["id"].clone()).collect();
        peer.send(json!({"id": ids[0], "error": {"code": -32603, "message": "boom"}}));
        peer.send(json!({"id": ids[1], "error": {"code": -32801, "message": "modified"}}));
        peer.send(json!({"id": ids[2], "result": 7}));
        assert_eq!(
            block_on(failing),
            Err(RpcError::server(code::INTERNAL_ERROR, "boom"))
        );
        let cancelled = block_on(cancelled).unwrap_err();
        assert!(cancelled.is_cancelled());
        assert_eq!(
            block_on(malformed),
            Err(RpcError::Malformed("not a string".into()))
        );
    }

    #[test]
    fn a_dropped_request_is_cancelled_and_its_late_answer_ignored() {
        let (connection, mut peer, recorder) = connect();
        let request = connection.request("slow", Params::None, Ok);
        let id = request.id();
        assert_eq!(peer.next()["id"], id);
        drop(request);
        let cancel = peer.next();
        assert_eq!(cancel["method"], "$/cancelRequest");
        assert_eq!(cancel["params"], json!({"id": id}));
        assert!(cancel.get("id").is_none());
        assert_eq!(connection.pending_count(), 0);
        // The server answers anyway: nothing happens.
        peer.send(json!({"id": id, "result": 1}));
        let next = connection.request("next", Params::None, Ok);
        let next_id = peer.next()["id"].clone();
        peer.send(json!({"id": next_id, "result": 2}));
        assert_eq!(block_on(next), Ok(json!(2)));
        assert!(recorder.log().is_empty(), "{:?}", recorder.log());
    }

    #[test]
    fn an_answered_request_is_not_cancelled_when_dropped() {
        let (connection, mut peer, _) = connect();
        let mut request = connection.request("quick", Params::None, Ok);
        let id = peer.next()["id"].clone();
        peer.send(json!({"id": id, "result": null}));
        wait_until(|| connection.pending_count() == 0);
        // Answered but not polled yet, then polled, then dropped.
        assert_eq!((&mut request).now_or_never(), Some(Ok(Value::Null)));
        drop(request);
        connection.notify("marker", Params::None);
        assert_eq!(peer.next()["method"], "marker");
    }

    #[test]
    fn notifications_and_lazy_parameters_keep_their_order() {
        let (connection, mut peer, _) = connect();
        connection.notify("one", json!([1]));
        connection.notify(
            "two",
            Params::Lazy(Box::new(|| json!({"text": "built later"}))),
        );
        connection.notify("three", Params::None);
        assert_eq!(peer.next()["method"], "one");
        let two = peer.next();
        assert_eq!(two["params"]["text"], "built later");
        assert_eq!(peer.next()["method"], "three");
    }

    #[test]
    fn server_requests_and_notifications_reach_the_handler() {
        let (_connection, mut peer, recorder) = connect();
        peer.send(json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {"type": 4, "message": "hi"}}));
        peer.send(json!({"jsonrpc": "2.0", "id": "s1", "method": "workspace/configuration", "params": {"items": []}}));
        peer.send(
            json!({"jsonrpc": "2.0", "id": 5, "method": "client/registerCapability", "params": {}}),
        );
        wait_until(|| recorder.responders.lock().unwrap().len() == 2);
        // Formatted as the handler formats it, whatever the key order of
        // JSON objects is in this build.
        let expected = format!("window/logMessage {}", json!({"type": 4, "message": "hi"}));
        assert_eq!(recorder.log(), [expected]);
        let mut responders = std::mem::take(&mut *recorder.responders.lock().unwrap());
        let (method, _, unanswered) = responders.pop().unwrap();
        assert_eq!(method, "client/registerCapability");
        let (method, params, responder) = responders.pop().unwrap();
        assert_eq!(method, "workspace/configuration");
        assert_eq!(params, json!({"items": []}));
        responder.respond(Ok(json!([null])));
        let answer = peer.next();
        assert_eq!(
            answer,
            json!({"jsonrpc": "2.0", "id": "s1", "result": [null]})
        );
        // Dropped without an answer: an error answer.
        drop(unanswered);
        let answer = peer.next();
        assert_eq!(answer["id"], 5);
        assert_eq!(answer["error"]["code"], code::INTERNAL_ERROR);
    }

    #[test]
    fn malformed_output_is_reported_and_reading_goes_on() {
        let (connection, mut peer, recorder) = connect();
        let request = connection.request("r", Params::None, Ok);
        let id = peer.next()["id"].clone();
        peer.send_raw(b"some log line\r\n\r\n");
        peer.send_raw(&framing::encode(b"[1, 2]"));
        peer.send_raw(&framing::encode(b"{\"method\": 3}"));
        peer.send_raw(&framing::encode(b"{\"id\": \"unknown\", \"result\": 1}"));
        peer.send_raw(&framing::encode(b"{broken"));
        peer.to_client
            .send(ProcessEvent::Stderr(b"warning: x\n".to_vec()))
            .unwrap();
        // The answer split in the middle of a multi-byte character.
        let answer = serde_json::to_vec(&json!({"id": id, "result": "é🦀"})).unwrap();
        let frame = framing::encode(&answer);
        let cut = frame.len() - 5;
        peer.send_raw(&frame[..cut]);
        peer.send_raw(&frame[cut..]);
        assert_eq!(block_on(request), Ok(json!("é🦀")));
        let log = recorder.log();
        assert_eq!(log.len(), 6, "{log:?}");
        assert!(log[0].starts_with("error a message header without Content-Length"));
        assert!(log[1].starts_with("error a message that is not an object"));
        assert!(log[2].starts_with("error a message with method 3"));
        assert!(log[3].starts_with("error an answer to an unknown request"));
        assert!(log[4].starts_with("error a message that is not JSON"));
        assert_eq!(log[5], "stderr warning: x\n");
    }

    #[test]
    fn exit_fails_pending_and_later_requests() {
        let (connection, mut peer, recorder) = connect();
        let pending = connection.request("never", Params::None, Ok);
        peer.next();
        peer.exit(3);
        assert_eq!(block_on(pending), Err(RpcError::Closed));
        wait_until(|| recorder.exited.lock().unwrap().is_some());
        assert_eq!(
            *recorder.exited.lock().unwrap(),
            Some(Some(ExitStatus {
                code: Some(3),
                signal: None
            }))
        );
        let late = connection.request("late", Params::None, Ok);
        assert_eq!(block_on(late), Err(RpcError::Closed));
        let (tx, rx) = flume::bounded(1);
        connection.request_with(
            "late",
            Params::None,
            Box::new(move |r| {
                let _ = tx.send(r);
            }),
        );
        assert_eq!(rx.recv_timeout(WAIT), Ok(Err(RpcError::Closed)));
    }

    #[test]
    fn closing_input_flushes_first_and_dropping_everything_closes_it() {
        let (connection, mut peer, _) = connect();
        connection.notify("exit", Params::None);
        connection.close_input();
        connection.notify("after-close", Params::None);
        assert_eq!(peer.next()["method"], "exit");
        peer.closed.recv_timeout(WAIT).unwrap();
        assert!(
            peer.written
                .recv_timeout(Duration::from_millis(50))
                .is_err()
        );

        let (connection, peer, _) = connect();
        drop(connection);
        peer.closed.recv_timeout(WAIT).unwrap();
    }
}
