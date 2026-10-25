use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use db_core::{
    Column, Connection, Cursor, Dialect, Error, ErrorKind, Outcome, Purpose, Row, TxMode,
};
use serde::Serialize;
use sql_syntax::StatementKind;
use tokio::sync::{mpsc, oneshot, watch};

use crate::data_source::{Shared, cancelled, close_in_background};
use crate::dialect;
use crate::event::{
    Command, DecisionReason, EventKind, JobEvent, JobId, JobOutcome, ResultId, SessionEvent,
    SessionId, StatementError, Timing, TxStatus,
};
use crate::job::{
    ErrorPolicy, Job, JobSource, JobStatement, TxEffect, may_change_schema, tx_effect,
};
use crate::options::{SessionOptions, TxPolicy};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A console session (ADR 0021): one connection with its own transaction
/// and current schema. Commands run in the order they are given; `cancel`
/// is out of band and stops the running job or fetch at once.
///
/// Every method returns at once; results arrive as events. Cheap to clone;
/// when the last clone is dropped the session closes its connection (an
/// open transaction is rolled back by the server).
#[derive(Clone)]
pub struct Session {
    pub(crate) inner: Arc<SessionInner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.inner.id)
            .finish_non_exhaustive()
    }
}

/// A snapshot of a session's state, for a toolbar drawn without waiting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SessionStatus {
    pub tx: TxStatus,
    /// The current schema, once known.
    pub schema: Option<String>,
    /// The session holds an open connection.
    pub connected: bool,
    /// The job running now.
    pub running: Option<JobId>,
    pub closed: bool,
}

pub(crate) struct SessionInner {
    pub(crate) id: SessionId,
    tx: mpsc::UnboundedSender<Cmd>,
    pub(crate) state: Arc<SessionState>,
    shared: Arc<Shared>,
}

/// What the session, its handles and the data source share.
pub(crate) struct SessionState {
    status: Mutex<SessionStatus>,
    /// Cancel switches of the jobs and fetches sent and not finished.
    cancels: Mutex<Vec<(JobId, watch::Sender<bool>)>>,
    last_ping: Mutex<Instant>,
}

pub(crate) enum Cmd {
    Execute {
        job: JobId,
        spec: Job,
        cancel: watch::Receiver<bool>,
    },
    Fetch {
        token: JobId,
        result: ResultId,
        max_rows: usize,
        cancel: watch::Receiver<bool>,
    },
    Commit,
    Rollback,
    SetSchema(String),
    SetTxMode(TxMode, TxPolicy),
    Close(TxPolicy),
    Disconnect {
        commit: bool,
        done: oneshot::Sender<()>,
    },
    KeepAlive,
}

impl SessionState {
    pub(crate) fn status(&self) -> SessionStatus {
        lock(&self.status).clone()
    }

    pub(crate) fn cancel_all(&self) {
        for (_, tx) in lock(&self.cancels).iter() {
            let _ = tx.send(true);
        }
    }

    fn add_cancel(&self, token: JobId) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        lock(&self.cancels).push((token, tx));
        rx
    }

    fn remove_cancel(&self, token: JobId) {
        lock(&self.cancels).retain(|(t, _)| *t != token);
    }

    /// Whether an idle connected session is due a keep-alive ping; marks the
    /// ping as sent.
    pub(crate) fn wants_ping(&self, now: Instant, every: Duration) -> bool {
        let status = lock(&self.status);
        if !status.connected || status.running.is_some() || status.closed {
            return false;
        }
        let mut last = lock(&self.last_ping);
        if now.saturating_duration_since(*last) >= every {
            *last = now;
            true
        } else {
            false
        }
    }

    fn update(&self, f: impl FnOnce(&mut SessionStatus)) {
        f(&mut lock(&self.status));
    }
}

