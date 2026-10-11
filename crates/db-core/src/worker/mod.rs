//! A dedicated thread per connection for drivers whose client library
//! blocks (SQLite through rusqlite, Oracle through ODPI-C).
//!
//! The thread owns the driver's [`Session`] and runs every call on it, one
//! at a time and in the order the calls were sent. The async side
//! ([`Worker`]) sends requests with one-shot reply channels and awaits the
//! answers, so waiting for the database never blocks a runtime thread.
//! Shared by the drivers:
//! - The cursor page protocol. A statement that returns rows keeps them open
//!   in the session's own stack frame and hands the thread a page reader
//!   ([`Rows::serve`]); the thread answers page requests until the rows end,
//!   the cursor is closed, or the next statement arrives (the cursor was then
//!   abandoned). Other calls (`commit`, `ping`, ...) still run meanwhile.
//! - Cancel. A call that runs is stopped through the driver's [`Interrupt`];
//!   statements and page requests still queued fail with `Cancelled` without
//!   starting; with nothing outstanding a cancel does nothing, so it never
//!   hits the next statement.
//! - Per-call timeouts ([`with_timeout`]), and calls whose future is dropped
//!   before the answer: they are cancelled, or the session is marked broken
//!   ([`OnAbandon`]).
//! - The one-cursor rule ([`BusyFlag`]) and the cursor handle.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::timeout::with_timeout;
use crate::{
    BusyFlag, BusyGuard, Column, Cursor, Error, ErrorKind, ExecOptions, Outcome, Result, Row,
    TxMode, Value,
};

/// Arguments of one statement.
#[derive(Debug, Clone)]
pub struct ExecArgs {
    pub sql: String,
    pub params: Vec<Value>,
    /// The caller's time limit, for sessions that enforce it themselves.
    pub timeout: Option<Duration>,
    pub fetch_size: usize,
    pub max_value_bytes: usize,
}

/// Arguments of one page request.
#[derive(Debug, Clone)]
pub struct FetchArgs {
    /// At least 1.
    pub max_rows: usize,
    pub timeout: Option<Duration>,
    pub max_value_bytes: usize,
}

/// One page of rows.
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub rows: Vec<Row>,
    /// The end of the rows was reached.
    pub exhausted: bool,
}

/// What a statement produced, see [`Session::execute`].
#[must_use]
pub struct Executed(Produced);

enum Produced {
    Affected(u64),
    /// The rows were served by [`Rows::serve`]; the reply is already sent.
    Served,
}

impl Executed {
    /// A statement without rows: the number of affected rows.
    pub fn affected(n: u64) -> Self {
        Executed(Produced::Affected(n))
    }
}

/// The driver side of a connection: runs on the connection's thread.
///
/// Every method takes `&self`, because the session stays borrowed by an
/// open cursor while other calls (`commit`, `ping`, ...) run; keep mutable
/// state in `Cell`s.
pub trait Session: Send + Sized + 'static {
    /// Run one statement. A statement without rows returns
    /// [`Executed::affected`]. One with rows calls [`Rows::serve`] with its
    /// columns and a page reader that borrows the open statement, and
    /// returns what `serve` returns; the statement is closed when `serve`
    /// returns.
    fn execute(&self, args: ExecArgs, rows: Rows<'_, Self>) -> Result<Executed>;

    /// Cheapest round trip that proves the session is alive.
    fn ping(&self) -> Result<()>;

    fn server_version(&self) -> Result<String> {
        Err(Error::unsupported(
            "the session does not report a server version",
        ))
    }

    fn set_tx_mode(&self, mode: TxMode) -> Result<()>;
    fn commit(&self) -> Result<()>;
    fn rollback(&self) -> Result<()>;

    /// Close the session; the thread ends afterwards.
    fn close(self) -> Result<()>;
}

