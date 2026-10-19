//! Integration tests against Oracle Database Free (`docker compose up -d
//! oracle`, then `cargo test -p db-oracle-thin -- --ignored`). Host, port
//! and service come from PYOXIDE_TEST_ORACLE_HOST/PORT/SERVICE.

use std::time::{Duration, Instant};

use crate::common::*;
use chrono::{DateTime, NaiveDate};
use db_core::{Connection, Driver, ErrorKind, ExecOptions, Outcome, TxMode, Value, ValueKind, opt};
use db_oracle_thin::{SDU_OPTION, ThinConnection, ThinDriver};

fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, nanos: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_nano_opt(h, mi, s, nanos)
        .unwrap()
}

async fn rows(c: &dyn Connection, sql: &str) -> Vec<Vec<Value>> {
    query(c, sql, &[]).await.rows
}

async fn single(c: &dyn Connection, sql: &str, params: &[Value]) -> Value {
    query(c, sql, params).await.rows.remove(0).remove(0)
}

#[tokio::test]
#[ignore]
async fn connects_and_reports_version() {
    let c = connect().await;
    let v = c.server_version().await.unwrap();
    assert!(v.contains("Oracle"), "{v}");
    assert!(!v.contains('\n'));
    c.ping().await.unwrap();
    assert_eq!(c.tx_mode().await, TxMode::Auto);
    c.close().await.unwrap();
    assert_eq!(c.ping().await.unwrap_err().kind, ErrorKind::ConnectionLost);
}

/// The query `pyoxide-db diagnose` uses.
#[tokio::test]
#[ignore]
async fn diagnose_probe_types() {
    let c = connect().await;
    let rs = query(
        &*c,
        "SELECT 42, 12345678901234567890.0123456789, CAST(1.5 AS BINARY_DOUBLE), 'Grüße', \
         DATE '2026-01-02', TIMESTAMP '2026-01-02 03:04:05.123456', \
         TIMESTAMP '2026-01-02 03:04:05.5 +03:00', HEXTORAW('DEADBEEF'), \
         INTERVAL '1-2' YEAR TO MONTH, NULL, INTERVAL '3 04:05:06.789' DAY TO SECOND FROM dual",
        &[],
    )
    .await;
    let row = &rs.rows[0];
    assert_eq!(row[0], Value::Int(42));
    assert_eq!(
        row[1],
        Value::Decimal("12345678901234567890.0123456789".into())
    );
    assert_eq!(row[2], Value::Float(1.5));
    assert_eq!(row[3], Value::Text("Grüße".into()));
    assert_eq!(row[4], Value::Timestamp(ts(2026, 1, 2, 0, 0, 0, 0)));
    assert_eq!(
        row[5],
        Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 123_456_000))
    );
    assert_eq!(
        row[6],
        Value::TimestampTz(DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5+03:00").unwrap())
    );
    assert_eq!(row[6].to_string(), "2026-01-02 03:04:05.500 +03:00");
    assert_eq!(row[7], Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]));
    assert_eq!(row[8], Value::Interval("+01-02".into()));
    assert_eq!(row[9], Value::Null);
    assert_eq!(row[10], Value::Interval("+03 04:05:06.789000".into()));
    let kinds: Vec<_> = rs.columns.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ValueKind::Decimal,
            ValueKind::Decimal,
            ValueKind::Float,
            ValueKind::Text,
            ValueKind::Timestamp,
            ValueKind::Timestamp,
            ValueKind::TimestampTz,
            ValueKind::Bytes,
            ValueKind::Interval,
            ValueKind::Text,
            ValueKind::Interval,
        ]
    );
}

#[tokio::test]
#[ignore]
async fn null_rows() {
    let c = connect().await;
    assert_eq!(
        rows(&*c, "SELECT NULL AS v FROM dual").await,
        vec![vec![Value::Null]]
    );
    assert_eq!(
        rows(&*c, "SELECT NULL, NULL FROM dual").await,
        vec![vec![Value::Null, Value::Null]]
    );
    assert_eq!(
        rows(&*c, "SELECT CAST(NULL AS NUMBER) FROM dual").await,
        vec![vec![Value::Null]]
    );
    assert_eq!(
        rows(
            &*c,
            "SELECT '' AS e, 1 AS n FROM dual CONNECT BY LEVEL <= 3"
        )
        .await
        .len(),
        3
    );
}

