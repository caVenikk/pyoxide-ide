use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use db_core::{Column, Error, Result, Row, Value, ValueKind};
use rusqlite::{Connection, OpenFlags, Rows, Statement};
use tokio::sync::oneshot;

use crate::convert::{Converter, Decl, bind_value, storage_class};
use crate::error::{is_busy, map_error, map_open_error};

pub(crate) type Reply<T> = oneshot::Sender<Result<T>>;

/// Work for the worker thread.
pub(crate) enum Request {
    Exec(ExecRequest),
    /// Read rows from the open cursor.
    Fetch {
        id: u64,
        max_rows: usize,
        reply: Reply<Page>,
    },
    /// Finalize the open cursor, if any. Without a reply it is fire and forget.
    CloseCursor {
        reply: Option<Reply<()>>,
    },
    Ping {
        reply: Reply<()>,
    },
    SetTxMode {
        manual: bool,
        reply: Reply<()>,
    },
    Commit {
        reply: Reply<()>,
    },
    Rollback {
        reply: Reply<()>,
    },
    Close {
        reply: Reply<()>,
    },
}

pub(crate) struct ExecRequest {
    /// Number of the request, see [`Shared::next_id`].
    pub id: u64,
    pub sql: String,
    pub params: Vec<Value>,
    pub max_value_bytes: usize,
    pub reply: Reply<ExecReply>,
}

pub(crate) enum ExecReply {
    Affected(u64),
    Rows {
        columns: Vec<Column>,
        /// The result had no rows at all; the statement is already finalized.
        exhausted: bool,
    },
}

pub(crate) struct Page {
    pub rows: Vec<Row>,
    /// The end of the result was reached; the cursor is finalized.
    pub exhausted: bool,
}

/// State shared between the async side and the worker thread.
pub(crate) struct Shared {
    /// Numbers the requests that can be cancelled, in the order they are sent.
    last_id: AtomicU64,
    /// Every request up to this number was cancelled before it could finish.
    cancelled_upto: AtomicU64,
    /// `true` while the worker runs a request that can be cancelled.
    running: Mutex<bool>,
    /// Read by the progress and busy handlers on the worker thread.
    cancel: Arc<AtomicBool>,
    /// Transaction mode: `true` for [`db_core::TxMode::Manual`].
    pub manual: AtomicBool,
}

impl Shared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            last_id: AtomicU64::new(0),
            cancelled_upto: AtomicU64::new(0),
            running: Mutex::new(false),
            cancel: Arc::new(AtomicBool::new(false)),
            manual: AtomicBool::new(false),
        })
    }

    /// A number for the next cancellable request.
    pub(crate) fn next_id(&self) -> u64 {
        self.last_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Cancel what was asked of the worker so far: the request that runs now
    /// is interrupted, and requests still queued fail without starting. A
    /// no-op when nothing is outstanding, so a late cancel never affects the
    /// next statement.
    pub(crate) fn cancel(&self) {
        self.cancelled_upto
            .fetch_max(self.last_id.load(Ordering::SeqCst), Ordering::SeqCst);
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if *running {
            self.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn begin(&self) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        *running = true;
        self.cancel.store(false, Ordering::SeqCst);
    }

    fn end(&self) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        *running = false;
        self.cancel.store(false, Ordering::SeqCst);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// The request was cancelled while it waited for the worker.
    fn cancelled_before_start(&self, id: u64) -> bool {
        self.cancelled_upto.load(Ordering::SeqCst) >= id
    }
}

/// Marks the span in which a cancel request takes effect.
struct OpScope<'a>(&'a Shared);

impl<'a> OpScope<'a> {
    fn begin(shared: &'a Shared) -> Self {
        shared.begin();
        Self(shared)
    }
}

impl Drop for OpScope<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}

pub(crate) struct OpenSettings {
    pub path: String,
    pub read_only: bool,
    pub create: bool,
    pub busy_timeout: Duration,
}

thread_local! {
    /// The busy handler is a plain function pointer, so it finds the cancel
    /// flag of its connection through the worker thread.
    static BUSY_CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static BUSY_LIMIT: Cell<Duration> = const { Cell::new(Duration::from_secs(5)) };
    static BUSY_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Sleeps in short steps while another connection holds a lock, and gives up
/// when the busy timeout elapsed or the statement was cancelled.
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
    std::thread::sleep(Duration::from_millis(step));
    true
}

const MAX_PROGRESS_OPS: i32 = 1000;