/// Stops the call that runs on a session, from any thread.
pub trait Interrupt: Send + Sync + 'static {
    /// Stop the running call. Only called while a call runs, with the call
    /// state locked, so the call cannot end (and a later one be hit by
    /// mistake) before the interrupt was issued.
    fn interrupt(&self) -> Result<()>;

    /// A call starts or ends: forget an earlier interrupt. Called with the
    /// call state locked.
    fn reset(&self) {}

    /// Whether [`Interrupt::interrupt`] may block; it then runs on a blocking
    /// thread instead of the caller's.
    fn may_block(&self) -> bool {
        false
    }
}

/// What happens when the future of a call is dropped before the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnAbandon {
    /// Cancel what was sent so far; the connection stays usable. Rows of an
    /// abandoned page request are kept for the next one.
    Cancel,
    /// Mark the connection broken (later calls fail with `ConnectionLost`),
    /// interrupt the running call and close the session.
    BreakSession,
}

/// How a [`Worker`] behaves.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Name of the connection thread.
    pub thread_name: String,
    pub on_abandon: OnAbandon,
    /// The session enforces `ExecArgs::timeout` itself (for example a client
    /// side call timeout). The async timer then only guards against a client
    /// that never answers: it fires at twice the limit plus 2 s.
    pub session_enforces_timeouts: bool,
}

impl WorkerConfig {
    /// When the async side gives up on a call with the time limit `timeout`.
    pub fn watchdog(&self, timeout: Option<Duration>) -> Option<Duration> {
        timeout.map(|t| {
            if self.session_enforces_timeouts {
                t.saturating_mul(2).saturating_add(Duration::from_secs(2))
            } else {
                t
            }
        })
    }
}

type Reply<T> = oneshot::Sender<Result<T>>;

enum ExecReply {
    Affected(u64),
    Rows {
        columns: Vec<Column>,
        exhausted: bool,
    },
}

/// A request for the connection thread.
enum Envelope {
    Execute {
        /// See [`Gate::next_id`].
        id: u64,
        args: ExecArgs,
        reply: Reply<ExecReply>,
    },
    Fetch {
        id: u64,
        args: FetchArgs,
        reply: Reply<Page>,
    },
    /// Close the open cursor, if any. Without a reply this is fire and
    /// forget (a dropped cursor).
    CloseCursor(Option<Reply<()>>),
    Call(Call),
    Close(Reply<()>),
}

/// Requests that do not touch a cursor.
enum Call {
    Ping(Reply<()>),
    ServerVersion(Reply<String>),
    SetTxMode(TxMode, Reply<()>),
    Commit(Reply<()>),
    Rollback(Reply<()>),
}

fn abandoned() -> Error {
    Error::connection_lost("the connection was abandoned or lost; open a new one")
}

fn gone() -> Error {
    Error::connection_lost("the connection is closed or its thread has stopped")
}

fn no_cursor() -> Error {
    Error::query("the cursor is closed")
}

enum Start {
    Run,
    Cancelled,
    Broken,
}

/// Shared state of one connection: whether a call runs (so that a cancel
/// interrupts something real), which requests were cancelled, and whether
/// the connection is still usable.
struct Gate {
    /// `true` while the thread runs a call on the session.
    in_call: Mutex<bool>,
    /// Numbers the statements and page requests in the order they are sent.
    last_id: AtomicU64,
    /// Every request up to this number was cancelled before it could run.
    cancelled_upto: AtomicU64,
    broken: AtomicBool,
    closed: AtomicBool,
    interrupt: Arc<dyn Interrupt>,
}

impl Gate {
    fn new(interrupt: Arc<dyn Interrupt>) -> Self {
        Gate {
            in_call: Mutex::new(false),
            last_id: AtomicU64::new(0),
            cancelled_upto: AtomicU64::new(0),
            broken: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            interrupt,
        }
    }