#[tokio::test]
#[ignore]
async fn typed_round_trip() {
    let c = connect().await;
    let p = prefix("types");
    let t = format!("{p}T");
    try_exec(&*c, &format!("DROP TABLE {t} PURGE")).await;
    exec(
        &*c,
        &format!(
            "CREATE TABLE {t} (id NUMBER(10) PRIMARY KEY, n38 NUMBER(38), ndec NUMBER(12,2), \
             nfree NUMBER, bd BINARY_DOUBLE, bf BINARY_FLOAT, d DATE, ts9 TIMESTAMP(9), \
             tstz TIMESTAMP(6) WITH TIME ZONE, tsltz TIMESTAMP WITH LOCAL TIME ZONE, \
             iym INTERVAL YEAR(2) TO MONTH, ids INTERVAL DAY(2) TO SECOND(6), \
             vc VARCHAR2(100 CHAR), ch CHAR(5), nvc NVARCHAR2(100), nch NCHAR(4), r RAW(16), \
             cl CLOB, bl BLOB, ncl NCLOB)"
        ),
        &[],
    )
    .await;
    let unicode = "Grüße 中文 Привет 😀";
    let n = exec(
        &*c,
        &format!(
            "INSERT INTO {t} (id, n38, ndec, nfree, bd, bf, d, ts9, tstz, tsltz, iym, ids, vc, ch, \
             nvc, nch, r, cl, bl, ncl) VALUES (:1, :2, :3, :4, :5, :6, :7, :8, :9, \
             TIMESTAMP '2026-01-02 03:04:05 +00:00', INTERVAL '1-2' YEAR TO MONTH, \
             INTERVAL '-3 04:05:06.789' DAY TO SECOND, :10, :11, :12, :13, :14, \
             'small clob', HEXTORAW('0102'), TO_NCLOB('Grüße'))"
        ),
        &[
            Value::Int(1),
            Value::Decimal("99999999999999999999999999999999999999".into()),
            Value::Decimal("-12.5".into()),
            Value::Decimal("-0.000123".into()),
            Value::Float(-1.5e300),
            Value::Float(2.25),
            Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 0)),
            Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 123_456_789)),
            Value::TimestampTz(
                DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5+03:30").unwrap(),
            ),
            Value::Text("Grüße".into()),
            Value::Text("ab".into()),
            Value::Text(unicode.into()),
            Value::Text("Ωx".into()),
            Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
        ],
    )
    .await;
    assert_eq!(n, 1);
    exec(
        &*c,
        &format!("INSERT INTO {t} (id, vc) VALUES (2, :1)"),
        &[Value::Text(String::new())],
    )
    .await;

    let expect_scalar = |row: &[Value]| {
        assert_eq!(row[0], Value::Int(1));
        assert_eq!(
            row[1],
            Value::Decimal("99999999999999999999999999999999999999".into())
        );
        assert_eq!(row[2], Value::Decimal("-12.5".into()));
        assert_eq!(row[3], Value::Decimal("-0.000123".into()));
        assert_eq!(row[4], Value::Float(-1.5e300));
        assert_eq!(row[5], Value::Float(2.25));
        assert_eq!(row[6], Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 0)));
        assert_eq!(
            row[7],
            Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 123_456_789))
        );
        assert_eq!(
            row[8],
            Value::TimestampTz(
                DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5+03:30").unwrap()
            )
        );
        assert!(matches!(row[9], Value::Timestamp(_)), "{:?}", row[9]);
        assert_eq!(row[10], Value::Interval("+01-02".into()));
        assert_eq!(row[11], Value::Interval("-03 04:05:06.789000".into()));
        assert_eq!(row[12], Value::Text("Grüße".into()));
        assert_eq!(row[13], Value::Text("ab   ".into()));
        assert_eq!(row[14], Value::Text(unicode.into()));
        assert_eq!(row[15], Value::Text("Ωx  ".into()));
        assert_eq!(row[16], Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]));
    };
    let cols = "id, n38, ndec, nfree, bd, bf, d, ts9, tstz, tsltz, iym, ids, vc, ch, nvc, nch, r";

    // REF CURSOR path (no LOB columns).
    let rs = query(
        &*c,
        &format!("SELECT {cols}, ROWID FROM {t} ORDER BY id"),
        &[],
    )
    .await;
    expect_scalar(&rs.rows[0]);
    assert!(
        matches!(&rs.rows[0][17], Value::Other { type_name, text } if type_name == "ROWID" && text.len() == 18)
    );
    assert!(
        rs.rows[1][1..17].iter().all(Value::is_null),
        "{:?}",
        rs.rows[1]
    );
    let types: Vec<_> = rs
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.type_name.as_str(), c.kind))
        .collect();
    assert_eq!(types[1], ("N38", "NUMBER", ValueKind::Decimal));
    assert_eq!(types[0], ("ID", "NUMBER", ValueKind::Int));
    assert_eq!(types[4], ("BD", "BINARY_DOUBLE", ValueKind::Float));
    assert_eq!(
        types[8],
        ("TSTZ", "TIMESTAMP WITH TIME ZONE", ValueKind::TimestampTz)
    );
    assert_eq!(
        types[11],
        ("IDS", "INTERVAL DAY TO SECOND", ValueKind::Interval)
    );
    assert_eq!(types[14], ("NVC", "NVARCHAR2", ValueKind::Text));
    assert_eq!(types[17], ("ROWID", "ROWID", ValueKind::Other));

    // The session time zone renders TIMESTAMP WITH LOCAL TIME ZONE.
    let local = single(
        &*c,
        &format!("SELECT TO_CHAR(tsltz, 'YYYY-MM-DD HH24:MI:SS') FROM {t} WHERE id = 1"),
        &[],
    )
    .await;
    assert_eq!(rs.rows[0][9].to_string()[..19], local.to_string());

    // LOB path (column defines through oracle-rs), with the same conversions.
    let rs = query(
        &*c,
        &format!("SELECT {cols}, cl, bl, ncl FROM {t} ORDER BY id"),
        &[],
    )
    .await;
    expect_scalar(&rs.rows[0]);
    assert_eq!(rs.rows[0][17], Value::Text("small clob".into()));
    assert_eq!(rs.rows[0][18], Value::Bytes(vec![1, 2]));
    assert_eq!(rs.rows[0][19], Value::Text("Grüße".into()));
    assert_eq!(rs.columns[19].type_name, "NCLOB");
    assert!(rs.rows[1][17..].iter().all(Value::is_null));

    exec(&*c, &format!("DROP TABLE {t} PURGE"), &[]).await;
}

