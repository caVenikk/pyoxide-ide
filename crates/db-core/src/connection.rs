use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Backend, ConnectParams, Dialect, Error, Result, TxMode, Value, ValueKind};

/// One result column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// The backend's type name (`VARCHAR2`, `numeric`, `TEXT`).
    pub type_name: String,
    pub kind: ValueKind,
    /// `true` when unknown.
    pub nullable: bool,
}

pub type Row = Vec<Value>;

/// A fully fetched (or capped) result, for the CLI, commands and tests.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    /// Affected rows for DML; 0 for queries.
    pub rows_affected: u64,
    /// More rows exist than were fetched.
    pub truncated: bool,
}

/// Per-call execution settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOptions {
    /// Applies to `execute` and to every `next_page` call. `None`: no limit.
    pub timeout: Option<Duration>,
    /// Rows fetched per network round trip.
    pub fetch_size: usize,
    /// Text, binary and LOB values longer than this come back as
    /// [`Value::Truncated`].
    pub max_value_bytes: usize,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            timeout: None,
            fetch_size: 500,
            max_value_bytes: 1 << 20,
        }
    }
}

/// What a statement produced.
pub enum Outcome {
    /// A row source; read it page by page.
    Rows(Box<dyn Cursor>),
    /// DML/DDL: number of affected rows (0 when the backend does not say).
    Affected(u64),
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Rows(c) => f
                .debug_struct("Rows")
                .field("columns", &c.columns())
                .finish_non_exhaustive(),
            Outcome::Affected(n) => f.debug_tuple("Affected").field(n).finish(),
        }
    }
}

/// An open server-side row source.
///
/// Contract for every driver:
/// - A connection has at most one open cursor. While it is open, `execute`
///   on the same connection fails with `ErrorKind::Busy`; `cancel`, `commit`,
///   `rollback` and `close` still work. Dropping or closing the cursor frees
///   the connection.
/// - Closing a cursor before it is exhausted stops the server-side work
///   (cancel or close the server cursor); it must not silently read the rest.
/// - Dropping any future returned by a cursor or connection method leaves the
///   connection usable or marks it broken (later calls fail with
///   `ErrorKind::ConnectionLost`). It never leaves it half-way in the protocol.
#[async_trait]
pub trait Cursor: Send {
    fn columns(&self) -> &[Column];

    /// Fetch up to `max_rows` more rows. An empty page means the cursor is
    /// exhausted.
    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>>;

    /// `true` once the backend has signalled the end of the rows.
    fn is_exhausted(&self) -> bool;

    /// Release the server cursor now instead of on drop.
    async fn close(self: Box<Self>) -> Result<()>;
}

/// A session with one database.
///
/// Methods take `&self` so a connection can be shared (`Arc`) between the
/// task running a statement and the task that cancels it.
#[async_trait]
pub trait Connection: Send + Sync {
    fn backend(&self) -> Backend;

    fn dialect(&self) -> Dialect {
        self.backend().dialect()
    }

    /// Run one statement. `params` bind positionally (Oracle `:1`,
    /// PostgreSQL `$1`, SQLite `?1`). Drivers bind at least `Null`, `Bool`,
    /// `Int`, `Float`, `Decimal`, `Text` and `Bytes`; anything else may fail
    /// with `ErrorKind::Unsupported`.
    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome>;

    /// Ask the server to stop the statement currently running on this
    /// connection (including a cursor fetch). Safe to call from another task
    /// and when nothing runs. Backends that cannot do this return
    /// `ErrorKind::Unsupported`; callers then drop the running future.
    async fn cancel(&self) -> Result<()>;

    /// Human-readable server version (`Oracle Database 19c ... 19.21.0.0.0`).
    async fn server_version(&self) -> Result<String>;

    /// Cheapest round trip that proves the session is alive.
    async fn ping(&self) -> Result<()>;

    async fn tx_mode(&self) -> TxMode;
    async fn set_tx_mode(&self, mode: TxMode) -> Result<()>;
    async fn commit(&self) -> Result<()>;
    async fn rollback(&self) -> Result<()>;

    async fn close(&self) -> Result<()>;

    /// Run a statement and collect at most `max_rows` rows.
    async fn query(&self, sql: &str, params: &[Value], max_rows: usize) -> Result<ResultSet> {
        let opts = ExecOptions {
            fetch_size: max_rows.clamp(1, ExecOptions::default().fetch_size),
            ..ExecOptions::default()
        };
        self.query_with(sql, params, max_rows, &opts).await
    }

