//! Integration tests against a real PostgreSQL server.
//!
//! They are `#[ignore]`d because they need the container from
//! `docker-compose.yml`:
//!
//! ```bash
//! docker compose up -d postgres
//! cargo test -p db-postgres -- --ignored
//! ```
//!
//! Address and credentials come from `PYOXIDE_TEST_PG_HOST` (default
//! `localhost`), `PYOXIDE_TEST_PG_PORT` (default `55432`) and
//! `PYOXIDE_DB_PASSWORD` (default `pyoxide`). Every test creates objects with
//! a unique `pxpg_<test>` prefix and drops them again.
//!
//! The TLS tests against a TLS-enabled server only run when
//! `PYOXIDE_TEST_PG_TLS_PORT` and `PYOXIDE_TEST_PG_TLS_CA` (path of the CA
//! certificate the server certificate was issued by) are set. The server
//! certificate must be valid for `localhost` and `127.0.0.1`. The mutual TLS
//! test also needs `PYOXIDE_TEST_PG_TLS_CLIENT_CERT` and
//! `PYOXIDE_TEST_PG_TLS_CLIENT_KEY` for a role that the server maps to the
//! certificate (`PYOXIDE_TEST_PG_TLS_CLIENT_USER`, default `pyoxide_cert`,
//! with a `hostssl ... cert` rule in `pg_hba.conf`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate};
use db_core::schema::{ConstraintKind, ObjectKind, introspector};
use db_core::{
    ConnectParams, Connection, Driver, ErrorKind, ExecOptions, Outcome, Secret, TlsMode, TxMode,
    Value,
};
use db_postgres::PostgresDriver;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn params() -> ConnectParams {
    ConnectParams {
        host: env_or("PYOXIDE_TEST_PG_HOST", "localhost"),
        port: env_or("PYOXIDE_TEST_PG_PORT", "55432")
            .parse()
            .expect("port"),
        database: "pyoxide".into(),
        user: "pyoxide".into(),
        password: Secret::new(env_or("PYOXIDE_DB_PASSWORD", "pyoxide")),
        ..Default::default()
    }
}

async fn connect() -> Box<dyn Connection> {
    PostgresDriver
        .connect(&params())
        .await
        .expect("connect to the test PostgreSQL (docker compose up -d postgres)")
}

async fn exec(conn: &dyn Connection, sql: &str) {
    if let Err(e) = conn.execute(sql, &[], &ExecOptions::default()).await {
        panic!("{sql}: {e}");
    }
}

async fn row(conn: &dyn Connection, sql: &str, params: &[Value]) -> Vec<Value> {
    let mut rs = conn
        .query(sql, params, 10)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    assert_eq!(rs.rows.len(), 1, "{sql}");
    rs.rows.remove(0)
}

fn utc(text: &str) -> Value {
    Value::TimestampTz(DateTime::parse_from_rfc3339(text).expect("timestamp"))
}

#[tokio::test]
#[ignore]
async fn connect_and_server_version() {
    let conn = connect().await;
    let version = conn.server_version().await.unwrap();
    assert!(version.starts_with("PostgreSQL "), "{version}");
    conn.ping().await.unwrap();
    // cached, and usable again
    assert_eq!(conn.server_version().await.unwrap(), version);
    assert_eq!(conn.tx_mode().await, TxMode::Auto);
    assert!(PostgresDriver.availability().is_ok());
    conn.close().await.unwrap();
    assert_eq!(
        conn.ping().await.unwrap_err().kind,
        ErrorKind::ConnectionLost
    );
}

#[tokio::test]
#[ignore]
async fn application_name_defaults_and_can_be_set() {
    let conn = connect().await;
    let r = row(&*conn, "SHOW application_name", &[]).await;
    assert_eq!(r[0], Value::from("pyoxide"));

    let mut p = params();
    p.options
        .insert(db_core::opt::APPLICATION_NAME.into(), "pxpg_custom".into());
    let c2 = PostgresDriver.connect(&p).await.unwrap();
    let r = row(&*c2, "SHOW application_name", &[]).await;
    assert_eq!(r[0], Value::from("pxpg_custom"));
}

#[tokio::test]
#[ignore]
async fn typed_round_trip() {
    let conn = connect().await;
    exec(&*conn, "DROP SCHEMA IF EXISTS pxpg_types CASCADE").await;
    exec(&*conn, "CREATE SCHEMA pxpg_types").await;
    exec(
        &*conn,
        "CREATE TYPE pxpg_types.mood AS ENUM ('sad', 'happy')",
    )
    .await;
    exec(
        &*conn,
        "CREATE DOMAIN pxpg_types.posint AS int4 CHECK (VALUE > 0)",
    )
    .await;

    let sql = "SELECT \
        true, false, (-32768)::int2, 2147483647::int4, 9223372036854775807::int8, \
        4000000000::oid, 1.5::float4, 0.1::float4, 2.5::float8, 'NaN'::float8, \
        '-Infinity'::float8, \
        12345678901234567890123456789012345678::numeric(38,0), \
        -0.000012345::numeric, 'NaN'::numeric, 100.50::numeric(10,2), \
        'héllo wörld ✓'::text, ''::text, 'ab'::varchar(10), 'ab'::bpchar(5), 'nm'::name, \
        '\\xdeadbeef'::bytea, '\\x'::bytea, \
        'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid, \
        '{\"a\": [1, 2, {\"b\": null}]}'::jsonb, '{\"a\":  1}'::json, \
        DATE '2026-01-02', 'infinity'::date, TIME '03:04:05.123456', \
        TIMESTAMP '2026-01-02 03:04:05.123456', \
        TIMESTAMPTZ '2026-01-02 03:04:05.123456+03', 'infinity'::timestamptz, \
        INTERVAL '1 year 2 mons 3 days 04:05:06.5', \
        NULL::int4, NULL::text, 'happy'::pxpg_types.mood, 7::pxpg_types.posint";
    let r = row(&*conn, sql, &[]).await;
    let expected = vec![
        Value::Bool(true),
        Value::Bool(false),
        Value::Int(-32768),
        Value::Int(2_147_483_647),
        Value::Int(i64::MAX),
        Value::Int(4_000_000_000),
        Value::Float(1.5),
        Value::Float(0.1),
        Value::Float(2.5),
        Value::Float(f64::NAN),
        Value::Float(f64::NEG_INFINITY),
        Value::Decimal("12345678901234567890123456789012345678".into()),
        Value::Decimal("-0.000012345".into()),
        Value::Other {
            type_name: "numeric".into(),
            text: "NaN".into(),
        },
        Value::Decimal("100.50".into()),
        Value::Text("héllo wörld ✓".into()),
        Value::Text(String::new()),
        Value::Text("ab".into()),
        Value::Text("ab   ".into()),
        Value::Text("nm".into()),
        Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
        Value::Bytes(vec![]),
        Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
        Value::Json("{\"a\": [1, 2, {\"b\": null}]}".into()),
        Value::Json("{\"a\":  1}".into()),
        Value::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
        Value::Other {
            type_name: "date".into(),
            text: "infinity".into(),
        },
        Value::Time(chrono::NaiveTime::from_hms_micro_opt(3, 4, 5, 123_456).unwrap()),
        Value::Timestamp(
            NaiveDate::from_ymd_opt(2026, 1, 2)
                .unwrap()
                .and_hms_micro_opt(3, 4, 5, 123_456)
                .unwrap(),
        ),
        // 03:04:05.123456+03 is 00:04:05.123456 UTC
        utc("2026-01-02T03:04:05.123456+03:00"),
        Value::Other {
            type_name: "timestamptz".into(),
            text: "infinity".into(),
        },
        Value::Interval("1 year 2 mons 3 days 04:05:06.5".into()),
        Value::Null,
        Value::Null,
        Value::Text("happy".into()),
        Value::Int(7),
    ];
    assert_eq!(r.len(), expected.len());
    for (i, (got, want)) in r.iter().zip(&expected).enumerate() {
        match (got, want) {
            (Value::Float(a), Value::Float(b)) if b.is_nan() => assert!(a.is_nan(), "col {i}"),
            _ => assert_eq!(got, want, "column {i}"),
        }
    }

    // timestamptz keeps the instant; the offset is UTC
    if let Value::TimestampTz(t) = &r[29] {
        assert_eq!(t.offset().local_minus_utc(), 0);
    }

    exec(&*conn, "DROP SCHEMA pxpg_types CASCADE").await;
}

