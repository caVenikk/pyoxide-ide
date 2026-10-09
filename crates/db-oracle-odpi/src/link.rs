//! The channel between async callers and the thread that owns a session.
//!
//! ODPI-C calls block. Every connection therefore has one dedicated thread
//! that runs all calls on its session, one at a time, in the order they were
//! sent. Async callers send a [`Request`] with a one-shot reply channel and
//! await the answer; nothing blocks a tokio worker. `cancel` bypasses the
//! queue: it asks the client library to break the call that is running right
//! now, from whichever thread it is called on.
//!
//! The thread talks to the database only through the [`Session`] trait and
//! cancels through [`Breaker`], so the whole protocol (paging, the one-cursor
//! rule, cancel, timeouts, abandoned calls) is tested with a scripted session.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use db_core::timeout::with_timeout;
use db_core::{Column, Error, ErrorKind, Result, Row, TxMode, Value};
use tokio::sync::oneshot;

/// Arguments of one statement execution.
#[derive(Debug, Clone)]
pub(crate) struct ExecArgs {
    pub sql: String,
    pub params: Vec<Value>,
    pub timeout: Option<Duration>,
    pub fetch_size: usize,
}

pub(crate) enum ExecReply {
    Affected(u64),
    /// A cursor is open in the session; fetch it with [`FetchArgs`].
    Rows(Vec<Column>),
}

#[derive(Debug, Clone)]
pub(crate) struct FetchArgs {
    pub max_rows: usize,
    pub timeout: Option<Duration>,
    pub max_value_bytes: usize,
}

pub(crate) struct Page {
    pub rows: Vec<Row>,
    /// The session saw the end of the rows and released the server cursor.
    pub exhausted: bool,
}

/// What the connection thread does with a database session.
pub(crate) trait Session: Send + 'static {
    fn execute(&mut self, args: ExecArgs) -> Result<ExecReply>;
    fn fetch(&mut self, args: FetchArgs) -> Result<Page>;
    /// Release the open cursor, if any. Idempotent.
    fn close_cursor(&mut self) -> Result<()>;
    fn ping(&mut self) -> Result<()>;
    fn server_version(&mut self) -> Result<String>;
    fn set_tx_mode(&mut self, mode: TxMode) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    fn rollback(&mut self) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}

/// Interrupts the call running on a session. Must be callable from any
/// thread while the session thread is inside a call.
pub(crate) trait Breaker: Send + Sync + 'static {
    fn break_execution(&self) -> Result<()>;
}

pub(crate) type Reply<T> = oneshot::Sender<Result<T>>;

pub(crate) enum Request {
    Execute(ExecArgs, Reply<ExecReply>),
    Fetch(FetchArgs, Reply<Page>),
    /// Without a reply this is fire-and-forget (a dropped cursor).
    CloseCursor(Option<Reply<()>>),
    Ping(Reply<()>),
    ServerVersion(Reply<String>),
    SetTxMode(TxMode, Reply<()>),
    Commit(Reply<()>),
    Rollback(Reply<()>),
    Close(Reply<()>),
}

#[derive(Debug, Default)]
struct CallState {
    /// The session thread is inside a call.
    in_call: bool,
    /// A cancel arrived while requests were queued but no call was running:
    /// the next request that starts is answered with `Cancelled` instead.
    cancel_pending: bool,
    /// Requests sent and not yet finished (running or waiting).
    queued: usize,
}

/// Shared state of one connection: whether a call is running (so a cancel
/// breaks something real) and whether the connection is still usable.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    state: Mutex<CallState>,
    broken: AtomicBool,
    closed: AtomicBool,
}