    /// [`Connection::query`] with explicit options.
    async fn query_with(
        &self,
        sql: &str,
        params: &[Value],
        max_rows: usize,
        opts: &ExecOptions,
    ) -> Result<ResultSet> {
        match self.execute(sql, params, opts).await? {
            Outcome::Affected(n) => Ok(ResultSet {
                rows_affected: n,
                ..ResultSet::default()
            }),
            Outcome::Rows(cursor) => collect(cursor, max_rows, opts.fetch_size).await,
        }
    }
}

/// Opens connections for one backend.
#[async_trait]
pub trait Driver: Send + Sync {
    fn backend(&self) -> Backend;

    /// Whether the client side is usable, with a version or a reason.
    /// Pure-Rust drivers return their own version; the Oracle Client driver
    /// reports the loaded client library or why it could not be loaded.
    fn availability(&self) -> Result<String>;

    async fn connect(&self, params: &ConnectParams) -> Result<Box<dyn Connection>>;
}

/// Read up to `max_rows` rows from a cursor into a [`ResultSet`] and close it.
pub async fn collect(
    mut cursor: Box<dyn Cursor>,
    max_rows: usize,
    page: usize,
) -> Result<ResultSet> {
    let columns = cursor.columns().to_vec();
    let mut rows: Vec<Row> = Vec::new();
    let page = page.max(1);
    while rows.len() < max_rows {
        let want = (max_rows - rows.len()).min(page);
        let batch = cursor.next_page(want).await?;
        if batch.is_empty() {
            break;
        }
        rows.extend(batch);
        if cursor.is_exhausted() {
            break;
        }
    }
    rows.truncate(max_rows);
    let truncated = !cursor.is_exhausted();
    cursor.close().await?;
    Ok(ResultSet {
        columns,
        rows,
        rows_affected: 0,
        truncated,
    })
}

/// Enforces "one open cursor per connection". The connection owns the flag;
/// each cursor holds a [`BusyGuard`] that clears it on drop.
#[derive(Debug, Clone, Default)]
pub struct BusyFlag(Arc<AtomicBool>);

impl BusyFlag {
    pub fn is_busy(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Fails with `ErrorKind::Busy` if a cursor is already open.
    pub fn check(&self) -> Result<()> {
        if self.is_busy() {
            Err(Error::busy())
        } else {
            Ok(())
        }
    }

    /// Mark a cursor as open.
    pub fn acquire(&self) -> Result<BusyGuard> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::busy())?;
        Ok(BusyGuard(self.0.clone()))
    }
}

#[derive(Debug)]
pub struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A cursor over rows already in memory. Handy for drivers that receive a
/// whole result at once and for tests.
pub struct VecCursor {
    columns: Vec<Column>,
    rows: VecDeque<Row>,
    _guard: Option<BusyGuard>,
}

impl VecCursor {
    pub fn new(columns: Vec<Column>, rows: Vec<Row>) -> Self {
        Self {
            columns,
            rows: rows.into(),
            _guard: None,
        }
    }

    pub fn with_guard(mut self, guard: BusyGuard) -> Self {
        self._guard = Some(guard);
        self
    }
}

#[async_trait]
impl Cursor for VecCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    async fn next_page(&mut self, max_rows: usize) -> Result<Vec<Row>> {
        let n = max_rows.min(self.rows.len());
        Ok(self.rows.drain(..n).collect())
    }

    fn is_exhausted(&self) -> bool {
        self.rows.is_empty()
    }

    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> Column {
        Column {
            name: name.into(),
            type_name: "INTEGER".into(),
            kind: ValueKind::Int,
            nullable: true,
        }
    }

    fn rows(n: i64) -> Vec<Row> {
        (0..n).map(|i| vec![Value::Int(i)]).collect()
    }

    #[tokio::test]
    async fn collect_caps_and_flags_truncation() {
        let rs = collect(Box::new(VecCursor::new(vec![col("n")], rows(10))), 4, 3)
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 4);
        assert!(rs.truncated);

        let rs = collect(Box::new(VecCursor::new(vec![col("n")], rows(3))), 10, 2)
            .await
            .unwrap();
        assert_eq!(rs.rows.len(), 3);
        assert!(!rs.truncated);
    }

    #[test]
    fn busy_flag_allows_one_cursor() {
        let flag = BusyFlag::default();
        let g = flag.acquire().unwrap();
        assert_eq!(flag.acquire().unwrap_err().kind, crate::ErrorKind::Busy);
        assert!(flag.check().is_err());
        drop(g);
        assert!(flag.check().is_ok());
        assert!(flag.acquire().is_ok());
    }
}
