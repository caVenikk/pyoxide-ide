use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use db_core::worker::{ExecArgs, Executed, Interrupt, Page, Rows, Session};
use db_core::{Column, Error, Result, Row, TxMode, Value, ValueKind};
use rusqlite::{Connection, OpenFlags, Statement};

use crate::convert::{Converter, Decl, bind_value, storage_class};
use crate::error::{is_busy, map_error, map_open_error};

pub(crate) struct OpenSettings {
    pub path: String,
    pub read_only: bool,
    pub create: bool,
    pub busy_timeout: Duration,
}

/// Interrupts the running statement: the progress and busy handlers read
/// the flag on the worker thread.
pub(crate) struct CancelFlag(pub Arc<AtomicBool>);

impl Interrupt for CancelFlag {
    fn interrupt(&self) -> Result<()> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

thread_local! {
    /// The busy handler is a plain function pointer, so it finds the cancel
    /// flag of its connection through the worker thread.
    static BUSY_CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static BUSY_LIMIT: Cell<Duration> = const { Cell::new(Duration::from_secs(5)) };
    static BUSY_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Waits in short steps while another connection holds a lock, and gives up
/// when the busy timeout elapsed or the statement was cancelled. A step ends
/// early when a connection of this process released its locks
/// ([`LockReleases`]).
fn busy_handler(attempt: i32) -> bool {
    let cancelled = BUSY_CANCEL.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
    });
    if cancelled {
        return false;
    }
    if attempt == 0 {
        BUSY_DEADLINE.set(Some(Instant::now() + BUSY_LIMIT.get()));
    }
    if BUSY_DEADLINE.get().is_some_and(|d| Instant::now() >= d) {
        return false;
    }
    let step = u64::try_from(attempt.clamp(0, 19)).unwrap_or(0) + 1;
    LOCK_RELEASES.wait(Duration::from_millis(step));
    true
}

/// Wakes the lock waits of this process when one of its connections lets go
/// of its locks.
///
/// SQLite keeps no queue of lock waiters: a connection that finds the file
/// locked polls through its busy handler. In rollback-journal mode a
/// connection that commits back to back holds the PENDING or EXCLUSIVE lock
/// for the whole of each commit (the journal and database syncs) and frees
/// the file only for the round trip to its next statement. A waiter that
/// sleeps between polls almost never lands in that gap, so it would wait
/// until the writer stops, longer than its busy timeout when the commits are
/// slow. Every connection therefore announces when it holds no transaction
/// any more, and a waiting busy handler retries at once instead of sleeping
/// on. Announcements are process-wide, not per file: a waiter on another
/// file wakes, tries again and waits on. Locks held by other processes are
/// still found by polling.
struct LockReleases {
    state: Mutex<Releases>,
    changed: Condvar,
}

struct Releases {
    /// Counts the announcements.
    count: u64,
    /// Busy handlers waiting now.
    waiting: usize,
}

static LOCK_RELEASES: LockReleases = LockReleases {
    state: Mutex::new(Releases {
        count: 0,
        waiting: 0,
    }),
    changed: Condvar::new(),
};

impl LockReleases {
    fn state(&self) -> MutexGuard<'_, Releases> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A connection may have released its locks: wake the waiting busy
    /// handlers so that they try again.
    fn announce(&self) {
        let mut state = self.state();
        state.count = state.count.wrapping_add(1);
        let waiting = state.waiting > 0;
        drop(state);
        if waiting {
            self.changed.notify_all();
        }
    }

    /// Wait for `max`, or less if a connection announces a release. A release
    /// announced between the failed lock attempt and this call is missed;
    /// the wait then ends by time.
    fn wait(&self, max: Duration) {
        let mut state = self.state();
        let seen = state.count;
        state.waiting += 1;
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, max, |s| s.count == seen)
            .unwrap_or_else(PoisonError::into_inner);
        state.waiting -= 1;
    }
}