#[tokio::test]
#[ignore]
async fn column_metadata() {
    let conn = connect().await;
    let rs = conn
        .query(
            "SELECT 1::int4 AS a, 'x'::text AS b, 1.5::numeric AS c, ARRAY[1]::int4[] AS d, \
             now() AS e, '{}'::jsonb AS f",
            &[],
            1,
        )
        .await
        .unwrap();
    let cols: Vec<_> = rs
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.type_name.as_str(), c.kind))
        .collect();
    use db_core::ValueKind as K;
    assert_eq!(
        cols,
        vec![
            ("a", "int4", K::Int),
            ("b", "text", K::Text),
            ("c", "numeric", K::Decimal),
            ("d", "int4[]", K::Array),
            ("e", "timestamptz", K::TimestampTz),
            ("f", "jsonb", K::Json),
        ]
    );
    assert!(rs.columns.iter().all(|c| c.nullable));
}

#[tokio::test]
#[ignore]
async fn arrays_nulls_and_nesting() {
    let conn = connect().await;
    let r = row(
        &*conn,
        "SELECT ARRAY[1, NULL, 3], ARRAY['a', NULL, 'c d'], ARRAY[[1, 2], [3, 4]], \
         '{}'::int4[], ARRAY[1.5, NULL]::numeric[], ARRAY['2026-01-02'::date], \
         ARRAY[true, false], ARRAY['{\"k\": 1}'::jsonb], \
         ARRAY[[[1], [2]], [[3], [4]]]::int8[]",
        &[],
    )
    .await;
    let ints = |v: &[Option<i64>]| {
        Value::Array(
            v.iter()
                .map(|i| i.map_or(Value::Null, Value::Int))
                .collect(),
        )
    };
    assert_eq!(r[0], ints(&[Some(1), None, Some(3)]));
    assert_eq!(
        r[1],
        Value::Array(vec![Value::from("a"), Value::Null, Value::from("c d")])
    );
    assert_eq!(
        r[2],
        Value::Array(vec![ints(&[Some(1), Some(2)]), ints(&[Some(3), Some(4)])])
    );
    assert_eq!(r[3], Value::Array(vec![]));
    assert_eq!(
        r[4],
        Value::Array(vec![Value::Decimal("1.5".into()), Value::Null])
    );
    assert_eq!(
        r[5],
        Value::Array(vec![Value::Date(
            NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()
        )])
    );
    assert_eq!(
        r[6],
        Value::Array(vec![Value::Bool(true), Value::Bool(false)])
    );
    assert_eq!(r[7], Value::Array(vec![Value::Json("{\"k\": 1}".into())]));
    // 2 x 2 x 1 nests by dimension
    let leaf = |a: i64, b: i64| {
        Value::Array(vec![
            Value::Array(vec![Value::Int(a)]),
            Value::Array(vec![Value::Int(b)]),
        ])
    };
    assert_eq!(r[8], Value::Array(vec![leaf(1, 2), leaf(3, 4)]));
}

#[tokio::test]
#[ignore]
async fn other_types_have_text_forms() {
    let conn = connect().await;
    let r = row(
        &*conn,
        "SELECT '192.168.1.5/24'::inet, '192.168.1.5'::inet, '10.0.0.0/8'::cidr, \
         '::1'::inet, '08:00:2b:01:02:03'::macaddr, B'10110'::bit(5), 12.34::money, \
         int4range(1, 10), 'empty'::int4range, daterange('2026-01-01', NULL), \
         ROW(1, 'a b', NULL), TIMETZ '04:05:06.5+03', '0/16B3748'::pg_lsn, \
         '(1,2)'::tid, int4multirange(int4range(1, 3), int4range(5, 7))",
        &[],
    )
    .await;
    let other = |t: &str, text: &str| Value::Other {
        type_name: t.into(),
        text: text.into(),
    };
    assert_eq!(r[0], other("inet", "192.168.1.5/24"));
    assert_eq!(r[1], other("inet", "192.168.1.5"));
    assert_eq!(r[2], other("cidr", "10.0.0.0/8"));
    assert_eq!(r[3], other("inet", "::1"));
    assert_eq!(r[4], other("macaddr", "08:00:2b:01:02:03"));
    assert_eq!(r[5], other("bit", "10110"));
    assert_eq!(r[6], other("money", "12.34"));
    assert_eq!(r[7], other("int4range", "[1,10)"));
    assert_eq!(r[8], other("int4range", "empty"));
    assert_eq!(r[9], other("daterange", "[2026-01-01,)"));
    assert_eq!(r[10], other("record", "(1,\"a b\",)"));
    assert_eq!(r[11], other("timetz", "04:05:06.5+03"));
    assert_eq!(r[12], other("pg_lsn", "0/16B3748"));
    assert_eq!(r[13], other("tid", "(1,2)"));
    assert_eq!(r[14], other("int4multirange", "{[1,3),[5,7)}"));
}

/// The decoders agree with the server's own text output.
#[tokio::test]
#[ignore]
async fn numeric_and_interval_match_server_text() {
    let conn = connect().await;
    let numerics = [
        "0",
        "0.00",
        "1",
        "-1",
        "10000",
        "10001",
        "99999999999999999999999999999999999999",
        "-99999999999999999999999999999999999999",
        "12345678901234567890123456789012345678",
        "0.1",
        "0.0001",
        "0.00001",
        "0.000000000000000000000000000000000001",
        "123456789.123456789123456789",
        "1.500",
        "-0.5",
        "1000000.000001",
        "70000000000000000000000000000000000000.000",
        "1e30",
        "1.5e-20",
        "3.14159265358979323846264338327950288419716939937510",
    ];
    for n in numerics {
        let r = row(
            &*conn,
            "SELECT $1::numeric, ($1::numeric)::text",
            &[Value::Text(n.into())],
        )
        .await;
        assert_eq!(
            r[0],
            Value::Decimal(r[1].as_str().unwrap().to_owned()),
            "numeric {n}"
        );
    }

    let intervals = [
        "0",
        "1 year",
        "1 year 2 mons 3 days 04:05:06.5",
        "-1 year -2 mons +3 days -04:05:06.5",
        "-3 days",
        "3 days -04:05:06",
        "1 mon -1 day",
        "-1 mon 1 day 00:00:01",
        "100 hours",
        "-100 hours",
        "1 microsecond",
        "-1 microsecond",
        "999999 hours 59 minutes 59.999999 seconds",
        "2 years 6 mons",
        "-2 years -6 mons",
        "1 day 1 microsecond",
        "-1 day +1 microsecond",
        "5 mons",
        "11 mons 29 days",
        "13 mons",
        "0.5 seconds",
        "1 year -1 mon",
    ];
    for i in intervals {
        let r = row(
            &*conn,
            "SELECT $1::interval, ($1::interval)::text",
            &[Value::Text(i.into())],
        )
        .await;
        assert_eq!(
            r[0],
            Value::Interval(r[1].as_str().unwrap().to_owned()),
            "interval {i}"
        );
    }
    let r = row(
        &*conn,
        "SELECT 'infinity'::interval, '-infinity'::interval",
        &[],
    )
    .await;
    // PostgreSQL 17 and later have infinite intervals
    assert_eq!(r[0], Value::Interval("infinity".into()));
    assert_eq!(r[1], Value::Interval("-infinity".into()));
}

