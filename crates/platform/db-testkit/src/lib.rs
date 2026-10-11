//! Conformance suite of the `db-core` contract (ADR 0002) for every
//! database driver.
//!
//! Each case is a plain async function that takes a [`Target`] (a
//! connection factory plus the dialect's SQL in [`Hooks`]) and panics with a
//! message when the driver breaks the contract. A case that needs something
//! the backend cannot do returns [`Ran::Skipped`]. A driver's test file runs
//! the cases with [`conformance_tests!`]:
//!
//! ```ignore
//! db_testkit::conformance_tests! {
//!     target: db_testkit::Target::new(Arc::new(MyDriver), params(), Hooks::postgres());
//!     #[ignore = "needs the postgres container"]
//!     typed_round_trip => Expect::Run;
//!     // ...
//! }
//! ```
//!
//! Cases create what they need in tables with unique names and drop them
//! again, so they can run in parallel against one database.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use db_core::{
    ConnectParams, Connection, Cursor, Driver, ErrorKind, ExecOptions, Outcome, ResultSet, TxMode,
    Value, ValueKind,
};

/// Whether a case ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    Yes,
    /// The backend cannot do what the case checks.
    Skipped(&'static str),
}

/// What a driver expects of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Run,
    Skip,
}

/// Fails when a case ran or was skipped against the driver's expectation.
pub fn check(case: &str, ran: Ran, expect: Expect) {
    match (ran, expect) {
        (Ran::Yes, Expect::Run) | (Ran::Skipped(_), Expect::Skip) => {}
        (Ran::Skipped(why), Expect::Run) => panic!("case {case} was skipped: {why}"),
        (Ran::Yes, Expect::Skip) => panic!("case {case} ran, but the driver expects a skip"),
    }
}

