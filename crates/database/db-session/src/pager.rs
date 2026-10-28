//! Pages of a table for the data grid (ADR 0021, "Paged results"): a
//! generated `SELECT` with the user's WHERE and ORDER BY text inserted as
//! written, paged with offset on a data connection, and `COUNT(*)` over the
//! same query only when asked.
//!
//! The statements follow what the reference sends to the server: a page
//! asks for one row more than its size, to know whether another page
//! follows without counting (`SELECT t.* FROM demo.events t LIMIT 501`,
//! the next one with `OFFSET 500`), and the count is a separate
//! `SELECT COUNT(*) FROM demo.events t` with the same condition. Without an
//! ORDER BY the server's order is kept, as the reference does.

use std::sync::{Arc, Mutex, PoisonError};

use db_core::{Column, Dialect, Error, ErrorKind, Row, Value};
use serde::Serialize;
use tokio::sync::watch;

use crate::cell::CellQuery;
use crate::data_source::{DataSourceHandle, Shared, cancelled};
use crate::event::JobId;
use crate::job::syntax_dialect;
use crate::session::{Guarded, guarded};

/// A table or view of a data source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TableRef {
    /// `None` for an engine without schemas or the connection's default.
    pub schema: Option<String>,
    pub name: String,
}

impl TableRef {
    pub fn new(schema: Option<&str>, name: &str) -> Self {
        Self {
            schema: schema.map(str::to_owned),
            name: name.to_owned(),
        }
    }

    /// `schema.name`, each part quoted only where the dialect needs it.
    pub fn qualified(&self, dialect: Dialect) -> String {
        let d = syntax_dialect(dialect);
        match &self.schema {
            Some(schema) => format!("{}.{}", d.quote(schema), d.quote(&self.name)),
            None => d.quote(&self.name),
        }
    }
}

/// What a grid of a table shows: the table, the condition and the order,
/// both as the user typed them (empty: none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableQuery {
    pub table: TableRef,
    /// The text of the WHERE field, without the keyword.
    pub filter: String,
    /// The text of the ORDER BY field, without the keywords.
    pub order: String,
}

impl TableQuery {
    pub fn new(table: TableRef) -> Self {
        Self {
            table,
            filter: String::new(),
            order: String::new(),
        }
    }

    /// `SELECT t.* FROM <table> t`, then the condition and the order on
    /// lines of their own (a `--` comment in one of them ends with its line).
    fn base(&self, dialect: Dialect, select: &str) -> String {
        let mut sql = format!("SELECT {select}\nFROM {} t", self.table.qualified(dialect));
        let filter = self.filter.trim();
        if !filter.is_empty() {
            sql.push_str("\nWHERE ");
            sql.push_str(filter);
        }
        sql
    }

    /// The statement of a page: `size` rows from `first_row`, asking for
    /// one more to know whether more follow.
    pub fn page_sql(&self, dialect: Dialect, first_row: u64, size: PageSize) -> String {
        let mut sql = self.base(dialect, "t.*");
        let order = self.order.trim();
        if !order.is_empty() {
            sql.push_str("\nORDER BY ");
            sql.push_str(order);
        }
        let limit = match size {
            PageSize::Rows(n) => Some(n.max(1) as u64 + 1),
            PageSize::All => None,
        };
        match dialect {
            Dialect::Oracle => {
                if first_row > 0 {
                    sql.push_str(&format!("\nOFFSET {first_row} ROWS"));
                }
                if let Some(limit) = limit {
                    sql.push_str(&format!("\nFETCH NEXT {limit} ROWS ONLY"));
                }
            }
            Dialect::Postgres | Dialect::Sqlite => {
                match limit {
                    Some(limit) => sql.push_str(&format!("\nLIMIT {limit}")),
                    // SQLite takes an offset only after a limit.
                    None if first_row > 0 && dialect == Dialect::Sqlite => {
                        sql.push_str("\nLIMIT -1")
                    }
                    None => {}
                }
                if first_row > 0 {
                    sql.push_str(&format!("\nOFFSET {first_row}"));
                }
            }
        }
        sql
    }

    /// `SELECT COUNT(*)` over the table with the same condition.
    pub fn count_sql(&self, dialect: Dialect) -> String {
        self.base(dialect, "COUNT(*)")
    }
}

/// Rows of one page: a number, or every row (the reference's "All").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    Rows(usize),
    All,
}

impl PageSize {
    /// The number of rows, `None` for all.
    pub fn rows(self) -> Option<usize> {
        match self {
            PageSize::Rows(n) => Some(n.max(1)),
            PageSize::All => None,
        }
    }
}

/// A page read by a [`ResultPager`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    /// Zero-based number of the first row of `rows` in the result.
    pub first_row: u64,
    /// More rows follow this page.
    pub has_more: bool,
    /// The statement as sent.
    pub sql: String,
}

/// A failed page or count: the server's error and the statement it was
/// about (its `position` points into it).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageError {
    pub error: Error,
    pub sql: String,
}