#[tokio::test]
#[ignore]
async fn bind_parameters() {
    let conn = connect().await;
    exec(&*conn, "DROP TABLE IF EXISTS pxpg_bind").await;
    exec(
        &*conn,
        "CREATE TABLE pxpg_bind (i2 int2, i4 int4, i8 int8, f4 float4, f8 float8, n numeric, \
         t text, vc varchar(5), b bytea, bo bool, u uuid, d date, ts timestamptz, j jsonb)",
    )
    .await;

    let values = vec![
        Value::Int(-7),
        Value::Int(70_000),
        Value::Int(i64::MAX),
        Value::Float(1.5),
        Value::Float(-2.25),
        Value::Decimal("12345678901234567890123456789012345678.123456".into()),
        Value::Text("héllo".into()),
        Value::Text("abc".into()),
        Value::Bytes(vec![0, 1, 255]),
        Value::Bool(true),
        Value::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
        Value::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
        Value::TimestampTz(DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5+03:00").unwrap()),
        Value::Text("{\"a\": 1}".into()),
    ];
    let placeholders: Vec<String> = (1..=values.len()).map(|i| format!("${i}")).collect();
    let insert = format!("INSERT INTO pxpg_bind VALUES ({})", placeholders.join(", "));
    let out = conn
        .execute(&insert, &values, &ExecOptions::default())
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(1)));

    let r = row(
        &*conn,
        "SELECT i2, i4, i8, f4, f8, n, t, vc, b, bo, u, d, ts, j FROM pxpg_bind",
        &[],
    )
    .await;
    assert_eq!(r[0], Value::Int(-7));
    assert_eq!(r[1], Value::Int(70_000));
    assert_eq!(r[2], Value::Int(i64::MAX));
    assert_eq!(r[3], Value::Float(1.5));
    assert_eq!(r[4], Value::Float(-2.25));
    assert_eq!(
        r[5],
        Value::Decimal("12345678901234567890123456789012345678.123456".into())
    );
    assert_eq!(r[6], Value::from("héllo"));
    assert_eq!(r[7], Value::from("abc"));
    assert_eq!(r[8], Value::Bytes(vec![0, 1, 255]));
    assert_eq!(r[9], Value::Bool(true));
    assert_eq!(
        r[10],
        Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())
    );
    assert_eq!(
        r[11],
        Value::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap())
    );
    assert_eq!(r[12], utc("2026-01-02T00:04:05.5Z"));
    assert_eq!(r[13], Value::Json("{\"a\": 1}".into()));

    // NULL into every column
    let nulls = vec![Value::Null; values.len()];
    conn.execute(&insert, &nulls, &ExecOptions::default())
        .await
        .unwrap();
    let rs = conn
        .query("SELECT * FROM pxpg_bind WHERE i2 IS NULL", &[], 10)
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 1);
    assert!(rs.rows[0].iter().all(Value::is_null));

    // parameters in a WHERE clause: Int into int4 and numeric, Text into text
    let found = conn
        .query(
            "SELECT count(*) FROM pxpg_bind WHERE i4 = $1 AND n > $2 AND t = $3",
            &[Value::Int(70_000), Value::Int(1), Value::from("héllo")],
            10,
        )
        .await
        .unwrap();
    assert_eq!(found.rows[0][0], Value::Int(1));

    // errors: out of range, wrong count, bytes into text
    let e = conn
        .execute(
            "INSERT INTO pxpg_bind (i2) VALUES ($1)",
            &[Value::Int(40_000)],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert!(e.message.contains("out of range"), "{}", e.message);
    let e = conn
        .execute("SELECT $1::int", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    let e = conn
        .execute(
            "INSERT INTO pxpg_bind (t) VALUES ($1)",
            &[Value::Bytes(vec![1])],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    // a server-side parse failure of a text bind is a query error with SQLSTATE
    let e = conn
        .execute(
            "INSERT INTO pxpg_bind (u) VALUES ($1)",
            &[Value::from("not a uuid")],
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("22P02"));
    // the connection is still fine
    conn.ping().await.unwrap();

    exec(&*conn, "DROP TABLE pxpg_bind").await;
}

#[tokio::test]
#[ignore]
async fn statements_without_rows_report_affected_counts() {
    let conn = connect().await;
    exec(&*conn, "DROP TABLE IF EXISTS pxpg_affected").await;
    exec(&*conn, "CREATE TABLE pxpg_affected (n int)").await;
    let out = conn
        .execute(
            "INSERT INTO pxpg_affected SELECT generate_series(1, 25)",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(25)), "{out:?}");
    let out = conn
        .execute(
            "DELETE FROM pxpg_affected WHERE n > 20",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(5)), "{out:?}");
    // INSERT ... RETURNING returns rows
    let rs = conn
        .query(
            "INSERT INTO pxpg_affected VALUES (100), (101) RETURNING n",
            &[],
            10,
        )
        .await
        .unwrap();
    assert_eq!(rs.rows, vec![vec![Value::Int(100)], vec![Value::Int(101)]]);
    exec(&*conn, "DROP TABLE pxpg_affected").await;
}

#[tokio::test]
#[ignore]
async fn page_through_100k_rows() {
    let conn = connect().await;
    let opts = ExecOptions::default();
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT g AS id, 'row ' || g AS label FROM generate_series(1, 100000) g",
            &[],
            &opts,
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    assert_eq!(cursor.columns().len(), 2);

    let mut expected = 1_i64;
    let mut pages = 0;
    loop {
        let page = cursor.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 500);
        for row in &page {
            assert_eq!(row[0], Value::Int(expected));
            assert_eq!(row[1], Value::Text(format!("row {expected}")));
            expected += 1;
        }
        pages += 1;
    }
    assert_eq!(expected, 100_001);
    assert_eq!(pages, 200);
    assert!(cursor.is_exhausted());
    // exhausted cursors keep answering with empty pages
    assert!(cursor.next_page(500).await.unwrap().is_empty());
    // and the connection is free without closing the cursor
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
    cursor.close().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn empty_result_is_exhausted_at_once() {
    let conn = connect().await;
    let Outcome::Rows(cursor) = conn
        .execute("SELECT 1 AS n WHERE false", &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected a cursor");
    };
    assert_eq!(cursor.columns().len(), 1);
    assert!(cursor.is_exhausted());
    drop(cursor);
    assert_eq!(row(&*conn, "SELECT 2", &[]).await[0], Value::Int(2));
}

#[tokio::test]
#[ignore]
async fn closing_a_big_cursor_early_is_fast() {
    let conn = connect().await;
    let opts = ExecOptions::default();
    // enough rows that reading them all would take far longer than a second;
    // a set-returning function in the select list streams its rows
    let big = "SELECT generate_series(1, 200000000) AS g, repeat('x', 100)";
    let Outcome::Rows(mut cursor) = conn.execute(big, &[], &opts).await.unwrap() else {
        panic!("expected rows");
    };
    let page = cursor.next_page(500).await.unwrap();
    assert_eq!(page.len(), 500);

    let started = Instant::now();
    cursor.close().await.unwrap();
    let r = row(&*conn, "SELECT 42", &[]).await;
    let elapsed = started.elapsed();
    assert_eq!(r[0], Value::Int(42));
    assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");

    // dropping the cursor is cleaned up in the background; the next
    // statement waits for it instead of failing with Busy
    let Outcome::Rows(mut cursor) = conn.execute(big, &[], &opts).await.unwrap() else {
        panic!("expected rows");
    };
    cursor.next_page(10).await.unwrap();
    let started = Instant::now();
    drop(cursor);
    let r = row(&*conn, "SELECT 43", &[]).await;
    let elapsed = started.elapsed();
    assert_eq!(r[0], Value::Int(43));
    assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");

    // the server really stopped: no other backend of this test is still busy
    let active = row(
        &*conn,
        "SELECT count(*) FROM pg_stat_activity \
         WHERE state = 'active' AND query LIKE '%generate_series(1, 200000000)%' \
         AND pid <> pg_backend_pid()",
        &[],
    )
    .await;
    assert_eq!(active[0], Value::Int(0));
}