#[tokio::test]
#[ignore]
async fn newer_types_on_23ai() {
    let c = connect().await;
    let v = c.server_version().await.unwrap();
    if !v.contains(" 23.") && !v.contains("23ai") && !v.contains("26ai") {
        eprintln!("skipped: BOOLEAN/JSON/VECTOR need 23ai ({v})");
        return;
    }
    let rs = query(&*c, "SELECT TRUE t, FALSE f, CAST(NULL AS BOOLEAN) n, JSON('{\"a\":[1,2]}') j, TO_VECTOR('[1,2,3]') v FROM dual", &[]).await;
    assert_eq!(rs.rows[0][0], Value::Bool(true));
    assert_eq!(rs.rows[0][1], Value::Bool(false));
    assert_eq!(rs.rows[0][2], Value::Null);
    assert_eq!(rs.rows[0][3], Value::Json("{\"a\":[1,2]}".into()));
    assert_eq!(
        rs.rows[0][4],
        Value::Other {
            type_name: "VECTOR".into(),
            text: "[1,2,3]".into()
        }
    );
    assert_eq!(rs.columns[0].kind, ValueKind::Bool);
    assert_eq!(rs.columns[3].kind, ValueKind::Json);
}

#[tokio::test]
#[ignore]
async fn binds_of_each_kind() {
    let c = connect().await;
    let rs = query(
        &*c,
        "SELECT :1 AS n, :2 AS i, :3 AS f, :4 AS dec, :5 AS t, :6 AS b, :7 AS d, :8 AS ts, :9 AS tz, :10 AS bo FROM dual",
        &[
            Value::Null,
            Value::Int(-9_007_199_254_740_993),
            Value::Float(0.1),
            Value::Decimal("123.456".into()),
            Value::Text("it's".into()),
            Value::Bytes(vec![0, 255]),
            Value::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
            Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 123_456_789)),
            Value::TimestampTz(DateTime::parse_from_rfc3339("2026-01-02T03:04:05-08:00").unwrap()),
            Value::Bool(true),
        ],
    )
    .await;
    let r = &rs.rows[0];
    assert_eq!(r[0], Value::Null);
    assert_eq!(r[1], Value::Int(-9_007_199_254_740_993));
    assert_eq!(r[2], Value::Float(0.1));
    assert_eq!(r[3], Value::Decimal("123.456".into()));
    assert_eq!(r[4], Value::Text("it's".into()));
    assert_eq!(r[5], Value::Bytes(vec![0, 255]));
    assert_eq!(r[6], Value::Timestamp(ts(2026, 1, 2, 0, 0, 0, 0)));
    assert_eq!(r[7], Value::Timestamp(ts(2026, 1, 2, 3, 4, 5, 123_456_789)));
    assert_eq!(
        r[8],
        Value::TimestampTz(DateTime::parse_from_rfc3339("2026-01-02T03:04:05-08:00").unwrap())
    );
    assert_eq!(r[9], Value::Int(1));

    // Repeated and named placeholders.
    assert_eq!(
        single(&*c, "SELECT :1 + :1 FROM dual", &[Value::Int(2)]).await,
        Value::Int(4)
    );
    assert_eq!(
        single(
            &*c,
            "SELECT :b || :a || :b FROM dual",
            &[Value::Text("x".into()), Value::Text("y".into())]
        )
        .await,
        Value::Text("xyx".into())
    );
    // Text over 4000 bytes in a query travels as a literal; in DML as a
    // temporary LOB.
    let long = "é'".repeat(3000);
    assert_eq!(
        single(
            &*c,
            "SELECT LENGTH(:1) FROM dual",
            &[Value::Text(long.clone())]
        )
        .await,
        Value::Int(6000)
    );
    let e = c
        .query(
            "SELECT DBMS_LOB.GETLENGTH(:1) FROM dual",
            &[Value::Bytes(vec![7; 4000])],
            1,
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    let t = format!("{}T", prefix("binds"));
    exec(
        &*c,
        &format!("CREATE TABLE {t} (c CLOB, b BLOB, v VARCHAR2(4000))"),
        &[],
    )
    .await;
    exec(
        &*c,
        &format!("INSERT INTO {t} VALUES (:1, :2, :3)"),
        &[
            Value::Text("c".repeat(20_000)),
            Value::Bytes(vec![7; 10_000]),
            Value::Text("v".repeat(4000)),
        ],
    )
    .await;
    let rs = query(
        &*c,
        &format!("SELECT LENGTH(c), DBMS_LOB.GETLENGTH(b), LENGTH(v) FROM {t}"),
        &[],
    )
    .await;
    assert_eq!(
        rs.rows[0],
        vec![Value::Int(20_000), Value::Int(10_000), Value::Int(4000)]
    );
    // Requests are limited to one network packet.
    let e = c
        .execute(
            &format!("INSERT INTO {t} (c) VALUES (:1)"),
            &[Value::Text("x".repeat(100_000))],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported, "{e:?}");
    exec(&*c, &format!("DROP TABLE {t} PURGE"), &[]).await;
    // Bind values on the LOB path are substituted as literals.
    assert_eq!(
        single(
            &*c,
            "SELECT TO_CLOB(:1) FROM dual",
            &[Value::Text("o'k".into())]
        )
        .await,
        Value::Text("o'k".into())
    );
    // Wrong counts are rejected before reaching the server.
    let e = c
        .query("SELECT :1, :2 FROM dual", &[Value::Int(1)], 1)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    let e = c
        .query("SELECT 1 FROM dual", &[Value::Int(1)], 1)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(single(&*c, "SELECT 1 FROM dual", &[]).await, Value::Int(1));
}

