use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use async_trait::async_trait;
use db_core::timeout::with_timeout;
use db_core::{
    Backend, BusyFlag, BusyGuard, Column, ConnectParams, Connection, Cursor, Driver, Error,
    ErrorKind, ExecOptions, Outcome, Result, Row, TxMode, Value, opt,
};
use tokio::sync::oneshot;

use crate::error::worker_gone;
use crate::worker::{ExecReply, ExecRequest, OpenSettings, Page, Reply, Request, Shared, spawn};

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
        let shared = Shared::new();
        let (tx, requests) = mpsc::channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        spawn(settings, shared.clone(), requests, ready_tx)
            .map_err(|e| Error::internal(format!("cannot start the SQLite worker thread: {e}")))?;
        // A zero timeout means no limit.
        let limit = (!params.connect_timeout.is_zero()).then_some(params.connect_timeout);
        let ready = async { ready_rx.await.unwrap_or_else(|_| Err(worker_gone())) };
        let opened = match limit {
            None => ready.await,
            Some(limit) => tokio::time::timeout(limit, ready)
                .await
                .unwrap_or_else(|_| {
                    Err(Error::new(
                        ErrorKind::Connect,
                        "timed out opening the SQLite database",
                    ))
                }),
        };
        opened?;
        Ok(Box::new(SqliteConnection {
            tx,
            shared,
            busy: BusyFlag::default(),
        }))
    }
}

/// A SQLite session. The `rusqlite` connection lives on a dedicated thread;
/// this handle talks to it through a channel and never blocks the caller's
/// thread.
pub struct SqliteConnection {
    tx: Sender<Request>,
    shared: Arc<Shared>,
    busy: BusyFlag,
}

impl SqliteConnection {
    async fn call<T>(
        &self,
        timeout: Option<Duration>,
        build: impl FnOnce(Reply<T>, u64) -> Request,
    ) -> Result<T> {
        let (reply, rx) = oneshot::channel();
        let id = self.shared.next_id();
        self.tx.send(build(reply, id)).map_err(|_| worker_gone())?;
        wait(&self.shared, timeout, rx).await
    }
}

/// Stops the running statement when the awaiting future is dropped before the
/// worker answered, so an abandoned call does not keep the worker busy.
struct AbortOnDrop<'a> {
    shared: &'a Shared,
    armed: bool,
}

impl Drop for AbortOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.shared.cancel();
        }
    }
}

async fn wait<T>(
    shared: &Shared,
    timeout: Option<Duration>,
    rx: oneshot::Receiver<Result<T>>,
) -> Result<T> {
    let mut abort = AbortOnDrop {
        shared,
        armed: true,
    };
    let answered = &mut abort.armed;
    let reply = async {
        let result = match rx.await {
            Ok(r) => r,
            Err(_) => Err(worker_gone()),
        };
        *answered = false;
        result
    };
    // The timeout policy cancels the statement and gives it a grace period to
    // report; a statement stuck in a single long VM step is abandoned.
    with_timeout(timeout, reply, async { shared.cancel() }).await
}

#[async_trait]
impl Connection for SqliteConnection {
    fn backend(&self) -> Backend {
        Backend::Sqlite
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        // One statement or cursor at a time per connection.
        let guard = self.busy.acquire()?;
        let reply = self
            .call(opts.timeout, |reply, id| {
                Request::Exec(ExecRequest {
                    id,
                    sql: sql.to_owned(),
                    params: params.to_vec(),
                    max_value_bytes: opts.max_value_bytes,
                    reply,
                })
            })
            .await?;
        Ok(match reply {
            ExecReply::Affected(n) => Outcome::Affected(n),
            ExecReply::Rows { columns, exhausted } => Outcome::Rows(Box::new(SqliteCursor {
                columns,
                tx: self.tx.clone(),
                shared: self.shared.clone(),
                timeout: opts.timeout,
                state: if exhausted {
                    State::Exhausted
                } else {
                    State::Open
                },
                guard: (!exhausted).then_some(guard),
            })),
        })
    }