    fn lock(&self) -> MutexGuard<'_, bool> {
        self.in_call.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A number for the next statement or page request.
    fn next_id(&self) -> u64 {
        self.last_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    fn mark_broken(&self) {
        self.broken.store(true, Ordering::Release);
    }

    fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn is_unusable(&self) -> bool {
        self.is_broken() || self.closed.load(Ordering::Acquire)
    }

    fn check_usable(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::connection_lost("the connection is closed"));
        }
        if self.is_broken() {
            return Err(abandoned());
        }
        Ok(())
    }

    /// The thread picks a request up. `id` is set for statements and page
    /// requests, which a cancel can stop before they start.
    fn start(&self, id: Option<u64>) -> Start {
        if self.is_broken() {
            return Start::Broken;
        }
        let mut in_call = self.lock();
        if id.is_some_and(|id| self.cancelled_upto.load(Ordering::SeqCst) >= id) {
            return Start::Cancelled;
        }
        *in_call = true;
        self.interrupt.reset();
        Start::Run
    }

    fn end(&self) {
        let mut in_call = self.lock();
        *in_call = false;
        self.interrupt.reset();
    }

    /// A dead session cannot serve further requests.
    fn note<T>(&self, result: &Result<T>) {
        if let Err(e) = result
            && e.kind == ErrorKind::ConnectionLost
        {
            self.mark_broken();
        }
    }

    /// Cancel what was asked of the thread so far: the call that runs now is
    /// interrupted, statements and page requests still queued fail without
    /// starting. Does nothing when nothing is outstanding.
    fn cancel(&self) -> Result<()> {
        self.cancelled_upto
            .fetch_max(self.last_id.load(Ordering::SeqCst), Ordering::SeqCst);
        let in_call = self.lock();
        if *in_call {
            self.interrupt.interrupt()
        } else {
            Ok(())
        }
    }
}

/// Where the thread goes after a cursor ended.
#[derive(Default)]
struct Flow {
    /// Handle this request next (it ended the cursor).
    next: Option<Envelope>,
    /// Answer this close request once the statement is closed.
    closed: Option<Reply<()>>,
    /// Stop serving: the connection is broken or all senders are gone.
    stop: bool,
}

/// Hands the rows of a statement to the connection thread, see
/// [`Session::execute`].
pub struct Rows<'a, S: Session> {
    session: &'a S,
    rx: &'a Receiver<Envelope>,
    gate: &'a Gate,
    reply: &'a mut Option<Reply<ExecReply>>,
    flow: &'a mut Flow,
}

impl<S: Session> Rows<'_, S> {
    /// Answer the statement with `columns` and serve page requests with
    /// `fetch` until the rows end (a page with `exhausted`), `fetch` fails,
    /// the cursor is closed, or the next statement arrives. `exhausted`
    /// says that there are no rows at all; `fetch` is then never called.
    pub fn serve(
        self,
        columns: Vec<Column>,
        exhausted: bool,
        mut fetch: impl FnMut(&FetchArgs) -> Result<Page>,
    ) -> Executed {
        let Rows {
            session,
            rx,
            gate,
            reply,
            flow,
        } = self;
        // The statement call is over; page requests are calls of their own.
        gate.end();
        let Some(reply) = reply.take() else {
            return Executed(Produced::Served);
        };
        if reply
            .send(Ok(ExecReply::Rows { columns, exhausted }))
            .is_err()
            || exhausted
        {
            // Nobody waits for this cursor, or it has no rows.
            return Executed(Produced::Served);
        }
        let mut held = Held::default();
        loop {
            let Ok(envelope) = rx.recv() else {
                flow.stop = true;
                break;
            };
            match envelope {
                Envelope::Fetch { id, args, reply } => {
                    if reply.is_closed() {
                        // The caller dropped the call; keep the position.
                        continue;
                    }
                    match gate.start(Some(id)) {
                        Start::Run => {}
                        Start::Cancelled => {
                            let _ = reply.send(Err(Error::cancelled()));
                            break;
                        }
                        Start::Broken => {
                            let _ = reply.send(Err(abandoned()));
                            flow.stop = true;
                            break;
                        }
                    }
                    let page = held.page(&args, &mut fetch);
                    gate.end();
                    gate.note(&page);
                    match page {
                        Ok(page) => {
                            let last = page.exhausted;
                            match reply.send(Ok(page)) {
                                Ok(()) if last => break,
                                Ok(()) => {}
                                Err(Ok(undelivered)) => held.put_back(undelivered),
                                Err(Err(_)) => {}
                            }
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            break;
                        }
                    }
                }
                Envelope::CloseCursor(reply) => {
                    flow.closed = reply;
                    break;
                }
                // The cursor was abandoned (the async side only sends a
                // statement once no cursor object is alive), or the
                // connection closes: end the cursor first.
                next @ (Envelope::Execute { .. } | Envelope::Close(_)) => {
                    flow.next = Some(next);
                    break;
                }
                Envelope::Call(call) => {
                    run_call(session, gate, call);
                    if gate.is_broken() {
                        flow.stop = true;
                        break;
                    }
                }
            }
        }
        Executed(Produced::Served)
    }
}