#[tokio::test]
#[ignore]
async fn lobs_truncate_at_max_value_bytes() {
    let c = connect().await;
    let p = prefix("lob");
    let t = format!("{p}T");
    try_exec(&*c, &format!("DROP TABLE {t} PURGE")).await;
    exec(
        &*c,
        &format!("CREATE TABLE {t} (id NUMBER, c CLOB, b BLOB)"),
        &[],
    )
    .await;
    exec(
        &*c,
        &format!(
            "DECLARE c CLOB; b BLOB; BEGIN \
               DBMS_LOB.CREATETEMPORARY(c, TRUE); DBMS_LOB.CREATETEMPORARY(b, TRUE); \
               FOR i IN 1 .. 128 LOOP \
                 DBMS_LOB.WRITEAPPEND(c, 16384, RPAD('c', 16384, 'c')); \
                 DBMS_LOB.WRITEAPPEND(b, 16384, UTL_RAW.COPIES(HEXTORAW('AB'), 16384)); \
               END LOOP; \
               INSERT INTO {t} VALUES (1, c, b); \
             END;"
        ),
        &[],
    )
    .await;
    let mib = 1 << 20;
    let rs = query(
        &*c,
        &format!("SELECT c, b, DBMS_LOB.GETLENGTH(c) FROM {t}"),
        &[],
    )
    .await;
    match &rs.rows[0][0] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(*full_len, Some(2 * mib as u64));
            assert!(
                matches!(&**prefix, Value::Text(s) if s.len() == mib && s.bytes().all(|b| b == b'c'))
            );
        }
        other => panic!("{other:?}"),
    }
    match &rs.rows[0][1] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(*full_len, Some(2 * mib as u64));
            assert!(
                matches!(&**prefix, Value::Bytes(b) if b.len() == mib && b.iter().all(|&x| x == 0xab))
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rs.rows[0][2], Value::Int(2 * mib as i64));
    // A larger limit returns the whole value.
    let opts = ExecOptions {
        max_value_bytes: 4 * mib,
        ..ExecOptions::default()
    };
    let rs = c
        .query_with(&format!("SELECT c FROM {t}"), &[], 10, &opts)
        .await
        .unwrap();
    assert!(matches!(&rs.rows[0][0], Value::Text(s) if s.len() == 2 * mib));
    exec(&*c, &format!("DROP TABLE {t} PURGE"), &[]).await;
}