impl PageError {
    /// Cancelled by [`ResultPager::cancel`] (or a disconnect).
    pub fn is_cancelled(&self) -> bool {
        self.error.kind == ErrorKind::Cancelled
    }
}

/// What a [`ResultPager`] reports, on a thread of the database runtime.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PagerEvent {
    Page {
        job: JobId,
        result: Result<Page, PageError>,
    },
    Count {
        job: JobId,
        result: Result<u64, PageError>,
    },
    /// A cell's whole value, read by [`ResultPager::cell`].
    Cell {
        job: JobId,
        result: Result<Value, PageError>,
    },
}

/// Receives a pager's events; it must return quickly (the UI crate turns an
/// event into a foreground task).
pub type PagerSink = Arc<dyn Fn(PagerEvent) + Send + Sync>;

/// Reads pages and counts of tables on the data connections of a data
/// source (never in a console's transaction), each read a job that
/// [`ResultPager::cancel`] stops on the server. Dropping the pager cancels
/// what it still reads.
pub struct ResultPager {
    shared: Arc<Shared>,
    sink: PagerSink,
    jobs: Arc<Mutex<Vec<JobId>>>,
}

impl std::fmt::Debug for ResultPager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultPager")
            .field("data_source", &self.shared.id)
            .finish_non_exhaustive()
    }
}