impl SessionInner {
    /// `false` when the session has stopped.
    pub(crate) fn send(&self, cmd: Cmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

impl Session {
    pub(crate) fn start(shared: Arc<Shared>, options: SessionOptions) -> Self {
        let id = SessionId::next();
        let (tx, rx) = mpsc::unbounded_channel();
        let state = Arc::new(SessionState {
            status: Mutex::new(SessionStatus {
                tx: TxStatus {
                    mode: options.tx_mode.unwrap_or(shared.options.default_tx_mode),
                    ..TxStatus::default()
                },
                ..SessionStatus::default()
            }),
            cancels: Mutex::new(Vec::new()),
            last_ping: Mutex::new(Instant::now()),
        });
        let inner = Arc::new(SessionInner {
            id,
            tx,
            state: state.clone(),
            shared: shared.clone(),
        });
        shared.register(id, &inner);
        let actor = Actor {
            id,
            tx: state.status().tx,
            state,
            shared: shared.clone(),
            conn: None,
            cursor: None,
            schema_wanted: options.schema,
        };
        // A panic in the actor (a driver bug) closes the session visibly
        // instead of leaving it silent.
        let watch_state = inner.state.clone();
        let watch_shared = shared.clone();
        shared.runtime.spawn(async move {
            if tokio::spawn(actor.run(rx)).await.is_err() {
                watch_state.update(|s| {
                    s.closed = true;
                    s.connected = false;
                    s.running = None;
                });
                watch_shared.unregister(id);
                watch_shared.emit(EventKind::Session {
                    session: id,
                    event: SessionEvent::Closed,
                });
            }
        });
        Self { inner }
    }

    pub(crate) fn from_inner(inner: Arc<SessionInner>) -> Self {
        Self { inner }
    }

    pub fn id(&self) -> SessionId {
        self.inner.id
    }

    pub fn status(&self) -> SessionStatus {
        self.inner.state.status()
    }

    /// Run a job. Its events ([`EventKind::Job`]) carry the returned id.
    pub fn execute(&self, job: Job) -> JobId {
        let id = JobId::next();
        let cancel = self.inner.state.add_cancel(id);
        if !self.inner.send(Cmd::Execute {
            job: id,
            spec: job,
            cancel,
        }) {
            self.inner.state.remove_cancel(id);
            let shared = &self.inner.shared;
            let emit = |event| {
                shared.emit(EventKind::Job {
                    session: self.inner.id,
                    job: id,
                    event,
                })
            };
            emit(JobEvent::Failed {
                index: 0,
                error: StatementError {
                    error: Error::new(ErrorKind::Internal, "the session is closed"),
                    position: 0,
                },
                timing: Timing::default(),
            });
            emit(JobEvent::Done {
                outcome: JobOutcome::Failed,
            });
        }
        id
    }

    /// Stop the running job or fetch, and the jobs queued behind it: the
    /// server is asked to cancel the statement; a driver that cannot (thin
    /// Oracle) or a statement still running after the grace period makes
    /// the session drop its connection and open a new one with the next
    /// command ([`SessionEvent::TransactionLost`] when changes were pending).
    pub fn cancel(&self) {
        self.inner.state.cancel_all();
    }

    /// Read up to `max_rows` more rows of the open result (the page size when
    /// `None`); answered by [`SessionEvent::Page`].
    pub fn fetch(&self, result: ResultId, max_rows: Option<usize>) {
        let token = JobId::next();
        let cancel = self.inner.state.add_cancel(token);
        let max_rows = max_rows.unwrap_or(self.inner.shared.options.page_size);
        if !self.inner.send(Cmd::Fetch {
            token,
            result,
            max_rows,
            cancel,
        }) {
            self.inner.state.remove_cancel(token);
        }
    }

    pub fn commit(&self) {
        self.inner.send(Cmd::Commit);
    }

    pub fn rollback(&self) {
        self.inner.send(Cmd::Rollback);
    }

    /// Switch the current schema (PostgreSQL: `SET search_path`, Oracle:
    /// `ALTER SESSION SET CURRENT_SCHEMA`); kept for later reconnects.
    pub fn set_schema(&self, schema: impl Into<String>) {
        self.inner.send(Cmd::SetSchema(schema.into()));
    }

    /// Switch the transaction mode. Leaving manual mode with pending changes
    /// asks first under [`TxPolicy::Ask`].
    pub fn set_tx_mode(&self, mode: TxMode, policy: TxPolicy) {
        self.inner.send(Cmd::SetTxMode(mode, policy));
    }

    /// Close the session. With pending changes and [`TxPolicy::Ask`] the
    /// session stays open and [`EventKind::DecisionNeeded`] is sent.
    pub fn close(&self, policy: TxPolicy) {
        self.inner.send(Cmd::Close(policy));
    }
}

/// How a call guarded by a cancel switch ended.
pub(crate) enum Guarded<T> {
    /// It finished (possibly with the server's cancel error).
    Done(db_core::Result<T>),
    /// Cancelled, and the driver could not stop it within the grace period
    /// or cannot cancel at all: the call was dropped and the connection must
    /// not be used again.
    Abandoned,
}

/// Run `fut` on `conn`; when `cancel` switches on, ask the server to cancel
/// and give the call `grace` to end before it is dropped.
pub(crate) async fn guarded<T>(
    conn: &Arc<dyn Connection>,
    fut: impl Future<Output = db_core::Result<T>>,
    cancel: Option<&mut watch::Receiver<bool>>,
    grace: Duration,
) -> Guarded<T> {
    tokio::pin!(fut);
    let Some(cancel) = cancel else {
        return Guarded::Done(fut.await);
    };
    tokio::select! {
        biased;
        r = &mut fut => return Guarded::Done(r),
        _ = cancelled(cancel) => {}
    }
    let deadline = tokio::time::Instant::now() + grace;
    let call = conn.cancel();
    tokio::pin!(call);
    let mut asked = false;
    loop {
        tokio::select! {
            r = &mut fut => return Guarded::Done(r),
            c = &mut call, if !asked => {
                asked = true;
                if c.is_err() {
                    return Guarded::Abandoned;
                }
            }
            _ = tokio::time::sleep_until(deadline) => return Guarded::Abandoned,
        }
    }
}

struct Live {
    conn: Arc<dyn Connection>,
    epoch: u64,
    dialect: Dialect,
}

struct OpenCursor {
    result: ResultId,
    cursor: Box<dyn Cursor>,
    fetched: u64,
    /// The statement's savepoint is released when the cursor closes
    /// (PostgreSQL, SQLite; nothing can run while a cursor is open).
    release_pending: bool,
}

/// A statement failed or was cancelled.
struct Failure {
    error: Error,
    cancelled: bool,
    timing: Timing,
}

impl Failure {
    fn new(error: Error) -> Self {
        let cancelled = error.kind == ErrorKind::Cancelled;
        Self {
            error,
            cancelled,
            timing: Timing::default(),
        }
    }

    fn cancelled() -> Self {
        Self::new(Error::cancelled())
    }

    fn timed(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }
}

enum StatementDone {
    Rows {
        result: ResultId,
        columns: Vec<Column>,
        rows: Vec<Row>,
        has_more: bool,
        timing: Timing,
    },
    Affected {
        rows: u64,
        timing: Timing,
    },
}

struct Actor {
    id: SessionId,
    state: Arc<SessionState>,
    shared: Arc<Shared>,
    conn: Option<Live>,
    cursor: Option<OpenCursor>,
    tx: TxStatus,
    schema_wanted: Option<String>,
}

impl Actor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Cmd>) {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Cmd::Execute { job, spec, cancel } => {
                    self.shared.touch();
                    self.state.update(|s| s.running = Some(job));
                    self.execute(job, spec, cancel).await;
                    self.state.update(|s| s.running = None);
                    self.state.remove_cancel(job);
                    self.shared.touch();
                }
                Cmd::Fetch {
                    token,
                    result,
                    max_rows,
                    cancel,
                } => {
                    self.shared.touch();
                    self.fetch(result, max_rows, cancel).await;
                    self.state.remove_cancel(token);
                }
                Cmd::Commit => {
                    self.shared.touch();
                    let r = self.end_transaction(true).await;
                    self.report(Command::Commit, r);
                }
                Cmd::Rollback => {
                    self.shared.touch();
                    let r = self.end_transaction(false).await;
                    self.report(Command::Rollback, r);
                }
                Cmd::SetSchema(schema) => {
                    self.shared.touch();
                    let r = self.set_schema(schema).await;
                    self.report(Command::SetSchema, r);
                }
                Cmd::SetTxMode(mode, policy) => {
                    self.shared.touch();
                    if let Some(r) = self.set_tx_mode(mode, policy).await {
                        self.report(Command::SetTxMode, r);
                    }
                }
                Cmd::Close(policy) => {
                    if self.tx.pending > 0 && policy == TxPolicy::Ask {
                        self.shared.emit(EventKind::DecisionNeeded {
                            reason: DecisionReason::Close,
                            sessions: vec![self.id],
                        });
                        continue;
                    }
                    let r = self.release_connection(policy == TxPolicy::Commit).await;
                    if let Err(error) = r {
                        self.session_event(SessionEvent::CommandFailed {
                            command: Command::Close,
                            error,
                        });
                    }
                    break;
                }
                Cmd::Disconnect { commit, done } => {
                    let _ = self.release_connection(commit).await;
                    let _ = done.send(());
                }
                Cmd::KeepAlive => self.keep_alive().await,
            }
        }
        // Closed, or every handle was dropped.
        if let Some(live) = self.conn.take() {
            self.cursor = None;
            close_in_background(&self.shared.runtime, live.conn);
        }
        self.state.update(|s| {
            s.closed = true;
            s.connected = false;
            s.running = None;
        });
        self.shared.unregister(self.id);
        self.session_event(SessionEvent::Closed);
    }

    fn session_event(&self, event: SessionEvent) {
        self.shared.emit(EventKind::Session {
            session: self.id,
            event,
        });
    }

    fn job_event(&self, job: JobId, event: JobEvent) {
        self.shared.emit(EventKind::Job {
            session: self.id,
            job,
            event,
        });
    }

    fn report(&self, command: Command, r: Result<(), Error>) {
        self.session_event(match r {
            Ok(()) => SessionEvent::CommandDone { command },
            Err(error) => SessionEvent::CommandFailed { command, error },
        });
    }

    fn grace(&self) -> Duration {
        self.shared.options.cancel_grace
    }

    fn publish_tx(&mut self, before: TxStatus) {
        if self.tx != before {
            let status = self.tx;
            self.state.update(|s| s.tx = status);
            self.session_event(SessionEvent::Transaction { status });
        }
    }

    /// The connection, opened (and prepared) when there is none or the data
    /// source was disconnected since it was opened.
    async fn ensure_conn(
        &mut self,
        cancel: Option<&mut watch::Receiver<bool>>,
    ) -> Result<(Arc<dyn Connection>, Dialect), Failure> {
        if let Some(live) = &self.conn {
            if live.epoch == self.shared.epoch() {
                return Ok((live.conn.clone(), live.dialect));
            }
            self.drop_connection();
        }
        let shared = self.shared.clone();
        let open = shared.open(Purpose::Console);
        let opened = match cancel {
            Some(cancel) => tokio::select! {
                r = open => r,
                _ = cancelled(cancel) => Err(Error::cancelled()),
            },
            None => open.await,
        };
        let (conn, epoch) = opened.map_err(Failure::new)?;
        if let Err(e) = conn.set_tx_mode(self.tx.mode).await {
            close_in_background(&self.shared.runtime, conn);
            return Err(Failure::new(e));
        }
        let dialect = conn.dialect();
        self.conn = Some(Live {
            conn: conn.clone(),
            epoch,
            dialect,
        });
        *lock(&self.state.last_ping) = Instant::now();
        self.state.update(|s| s.connected = true);
        let before = self.tx;
        self.tx.open = false;
        self.tx.pending = 0;
        self.session_event(SessionEvent::Connected);
        self.publish_tx(before);
        if let Some(schema) = self.schema_wanted.clone()
            && let Some(sql) = dialect::set_schema(dialect, &schema)
            && let Err(f) = self.exec_internal(&sql, None).await
        {
            self.session_event(SessionEvent::CommandFailed {
                command: Command::SetSchema,
                error: f.error,
            });
        }
        self.read_schema().await;
        Ok((conn, dialect))
    }

    /// Forget the connection (closed in the background) and the transaction
    /// on it.
    fn drop_connection(&mut self) {
        self.cursor = None;
        if let Some(live) = self.conn.take() {
            close_in_background(&self.shared.runtime, live.conn);
        }
        self.state.update(|s| s.connected = false);
        let before = self.tx;
        if self.tx.pending > 0 {
            self.session_event(SessionEvent::TransactionLost {
                pending: self.tx.pending,
            });
        }
        self.tx.open = false;
        self.tx.pending = 0;
        self.publish_tx(before);
    }

    fn after_error(&mut self, e: &Error) {
        if e.kind == ErrorKind::ConnectionLost {
            self.drop_connection();
        }
    }

    /// A statement of the session itself (savepoints, schema switch).
    async fn exec_internal(
        &mut self,
        sql: &str,
        cancel: Option<&mut watch::Receiver<bool>>,
    ) -> Result<(), Failure> {
        let Some(live) = &self.conn else {
            return Err(Failure::new(Error::connection_lost(
                "the session has no connection",
            )));
        };
        let conn = live.conn.clone();
        let opts = self.shared.exec_options();
        match guarded(&conn, conn.execute(sql, &[], &opts), cancel, self.grace()).await {
            Guarded::Done(Ok(Outcome::Rows(cursor))) => {
                let _ = cursor.close().await;
                Ok(())
            }
            Guarded::Done(Ok(Outcome::Affected(_))) => Ok(()),
            Guarded::Done(Err(e)) => {
                self.after_error(&e);
                Err(Failure::new(e))
            }
            Guarded::Abandoned => {
                self.drop_connection();
                Err(Failure::cancelled())
            }
        }
    }

    async fn read_schema(&mut self) {
        let Some(live) = &self.conn else { return };
        let conn = live.conn.clone();
        let sql = dialect::current_schema(live.dialect);
        let Ok(Ok(rs)) = tokio::time::timeout(self.grace(), conn.query(sql, &[], 1)).await else {
            return;
        };
        let Some(schema) = rs
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(|v| v.as_str().map(str::to_owned))
        else {
            return;
        };
        let changed = self.state.status().schema.as_deref() != Some(schema.as_str());
        if changed {
            self.state.update(|s| s.schema = Some(schema.clone()));
            self.session_event(SessionEvent::Schema { schema });
        }
    }

    /// Close the open cursor, if any, and release the savepoint of its
    /// statement.
    async fn close_cursor(&mut self) {
        let Some(open) = self.cursor.take() else {
            return;
        };
        let closed = tokio::time::timeout(self.grace(), open.cursor.close()).await;
        self.session_event(SessionEvent::ResultClosed {
            result: open.result,
        });
        match closed {
            Ok(Ok(())) => {}
            Ok(Err(e)) => self.after_error(&e),
            Err(_) => {
                self.drop_connection();
                return;
            }
        }
        if open.release_pending {
            self.release_savepoint().await;
        }
    }

    async fn release_savepoint(&mut self) {
        let Some(live) = &self.conn else { return };
        if let Some(sql) = dialect::release_savepoint(live.dialect) {
            let _ = self.exec_internal(&sql, None).await;
        }
    }

    async fn rollback_to_savepoint(&mut self) {
        if self.conn.is_none() {
            return;
        }
        if self
            .exec_internal(&dialect::rollback_to_savepoint(), None)
            .await
            .is_ok()
        {
            self.release_savepoint().await;
        }
    }

    async fn execute(&mut self, job: JobId, spec: Job, mut cancel: watch::Receiver<bool>) {
        let policy = spec.on_error;
        let dialect = match self.ensure_conn(Some(&mut cancel)).await {
            Ok((_, dialect)) => dialect,
            Err(f) => {
                let (text, range) = match &spec.source {
                    JobSource::Script(s) => (s.clone(), 0..s.len()),
                    JobSource::Statements(v) => v
                        .first()
                        .map(|s| (s.text.clone(), s.range.clone()))
                        .unwrap_or_default(),
                };
                let position = range.start;
                self.job_event(
                    job,
                    JobEvent::Started {
                        index: 0,
                        range,
                        text,
                    },
                );
                let outcome = if f.cancelled {
                    JobOutcome::Cancelled
                } else {
                    JobOutcome::Failed
                };
                self.job_event(
                    job,
                    JobEvent::Failed {
                        index: 0,
                        error: StatementError {
                            error: f.error,
                            position,
                        },
                        timing: f.timing,
                    },
                );
                self.job_event(job, JobEvent::Done { outcome });
                return;
            }
        };
        let statements = spec.resolve(dialect);
        let mut outcome = JobOutcome::Completed;
        let mut schema_may_change = false;
        for (index, st) in statements.iter().enumerate() {
            if *cancel.borrow() {
                outcome = JobOutcome::Cancelled;
                break;
            }
            self.close_cursor().await;
            self.job_event(
                job,
                JobEvent::Started {
                    index,
                    range: st.range.clone(),
                    text: st.text.clone(),
                },
            );
            schema_may_change |= may_change_schema(st, dialect);
            match self.run_statement(st, &mut cancel).await {
                Ok(StatementDone::Affected { rows, timing }) => {
                    self.job_event(job, JobEvent::Affected { index, rows });
                    self.job_event(job, JobEvent::Finished { index, timing });
                }
                Ok(StatementDone::Rows {
                    result,
                    columns,
                    rows,
                    has_more,
                    timing,
                }) => {
                    self.job_event(
                        job,
                        JobEvent::ResultOpened {
                            index,
                            result,
                            columns,
                            rows,
                            has_more,
                        },
                    );
                    self.job_event(job, JobEvent::Finished { index, timing });
                }
                Err(f) => {
                    let position = error_position(st, &f.error);
                    let stop_cancelled = f.cancelled || *cancel.borrow();
                    let lost = f.error.kind == ErrorKind::ConnectionLost;
                    self.job_event(
                        job,
                        JobEvent::Failed {
                            index,
                            error: StatementError {
                                error: f.error,
                                position,
                            },
                            timing: f.timing,
                        },
                    );
                    if stop_cancelled {
                        outcome = JobOutcome::Cancelled;
                        break;
                    }
                    if lost || policy == ErrorPolicy::Stop {
                        outcome = JobOutcome::Failed;
                        break;
                    }
                }
            }
        }
        if schema_may_change {
            self.read_schema().await;
        }
        self.job_event(job, JobEvent::Done { outcome });
    }

    async fn run_statement(
        &mut self,
        st: &JobStatement,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<StatementDone, Failure> {
        let (conn, dialect) = self.ensure_conn(Some(cancel)).await?;
        let effect = tx_effect(st, dialect);
        // Manual mode wraps every statement but the user's own transaction
        // control in a savepoint, so a failing one leaves the transaction
        // usable (ADR 0021).
        let wrap = self.tx.mode == TxMode::Manual && st.kind != StatementKind::Tcl;
        if wrap {
            if dialect::explicit_begin(dialect) && !self.tx.open {
                self.exec_internal("BEGIN", Some(cancel)).await?;
                let before = self.tx;
                self.tx.open = true;
                self.publish_tx(before);
            }
            self.exec_internal(&dialect::savepoint(), Some(cancel))
                .await?;
        }
        let opts = self.shared.exec_options();
        let started = Instant::now();
        let executed = guarded(
            &conn,
            conn.execute(&st.text, &st.params, &opts),
            Some(&mut *cancel),
            self.grace(),
        )
        .await;
        let execute = started.elapsed();
        let timing = Timing {
            execute,
            fetch: Duration::ZERO,
        };
        let outcome = match executed {
            Guarded::Abandoned => {
                self.drop_connection();
                return Err(Failure::cancelled().timed(timing));
            }
            Guarded::Done(Err(e)) => {
                self.statement_failed(wrap, &e).await;
                return Err(Failure::new(e).timed(timing));
            }
            Guarded::Done(Ok(outcome)) => outcome,
        };
        let mut cursor = match outcome {
            Outcome::Affected(rows) => {
                if wrap {
                    self.release_savepoint().await;
                }
                self.note_success(st, effect, wrap, dialect);
                return Ok(StatementDone::Affected { rows, timing });
            }
            Outcome::Rows(cursor) => cursor,
        };
        let columns = cursor.columns().to_vec();
        let started = Instant::now();
        let page_size = self.shared.options.page_size.max(1);
        let page = guarded(
            &conn,
            cursor.next_page(page_size),
            Some(cancel),
            self.grace(),
        )
        .await;
        let timing = Timing {
            execute,
            fetch: started.elapsed(),
        };
        let rows = match page {
            Guarded::Abandoned => {
                drop(cursor);
                self.drop_connection();
                return Err(Failure::cancelled().timed(timing));
            }
            Guarded::Done(Err(e)) => {
                drop(cursor);
                self.statement_failed(wrap, &e).await;
                return Err(Failure::new(e).timed(timing));
            }
            Guarded::Done(Ok(rows)) => rows,
        };
        self.note_success(st, effect, wrap, dialect);
        let result = ResultId::next();
        let has_more = !cursor.is_exhausted();
        let release = wrap && dialect::release_savepoint(dialect).is_some();
        if has_more {
            self.cursor = Some(OpenCursor {
                result,
                cursor,
                fetched: rows.len() as u64,
                release_pending: release,
            });
        } else {
            match tokio::time::timeout(self.grace(), cursor.close()).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => self.after_error(&e),
                Err(_) => self.drop_connection(),
            }
            if release {
                self.release_savepoint().await;
            }
        }
        Ok(StatementDone::Rows {
            result,
            columns,
            rows,
            has_more,
            timing,
        })
    }

    async fn statement_failed(&mut self, wrapped: bool, e: &Error) {
        self.after_error(e);
        if wrapped {
            self.rollback_to_savepoint().await;
        }
    }

    fn note_success(&mut self, st: &JobStatement, effect: TxEffect, wrapped: bool, d: Dialect) {
        let before = self.tx;
        match effect {
            TxEffect::Begins => self.tx.open = true,
            TxEffect::Ends => {
                self.tx.open = false;
                self.tx.pending = 0;
            }
            TxEffect::Keeps => {}
        }
        if wrapped {
            self.tx.open = true;
        }
        let ddl = st.kind == StatementKind::Ddl;
        if self.tx.open && (st.writes || (ddl && !dialect::ddl_commits(d))) {
            self.tx.pending += 1;
        }
        if ddl && dialect::ddl_commits(d) {
            self.tx.open = false;
            self.tx.pending = 0;
        }
        self.publish_tx(before);
    }

    async fn fetch(
        &mut self,
        result: ResultId,
        max_rows: usize,
        mut cancel: watch::Receiver<bool>,
    ) {
        let Some(mut open) = self.cursor.take_if(|c| c.result == result) else {
            self.session_event(SessionEvent::CommandFailed {
                command: Command::Fetch,
                error: Error::new(
                    ErrorKind::Unsupported,
                    "the result is closed; run the statement again",
                ),
            });
            return;
        };
        let Some(live) = &self.conn else {
            return;
        };
        let conn = live.conn.clone();
        let page = guarded(
            &conn,
            open.cursor.next_page(max_rows.max(1)),
            Some(&mut cancel),
            self.grace(),
        )
        .await;
        match page {
            Guarded::Done(Ok(rows)) => {
                let first_row = open.fetched;
                open.fetched += rows.len() as u64;
                let has_more = !open.cursor.is_exhausted();
                self.session_event(SessionEvent::Page {
                    result,
                    first_row,
                    rows,
                    has_more,
                });
                self.cursor = Some(open);
                if !has_more {
                    self.close_cursor().await;
                }
            }
            Guarded::Done(Err(error)) => {
                let release = open.release_pending;
                drop(open);
                self.session_event(SessionEvent::ResultClosed { result });
                self.after_error(&error);
                if release {
                    self.rollback_to_savepoint().await;
                }
                self.session_event(SessionEvent::CommandFailed {
                    command: Command::Fetch,
                    error,
                });
            }
            Guarded::Abandoned => {
                drop(open);
                self.session_event(SessionEvent::ResultClosed { result });
                self.drop_connection();
                self.session_event(SessionEvent::CommandFailed {
                    command: Command::Fetch,
                    error: Error::cancelled(),
                });
            }
        }
    }

    async fn end_transaction(&mut self, commit: bool) -> Result<(), Error> {
        self.close_cursor().await;
        let Some(live) = &self.conn else {
            return Ok(());
        };
        let conn = live.conn.clone();
        let ended = if commit {
            conn.commit().await
        } else {
            conn.rollback().await
        };
        match ended {
            Ok(()) => {
                let before = self.tx;
                self.tx.open = false;
                self.tx.pending = 0;
                self.publish_tx(before);
                Ok(())
            }
            Err(e) => {
                self.after_error(&e);
                Err(e)
            }
        }
    }

    async fn set_schema(&mut self, schema: String) -> Result<(), Error> {
        self.schema_wanted = Some(schema.clone());
        let Some(live) = &self.conn else {
            return Ok(());
        };
        let Some(sql) = dialect::set_schema(live.dialect, &schema) else {
            return Err(Error::unsupported("this database has no schemas to switch"));
        };
        self.close_cursor().await;
        self.exec_internal(&sql, None).await.map_err(|f| f.error)?;
        self.read_schema().await;
        Ok(())
    }

    /// `None` when the switch waits for a decision.
    async fn set_tx_mode(&mut self, mode: TxMode, policy: TxPolicy) -> Option<Result<(), Error>> {
        if mode == self.tx.mode {
            return Some(Ok(()));
        }
        if mode == TxMode::Auto && self.tx.pending > 0 {
            match policy {
                TxPolicy::Ask => {
                    self.shared.emit(EventKind::DecisionNeeded {
                        reason: DecisionReason::SwitchToAuto,
                        sessions: vec![self.id],
                    });
                    return None;
                }
                TxPolicy::Commit => {
                    if let Err(e) = self.end_transaction(true).await {
                        return Some(Err(e));
                    }
                }
                TxPolicy::Rollback => {
                    if let Err(e) = self.end_transaction(false).await {
                        return Some(Err(e));
                    }
                }
            }
        }
        if self.conn.is_some() {
            self.close_cursor().await;
            // A transaction without changes (queries in manual mode) ends
            // here, not with a commit by the driver.
            if mode == TxMode::Auto
                && self.tx.open
                && let Err(e) = self.end_transaction(false).await
            {
                return Some(Err(e));
            }
        }
        if let Some(live) = &self.conn {
            let conn = live.conn.clone();
            if let Err(e) = conn.set_tx_mode(mode).await {
                self.after_error(&e);
                return Some(Err(e));
            }
        }
        let before = self.tx;
        self.tx.mode = mode;
        self.publish_tx(before);
        Some(Ok(()))
    }

    /// End the transaction (commit or roll back) and close the connection.
    async fn release_connection(&mut self, commit: bool) -> Result<(), Error> {
        self.close_cursor().await;
        let mut result = Ok(());
        if self.conn.is_some() && self.tx.open {
            result = self.end_transaction(commit && self.tx.pending > 0).await;
        }
        if let Some(live) = self.conn.take() {
            let _ = tokio::time::timeout(self.grace(), live.conn.close()).await;
        }
        self.state.update(|s| s.connected = false);
        let before = self.tx;
        self.tx.open = false;
        self.tx.pending = 0;
        self.publish_tx(before);
        result
    }

    async fn keep_alive(&mut self) {
        if self.cursor.is_some() {
            return;
        }
        let Some(live) = &self.conn else { return };
        let conn = live.conn.clone();
        match tokio::time::timeout(self.grace(), conn.ping()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) if e.kind != ErrorKind::ConnectionLost => {}
            _ => self.drop_connection(),
        }
    }
}

/// Byte offset in the script where the error of `st` is.
fn error_position(st: &JobStatement, _error: &Error) -> usize {
    st.range.start
}
