use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{
    Column, Connection, Dialect, Error, ErrorKind, ExecOptions, Outcome, Purpose, Value,
};
use serde::Serialize;
use tokio::sync::watch;

use super::{ColumnMap, Encoding, ImportFormat, convert, open_records};
use crate::data_source::{DataSourceHandle, Shared, cancelled, close_in_background};
use crate::event::JobId;
use crate::job::syntax_dialect;
use crate::pager::TableRef;
use crate::session::{Guarded, guarded};

/// Rows of one `INSERT` at most.
const BATCH_ROWS: usize = 500;
/// Bound parameters of one `INSERT` at most; Oracle's thin driver limits a
/// request to 63 KiB, so its statements are smaller (and halve again when
/// the driver still refuses one).
const BATCH_PARAMS: usize = 2000;
const ORACLE_BATCH_PARAMS: usize = 400;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
const SAVEPOINT: &str = "px_import";

#[derive(Debug, Clone, PartialEq)]
pub struct ImportRequest {
    pub table: TableRef,
    pub file: PathBuf,
    pub format: ImportFormat,
    /// `None`: found from the file.
    pub encoding: Option<Encoding>,
    /// The columns to fill and where their values come from.
    pub mapping: Vec<ColumnMap>,
    /// A value that does not fit its column becomes NULL instead of failing
    /// the import.
    pub inconvertible_as_null: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportDone {
    pub rows: u64,
    #[serde(skip)]
    pub elapsed: Duration,
}

/// Why an import did not happen, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportFailure {
    /// The data row, from 1 (the header is not a row).
    pub row: Option<u64>,
    /// The line of the file the row starts on.
    pub line: Option<u64>,
    pub reason: String,
    /// The server's error, when it gave one.
    pub error: Option<Error>,
}

impl ImportFailure {
    fn general(reason: impl Into<String>) -> Self {
        Self {
            row: None,
            line: None,
            reason: reason.into(),
            error: None,
        }
    }

    fn of_error(e: Error) -> Self {
        Self {
            row: None,
            line: None,
            reason: e.message.clone(),
            error: Some(e),
        }
    }
}