impl Gate {
    fn lock(&self) -> MutexGuard<'_, CallState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    pub(crate) fn mark_broken(&self) {
        self.broken.store(true, Ordering::Release);
    }

    pub(crate) fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub(crate) fn is_unusable(&self) -> bool {
        self.is_broken() || self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn check_usable(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::connection_lost("the connection is closed"));
        }
        if self.is_broken() {
            return Err(Error::connection_lost(
                "the connection was abandoned or lost; open a new one",
            ));
        }
        Ok(())
    }

    /// A request is about to be sent.
    fn enqueue(&self) {
        self.lock().queued += 1;
    }

    /// The session thread picked a request up. `false`: a cancel was waiting
    /// for it, answer `Cancelled` without touching the database.
    fn begin(&self) -> bool {
        let mut st = self.lock();
        if st.cancel_pending {
            st.cancel_pending = false;
            st.queued = st.queued.saturating_sub(1);
            return false;
        }
        st.in_call = true;
        true
    }

    fn end(&self) {
        let mut st = self.lock();
        st.in_call = false;
        st.queued = st.queued.saturating_sub(1);
    }

    /// Request that the running statement stops.
    ///
    /// With a call in progress `brk` runs while the state is locked, so the
    /// call cannot finish (and an idle session be interrupted by mistake)
    /// before the break was issued. With requests queued but no call running,
    /// the next one is cancelled instead. With nothing to cancel this does
    /// nothing at all: a break sent to an idle session could otherwise fail
    /// the next unrelated statement.
    pub(crate) fn cancel(&self, brk: impl FnOnce() -> Result<()>) -> Result<()> {
        let mut st = self.lock();
        if st.in_call {
            brk()
        } else {
            if st.queued > 0 {
                st.cancel_pending = true;
            }
            Ok(())
        }
    }
}

/// The async side of one connection.
pub(crate) struct Link {
    tx: mpsc::Sender<Request>,
    gate: Arc<Gate>,
    breaker: Arc<dyn Breaker>,
    tx_auto: AtomicBool,
    /// The client library supports `set_call_timeout` (18c and later), so
    /// timeouts are enforced by the client and the async watchdog is only a
    /// backstop. Otherwise the watchdog breaks the call itself.
    call_timeout_supported: bool,
}

impl Link {
    /// Start the session thread.
    pub(crate) fn spawn<S: Session>(
        session: S,
        breaker: Arc<dyn Breaker>,
        call_timeout_supported: bool,
    ) -> Result<Arc<Link>> {
        let (tx, rx) = mpsc::channel();
        let gate = Arc::new(Gate::default());
        let thread_gate = gate.clone();
        std::thread::Builder::new()
            .name("pyoxide-odpi".into())
            .spawn(move || worker_loop(session, rx, thread_gate))
            .map_err(|e| Error::internal(format!("cannot start the connection thread: {e}")))?;
        Ok(Arc::new(Link {
            tx,
            gate,
            breaker,
            tx_auto: AtomicBool::new(true),
            call_timeout_supported,
        }))
    }

    pub(crate) fn gate(&self) -> &Gate {
        &self.gate
    }

    pub(crate) fn tx_mode(&self) -> TxMode {
        if self.tx_auto.load(Ordering::Acquire) {
            TxMode::Auto
        } else {
            TxMode::Manual
        }
    }

    pub(crate) fn note_tx_mode(&self, mode: TxMode) {
        self.tx_auto.store(mode == TxMode::Auto, Ordering::Release);
    }