    async fn cancel(&self) -> Result<()> {
        self.shared.cancel();
        Ok(())
    }

    async fn server_version(&self) -> Result<String> {
        Ok(format!("SQLite {}", rusqlite::version()))
    }

    async fn ping(&self) -> Result<()> {
        self.call(None, |reply, _| Request::Ping { reply }).await
    }

    async fn tx_mode(&self) -> TxMode {
        if self.shared.manual.load(Ordering::SeqCst) {
            TxMode::Manual
        } else {
            TxMode::Auto
        }
    }

    /// Switching to `Auto` commits what a manual transaction left pending.
    /// In `Manual` mode a transaction starts before the first statement that
    /// writes and lasts until `commit` or `rollback`; reads do not start one,
    /// so an idle session never holds a lock.
    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.call(None, |reply, _| Request::SetTxMode {
            manual: mode == TxMode::Manual,
            reply,
        })
        .await
    }

    async fn commit(&self) -> Result<()> {
        self.call(None, |reply, _| Request::Commit { reply }).await
    }

    async fn rollback(&self) -> Result<()> {
        self.call(None, |reply, _| Request::Rollback { reply })
            .await
    }

    /// Stops a running statement, closes the database and ends the worker.
    async fn close(&self) -> Result<()> {
        self.shared.cancel();
        let (reply, rx) = oneshot::channel();
        if self.tx.send(Request::Close { reply }).is_err() {
            // Already closed.
            return Ok(());
        }
        rx.await.unwrap_or(Ok(()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The worker holds the statement.
    Open,
    /// The end of the rows was reached; the statement is finalized.
    Exhausted,
    /// Closed or failed; no more rows.
    Dead,
}

/// Rows of one statement, read on demand from the worker thread.
struct SqliteCursor {
    columns: Vec<Column>,
    tx: Sender<Request>,
    shared: Arc<Shared>,
    timeout: Option<Duration>,
    state: State,
    /// Keeps the connection busy while the worker holds the statement.
    guard: Option<BusyGuard>,
}

impl SqliteCursor {
    /// Ask the worker to finalize the statement, then free the connection.
    /// The request goes out before the guard drops, so a statement started
    /// afterwards queues behind it.
    fn release(&mut self, reply: Option<Reply<()>>) {
        let _ = self.tx.send(Request::CloseCursor { reply });
        self.state = State::Dead;
        self.guard = None;
    }
}

#[async_trait]
impl Cursor for SqliteCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        match self.state {
            State::Exhausted => return Ok(Vec::new()),
            State::Dead => return Err(Error::query("the cursor is closed")),
            State::Open => {}
        }
        let (reply, rx) = oneshot::channel();
        let id = self.shared.next_id();
        let sent = self.tx.send(Request::Fetch {
            id,
            max_rows,
            reply,
        });
        if sent.is_err() {
            self.state = State::Dead;
            self.guard = None;
            return Err(worker_gone());
        }
        let page: Result<Page> = wait(&self.shared, self.timeout, rx).await;
        match page {
            Ok(page) => {
                if page.exhausted {
                    // The worker finalized the statement before answering.
                    self.state = State::Exhausted;
                    self.guard = None;
                }
                Ok(page.rows)
            }
            Err(e) => {
                self.release(None);
                Err(e)
            }
        }
    }

    fn is_exhausted(&self) -> bool {
        self.state != State::Open
    }

    async fn close(mut self: Box<Self>) -> Result<()> {
        if self.state != State::Open {
            return Ok(());
        }
        let (reply, rx) = oneshot::channel();
        self.release(Some(reply));
        // The worker answers as soon as the statement is finalized.
        rx.await.unwrap_or(Ok(()))
    }
}

impl Drop for SqliteCursor {
    fn drop(&mut self) {
        if self.state == State::Open {
            self.release(None);
        }
    }
}
