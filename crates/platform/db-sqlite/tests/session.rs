mod common;

use std::time::{Duration, Instant};

use common::{file_params, memory, new_file, open, rows, run, scalar, with_create};
use db_core::{
    Backend, ConnectParams, Connection, Driver, ErrorKind, ExecOptions, Outcome, TxMode, Value,
};
use db_sqlite::{OPT_BUSY_TIMEOUT_MS, SqliteDriver};

async fn count(conn: &std::sync::Arc<dyn Connection>, table: &str) -> i64 {
    scalar(conn, &format!("SELECT count(*) FROM \"{table}\""))
        .await
        .as_i64()
        .unwrap()
}

#[test]
fn driver_identity() {
    assert_eq!(SqliteDriver.backend(), Backend::Sqlite);
    let version = SqliteDriver.availability().unwrap();
    assert!(version.starts_with("SQLite 3."), "{version}");
    assert!(version.ends_with("(bundled)"), "{version}");
}

#[tokio::test]
async fn basics() {
    let conn = memory().await;
    assert_eq!(conn.backend(), Backend::Sqlite);
    assert_eq!(conn.dialect(), db_core::Dialect::Sqlite);
    conn.ping().await.unwrap();
    assert!(
        conn.server_version()
            .await
            .unwrap()
            .starts_with("SQLite 3.")
    );
    assert_eq!(conn.tx_mode().await, TxMode::Auto);
}

#[tokio::test]
async fn affected_rows() {
    let conn = memory().await;
    let opts = ExecOptions::default();
    let affected = |o: Outcome| match o {
        Outcome::Affected(n) => n,
        Outcome::Rows(_) => panic!("expected Affected"),
    };
    assert_eq!(
        affected(conn.execute("CREATE TABLE t(a)", &[], &opts).await.unwrap()),
        0
    );
    assert_eq!(
        affected(
            conn.execute("INSERT INTO t VALUES (1),(2),(3)", &[], &opts)
                .await
                .unwrap()
        ),
        3
    );
    // DDL after DML must not repeat the previous count.
    assert_eq!(
        affected(conn.execute("CREATE TABLE u(a)", &[], &opts).await.unwrap()),
        0
    );
    assert_eq!(
        affected(
            conn.execute("UPDATE t SET a = a + 1 WHERE a > 1", &[], &opts)
                .await
                .unwrap()
        ),
        2
    );
    assert_eq!(
        affected(
            conn.execute("UPDATE t SET a = 0 WHERE a > 100", &[], &opts)
                .await
                .unwrap()
        ),
        0
    );
    assert_eq!(
        affected(conn.execute("DELETE FROM t", &[], &opts).await.unwrap()),
        3
    );
    // A trigger's own changes are not counted.
    conn.execute(
        "CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO u VALUES (new.a); END",
        &[],
        &opts,
    )
    .await
    .unwrap();
    assert_eq!(
        affected(
            conn.execute("INSERT INTO t VALUES (9)", &[], &opts)
                .await
                .unwrap()
        ),
        1
    );
    assert_eq!(count(&conn, "u").await, 1);
}

#[tokio::test]
async fn statement_errors_carry_codes() {
    let conn = memory().await;
    let opts = ExecOptions::default();
    let e = conn.execute("SELEKT 1", &[], &opts).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("SQLITE_ERROR"));

    run(
        &conn,
        "CREATE TABLE p(id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL)",
    )
    .await;
    run(&conn, "CREATE TABLE c(pid INTEGER REFERENCES p(id))").await;
    run(&conn, "INSERT INTO p VALUES (1, 'a')").await;
    let e = conn
        .execute("INSERT INTO p VALUES (2, 'a')", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("SQLITE_CONSTRAINT_UNIQUE"));
    let e = conn
        .execute("INSERT INTO p VALUES (3, NULL)", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("SQLITE_CONSTRAINT_NOTNULL"));
    // Foreign keys are enforced.
    let e = conn
        .execute("INSERT INTO c VALUES (99)", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("SQLITE_CONSTRAINT_FOREIGNKEY"));
    // A failed statement leaves the connection usable.
    assert_eq!(count(&conn, "p").await, 1);
}

#[tokio::test]
async fn script_and_empty_statements_are_rejected_clearly() {
    let conn = memory().await;
    let opts = ExecOptions::default();
    let e = conn
        .execute("SELECT 1; SELECT 2", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert!(e.message.contains("one statement"), "{}", e.message);
    for sql in ["", "   ", "-- just a comment", ";"] {
        let e = conn.execute(sql, &[], &opts).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query, "{sql:?}");
    }
    // One trailing semicolon or comment is fine.
    assert_eq!(scalar(&conn, "SELECT 1;").await, Value::Int(1));
    assert_eq!(scalar(&conn, "SELECT 2 -- two").await, Value::Int(2));
}