/// Stopping a statement that changes data before it has completed would roll
/// the change back, so its cursor is read to the end when it is closed early.
/// A deferred trigger keeps the statement busy at commit time.
#[tokio::test]
#[ignore]
async fn closing_a_data_modifying_cursor_keeps_the_change() {
    let conn = connect().await;
    let other = connect().await;
    exec(&*conn, "DROP TABLE IF EXISTS pxpg_returning").await;
    exec(&*conn, "DROP FUNCTION IF EXISTS pxpg_slow_commit()").await;
    exec(&*conn, "CREATE TABLE pxpg_returning (n int)").await;
    exec(
        &*conn,
        "CREATE FUNCTION pxpg_slow_commit() RETURNS trigger LANGUAGE plpgsql \
         AS 'BEGIN PERFORM pg_sleep(0.4); RETURN NULL; END'",
    )
    .await;
    exec(
        &*conn,
        "CREATE CONSTRAINT TRIGGER pxpg_slow AFTER INSERT ON pxpg_returning \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION pxpg_slow_commit()",
    )
    .await;

    let insert =
        "INSERT INTO pxpg_returning SELECT generate_series(1, 3) RETURNING n, repeat('x', 4000)";
    let count = || row(&*other, "SELECT count(*) FROM pxpg_returning", &[]);

    let Outcome::Rows(mut cursor) = conn
        .execute(insert, &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    assert_eq!(cursor.next_page(1).await.unwrap().len(), 1);
    cursor.close().await.unwrap();
    assert_eq!(count().await[0], Value::Int(3));

    // the same when the cursor is dropped
    let Outcome::Rows(mut cursor) = conn
        .execute(insert, &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    cursor.next_page(1).await.unwrap();
    drop(cursor);
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
    assert_eq!(count().await[0], Value::Int(6));

    exec(&*conn, "DROP TABLE pxpg_returning").await;
    exec(&*conn, "DROP FUNCTION pxpg_slow_commit()").await;
}

#[tokio::test]
#[ignore]
async fn one_open_cursor_per_connection() {
    let conn = connect().await;
    let opts = ExecOptions::default();
    let Outcome::Rows(mut cursor) = conn
        .execute("SELECT g FROM generate_series(1, 5000) g", &[], &opts)
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    cursor.next_page(10).await.unwrap();

    let e = conn.execute("SELECT 1", &[], &opts).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    let e = conn.query("SELECT 1", &[], 1).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Busy);
    // ping answers without touching the cursor
    conn.ping().await.unwrap();
    // cancel is allowed while a cursor is open
    conn.cancel().await.unwrap();

    cursor.close().await.unwrap();
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
}

#[tokio::test]
#[ignore]
async fn cancel_ends_a_running_statement() {
    let conn: Arc<dyn Connection> = Arc::from(connect().await);
    let canceller = {
        let conn = Arc::clone(&conn);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            conn.cancel().await
        })
    };
    let started = Instant::now();
    let e = conn
        .query("SELECT pg_sleep(30)", &[], 10)
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(e.kind, ErrorKind::Cancelled, "{e}");
    assert_eq!(e.code.as_deref(), Some("57014"));
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    canceller.await.unwrap().unwrap();

    // the connection is usable again
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
    // cancel while idle is harmless, and the next statement is not hit by it
    conn.cancel().await.unwrap();
    assert_eq!(row(&*conn, "SELECT 2", &[]).await[0], Value::Int(2));
}

#[tokio::test]
#[ignore]
async fn cancel_ends_a_slow_fetch_and_frees_the_connection() {
    let conn: Arc<dyn Connection> = Arc::from(connect().await);
    // 30 fat rows fill the server's send buffer, then every row sleeps
    let sql = "SELECT repeat('x', 1000), pg_sleep(CASE WHEN i > 20 THEN 30 ELSE 0 END) \
               FROM generate_series(1, 30) i";
    let Outcome::Rows(mut cursor) = conn
        .execute(sql, &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    assert_eq!(cursor.next_page(10).await.unwrap().len(), 10);
    let canceller = {
        let conn = Arc::clone(&conn);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            conn.cancel().await
        })
    };
    let started = Instant::now();
    // 10 more rows are buffered, the next ones block in pg_sleep
    let mut err = None;
    for _ in 0..4 {
        match cursor.next_page(10).await {
            Ok(_) => {}
            Err(e) => {
                err = Some(e);
                break;
            }
        }
    }
    let e = err.expect("the fetch should have been cancelled");
    assert_eq!(e.kind, ErrorKind::Cancelled, "{e}");
    assert!(started.elapsed() < Duration::from_secs(5));
    canceller.await.unwrap().unwrap();
    // the failed cursor stays failed, and the connection is free
    assert!(cursor.next_page(10).await.is_err());
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
}

#[tokio::test]
#[ignore]
async fn timeout_applies_to_execute() {
    let conn = connect().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_secs(1)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = conn
        .execute("SELECT pg_sleep(30), 'pxpg_timeout_exec'", &[], &opts)
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(e.kind, ErrorKind::Timeout, "{e}");
    assert!(elapsed >= Duration::from_millis(900), "took {elapsed:?}");
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");

    // the statement was cancelled on the server and the connection works
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
    let running = row(
        &*conn,
        "SELECT count(*) FROM pg_stat_activity \
         WHERE state = 'active' AND query LIKE '%pxpg_timeout_exec%' AND pid <> pg_backend_pid()",
        &[],
    )
    .await;
    assert_eq!(running[0], Value::Int(0));
}

#[tokio::test]
#[ignore]
async fn timeout_applies_to_every_page() {
    let conn = connect().await;
    let opts = ExecOptions {
        timeout: Some(Duration::from_secs(1)),
        ..ExecOptions::default()
    };
    let sql = "SELECT repeat('x', 1000), pg_sleep(CASE WHEN i > 20 THEN 30 ELSE 0 END) \
               FROM generate_series(1, 30) i";
    let Outcome::Rows(mut cursor) = conn.execute(sql, &[], &opts).await.unwrap() else {
        panic!("expected rows");
    };
    let started = Instant::now();
    let mut err = None;
    for _ in 0..4 {
        if let Err(e) = cursor.next_page(10).await {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("the page fetch should time out");
    assert_eq!(e.kind, ErrorKind::Timeout, "{e}");
    assert!(started.elapsed() < Duration::from_secs(6));
    // the connection is usable (the cursor stops in the background)
    drop(cursor);
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
}

/// A cancel sent while closing a cursor must be finished on the server before
/// the next statement starts, or it would cancel that statement.
#[tokio::test]
#[ignore]
async fn closing_cursors_never_cancels_the_next_statement() {
    let conn = connect().await;
    let opts = ExecOptions::default();
    let big = "SELECT generate_series(1, 100000000) AS g";
    for i in 0..40_i64 {
        let Outcome::Rows(mut cursor) = conn.execute(big, &[], &opts).await.unwrap() else {
            panic!("expected rows");
        };
        cursor.next_page(1).await.unwrap();
        if i % 2 == 0 {
            cursor.close().await.unwrap();
        } else {
            drop(cursor);
        }
        let r = row(&*conn, "SELECT $1::int8 + 1", &[Value::Int(i)]).await;
        assert_eq!(r[0], Value::Int(i + 1), "iteration {i}");
    }
    // the same for the cancel that ends a timeout
    let quick = ExecOptions {
        timeout: Some(Duration::from_millis(100)),
        ..ExecOptions::default()
    };
    for i in 0..5_i64 {
        let e = conn
            .execute("SELECT pg_sleep(30)", &[], &quick)
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout);
        let r = row(&*conn, "SELECT $1::int8 + 1", &[Value::Int(i)]).await;
        assert_eq!(r[0], Value::Int(i + 1));
    }
}

#[tokio::test]
#[ignore]
async fn transactions_manual_and_auto() {
    let conn = connect().await;
    let other = connect().await;
    exec(&*conn, "DROP TABLE IF EXISTS pxpg_tx").await;
    exec(&*conn, "CREATE TABLE pxpg_tx (n int)").await;
    async fn rows_seen(conn: &dyn Connection) -> i64 {
        row(conn, "SELECT count(*) FROM pxpg_tx", &[]).await[0]
            .as_i64()
            .unwrap()
    }

    // auto mode commits every statement
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (1)").await;
    assert_eq!(rows_seen(&*other).await, 1);

    // manual mode: rollback discards
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    assert_eq!(conn.tx_mode().await, TxMode::Manual);
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (2)").await;
    assert_eq!(rows_seen(&*conn).await, 2);
    assert_eq!(
        rows_seen(&*other).await,
        1,
        "uncommitted rows are invisible"
    );
    conn.rollback().await.unwrap();
    assert_eq!(rows_seen(&*conn).await, 1);

    // manual mode: commit persists
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (3)").await;
    assert_eq!(rows_seen(&*other).await, 1);
    conn.commit().await.unwrap();
    assert_eq!(rows_seen(&*other).await, 2);

    // commit with nothing pending is fine
    conn.commit().await.unwrap();
    conn.rollback().await.unwrap();

    // a transaction statement typed by the user ends the tracked transaction
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (4)").await;
    exec(&*conn, "COMMIT").await;
    assert_eq!(rows_seen(&*other).await, 3);
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (5)").await;
    assert_eq!(rows_seen(&*other).await, 3, "a new transaction began");
    conn.rollback().await.unwrap();

    // a failed statement leaves the transaction to be rolled back
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (6)").await;
    let e = conn
        .execute("SELECT 1/0", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("22012"));
    let e = conn
        .execute("SELECT 1", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("25P02"), "{e}");
    conn.rollback().await.unwrap();
    assert_eq!(rows_seen(&*conn).await, 3);

    // switching back to auto commits what is pending
    exec(&*conn, "INSERT INTO pxpg_tx VALUES (7)").await;
    conn.set_tx_mode(TxMode::Auto).await.unwrap();
    assert_eq!(rows_seen(&*other).await, 4);

    // commit and rollback with an open cursor end it; auto mode leaves it alone
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT g FROM generate_series(1, 100000) g",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    cursor.next_page(10).await.unwrap();
    conn.commit().await.unwrap();
    assert_eq!(
        cursor.next_page(10).await.unwrap().len(),
        10,
        "auto mode: unaffected"
    );
    cursor.close().await.unwrap();

    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT g FROM generate_series(1, 100000) g",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    cursor.next_page(10).await.unwrap();
    conn.commit().await.unwrap();
    let e = cursor.next_page(10).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled, "{e}");
    assert_eq!(row(&*conn, "SELECT 5", &[]).await[0], Value::Int(5));
    conn.rollback().await.unwrap();

    conn.set_tx_mode(TxMode::Auto).await.unwrap();
    exec(&*conn, "DROP TABLE pxpg_tx").await;
}