/// Generate one `#[tokio::test]` per case. Every case line can carry
/// attributes such as `#[ignore = "..."]`.
#[macro_export]
macro_rules! conformance_tests {
    (target: $target:expr; $( $(#[$attr:meta])* $case:ident => $expect:expr; )*) => {
        $(
            $(#[$attr])*
            #[tokio::test]
            async fn $case() {
                let target = $target;
                $crate::check(stringify!($case), $crate::$case(&target).await, $expect);
            }
        )*
    };
}

/// SQL for one dialect, see the constructors for the built-in ones.
#[derive(Clone)]
pub struct Hooks {
    /// A query returning `n` rows (`n >= 1`) of one integer column counting
    /// from 1, produced lazily: a large `n` costs nothing until it is read.
    pub rows: fn(u64) -> String,
    /// A statement that runs for at least `secs` seconds unless cancelled;
    /// `None` when the backend has no way to wait.
    pub sleep: Option<fn(u32) -> String>,
    /// Create the table `name` with the columns `id` (integer) and `label`
    /// (text).
    pub create_table: fn(&str) -> String,
    pub drop_table: fn(&str) -> String,
    /// The bind placeholder of parameter `n` (from 1).
    pub placeholder: fn(usize) -> String,
    /// A one-row query with typed values and what they read back as.
    pub probe: fn() -> Probe,
}

/// A one-row query and its expected columns.
pub struct Probe {
    pub sql: String,
    pub params: Vec<Value>,
    /// Per column: the expected kind (`None`: not checked) and value.
    pub expect: Vec<(Option<ValueKind>, Value)>,
}

const BOUND_TEXT: &str = "bound ü ✓";

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a valid date")
}

impl Hooks {
    pub fn sqlite() -> Self {
        Hooks {
            rows: |n| {
                format!(
                    "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c \
                     WHERE n < {n}) SELECT n FROM c"
                )
            },
            // SQLite has no sleep function; counting takes minutes.
            sleep: Some(|_| {
                "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                 WHERE x < 1000000000000) SELECT count(*) FROM c"
                    .to_owned()
            }),
            create_table: |name| format!("CREATE TABLE {name} (id INTEGER, label TEXT)"),
            drop_table: |name| format!("DROP TABLE IF EXISTS {name}"),
            placeholder: |n| format!("?{n}"),
            probe: || Probe {
                sql: "SELECT 42 AS i, 1.5 AS f, 'text' AS t, x'00ff' AS b, NULL AS z, ?1 AS p"
                    .into(),
                params: vec![Value::Text(BOUND_TEXT.into())],
                expect: vec![
                    (Some(ValueKind::Int), Value::Int(42)),
                    (Some(ValueKind::Float), Value::Float(1.5)),
                    (Some(ValueKind::Text), Value::Text("text".into())),
                    (Some(ValueKind::Bytes), Value::Bytes(vec![0, 255])),
                    (None, Value::Null),
                    (Some(ValueKind::Text), Value::Text(BOUND_TEXT.into())),
                ],
            },
        }
    }

    pub fn postgres() -> Self {
        Hooks {
            // A set-returning function in the select list streams its rows.
            rows: |n| format!("SELECT generate_series(1, {n})::int8 AS n"),
            sleep: Some(|secs| format!("SELECT pg_sleep({secs})")),
            create_table: |name| format!("CREATE TABLE {name} (id bigint, label text)"),
            drop_table: |name| format!("DROP TABLE IF EXISTS {name}"),
            placeholder: |n| format!("${n}"),
            probe: || Probe {
                sql: "SELECT 42::int8 AS i, 1.5::float8 AS f, 'text'::text AS t, \
                      '\\x00ff'::bytea AS b, NULL::int4 AS z, $1::text AS p, \
                      12.25::numeric(10,2) AS d, DATE '2026-01-02' AS dt, true AS bo"
                    .into(),
                params: vec![Value::Text(BOUND_TEXT.into())],
                expect: vec![
                    (Some(ValueKind::Int), Value::Int(42)),
                    (Some(ValueKind::Float), Value::Float(1.5)),
                    (Some(ValueKind::Text), Value::Text("text".into())),
                    (Some(ValueKind::Bytes), Value::Bytes(vec![0, 255])),
                    (Some(ValueKind::Int), Value::Null),
                    (Some(ValueKind::Text), Value::Text(BOUND_TEXT.into())),
                    (Some(ValueKind::Decimal), Value::Decimal("12.25".into())),
                    (Some(ValueKind::Date), Value::Date(date(2026, 1, 2))),
                    (Some(ValueKind::Bool), Value::Bool(true)),
                ],
            },
        }
    }

    /// Oracle, for both Oracle drivers.
    pub fn oracle() -> Self {
        Hooks {
            rows: |n| format!("SELECT LEVEL AS n FROM dual CONNECT BY LEVEL <= {n}"),
            sleep: Some(|secs| format!("BEGIN DBMS_SESSION.SLEEP({secs}); END;")),
            create_table: |name| {
                format!("CREATE TABLE {name} (id NUMBER(18), label VARCHAR2(200))")
            },
            drop_table: |name| format!("DROP TABLE {name} PURGE"),
            placeholder: |n| format!(":{n}"),
            probe: || Probe {
                sql: "SELECT CAST(42 AS NUMBER(10)) AS i, CAST(1.5 AS BINARY_DOUBLE) AS f, \
                      'text' AS t, HEXTORAW('00FF') AS b, CAST(NULL AS NUMBER) AS z, \
                      CAST(:1 AS VARCHAR2(100)) AS p, CAST(12.25 AS NUMBER(10,2)) AS d, \
                      DATE '2026-01-02' AS dt FROM dual"
                    .into(),
                params: vec![Value::Text(BOUND_TEXT.into())],
                expect: vec![
                    (Some(ValueKind::Int), Value::Int(42)),
                    (Some(ValueKind::Float), Value::Float(1.5)),
                    (Some(ValueKind::Text), Value::Text("text".into())),
                    (Some(ValueKind::Bytes), Value::Bytes(vec![0, 255])),
                    (None, Value::Null),
                    (Some(ValueKind::Text), Value::Text(BOUND_TEXT.into())),
                    (Some(ValueKind::Decimal), Value::Decimal("12.25".into())),
                    (
                        Some(ValueKind::Timestamp),
                        Value::Timestamp(date(2026, 1, 2).and_hms_opt(0, 0, 0).expect("midnight")),
                    ),
                ],
            },
        }
    }
}

/// Where the cases connect to: a driver, its connection parameters and the
/// dialect's SQL.
pub struct Target {
    pub driver: Arc<dyn Driver>,
    pub params: ConnectParams,
    pub hooks: Hooks,
    keep: Vec<Box<dyn Any + Send + Sync>>,
}

impl Target {
    pub fn new(driver: Arc<dyn Driver>, params: ConnectParams, hooks: Hooks) -> Self {
        Target {
            driver,
            params,
            hooks,
            keep: Vec::new(),
        }
    }

    /// Keep `value` alive as long as the target, for example the temporary
    /// directory of a database file.
    pub fn keep(mut self, value: impl Any + Send + Sync) -> Self {
        self.keep.push(Box::new(value));
        self
    }

    /// A new connection; every call opens another session to the same
    /// database.
    pub async fn connect(&self) -> Box<dyn Connection> {
        self.driver
            .connect(&self.params)
            .await
            .unwrap_or_else(|e| panic!("connect: {e:?}"))
    }

    /// A new read-only connection to the same database.
    pub async fn connect_read_only(&self) -> Box<dyn Connection> {
        let mut params = self.params.clone();
        params.read_only = true;
        self.driver
            .connect(&params)
            .await
            .unwrap_or_else(|e| panic!("connect read-only: {e:?}"))
    }

    fn rows(&self, n: u64) -> String {
        (self.hooks.rows)(n)
    }
}

/// A table name no other case (or run) uses; short enough for any backend.
fn unique(tag: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let salt = nanos ^ std::process::id().rotate_left(16);
    format!("pxkit_{tag}_{salt:08x}{n:x}")
}

fn opts() -> ExecOptions {
    ExecOptions::default()
}

fn int(v: &Value) -> i64 {
    v.as_i64()
        .unwrap_or_else(|| panic!("not an integer: {v:?}"))
}

/// Run a statement that returns no rows; the number of affected rows.
async fn exec(conn: &dyn Connection, sql: &str, params: &[Value]) -> u64 {
    match conn.execute(sql, params, &opts()).await {
        Ok(Outcome::Affected(n)) => n,
        Ok(Outcome::Rows(cursor)) => {
            let _ = cursor.close().await;
            0
        }
        Err(e) => panic!("{sql}: {e:?}"),
    }
}

/// Run a statement whose failure does not matter (cleanup).
async fn try_exec(conn: &dyn Connection, sql: &str) {
    if let Ok(Outcome::Rows(cursor)) = conn.execute(sql, &[], &opts()).await {
        let _ = cursor.close().await;
    }
}

async fn query(conn: &dyn Connection, sql: &str, params: &[Value]) -> ResultSet {
    conn.query(sql, params, 1000)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
}

async fn scalar(conn: &dyn Connection, sql: &str) -> Value {
    query(conn, sql, &[])
        .await
        .rows
        .first()
        .and_then(|row| row.first())
        .cloned()
        .unwrap_or_else(|| panic!("{sql}: no rows"))
}

async fn count(conn: &dyn Connection, table: &str) -> i64 {
    int(&scalar(conn, &format!("SELECT COUNT(*) FROM {table}")).await)
}

async fn open_rows(conn: &dyn Connection, sql: &str) -> Box<dyn Cursor> {
    match conn.execute(sql, &[], &opts()).await {
        Ok(Outcome::Rows(cursor)) => cursor,
        Ok(other) => panic!("{sql}: rows expected, got {other:?}"),
        Err(e) => panic!("{sql}: {e:?}"),
    }
}

/// The connection still runs statements.
async fn assert_usable(t: &Target, conn: &dyn Connection, after: &str) {
    match conn.query(&t.rows(1), &[], 10).await {
        Ok(rs) => assert_eq!(int(&rs.rows[0][0]), 1, "after {after}"),
        Err(e) => panic!("the connection is not usable after {after}: {e:?}"),
    }
}

/// Typed values come back with their kinds, through literals and binds.
pub async fn typed_round_trip(t: &Target) -> Ran {
    let conn = t.connect().await;
    let probe = (t.hooks.probe)();
    let rs = conn
        .query(&probe.sql, &probe.params, 10)
        .await
        .unwrap_or_else(|e| panic!("{}: {e:?}", probe.sql));
    assert_eq!(rs.rows.len(), 1, "the probe returns one row");
    assert_eq!(rs.columns.len(), probe.expect.len(), "probe columns");
    for (i, ((kind, value), column)) in probe.expect.iter().zip(&rs.columns).enumerate() {
        if let Some(kind) = kind {
            assert_eq!(column.kind, *kind, "kind of column {i} ({})", column.name);
        }
        assert_eq!(
            &rs.rows[0][i], value,
            "value of column {i} ({})",
            column.name
        );
    }

    // Bound values stored in a table and read back.
    let table = unique("rt");
    try_exec(&*conn, &(t.hooks.drop_table)(&table)).await;
    exec(&*conn, &(t.hooks.create_table)(&table), &[]).await;
    let p = t.hooks.placeholder;
    let insert = format!(
        "INSERT INTO {table} (id, label) VALUES ({}, {})",
        p(1),
        p(2)
    );
    let text = "ünïcode ✓ 'quoted'";
    assert_eq!(
        exec(&*conn, &insert, &[Value::Int(1), Value::Text(text.into())]).await,
        1,
        "rows affected by an INSERT"
    );
    exec(&*conn, &insert, &[Value::Int(2), Value::Null]).await;
    let rs = query(
        &*conn,
        &format!(
            "SELECT id, label FROM {table} WHERE id >= {} ORDER BY id",
            p(1)
        ),
        &[Value::Int(1)],
    )
    .await;
    assert_eq!(rs.rows.len(), 2);
    assert_eq!(int(&rs.rows[0][0]), 1);
    assert_eq!(rs.rows[0][1], Value::Text(text.into()));
    assert_eq!(int(&rs.rows[1][0]), 2);
    assert_eq!(rs.rows[1][1], Value::Null);
    try_exec(&*conn, &(t.hooks.drop_table)(&table)).await;
    let _ = conn.close().await;
    Ran::Yes
}

/// Pages of any size continue where the last one ended, without gaps, and
/// the end is reported.
pub async fn paging(t: &Target) -> Ran {
    const N: i64 = 12_345;
    let conn = t.connect().await;
    let mut cursor = open_rows(&*conn, &t.rows(N as u64)).await;
    assert_eq!(cursor.columns().len(), 1);
    let sizes = [1usize, 7, 500, 1000, 2500, 3];
    let mut next = 1i64;
    for size in sizes.iter().cycle() {
        let page = cursor
            .next_page(*size)
            .await
            .unwrap_or_else(|e| panic!("page after row {}: {e:?}", next - 1));
        assert!(
            page.len() <= *size,
            "a page of {} rows for {size}",
            page.len()
        );
        if page.is_empty() {
            break;
        }
        for row in &page {
            assert_eq!(int(&row[0]), next, "gap or repeat at row {next}");
            next += 1;
        }
        if cursor.is_exhausted() {
            break;
        }
        assert!(next <= N + 1, "more rows than the query returns");
    }
    assert_eq!(next, N + 1, "all rows were read");
    assert!(cursor.is_exhausted());
    assert!(
        cursor.next_page(10).await.unwrap().is_empty(),
        "an exhausted cursor returns empty pages"
    );
    cursor.close().await.unwrap();

    // Capped reads stop at the cap and say that more rows exist. (Whether
    // a result of exactly the cap counts as truncated depends on the
    // driver reading ahead, so that is not checked.)
    let rs = conn.query(&t.rows(3), &[], 3).await.unwrap();
    assert_eq!(rs.rows.len(), 3);
    let rs = conn.query(&t.rows(3), &[], 2).await.unwrap();
    assert_eq!(rs.rows.len(), 2);
    assert!(rs.truncated, "a third row exists");
    assert_usable(t, &*conn, "paging").await;
    let _ = conn.close().await;
    Ran::Yes
}

/// One open cursor per connection: other statements are `Busy`; ping,
/// cancel, commit and rollback still work, and so does the cursor.
pub async fn busy_rule(t: &Target) -> Ran {
    let conn = t.connect().await;
    let mut cursor = open_rows(&*conn, &t.rows(1000)).await;
    let first: Vec<i64> = cursor
        .next_page(10)
        .await
        .unwrap()
        .iter()
        .map(|r| int(&r[0]))
        .collect();
    assert_eq!(first, (1..=10).collect::<Vec<_>>());

    let e = conn.execute(&t.rows(1), &[], &opts()).await.unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Busy,
        "execute while a cursor is open: {e:?}"
    );
    let e = conn.query(&t.rows(1), &[], 1).await.unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Busy,
        "query while a cursor is open: {e:?}"
    );

    conn.ping().await.unwrap();
    match conn.cancel().await {
        Ok(()) => {}
        Err(e) if e.kind == ErrorKind::Unsupported => {}
        Err(e) => panic!("cancel while a cursor is open: {e:?}"),
    }
    let page = cursor.next_page(10).await.unwrap();
    assert_eq!(int(&page[0][0]), 11, "the cursor goes on where it was");
    conn.commit().await.unwrap();
    conn.rollback().await.unwrap();
    cursor.close().await.unwrap();
    assert_usable(t, &*conn, "closing the cursor").await;

    // Dropping a cursor frees the connection too.
    let mut cursor = open_rows(&*conn, &t.rows(1000)).await;
    cursor.next_page(5).await.unwrap();
    drop(cursor);
    assert_usable(t, &*conn, "dropping the cursor").await;
    let _ = conn.close().await;
    Ran::Yes
}

