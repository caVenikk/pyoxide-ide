use crate::common;

use crate::common::{memory, rows, run};
use chrono::{DateTime, NaiveDate, NaiveTime};
use db_core::{ErrorKind, ExecOptions, Outcome, Value, ValueKind};

const UUID_TEXT: &str = "123e4567-e89b-12d3-a456-426614174000";

#[tokio::test]
async fn typed_round_trip() {
    let conn = memory().await;
    run(
        &conn,
        "CREATE TABLE t(i INTEGER, r REAL, s TEXT, b BLOB, flag BOOLEAN, d DATE, tm TIME, \
         ts TIMESTAMP, tz DATETIME, j JSON, u UUID, dec DECIMAL(10,2), n)",
    )
    .await;

    let date = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    let time = NaiveTime::from_hms_micro_opt(3, 4, 5, 600).unwrap();
    let ts = date.and_time(time);
    let tz = DateTime::parse_from_rfc3339("2026-01-02T03:04:05+03:00").unwrap();
    let params = vec![
        Value::Int(i64::MIN),
        Value::Float(2.5),
        Value::Text("héllo wörld".into()),
        Value::Bytes(vec![0, 1, 254, 255]),
        Value::Bool(true),
        Value::Date(date),
        Value::Time(time),
        Value::Timestamp(ts),
        Value::TimestampTz(tz),
        Value::Json("{\"a\": [1, 2]}".into()),
        Value::Uuid(UUID_TEXT.into()),
        Value::Decimal("12.50".into()),
        Value::Null,
    ];
    let marks = (1..=13)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let out = conn
        .execute(
            &format!("INSERT INTO t VALUES ({marks})"),
            &params,
            &ExecOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(1)));

    let rs = rows(&conn, "SELECT * FROM t", &[]).await;
    assert_eq!(rs.rows.len(), 1);
    let row = &rs.rows[0];
    assert_eq!(row[0], Value::Int(i64::MIN));
    assert_eq!(row[1], Value::Float(2.5));
    assert_eq!(row[2], Value::Text("héllo wörld".into()));
    assert_eq!(row[3], Value::Bytes(vec![0, 1, 254, 255]));
    assert_eq!(row[4], Value::Bool(true));
    assert_eq!(row[5], Value::Date(date));
    assert_eq!(row[6], Value::Time(time));
    assert_eq!(row[7], Value::Timestamp(ts));
    assert_eq!(row[8], Value::TimestampTz(tz));
    assert_eq!(row[9], Value::Json("{\"a\": [1, 2]}".into()));
    assert_eq!(row[10], Value::Uuid(UUID_TEXT.into()));
    // NUMERIC affinity turns '12.50' into the REAL 12.5; the stored class wins.
    assert_eq!(row[11], Value::Float(12.5));
    assert_eq!(row[12], Value::Null);

    let kinds: Vec<_> = rs.columns.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ValueKind::Int,
            ValueKind::Float,
            ValueKind::Text,
            ValueKind::Bytes,
            ValueKind::Bool,
            ValueKind::Date,
            ValueKind::Time,
            ValueKind::Timestamp,
            ValueKind::Timestamp,
            ValueKind::Json,
            ValueKind::Uuid,
            ValueKind::Decimal,
            // No declared type: the first row decides (NULL falls back to text).
            ValueKind::Text,
        ]
    );
    let types: Vec<_> = rs.columns.iter().map(|c| c.type_name.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "INTEGER",
            "REAL",
            "TEXT",
            "BLOB",
            "BOOLEAN",
            "DATE",
            "TIME",
            "TIMESTAMP",
            "DATETIME",
            "JSON",
            "UUID",
            "DECIMAL(10,2)",
            "NULL",
        ]
    );
}

