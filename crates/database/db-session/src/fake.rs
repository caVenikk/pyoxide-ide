//! A scripted [`ConnectionSource`] for tests (feature `test-support`):
//! statements react by the first rule whose pattern they contain, every call
//! is logged, and connections can hang, ignore cancels or break.
//!
//! The fake keeps PostgreSQL's transaction rule that matters to the session
//! layer: after a failed statement inside a transaction every statement but
//! `ROLLBACK [TO SAVEPOINT]` and `COMMIT` fails until the transaction is
//! repaired or ended (Oracle and SQLite backends do not have the rule).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use db_core::{
    Backend, Column, Connection, ConnectionSource, DataSourceId, Error, ErrorKind, ExecOptions,
    Outcome, Purpose, Result, Row, TxMode, Value, ValueKind, VecCursor,
};
use tokio::sync::Notify;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a matching statement behaves.
#[derive(Debug, Clone)]
pub enum Reaction {
    /// Rows of text columns with these names.
    Rows {
        columns: Vec<String>,
        rows: Vec<Row>,
    },
    Affected(u64),
    Fail(Error),
    /// Runs until cancelled (then fails with a cancel error), or forever when
    /// the source does not support cancel.
    Hang,
    /// Runs forever; a cancel request is accepted but has no effect.
    HangIgnoringCancel,
    /// Waits, then reacts.
    Delay(Duration, Box<Reaction>),
    /// The connection breaks: this and every later call fail with
    /// `ConnectionLost`.
    LoseConnection,
}

impl Reaction {
    /// One text column with one row per value.
    pub fn column(name: &str, values: &[&str]) -> Self {
        Reaction::Rows {
            columns: vec![name.to_owned()],
            rows: values.iter().map(|v| vec![Value::from(*v)]).collect(),
        }
    }

    pub fn query_error(code: &str, message: &str) -> Self {
        Reaction::Fail(Error::query(message).with_code(code))
    }
}

/// How opening a connection behaves.
#[derive(Debug, Clone)]
pub enum OpenBehavior {
    Connect,
    Fail(Error),
    Hang,
}

/// One logged call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Number of the connection, in the order they were opened (from 1).
    pub connection: usize,
    pub purpose: Purpose,
    /// The statement, or `<commit>`, `<rollback>`, `<ping>`, `<cancel>`,
    /// `<close>`, `<tx manual>`, `<tx auto>`.
    pub sql: String,
}

/// The scripted source. Cheap to clone; clones share rules and logs.
#[derive(Clone)]
pub struct FakeSource {
    inner: Arc<FakeInner>,
}

struct FakeInner {
    backend: Backend,
    rules: Mutex<Vec<(String, Reaction)>>,
    open: Mutex<OpenBehavior>,
    cancel_supported: AtomicBool,
    log: Mutex<Vec<LogEntry>>,
    opened: Mutex<Vec<Purpose>>,
    open_now: AtomicUsize,
}

impl FakeSource {
    pub fn new(backend: Backend) -> Self {
        Self {
            inner: Arc::new(FakeInner {
                backend,
                rules: Mutex::new(Vec::new()),
                open: Mutex::new(OpenBehavior::Connect),
                cancel_supported: AtomicBool::new(true),
                log: Mutex::new(Vec::new()),
                opened: Mutex::new(Vec::new()),
                open_now: AtomicUsize::new(0),
            }),
        }
    }

    /// Statements containing `pattern` (case-insensitive) react so. Rules
    /// added earlier win. Without a match a `SELECT` returns one row with
    /// `1`, anything else affects 0 rows.
    pub fn on(&self, pattern: &str, reaction: Reaction) -> &Self {
        lock(&self.inner.rules).push((pattern.to_ascii_lowercase(), reaction));
        self
    }

    pub fn set_open(&self, behavior: OpenBehavior) {
        *lock(&self.inner.open) = behavior;
    }

    /// `false`: `cancel` fails with `Unsupported`, like the thin Oracle
    /// driver.
    pub fn set_cancel_supported(&self, supported: bool) {
        self.inner
            .cancel_supported
            .store(supported, Ordering::SeqCst);
    }

    pub fn log(&self) -> Vec<LogEntry> {
        lock(&self.inner.log).clone()
    }

    /// The logged statements and calls of connections opened for `purpose`.
    pub fn statements(&self, purpose: Purpose) -> Vec<String> {
        self.log()
            .into_iter()
            .filter(|e| e.purpose == purpose)
            .map(|e| e.sql)
            .collect()
    }

    /// How many connections were opened for `purpose`.
    pub fn opened(&self, purpose: Purpose) -> usize {
        lock(&self.inner.opened)
            .iter()
            .filter(|p| **p == purpose)
            .count()
    }

