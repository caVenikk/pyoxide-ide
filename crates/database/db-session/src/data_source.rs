use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use db_core::schema::{Level, Scope, introspector};
use db_core::{
    Connection, ConnectionSource, DataSourceId, Error, ExecOptions, Outcome, Purpose, ResultSet,
};
use db_model::{Model, ModelHandle};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch};

use crate::dialect;
use crate::event::{
    DataSourceState, DecisionReason, EventKind, EventSink, JobId, RefreshEvent, SessionId,
};
use crate::job::syntax_dialect;
use crate::options::{DataSourceOptions, SessionOptions, TxPolicy};
use crate::runtime::DbRuntime;
use crate::session::{Cmd, Session, SessionInner, SessionState};

/// One configured data source (ADR 0021): its connection state, its console
/// sessions, up to [`DataSourceOptions::data_connections`] data connections
/// for table grids and export, and the metadata model that refreshes fill
/// on short connections of their own.
///
/// Every method returns at once; results arrive as events through the sink
/// given to [`DbRuntime::data_source`]. Cheap to clone.
#[derive(Clone)]
pub struct DataSourceHandle {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for DataSourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataSourceHandle")
            .field("id", &self.shared.id)
            .finish_non_exhaustive()
    }
}

pub(crate) struct Shared {
    pub(crate) id: DataSourceId,
    pub(crate) options: DataSourceOptions,
    source: Arc<dyn ConnectionSource>,
    sink: EventSink,
    pub(crate) runtime: DbRuntime,
    state: Mutex<DataSourceState>,
    /// Increased by every disconnect: a connection opened before it is
    /// closed instead of being used again.
    epoch: AtomicU64,
    sessions: Mutex<Vec<SessionEntry>>,
    single: Mutex<Weak<SessionInner>>,
    data: DataPool,
    model: Arc<ModelHandle>,
    /// Last user activity (statements, fetches, reads, refreshes); keep-alive
    /// pings do not count.
    last_used: Mutex<Instant>,
    /// The user was asked about an idle disconnect in this idle period.
    idle_asked: AtomicBool,
    jobs: Mutex<HashMap<JobId, watch::Sender<bool>>>,
}

struct SessionEntry {
    id: SessionId,
    inner: Weak<SessionInner>,
    state: Arc<SessionState>,
}

struct DataPool {
    idle: Mutex<Vec<IdleConnection>>,
    permits: Arc<Semaphore>,
}

struct IdleConnection {
    conn: Arc<dyn Connection>,
    epoch: u64,
    last_ping: Instant,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Waits until `rx` says cancelled; never returns when its sender is gone.
pub(crate) async fn cancelled(rx: &mut watch::Receiver<bool>) {
    if rx.wait_for(|v| *v).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Close a connection without waiting for it; a hung close is given up.
pub(crate) fn close_in_background(runtime: &DbRuntime, conn: Arc<dyn Connection>) {
    runtime.spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(10), conn.close()).await;
    });
}