#[tokio::test]
async fn expressions_use_storage_classes() {
    let conn = memory().await;
    let rs = rows(
        &conn,
        "SELECT 1 AS a, 2.5 AS b, 'x' AS c, x'00ff' AS d, NULL AS e",
        &[],
    )
    .await;
    let got: Vec<_> = rs
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.type_name.as_str(), c.kind))
        .collect();
    assert_eq!(
        got,
        vec![
            ("a", "INTEGER", ValueKind::Int),
            ("b", "REAL", ValueKind::Float),
            ("c", "TEXT", ValueKind::Text),
            ("d", "BLOB", ValueKind::Bytes),
            ("e", "NULL", ValueKind::Text),
        ]
    );
    assert_eq!(
        rs.rows[0],
        vec![
            Value::Int(1),
            Value::Float(2.5),
            Value::Text("x".into()),
            Value::Bytes(vec![0, 255]),
            Value::Null
        ]
    );
}

#[tokio::test]
async fn declared_type_refines_only_what_parses() {
    let conn = memory().await;
    run(
        &conn,
        "CREATE TABLE t(flag BOOLEAN, d DATE, ts TIMESTAMP, tm TIME, j JSON, u UUID, dec DECIMAL)",
    )
    .await;
    run(
        &conn,
        "INSERT INTO t VALUES (2, 'not a date', '2026-01-02', 'noon', '{broken', 'xyz', 'abc'), \
         (0, '2026-01-02', '2026-01-02T03:04:05Z', '03:04', '[1]', X'123E4567E89B12D3A456426614174000', 7)",
    )
    .await;
    let rs = rows(&conn, "SELECT * FROM t ORDER BY rowid", &[]).await;
    // Values that do not fit the declared type stay what they are.
    assert_eq!(rs.rows[0][0], Value::Int(2));
    assert_eq!(rs.rows[0][1], Value::Text("not a date".into()));
    assert_eq!(rs.rows[0][2], Value::Text("2026-01-02".into()));
    assert_eq!(rs.rows[0][3], Value::Text("noon".into()));
    assert_eq!(rs.rows[0][4], Value::Text("{broken".into()));
    assert_eq!(rs.rows[0][5], Value::Text("xyz".into()));
    assert_eq!(rs.rows[0][6], Value::Text("abc".into()));
    // Values that fit are typed.
    assert_eq!(rs.rows[1][0], Value::Bool(false));
    assert_eq!(
        rs.rows[1][1],
        Value::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap())
    );
    assert!(matches!(rs.rows[1][2], Value::TimestampTz(_)));
    assert_eq!(
        rs.rows[1][3],
        Value::Time(NaiveTime::from_hms_opt(3, 4, 0).unwrap())
    );
    assert_eq!(rs.rows[1][4], Value::Json("[1]".into()));
    assert_eq!(rs.rows[1][5], Value::Uuid(UUID_TEXT.into()));
    assert_eq!(rs.rows[1][6], Value::Int(7));
}

#[tokio::test]
async fn integers_and_epochs_stay_integers() {
    let conn = memory().await;
    run(&conn, "CREATE TABLE t(ts TIMESTAMP, d DATE)").await;
    run(&conn, "INSERT INTO t VALUES (1767323045, 2460000)").await;
    let rs = rows(&conn, "SELECT ts, d FROM t", &[]).await;
    assert_eq!(
        rs.rows[0],
        vec![Value::Int(1_767_323_045), Value::Int(2_460_000)]
    );
}