/// Rows of a page that could not be delivered (the caller dropped the
/// call); the next page request gets them first.
#[derive(Default)]
struct Held {
    rows: VecDeque<Row>,
    /// The session reported the end of the rows with these.
    last: bool,
}

impl Held {
    fn put_back(&mut self, page: Page) {
        for row in page.rows.into_iter().rev() {
            self.rows.push_front(row);
        }
        self.last |= page.exhausted;
    }

    fn page(
        &mut self,
        args: &FetchArgs,
        fetch: &mut impl FnMut(&FetchArgs) -> Result<Page>,
    ) -> Result<Page> {
        if self.rows.is_empty() {
            return fetch(args);
        }
        let take = args.max_rows.min(self.rows.len());
        let mut rows: Vec<Row> = self.rows.drain(..take).collect();
        if !self.rows.is_empty() {
            return Ok(Page {
                rows,
                exhausted: false,
            });
        }
        if self.last || rows.len() == args.max_rows {
            return Ok(Page {
                rows,
                exhausted: self.last,
            });
        }
        let more = fetch(&FetchArgs {
            max_rows: args.max_rows - rows.len(),
            ..args.clone()
        })?;
        rows.extend(more.rows);
        Ok(Page {
            rows,
            exhausted: more.exhausted,
        })
    }
}

/// Run a call that does not touch a cursor and send its answer.
fn run_call<S: Session>(session: &S, gate: &Gate, call: Call) {
    fn go<T>(gate: &Gate, reply: Reply<T>, f: impl FnOnce() -> Result<T>) {
        if let Start::Broken = gate.start(None) {
            let _ = reply.send(Err(abandoned()));
            return;
        }
        let result = f();
        gate.end();
        gate.note(&result);
        let _ = reply.send(result);
    }
    match call {
        Call::Ping(reply) => go(gate, reply, || session.ping()),
        Call::ServerVersion(reply) => go(gate, reply, || session.server_version()),
        Call::SetTxMode(mode, reply) => go(gate, reply, || session.set_tx_mode(mode)),
        Call::Commit(reply) => go(gate, reply, || session.commit()),
        Call::Rollback(reply) => go(gate, reply, || session.rollback()),
    }
}

