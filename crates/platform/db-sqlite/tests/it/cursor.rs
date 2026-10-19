use std::time::{Duration, Instant};

use crate::common::{SLOW_COUNT, SLOW_SECOND_ROW, memory, rows, run, scalar};
use db_core::{ErrorKind, ExecOptions, Outcome, Value};

const HUNDRED_THOUSAND: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 100000) \
     SELECT x FROM c";

async fn rows_outcome(conn: &dyn db_core::Connection, sql: &str) -> Box<dyn db_core::Cursor> {
    match conn
        .execute(sql, &[], &ExecOptions::default())
        .await
        .unwrap()
    {
        Outcome::Rows(c) => c,
        Outcome::Affected(n) => panic!("expected rows, got Affected({n})"),
    }
}

#[tokio::test]
async fn pages_100k_rows_without_gaps() {
    let conn = memory().await;
    let mut cursor = rows_outcome(&*conn, HUNDRED_THOUSAND).await;
    assert_eq!(cursor.columns().len(), 1);
    assert_eq!(cursor.columns()[0].name, "x");
    assert!(!cursor.is_exhausted());

    let mut expected = 1i64;
    let mut pages = 0;
    loop {
        let page = cursor.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 500);
        for row in &page {
            assert_eq!(row[0], Value::Int(expected), "gap before {expected}");
            expected += 1;
        }
        pages += 1;
        if cursor.is_exhausted() {
            break;
        }
    }
    assert_eq!(expected, 100_001);
    assert_eq!(pages, 200);
    // The look-ahead knew that page 200 was the last one.
    assert!(cursor.is_exhausted());
    assert!(cursor.next_page(500).await.unwrap().is_empty());
    cursor.close().await.unwrap();
    // The connection is free again.
    assert_eq!(scalar(&conn, "SELECT 1").await, Value::Int(1));
}

#[tokio::test]
async fn empty_result_is_exhausted_at_once() {
    let conn = memory().await;
    let mut cursor = rows_outcome(&*conn, "SELECT 1 AS one, 'a' AS two WHERE 0").await;
    assert!(cursor.is_exhausted());
    assert_eq!(cursor.columns().len(), 2);
    assert_eq!(cursor.columns()[1].name, "two");
    assert!(cursor.next_page(10).await.unwrap().is_empty());
    // No cursor is held, so the next statement does not need the first closed.
    assert_eq!(scalar(&conn, "SELECT 2").await, Value::Int(2));
}

#[tokio::test]
async fn one_open_cursor_per_connection() {
    let conn = memory().await;
    let mut cursor = rows_outcome(&*conn, HUNDRED_THOUSAND).await;
    let err = conn
        .execute("SELECT 1", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Busy);
    // Cancel, ping, commit, rollback and server_version still work.
    conn.ping().await.unwrap();
    conn.commit().await.unwrap();
    conn.rollback().await.unwrap();
    conn.cancel().await.unwrap();
    assert!(
        conn.server_version()
            .await
            .unwrap()
            .starts_with("SQLite 3.")
    );
    // The cursor is unharmed.
    assert_eq!(cursor.next_page(3).await.unwrap().len(), 3);
    cursor.close().await.unwrap();
    assert_eq!(scalar(&conn, "SELECT 1").await, Value::Int(1));
}

#[tokio::test]
async fn cancel_stops_a_running_statement_quickly() {
    let conn = memory().await;
    let runner = conn.clone();
    let task = tokio::spawn(async move {
        runner
            .execute(SLOW_COUNT, &[], &ExecOptions::default())
            .await
            .map(|_| ())
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let asked = Instant::now();
    conn.cancel().await.unwrap();
    let err = task.await.unwrap().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
    assert_eq!(err.code.as_deref(), Some("SQLITE_INTERRUPT"));
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    // The connection works afterwards.
    assert_eq!(scalar(&conn, "SELECT 4").await, Value::Int(4));
}

#[tokio::test]
async fn cancel_covers_a_statement_that_has_not_started_yet() {
    let conn = memory().await;
    // The request is sent when the first future is polled, the cancel right
    // after: whether or not the worker has picked the statement up by then,
    // it must not run to the end.
    let started = Instant::now();
    let opts = ExecOptions::default();
    let (result, ()) = tokio::join!(conn.execute(SLOW_COUNT, &[], &opts), async {
        conn.cancel().await.unwrap()
    });
    assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(scalar(&conn, "SELECT 10").await, Value::Int(10));
}

/// Its first row comes only after all 300,000 numbers are generated and
/// sorted (tens of milliseconds), so the statement is not answered at once.
const FIRST_ROW_LATE: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 300000) \
     SELECT x FROM c ORDER BY x DESC";

#[tokio::test]
async fn a_cursor_nobody_received_does_not_block_the_connection() {
    let conn = memory().await;
    let opts = ExecOptions::default();
    // Poll once so the request goes out, let the worker answer, then give up
    // without reading the answer: the worker holds a cursor that no object
    // refers to. A statement answered at once would be received by that
    // first poll (the worker thread can run before the poll returns), so the
    // statement needs time before its first row.
    let mut call = Box::pin(conn.execute(FIRST_ROW_LATE, &[], &opts));
    tokio::select! {
        biased;
        _ = &mut call => panic!("the statement cannot be answered yet"),
        () = std::future::ready(()) => {}
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(call);
    assert_eq!(scalar(&conn, "SELECT 11").await, Value::Int(11));
}

#[tokio::test]
async fn cancel_stops_a_slow_fetch_and_ends_the_cursor() {
    let conn = memory().await;
    let mut cursor = rows_outcome(&*conn, SLOW_SECOND_ROW).await;
    let canceller = conn.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        canceller.cancel().await.unwrap();
    });
    let asked = Instant::now();
    let err = cursor.next_page(10).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "{:?}",
        asked.elapsed()
    );
    assert!(cursor.is_exhausted());
    assert!(cursor.next_page(10).await.is_err());
    drop(cursor);
    assert_eq!(scalar(&conn, "SELECT 5").await, Value::Int(5));
}