const PAGING_SQL: &str = "SELECT LEVEL AS id, 'row ' || LEVEL AS name, \
    TIMESTAMP '2026-01-01 00:00:00' + NUMTODSINTERVAL(LEVEL, 'SECOND') AS ts, \
    CAST(LEVEL * 1.25 AS NUMBER(12,2)) AS amount FROM dual CONNECT BY LEVEL <= 100000";

#[tokio::test]
#[ignore]
async fn pages_100k_rows() {
    let c = connect().await;
    let Outcome::Rows(mut cur) = c
        .execute(PAGING_SQL, &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    assert_eq!(cur.columns().len(), 4);
    assert_eq!(cur.columns()[3].kind, ValueKind::Decimal);
    let mut next = 1i64;
    let mut pages = 0;
    loop {
        let page = cur.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        pages += 1;
        assert!(
            page.len() == 500 || cur.is_exhausted(),
            "short page {}",
            page.len()
        );
        for row in &page {
            assert_eq!(row[0], Value::Int(next));
            assert_eq!(row[1], Value::Text(format!("row {next}")));
            next += 1;
        }
        if next == 100_001 {
            let last = page.last().unwrap();
            assert_eq!(last[2], Value::Timestamp(ts(2026, 1, 2, 3, 46, 40, 0)));
            assert_eq!(last[3], Value::Decimal("125000".into()));
        }
    }
    assert_eq!(next, 100_001);
    assert_eq!(pages, 200);
    assert!(cur.is_exhausted());
    cur.close().await.unwrap();
    assert_eq!(single(&*c, "SELECT 7 FROM dual", &[]).await, Value::Int(7));
}

#[tokio::test]
#[ignore]
async fn wide_rows_fit_each_round_trip() {
    let c = connect().await;
    let rs = query(
        &*c,
        "SELECT LEVEL n, RPAD(TO_CHAR(LEVEL), 4000, 'x') s FROM dual CONNECT BY LEVEL <= 300",
        &[],
    )
    .await;
    assert_eq!(rs.rows.len(), 300);
    assert!(rs.rows.iter().enumerate().all(|(i, r)| {
        r[0] == Value::Int(i as i64 + 1) && matches!(&r[1], Value::Text(s) if s.len() == 4000)
    }));
}

#[tokio::test]
#[ignore]
async fn busy_rule() {
    let c = connect().await;
    let Outcome::Rows(mut cur) = c
        .execute(
            "SELECT LEVEL FROM dual CONNECT BY LEVEL <= 2000",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    assert_eq!(cur.next_page(10).await.unwrap().len(), 10);
    let e = c
        .execute("SELECT 1 FROM dual", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    let e = c
        .execute(
            "UPDATE dual SET dummy = 'X' WHERE 1 = 0",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    c.commit().await.unwrap();
    c.rollback().await.unwrap();
    assert_eq!(cur.next_page(10).await.unwrap()[0][0], Value::Int(11));
    cur.close().await.unwrap();
    assert_eq!(single(&*c, "SELECT 1 FROM dual", &[]).await, Value::Int(1));
    // Dropping frees the connection too.
    let Outcome::Rows(cur) = c
        .execute("SELECT 1 FROM dual", &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    drop(cur);
    assert_eq!(single(&*c, "SELECT 2 FROM dual", &[]).await, Value::Int(2));
}

#[tokio::test]
#[ignore]
async fn timeout_marks_connection_lost() {
    let c = connect().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_secs(1)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = c
        .execute("BEGIN DBMS_SESSION.SLEEP(30); END;", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let e = c.query("SELECT 1 FROM dual", &[], 1).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConnectionLost, "{e:?}");
    assert_eq!(c.cancel().await.unwrap_err().kind, ErrorKind::Unsupported);
    c.close().await.unwrap();
    let fresh = connect().await;
    assert_eq!(
        single(&*fresh, "SELECT 1 FROM dual", &[]).await,
        Value::Int(1)
    );
}

#[tokio::test]
#[ignore]
async fn errors_are_trapped_and_the_session_survives() {
    let c = connect().await;
    let cases = [
        ("SELECT FROM", "ORA-00936"),
        ("SELECT * FROM pxthin_no_such_table", "ORA-00942"),
        (
            "BEGIN RAISE_APPLICATION_ERROR(-20001, 'boom'); END;",
            "ORA-20001",
        ),
        ("SELECT text FROM all_views WHERE ROWNUM = 1", "ORA-20901"),
        ("INSERT INTO pxthin_no_such_table VALUES (1)", "ORA-00942"),
        ("CREATE TABLE", "ORA-00903"),
    ];
    for (sql, code) in cases {
        let e = c
            .execute(sql, &[], &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query, "{sql}: {e:?}");
        assert_eq!(e.code.as_deref(), Some(code), "{sql}: {e:?}");
        assert!(!e.message.contains("SYS.DBMS_SQL"), "{}", e.message);
    }
    let t = format!("{}T", prefix("err"));
    exec(
        &*c,
        &format!("CREATE TABLE {t} (id NUMBER PRIMARY KEY)"),
        &[],
    )
    .await;
    exec(&*c, &format!("INSERT INTO {t} VALUES (1)"), &[]).await;
    let e = c
        .execute(
            &format!("INSERT INTO {t} VALUES (1)"),
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("ORA-00001"));
    exec(&*c, &format!("DROP TABLE {t} PURGE"), &[]).await;
    assert_eq!(single(&*c, "SELECT 1 FROM dual", &[]).await, Value::Int(1));
}

#[tokio::test]
#[ignore]
async fn fetch_time_error_loses_the_connection() {
    // oracle-rs drops the session when the server reports an error while
    // rows are fetched; the driver reports it as ConnectionLost.
    let c = connect().await;
    let e = c.query("SELECT 1/0 FROM dual", &[], 10).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConnectionLost, "{e:?}");
    assert_eq!(c.ping().await.unwrap_err().kind, ErrorKind::ConnectionLost);
    let fresh = connect().await;
    assert_eq!(
        single(&*fresh, "SELECT 1 FROM dual", &[]).await,
        Value::Int(1)
    );
}

#[tokio::test]
#[ignore]
async fn plsql_blocks_run() {
    let c = connect().await;
    assert_eq!(
        exec(&*c, "BEGIN DBMS_SESSION.SLEEP(0.2); END;", &[]).await,
        0
    );
    assert_eq!(exec(&*c, "CALL DBMS_SESSION.SLEEP(0)", &[]).await, 0);
    exec(
        &*c,
        "DECLARE x NUMBER := :1; BEGIN IF x <> 5 THEN RAISE_APPLICATION_ERROR(-20002, 'bad'); END IF; END;",
        &[Value::Int(5)],
    )
    .await;
    let p = format!("{}P", prefix("plsql"));
    exec(
        &*c,
        &format!("CREATE OR REPLACE PROCEDURE {p} AS BEGIN NULL; END;\n/"),
        &[],
    )
    .await;
    exec(&*c, &format!("BEGIN {p}; END;"), &[]).await;
    let e = c
        .execute(
            &format!("CREATE OR REPLACE PROCEDURE {p} AS BEGIN x; END;"),
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("ORA-24344"));
    exec(&*c, &format!("DROP PROCEDURE {p}"), &[]).await;
}

#[tokio::test]
#[ignore]
async fn select_for_update_with_rowid() {
    let c = connect().await;
    let t = format!("{}T", prefix("fu"));
    exec(
        &*c,
        &format!("CREATE TABLE {t} (id NUMBER, v VARCHAR2(10))"),
        &[],
    )
    .await;
    exec(&*c, &format!("INSERT INTO {t} VALUES (1, 'a')"), &[]).await;
    c.set_tx_mode(TxMode::Manual).await.unwrap();
    let rs = query(
        &*c,
        &format!("SELECT ROWID, t.* FROM {t} t WHERE id = :1 FOR UPDATE NOWAIT"),
        &[Value::Int(1)],
    )
    .await;
    let Value::Other { text: rowid, .. } = &rs.rows[0][0] else {
        panic!("{:?}", rs.rows[0]);
    };
    let n = exec(
        &*c,
        &format!("UPDATE {t} SET v = 'b' WHERE ROWID = CHARTOROWID(:1)"),
        &[Value::Text(rowid.clone())],
    )
    .await;
    assert_eq!(n, 1);
    c.commit().await.unwrap();
    assert_eq!(
        single(&*c, &format!("SELECT v FROM {t}"), &[]).await,
        Value::Text("b".into())
    );
    exec(&*c, &format!("DROP TABLE {t} PURGE"), &[]).await;
}

#[tokio::test]
#[ignore]
async fn read_only_rejects_writes() {
    let mut p = params();
    p.read_only = true;
    let c = ThinDriver.connect(&p).await.unwrap();
    assert_eq!(single(&*c, "SELECT 1 FROM dual", &[]).await, Value::Int(1));
    for sql in [
        "INSERT INTO t VALUES (1)",
        "BEGIN NULL; END;",
        "CREATE TABLE t (a NUMBER)",
    ] {
        let e = c
            .execute(sql, &[], &ExecOptions::default())
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query, "{sql}");
    }
}

#[tokio::test]
#[ignore]
async fn wrong_password_is_auth_error() {
    let mut p = params();
    p.password = db_core::Secret::new("definitely-wrong-pw");
    let e = ThinDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Auth, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-01017"), "{e:?}");
    assert!(!e.message.contains("definitely-wrong-pw"));
}

#[tokio::test]
#[ignore]
async fn unknown_service_and_sid_are_connect_errors() {
    let mut p = params();
    p.database = "PXTHIN_NO_SUCH_SERVICE".into();
    let e = ThinDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Connect, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-12514"));
    p.options.insert(opt::ORACLE_SID.into(), "true".into());
    let e = ThinDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Connect, "{e:?}");
    assert_eq!(e.code.as_deref(), Some("ORA-12505"));
}

#[tokio::test]
#[ignore]
async fn closed_port_is_network_error() {
    let mut p = params();
    p.host = "127.0.0.1".into();
    p.port = 1;
    p.connect_timeout = Duration::from_secs(3);
    let e = ThinDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Network, "{e:?}");
}

#[tokio::test]
#[ignore]
async fn connects_with_a_connect_string() {
    let base = params();
    for dsn in [
        format!("{}:{}/{}", base.host, base.port, base.database),
        format!(
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST={})(PORT={}))(CONNECT_DATA=(SERVICE_NAME={})))",
            base.host, base.port, base.database
        ),
    ] {
        let mut p = params();
        p.host = String::new();
        p.database = String::new();
        p.options.insert(opt::ORACLE_DSN.into(), dsn.clone());
        let c = ThinDriver
            .connect(&p)
            .await
            .unwrap_or_else(|e| panic!("{dsn}: {e:?}"));
        assert_eq!(single(&*c, "SELECT 1 FROM dual", &[]).await, Value::Int(1));
    }
}