/// The connection thread: serve requests until the connection is closed,
/// broken, or dropped.
fn serve<S: Session>(session: S, rx: Receiver<Envelope>, gate: Arc<Gate>) {
    let mut flow = Flow::default();
    let mut closing = None;
    loop {
        if flow.stop || gate.is_broken() {
            break;
        }
        let envelope = match flow.next.take() {
            Some(envelope) => envelope,
            None => match rx.recv() {
                Ok(envelope) => envelope,
                Err(_) => break,
            },
        };
        match envelope {
            Envelope::Execute { id, args, reply } => {
                if reply.is_closed() {
                    // The caller gave up while the request was queued.
                    continue;
                }
                match gate.start(Some(id)) {
                    Start::Run => {}
                    Start::Cancelled => {
                        let _ = reply.send(Err(Error::cancelled()));
                        continue;
                    }
                    Start::Broken => {
                        let _ = reply.send(Err(abandoned()));
                        break;
                    }
                }
                let mut pending = Some(reply);
                let rows = Rows {
                    session: &session,
                    rx: &rx,
                    gate: &gate,
                    reply: &mut pending,
                    flow: &mut flow,
                };
                let result = session.execute(args, rows);
                // The statement is closed now.
                if let Some(closed) = flow.closed.take() {
                    let _ = closed.send(Ok(()));
                }
                // `serve` ends the call and answers itself.
                if let Some(reply) = pending {
                    gate.end();
                    let answer = match result {
                        Ok(Executed(Produced::Affected(n))) => Ok(ExecReply::Affected(n)),
                        Ok(Executed(Produced::Served)) => {
                            Err(Error::internal("the statement served no rows"))
                        }
                        Err(e) => Err(e),
                    };
                    gate.note(&answer);
                    let _ = reply.send(answer);
                }
            }
            Envelope::Fetch { reply, .. } => {
                let _ = reply.send(Err(no_cursor()));
            }
            Envelope::CloseCursor(reply) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
            }
            Envelope::Call(call) => run_call(&session, &gate, call),
            Envelope::Close(reply) => {
                closing = Some(reply);
                break;
            }
        }
    }
    gate.mark_closed();
    let result = session.close();
    if let Some(reply) = closing {
        let _ = reply.send(result);
    }
}

struct Link {
    tx: Sender<Envelope>,
    gate: Arc<Gate>,
    config: WorkerConfig,
    busy: BusyFlag,
    /// The transaction mode last set successfully.
    manual: AtomicBool,
}

impl Link {
    /// Send a request and wait for its answer. If the future is dropped
    /// before the answer arrives, the call is abandoned (see [`OnAbandon`]).
    async fn call<T>(&self, build: impl FnOnce(Reply<T>, u64) -> Envelope) -> Result<T> {
        self.gate.check_usable()?;
        let (reply, answer) = oneshot::channel();
        let id = self.gate.next_id();
        let mut in_flight = InFlight {
            link: self,
            armed: true,
        };
        if self.tx.send(build(reply, id)).is_err() {
            in_flight.armed = false;
            self.gate.mark_broken();
            return Err(gone());
        }
        let outcome = answer.await;
        in_flight.armed = false;
        match outcome {
            Ok(result) => {
                self.gate.note(&result);
                result
            }
            Err(_) => {
                self.gate.mark_broken();
                Err(gone())
            }
        }
    }

    /// [`Link::call`] with the caller's time limit: when it elapses the call
    /// is cancelled and given a grace period ([`with_timeout`]).
    async fn timed<T>(
        &self,
        timeout: Option<Duration>,
        build: impl FnOnce(Reply<T>, u64) -> Envelope,
    ) -> Result<T> {
        with_timeout(self.config.watchdog(timeout), self.call(build), async {
            let _ = self.cancel().await;
        })
        .await
    }

    async fn cancel(&self) -> Result<()> {
        if self.gate.is_unusable() {
            return Ok(());
        }
        if self.gate.interrupt.may_block() {
            let gate = self.gate.clone();
            tokio::task::spawn_blocking(move || gate.cancel())
                .await
                .map_err(|e| Error::internal(format!("cancel task failed: {e}")))?
        } else {
            self.gate.cancel()
        }
    }