/// Closing or dropping a cursor early stops the server work: the next
/// statement runs at once instead of waiting for the rest of the rows.
pub async fn early_close(t: &Target) -> Ran {
    let conn = t.connect().await;
    let huge = t.rows(100_000_000);
    let mut cursor = open_rows(&*conn, &huge).await;
    assert_eq!(cursor.next_page(500).await.unwrap().len(), 500);
    assert!(!cursor.is_exhausted());
    let started = Instant::now();
    cursor.close().await.unwrap();
    assert_usable(t, &*conn, "closing a cursor early").await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "closing must not read the rest: {:?}",
        started.elapsed()
    );

    let mut cursor = open_rows(&*conn, &huge).await;
    assert_eq!(cursor.next_page(10).await.unwrap().len(), 10);
    let started = Instant::now();
    drop(cursor);
    assert_usable(t, &*conn, "dropping a cursor early").await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "dropping must not read the rest: {:?}",
        started.elapsed()
    );
    let _ = conn.close().await;
    Ran::Yes
}

/// `cancel` from another task stops a running statement; the connection
/// stays usable and an idle cancel is harmless.
pub async fn cancel(t: &Target) -> Ran {
    let Some(sleep) = t.hooks.sleep else {
        return Ran::Skipped("the backend cannot run a slow statement");
    };
    let conn: Arc<dyn Connection> = Arc::from(t.connect().await);
    match conn.cancel().await {
        Ok(()) => {}
        Err(e) if e.kind == ErrorKind::Unsupported => {
            return Ran::Skipped("the driver cannot cancel a statement");
        }
        Err(e) => panic!("cancel with nothing running: {e:?}"),
    }
    let runner = {
        let conn = conn.clone();
        let sql = sleep(30);
        tokio::spawn(async move { conn.execute(&sql, &[], &opts()).await.map(|_| ()) })
    };
    // Let the statement start on the server.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let asked = Instant::now();
    conn.cancel().await.unwrap();
    let e = runner
        .await
        .unwrap()
        .expect_err("the statement should be cancelled");
    assert_eq!(e.kind, ErrorKind::Cancelled, "{e:?}");
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?}",
        asked.elapsed()
    );
    assert_usable(t, &*conn, "a cancel").await;
    conn.cancel().await.unwrap();
    assert_usable(t, &*conn, "an idle cancel").await;
    let _ = conn.close().await;
    Ran::Yes
}