fn open(settings: &OpenSettings, shared: &Shared) -> Result<Connection> {
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
    let conn = Connection::open_with_flags(path, flags).map_err(|e| map_open_error(&e, path))?;

    BUSY_CANCEL.set(Some(shared.cancel.clone()));
    BUSY_LIMIT.set(settings.busy_timeout);
    let cancel = shared.cancel.clone();
    conn.busy_handler(Some(busy_handler))
        .and_then(|()| {
            conn.progress_handler(
                MAX_PROGRESS_OPS,
                Some(move || cancel.load(Ordering::Relaxed)),
            )
        })
        .and_then(|()| conn.execute_batch("PRAGMA foreign_keys = ON"))
        .map_err(|e| map_open_error(&e, path))?;
    if settings.read_only && memory {
        conn.execute_batch("PRAGMA query_only = ON")
            .map_err(|e| map_open_error(&e, path))?;
    }
    // SQLite opens files lazily; read the header now so that a file that is
    // not a database fails at connect time.
    conn.query_row("PRAGMA schema_version", [], |_| Ok(()))
        .map_err(|e| map_open_error(&e, path))?;
    Ok(conn)
}

/// Open the database on a new thread and serve requests until all senders are
/// gone or `Close` arrives.
pub(crate) fn spawn(
    settings: OpenSettings,
    shared: Arc<Shared>,
    requests: Receiver<Request>,
    ready: oneshot::Sender<Result<()>>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("db-sqlite".into())
        .spawn(move || {
            let conn = match open(&settings, &shared) {
                Ok(c) => c,
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            if ready.send(Ok(())).is_err() {
                return;
            }
            Worker {
                conn,
                requests,
                shared,
            }
            .run();
        })
        .map(|_| ())
}

enum Flow {
    Continue,
    /// Stop serving; reply once the database is closed.
    Close(Reply<()>),
    /// Run this request next; a cursor was abandoned for it.
    Run(Request),
}

struct Worker {
    conn: Connection,
    requests: Receiver<Request>,
    shared: Arc<Shared>,
}

impl Worker {
    fn run(self) {
        let mut closing = None;
        let mut next = None;
        loop {
            let request = match next.take() {
                Some(request) => request,
                None => match self.requests.recv() {
                    Ok(request) => request,
                    Err(_) => break,
                },
            };
            let flow = match request {
                Request::Exec(e) => self.exec(e),
                Request::Fetch { reply, .. } => {
                    let _ = reply.send(Err(no_cursor()));
                    Flow::Continue
                }
                Request::CloseCursor { reply } => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Ok(()));
                    }
                    Flow::Continue
                }
                other => self.simple(other),
            };
            match flow {
                Flow::Continue => {}
                Flow::Close(reply) => {
                    closing = Some(reply);
                    break;
                }
                Flow::Run(request) => next = Some(request),
            }
        }
        let Worker { conn, .. } = self;
        let result = conn.close().map_err(|(_, e)| map_error(&e));
        if let Some(reply) = closing {
            let _ = reply.send(result);
        }
    }

    /// Requests that do not touch a cursor.
    fn simple(&self, request: Request) -> Flow {
        match request {
            Request::Ping { reply } => {
                let scope = OpScope::begin(&self.shared);
                let r = self
                    .conn
                    .query_row("SELECT 1", [], |_| Ok(()))
                    .map_err(|e| self.err(&e));
                drop(scope);
                let _ = reply.send(r);
            }
            Request::SetTxMode { manual, reply } => {
                let scope = OpScope::begin(&self.shared);
                let r = if !manual && !self.conn.is_autocommit() {
                    // Leaving manual mode commits what is pending, like JDBC.
                    self.conn.execute_batch("COMMIT")
                } else {
                    Ok(())
                };
                drop(scope);
                if r.is_ok() {
                    self.shared.manual.store(manual, Ordering::SeqCst);
                }
                let _ = reply.send(r.map_err(|e| self.err(&e)));
            }
            Request::Commit { reply } => {
                let _ = reply.send(self.end_transaction("COMMIT"));
            }
            Request::Rollback { reply } => {
                let _ = reply.send(self.end_transaction("ROLLBACK"));
            }
            Request::Close { reply } => return Flow::Close(reply),
            // Handled by the callers.
            Request::Exec(e) => {
                let _ = e.reply.send(Err(Error::internal("unexpected request")));
            }
            Request::Fetch { reply, .. } => {
                let _ = reply.send(Err(no_cursor()));
            }
            Request::CloseCursor { reply } => {
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
            }
        }
        Flow::Continue
    }

    fn end_transaction(&self, statement: &str) -> Result<()> {
        if self.conn.is_autocommit() {
            return Ok(());
        }
        let scope = OpScope::begin(&self.shared);
        let r = self.conn.execute_batch(statement).map_err(|e| self.err(&e));
        drop(scope);
        r
    }

    /// Map an error, reporting a lock wait that was cut short by a cancel as
    /// a cancel rather than as a lock failure.
    fn err(&self, e: &rusqlite::Error) -> Error {
        if is_busy(e) && self.shared.cancelled() {
            return Error::cancelled().with_code("SQLITE_BUSY");
        }
        map_error(e)
    }

    fn exec(&self, request: ExecRequest) -> Flow {
        let ExecRequest {
            id,
            sql,
            params,
            max_value_bytes,
            reply,
        } = request;
        if reply.is_closed() {
            // The caller gave up while the request was queued.
            return Flow::Continue;
        }
        if self.shared.cancelled_before_start(id) {
            let _ = reply.send(Err(Error::cancelled()));
            return Flow::Continue;
        }
        let scope = OpScope::begin(&self.shared);
        let mut stmt = match self.prepare(&sql, &params) {
            Ok(s) => s,
            Err(e) => {
                drop(scope);
                let _ = reply.send(Err(e));
                return Flow::Continue;
            }
        };

        if stmt.column_count() == 0 {
            let result = self.run_without_rows(&mut stmt);
            drop(scope);
            let _ = reply.send(result.map(ExecReply::Affected));
            return Flow::Continue;
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
            converter: Converter::new(decls.clone(), max_value_bytes),
            ahead: VecDeque::new(),
            ahead_error: None,
            finished: false,
            shared: &self.shared,
        };
        // Step the first row now: errors and side effects of the statement
        // (INSERT ... RETURNING) surface at execute time, and the first row
        // tells the kind of columns that have no declared type.
        match cursor.step() {
            Ok(Some(row)) => cursor.ahead.push_back(row),
            Ok(None) => cursor.finished = true,
            Err(e) => {
                drop(scope);
                let _ = reply.send(Err(e));
                return Flow::Continue;
            }
        }
        let columns = build_columns(&names, &decls, cursor.ahead.front());
        let exhausted = cursor.finished;
        drop(scope);
        if reply
            .send(Ok(ExecReply::Rows { columns, exhausted }))
            .is_err()
            || exhausted
        {
            return Flow::Continue;
        }
        self.serve_cursor(cursor)
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
        if self.shared.manual.load(Ordering::SeqCst)
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

    /// Serve the open cursor until it ends. The async side only sends an
    /// execute once no cursor object is alive (its busy flag), so an execute
    /// arriving here means the cursor was abandoned, for instance when the
    /// call that created it was dropped before it got the cursor: it is
    /// closed and the statement runs.
    fn serve_cursor(&self, mut cursor: CursorState<'_>) -> Flow {
        loop {
            let Ok(request) = self.requests.recv() else {
                return Flow::Continue;
            };
            match request {
                Request::Fetch {
                    id,
                    max_rows,
                    reply,
                } => {
                    if reply.is_closed() {
                        // The caller dropped the call; keep the position.
                        continue;
                    }
                    if self.shared.cancelled_before_start(id) {
                        let _ = reply.send(Err(Error::cancelled()));
                        return Flow::Continue;
                    }
                    let scope = OpScope::begin(&self.shared);
                    let page = cursor.fetch(max_rows.max(1));
                    drop(scope);
                    match page {
                        Ok(page) => {
                            let exhausted = page.exhausted;
                            match reply.send(Ok(page)) {
                                Ok(()) if exhausted => return Flow::Continue,
                                Ok(()) => {}
                                Err(Ok(undelivered)) => cursor.put_back(undelivered.rows),
                                Err(Err(_)) => {}
                            }
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            return Flow::Continue;
                        }
                    }
                }
                Request::CloseCursor { reply } => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Ok(()));
                    }
                    return Flow::Continue;
                }
                Request::Exec(e) => return Flow::Run(Request::Exec(e)),
                other => {
                    if let Flow::Close(reply) = self.simple(other) {
                        return Flow::Close(reply);
                    }
                }
            }
        }
    }
}

fn no_cursor() -> Error {
    Error::query("the cursor is closed")
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
    rows: Rows<'s>,
    converter: Converter,
    /// Rows already read but not yet handed out; at most one in normal use.
    ahead: VecDeque<Row>,
    /// An error hit while reading ahead, reported by the next fetch.
    ahead_error: Option<Error>,
    /// SQLite reported the end of the rows.
    finished: bool,
    shared: &'s Shared,
}

impl CursorState<'_> {
    fn step(&mut self) -> Result<Option<Row>> {
        match self.rows.next() {
            Ok(Some(row)) => self.converter.row(row).map(Some),
            Ok(None) => Ok(None),
            Err(e) if is_busy(&e) && self.shared.cancelled() => {
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

    /// Rows that could not be delivered go back in front of the queue.
    fn put_back(&mut self, rows: Vec<Row>) {
        for row in rows.into_iter().rev() {
            self.ahead.push_front(row);
        }
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