/// Closing a result early must not abort the user's transaction in a
/// manual-commit session (a cancel would).
#[tokio::test]
#[ignore]
async fn manual_mode_early_close_keeps_the_transaction() {
    let conn = connect().await;
    let other = connect().await;
    exec(&*conn, "DROP TABLE IF EXISTS pxpg_manual").await;
    exec(&*conn, "CREATE TABLE pxpg_manual (n int)").await;
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    let opts = ExecOptions::default();
    let big = "SELECT generate_series(1, 100000000) AS g";

    for i in 0..6_i64 {
        conn.execute(
            "INSERT INTO pxpg_manual VALUES ($1)",
            &[Value::Int(i)],
            &opts,
        )
        .await
        .unwrap();
        let Outcome::Rows(mut cursor) = conn.execute(big, &[], &opts).await.unwrap() else {
            panic!("expected rows");
        };
        let page = cursor.next_page(10).await.unwrap();
        assert_eq!(page.len(), 10);
        assert_eq!(page[0][0], Value::Int(1));
        if i % 2 == 0 {
            cursor.close().await.unwrap();
        } else {
            drop(cursor);
        }
        // the transaction is alive and holds the inserts so far
        let r = row(&*conn, "SELECT count(*) FROM pxpg_manual", &[]).await;
        assert_eq!(r[0], Value::Int(i + 1), "iteration {i}");
        // the SQL cursor was closed, not left behind
        let open = row(
            &*conn,
            "SELECT count(*) FROM pg_cursors \
             WHERE statement LIKE '%generate_series(1, 100000000)%' \
             AND statement NOT LIKE '%pg_cursors%'",
            &[],
        )
        .await;
        assert_eq!(open[0], Value::Int(0), "iteration {i}");
    }
    assert_eq!(
        row(&*other, "SELECT count(*) FROM pxpg_manual", &[]).await[0],
        Value::Int(0),
        "nothing was committed yet"
    );
    conn.commit().await.unwrap();
    assert_eq!(
        row(&*other, "SELECT count(*) FROM pxpg_manual", &[]).await[0],
        Value::Int(6)
    );

    conn.set_tx_mode(TxMode::Auto).await.unwrap();
    exec(&*conn, "DROP TABLE pxpg_manual").await;
}

#[tokio::test]
#[ignore]
async fn manual_mode_pages_through_100k_rows() {
    let conn = connect().await;
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT g AS id, 'row ' || g AS label FROM generate_series(1, $1::int) g",
            &[Value::Int(100_000)],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    assert_eq!(cursor.columns().len(), 2);
    let mut expected = 1_i64;
    let mut pages = 0;
    loop {
        let page = cursor.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        for row in &page {
            assert_eq!(row[0], Value::Int(expected));
            assert_eq!(row[1], Value::Text(format!("row {expected}")));
            expected += 1;
        }
        pages += 1;
        if cursor.is_exhausted() {
            break;
        }
    }
    assert_eq!(expected, 100_001);
    // the end of the result is known as soon as the last row is delivered
    assert_eq!(pages, 200);
    assert!(cursor.is_exhausted());
    // exhausted: the connection is free, and the transaction goes on
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
    cursor.close().await.unwrap();
    conn.rollback().await.unwrap();

    // collect() reports a result that ends exactly at the limit as complete
    let rs = conn
        .query("SELECT g FROM generate_series(1, 7) g", &[], 7)
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 7);
    assert!(!rs.truncated);
    let rs = conn
        .query("SELECT g FROM generate_series(1, 8) g", &[], 7)
        .await
        .unwrap();
    assert!(rs.truncated);
    conn.rollback().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn manual_mode_errors_and_timeouts() {
    let conn = connect().await;
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    let opts = ExecOptions::default();

    // a runtime error in a later page: the pages before it arrive first (a
    // page that fails is lost as a whole, the server reports only the error)
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT 10 / (1200 - g) FROM generate_series(1, 2000) g",
            &[],
            &opts,
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    let mut delivered = 0;
    let e = loop {
        match cursor.next_page(500).await {
            Ok(page) => delivered += page.len(),
            Err(e) => break e,
        }
    };
    assert_eq!(e.code.as_deref(), Some("22012"));
    assert!((1000..1200).contains(&delivered), "{delivered}");
    assert!(cursor.next_page(500).await.is_err());
    drop(cursor);
    // the failed statement aborted the transaction, as in any PostgreSQL session
    let e = conn.query("SELECT 1", &[], 1).await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("25P02"), "{e}");
    conn.rollback().await.unwrap();
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));

    // an error in the first row surfaces from execute
    let e = conn.execute("SELECT 1/0", &[], &opts).await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("22012"));
    conn.rollback().await.unwrap();

    // a time limit ends a slow first fetch
    let quick = ExecOptions {
        timeout: Some(Duration::from_millis(500)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let e = conn
        .execute("SELECT pg_sleep(30)", &[], &quick)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout, "{e}");
    assert!(started.elapsed() < Duration::from_secs(5));
    conn.rollback().await.unwrap();
    assert_eq!(row(&*conn, "SELECT 2", &[]).await[0], Value::Int(2));

    // a binding error leaves nothing behind
    let e = conn
        .execute("SELECT $1::int2", &[Value::Int(70_000)], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    conn.rollback().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn read_only_sessions_reject_writes() {
    let mut p = params();
    p.read_only = true;
    let conn = PostgresDriver.connect(&p).await.unwrap();
    assert_eq!(
        row(&*conn, "SHOW default_transaction_read_only", &[]).await[0],
        Value::from("on")
    );
    let e = conn
        .execute("CREATE TABLE pxpg_ro (n int)", &[], &ExecOptions::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("25006"), "{e}");
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
}

#[tokio::test]
#[ignore]
async fn wrong_password_is_an_auth_error_without_the_password() {
    let mut p = params();
    p.password = Secret::new("definitely-wrong-password-1234");
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Auth, "{e}");
    assert_eq!(e.code.as_deref(), Some("28P01"));
    assert!(!e.message.contains("definitely-wrong-password-1234"));
    assert!(!format!("{e:?}").contains("definitely-wrong-password-1234"));
}

#[tokio::test]
#[ignore]
async fn unknown_database_is_a_connect_error() {
    let mut p = params();
    p.database = "pxpg_no_such_database".into();
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Connect, "{e}");
    assert_eq!(e.code.as_deref(), Some("3D000"));
}