    /// Send a request and wait for its answer.
    ///
    /// If the returned future is dropped before the answer arrives, the call
    /// is still running or queued in the session thread and its outcome is
    /// lost. The connection is then marked broken (every later call fails with
    /// `ConnectionLost`), the running call is interrupted, and the session
    /// thread closes the session. The contract allows exactly this: a dropped
    /// future leaves the connection usable or broken, never half-way.
    pub(crate) async fn call<T>(&self, build: impl FnOnce(Reply<T>) -> Request) -> Result<T> {
        self.gate.check_usable()?;
        let (reply, answer) = oneshot::channel();
        self.gate.enqueue();
        let mut in_flight = InFlight {
            link: self,
            answered: false,
        };
        if self.tx.send(build(reply)).is_err() {
            in_flight.answered = true;
            self.gate.mark_broken();
            return Err(Error::connection_lost("the connection thread has stopped"));
        }
        let outcome = answer.await;
        in_flight.answered = true;
        match outcome {
            Ok(result) => {
                if let Err(e) = &result
                    && e.kind == ErrorKind::ConnectionLost
                {
                    self.gate.mark_broken();
                }
                result
            }
            Err(_) => {
                self.gate.mark_broken();
                Err(Error::connection_lost("the connection thread has stopped"))
            }
        }
    }

    /// [`Link::call`] with the caller's time limit. When it elapses the
    /// running statement is cancelled and given a grace period (see
    /// [`with_timeout`]); the error is then `ErrorKind::Timeout`.
    pub(crate) async fn timed<T>(
        self: &Arc<Self>,
        timeout: Option<Duration>,
        build: impl FnOnce(Reply<T>) -> Request,
    ) -> Result<T> {
        let watchdog = watchdog_limit(timeout, self.call_timeout_supported);
        with_timeout(watchdog, self.call(build), async {
            let _ = self.cancel().await;
        })
        .await
    }

    /// Interrupt the running statement, from any task. Does nothing when no
    /// statement runs or the connection is already unusable.
    pub(crate) async fn cancel(self: &Arc<Self>) -> Result<()> {
        if self.gate.is_unusable() {
            return Ok(());
        }
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.cancel_blocking())
            .await
            .map_err(|e| Error::internal(format!("cancel task failed: {e}")))?
    }

    fn cancel_blocking(&self) -> Result<()> {
        self.gate.cancel(|| self.breaker.break_execution())
    }

    /// Fire-and-forget: release the server cursor of a dropped `Cursor`.
    pub(crate) fn release_cursor(&self) {
        let _ = self.tx.send(Request::CloseCursor(None));
    }

    /// A call was abandoned (its future dropped before the answer).
    fn abandon(&self) {
        self.gate.mark_broken();
        let gate = self.gate.clone();
        let breaker = self.breaker.clone();
        // Not on the caller's thread: a drop must not block, and the break may
        // wait for the client library.
        let _ = std::thread::Builder::new()
            .name("pyoxide-odpi-break".into())
            .spawn(move || {
                let _ = gate.cancel(|| breaker.break_execution());
            });
    }
}

struct InFlight<'a> {
    link: &'a Link,
    answered: bool,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if !self.answered {
            self.link.abandon();
        }
    }
}

/// When the async side gives up on a call that has a time limit. With
/// client-side call timeouts the client raises its own error at the limit
/// (and may need as long again to clean up), so the watchdog only guards
/// against a client that never answers. Without them it is the timer.
pub(crate) fn watchdog_limit(
    timeout: Option<Duration>,
    call_timeout_supported: bool,
) -> Option<Duration> {
    timeout.map(|t| {
        if call_timeout_supported {
            t.saturating_mul(2).saturating_add(Duration::from_secs(2))
        } else {
            t
        }
    })
}