impl DataSourceHandle {
    pub(crate) fn new(
        runtime: DbRuntime,
        id: DataSourceId,
        options: DataSourceOptions,
        source: Arc<dyn ConnectionSource>,
        sink: EventSink,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(options.data_connections.max(1)));
        let model = Arc::new(ModelHandle::new(Model::empty(id.clone())));
        let shared = Arc::new(Shared {
            id,
            options,
            source,
            sink,
            runtime,
            state: Mutex::new(DataSourceState::Disconnected),
            epoch: AtomicU64::new(0),
            sessions: Mutex::new(Vec::new()),
            single: Mutex::new(Weak::new()),
            data: DataPool {
                idle: Mutex::new(Vec::new()),
                permits,
            },
            model,
            last_used: Mutex::new(Instant::now()),
            idle_asked: AtomicBool::new(false),
            jobs: Mutex::new(HashMap::new()),
        });
        Shared::start_ticker(&shared);
        Self { shared }
    }

    pub fn id(&self) -> &DataSourceId {
        &self.shared.id
    }

    pub fn options(&self) -> &DataSourceOptions {
        &self.shared.options
    }

    pub fn state(&self) -> DataSourceState {
        lock(&self.shared.state).clone()
    }

    /// The metadata model of the data source: read it at any time without
    /// waiting; a refresh swaps it whole. Load the disk cache into it with
    /// [`ModelHandle::set`].
    pub fn model(&self) -> Arc<ModelHandle> {
        self.shared.model.clone()
    }

    /// Open a data connection now (and keep it for table grids), to show the
    /// data source as connected or to report why it cannot connect.
    pub fn connect(&self) {
        let shared = self.shared.clone();
        self.shared.runtime.spawn(async move {
            // The lease goes back to the pool when dropped.
            let _ = shared.lease_data().await;
        });
    }

    /// A console session. Its connection opens with the first command. With
    /// [`DataSourceOptions::single_session`] every call returns the same
    /// session while it is alive.
    pub fn open_session(&self, options: SessionOptions) -> Session {
        let shared = &self.shared;
        if shared.options.single_session {
            let mut single = lock(&shared.single);
            if let Some(inner) = single.upgrade() {
                return Session::from_inner(inner);
            }
            let session = Session::start(shared.clone(), options);
            *single = Arc::downgrade(&session.inner);
            return session;
        }
        Session::start(shared.clone(), options)
    }

    /// Close every connection. Sessions stay usable and reconnect with their
    /// next command. With [`TxPolicy::Ask`] and pending changes in a session,
    /// nothing is closed and [`EventKind::DecisionNeeded`] is sent. Running
    /// jobs are cancelled.
    pub fn disconnect(&self, policy: TxPolicy) {
        let pending = self.shared.pending_sessions();
        if policy == TxPolicy::Ask && !pending.is_empty() {
            self.shared.emit(EventKind::DecisionNeeded {
                reason: DecisionReason::Disconnect,
                sessions: pending,
            });
            return;
        }
        let shared = self.shared.clone();
        self.shared.runtime.spawn(async move {
            shared.disconnect_all(policy == TxPolicy::Commit).await;
        });
    }

    /// Load the metadata of `scope` on a short connection of its own (so it
    /// neither sees nor ends a transaction of a console) and merge it into
    /// the model: the schemas read replace their old versions, the others
    /// stay. Reported as [`EventKind::Refresh`]; cancel with
    /// [`DataSourceHandle::cancel_job`], which leaves the model unchanged.
    pub fn refresh(&self, scope: Scope, level: Level) -> JobId {
        let (job, rx) = self.shared.new_job();
        let shared = self.shared.clone();
        self.shared.runtime.spawn(async move {
            let inner = shared.clone();
            let run = tokio::spawn(async move { inner.run_refresh(job, scope, level, rx).await });
            if let Err(e) = run.await {
                shared.emit(EventKind::Refresh {
                    job,
                    event: RefreshEvent::Failed {
                        error: Error::internal(format!("the refresh stopped: {e}")),
                    },
                });
            }
            shared.end_job(job);
        });
        job
    }

    /// Run a query on a data connection (never in a console's transaction)
    /// and collect up to `max_rows` rows; reported as
    /// [`EventKind::DataRead`]. For `COUNT(*)` and small reads.
    pub fn read(&self, sql: impl Into<String>, max_rows: usize) -> JobId {
        let (job, rx) = self.shared.new_job();
        let shared = self.shared.clone();
        let sql = sql.into();
        self.shared.runtime.spawn(async move {
            let inner = shared.clone();
            let run = tokio::spawn(async move { inner.run_read(&sql, max_rows, rx).await });
            let result = run
                .await
                .unwrap_or_else(|e| Err(Error::internal(format!("the read stopped: {e}"))));
            shared.emit(EventKind::DataRead { job, result });
            shared.end_job(job);
        });
        job
    }

    /// Cancel a refresh or a read. Statement jobs are cancelled with
    /// [`Session::cancel`].
    pub fn cancel_job(&self, job: JobId) {
        if let Some(tx) = lock(&self.shared.jobs).get(&job) {
            let _ = tx.send(true);
        }
    }
}