impl std::fmt::Display for ImportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.row, self.line) {
            (Some(row), Some(line)) => write!(f, "Row {row} (line {line}): {}", self.reason),
            (Some(row), None) => write!(f, "Row {row}: {}", self.reason),
            _ => f.write_str(&self.reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    Done(ImportDone),
    /// Stopped by [`DataSourceHandle::cancel_job`]; nothing was inserted.
    Cancelled,
    /// Nothing was inserted.
    Failed(ImportFailure),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportEvent {
    Progress {
        job: JobId,
        rows: u64,
        /// Bytes of the file read and its size.
        bytes: u64,
        total: u64,
    },
    Finished {
        job: JobId,
        outcome: ImportOutcome,
    },
}

pub type ImportSink = Arc<dyn Fn(ImportEvent) + Send + Sync>;

impl DataSourceHandle {
    /// Imports the file of `request` in one transaction on a connection of
    /// its own; progress and the end go to `sink`.
    /// [`DataSourceHandle::cancel_job`] stops it.
    pub fn import(&self, request: ImportRequest, sink: ImportSink) -> JobId {
        let (job, rx) = self.shared.new_job();
        let shared = self.shared.clone();
        self.shared.runtime.spawn(async move {
            let inner = shared.clone();
            let progress = sink.clone();
            let task =
                tokio::spawn(async move { inner.run_import(job, request, progress, rx).await });
            let outcome = match task.await {
                Ok(outcome) => outcome,
                Err(e) => ImportOutcome::Failed(ImportFailure::general(format!(
                    "the import stopped: {e}"
                ))),
            };
            shared.end_job(job);
            sink(ImportEvent::Finished { job, outcome });
        });
        job
    }
}

/// A batch read from the file.
struct Batch {
    rows: Vec<Vec<Value>>,
    /// The data row number of `rows[0]` and the lines the rows start on.
    first_row: u64,
    lines: Vec<u64>,
}

struct Reading {
    reader: super::CsvReader<Box<dyn std::io::BufRead + Send>>,
    format: ImportFormat,
    /// The file column and the table column of every value of a row.
    plan: Vec<(usize, Column)>,
    inconvertible_as_null: bool,
    /// Data rows read.
    rows: u64,
}

impl Reading {
    fn next_batch(&mut self, max: usize) -> Result<Batch, ImportFailure> {
        let first_row = self.rows + 1;
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        while rows.len() < max {
            let record = match self.reader.next_record() {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) => {
                    return Err(ImportFailure {
                        row: Some(self.rows + 1),
                        line: None,
                        reason: e.to_string(),
                        error: None,
                    });
                }
            };
            self.rows += 1;
            let mut values = Vec::with_capacity(self.plan.len());
            for (source, column) in &self.plan {
                let Some(field) = record.fields.get(*source) else {
                    return Err(ImportFailure {
                        row: Some(self.rows),
                        line: Some(record.line),
                        reason: format!(
                            "the row has {} fields, column {} takes field {}",
                            record.fields.len(),
                            column.name,
                            source + 1
                        ),
                        error: None,
                    });
                };
                let value = match self.format.value_of(field) {
                    None => Value::Null,
                    Some(text) => match convert(&text, column.kind) {
                        Ok(v) => v,
                        Err(_) if self.inconvertible_as_null => Value::Null,
                        Err(why) => {
                            return Err(ImportFailure {
                                row: Some(self.rows),
                                line: Some(record.line),
                                reason: format!("column {}: {why}", column.name),
                                error: None,
                            });
                        }
                    },
                };
                values.push(value);
            }
            rows.push(values);
            lines.push(record.line);
        }
        Ok(Batch {
            rows,
            first_row,
            lines,
        })
    }
}

fn placeholder(dialect: Dialect, n: usize) -> String {
    match dialect {
        Dialect::Postgres => format!("${n}"),
        Dialect::Sqlite => format!("?{n}"),
        Dialect::Oracle => format!(":{n}"),
    }
}

/// One statement inserting `rows` rows. PostgreSQL and SQLite take
/// `INSERT INTO t (a, b) VALUES ($1, $2), ($3, $4)`; Oracle has no such
/// list, a multitable insert does the same:
/// `INSERT ALL INTO t (a, b) VALUES (:1, :2) INTO t (a, b) VALUES (:3, :4)
/// SELECT 1 FROM dual`.
fn insert_sql(dialect: Dialect, table: &str, columns: &str, width: usize, rows: usize) -> String {
    let one = |r: usize| {
        let marks: Vec<String> = (0..width)
            .map(|c| placeholder(dialect, r * width + c + 1))
            .collect();
        format!("({})", marks.join(", "))
    };
    if dialect == Dialect::Oracle {
        if rows == 1 {
            return format!("INSERT INTO {table} ({columns}) VALUES {}", one(0));
        }
        let mut sql = String::from("INSERT ALL");
        for r in 0..rows {
            sql.push_str(&format!(" INTO {table} ({columns}) VALUES {}", one(r)));
        }
        sql.push_str(" SELECT 1 FROM dual");
        return sql;
    }
    let tuples: Vec<String> = (0..rows).map(one).collect();
    format!(
        "INSERT INTO {table} ({columns}) VALUES {}",
        tuples.join(", ")
    )
}

/// The failure of a statement as an import's: cancelled or not.
enum Stop {
    Cancelled,
    Failed(Error),
}

impl Shared {
    async fn insert(
        &self,
        conn: &Arc<dyn Connection>,
        sql: &str,
        params: &[Value],
        opts: &ExecOptions,
        rx: &mut watch::Receiver<bool>,
    ) -> Result<(), Stop> {
        let run = conn.execute(sql, params, opts);
        match guarded(conn, run, Some(rx), self.options.cancel_grace).await {
            Guarded::Done(Ok(Outcome::Affected(_))) => Ok(()),
            Guarded::Done(Ok(Outcome::Rows(cursor))) => {
                let _ = cursor.close().await;
                Ok(())
            }
            Guarded::Done(Err(e)) if e.kind == ErrorKind::Cancelled => Err(Stop::Cancelled),
            Guarded::Done(Err(e)) => Err(Stop::Failed(e)),
            Guarded::Abandoned => Err(Stop::Cancelled),
        }
    }