fn worker_loop<S: Session>(mut session: S, rx: mpsc::Receiver<Request>, gate: Arc<Gate>) {
    while let Ok(request) = rx.recv() {
        let keep_going = match request {
            Request::CloseCursor(None) => {
                let _ = session.close_cursor();
                true
            }
            Request::CloseCursor(Some(reply)) => gated(&gate, reply, || session.close_cursor()),
            Request::Ping(reply) => gated(&gate, reply, || session.ping()),
            Request::ServerVersion(reply) => gated(&gate, reply, || session.server_version()),
            Request::SetTxMode(mode, reply) => gated(&gate, reply, || session.set_tx_mode(mode)),
            Request::Commit(reply) => gated(&gate, reply, || session.commit()),
            Request::Rollback(reply) => gated(&gate, reply, || session.rollback()),
            Request::Fetch(args, reply) => gated(&gate, reply, || session.fetch(args)),
            Request::Execute(args, reply) => {
                let mut cursor_open = false;
                let mut answer = None;
                let keep = gated_with(
                    &gate,
                    || {
                        let r = session.execute(args);
                        cursor_open = matches!(r, Ok(ExecReply::Rows(_)));
                        r
                    },
                    |r| answer = Some(r),
                );
                if let Some(r) = answer
                    && reply.send(r).is_err()
                    && cursor_open
                {
                    // Nobody is waiting for this cursor any more.
                    let _ = session.close_cursor();
                }
                keep
            }
            Request::Close(reply) => {
                let r = session.close();
                gate.mark_closed();
                let _ = reply.send(r);
                return;
            }
        };
        if !keep_going || gate.is_broken() {
            break;
        }
    }
    // The connection was dropped or abandoned: free the session.
    gate.mark_closed();
    let _ = session.close();
}

/// Run one request: skip it when the connection is broken or a cancel was
/// waiting for it; otherwise run it inside the gate and send the answer.
fn gated<T>(gate: &Gate, reply: Reply<T>, f: impl FnOnce() -> Result<T>) -> bool {
    let mut answer = None;
    let keep = gated_with(gate, f, |r| answer = Some(r));
    if let Some(r) = answer {
        let _ = reply.send(r);
    }
    keep
}

/// Returns `false` when the worker should stop after this request.
fn gated_with<T>(
    gate: &Gate,
    f: impl FnOnce() -> Result<T>,
    deliver: impl FnOnce(Result<T>),
) -> bool {
    if gate.is_broken() {
        deliver(Err(Error::connection_lost(
            "the connection was abandoned or lost",
        )));
        gate.end_skipped();
        return false;
    }
    if !gate.begin() {
        deliver(Err(Error::cancelled()));
        return true;
    }
    let result = f();
    gate.end();
    // A dead session cannot serve further requests.
    if let Err(e) = &result
        && e.kind == ErrorKind::ConnectionLost
    {
        gate.mark_broken();
    }
    deliver(result);
    true
}