#[tokio::test]
async fn closed_connection_reports_connection_lost() {
    let conn = memory().await;
    conn.close().await.unwrap();
    let e = conn
        .execute("SELECT 1", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConnectionLost);
    assert_eq!(
        conn.ping().await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
    // Closing twice is fine.
    conn.close().await.unwrap();
}

#[tokio::test]
async fn closing_with_an_open_cursor_works() {
    let conn = memory().await;
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000) SELECT x FROM c",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    cursor.next_page(10).await.unwrap();
    conn.close().await.unwrap();
    assert!(cursor.next_page(10).await.is_err());
}

#[tokio::test]
async fn closing_stops_a_running_statement() {
    let conn = memory().await;
    let runner = conn.clone();
    let task = tokio::spawn(async move {
        runner
            .execute(common::SLOW_COUNT, &[], &ExecOptions::default())
            .await
            .map(|_| ())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    conn.close().await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn manual_mode_ddl_is_transactional() {
    let conn = memory().await;
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    run(&conn, "CREATE TABLE t(x)").await;
    conn.rollback().await.unwrap();
    let e = conn
        .execute("SELECT * FROM t", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
}

#[tokio::test]
async fn reads_do_not_open_a_transaction_in_manual_mode() {
    let dir = tempfile::tempdir().unwrap();
    let a = new_file(dir.path(), "reads.db").await;
    let b = open(&file_params(&dir.path().join("reads.db"))).await;
    run(&a, "CREATE TABLE t(x)").await;
    a.set_tx_mode(TxMode::Manual).await.unwrap();
    assert_eq!(count(&a, "t").await, 0);
    // Another connection can write at once: no read lock is held.
    run(&b, "INSERT INTO t VALUES (1)").await;
    assert_eq!(count(&a, "t").await, 1);
}

#[tokio::test]
async fn pragma_and_vacuum_work_in_manual_mode() {
    let dir = tempfile::tempdir().unwrap();
    let a = new_file(dir.path(), "pragma.db").await;
    a.set_tx_mode(TxMode::Manual).await.unwrap();
    run(&a, "PRAGMA journal_mode = DELETE").await;
    run(&a, "VACUUM").await;
}

fn busy_params(path: &std::path::Path, ms: &str) -> ConnectParams {
    let mut p = file_params(path);
    p.options.insert(OPT_BUSY_TIMEOUT_MS.into(), ms.into());
    p
}

#[tokio::test]
async fn lock_conflicts_report_busy_after_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("busy.db");
    let a = open(&with_create(file_params(&path))).await;
    run(&a, "CREATE TABLE t(x)").await;
    a.set_tx_mode(TxMode::Manual).await.unwrap();
    run(&a, "INSERT INTO t VALUES (1)").await;

    let b = open(&busy_params(&path, "150")).await;
    let started = Instant::now();
    let e = b
        .execute("INSERT INTO t VALUES (2)", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("SQLITE_BUSY"));
    assert!(e.message.contains("lock"), "{}", e.message);
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(100) && waited < Duration::from_secs(3),
        "{waited:?}"
    );

    a.rollback().await.unwrap();
    run(&b, "INSERT INTO t VALUES (2)").await;
    assert_eq!(count(&b, "t").await, 1);
}

#[tokio::test]
async fn cancel_interrupts_a_lock_wait() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.db");
    let a = open(&with_create(file_params(&path))).await;
    run(&a, "CREATE TABLE t(x)").await;
    a.set_tx_mode(TxMode::Manual).await.unwrap();
    run(&a, "INSERT INTO t VALUES (1)").await;

    // The default busy timeout is 5 s; a cancel must not wait for it.
    let b = open(&file_params(&path)).await;
    let runner = b.clone();
    let task = tokio::spawn(async move {
        runner
            .execute("INSERT INTO t VALUES (2)", &[], &ExecOptions::default())
            .await
            .map(|_| ())
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let asked = Instant::now();
    b.cancel().await.unwrap();
    let e = task.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );

    a.rollback().await.unwrap();
    run(&b, "INSERT INTO t VALUES (3)").await;
}

#[tokio::test]
async fn timeout_interrupts_a_lock_wait() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait2.db");
    let a = open(&with_create(file_params(&path))).await;
    run(&a, "CREATE TABLE t(x)").await;
    a.set_tx_mode(TxMode::Manual).await.unwrap();
    run(&a, "INSERT INTO t VALUES (1)").await;

    let b = open(&file_params(&path)).await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_millis(200)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = b
        .execute("INSERT INTO t VALUES (2)", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn read_only_connections_reject_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro.db");
    let w = open(&with_create(file_params(&path))).await;
    run(&w, "CREATE TABLE t(x)").await;
    run(&w, "INSERT INTO t VALUES (1)").await;
    w.close().await.unwrap();

    let mut p = file_params(&path);
    p.read_only = true;
    let r = open(&p).await;
    assert_eq!(count(&r, "t").await, 1);
    for sql in [
        "INSERT INTO t VALUES (2)",
        "CREATE TABLE u(x)",
        "DELETE FROM t",
    ] {
        let e = r
            .execute(sql, &[], &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query, "{sql}");
        assert_eq!(e.code.as_deref(), Some("SQLITE_READONLY"), "{sql}");
    }
    // Manual mode on a read-only connection reads fine and rolls back cleanly.
    r.set_tx_mode(TxMode::Manual).await.unwrap();
    assert_eq!(count(&r, "t").await, 1);
    r.commit().await.unwrap();
}

#[tokio::test]
async fn read_only_memory_database_rejects_writes() {
    let mut p = common::memory_params();
    p.read_only = true;
    let r = open(&p).await;
    let e = r
        .execute("CREATE TABLE t(x)", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
}

#[tokio::test]
async fn a_missing_file_is_a_config_error_unless_creation_is_requested() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    let err = SqliteDriver
        .connect(&file_params(&path))
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind, ErrorKind::Config);
    assert!(err.message.contains("missing.db"), "{}", err.message);
    assert!(err.message.contains("sqlite.create"), "{}", err.message);
    assert!(!path.exists(), "a failed connect must not create the file");

    // Read-only never creates a file either.
    let mut ro = file_params(&path);
    ro.read_only = true;
    assert_eq!(
        SqliteDriver.connect(&ro).await.err().unwrap().kind,
        ErrorKind::Config
    );
    assert!(!path.exists());

    let conn = open(&with_create(file_params(&path))).await;
    run(&conn, "CREATE TABLE t(x)").await;
    assert!(path.exists());
}

#[tokio::test]
async fn bad_connect_parameters_are_config_errors() {
    let empty = ConnectParams::default();
    assert_eq!(
        SqliteDriver.connect(&empty).await.err().unwrap().kind,
        ErrorKind::Config
    );

    let mut bad = common::memory_params();
    bad.options
        .insert(OPT_BUSY_TIMEOUT_MS.into(), "soon".into());
    assert_eq!(
        SqliteDriver.connect(&bad).await.err().unwrap().kind,
        ErrorKind::Config
    );

    // A directory cannot be opened as a database.
    let dir = tempfile::tempdir().unwrap();
    let err = SqliteDriver
        .connect(&file_params(dir.path()))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(err.kind, ErrorKind::Config | ErrorKind::Connect),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_file_that_is_not_a_database_fails_at_connect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text.db");
    std::fs::write(
        &path,
        "this is not a database, just some text that is long enough to look like a header "
            .repeat(20),
    )
    .unwrap();
    let err = SqliteDriver
        .connect(&file_params(&path))
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind, ErrorKind::Connect);
    assert_eq!(err.code.as_deref(), Some("SQLITE_NOTADB"));
}