const MAX_PROGRESS_OPS: i32 = 1000;

/// One SQLite connection, owned by its worker thread (see
/// [`db_core::worker`]).
pub(crate) struct SqliteSession {
    conn: Connection,
    /// Set by [`CancelFlag`]; read by the progress and busy handlers.
    cancel: Arc<AtomicBool>,
    /// Transaction mode: `true` for [`TxMode::Manual`].
    manual: Cell<bool>,
}

impl SqliteSession {
    /// Open the database. Runs on the worker thread: the busy handler finds
    /// its settings in thread-local storage.
    pub(crate) fn open(settings: &OpenSettings, cancel: Arc<AtomicBool>) -> Result<Self> {
        let path = settings.path.as_str();
        let memory = path == ":memory:";
        let uri = path.starts_with("file:");
        if !memory && !uri && !settings.create && !std::path::Path::new(path).exists() {
            return Err(Error::config(format!(
                "SQLite database file '{path}' does not exist; \
                 set the option sqlite.create to true to create it"
            )));
        }
        let mut flags = OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        if settings.read_only && !memory {
            flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
        } else {
            flags |= OpenFlags::SQLITE_OPEN_READ_WRITE;
            if settings.create || memory {
                flags |= OpenFlags::SQLITE_OPEN_CREATE;
            }
        }
        let conn =
            Connection::open_with_flags(path, flags).map_err(|e| map_open_error(&e, path))?;

        BUSY_CANCEL.set(Some(cancel.clone()));
        BUSY_LIMIT.set(settings.busy_timeout);
        let progress = cancel.clone();
        conn.busy_handler(Some(busy_handler))
            .and_then(|()| {
                conn.progress_handler(
                    MAX_PROGRESS_OPS,
                    Some(move || progress.load(Ordering::Relaxed)),
                )
            })
            .and_then(|()| conn.execute_batch("PRAGMA foreign_keys = ON"))
            .map_err(|e| map_open_error(&e, path))?;
        if settings.read_only && memory {
            conn.execute_batch("PRAGMA query_only = ON")
                .map_err(|e| map_open_error(&e, path))?;
        }
        // SQLite opens files lazily; read the header now so that a file that
        // is not a database fails at connect time.
        conn.query_row("PRAGMA schema_version", [], |_| Ok(()))
            .map_err(|e| map_open_error(&e, path))?;
        Ok(SqliteSession {
            conn,
            cancel,
            manual: Cell::new(false),
        })
    }

    fn end_transaction(&self, statement: &str) -> Result<()> {
        if self.conn.is_autocommit() {
            return Ok(());
        }
        let ended = self.conn.execute_batch(statement).map_err(|e| self.err(&e));
        self.announce_release();
        ended
    }

    /// Wake the lock waits of other connections when this one holds no
    /// transaction and so no lock any more (see [`LockReleases`]). Called
    /// when no statement is open.
    fn announce_release(&self) {
        if self.conn.is_autocommit() {
            LOCK_RELEASES.announce();
        }
    }

    /// Map an error, reporting a lock wait that was cut short by a cancel as
    /// a cancel rather than as a lock failure.
    fn err(&self, e: &rusqlite::Error) -> Error {
        if is_busy(e) && self.cancel.load(Ordering::SeqCst) {
            return Error::cancelled().with_code("SQLITE_BUSY");
        }
        map_error(e)
    }