impl Gate {
    /// A queued request was dropped without running.
    fn end_skipped(&self) {
        let mut st = self.lock();
        st.queued = st.queued.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::testing::{Script, mock_link};

    #[test]
    fn gate_cancel_with_nothing_running_does_nothing() {
        let gate = Gate::default();
        let mut broke = false;
        gate.cancel(|| {
            broke = true;
            Ok(())
        })
        .unwrap();
        assert!(!broke);
        // And it does not poison the next request.
        gate.enqueue();
        assert!(gate.begin());
        gate.end();
    }

    #[test]
    fn gate_breaks_only_inside_a_call() {
        let gate = Gate::default();
        gate.enqueue();
        assert!(gate.begin());
        let mut broke = false;
        gate.cancel(|| {
            broke = true;
            Ok(())
        })
        .unwrap();
        assert!(broke);
        gate.end();
        // The call is over: another cancel is a no-op again.
        broke = false;
        gate.cancel(|| {
            broke = true;
            Ok(())
        })
        .unwrap();
        assert!(!broke);
    }

    #[test]
    fn gate_cancel_of_a_queued_request_cancels_it_once() {
        let gate = Gate::default();
        gate.enqueue();
        gate.cancel(|| panic!("nothing runs yet")).unwrap();
        assert!(!gate.begin(), "the queued request is cancelled");
        // The flag was consumed.
        gate.enqueue();
        assert!(gate.begin());
        gate.end();
    }

    #[test]
    fn gate_usability() {
        let gate = Gate::default();
        assert!(gate.check_usable().is_ok());
        gate.mark_broken();
        assert_eq!(
            gate.check_usable().unwrap_err().kind,
            ErrorKind::ConnectionLost
        );
        let gate = Gate::default();
        gate.mark_closed();
        assert_eq!(
            gate.check_usable().unwrap_err().kind,
            ErrorKind::ConnectionLost
        );
        assert!(gate.is_unusable());
    }

    #[test]
    fn watchdog_depends_on_client_call_timeouts() {
        let s = Duration::from_secs;
        assert_eq!(watchdog_limit(None, true), None);
        assert_eq!(watchdog_limit(None, false), None);
        assert_eq!(watchdog_limit(Some(s(10)), false), Some(s(10)));
        assert_eq!(watchdog_limit(Some(s(10)), true), Some(s(22)));
    }

    // ---- the protocol, against a scripted session ----------------------

    fn link(supported: bool) -> (Arc<Link>, Arc<Script>) {
        mock_link(supported)
    }

    fn exec(sql: &str) -> ExecArgs {
        ExecArgs {
            sql: sql.into(),
            params: vec![],
            timeout: None,
            fetch_size: 500,
        }
    }

    async fn run(link: &Arc<Link>, sql: &str) -> Result<ExecReply> {
        link.timed(None, |tx| Request::Execute(exec(sql), tx)).await
    }

    async fn fetch(link: &Arc<Link>, n: usize) -> Result<Page> {
        link.timed(None, |tx| {
            Request::Fetch(
                FetchArgs {
                    max_rows: n,
                    timeout: None,
                    max_value_bytes: 1 << 20,
                },
                tx,
            )
        })
        .await
    }

    fn ints(page: &Page) -> Vec<i64> {
        page.rows.iter().map(|r| r[0].as_i64().unwrap()).collect()
    }

    async fn wait_until(mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "timed out waiting"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn requests_run_in_order_and_answer() {
        let (link, script) = link(true);
        assert!(matches!(
            run(&link, "affected").await.unwrap(),
            ExecReply::Affected(3)
        ));
        link.call(Request::Ping).await.unwrap();
        link.call(Request::Commit).await.unwrap();
        assert_eq!(
            link.call(Request::ServerVersion).await.unwrap(),
            "Oracle Mock 1.0"
        );
        assert_eq!(script.entries(), ["execute affected", "ping", "commit"]);
    }

    #[tokio::test]
    async fn paging_continues_where_the_last_page_ended() {
        let (link, _) = link(true);
        let ExecReply::Rows(cols) = run(&link, "rows 1000").await.unwrap() else {
            panic!("rows expected")
        };
        assert_eq!(cols.len(), 1);
        let mut seen = Vec::new();
        for want in [300usize, 300, 300, 300] {
            let page = fetch(&link, want).await.unwrap();
            seen.extend(ints(&page));
            if page.exhausted {
                break;
            }
        }
        assert_eq!(seen, (0..1000).collect::<Vec<_>>());
        // After the end: empty and exhausted.
        let page = fetch(&link, 10).await.unwrap();
        assert!(page.rows.is_empty() && page.exhausted);
    }

    #[tokio::test]
    async fn query_errors_leave_the_connection_usable() {
        let (link, _) = link(true);
        let e = run(&link, "nonsense").await.err().unwrap();
        assert_eq!(e.kind, ErrorKind::Query);
        assert!(run(&link, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn cancel_interrupts_the_running_statement_and_the_connection_survives() {
        let (link, script) = link(true);
        let runner = {
            let link = link.clone();
            tokio::spawn(async move { run(&link, "block").await })
        };
        wait_until(|| script.entries().contains(&"execute block".to_string())).await;
        link.cancel().await.unwrap();
        let e = runner.await.unwrap().err().unwrap();
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
        assert!(run(&link, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn cancel_while_idle_is_a_no_op() {
        let (link, script) = link(true);
        link.cancel().await.unwrap();
        assert_eq!(script.breaks.load(Ordering::SeqCst), 0);
        assert!(
            run(&link, "affected").await.is_ok(),
            "the next statement is not cancelled"
        );
    }

    #[tokio::test]
    async fn timeout_cancels_and_reports_timeout() {
        // No client call timeouts: the watchdog is the timer.
        let (link, script) = link(false);
        let started = Instant::now();
        let e = link
            .timed(Some(Duration::from_millis(100)), |tx| {
                Request::Execute(exec("block"), tx)
            })
            .await
            .err()
            .unwrap();
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(script.breaks.load(Ordering::SeqCst), 1);
        // The statement ended with the break; the connection works.
        assert!(run(&link, "affected").await.is_ok());
    }

    #[tokio::test]
    async fn dropping_a_running_call_breaks_it_and_marks_the_connection_broken() {
        let (link, script) = link(true);
        let result = tokio::time::timeout(Duration::from_millis(100), run(&link, "block")).await;
        assert!(result.is_err(), "the future is dropped");
        // The running statement is interrupted and the session closed.
        wait_until(|| script.closed.load(Ordering::SeqCst)).await;
        assert!(script.breaks.load(Ordering::SeqCst) >= 1);
        let e = run(&link, "affected").await.err().unwrap();
        assert_eq!(e.kind, ErrorKind::ConnectionLost);
        assert_eq!(
            link.cancel().await.ok(),
            Some(()),
            "cancel on a broken connection is harmless"
        );
    }

    #[tokio::test]
    async fn a_lost_session_marks_the_connection_broken() {
        let (link, script) = link(true);
        let e = run(&link, "lost").await.err().unwrap();
        assert_eq!(e.kind, ErrorKind::ConnectionLost);
        wait_until(|| script.closed.load(Ordering::SeqCst)).await;
        assert_eq!(
            run(&link, "affected").await.err().unwrap().kind,
            ErrorKind::ConnectionLost
        );
    }

    #[tokio::test]
    async fn close_ends_the_session_and_later_calls_fail() {
        let (link, script) = link(true);
        link.call(Request::Close).await.unwrap();
        assert!(script.closed.load(Ordering::SeqCst));
        assert_eq!(
            run(&link, "affected").await.err().unwrap().kind,
            ErrorKind::ConnectionLost
        );
    }

    #[tokio::test]
    async fn dropping_the_link_closes_the_session() {
        let (link, script) = link(true);
        run(&link, "rows 10").await.unwrap();
        drop(link);
        wait_until(|| script.closed.load(Ordering::SeqCst)).await;
    }

    #[tokio::test]
    async fn released_cursor_is_closed_before_the_next_statement() {
        let (link, script) = link(true);
        run(&link, "rows 10").await.unwrap();
        link.release_cursor();
        run(&link, "affected").await.unwrap();
        let log = script.entries();
        let close = log.iter().position(|e| e == "close_cursor").unwrap();
        let next = log.iter().position(|e| e == "execute affected").unwrap();
        assert!(close < next, "{log:?}");
    }

    #[tokio::test]
    async fn an_unanswered_cursor_is_released() {
        // The caller of `execute` gave up (reply channel closed) after the
        // statement had opened a cursor: the session thread releases it.
        let (link, script) = link(true);
        let (reply, answer) = oneshot::channel();
        drop(answer);
        link.gate().enqueue();
        link.tx
            .send(Request::Execute(exec("rows 5"), reply))
            .unwrap();
        wait_until(|| script.entries().contains(&"close_cursor".to_string())).await;
    }

    #[tokio::test]
    async fn tx_mode_is_tracked() {
        let (link, _) = link(true);
        assert_eq!(link.tx_mode(), TxMode::Auto);
        link.call(|tx| Request::SetTxMode(TxMode::Manual, tx))
            .await
            .unwrap();
        link.note_tx_mode(TxMode::Manual);
        assert_eq!(link.tx_mode(), TxMode::Manual);
    }
}