#[tokio::test]
#[ignore]
async fn unreachable_servers_are_network_errors() {
    // a port nothing listens on
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let p = ConnectParams {
        host: "127.0.0.1".into(),
        port,
        user: "pyoxide".into(),
        database: "pyoxide".into(),
        password: Secret::new("pw-for-closed-port"),
        connect_timeout: Duration::from_secs(3),
        ..Default::default()
    };
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Network, "{e}");
    assert!(!e.message.contains("pw-for-closed-port"));

    // a name that does not resolve
    let p = ConnectParams {
        host: "pxpg-no-such-host.invalid".into(),
        port: 5432,
        connect_timeout: Duration::from_secs(5),
        ..p
    };
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Network, "{e}");
}

#[tokio::test]
#[ignore]
async fn a_silent_server_hits_the_connect_timeout() {
    // accepts TCP connections and never answers
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _hold = tokio::spawn(async move {
        let mut sockets = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            sockets.push(s);
        }
    });
    let p = ConnectParams {
        host: "127.0.0.1".into(),
        port,
        user: "pyoxide".into(),
        database: "pyoxide".into(),
        connect_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let started = Instant::now();
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Network, "{e}");
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[tokio::test]
#[ignore]
async fn tls_against_a_server_without_tls() {
    // Require must fail when the server offers no TLS
    let mut p = params();
    p.tls.mode = TlsMode::Require;
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Tls, "{e}");

    // Prefer falls back to plain text
    p.tls.mode = TlsMode::Prefer;
    let conn = PostgresDriver.connect(&p).await.unwrap();
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));
    // cancel works over the same fallback
    conn.cancel().await.unwrap();

    // Disable is explicit plain text
    p.tls.mode = TlsMode::Disable;
    PostgresDriver.connect(&p).await.unwrap();
}

/// Runs only with a TLS-enabled server, see the module documentation.
#[tokio::test]
#[ignore]
async fn tls_modes_against_a_tls_server() {
    let (Ok(port), Ok(ca)) = (
        std::env::var("PYOXIDE_TEST_PG_TLS_PORT"),
        std::env::var("PYOXIDE_TEST_PG_TLS_CA"),
    ) else {
        eprintln!("skipped: PYOXIDE_TEST_PG_TLS_PORT / PYOXIDE_TEST_PG_TLS_CA not set");
        return;
    };
    let mut p = params();
    p.port = port.parse().expect("TLS port");
    p.tls.ca_file = Some(ca.into());

    for mode in [
        TlsMode::Prefer,
        TlsMode::Require,
        TlsMode::VerifyCa,
        TlsMode::VerifyFull,
    ] {
        p.tls.mode = mode;
        let conn = PostgresDriver
            .connect(&p)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        let r = row(
            &*conn,
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await;
        assert_eq!(r[0], Value::Bool(true), "{mode:?} must use TLS");
        // the cancel request is also sent over TLS
        let conn: Arc<dyn Connection> = Arc::from(conn);
        let c2 = Arc::clone(&conn);
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c2.cancel().await
        });
        let e = conn.query("SELECT pg_sleep(30)", &[], 1).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Cancelled, "{mode:?}: {e}");
        canceller.await.unwrap().unwrap();
    }

    // The expected host name comes from server_name when it is set: a name
    // the certificate does not carry fails VerifyFull, but not VerifyCa.
    p.host = "127.0.0.1".into();
    p.tls.server_name = Some("wrong.example.test".into());
    p.tls.mode = TlsMode::VerifyFull;
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Tls, "{e}");
    p.tls.mode = TlsMode::VerifyCa;
    PostgresDriver
        .connect(&p)
        .await
        .expect("verify-ca ignores the name");
    p.tls.mode = TlsMode::VerifyFull;
    p.tls.server_name = Some("localhost".into());
    PostgresDriver
        .connect(&p)
        .await
        .expect("verify-full with a matching server_name");

    // without the CA file the chain is not trusted
    p.tls.ca_file = None;
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Tls, "{e}");
}

/// Runs only with a TLS server that requires client certificates for a role,
/// see the module documentation.
#[tokio::test]
#[ignore]
async fn mutual_tls_with_a_client_certificate() {
    let (Ok(port), Ok(ca), Ok(cert), Ok(key)) = (
        std::env::var("PYOXIDE_TEST_PG_TLS_PORT"),
        std::env::var("PYOXIDE_TEST_PG_TLS_CA"),
        std::env::var("PYOXIDE_TEST_PG_TLS_CLIENT_CERT"),
        std::env::var("PYOXIDE_TEST_PG_TLS_CLIENT_KEY"),
    ) else {
        eprintln!("skipped: the PYOXIDE_TEST_PG_TLS_* variables are not set");
        return;
    };
    let user = env_or("PYOXIDE_TEST_PG_TLS_CLIENT_USER", "pyoxide_cert");
    let mut p = params();
    p.port = port.parse().expect("TLS port");
    p.user = user.clone();
    p.password = Secret::default();
    p.tls.mode = TlsMode::VerifyFull;
    p.tls.ca_file = Some(ca.into());
    p.tls.client_cert = Some(cert.into());
    p.tls.client_key = Some(key.into());

    let conn = PostgresDriver.connect(&p).await.expect("mutual TLS");
    let r = row(&*conn, "SELECT current_user::text", &[]).await;
    assert_eq!(r[0], Value::from(user.as_str()));

    // without the client certificate the server refuses the login
    p.tls.client_cert = None;
    p.tls.client_key = None;
    let e = PostgresDriver.connect(&p).await.err().unwrap();
    assert!(
        matches!(e.kind, ErrorKind::Auth | ErrorKind::Tls),
        "unexpected {e:?}"
    );
}

#[tokio::test]
#[ignore]
async fn long_values_are_truncated() {
    let conn = connect().await;
    let opts = ExecOptions {
        max_value_bytes: 1024,
        ..ExecOptions::default()
    };
    let rs = db_core::collect(
        match conn
            .execute(
                "SELECT repeat('é', 5000), repeat('a', 1024), '\\x'::bytea || decode(repeat('ab', 3000), 'hex'), \
                 ('{\"k\": \"' || repeat('v', 3000) || '\"}')::jsonb, 'short'",
                &[],
                &opts,
            )
            .await
            .unwrap()
        {
            Outcome::Rows(c) => c,
            other => panic!("{other:?}"),
        },
        10,
        10,
    )
    .await
    .unwrap();
    let r = &rs.rows[0];
    match &r[0] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(*full_len, Some(10_000));
            let Value::Text(s) = &**prefix else { panic!() };
            assert_eq!(s.len(), 1024);
            assert!(s.chars().all(|c| c == 'é'));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&r[1], Value::Text(s) if s.len() == 1024),
        "{:?}",
        r[1]
    );
    match &r[2] {
        Value::Truncated { prefix, full_len } => {
            assert_eq!(*full_len, Some(3000));
            assert!(matches!(&**prefix, Value::Bytes(b) if b.len() == 1024));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&r[3], Value::Truncated { prefix, .. } if matches!(&**prefix, Value::Json(_)))
    );
    assert_eq!(r[4], Value::from("short"));
}

#[tokio::test]
#[ignore]
async fn errors_surface_with_sqlstate() {
    let conn = connect().await;
    let opts = ExecOptions::default();
    let e = conn.execute("SELEC 1", &[], &opts).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Query);
    assert_eq!(e.code.as_deref(), Some("42601"));

    // a runtime error in the first row is raised by execute
    let e = conn.execute("SELECT 1/0", &[], &opts).await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("22012"));

    // ... and one later in the result by the page that reaches it, after the
    // rows before it were delivered
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT 10 / (5 - g) FROM generate_series(1, 10) g",
            &[],
            &opts,
        )
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    let page = cursor.next_page(100).await.unwrap();
    assert_eq!(page.len(), 4);
    let e = cursor.next_page(100).await.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("22012"));
    assert!(cursor.next_page(100).await.is_err());
    assert_eq!(row(&*conn, "SELECT 1", &[]).await[0], Value::Int(1));

    exec(&*conn, "DROP TABLE IF EXISTS pxpg_err").await;
    exec(&*conn, "CREATE TABLE pxpg_err (n int PRIMARY KEY)").await;
    exec(&*conn, "INSERT INTO pxpg_err VALUES (1)").await;
    let e = conn
        .execute("INSERT INTO pxpg_err VALUES (1)", &[], &opts)
        .await
        .unwrap_err();
    assert_eq!(e.code.as_deref(), Some("23505"));
    assert!(e.message.contains("duplicate key"), "{}", e.message);
    exec(&*conn, "DROP TABLE pxpg_err").await;

    // a terminated backend is reported as a lost connection
    let victim = connect().await;
    let pid = row(&*victim, "SELECT pg_backend_pid()", &[]).await[0]
        .as_i64()
        .unwrap();
    let r = row(
        &*conn,
        "SELECT pg_terminate_backend($1::int4)",
        &[Value::Int(pid)],
    )
    .await;
    assert_eq!(r[0], Value::Bool(true));
    let e = victim.query("SELECT 1", &[], 1).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConnectionLost, "{e}");
    let e = victim.ping().await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConnectionLost, "{e}");
}