#[tokio::test]
#[ignore]
async fn renews_the_session_before_open_cursors_runs_out() {
    let c = ThinConnection::open(&params()).await.unwrap();
    exec(&c, "ALTER SESSION SET NLS_DATE_FORMAT = 'YYYY/MM/DD'", &[]).await;
    let (_, limit) = c.cursor_usage();
    let mut i = 0;
    while c.session_renewals() == 0 {
        i += 1;
        assert!(i < limit * 2, "no renewal after {i} statements");
        let v = single(&c, &format!("SELECT {i} FROM dual"), &[]).await;
        assert_eq!(v, Value::Int(i as i64));
    }
    let fmt = single(
        &c,
        "SELECT value FROM nls_session_parameters WHERE parameter = 'NLS_DATE_FORMAT'",
        &[],
    )
    .await;
    assert_eq!(
        fmt,
        Value::Text("YYYY/MM/DD".into()),
        "ALTER SESSION is replayed"
    );

    // With uncommitted changes the session cannot be renewed.
    let t = format!("{}T", prefix("renew"));
    exec(&c, &format!("CREATE TABLE {t} (id NUMBER)"), &[]).await;
    c.set_tx_mode(TxMode::Manual).await.unwrap();
    exec(&c, &format!("INSERT INTO {t} VALUES (1)"), &[]).await;
    let mut refused = false;
    for _ in 0..limit {
        match c.query("SELECT 1 FROM dual", &[], 1).await {
            Ok(_) => {}
            Err(e) => {
                assert_eq!(e.kind, ErrorKind::Unsupported, "{e:?}");
                refused = true;
                break;
            }
        }
    }
    assert!(refused);
    c.rollback().await.unwrap();
    assert_eq!(
        single(&c, &format!("SELECT COUNT(*) FROM {t}"), &[]).await,
        Value::Int(0)
    );
    c.set_tx_mode(TxMode::Auto).await.unwrap();
    exec(&c, &format!("DROP TABLE {t} PURGE"), &[]).await;
}

