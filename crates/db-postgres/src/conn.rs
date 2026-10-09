//! The connection: statements, transactions, cancel and close.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use db_core::timeout::{CANCEL_GRACE, with_timeout};
use db_core::{
    Backend, BusyFlag, BusyGuard, Connection, Error, ExecOptions, Outcome, Result, Secret, TxMode,
    Value,
};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_postgres::error::SqlState;
use tokio_postgres::{CancelToken, Client, SimpleQueryMessage};

use tokio_postgres_rustls::MakeRustlsConnect;

use crate::bind;
use crate::cancel::{self, Target};
use crate::cursor::{CursorCore, PgCursor};
use crate::decode::column_info;
use crate::error::{self, Phase, map_scrubbed};
use crate::stmt::{self, TxEffect};

/// State shared by the connection and its cursor.
pub(crate) struct Inner {
    client: Mutex<Option<Arc<Client>>>,
    cancel: CancelToken,
    target: Target,
    tls: MakeRustlsConnect,
    secret: Secret,
    busy: BusyFlag,
    /// Cursors being stopped in the background (see `Cursor` drop).
    pub(crate) cleanups: AtomicUsize,
    pub(crate) released: Notify,
    tx_mode: Mutex<TxMode>,
    in_tx: AtomicBool,
    active: Mutex<Option<Weak<CursorCore>>>,
    version: tokio::sync::OnceCell<String>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Inner {
    pub(crate) fn new(
        client: Client,
        task: JoinHandle<()>,
        tls: MakeRustlsConnect,
        secret: Secret,
        target: Target,
    ) -> Arc<Self> {
        Arc::new(Self {
            cancel: client.cancel_token(),
            client: Mutex::new(Some(Arc::new(client))),
            target,
            tls,
            secret,
            busy: BusyFlag::default(),
            cleanups: AtomicUsize::new(0),
            released: Notify::new(),
            tx_mode: Mutex::new(TxMode::Auto),
            in_tx: AtomicBool::new(false),
            active: Mutex::new(None),
            version: tokio::sync::OnceCell::new(),
            task: Mutex::new(Some(task)),
        })
    }

    pub(crate) fn map(&self, e: &tokio_postgres::Error) -> Error {
        map_scrubbed(e, Phase::Session, self.secret.expose())
    }

    fn client(&self) -> Result<Arc<Client>> {
        let client = self
            .client
            .lock()
            .map_err(|_| Error::internal("connection state is poisoned"))?
            .clone()
            .ok_or_else(error::closed)?;
        if client.is_closed() {
            return Err(Error::connection_lost("the server closed the connection"));
        }
        Ok(client)
    }

    fn mode(&self) -> TxMode {
        self.tx_mode.lock().map(|m| *m).unwrap_or_default()
    }

    /// Ask the server to stop the running statement.
    async fn send_cancel(&self) -> Result<()> {
        let closed = self.client.lock().map(|c| c.is_none()).unwrap_or(true);
        if closed {
            return Ok(());
        }
        cancel::request(&self.cancel, &self.tls, &self.target, self.secret.expose()).await
    }

    pub(crate) async fn cancel_quietly(&self) {
        let _ = self.send_cancel().await;
    }

    /// Claim the connection for a statement. A cursor that is being stopped
    /// in the background is waited for; an open cursor is `Busy`.
    async fn acquire_guard(&self) -> Result<BusyGuard> {
        let deadline = tokio::time::Instant::now() + CANCEL_GRACE * 2;
        loop {
            let notified = self.released.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.busy.acquire() {
                Ok(guard) => return Ok(guard),
                Err(e) => {
                    if self.cleanups.load(Ordering::Acquire) == 0 {
                        return Err(e);
                    }
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(Error::busy());
            }
        }
    }

    /// Stop the open cursor, if any (before commit, rollback and close).
    async fn abort_active(&self) {
        let core = self
            .active
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .and_then(|weak| weak.upgrade());
        if let Some(core) = core {
            core.terminate().await;
        }
    }

    async fn begin_if_needed(&self, client: &Client, sql: &str) -> Result<()> {
        if self.mode() == TxMode::Manual
            && !self.in_tx.load(Ordering::Acquire)
            && stmt::needs_implicit_begin(sql)
        {
            client
                .batch_execute("BEGIN")
                .await
                .map_err(|e| self.map(&e))?;
            self.in_tx.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn note_statement(&self, sql: &str) {
        match stmt::tx_effect(sql) {
            TxEffect::Begin => self.in_tx.store(true, Ordering::Release),
            TxEffect::End => self.in_tx.store(false, Ordering::Release),
            TxEffect::None => {}
        }
    }

    async fn run_statement(
        self: &Arc<Self>,
        client: &Client,
        sql: &str,
        params: &[Value],
        opts: &ExecOptions,
        guard: BusyGuard,
    ) -> Result<Outcome> {
        let stmt = match client.prepare(sql).await {
            Ok(stmt) => stmt,
            Err(e) if params.is_empty() && is_multi_statement_error(&e) => {
                return self.run_script(client, sql).await;
            }
            Err(e) => return Err(self.map(&e)),
        };
        let binds = bind::convert_all(params, stmt.params())?;
        self.begin_if_needed(client, sql).await?;

        if stmt.columns().is_empty() {
            let affected = client
                .execute_raw(&stmt, &binds)
                .await
                .map_err(|e| self.map(&e))?;
            self.note_statement(sql);
            drop(guard);
            return Ok(Outcome::Affected(affected));
        }

        let stream = client
            .query_raw(&stmt, &binds)
            .await
            .map_err(|e| self.map(&e))?;
        self.note_statement(sql);
        let columns: Vec<_> = stmt.columns().iter().map(column_info).collect();
        let core = CursorCore::new(
            Arc::clone(self),
            stream,
            guard,
            stmt::modifies_data(sql),
            opts.max_value_bytes,
        );
        let exhausted = core.prefetch().await?;
        if let Ok(mut slot) = self.active.lock() {
            *slot = Some(Arc::downgrade(&core));
        }
        Ok(Outcome::Rows(Box::new(PgCursor::new(
            core,
            columns,
            opts.timeout,
            exhausted,
        ))))
    }

    /// Several commands in one text cannot be prepared; run them through the
    /// simple protocol. Row results are discarded, affected rows are summed.
    async fn run_script(&self, client: &Client, sql: &str) -> Result<Outcome> {
        self.begin_if_needed(client, sql).await?;
        let messages = client.simple_query(sql).await.map_err(|e| self.map(&e))?;
        let affected = messages
            .iter()
            .filter_map(|m| match m {
                SimpleQueryMessage::CommandComplete(n) => Some(*n),
                _ => None,
            })
            .sum();
        Ok(Outcome::Affected(affected))
    }

    async fn end_transaction(&self, sql: &str) -> Result<()> {
        let client = self.client()?;
        if !self.in_tx.load(Ordering::Acquire) {
            // Nothing pending (always the case in auto-commit mode).
            return Ok(());
        }
        // A transaction ends its portals, so the open cursor ends with it.
        self.abort_active().await;
        self.in_tx.store(false, Ordering::Release);
        client.batch_execute(sql).await.map_err(|e| self.map(&e))?;
        Ok(())
    }
}

fn is_multi_statement_error(e: &tokio_postgres::Error) -> bool {
    e.as_db_error().is_some_and(|d| {
        *d.code() == SqlState::SYNTAX_ERROR && d.message().contains("multiple commands")
    })
}

/// A PostgreSQL session.
pub struct PostgresConnection {
    inner: Arc<Inner>,
}

impl PostgresConnection {
    pub(crate) fn new(inner: Arc<Inner>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Connection for PostgresConnection {
    fn backend(&self) -> Backend {
        Backend::Postgres
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        let inner = &self.inner;
        let client = inner.client()?;
        let guard = inner.acquire_guard().await?;
        let run = inner.run_statement(&client, sql, params, opts, guard);
        with_timeout(opts.timeout, run, inner.cancel_quietly()).await
    }

    async fn cancel(&self) -> Result<()> {
        self.inner.send_cancel().await
    }

    async fn server_version(&self) -> Result<String> {
        let inner = &self.inner;
        if let Some(v) = inner.version.get() {
            return Ok(v.clone());
        }
        let client = inner.client()?;
        let _guard = inner.acquire_guard().await?;
        let version = inner
            .version
            .get_or_try_init(|| async {
                let row = client
                    .query_one("SELECT version()", &[])
                    .await
                    .map_err(|e| inner.map(&e))?;
                row.try_get::<_, String>(0).map_err(|e| inner.map(&e))
            })
            .await?;
        Ok(version.clone())
    }

    async fn ping(&self) -> Result<()> {
        let inner = &self.inner;
        let client = inner.client()?;
        if inner.busy.is_busy() {
            // A statement or cursor is using the session; it is alive.
            return Ok(());
        }
        client.check_connection().await.map_err(|e| inner.map(&e))
    }

    async fn tx_mode(&self) -> TxMode {
        self.inner.mode()
    }

    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        let previous = {
            let mut slot = self
                .inner
                .tx_mode
                .lock()
                .map_err(|_| Error::internal("connection state is poisoned"))?;
            std::mem::replace(&mut *slot, mode)
        };
        if previous == TxMode::Manual && mode == TxMode::Auto {
            // Leaving manual mode commits what is pending, like JDBC does.
            self.inner.end_transaction("COMMIT").await?;
        }
        Ok(())
    }

    async fn commit(&self) -> Result<()> {
        self.inner.end_transaction("COMMIT").await
    }

    async fn rollback(&self) -> Result<()> {
        self.inner.end_transaction("ROLLBACK").await
    }

    async fn close(&self) -> Result<()> {
        let inner = &self.inner;
        inner.abort_active().await;
        let client = inner.client.lock().ok().and_then(|mut slot| slot.take());
        drop(client);
        let task = inner.task.lock().ok().and_then(|mut slot| slot.take());
        if let Some(mut task) = task {
            // Dropping the last client sends Terminate; wait for the socket.
            if tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for PostgresConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConnection").finish_non_exhaustive()
    }
}