    fn abandon(&self) {
        let cancel_here = match self.config.on_abandon {
            OnAbandon::Cancel => !self.gate.interrupt.may_block(),
            OnAbandon::BreakSession => {
                self.gate.mark_broken();
                false
            }
        };
        if cancel_here {
            let _ = self.gate.cancel();
            return;
        }
        // Not on the caller's thread: a drop must not block.
        let gate = self.gate.clone();
        let _ = std::thread::Builder::new()
            .name(format!("{}-cancel", self.config.thread_name))
            .spawn(move || {
                let _ = gate.cancel();
            });
    }

    /// Fire and forget: close the cursor of a dropped handle.
    fn release_cursor(&self) {
        let _ = self.tx.send(Envelope::CloseCursor(None));
    }
}

/// Abandons the call when its future is dropped before the answer.
struct InFlight<'a> {
    link: &'a Link,
    armed: bool,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.link.abandon();
        }
    }
}

/// The async side of a connection served by a dedicated thread. Drivers
/// implement [`crate::Connection`] by delegating to it.
pub struct Worker {
    link: Arc<Link>,
}

impl Worker {
    /// Start the connection thread, open the session on it with `open` and
    /// wait until that is done. `interrupt` stops calls on that session.
    pub async fn start<S, F>(
        config: WorkerConfig,
        interrupt: Arc<dyn Interrupt>,
        open: F,
    ) -> Result<Self>
    where
        S: Session,
        F: FnOnce() -> Result<S> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let gate = Arc::new(Gate::new(interrupt));
        let thread_gate = gate.clone();
        std::thread::Builder::new()
            .name(config.thread_name.clone())
            .spawn(move || {
                let session = match open() {
                    Ok(session) => session,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    // The caller gave up waiting.
                    let _ = session.close();
                    return;
                }
                serve(session, rx, thread_gate);
            })
            .map_err(|e| {
                Error::internal(format!(
                    "cannot start the {} thread: {e}",
                    config.thread_name
                ))
            })?;
        ready_rx.await.unwrap_or_else(|_| Err(gone()))?;
        Ok(Worker {
            link: Arc::new(Link {
                tx,
                gate,
                config,
                busy: BusyFlag::default(),
                manual: AtomicBool::new(false),
            }),
        })
    }

    /// Run one statement (see [`crate::Connection::execute`]): `Busy` while
    /// a cursor is open, a [`Cursor`] when the statement returns rows.
    pub async fn execute(
        &self,
        sql: &str,
        params: &[Value],
        opts: &ExecOptions,
    ) -> Result<Outcome> {
        // One statement or cursor at a time per connection.
        let guard = self.link.busy.acquire()?;
        let args = ExecArgs {
            sql: sql.to_owned(),
            params: params.to_vec(),
            timeout: opts.timeout,
            fetch_size: opts.fetch_size,
            max_value_bytes: opts.max_value_bytes,
        };
        let reply = self
            .link
            .timed(opts.timeout, |reply, id| Envelope::Execute {
                id,
                args,
                reply,
            })
            .await?;
        Ok(match reply {
            ExecReply::Affected(n) => Outcome::Affected(n),
            ExecReply::Rows { columns, exhausted } => Outcome::Rows(Box::new(WorkerCursor {
                link: self.link.clone(),
                columns,
                state: if exhausted {
                    State::Exhausted
                } else {
                    State::Open
                },
                timeout: opts.timeout,
                max_value_bytes: opts.max_value_bytes,
                guard: (!exhausted).then_some(guard),
            })),
        })
    }

    /// Stop the running statement or page request (see
    /// [`crate::Connection::cancel`]). Does nothing when nothing runs or the
    /// connection is unusable.
    pub async fn cancel(&self) -> Result<()> {
        self.link.cancel().await
    }

    pub async fn ping(&self) -> Result<()> {
        self.link
            .call(|reply, _| Envelope::Call(Call::Ping(reply)))
            .await
    }

    pub async fn server_version(&self) -> Result<String> {
        self.link
            .call(|reply, _| Envelope::Call(Call::ServerVersion(reply)))
            .await
    }

    /// The transaction mode last set successfully.
    pub fn tx_mode(&self) -> TxMode {
        if self.link.manual.load(Ordering::Acquire) {
            TxMode::Manual
        } else {
            TxMode::Auto
        }
    }

    pub async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.link
            .call(|reply, _| Envelope::Call(Call::SetTxMode(mode, reply)))
            .await?;
        self.link
            .manual
            .store(mode == TxMode::Manual, Ordering::Release);
        Ok(())
    }

    pub async fn commit(&self) -> Result<()> {
        self.link
            .call(|reply, _| Envelope::Call(Call::Commit(reply)))
            .await
    }

    pub async fn rollback(&self) -> Result<()> {
        self.link
            .call(|reply, _| Envelope::Call(Call::Rollback(reply)))
            .await
    }

    /// Close the session and end the thread, after the call that runs now
    /// (cancel it first to stop it). Closing a closed or broken connection
    /// is not an error.
    pub async fn close(&self) -> Result<()> {
        if self.link.gate.is_unusable() {
            return Ok(());
        }
        let (reply, answer) = oneshot::channel();
        if self.link.tx.send(Envelope::Close(reply)).is_err() {
            return Ok(());
        }
        answer.await.unwrap_or(Ok(()))
    }

    /// The connection was closed, or broken by an abandoned call or a lost
    /// session.
    pub fn is_unusable(&self) -> bool {
        self.link.gate.is_unusable()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    /// The thread holds the open statement.
    Open,
    /// The end of the rows was reached; the statement is closed.
    Exhausted,
    /// A page request failed; repeated by later calls.
    Failed(Error),
}

