use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use async_trait::async_trait;
use db_core::worker::{OnAbandon, Worker, WorkerConfig};
use db_core::{
    Backend, ConnectParams, Connection, Driver, Error, ErrorKind, ExecOptions, Outcome, Result,
    TxMode, Value, opt,
};

use crate::session::{CancelFlag, OpenSettings, SqliteSession};

/// Backend option: how long a statement waits for a lock held by another
/// connection, in milliseconds (default 5000).
pub const OPT_BUSY_TIMEOUT_MS: &str = opt::SQLITE_BUSY_TIMEOUT_MS;

const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens SQLite databases with the bundled SQLite library.
///
/// `ConnectParams::database` is a file path, `:memory:` or a `file:` URI.
/// A missing file is an error unless the option [`opt::SQLITE_CREATE`] is
/// `true`; `read_only` opens it read-only.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteDriver;

#[async_trait]
impl Driver for SqliteDriver {
    fn backend(&self) -> Backend {
        Backend::Sqlite
    }

    fn availability(&self) -> Result<String> {
        Ok(format!("SQLite {} (bundled)", rusqlite::version()))
    }

    async fn connect(&self, params: &ConnectParams) -> Result<Box<dyn Connection>> {
        if params.database.trim().is_empty() {
            return Err(Error::config(
                "the SQLite database is empty; give a file path or :memory:",
            ));
        }
        let busy_timeout = match params.option(OPT_BUSY_TIMEOUT_MS) {
            None => DEFAULT_BUSY_TIMEOUT,
            Some(ms) => Duration::from_millis(ms.parse().map_err(|_| {
                Error::config(format!(
                    "{OPT_BUSY_TIMEOUT_MS} must be a number of milliseconds, got '{ms}'"
                ))
            })?),
        };
        let settings = OpenSettings {
            path: params.database.clone(),
            read_only: params.read_only,
            create: params.flag(opt::SQLITE_CREATE),
            busy_timeout,
        };
        let config = WorkerConfig {
            thread_name: "db-sqlite".into(),
            // Interrupting a statement leaves SQLite consistent, so an
            // abandoned call is cancelled and the connection stays usable.
            on_abandon: OnAbandon::Cancel,
            session_enforces_timeouts: false,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let start = Worker::start(config, Arc::new(CancelFlag(cancel)), move || {
            SqliteSession::open(&settings, flag)
        });
        // A zero timeout means no limit.
        let worker = if params.connect_timeout.is_zero() {
            start.await?
        } else {
            tokio::time::timeout(params.connect_timeout, start)
                .await
                .unwrap_or_else(|_| {
                    Err(Error::new(
                        ErrorKind::Connect,
                        "timed out opening the SQLite database",
                    ))
                })?
        };
        Ok(Box::new(SqliteConnection { worker }))
    }
}

/// A SQLite session. The `rusqlite` connection lives on a dedicated thread
/// (see [`db_core::worker`]); this handle talks to it through a channel and
/// never blocks the caller's thread. Result sets are read incrementally: the
/// thread keeps the prepared statement open and steps only as many rows as
/// each page asks for.
pub struct SqliteConnection {
    worker: Worker,
}

#[async_trait]
impl Connection for SqliteConnection {
    fn backend(&self) -> Backend {
        Backend::Sqlite
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        self.worker.execute(sql, params, opts).await
    }

    async fn cancel(&self) -> Result<()> {
        self.worker.cancel().await
    }

    async fn server_version(&self) -> Result<String> {
        Ok(format!("SQLite {}", rusqlite::version()))
    }

    async fn ping(&self) -> Result<()> {
        self.worker.ping().await
    }

    async fn tx_mode(&self) -> TxMode {
        self.worker.tx_mode()
    }

    /// Switching to `Auto` commits what a manual transaction left pending.
    /// In `Manual` mode a transaction starts before the first statement that
    /// writes and lasts until `commit` or `rollback`; reads do not start one,
    /// so an idle session never holds a lock.
    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.worker.set_tx_mode(mode).await
    }

    async fn commit(&self) -> Result<()> {
        self.worker.commit().await
    }

    async fn rollback(&self) -> Result<()> {
        self.worker.rollback().await
    }

    /// Stops a running statement, closes the database and ends the thread.
    async fn close(&self) -> Result<()> {
        self.worker.cancel().await?;
        self.worker.close().await
    }
}