impl DataSourceHandle {
    /// A pager for table grids of this data source; its pages and counts
    /// go to `sink`, not to the data source's event sink.
    pub fn pager(&self, sink: impl Fn(PagerEvent) + Send + Sync + 'static) -> ResultPager {
        ResultPager {
            shared: self.shared.clone(),
            sink: Arc::new(sink),
            jobs: Arc::default(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ResultPager {
    /// Reads the page of `query` that starts at `first_row`; reported as
    /// [`PagerEvent::Page`].
    pub fn page(&self, query: &TableQuery, first_row: u64, size: PageSize) -> JobId {
        let query = query.clone();
        self.spawn(move |shared, job, rx| async move {
            let result = shared.run_page(&query, first_row, size, rx).await;
            PagerEvent::Page { job, result }
        })
    }

    /// Counts the rows of `query` with a separate `COUNT(*)`; reported as
    /// [`PagerEvent::Count`].
    pub fn count(&self, query: &TableQuery) -> JobId {
        let query = query.clone();
        self.spawn(move |shared, job, rx| async move {
            let result = shared.run_count(&query, rx).await;
            PagerEvent::Count { job, result }
        })
    }

    /// Reads the whole value of one cell, found by its row's key (see
    /// [`CellQuery`]); reported as [`PagerEvent::Cell`].
    pub fn cell(&self, query: &CellQuery) -> JobId {
        let query = query.clone();
        self.spawn(move |shared, job, rx| async move {
            let result = shared.run_cell(&query, rx).await;
            PagerEvent::Cell { job, result }
        })
    }

    /// Stops a read of this pager: the server is asked to cancel, the job
    /// ends with a cancelled error.
    pub fn cancel(&self, job: JobId) {
        self.shared.cancel_job(job);
    }

    /// The reads that did not end yet.
    pub fn running(&self) -> Vec<JobId> {
        lock(&self.jobs).clone()
    }

    fn spawn<F, Fut>(&self, run: F) -> JobId
    where
        F: FnOnce(Arc<Shared>, JobId, watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = PagerEvent> + Send + 'static,
    {
        let (job, rx) = self.shared.new_job();
        lock(&self.jobs).push(job);
        let shared = self.shared.clone();
        let sink = self.sink.clone();
        let jobs = self.jobs.clone();
        self.shared.runtime.spawn(async move {
            let inner = shared.clone();
            let task = tokio::spawn(run(inner, job, rx));
            let event = match task.await {
                Ok(event) => event,
                Err(e) => PagerEvent::Page {
                    job,
                    result: Err(PageError {
                        error: Error::internal(format!("the read stopped: {e}")),
                        sql: String::new(),
                    }),
                },
            };
            shared.end_job(job);
            lock(&jobs).retain(|j| *j != job);
            sink(event);
        });
        job
    }
}

impl Drop for ResultPager {
    fn drop(&mut self) {
        for job in self.running() {
            self.cancel(job);
        }
    }
}

impl Shared {
    async fn run_page(
        self: &Arc<Self>,
        query: &TableQuery,
        first_row: u64,
        size: PageSize,
        rx: watch::Receiver<bool>,
    ) -> Result<Page, PageError> {
        let max_rows = size.rows().map_or(usize::MAX, |n| n + 1);
        let (columns, mut rows, sql) = self
            .run_select(|d| query.page_sql(d, first_row, size), max_rows, None, rx)
            .await?;
        let has_more = size.rows().is_some_and(|n| rows.len() > n);
        if let Some(n) = size.rows() {
            rows.truncate(n);
        }
        Ok(Page {
            columns,
            rows,
            first_row,
            has_more,
            sql,
        })
    }

    async fn run_count(
        self: &Arc<Self>,
        query: &TableQuery,
        rx: watch::Receiver<bool>,
    ) -> Result<u64, PageError> {
        let (_, rows, sql) = self.run_select(|d| query.count_sql(d), 1, None, rx).await?;
        let value = rows.first().and_then(|r| r.first());
        count_of(value).ok_or_else(|| PageError {
            error: Error::internal(format!("the count is not a number: {value:?}")),
            sql,
        })
    }

    /// Runs the statement `sql` makes for the connection's dialect on a data
    /// connection, collecting up to `max_rows` rows.
    pub(crate) async fn run_select(
        self: &Arc<Self>,
        sql: impl FnOnce(Dialect) -> String,
        max_rows: usize,
        max_value_bytes: Option<usize>,
        mut rx: watch::Receiver<bool>,
    ) -> Result<(Vec<Column>, Vec<Row>, String), PageError> {
        self.touch();
        let mut lease = tokio::select! {
            lease = self.lease_data() => lease.map_err(|error| PageError { error, sql: String::new() })?,
            _ = cancelled(&mut rx) => {
                return Err(PageError { error: Error::cancelled(), sql: String::new() })
            }
        };
        let conn = lease.conn().clone();
        let sql = sql(conn.dialect());
        let mut opts = self.exec_options();
        if let Some(limit) = max_value_bytes {
            opts.max_value_bytes = limit;
        }
        let read = conn.query_with(&sql, &[], max_rows, &opts);
        let result = match guarded(&conn, read, Some(&mut rx), self.options.cancel_grace).await {
            Guarded::Done(r) => {
                if let Err(e) = &r
                    && e.kind == ErrorKind::ConnectionLost
                {
                    lease.broken();
                }
                r
            }
            Guarded::Abandoned => {
                lease.broken();
                Err(Error::cancelled())
            }
        };
        match result {
            Ok(set) => Ok((set.columns, set.rows, sql)),
            Err(error) => Err(PageError { error, sql }),
        }
    }
}

fn count_of(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Int(n) => u64::try_from(*n).ok(),
        Value::Decimal(s) | Value::Text(s) => s.trim().parse().ok(),
        Value::Float(f) if *f >= 0.0 => Some(*f as u64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events() -> TableQuery {
        TableQuery::new(TableRef::new(Some("demo"), "events"))
    }

    #[test]
    fn page_statements_follow_the_reference() {
        let q = events();
        assert_eq!(
            q.page_sql(Dialect::Postgres, 0, PageSize::Rows(500)),
            "SELECT t.*\nFROM demo.events t\nLIMIT 501"
        );
        assert_eq!(
            q.page_sql(Dialect::Postgres, 500, PageSize::Rows(500)),
            "SELECT t.*\nFROM demo.events t\nLIMIT 501\nOFFSET 500"
        );
        assert_eq!(
            q.count_sql(Dialect::Postgres),
            "SELECT COUNT(*)\nFROM demo.events t"
        );
        let q = TableQuery {
            filter: " kind='error' ".into(),
            order: "at desc".into(),
            ..events()
        };
        assert_eq!(
            q.page_sql(Dialect::Postgres, 0, PageSize::Rows(500)),
            "SELECT t.*\nFROM demo.events t\nWHERE kind='error'\nORDER BY at desc\nLIMIT 501"
        );
        assert_eq!(
            q.count_sql(Dialect::Postgres),
            "SELECT COUNT(*)\nFROM demo.events t\nWHERE kind='error'"
        );
    }

    #[test]
    fn oracle_and_sqlite_page_in_their_syntax() {
        let q = TableQuery::new(TableRef::new(Some("HR"), "Emp"));
        assert_eq!(
            q.page_sql(Dialect::Oracle, 100, PageSize::Rows(100)),
            "SELECT t.*\nFROM HR.\"Emp\" t\nOFFSET 100 ROWS\nFETCH NEXT 101 ROWS ONLY"
        );
        assert_eq!(
            q.page_sql(Dialect::Oracle, 0, PageSize::All),
            "SELECT t.*\nFROM HR.\"Emp\" t"
        );
        let q = TableQuery::new(TableRef::new(None, "t"));
        assert_eq!(
            q.page_sql(Dialect::Sqlite, 0, PageSize::Rows(10)),
            "SELECT t.*\nFROM t t\nLIMIT 11"
        );
        assert_eq!(
            q.page_sql(Dialect::Sqlite, 20, PageSize::All),
            "SELECT t.*\nFROM t t\nLIMIT -1\nOFFSET 20"
        );
    }

    #[test]
    fn counts_of_every_engine() {
        assert_eq!(count_of(Some(&Value::Int(12))), Some(12));
        assert_eq!(count_of(Some(&Value::Decimal("12000".into()))), Some(12000));
        assert_eq!(count_of(Some(&Value::Null)), None);
        assert_eq!(count_of(None), None);
    }
}