#[tokio::test]
#[ignore]
async fn statements_with_several_commands_run_as_a_script() {
    let conn = connect().await;
    exec(&*conn, "DROP TABLE IF EXISTS pxpg_script").await;
    let out = conn
        .execute(
            "CREATE TABLE pxpg_script (n int); INSERT INTO pxpg_script VALUES (1), (2); \
             INSERT INTO pxpg_script VALUES (3);",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(3)), "{out:?}");
    assert_eq!(
        row(&*conn, "SELECT count(*) FROM pxpg_script", &[]).await[0],
        Value::Int(3)
    );
    exec(&*conn, "DROP TABLE pxpg_script").await;
}

#[tokio::test]
#[ignore]
async fn unicode_text_and_empty_statements() {
    let conn = connect().await;
    let r = row(&*conn, "SELECT $1::text", &[Value::from("日本語 ✓ 🎉")]).await;
    assert_eq!(r[0], Value::from("日本語 ✓ 🎉"));
    let out = conn
        .execute("", &[], &ExecOptions::default())
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(0)), "{out:?}");
    let out = conn
        .execute("-- just a comment", &[], &ExecOptions::default())
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Affected(0)), "{out:?}");
}

#[tokio::test]
#[ignore]
async fn introspection_of_a_test_schema() {
    let conn = connect().await;
    exec(&*conn, "DROP SCHEMA IF EXISTS pxpg_intro CASCADE").await;
    for sql in [
        "CREATE SCHEMA pxpg_intro",
        "CREATE TABLE pxpg_intro.parent (id serial PRIMARY KEY, code text NOT NULL UNIQUE, \
         note text DEFAULT 'n/a')",
        "COMMENT ON TABLE pxpg_intro.parent IS 'parent table'",
        "COMMENT ON COLUMN pxpg_intro.parent.code IS 'business code'",
        "CREATE TABLE pxpg_intro.\"Child Table\" (id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, \
         parent_id int NOT NULL REFERENCES pxpg_intro.parent (id) ON DELETE CASCADE, \
         qty numeric(10,2) CHECK (qty > 0), label varchar(40))",
        "CREATE INDEX pxpg_intro_label_idx ON pxpg_intro.\"Child Table\" (lower(label), qty)",
        "COMMENT ON INDEX pxpg_intro.pxpg_intro_label_idx IS 'label lookups'",
        "CREATE VIEW pxpg_intro.v_child AS SELECT id, label FROM pxpg_intro.\"Child Table\"",
        "CREATE MATERIALIZED VIEW pxpg_intro.mv_total AS SELECT 1 AS x",
        "CREATE SEQUENCE pxpg_intro.seq_a",
        "CREATE FUNCTION pxpg_intro.fn_add(a int, b int) RETURNS int LANGUAGE sql IMMUTABLE \
         AS 'SELECT a + b'",
        "CREATE FUNCTION pxpg_intro.fn_add(a numeric, b numeric) RETURNS numeric LANGUAGE sql \
         IMMUTABLE AS 'SELECT a + b'",
        "CREATE PROCEDURE pxpg_intro.pr_noop() LANGUAGE sql AS 'SELECT 1'",
        "CREATE FUNCTION pxpg_intro.trg_fn() RETURNS trigger LANGUAGE plpgsql \
         AS 'BEGIN RETURN NEW; END'",
        "CREATE TRIGGER trg_child_ins BEFORE INSERT ON pxpg_intro.\"Child Table\" \
         FOR EACH ROW EXECUTE FUNCTION pxpg_intro.trg_fn()",
        "CREATE TYPE pxpg_intro.mood AS ENUM ('sad', 'ok')",
        "CREATE TYPE pxpg_intro.pair AS (a int, b text)",
        "CREATE DOMAIN pxpg_intro.posint AS int CHECK (VALUE > 0)",
        "CREATE TABLE pxpg_intro.part_root (d date) PARTITION BY RANGE (d)",
    ] {
        exec(&*conn, sql).await;
    }

    let meta = introspector(&*conn);

    assert_eq!(meta.current_schema().await.unwrap(), "public");

    let schemas = meta.schemas().await.unwrap();
    let find = |name: &str| schemas.iter().find(|s| s.name == name).unwrap();
    assert!(find("pg_catalog").is_system);
    assert!(find("information_schema").is_system);
    assert!(find("pg_toast").is_system);
    assert!(!find("pxpg_intro").is_system);
    assert!(!find("pxpg_intro").is_current);
    assert!(find("public").is_current);

    let objects = meta.objects("pxpg_intro", &[]).await.unwrap();
    let has = |kind: ObjectKind, name: &str| {
        objects
            .iter()
            .find(|o| o.kind == kind && o.name == name)
            .unwrap_or_else(|| panic!("missing {kind:?} {name}: {objects:#?}"))
    };
    assert_eq!(
        has(ObjectKind::Table, "parent").comment.as_deref(),
        Some("parent table")
    );
    has(ObjectKind::Table, "Child Table");
    has(ObjectKind::Table, "part_root");
    has(ObjectKind::View, "v_child");
    has(ObjectKind::MaterializedView, "mv_total");
    has(ObjectKind::Sequence, "seq_a");
    has(ObjectKind::Sequence, "parent_id_seq");
    let idx = has(ObjectKind::Index, "pxpg_intro_label_idx");
    assert_eq!(idx.parent.as_deref(), Some("Child Table"));
    assert_eq!(idx.comment.as_deref(), Some("label lookups"));
    assert_eq!(
        has(ObjectKind::Index, "parent_pkey").parent.as_deref(),
        Some("parent")
    );
    let trg = has(ObjectKind::Trigger, "trg_child_ins");
    assert_eq!(trg.parent.as_deref(), Some("Child Table"));
    has(ObjectKind::Procedure, "pr_noop");
    has(ObjectKind::Function, "trg_fn");
    assert_eq!(
        objects
            .iter()
            .filter(|o| o.kind == ObjectKind::Function && o.name == "fn_add")
            .count(),
        2,
        "both overloads are listed"
    );
    has(ObjectKind::Type, "mood");
    has(ObjectKind::Type, "pair");
    has(ObjectKind::Type, "posint");
    // table row types are not listed as types
    assert!(
        !objects
            .iter()
            .any(|o| o.kind == ObjectKind::Type && o.name == "parent")
    );
    assert!(objects.iter().all(|o| o.schema == "pxpg_intro"));

    // filtering by kind
    let only_views = meta
        .objects(
            "pxpg_intro",
            &[ObjectKind::View, ObjectKind::MaterializedView],
        )
        .await
        .unwrap();
    let names: Vec<_> = only_views.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["mv_total", "v_child"]);
    let only_routines = meta
        .objects("pxpg_intro", &[ObjectKind::Procedure])
        .await
        .unwrap();
    assert_eq!(only_routines.len(), 1);
    assert!(
        meta.objects("pxpg_no_such_schema", &[])
            .await
            .unwrap()
            .is_empty()
    );

    // columns
    let cols = meta.columns("pxpg_intro", "parent").await.unwrap();
    assert_eq!(cols.len(), 3);
    assert_eq!(cols[0].name, "id");
    assert_eq!(cols[0].position, 1);
    assert_eq!(cols[0].data_type, "integer");
    assert!(!cols[0].nullable);
    assert!(cols[0].auto_increment, "serial");
    assert!(cols[0].default.as_deref().unwrap().starts_with("nextval("));
    assert_eq!(cols[1].name, "code");
    assert!(!cols[1].nullable);
    assert_eq!(cols[1].comment.as_deref(), Some("business code"));
    assert_eq!(cols[2].default.as_deref(), Some("'n/a'::text"));
    assert!(cols[2].nullable);
    assert!(!cols[2].auto_increment);

    let cols = meta.columns("pxpg_intro", "Child Table").await.unwrap();
    assert!(cols[0].auto_increment, "identity");
    assert_eq!(cols[0].default, None);
    assert_eq!(cols[2].data_type, "numeric(10,2)");
    assert_eq!(cols[3].data_type, "character varying(40)");
    let view_cols = meta.columns("pxpg_intro", "v_child").await.unwrap();
    assert_eq!(view_cols.len(), 2);
    assert!(
        meta.columns("pxpg_intro", "no_such_table")
            .await
            .unwrap()
            .is_empty()
    );

    // indexes
    let indexes = meta.indexes("pxpg_intro", "Child Table").await.unwrap();
    let label = indexes
        .iter()
        .find(|i| i.name == "pxpg_intro_label_idx")
        .unwrap();
    assert!(!label.unique);
    assert_eq!(label.method.as_deref(), Some("btree"));
    assert_eq!(label.columns, ["lower(label::text)", "qty"]);
    let pk = indexes
        .iter()
        .find(|i| i.name == "Child Table_pkey")
        .unwrap();
    assert!(pk.unique);
    assert_eq!(pk.columns, ["id"]);
    let parent_idx = meta.indexes("pxpg_intro", "parent").await.unwrap();
    assert_eq!(parent_idx.len(), 2);
    assert!(parent_idx.iter().all(|i| i.unique));

    // constraints
    let cons = meta.constraints("pxpg_intro", "Child Table").await.unwrap();
    let kinds: Vec<_> = cons.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ConstraintKind::PrimaryKey,
            ConstraintKind::ForeignKey,
            ConstraintKind::Check
        ]
    );
    assert_eq!(cons[0].columns, ["id"]);
    let fk = &cons[1];
    assert_eq!(fk.columns, ["parent_id"]);
    assert_eq!(fk.ref_schema.as_deref(), Some("pxpg_intro"));
    assert_eq!(fk.ref_table.as_deref(), Some("parent"));
    assert_eq!(fk.ref_columns, ["id"]);
    assert_eq!(fk.on_delete.as_deref(), Some("CASCADE"));
    assert!(fk.check.is_none());
    let check = &cons[2];
    assert_eq!(check.columns, ["qty"]);
    assert!(
        check.check.as_deref().unwrap().contains("qty > 0"),
        "{:?}",
        check.check
    );
    let cons = meta.constraints("pxpg_intro", "parent").await.unwrap();
    let kinds: Vec<_> = cons.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [ConstraintKind::PrimaryKey, ConstraintKind::Unique]);
    assert_eq!(cons[1].columns, ["code"]);

    // source
    let view = meta
        .source("pxpg_intro", "v_child", ObjectKind::View)
        .await
        .unwrap()
        .unwrap();
    assert!(
        view.starts_with("CREATE OR REPLACE VIEW pxpg_intro.v_child AS"),
        "{view}"
    );
    assert!(view.contains("SELECT"), "{view}");
    let mv = meta
        .source("pxpg_intro", "mv_total", ObjectKind::MaterializedView)
        .await
        .unwrap()
        .unwrap();
    assert!(
        mv.starts_with("CREATE MATERIALIZED VIEW pxpg_intro.mv_total AS"),
        "{mv}"
    );
    let func = meta
        .source("pxpg_intro", "fn_add", ObjectKind::Function)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        func.matches("CREATE OR REPLACE FUNCTION pxpg_intro.fn_add")
            .count(),
        2,
        "{func}"
    );
    let proc = meta
        .source("pxpg_intro", "pr_noop", ObjectKind::Procedure)
        .await
        .unwrap()
        .unwrap();
    assert!(
        proc.contains("CREATE OR REPLACE PROCEDURE pxpg_intro.pr_noop"),
        "{proc}"
    );
    assert!(
        meta.source("pxpg_intro", "pr_noop", ObjectKind::Function)
            .await
            .unwrap()
            .is_none()
    );
    let trigger = meta
        .source("pxpg_intro", "trg_child_ins", ObjectKind::Trigger)
        .await
        .unwrap()
        .unwrap();
    assert!(
        trigger.contains("CREATE TRIGGER trg_child_ins BEFORE INSERT"),
        "{trigger}"
    );
    let index = meta
        .source("pxpg_intro", "pxpg_intro_label_idx", ObjectKind::Index)
        .await
        .unwrap()
        .unwrap();
    assert!(
        index.starts_with("CREATE INDEX pxpg_intro_label_idx"),
        "{index}"
    );
    assert!(
        meta.source("pxpg_intro", "parent", ObjectKind::Table)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        meta.source("pxpg_intro", "does_not_exist", ObjectKind::View)
            .await
            .unwrap()
            .is_none()
    );

    // names are bound, never spliced
    assert!(
        meta.columns("pxpg_intro", "x' OR '1'='1")
            .await
            .unwrap()
            .is_empty()
    );

    // the same answers in a manual-commit session, where results are paged
    // through SQL cursors
    let expected_objects = meta.objects("pxpg_intro", &[]).await.unwrap();
    let expected_columns = meta.columns("pxpg_intro", "Child Table").await.unwrap();
    let expected_constraints = meta.constraints("pxpg_intro", "Child Table").await.unwrap();
    drop(meta);
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    let meta = introspector(&*conn);
    assert_eq!(
        meta.objects("pxpg_intro", &[]).await.unwrap(),
        expected_objects
    );
    assert_eq!(
        meta.columns("pxpg_intro", "Child Table").await.unwrap(),
        expected_columns
    );
    assert_eq!(
        meta.constraints("pxpg_intro", "Child Table").await.unwrap(),
        expected_constraints
    );
    drop(meta);
    conn.rollback().await.unwrap();
    conn.set_tx_mode(TxMode::Auto).await.unwrap();

    exec(&*conn, "DROP SCHEMA pxpg_intro CASCADE").await;
}