#[tokio::test]
async fn cancel_while_idle_does_not_poison_the_next_statement() {
    let conn = memory().await;
    for _ in 0..3 {
        conn.cancel().await.unwrap();
    }
    assert_eq!(scalar(&conn, "SELECT 6").await, Value::Int(6));

    // An idle open cursor is not affected either.
    let mut cursor = rows_outcome(&*conn, HUNDRED_THOUSAND).await;
    conn.cancel().await.unwrap();
    assert_eq!(cursor.next_page(5).await.unwrap().len(), 5);
    cursor.close().await.unwrap();
}

#[tokio::test]
async fn a_dropped_call_stops_its_statement() {
    let conn = memory().await;
    let runner = conn.clone();
    let task = tokio::spawn(async move {
        let _ = runner
            .execute(SLOW_COUNT, &[], &ExecOptions::default())
            .await;
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    task.abort();
    let _ = task.await;
    let started = Instant::now();
    assert_eq!(scalar(&conn, "SELECT 7").await, Value::Int(7));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn execute_timeout_reports_timeout_and_keeps_the_connection() {
    let conn = memory().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_millis(200)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let err = conn.execute(SLOW_COUNT, &[], &opts).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(scalar(&conn, "SELECT 8").await, Value::Int(8));
}

#[tokio::test]
async fn fetch_timeout_reports_timeout() {
    let conn = memory().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_millis(200)),
        ..ExecOptions::default()
    };
    let Outcome::Rows(mut cursor) = conn.execute(SLOW_SECOND_ROW, &[], &opts).await.unwrap() else {
        panic!("expected rows");
    };
    let started = Instant::now();
    let err = cursor.next_page(10).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    drop(cursor);
    assert_eq!(scalar(&conn, "SELECT 9").await, Value::Int(9));
}

#[tokio::test]
async fn collect_reports_truncation_exactly() {
    let conn = memory().await;
    run(&conn, "CREATE TABLE t(x)").await;
    run(&conn, "INSERT INTO t VALUES (1),(2),(3)").await;
    let rs = conn.query("SELECT x FROM t", &[], 3).await.unwrap();
    assert_eq!(rs.rows.len(), 3);
    assert!(!rs.truncated, "exactly 3 rows exist");
    let rs = conn.query("SELECT x FROM t", &[], 2).await.unwrap();
    assert_eq!(rs.rows.len(), 2);
    assert!(rs.truncated);
}

#[tokio::test]
async fn insert_returning_applies_even_if_the_cursor_is_closed_early() {
    let conn = memory().await;
    run(&conn, "CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT)").await;
    let mut cursor = rows_outcome(
        &*conn,
        "INSERT INTO t(b) VALUES ('x'),('y'),('z') RETURNING a, b",
    )
    .await;
    let first = cursor.next_page(1).await.unwrap();
    assert_eq!(first, vec![vec![Value::Int(1), Value::Text("x".into())]]);
    cursor.close().await.unwrap();
    assert_eq!(scalar(&conn, "SELECT count(*) FROM t").await, Value::Int(3));
}

#[tokio::test]
async fn rows_are_converted_in_the_worker_for_wide_results() {
    let conn = memory().await;
    let rs = rows(
        &conn,
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000) \
         SELECT x, x * 1.5, 'row ' || x, x % 2 = 0 FROM c",
        &[],
    )
    .await;
    assert_eq!(rs.rows.len(), 1000);
    assert_eq!(rs.rows[9][2], Value::Text("row 10".into()));
}