#[tokio::test]
async fn databases_can_be_attached() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.db");
    let conn = memory().await;
    // ATTACH creates the file; the main connection needs no create option.
    run(
        &conn,
        &format!("ATTACH DATABASE '{}' AS other", other.display()),
    )
    .await;
    run(&conn, "CREATE TABLE other.t(x)").await;
    run(&conn, "INSERT INTO other.t VALUES (1)").await;
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM other.t").await,
        Value::Int(1)
    );
}

#[tokio::test]
async fn two_connections_work_in_parallel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("parallel.db");
    let a = open(&with_create(file_params(&path))).await;
    run(&a, "CREATE TABLE t(x)").await;
    let b = open(&file_params(&path)).await;
    let (ra, rb) = tokio::join!(
        async {
            for i in 0..50 {
                a.execute(
                    "INSERT INTO t VALUES (?1)",
                    &[Value::Int(i)],
                    &ExecOptions::default(),
                )
                .await
                .unwrap();
            }
        },
        async {
            for _ in 0..50 {
                let _ = rows(&b, "SELECT count(*) FROM t", &[]).await;
            }
        }
    );
    let _ = (ra, rb);
    assert_eq!(count(&b, "t").await, 50);
}

#[tokio::test]
async fn finished_queries_hold_no_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("idle.db");
    let a = open(&with_create(file_params(&path))).await;
    run(&a, "CREATE TABLE t(x)").await;
    run(&a, "INSERT INTO t VALUES (1), (2)").await;
    // No waiting: any lock left by `a` fails the exclusive transaction.
    let probe = &open(&busy_params(&path, "0")).await;
    let exclusive = || async move {
        run(probe, "BEGIN EXCLUSIVE").await;
        run(probe, "COMMIT").await;
    };
    // A query read to the end.
    assert_eq!(rows(&a, "SELECT x FROM t", &[]).await.rows.len(), 2);
    exclusive().await;
    // A query cut short: the cursor is closed before `query` returns.
    let cut = a.query("SELECT x FROM t", &[], 1).await.unwrap();
    assert!(cut.truncated);
    exclusive().await;
    // A statement without rows.
    run(&a, "UPDATE t SET x = x + 1").await;
    exclusive().await;
}