/// Servers that negotiate an 8 KiB session data unit (the documented server
/// default) get smaller fetches and a request size limit.
#[tokio::test]
#[ignore]
async fn small_packets_still_work() {
    let mut p = params();
    p.options.insert(SDU_OPTION.into(), "8192".into());
    let c = ThinDriver.connect(&p).await.unwrap();
    let rs = query(
        &*c,
        "SELECT 42, CAST(1.5 AS BINARY_DOUBLE), TIMESTAMP '2026-01-02 03:04:05.5 +03:00', NULL FROM dual",
        &[],
    )
    .await;
    assert_eq!(rs.rows[0][0], Value::Int(42));
    assert_eq!(rs.rows[0][1], Value::Float(1.5));
    let rs = query(&*c, &PAGING_SQL.replace("100000", "20000"), &[]).await;
    assert_eq!(rs.rows.len(), 20_000);
    assert_eq!(rs.rows[19_999][0], Value::Int(20_000));
    let rs = query(
        &*c,
        "SELECT LEVEL n, RPAD(TO_CHAR(LEVEL), 4000, 'x') s FROM dual CONNECT BY LEVEL <= 40",
        &[],
    )
    .await;
    assert_eq!(rs.rows.len(), 40);
    let long = format!("SELECT 1 FROM dual WHERE 'x' <> '{}'", "y".repeat(10_000));
    let e = c.query(&long, &[], 1).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported, "{e:?}");
    assert_eq!(single(&*c, "SELECT 2 FROM dual", &[]).await, Value::Int(2));
}