/// Fetch 100,000 rows in pages of 500; returns (first page, total) times.
async fn fetch_100k(conn: &dyn Connection) -> (Duration, Duration) {
    let sql = "SELECT g::int4, 'row number ' || g, \
               TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * INTERVAL '1 second', \
               (g * 1.25)::numeric(20,4) \
               FROM generate_series(1, 100000) g";
    let started = Instant::now();
    let Outcome::Rows(mut cursor) = conn
        .execute(sql, &[], &ExecOptions::default())
        .await
        .unwrap()
    else {
        panic!("expected rows");
    };
    let first = cursor.next_page(500).await.unwrap();
    let first_page = started.elapsed();
    assert_eq!(first.len(), 500);

    let mut total = first.len();
    loop {
        let page = cursor.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        total += page.len();
    }
    assert_eq!(total, 100_000);
    (first_page, started.elapsed())
}

/// Measurement: `cargo test --release -p db-postgres -- --ignored bench_fetch_100k --nocapture`
#[tokio::test]
#[ignore]
async fn bench_fetch_100k() {
    let started = Instant::now();
    let conn = connect().await;
    let connect_time = started.elapsed();

    // warm up the type and statement paths with a throw-away query
    conn.query("SELECT 1", &[], 1).await.unwrap();

    let (first_page, elapsed) = fetch_100k(&*conn).await;
    let rows_per_second = 100_000_f64 / elapsed.as_secs_f64();
    println!("connect:              {connect_time:?}");
    println!("first page (500):     {first_page:?}");
    println!("100000 rows total:    {elapsed:?}");
    println!("throughput:           {rows_per_second:.0} rows/s");

    // a manual-commit session pages through an SQL cursor (one FETCH per page)
    conn.set_tx_mode(TxMode::Manual).await.unwrap();
    fetch_100k(&*conn).await;
    let (first_page, elapsed) = fetch_100k(&*conn).await;
    let rows_per_second = 100_000_f64 / elapsed.as_secs_f64();
    println!("manual mode, first page (500):  {first_page:?}");
    println!("manual mode, 100000 rows total: {elapsed:?}");
    println!("manual mode, throughput:        {rows_per_second:.0} rows/s");
    conn.rollback().await.unwrap();
}