    /// Prepare, check and bind a statement, and open the transaction first
    /// when the session is in manual mode and the statement writes.
    fn prepare<'c>(&'c self, sql: &str, params: &[Value]) -> Result<Statement<'c>> {
        if is_blank(sql) {
            return Err(Error::query("the statement is empty"));
        }
        let mut stmt = self.conn.prepare(sql).map_err(|e| self.err(&e))?;
        if stmt.parameter_count() != params.len() {
            return Err(Error::query(format!(
                "the statement has {} bind parameter(s) but {} value(s) were given",
                stmt.parameter_count(),
                params.len()
            )));
        }
        for (i, value) in params.iter().enumerate() {
            let bound = bind_value(value)?;
            stmt.raw_bind_parameter(i + 1, bound)
                .map_err(|e| self.err(&e))?;
        }
        if self.manual.get()
            && self.conn.is_autocommit()
            && !stmt.readonly()
            && !matches!(first_keyword(sql).as_str(), "VACUUM" | "PRAGMA")
        {
            self.conn.execute_batch("BEGIN").map_err(|e| self.err(&e))?;
        }
        Ok(stmt)
    }

    fn run_without_rows(&self, stmt: &mut Statement<'_>) -> Result<u64> {
        let before = self.conn.total_changes();
        let changes = stmt.raw_execute().map_err(|e| self.err(&e))?;
        // `changes` keeps the count of the last INSERT/UPDATE/DELETE, so DDL
        // after a DML statement would report stale numbers.
        Ok(if self.conn.total_changes() > before {
            changes as u64
        } else {
            0
        })
    }

    /// Run one statement, see [`Session::execute`].
    fn run(&self, args: ExecArgs, rows: Rows<'_, Self>) -> Result<Executed> {
        let mut stmt = self.prepare(&args.sql, &args.params)?;
        if stmt.column_count() == 0 {
            return self.run_without_rows(&mut stmt).map(Executed::affected);
        }

        let names: Vec<(String, Option<String>)> = stmt
            .columns()
            .iter()
            .map(|c| (c.name().to_owned(), c.decl_type().map(str::to_owned)))
            .collect();
        let decls: Vec<Decl> = names
            .iter()
            .map(|(_, d)| Decl::classify(d.as_deref()))
            .collect();
        let mut cursor = CursorState {
            rows: stmt.raw_query(),
            converter: Converter::new(decls.clone(), args.max_value_bytes),
            ahead: VecDeque::new(),
            ahead_error: None,
            finished: false,
            cancel: &self.cancel,
        };
        // Step the first row now: errors and side effects of the statement
        // (INSERT ... RETURNING) surface at execute time, and the first row
        // tells the kind of columns that have no declared type.
        match cursor.step()? {
            Some(row) => cursor.ahead.push_back(row),
            None => cursor.finished = true,
        }
        let columns = build_columns(&names, &decls, cursor.ahead.front());
        let exhausted = cursor.finished;
        Ok(rows.serve(columns, exhausted, |f| cursor.fetch(f.max_rows)))
    }
}

impl Session for SqliteSession {
    fn execute(&self, args: ExecArgs, rows: Rows<'_, Self>) -> Result<Executed> {
        let executed = self.run(args, rows);
        // The statement is closed now.
        self.announce_release();
        executed
    }

    fn ping(&self) -> Result<()> {
        self.conn
            .query_row("SELECT 1", [], |_| Ok(()))
            .map_err(|e| self.err(&e))
    }

    fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        let manual = mode == TxMode::Manual;
        if !manual && !self.conn.is_autocommit() {
            // Leaving manual mode commits what is pending, like JDBC.
            self.conn
                .execute_batch("COMMIT")
                .map_err(|e| self.err(&e))?;
            self.announce_release();
        }
        self.manual.set(manual);
        Ok(())
    }

    fn commit(&self) -> Result<()> {
        self.end_transaction("COMMIT")
    }

    fn rollback(&self) -> Result<()> {
        self.end_transaction("ROLLBACK")
    }

    fn close(self) -> Result<()> {
        let closed = self.conn.close().map_err(|(_, e)| map_error(&e));
        // Closing ends an open transaction.
        LOCK_RELEASES.announce();
        closed
    }
}