    /// Connections opened and not closed yet.
    pub fn open_now(&self) -> usize {
        self.inner.open_now.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ConnectionSource for FakeSource {
    async fn open(
        &self,
        _data_source: &DataSourceId,
        purpose: Purpose,
    ) -> Result<Arc<dyn Connection>> {
        let behavior = lock(&self.inner.open).clone();
        match behavior {
            OpenBehavior::Connect => {}
            OpenBehavior::Fail(e) => return Err(e),
            OpenBehavior::Hang => std::future::pending::<()>().await,
        }
        let number = {
            let mut opened = lock(&self.inner.opened);
            opened.push(purpose);
            opened.len()
        };
        self.inner.open_now.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(FakeConnection {
            source: self.inner.clone(),
            number,
            purpose,
            state: Mutex::new(ConnState::default()),
            cancel: Notify::new(),
            broken: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }))
    }
}

#[derive(Default)]
struct ConnState {
    mode: TxMode,
    in_tx: bool,
    aborted: bool,
}

struct FakeConnection {
    source: Arc<FakeInner>,
    number: usize,
    purpose: Purpose,
    state: Mutex<ConnState>,
    cancel: Notify,
    broken: AtomicBool,
    closed: AtomicBool,
}

impl FakeConnection {
    fn log(&self, sql: &str) {
        lock(&self.source.log).push(LogEntry {
            connection: self.number,
            purpose: self.purpose,
            sql: sql.to_owned(),
        });
    }

    fn check(&self) -> Result<()> {
        if self.broken.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            Err(Error::connection_lost("the fake connection is gone"))
        } else {
            Ok(())
        }
    }

    fn reaction(&self, sql: &str) -> Reaction {
        let lower = sql.to_ascii_lowercase();
        let rules = lock(&self.source.rules);
        if let Some((_, r)) = rules.iter().find(|(p, _)| lower.contains(p.as_str())) {
            return r.clone();
        }
        if lower.trim_start().starts_with("select") {
            Reaction::Rows {
                columns: vec!["?column?".into()],
                rows: vec![vec![Value::Int(1)]],
            }
        } else {
            Reaction::Affected(0)
        }
    }

    /// Transaction bookkeeping before a statement runs; an error when the
    /// transaction is aborted.
    fn before(&self, sql: &str) -> Result<()> {
        let lower = sql.trim().to_ascii_lowercase();
        let mut st = lock(&self.state);
        let repairs = lower.starts_with("rollback") || lower.starts_with("commit");
        if st.aborted && !repairs {
            return Err(Error::query(
                "current transaction is aborted, commands ignored until end of transaction block",
            )
            .with_code("25P02"));
        }
        if lower.starts_with("begin") {
            st.in_tx = true;
        } else if lower.starts_with("rollback to") {
            st.aborted = false;
        } else if lower.starts_with("commit") || lower.starts_with("rollback") {
            st.in_tx = false;
            st.aborted = false;
        } else if st.mode == TxMode::Manual
            && self.source.backend.dialect() != db_core::Dialect::Postgres
        {
            st.in_tx = true;
        }
        Ok(())
    }

    fn failed(&self) {
        let mut st = lock(&self.state);
        if st.in_tx && self.source.backend == Backend::Postgres {
            st.aborted = true;
        }
    }

    fn react<'a>(
        &'a self,
        reaction: Reaction,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Outcome>> + Send + 'a>> {
        Box::pin(async move {
            match reaction {
                Reaction::Rows { columns, rows } => {
                    let columns = columns
                        .into_iter()
                        .map(|name| Column {
                            name,
                            type_name: "text".into(),
                            kind: ValueKind::Text,
                            nullable: true,
                        })
                        .collect();
                    Ok(Outcome::Rows(Box::new(VecCursor::new(columns, rows))))
                }
                Reaction::Affected(n) => Ok(Outcome::Affected(n)),
                Reaction::Fail(e) => Err(e),
                Reaction::Hang => {
                    self.cancel.notified().await;
                    Err(Error::cancelled().with_code("57014"))
                }
                Reaction::HangIgnoringCancel => std::future::pending().await,
                Reaction::Delay(d, then) => {
                    tokio::time::sleep(d).await;
                    self.react(*then).await
                }
                Reaction::LoseConnection => {
                    self.broken.store(true, Ordering::SeqCst);
                    Err(Error::connection_lost("server closed the connection"))
                }
            }
        })
    }
}

#[async_trait]
impl Connection for FakeConnection {
    fn backend(&self) -> Backend {
        self.source.backend
    }

    async fn execute(&self, sql: &str, _params: &[Value], _opts: &ExecOptions) -> Result<Outcome> {
        self.log(sql);
        self.check()?;
        self.before(sql)?;
        let r = self.react(self.reaction(sql)).await;
        if let Err(e) = &r
            && e.kind != ErrorKind::ConnectionLost
        {
            self.failed();
        }
        r
    }

    async fn cancel(&self) -> Result<()> {
        self.log("<cancel>");
        if !self.source.cancel_supported.load(Ordering::SeqCst) {
            return Err(Error::unsupported("the fake driver cannot cancel"));
        }
        self.cancel.notify_waiters();
        Ok(())
    }

    async fn server_version(&self) -> Result<String> {
        Ok("fake 1.0".into())
    }

    async fn ping(&self) -> Result<()> {
        self.log("<ping>");
        self.check()
    }

    async fn tx_mode(&self) -> TxMode {
        lock(&self.state).mode
    }

    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.log(match mode {
            TxMode::Auto => "<tx auto>",
            TxMode::Manual => "<tx manual>",
        });
        self.check()?;
        lock(&self.state).mode = mode;
        Ok(())
    }

    async fn commit(&self) -> Result<()> {
        self.log("<commit>");
        self.check()?;
        let mut st = lock(&self.state);
        st.in_tx = false;
        st.aborted = false;
        Ok(())
    }

    async fn rollback(&self) -> Result<()> {
        self.log("<rollback>");
        self.check()?;
        let mut st = lock(&self.state);
        st.in_tx = false;
        st.aborted = false;
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.log("<close>");
            self.source.open_now.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