impl Shared {
    pub(crate) fn emit(&self, kind: EventKind) {
        self.sink.emit(&self.id, kind);
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub(crate) fn exec_options(&self) -> ExecOptions {
        ExecOptions {
            timeout: self.options.statement_timeout,
            fetch_size: self.options.fetch_size.max(1),
            max_value_bytes: self.options.max_value_bytes,
        }
    }

    /// User activity: restarts the idle time.
    pub(crate) fn touch(&self) {
        *lock(&self.last_used) = Instant::now();
        self.idle_asked.store(false, Ordering::Release);
    }

    fn set_state(&self, state: DataSourceState) {
        let mut current = lock(&self.state);
        if *current != state {
            *current = state.clone();
            drop(current);
            self.emit(EventKind::State { state });
        }
    }

    pub(crate) fn register(&self, id: SessionId, inner: &Arc<SessionInner>) {
        lock(&self.sessions).push(SessionEntry {
            id,
            inner: Arc::downgrade(inner),
            state: inner.state.clone(),
        });
    }

    pub(crate) fn unregister(&self, id: SessionId) {
        lock(&self.sessions).retain(|e| e.id != id);
    }

    fn pending_sessions(&self) -> Vec<SessionId> {
        lock(&self.sessions)
            .iter()
            .filter(|e| e.state.status().tx.pending > 0)
            .map(|e| e.id)
            .collect()
    }

    fn new_job(&self) -> (JobId, watch::Receiver<bool>) {
        let job = JobId::next();
        let (tx, rx) = watch::channel(false);
        lock(&self.jobs).insert(job, tx);
        (job, rx)
    }

    fn end_job(&self, job: JobId) {
        lock(&self.jobs).remove(&job);
    }

    /// Open a connection for `purpose` and prepare it (time zone, startup
    /// script, read-only mode), moving the state machine along. Returns the
    /// epoch the connection belongs to.
    pub(crate) async fn open(
        self: &Arc<Self>,
        purpose: Purpose,
    ) -> Result<(Arc<dyn Connection>, u64), Error> {
        let epoch = self.epoch();
        {
            let mut state = lock(&self.state);
            if matches!(
                *state,
                DataSourceState::Disconnected | DataSourceState::Failed { .. }
            ) {
                *state = DataSourceState::Connecting;
                drop(state);
                self.emit(EventKind::State {
                    state: DataSourceState::Connecting,
                });
            }
        }
        // Puts a `Connecting` state back when the caller stops waiting.
        struct Attempt<'a>(&'a Shared, bool);
        impl Drop for Attempt<'_> {
            fn drop(&mut self) {
                if !self.1 && *lock(&self.0.state) == DataSourceState::Connecting {
                    self.0.set_state(DataSourceState::Disconnected);
                }
            }
        }
        let mut attempt = Attempt(self, false);
        let opened = self.source.open(&self.id, purpose).await;
        attempt.1 = true;
        match opened {
            Ok(conn) => {
                self.setup(&*conn, purpose).await;
                self.set_state(DataSourceState::Connected);
                Ok((conn, epoch))
            }
            Err(error) => {
                if *lock(&self.state) == DataSourceState::Connecting {
                    self.set_state(DataSourceState::Failed {
                        error: error.clone(),
                    });
                }
                Err(error)
            }
        }
    }

    /// Time zone, startup script and read-only mode of a new connection.
    async fn setup(&self, conn: &dyn Connection, purpose: Purpose) {
        let dialect = conn.dialect();
        let mut statements = Vec::new();
        if let Some(zone) = &self.options.time_zone
            && let Some(sql) = dialect::time_zone(dialect, zone)
        {
            statements.push(sql);
        }
        if let Some(script) = &self.options.startup_script {
            statements.extend(
                sql_syntax::split(script, syntax_dialect(dialect))
                    .into_iter()
                    .map(|s| s.text),
            );
        }
        if self.options.read_only
            && let Some(sql) = dialect::read_only(dialect)
        {
            statements.push(sql.to_owned());
        }
        let opts = self.exec_options();
        for sql in statements {
            let result = match conn.execute(&sql, &[], &opts).await {
                Ok(Outcome::Rows(cursor)) => cursor.close().await,
                Ok(Outcome::Affected(_)) => Ok(()),
                Err(e) => Err(e),
            };
            if let Err(error) = result {
                self.emit(EventKind::SetupFailed { purpose, error });
            }
        }
    }

    pub(crate) async fn lease_data(self: &Arc<Self>) -> Result<DataLease, Error> {
        let permit = self
            .data
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::internal("the data connection pool is closed"))?;
        let epoch = self.epoch();
        let reuse = {
            let mut idle = lock(&self.data.idle);
            let mut found = None;
            while let Some(c) = idle.pop() {
                if c.epoch == epoch {
                    found = Some(c);
                    break;
                }
                close_in_background(&self.runtime, c.conn);
            }
            found
        };
        let (conn, epoch) = match reuse {
            Some(c) => (c.conn, c.epoch),
            None => self.open(Purpose::Data).await?,
        };
        Ok(DataLease {
            conn: Some(conn),
            epoch,
            shared: self.clone(),
            broken: false,
            _permit: permit,
        })
    }

    async fn run_read(
        self: &Arc<Self>,
        sql: &str,
        max_rows: usize,
        mut rx: watch::Receiver<bool>,
    ) -> Result<ResultSet, Error> {
        self.touch();
        let mut lease = tokio::select! {
            lease = self.lease_data() => lease?,
            _ = cancelled(&mut rx) => return Err(Error::cancelled()),
        };
        let conn = lease.conn().clone();
        let opts = self.exec_options();
        let read = conn.query_with(sql, &[], max_rows, &opts);
        match crate::session::guarded(&conn, read, Some(&mut rx), self.options.cancel_grace).await {
            crate::session::Guarded::Done(r) => {
                if let Err(e) = &r
                    && e.kind == db_core::ErrorKind::ConnectionLost
                {
                    lease.broken();
                }
                r
            }
            crate::session::Guarded::Abandoned => {
                lease.broken();
                Err(Error::cancelled())
            }
        }
    }

    async fn run_refresh(
        self: &Arc<Self>,
        job: JobId,
        scope: Scope,
        level: Level,
        mut rx: watch::Receiver<bool>,
    ) {
        let emit = |event| self.emit(EventKind::Refresh { job, event });
        emit(RefreshEvent::Started);
        self.touch();
        let opened = tokio::select! {
            r = self.open(Purpose::Meta) => r,
            _ = cancelled(&mut rx) => {
                emit(RefreshEvent::Cancelled);
                return;
            }
        };
        let conn = match opened {
            Ok((conn, _)) => conn,
            Err(error) => {
                emit(RefreshEvent::Failed { error });
                return;
            }
        };
        let loaded = {
            let intro = introspector(&*conn);
            let load = intro.load(&scope, level);
            match crate::session::guarded(&conn, load, Some(&mut rx), self.options.cancel_grace)
                .await
            {
                crate::session::Guarded::Done(r) => Some(r),
                crate::session::Guarded::Abandoned => None,
            }
        };
        close_in_background(&self.runtime, conn);
        let snapshot = match loaded {
            _ if *rx.borrow() => {
                emit(RefreshEvent::Cancelled);
                return;
            }
            None => {
                emit(RefreshEvent::Cancelled);
                return;
            }
            Some(Err(error)) => {
                emit(RefreshEvent::Failed { error });
                return;
            }
            Some(Ok(snapshot)) => snapshot,
        };
        let model = self.model.clone();
        let flag = rx.clone();
        let built = tokio::task::spawn_blocking(move || {
            model.update(|m| {
                m.with_snapshot_cancellable(snapshot, &|| *flag.borrow())
                    .map(Some)
            })
        })
        .await;
        match built {
            Ok(Ok(model)) => emit(RefreshEvent::Finished {
                generation: self.model.generation(),
                objects: model.len(),
            }),
            Ok(Err(_)) => emit(RefreshEvent::Cancelled),
            Err(e) => emit(RefreshEvent::Failed {
                error: Error::internal(format!("building the model failed: {e}")),
            }),
        }
    }

    pub(crate) async fn disconnect_all(self: &Arc<Self>, commit: bool) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        for tx in lock(&self.jobs).values() {
            let _ = tx.send(true);
        }
        let sessions: Vec<Arc<SessionInner>> = lock(&self.sessions)
            .iter()
            .filter_map(|e| e.inner.upgrade())
            .collect();
        let mut acks = Vec::new();
        for inner in sessions {
            inner.state.cancel_all();
            let (done, ack) = oneshot::channel();
            if inner.send(Cmd::Disconnect { commit, done }) {
                acks.push(ack);
            }
        }
        for ack in acks {
            let _ = ack.await;
        }
        let idle: Vec<IdleConnection> = std::mem::take(&mut *lock(&self.data.idle));
        for c in idle {
            close_in_background(&self.runtime, c.conn);
        }
        self.idle_asked.store(false, Ordering::Release);
        self.set_state(DataSourceState::Disconnected);
    }

    fn start_ticker(shared: &Arc<Self>) {
        let opts = &shared.options;
        let Some(shortest) = [opts.keep_alive, opts.auto_disconnect]
            .into_iter()
            .flatten()
            .min()
        else {
            return;
        };
        let period = (shortest / 4).clamp(Duration::from_millis(10), Duration::from_secs(1));
        let weak = Arc::downgrade(shared);
        shared.runtime.spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(shared) = weak.upgrade() else { break };
                shared.tick().await;
            }
        });
    }

    async fn tick(self: &Arc<Self>) {
        if *lock(&self.state) != DataSourceState::Connected {
            return;
        }
        let now = Instant::now();
        if let Some(every) = self.options.keep_alive {
            self.keep_alive(now, every).await;
        }
        let Some(after) = self.options.auto_disconnect else {
            return;
        };
        let idle_for = now.saturating_duration_since(*lock(&self.last_used));
        let busy = !lock(&self.jobs).is_empty()
            || lock(&self.sessions)
                .iter()
                .any(|e| e.state.status().running.is_some());
        if idle_for < after || busy {
            return;
        }
        let pending = self.pending_sessions();
        if pending.is_empty() {
            self.disconnect_all(false).await;
        } else if !self.idle_asked.swap(true, Ordering::AcqRel) {
            self.emit(EventKind::DecisionNeeded {
                reason: DecisionReason::IdleDisconnect,
                sessions: pending,
            });
        }
    }

    async fn keep_alive(self: &Arc<Self>, now: Instant, every: Duration) {
        for e in lock(&self.sessions).iter() {
            if e.state.wants_ping(now, every)
                && let Some(inner) = e.inner.upgrade()
            {
                inner.send(Cmd::KeepAlive);
            }
        }
        let due: Vec<IdleConnection> = {
            let mut idle = lock(&self.data.idle);
            let (due, keep) = std::mem::take(&mut *idle)
                .into_iter()
                .partition(|c| now.saturating_duration_since(c.last_ping) >= every);
            *idle = keep;
            due
        };
        for mut c in due {
            let alive = tokio::time::timeout(self.options.cancel_grace, c.conn.ping()).await;
            if matches!(alive, Ok(Ok(()))) {
                c.last_ping = Instant::now();
                lock(&self.data.idle).push(c);
            } else {
                close_in_background(&self.runtime, c.conn);
            }
        }
    }
}

/// A data connection borrowed from the pool; it goes back on drop unless it
/// broke or the data source was disconnected meanwhile.
pub(crate) struct DataLease {
    conn: Option<Arc<dyn Connection>>,
    epoch: u64,
    shared: Arc<Shared>,
    broken: bool,
    _permit: OwnedSemaphorePermit,
}

impl DataLease {
    pub(crate) fn conn(&self) -> &Arc<dyn Connection> {
        self.conn.as_ref().expect("taken only on drop")
    }

    pub(crate) fn broken(&mut self) {
        self.broken = true;
    }
}

impl Drop for DataLease {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else { return };
        if self.broken || self.epoch != self.shared.epoch() {
            close_in_background(&self.shared.runtime, conn);
        } else {
            lock(&self.shared.data.idle).push(IdleConnection {
                conn,
                epoch: self.epoch,
                last_ping: Instant::now(),
            });
        }
    }
}
