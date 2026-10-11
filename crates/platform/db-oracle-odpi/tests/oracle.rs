//! Integration tests against a real Oracle database, through an installed
//! Oracle Client. They are `#[ignore]`d: run them with
//!
//! ```text
//! docker compose up -d oracle
//! cargo test -p db-oracle-odpi -- --ignored
//! ```
//!
//! on a machine where the Oracle Client libraries can be found (see the crate
//! README). See `docs/testing-databases.md` for the connection defaults and
//! the `PYOXIDE_TEST_ORACLE_*` overrides.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use chrono::{FixedOffset, NaiveDate, TimeZone};
use db_core::{
    ConnectParams, Connection, Driver, ErrorKind, ExecOptions, Outcome, ResultSet, Secret, TxMode,
    Value, ValueKind, opt,
};
use db_oracle_odpi::OdpiDriver;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn base_params() -> ConnectParams {
    ConnectParams {
        host: env_or("PYOXIDE_TEST_ORACLE_HOST", "localhost"),
        port: env_or("PYOXIDE_TEST_ORACLE_PORT", "1521").parse().unwrap(),
        database: env_or("PYOXIDE_TEST_ORACLE_SERVICE", "FREEPDB1"),
        user: "pyoxide".into(),
        password: Secret::new(env_or("PYOXIDE_DB_PASSWORD", "pyoxide")),
        connect_timeout: Duration::from_secs(10),
        ..ConnectParams::default()
    }
}

async fn connect() -> Box<dyn Connection> {
    OdpiDriver
        .connect(&base_params())
        .await
        .expect("connect (is the Oracle Client installed and the container healthy?)")
}

/// A table name that no other test (or run) uses.
fn unique(tag: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("ODPI_{tag}_{}_{n}_{nanos}", std::process::id())
        .to_uppercase()
        .chars()
        .take(30)
        .collect()
}

async fn run(conn: &dyn Connection, sql: &str) -> Outcome {
    conn.execute(sql, &[], &ExecOptions::default())
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
}

async fn query(conn: &dyn Connection, sql: &str, params: &[Value]) -> ResultSet {
    conn.query(sql, params, 1000)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
}

async fn scalar(conn: &dyn Connection, sql: &str, params: &[Value]) -> Value {
    query(conn, sql, params).await.rows[0][0].clone()
}

async fn drop_table(conn: &dyn Connection, name: &str) {
    let _ = conn
        .execute(
            &format!("DROP TABLE {name} PURGE"),
            &[],
            &ExecOptions::default(),
        )
        .await;
}