/// Rows of one statement, read page by page from the connection thread.
struct WorkerCursor {
    link: Arc<Link>,
    columns: Vec<Column>,
    state: State,
    timeout: Option<Duration>,
    max_value_bytes: usize,
    /// Keeps the connection busy while the thread holds the statement.
    guard: Option<BusyGuard>,
}

impl WorkerCursor {
    /// Ask the thread to close the statement, then free the connection. The
    /// request goes out before the guard drops, so a statement started
    /// afterwards queues behind it.
    fn release(&mut self, reply: Option<Reply<()>>) {
        let _ = self.link.tx.send(Envelope::CloseCursor(reply));
        self.guard = None;
    }
}

#[async_trait]
impl Cursor for WorkerCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        match &self.state {
            State::Exhausted => return Ok(Vec::new()),
            State::Failed(e) => return Err(e.clone()),
            State::Open => {}
        }
        let args = FetchArgs {
            max_rows: max_rows.max(1),
            timeout: self.timeout,
            max_value_bytes: self.max_value_bytes,
        };
        match self
            .link
            .timed(self.timeout, |reply, id| Envelope::Fetch {
                id,
                args,
                reply,
            })
            .await
        {
            Ok(page) => {
                if page.exhausted {
                    // The thread closed the statement before answering.
                    self.state = State::Exhausted;
                    self.guard = None;
                }
                Ok(page.rows)
            }
            Err(e) => {
                self.release(None);
                self.state = State::Failed(e.clone());
                Err(e)
            }
        }
    }

    fn is_exhausted(&self) -> bool {
        self.state != State::Open
    }

    async fn close(mut self: Box<Self>) -> Result<()> {
        if self.state != State::Open || self.link.gate.is_unusable() {
            self.guard = None;
            self.state = State::Exhausted;
            return Ok(());
        }
        let (reply, answer) = oneshot::channel();
        self.release(Some(reply));
        self.state = State::Exhausted;
        // The thread answers as soon as the statement is closed.
        answer.await.unwrap_or(Ok(()))
    }
}

impl Drop for WorkerCursor {
    fn drop(&mut self) {
        if self.state == State::Open {
            self.link.release_cursor();
        }
    }
}

#[cfg(test)]
mod tests;