    async fn run_import(
        self: &Arc<Self>,
        job: JobId,
        request: ImportRequest,
        progress: ImportSink,
        mut rx: watch::Receiver<bool>,
    ) -> ImportOutcome {
        let started = Instant::now();
        self.touch();
        // A connection of its own: the import's transaction is nobody
        // else's.
        let opened = tokio::select! {
            r = self.open(Purpose::Data) => r,
            _ = cancelled(&mut rx) => return ImportOutcome::Cancelled,
        };
        let conn = match opened {
            Ok((conn, _)) => conn,
            Err(e) => return ImportOutcome::Failed(ImportFailure::of_error(e)),
        };
        let outcome = self
            .import_on(&conn, job, request, &progress, &mut rx, started)
            .await;
        if !matches!(outcome, ImportOutcome::Done(_)) {
            let _ = tokio::time::timeout(self.options.cancel_grace, conn.rollback()).await;
        }
        close_in_background(&self.runtime, conn);
        outcome
    }

    async fn import_on(
        self: &Arc<Self>,
        conn: &Arc<dyn Connection>,
        job: JobId,
        request: ImportRequest,
        progress: &ImportSink,
        rx: &mut watch::Receiver<bool>,
        started: Instant,
    ) -> ImportOutcome {
        let dialect = conn.dialect();
        let d = syntax_dialect(dialect);
        let table = request.table.qualified(dialect);
        let opts = self.exec_options();
        let grace = self.options.cancel_grace;

        // The table's columns with their kinds.
        let describe = format!("SELECT * FROM {table} WHERE 1 = 0");
        let columns = match guarded(
            conn,
            conn.execute(&describe, &[], &opts),
            Some(&mut *rx),
            grace,
        )
        .await
        {
            Guarded::Done(Ok(Outcome::Rows(cursor))) => {
                let columns = cursor.columns().to_vec();
                let _ = cursor.close().await;
                columns
            }
            Guarded::Done(Ok(Outcome::Affected(_))) => {
                return ImportOutcome::Failed(ImportFailure::general("the table has no columns"));
            }
            Guarded::Done(Err(e)) if e.kind == ErrorKind::Cancelled => {
                return ImportOutcome::Cancelled;
            }
            Guarded::Done(Err(e)) => return ImportOutcome::Failed(ImportFailure::of_error(e)),
            Guarded::Abandoned => return ImportOutcome::Cancelled,
        };
        let mut plan = Vec::new();
        for map in &request.mapping {
            let Some(source) = map.source else { continue };
            let found = columns.iter().find(|c| c.name == map.column).or_else(|| {
                columns
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(&map.column))
            });
            let Some(column) = found else {
                return ImportOutcome::Failed(ImportFailure::general(format!(
                    "the table has no column {}",
                    map.column
                )));
            };
            plan.push((source, column.clone()));
        }
        if plan.is_empty() {
            return ImportOutcome::Failed(ImportFailure::general("no column is mapped"));
        }
        let names: Vec<String> = plan.iter().map(|(_, c)| d.quote(&c.name)).collect();
        let names = names.join(", ");
        let width = plan.len();