fn build_columns(
    names: &[(String, Option<String>)],
    decls: &[Decl],
    first_row: Option<&Row>,
) -> Vec<Column> {
    names
        .iter()
        .zip(decls)
        .enumerate()
        .map(|(i, ((name, declared), decl))| {
            let (type_name, kind) = match (declared, decl.kind()) {
                (Some(declared), Some(kind)) => (declared.trim().to_owned(), kind),
                _ => match first_row.and_then(|r| r.get(i)) {
                    Some(v) => {
                        let (class, kind) = storage_class(v);
                        (class.to_owned(), kind)
                    }
                    None => ("ANY".to_owned(), ValueKind::Text),
                },
            };
            Column {
                name: name.clone(),
                type_name,
                kind,
                nullable: true,
            }
        })
        .collect()
}

/// The statement being read plus the rows read ahead of the caller.
struct CursorState<'s> {
    rows: rusqlite::Rows<'s>,
    converter: Converter,
    /// Rows already read but not yet handed out; at most one in normal use.
    ahead: VecDeque<Row>,
    /// An error hit while reading ahead, reported by the next fetch.
    ahead_error: Option<Error>,
    /// SQLite reported the end of the rows.
    finished: bool,
    cancel: &'s AtomicBool,
}

impl CursorState<'_> {
    fn step(&mut self) -> Result<Option<Row>> {
        match self.rows.next() {
            Ok(Some(row)) => self.converter.row(row).map(Some),
            Ok(None) => Ok(None),
            Err(e) if is_busy(&e) && self.cancel.load(Ordering::SeqCst) => {
                Err(Error::cancelled().with_code("SQLITE_BUSY"))
            }
            Err(e) => Err(map_error(&e)),
        }
    }

    fn fetch(&mut self, max_rows: usize) -> Result<Page> {
        let mut out = Vec::with_capacity(max_rows.min(1024));
        while out.len() < max_rows {
            if let Some(row) = self.ahead.pop_front() {
                out.push(row);
                continue;
            }
            if let Some(e) = self.ahead_error.take() {
                if out.is_empty() {
                    return Err(e);
                }
                self.ahead_error = Some(e);
                break;
            }
            if self.finished {
                break;
            }
            match self.step()? {
                Some(row) => out.push(row),
                None => self.finished = true,
            }
        }
        // After a full page look one row ahead, so the caller learns that the
        // result ended without asking for an empty page.
        if out.len() == max_rows
            && self.ahead.is_empty()
            && self.ahead_error.is_none()
            && !self.finished
        {
            match self.step() {
                Ok(Some(row)) => self.ahead.push_back(row),
                Ok(None) => self.finished = true,
                Err(e) => self.ahead_error = Some(e),
            }
        }
        let exhausted = self.finished && self.ahead.is_empty() && self.ahead_error.is_none();
        Ok(Page {
            rows: out,
            exhausted,
        })
    }
}

/// Skips whitespace, `--` and `/* */` comments and `;`.
fn skip_noise(sql: &str) -> &str {
    let mut rest = sql;
    loop {
        let trimmed = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
        if let Some(after) = trimmed.strip_prefix("--") {
            rest = after.find('\n').map_or("", |i| &after[i + 1..]);
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |i| &after[i + 2..]);
        } else {
            return trimmed;
        }
    }
}

/// The statement has no SQL: only whitespace, comments and semicolons.
fn is_blank(sql: &str) -> bool {
    skip_noise(sql).is_empty()
}

/// First keyword of a statement in upper case; empty when there is none.
fn first_keyword(sql: &str) -> String {
    skip_noise(sql)
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_and_blank() {
        assert!(is_blank(""));
        assert!(is_blank("  -- nothing\n  /* more */ ; "));
        assert!(!is_blank("select 1"));
        assert_eq!(first_keyword("  -- x\n vacuum;"), "VACUUM");
        assert_eq!(first_keyword("/* c */ PRAGMA foreign_keys"), "PRAGMA");
        assert_eq!(first_keyword("(select 1)"), "");
        assert_eq!(
            first_keyword("with x as (select 1) select * from x"),
            "WITH"
        );
    }
}