fn col(rs: &ResultSet, name: &str) -> usize {
    rs.columns
        .iter()
        .position(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name}"))
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn connect_reports_client_and_server_version() {
    let client = OdpiDriver.availability().expect("Oracle Client installed");
    assert!(client.starts_with("Oracle Client "), "{client}");
    let conn = connect().await;
    let version = conn.server_version().await.unwrap();
    assert!(version.contains("Oracle Database"), "{version}");
    conn.ping().await.unwrap();
    assert_eq!(conn.tx_mode().await, TxMode::Auto);
    conn.close().await.unwrap();
    assert_eq!(
        conn.ping().await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn typed_values_round_trip() {
    let conn = connect().await;
    let rs = query(
        &*conn,
        "SELECT 42 AS I, \
                CAST(12345678901234567890123456789012345678 AS NUMBER(38)) AS BIG, \
                CAST(12.5 AS NUMBER(10,2)) AS FIXED, \
                CAST(7 AS NUMBER(10)) AS WHOLE, \
                1.5 AS FRAC, \
                CAST(-0.05 AS NUMBER(10,2)) AS NEG, \
                CAST(1.5 AS BINARY_DOUBLE) AS BD, \
                CAST(0.1 AS BINARY_FLOAT) AS BF, \
                'ёжик ünï ☃' AS TXT, \
                TO_CHAR(NULL) AS NUL, \
                DATE '2026-01-02' AS D, \
                TIMESTAMP '2026-01-02 03:04:05.123456' AS TS, \
                TIMESTAMP '2026-01-02 03:04:05.5 +03:30' AS TSTZ, \
                CAST(TIMESTAMP '2026-01-02 03:04:05' AS TIMESTAMP WITH LOCAL TIME ZONE) AS LTZ, \
                INTERVAL '1-2' YEAR TO MONTH AS IYM, \
                INTERVAL '3 4:5:6' DAY TO SECOND AS IDS, \
                HEXTORAW('DEADBEEF') AS RW, \
                CAST('abc' AS CHAR(5)) AS CH, \
                ROWID AS RID \
         FROM DUAL",
        &[],
    )
    .await;
    let row = &rs.rows[0];
    let v = |name: &str| row[col(&rs, name)].clone();
    let kind = |name: &str| rs.columns[col(&rs, name)].kind;

    assert_eq!(v("I"), Value::Int(42));
    assert_eq!(
        v("BIG"),
        Value::Decimal("12345678901234567890123456789012345678".into())
    );
    assert_eq!(v("FIXED"), Value::Decimal("12.50".into()));
    assert_eq!(v("WHOLE"), Value::Int(7));
    assert_eq!(kind("WHOLE"), ValueKind::Int);
    assert_eq!(kind("FIXED"), ValueKind::Decimal);
    assert_eq!(v("FRAC"), Value::Decimal("1.5".into()));
    assert_eq!(v("NEG"), Value::Decimal("-0.05".into()));
    assert_eq!(v("BD"), Value::Float(1.5));
    assert_eq!(v("BF"), Value::Float(0.1));
    assert_eq!(v("TXT"), Value::Text("ёжик ünï ☃".into()));
    assert_eq!(v("NUL"), Value::Null);
    assert_eq!(
        v("D"),
        Value::Timestamp(
            NaiveDate::from_ymd_opt(2026, 1, 2)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        )
    );
    assert_eq!(
        v("TS"),
        Value::Timestamp(
            NaiveDate::from_ymd_opt(2026, 1, 2)
                .unwrap()
                .and_hms_micro_opt(3, 4, 5, 123_456)
                .unwrap()
        )
    );
    let tz = FixedOffset::east_opt(3 * 3600 + 1800)
        .unwrap()
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 1, 2)
                .unwrap()
                .and_hms_milli_opt(3, 4, 5, 500)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(v("TSTZ"), Value::TimestampTz(tz));
    assert!(matches!(v("LTZ"), Value::TimestampTz(_)), "{:?}", v("LTZ"));
    assert_eq!(kind("LTZ"), ValueKind::TimestampTz);
    assert_eq!(v("IYM"), Value::Interval("+01-02".into()));
    assert_eq!(v("IDS"), Value::Interval("+03 04:05:06.000000".into()));
    assert_eq!(v("RW"), Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
    assert_eq!(v("CH"), Value::Text("abc  ".into()));
    assert!(matches!(
        v("RID"),
        Value::Other { ref type_name, .. } if type_name == "ROWID"
    ));
    assert_eq!(rs.columns[col(&rs, "TXT")].type_name, "VARCHAR2");
    assert_eq!(rs.columns[col(&rs, "TS")].type_name, "TIMESTAMP");
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn empty_string_is_null_in_oracle() {
    let conn = connect().await;
    assert_eq!(
        scalar(&*conn, "SELECT '' FROM DUAL", &[]).await,
        Value::Null
    );
    assert_eq!(
        scalar(&*conn, "SELECT :1 FROM DUAL", &[Value::Text(String::new())]).await,
        Value::Null
    );
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container (23ai for BOOLEAN)"]
async fn boolean_on_23ai() {
    let conn = connect().await;
    let version = conn.server_version().await.unwrap();
    if !version.contains("23") {
        eprintln!("skipped: SQL BOOLEAN needs Oracle Database 23ai ({version})");
        return;
    }
    assert_eq!(
        scalar(&*conn, "SELECT TRUE FROM DUAL", &[]).await,
        Value::Bool(true)
    );
    conn.close().await.unwrap();
}

/// A 2 MiB CLOB and BLOB, built on the server.
#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn large_lobs_are_truncated_with_their_full_length() {
    let conn = connect().await;
    let t = unique("LOB");
    drop_table(&*conn, &t).await;
    run(
        &*conn,
        &format!("CREATE TABLE {t} (id NUMBER, c CLOB, b BLOB, nc NCLOB)"),
    )
    .await;
    run(
        &*conn,
        &format!(
            "DECLARE c CLOB; b BLOB; nc NCLOB; \
             BEGIN \
               DBMS_LOB.CREATETEMPORARY(c, TRUE); \
               DBMS_LOB.CREATETEMPORARY(b, TRUE); \
               DBMS_LOB.CREATETEMPORARY(nc, TRUE); \
               FOR i IN 1..128 LOOP \
                 DBMS_LOB.WRITEAPPEND(c, 16384, RPAD('x', 16384, 'x')); \
                 DBMS_LOB.WRITEAPPEND(nc, 16384, RPAD('y', 16384, 'y')); \
               END LOOP; \
               FOR i IN 1..1024 LOOP \
                 DBMS_LOB.WRITEAPPEND(b, 2048, UTL_RAW.CAST_TO_RAW(RPAD('z', 2048, 'z'))); \
               END LOOP; \
               INSERT INTO {t} VALUES (1, c, b, nc); \
               INSERT INTO {t} VALUES (2, TO_CLOB('small ёж'), UTL_RAW.CAST_TO_RAW('small'), TO_NCLOB('ноу')); \
               INSERT INTO {t} VALUES (3, EMPTY_CLOB(), EMPTY_BLOB(), NULL); \
             END;"
        ),
    )
    .await;

    let rs = query(
        &*conn,
        &format!("SELECT id, c, b, nc FROM {t} ORDER BY id"),
        &[],
    )
    .await;
    let mib = 1usize << 20;
    let full = 2 * mib as u64;

    // Row 1: truncated at max_value_bytes, with the full length.
    match &rs.rows[0][1] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(*full_len, Some(full));
            match prefix.as_ref() {
                Value::Text(t) => assert_eq!(t.len(), mib),
                other => panic!("{other:?}"),
            }
        }
        other => panic!("CLOB: {other:?}"),
    }
    match &rs.rows[0][2] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(*full_len, Some(full));
            match prefix.as_ref() {
                Value::Bytes(b) => {
                    assert_eq!(b.len(), mib);
                    assert!(b.iter().all(|&x| x == b'z'));
                }
                other => panic!("{other:?}"),
            }
        }
        other => panic!("BLOB: {other:?}"),
    }
    assert!(
        matches!(&rs.rows[0][3], Value::Truncated { .. }),
        "NCLOB: {:?}",
        rs.rows[0][3]
    );
    assert_eq!(rs.columns[1].kind, ValueKind::Text);
    assert_eq!(rs.columns[2].kind, ValueKind::Bytes);

    // Row 2: small values arrive whole.
    assert_eq!(rs.rows[1][1], Value::Text("small ёж".into()));
    assert_eq!(rs.rows[1][2], Value::Bytes(b"small".to_vec()));
    assert_eq!(rs.rows[1][3], Value::Text("ноу".into()));
    // Row 3: empty LOBs are empty, not NULL; a NULL LOB is NULL.
    assert_eq!(rs.rows[2][1], Value::Text(String::new()));
    assert_eq!(rs.rows[2][2], Value::Bytes(Vec::new()));
    assert_eq!(rs.rows[2][3], Value::Null);

    // A smaller limit applies to every kind of value.
    let opts = ExecOptions {
        max_value_bytes: 4,
        ..ExecOptions::default()
    };
    let rs = conn
        .query_with(
            &format!("SELECT c, b, 'abcdefgh' AS s FROM {t} WHERE id = 2"),
            &[],
            10,
            &opts,
        )
        .await
        .unwrap();
    assert!(matches!(&rs.rows[0][0], Value::Truncated { .. }));
    assert!(matches!(&rs.rows[0][1], Value::Truncated { .. }));
    match &rs.rows[0][2] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(**prefix, Value::Text("abcd".into()));
            assert_eq!(*full_len, Some(8));
        }
        other => panic!("{other:?}"),
    }

    drop_table(&*conn, &t).await;
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn binds_round_trip_every_value_kind() {
    let conn = connect().await;
    let t = unique("BIND");
    drop_table(&*conn, &t).await;
    run(
        &*conn,
        &format!(
            "CREATE TABLE {t} (id NUMBER(10), name VARCHAR2(100), amount NUMBER(38,10), \
             ratio BINARY_DOUBLE, born DATE, ts TIMESTAMP(6), tstz TIMESTAMP WITH TIME ZONE, \
             data RAW(100), flag NUMBER(1), note VARCHAR2(100), big CLOB, blob BLOB)"
        ),
    )
    .await;

    let born = NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(5, 6, 7)
        .unwrap();
    let ts = NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_micro_opt(5, 6, 7, 890_123)
        .unwrap();
    let tz = FixedOffset::west_opt(5 * 3600 + 1800)
        .unwrap()
        .from_local_datetime(&ts)
        .unwrap();
    let big_text = "я".repeat(100_000);
    let big_bytes: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
    let affected = conn
        .execute(
            &format!("INSERT INTO {t} VALUES (:1, :2, :3, :4, :5, :6, :7, :8, :9, :10, :11, :12)"),
            &[
                Value::Int(1),
                Value::Text("ёжик; DROP TABLE x --".into()),
                Value::Decimal("1234567890123456789012345678.0123456789".into()),
                Value::Float(0.1),
                Value::Timestamp(born),
                Value::Timestamp(ts),
                Value::TimestampTz(tz),
                Value::Bytes(vec![0, 1, 2, 255]),
                Value::Bool(true),
                Value::Null,
                Value::Text(big_text.clone()),
                Value::Bytes(big_bytes.clone()),
            ],
            &ExecOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(affected, Outcome::Affected(1)));

    let rs = query(
        &*conn,
        &format!("SELECT * FROM {t} WHERE id = :1"),
        &[Value::Int(1)],
    )
    .await;
    assert_eq!(rs.rows.len(), 1);
    let row = &rs.rows[0];
    assert_eq!(row[0], Value::Int(1));
    assert_eq!(row[1], Value::Text("ёжик; DROP TABLE x --".into()));
    assert_eq!(
        row[2],
        Value::Decimal("1234567890123456789012345678.0123456789".into()),
        "decimals are exact"
    );
    assert_eq!(row[3], Value::Float(0.1));
    assert_eq!(row[4], Value::Timestamp(born));
    assert_eq!(row[5], Value::Timestamp(ts));
    assert_eq!(row[6], Value::TimestampTz(tz));
    assert_eq!(row[7], Value::Bytes(vec![0, 1, 2, 255]));
    assert_eq!(row[8], Value::Int(1));
    assert_eq!(row[9], Value::Null);
    assert_eq!(row[10], Value::Text(big_text));
    assert_eq!(row[11], Value::Bytes(big_bytes));

    // Binds in a WHERE clause: text, a fractional timestamp, NULL handling.
    assert_eq!(
        scalar(
            &*conn,
            &format!("SELECT COUNT(*) FROM {t} WHERE name = :1 AND ts = :2"),
            &[
                Value::Text("ёжик; DROP TABLE x --".into()),
                Value::Timestamp(ts)
            ]
        )
        .await,
        Value::Int(1)
    );

    // Bind-count mismatches are errors, not silent.
    let e = conn
        .execute(
            &format!("SELECT 1 FROM {t} WHERE id = :1"),
            &[Value::Int(1), Value::Int(2)],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    let e = conn
        .execute(
            &format!("SELECT 1 FROM {t} WHERE id = :1"),
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("ORA-01008"));

    // Unsupported value kinds are reported before anything is sent.
    let e = conn
        .execute(
            "SELECT :1 FROM DUAL",
            &[Value::Array(vec![])],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);

    drop_table(&*conn, &t).await;
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn terminators_and_blocks() {
    let conn = connect().await;
    let t = unique("TERM");
    let f = unique("FN");
    drop_table(&*conn, &t).await;
    // DDL and DML with the `;` an editor sends.
    assert!(matches!(
        run(&*conn, &format!("CREATE TABLE {t} (a NUMBER);")).await,
        Outcome::Affected(0)
    ));
    assert!(matches!(
        run(&*conn, &format!("INSERT INTO {t} VALUES (1);")).await,
        Outcome::Affected(1)
    ));
    assert!(matches!(
        run(&*conn, &format!("UPDATE {t} SET a = a + 1 WHERE a < 100;")).await,
        Outcome::Affected(1)
    ));
    // PL/SQL keeps its `;`, with or without the SQL*Plus `/` line.
    assert!(matches!(
        run(&*conn, "BEGIN NULL; END;").await,
        Outcome::Affected(_)
    ));
    assert!(matches!(
        run(&*conn, "BEGIN NULL; END;\n/").await,
        Outcome::Affected(_)
    ));
    assert!(matches!(
        run(
            &*conn,
            &format!("CREATE OR REPLACE FUNCTION {f} RETURN NUMBER IS BEGIN RETURN 1; END;")
        )
        .await,
        Outcome::Affected(_)
    ));
    assert_eq!(
        scalar(&*conn, &format!("SELECT {f} FROM DUAL;"), &[]).await,
        Value::Int(1)
    );
    run(&*conn, &format!("DROP FUNCTION {f}")).await;
    drop_table(&*conn, &t).await;
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn pages_of_100k_rows_are_continuous() {
    let conn = connect().await;
    let opts = ExecOptions {
        fetch_size: 500,
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT LEVEL AS n, 'row ' || LEVEL AS label FROM DUAL CONNECT BY LEVEL <= 100000",
            &[],
            &opts,
        )
        .await
        .unwrap()
    else {
        panic!("rows expected");
    };
    assert_eq!(cursor.columns().len(), 2);
    assert_eq!(cursor.columns()[0].name, "N");
    let mut expected = 1i64;
    let mut first_page = None;
    loop {
        let page = cursor.next_page(500).await.unwrap();
        first_page.get_or_insert_with(|| started.elapsed());
        if page.is_empty() {
            break;
        }
        for row in &page {
            assert_eq!(row[0].as_i64(), Some(expected));
            assert_eq!(row[1], Value::Text(format!("row {expected}")));
            expected += 1;
        }
        if cursor.is_exhausted() {
            break;
        }
    }
    assert_eq!(expected, 100_001);
    assert!(cursor.is_exhausted());
    assert!(cursor.next_page(500).await.unwrap().is_empty());
    cursor.close().await.unwrap();
    // The first page must not wait for the other 99,500 rows.
    assert!(first_page.unwrap() < Duration::from_secs(5));
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container (12c+ for DBMS_SESSION.SLEEP)"]
async fn cancel_from_another_task_ends_a_sleeping_statement() {
    let conn: Arc<dyn Connection> = Arc::from(connect().await);
    let runner = {
        let conn = conn.clone();
        tokio::spawn(async move {
            conn.execute(
                "BEGIN DBMS_SESSION.SLEEP(30); END;",
                &[],
                &ExecOptions::default(),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    let started = Instant::now();
    conn.cancel().await.unwrap();
    let e = runner.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-01013"));
    assert!(started.elapsed() < Duration::from_secs(10));
    // The connection still works, and a cancel with nothing running is harmless.
    conn.cancel().await.unwrap();
    assert_eq!(
        scalar(&*conn, "SELECT 3 FROM DUAL", &[]).await.as_i64(),
        Some(3)
    );
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container (12c+ for WITH FUNCTION and DBMS_SESSION.SLEEP)"]
async fn cancel_interrupts_a_fetch_in_progress() {
    let conn: Arc<dyn Connection> = Arc::from(connect().await);
    // The first page is quick; the second stalls on row 150.
    let sql = "WITH FUNCTION slow(n NUMBER) RETURN NUMBER IS \
               BEGIN IF n = 150 THEN DBMS_SESSION.SLEEP(30); END IF; RETURN n; END; \
               SELECT slow(LEVEL) AS n FROM DUAL CONNECT BY LEVEL <= 1000";
    let opts = ExecOptions {
        fetch_size: 100,
        ..ExecOptions::default()
    };
    let Outcome::Rows(mut cursor) = conn.execute(sql, &[], &opts).await.unwrap() else {
        panic!("rows expected");
    };
    assert_eq!(cursor.next_page(100).await.unwrap().len(), 100);
    let fetcher = tokio::spawn(async move {
        let result = cursor.next_page(100).await;
        (result, cursor)
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    let started = Instant::now();
    conn.cancel().await.unwrap();
    let (result, mut cursor) = fetcher.await.unwrap();
    let e = result.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled, "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    // The cursor is finished; the connection is not.
    assert_eq!(cursor.next_page(100).await.unwrap_err(), e);
    drop(cursor);
    assert_eq!(
        scalar(&*conn, "SELECT 6 FROM DUAL", &[]).await.as_i64(),
        Some(6)
    );
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container (12c+ for DBMS_SESSION.SLEEP)"]
async fn statement_timeout_reports_timeout() {
    let conn = connect().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_secs(1)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = conn
        .execute("BEGIN DBMS_SESSION.SLEEP(30); END;", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout, "{e:?}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    // The client cleans the connection up after a call timeout.
    conn.ping().await.unwrap();
    assert_eq!(
        scalar(&*conn, "SELECT 4 FROM DUAL", &[]).await.as_i64(),
        Some(4)
    );
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn read_only_connection_runs_queries_only() {
    let setup = connect().await;
    let t = unique("RO");
    drop_table(&*setup, &t).await;
    run(&*setup, &format!("CREATE TABLE {t} (n NUMBER)")).await;
    run(&*setup, &format!("INSERT INTO {t} VALUES (1)")).await;

    let mut p = base_params();
    p.read_only = true;
    let ro = OdpiDriver.connect(&p).await.unwrap();
    assert_eq!(
        scalar(&*ro, &format!("SELECT COUNT(*) FROM {t}"), &[])
            .await
            .as_i64(),
        Some(1)
    );
    for sql in [
        format!("INSERT INTO {t} VALUES (2)"),
        format!("DELETE FROM {t}"),
        format!("DROP TABLE {t}"),
        "BEGIN NULL; END;".to_string(),
    ] {
        let e = ro
            .execute(&sql, &[], &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query, "{sql}");
        assert!(e.message.contains("read-only"), "{}", e.message);
    }
    // Manual mode runs inside SET TRANSACTION READ ONLY.
    ro.set_tx_mode(TxMode::Manual).await.unwrap();
    assert_eq!(
        scalar(&*ro, &format!("SELECT COUNT(*) FROM {t}"), &[])
            .await
            .as_i64(),
        Some(1)
    );
    ro.commit().await.unwrap();
    assert_eq!(
        scalar(&*ro, &format!("SELECT COUNT(*) FROM {t}"), &[])
            .await
            .as_i64(),
        Some(1)
    );
    ro.close().await.unwrap();

    drop_table(&*setup, &t).await;
    setup.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn server_errors_are_classified() {
    let conn = connect().await;
    let kind_code = |e: db_core::Error| (e.kind, e.code.unwrap_or_default());
    let e = conn
        .execute("SELEC 1 FROM DUAL", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    let e = conn
        .execute(
            "SELECT * FROM no_such_table_odpi",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(kind_code(e), (ErrorKind::Query, "ORA-00942".to_string()));
    // Raised while the first rows are fetched, whichever call that is.
    let e = conn
        .query("SELECT 1/0 FROM DUAL", &[], 10)
        .await
        .unwrap_err();
    assert_eq!(kind_code(e), (ErrorKind::Query, "ORA-01476".to_string()));
    // An error leaves the connection usable.
    assert_eq!(
        scalar(&*conn, "SELECT 5 FROM DUAL", &[]).await.as_i64(),
        Some(5)
    );
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn wrong_password_is_an_auth_error_without_the_password() {
    let mut p = base_params();
    p.password = Secret::new("definitely-wrong-password");
    let e = match OdpiDriver.connect(&p).await {
        Ok(_) => panic!("the login must fail"),
        Err(e) => e,
    };
    assert_eq!(e.kind, ErrorKind::Auth, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-01017"));
    assert!(!e.message.contains("definitely-wrong-password"));
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn unknown_service_is_a_connect_error() {
    let mut p = base_params();
    p.database = "NO_SUCH_SERVICE".into();
    let e = match OdpiDriver.connect(&p).await {
        Ok(_) => panic!("the service does not exist"),
        Err(e) => e,
    };
    assert_eq!(e.kind, ErrorKind::Connect, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-12514"));
}

#[tokio::test]
#[ignore = "needs an Oracle Client"]
async fn closed_port_is_a_network_error() {
    let mut p = base_params();
    p.host = "127.0.0.1".into();
    p.port = 1;
    p.connect_timeout = Duration::from_secs(5);
    let started = Instant::now();
    let e = match OdpiDriver.connect(&p).await {
        Ok(_) => panic!("nothing listens on port 1"),
        Err(e) => e,
    };
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-12541"));
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[tokio::test]
#[ignore = "needs an Oracle Client and the oracle container"]
async fn connect_string_options_reach_the_client() {
    // A SID-style descriptor is not available on Oracle Free; the Easy Connect
    // string in `oracle.dsn` is used verbatim instead.
    let base = base_params();
    let mut p = base.clone();
    p.host = "ignored.invalid".into();
    p.options.insert(
        opt::ORACLE_DSN.into(),
        format!("{}:{}/{}", base.host, base.port, base.database),
    );
    let conn = OdpiDriver.connect(&p).await.unwrap();
    conn.ping().await.unwrap();
    conn.close().await.unwrap();

    // A parameter-built descriptor with a connect timeout.
    let mut p = base_params();
    p.connect_timeout = Duration::from_secs(5);
    let conn = OdpiDriver.connect(&p).await.unwrap();
    conn.ping().await.unwrap();
    conn.close().await.unwrap();
}

/// Prints the numbers `replica/perf.md` records for the database area:
/// connect time, first page, and rows per second for 100,000 rows.
#[tokio::test]
#[ignore = "benchmark: needs an Oracle Client and the oracle container"]
async fn bench_fetch_100k() {
    let t0 = Instant::now();
    let conn = connect().await;
    let connect_time = t0.elapsed();

    let t1 = Instant::now();
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT LEVEL AS id, 'name ' || LEVEL AS name, LEVEL * 1.5 AS amount, \
                    SYSDATE + LEVEL AS day \
             FROM DUAL CONNECT BY LEVEL <= 100000",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("rows expected");
    };
    let first = cursor.next_page(500).await.unwrap();
    let first_page = t1.elapsed();
    assert_eq!(first.len(), 500);

    let t2 = Instant::now();
    let mut rows = first.len();
    loop {
        let page = cursor.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        rows += page.len();
    }
    let rest = t2.elapsed();
    assert_eq!(rows, 100_000);
    cursor.close().await.unwrap();

    let total = t1.elapsed();
    println!("oracle-odpi connect:     {connect_time:?}");
    println!("oracle-odpi first page:  {first_page:?} (500 rows, includes execute)");
    println!(
        "oracle-odpi remaining:   {rest:?} ({} rows)",
        rows - first.len()
    );
    println!(
        "oracle-odpi throughput:  {:.0} rows/s over the whole result ({total:?})",
        rows as f64 / total.as_secs_f64()
    );
    conn.close().await.unwrap();
}