        let (reader, _) = match open_records(&request.file, &request.format, request.encoding) {
            Ok(r) => r,
            Err(e) => {
                return ImportOutcome::Failed(ImportFailure::general(format!(
                    "cannot read {}: {e}",
                    request.file.display()
                )));
            }
        };
        let total = std::fs::metadata(&request.file).map_or(0, |m| m.len());
        let mut reading = Some(Reading {
            reader,
            format: request.format.clone(),
            plan,
            inconvertible_as_null: request.inconvertible_as_null,
            rows: 0,
        });
        // Skip the header (off the async threads, as every read).
        if request.format.header {
            let mut r = reading.take().expect("set above");
            let skipped = tokio::task::spawn_blocking(move || {
                let res = r.reader.next_record();
                (r, res)
            })
            .await;
            match skipped {
                Ok((r, Ok(_))) => reading = Some(r),
                Ok((_, Err(e))) => {
                    return ImportOutcome::Failed(ImportFailure::general(e.to_string()));
                }
                Err(e) => {
                    return ImportOutcome::Failed(ImportFailure::general(format!(
                        "the reader stopped: {e}"
                    )));
                }
            }
        }

        // One transaction.
        if let Err(e) = conn.set_tx_mode(db_core::TxMode::Manual).await {
            return ImportOutcome::Failed(ImportFailure::of_error(e));
        }
        // PostgreSQL aborts the transaction when a statement fails, so every
        // batch gets a savepoint to go back to; SQLite and Oracle undo only
        // the failed statement.
        let savepoints = dialect == Dialect::Postgres;
        if dialect != Dialect::Oracle
            && let Err(stop) = self.insert(conn, "BEGIN", &[], &opts, rx).await
        {
            return stop_outcome(stop);
        }
        let per_statement = if dialect == Dialect::Oracle {
            ORACLE_BATCH_PARAMS
        } else {
            BATCH_PARAMS
        };
        let mut size = (per_statement / width).clamp(1, BATCH_ROWS);
        let cx = InsertCx {
            dialect,
            table,
            names,
            width,
            savepoints,
            opts,
        };

        let mut inserted: u64 = 0;
        let mut last_progress = Instant::now();
        loop {
            if *rx.borrow() {
                return ImportOutcome::Cancelled;
            }
            let mut r = reading
                .take()
                .expect("the reader is back after every batch");
            let read_size = size;
            let read = tokio::task::spawn_blocking(move || {
                let b = r.next_batch(read_size);
                (r, b)
            })
            .await;
            let (batch, bytes) = match read {
                Ok((r, b)) => {
                    let bytes = r.reader.bytes_read();
                    reading = Some(r);
                    match b {
                        Ok(b) => (b, bytes),
                        Err(f) => return ImportOutcome::Failed(f),
                    }
                }
                Err(e) => {
                    return ImportOutcome::Failed(ImportFailure::general(format!(
                        "the reader stopped: {e}"
                    )));
                }
            };
            if batch.rows.is_empty() {
                break;
            }
            let n = batch.rows.len();
            if let Err(outcome) = self.insert_batch(conn, &cx, &batch, &mut size, rx).await {
                return outcome;
            }
            inserted += n as u64;
            if last_progress.elapsed() >= PROGRESS_EVERY {
                last_progress = Instant::now();
                progress(ImportEvent::Progress {
                    job,
                    rows: inserted,
                    bytes,
                    total,
                });
            }
        }
        match guarded(conn, conn.commit(), Some(&mut *rx), grace).await {
            Guarded::Done(Ok(())) => ImportOutcome::Done(ImportDone {
                rows: inserted,
                elapsed: started.elapsed(),
            }),
            Guarded::Done(Err(e)) if e.kind == ErrorKind::Cancelled => ImportOutcome::Cancelled,
            Guarded::Done(Err(e)) => ImportOutcome::Failed(ImportFailure::of_error(e)),
            Guarded::Abandoned => ImportOutcome::Cancelled,
        }
    }
}

/// What the statements of an import share.
struct InsertCx {
    dialect: Dialect,
    table: String,
    /// The quoted column names, joined.
    names: String,
    width: usize,
    savepoints: bool,
    opts: ExecOptions,
}