/// `ExecOptions::timeout` ends a slow statement with `Timeout`; afterwards
/// the connection works or reports `ConnectionLost`.
pub async fn timeout(t: &Target) -> Ran {
    let Some(sleep) = t.hooks.sleep else {
        return Ran::Skipped("the backend cannot run a slow statement");
    };
    let conn = t.connect().await;
    let generous = ExecOptions {
        timeout: Some(Duration::from_secs(10)),
        ..ExecOptions::default()
    };
    let rs = conn
        .query_with(&t.rows(3), &[], 10, &generous)
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 3, "a fast statement is not affected");

    let limit = ExecOptions {
        timeout: Some(Duration::from_secs(1)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = match conn.execute(&sleep(30), &[], &limit).await {
        Ok(_) => panic!("the statement should time out"),
        Err(e) => e,
    };
    assert_eq!(e.kind, ErrorKind::Timeout, "{e:?}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the timeout took {:?}",
        started.elapsed()
    );
    match conn.query(&t.rows(1), &[], 10).await {
        Ok(rs) => assert_eq!(int(&rs.rows[0][0]), 1),
        Err(e) => {
            assert_eq!(
                e.kind,
                ErrorKind::ConnectionLost,
                "after a timeout the connection works or reports ConnectionLost: {e:?}"
            );
            let fresh = t.connect().await;
            assert_usable(t, &*fresh, "reconnecting").await;
        }
    }
    let _ = conn.close().await;
    Ran::Yes
}

/// Auto mode commits every statement; manual mode keeps changes until
/// commit or rollback, and leaving it commits what is pending.
pub async fn transactions(t: &Target) -> Ran {
    let a = t.connect().await;
    let b = t.connect().await;
    let table = unique("tx");
    try_exec(&*a, &(t.hooks.drop_table)(&table)).await;
    exec(&*a, &(t.hooks.create_table)(&table), &[]).await;
    let insert = |id: i64| format!("INSERT INTO {table} (id, label) VALUES ({id}, 'x')");

    assert_eq!(a.tx_mode().await, TxMode::Auto);
    exec(&*a, &insert(1), &[]).await;
    assert_eq!(count(&*b, &table).await, 1, "auto mode commits");
    a.rollback().await.unwrap();
    assert_eq!(
        count(&*a, &table).await,
        1,
        "nothing to roll back in auto mode"
    );

    a.set_tx_mode(TxMode::Manual).await.unwrap();
    assert_eq!(a.tx_mode().await, TxMode::Manual);
    exec(&*a, &insert(2), &[]).await;
    assert_eq!(count(&*a, &table).await, 2, "own changes are visible");
    assert_eq!(count(&*b, &table).await, 1, "uncommitted changes are not");
    a.rollback().await.unwrap();
    assert_eq!(count(&*a, &table).await, 1, "rollback discards");
    exec(&*a, &insert(3), &[]).await;
    a.commit().await.unwrap();
    assert_eq!(count(&*b, &table).await, 2, "commit publishes");
    exec(&*a, &insert(4), &[]).await;
    assert_eq!(
        count(&*b, &table).await,
        2,
        "the next change is pending again"
    );
    a.rollback().await.unwrap();
    assert_eq!(count(&*a, &table).await, 2);
    // Nothing pending: both are no-ops.
    a.commit().await.unwrap();
    a.rollback().await.unwrap();

    exec(&*a, &insert(5), &[]).await;
    a.set_tx_mode(TxMode::Auto).await.unwrap();
    assert_eq!(a.tx_mode().await, TxMode::Auto);
    assert_eq!(count(&*b, &table).await, 3, "leaving manual mode commits");
    exec(&*a, &insert(6), &[]).await;
    assert_eq!(count(&*b, &table).await, 4);

    try_exec(&*a, &(t.hooks.drop_table)(&table)).await;
    let _ = b.close().await;
    let _ = a.close().await;
    Ran::Yes
}

/// A read-only connection reads but rejects writes as statement errors.
pub async fn read_only(t: &Target) -> Ran {
    let w = t.connect().await;
    let table = unique("ro");
    try_exec(&*w, &(t.hooks.drop_table)(&table)).await;
    exec(&*w, &(t.hooks.create_table)(&table), &[]).await;
    exec(
        &*w,
        &format!("INSERT INTO {table} (id, label) VALUES (1, 'x')"),
        &[],
    )
    .await;

    let ro = t.connect_read_only().await;
    assert_eq!(count(&*ro, &table).await, 1);
    let other = unique("ro");
    for sql in [
        format!("INSERT INTO {table} (id, label) VALUES (2, 'y')"),
        format!("DELETE FROM {table}"),
        (t.hooks.create_table)(&other),
    ] {
        match ro.execute(&sql, &[], &opts()).await {
            Ok(_) => panic!("a read-only connection ran {sql}"),
            Err(e) => assert_eq!(e.kind, ErrorKind::Query, "{sql}: {e:?}"),
        }
    }
    assert_eq!(count(&*ro, &table).await, 1, "nothing was written");
    let _ = ro.close().await;
    try_exec(&*w, &(t.hooks.drop_table)(&other)).await;
    try_exec(&*w, &(t.hooks.drop_table)(&table)).await;
    let _ = w.close().await;
    Ran::Yes
}

/// Statement errors are `Query` errors and leave the connection usable.
pub async fn error_kinds(t: &Target) -> Ran {
    let conn = t.connect().await;
    let missing = format!("SELECT * FROM {}", unique("no"));
    for sql in ["SELEKT 1", missing.as_str()] {
        match conn.execute(sql, &[], &opts()).await {
            Ok(_) => panic!("{sql} should fail"),
            Err(e) => assert_eq!(e.kind, ErrorKind::Query, "{sql}: {e:?}"),
        }
        assert_usable(t, &*conn, sql).await;
    }
    let _ = conn.close().await;
    Ran::Yes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_are_short_and_distinct() {
        let a = unique("tx");
        let b = unique("tx");
        assert_ne!(a, b);
        assert!(a.len() <= 30, "{a}");
        assert!(a.starts_with("pxkit_tx_"));
    }

    #[test]
    fn expectations() {
        check("c", Ran::Yes, Expect::Run);
        check("c", Ran::Skipped("why"), Expect::Skip);
        assert!(std::panic::catch_unwind(|| check("c", Ran::Skipped("why"), Expect::Run)).is_err());
        assert!(std::panic::catch_unwind(|| check("c", Ran::Yes, Expect::Skip)).is_err());
    }

    #[test]
    fn hooks_render_sql() {
        for hooks in [Hooks::sqlite(), Hooks::postgres(), Hooks::oracle()] {
            assert!((hooks.rows)(5).contains('5'));
            assert!((hooks.create_table)("t1").contains("t1"));
            assert!((hooks.drop_table)("t1").contains("t1"));
            assert!((hooks.placeholder)(2).contains('2'));
            assert!(hooks.sleep.is_some());
            let probe = (hooks.probe)();
            assert!(!probe.expect.is_empty());
        }
    }
}