#[tokio::test]
async fn truncates_long_values() {
    let conn = memory().await;
    run(&conn, "CREATE TABLE t(b BLOB, s TEXT)").await;
    let two_mib = 2usize << 20;
    let params = [
        Value::Bytes(vec![7; two_mib]),
        Value::Text("é".repeat(two_mib / 2)),
    ];
    conn.execute(
        "INSERT INTO t VALUES (?1, ?2)",
        &params,
        &ExecOptions::default(),
    )
    .await
    .unwrap();

    let rs = rows(&conn, "SELECT b, s FROM t", &[]).await;
    let Value::Truncated { prefix, full_len } = &rs.rows[0][0] else {
        panic!("blob not truncated: {:?}", rs.rows[0][0].kind());
    };
    assert_eq!(*full_len, Some(two_mib as u64));
    assert!(matches!(&**prefix, Value::Bytes(b) if b.len() == 1 << 20));
    let Value::Truncated { prefix, full_len } = &rs.rows[0][1] else {
        panic!("text not truncated");
    };
    assert_eq!(*full_len, Some(two_mib as u64));
    assert!(matches!(&**prefix, Value::Text(s) if s.len() == 1 << 20 && s.starts_with('é')));

    // The limit never splits a character.
    run(&conn, "DELETE FROM t").await;
    conn.execute(
        "INSERT INTO t(s) VALUES (?1)",
        &[Value::Text("aéé".into())],
        &ExecOptions::default(),
    )
    .await
    .unwrap();
    let opts = ExecOptions {
        max_value_bytes: 2,
        ..ExecOptions::default()
    };
    let rs = conn
        .query_with("SELECT s FROM t", &[], 10, &opts)
        .await
        .unwrap();
    assert_eq!(
        rs.rows[0][0],
        Value::Truncated {
            prefix: Box::new(Value::Text("a".into())),
            full_len: Some(5),
        }
    );
}

#[tokio::test]
async fn non_utf8_text_is_replaced_not_dropped() {
    let conn = memory().await;
    let v = common::scalar(&conn, "SELECT CAST(x'48ff49' AS TEXT)").await;
    assert_eq!(v, Value::Text("H\u{fffd}I".into()));
}

#[tokio::test]
async fn bind_errors() {
    let conn = memory().await;
    let opts = ExecOptions::default();
    let e = conn
        .execute("SELECT ?1, ?2", &[Value::Int(1)], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    let e = conn
        .execute("SELECT 1", &[Value::Int(1)], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    let e = conn
        .execute("SELECT ?1", &[Value::Array(vec![])], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    // The same parameter can be used twice.
    let rs = rows(&conn, "SELECT ?1 + ?1", &[Value::Int(21)]).await;
    assert_eq!(rs.rows[0][0], Value::Int(42));
    // NULL binds.
    let rs = rows(&conn, "SELECT ?1 IS NULL", &[Value::Null]).await;
    assert_eq!(rs.rows[0][0], Value::Int(1));
}

#[tokio::test]
async fn null_empty_text_and_empty_blob_stay_distinct() {
    let conn = memory().await;
    run(&conn, "CREATE TABLE t(s TEXT, b BLOB, n INTEGER)").await;
    run(
        &conn,
        "INSERT INTO t VALUES (NULL, NULL, NULL), ('', X'', 0), ('NULL', X'00', 9223372036854775807)",
    )
    .await;
    let rs = rows(&conn, "SELECT * FROM t ORDER BY rowid", &[]).await;
    assert_eq!(rs.rows[0], vec![Value::Null, Value::Null, Value::Null]);
    assert_eq!(
        rs.rows[1],
        vec![
            Value::Text(String::new()),
            Value::Bytes(vec![]),
            Value::Int(0)
        ]
    );
    assert_eq!(
        rs.rows[2],
        vec![
            Value::Text("NULL".into()),
            Value::Bytes(vec![0]),
            Value::Int(i64::MAX)
        ]
    );
}

#[tokio::test]
async fn pragma_and_explain_return_rows() {
    let conn = memory().await;
    run(
        &conn,
        "CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT NOT NULL)",
    )
    .await;
    let rs = rows(&conn, "PRAGMA table_info(t)", &[]).await;
    let names: Vec<_> = rs.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["cid", "name", "type", "notnull", "dflt_value", "pk"]
    );
    assert_eq!(rs.rows.len(), 2);
    assert_eq!(rs.rows[1][1], Value::Text("b".into()));
    // Foreign keys are on for every connection.
    assert_eq!(
        common::scalar(&conn, "PRAGMA foreign_keys").await,
        Value::Int(1)
    );
    let plan = rows(&conn, "EXPLAIN QUERY PLAN SELECT * FROM t WHERE a = 1", &[]).await;
    assert!(!plan.rows.is_empty());
}