impl Shared {
    /// One chunk of rows in one statement, under a savepoint where the
    /// server needs one.
    async fn insert_chunk(
        &self,
        conn: &Arc<dyn Connection>,
        cx: &InsertCx,
        rows: &[Vec<Value>],
        rx: &mut watch::Receiver<bool>,
    ) -> Result<(), Stop> {
        let sql = insert_sql(cx.dialect, &cx.table, &cx.names, cx.width, rows.len());
        let params: Vec<Value> = rows.iter().flatten().cloned().collect();
        let guard = cx.savepoints && rows.len() > 1;
        if guard {
            self.insert(conn, &format!("SAVEPOINT {SAVEPOINT}"), &[], &cx.opts, rx)
                .await?;
        }
        self.insert(conn, &sql, &params, &cx.opts, rx).await?;
        if guard {
            self.insert(
                conn,
                &format!("RELEASE SAVEPOINT {SAVEPOINT}"),
                &[],
                &cx.opts,
                rx,
            )
            .await?;
        }
        Ok(())
    }

    /// Inserts a batch of rows. A driver that cannot take a statement that
    /// large (`Unsupported`) gets smaller ones, and `size` follows. A
    /// server error is traced to its row by inserting the failing statement's
    /// rows one by one.
    async fn insert_batch(
        &self,
        conn: &Arc<dyn Connection>,
        cx: &InsertCx,
        batch: &Batch,
        size: &mut usize,
        rx: &mut watch::Receiver<bool>,
    ) -> Result<(), ImportOutcome> {
        let mut start = 0;
        while start < batch.rows.len() {
            let end = (start + *size).min(batch.rows.len());
            let chunk = &batch.rows[start..end];
            let e = match self.insert_chunk(conn, cx, chunk, rx).await {
                Ok(()) => {
                    start = end;
                    continue;
                }
                Err(Stop::Cancelled) => return Err(ImportOutcome::Cancelled),
                Err(Stop::Failed(e)) => e,
            };
            let n = chunk.len();
            if e.kind == ErrorKind::Unsupported && n > 1 {
                *size = (n / 2).max(1);
                continue;
            }
            if e.kind == ErrorKind::ConnectionLost || n == 1 {
                return Err(ImportOutcome::Failed(failure_at(batch, start, e)));
            }
            if cx.savepoints {
                let back = format!("ROLLBACK TO SAVEPOINT {SAVEPOINT}");
                let _ = self.insert(conn, &back, &[], &cx.opts, rx).await;
            }
            for (i, row) in chunk.iter().enumerate() {
                let sql = insert_sql(cx.dialect, &cx.table, &cx.names, cx.width, 1);
                match self.insert(conn, &sql, row, &cx.opts, rx).await {
                    Ok(()) => {}
                    Err(Stop::Cancelled) => return Err(ImportOutcome::Cancelled),
                    // The server named the row.
                    Err(Stop::Failed(row_error)) if row_error.kind == ErrorKind::Query => {
                        return Err(ImportOutcome::Failed(failure_at(
                            batch,
                            start + i,
                            row_error,
                        )));
                    }
                    // The search itself failed (a driver limit): the
                    // statement's own error stands.
                    Err(Stop::Failed(_)) => break,
                }
            }
            return Err(ImportOutcome::Failed(failure_at(batch, start, e)));
        }
        Ok(())
    }
}

fn stop_outcome(stop: Stop) -> ImportOutcome {
    match stop {
        Stop::Cancelled => ImportOutcome::Cancelled,
        Stop::Failed(e) => ImportOutcome::Failed(ImportFailure::of_error(e)),
    }
}

/// The failure of the row at `index` of `batch`.
fn failure_at(batch: &Batch, index: usize, e: Error) -> ImportFailure {
    ImportFailure {
        row: Some(batch.first_row + index as u64),
        line: batch.lines.get(index).copied(),
        reason: e.message.clone(),
        error: Some(e),
    }
}
