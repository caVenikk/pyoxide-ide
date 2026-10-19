use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::common::{file_params, memory_params, open, run, with_create};
use db_core::{ConnectParams, Connection, Driver, ExecOptions, Outcome};
use db_sqlite::SqliteDriver;

const ROWS: usize = 100_000;
const PAGE: usize = 500;
const RUNS: usize = 25;

const FILL: &str = "INSERT INTO bench \
    WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 100000) \
    SELECT x, 'name ' || x, x * 0.25, \
           strftime('%Y-%m-%d %H:%M:%f', 1767225600 + x, 'unixepoch') FROM c";

async fn fill(conn: &Arc<dyn Connection>, timestamp_type: &str) {
    run(
        conn,
        &format!(
            "CREATE TABLE bench(id INTEGER PRIMARY KEY, name TEXT, score REAL, created {timestamp_type})"
        ),
    )
    .await;
    run(conn, FILL).await;
}

struct Timing {
    first_page: Duration,
    total: Duration,
}

/// Execute, read the first page, then the rest, in pages of `PAGE` rows.
async fn fetch_all(conn: &Arc<dyn Connection>) -> Timing {
    let started = Instant::now();
    let Outcome::Rows(mut cursor) = conn
        .execute(
            "SELECT id, name, score, created FROM bench",
            &[],
            &ExecOptions::default(),
        )
        .await
        .unwrap()
    else {
        panic!("rows expected");
    };
    let first = cursor.next_page(PAGE).await.unwrap();
    assert_eq!(first.len(), PAGE);
    let first_page = started.elapsed();
    let mut rows = first.len();
    while !cursor.is_exhausted() {
        rows += cursor.next_page(PAGE).await.unwrap().len();
    }
    let total = started.elapsed();
    assert_eq!(rows, ROWS);
    cursor.close().await.unwrap();
    Timing { first_page, total }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn best(v: &[Duration]) -> Duration {
    v.iter().copied().min().unwrap_or_default()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

async fn report(label: &str, params: &ConnectParams, timestamp_type: &str) {
    let in_memory = params.database == ":memory:";
    let mut opens = Vec::new();
    let mut firsts = Vec::new();
    let mut totals = Vec::new();
    // An in-memory database lives and dies with its connection, so one
    // connection holds the data for all runs; a file is opened fresh each run.
    let mut data: Option<Arc<dyn Connection>> = None;
    if in_memory {
        let c = open(params).await;
        fill(&c, timestamp_type).await;
        data = Some(c);
    }
    for _ in 0..RUNS {
        let started = Instant::now();
        let fresh: Arc<dyn Connection> = Arc::from(SqliteDriver.connect(params).await.unwrap());
        opens.push(started.elapsed());
        let t = if let Some(c) = &data {
            fetch_all(c).await
        } else {
            fetch_all(&fresh).await
        };
        firsts.push(t.first_page);
        totals.push(t.total);
        fresh.close().await.unwrap();
    }
    let (typical, fastest) = (median(totals.clone()), best(&totals));
    println!(
        "{label:<36} open {:>6.3} ms | first page of {PAGE} {:>6.3} ms | {ROWS} rows {:>6.2} ms (best {:>6.2}) | {:>5.2} M rows/s (best {:>5.2})",
        ms(median(opens)),
        ms(median(firsts)),
        ms(typical),
        ms(fastest),
        ROWS as f64 / typical.as_secs_f64() / 1e6,
        ROWS as f64 / fastest.as_secs_f64() / 1e6,
    );
}

/// Same query with plain rusqlite and no conversion: the floor for this data.
fn raw_rusqlite(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut times = Vec::new();
    for _ in 0..RUNS {
        let started = Instant::now();
        let mut stmt = conn
            .prepare("SELECT id, name, score, created FROM bench")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        let mut n = 0;
        let mut bytes = 0usize;
        while let Some(row) = rows.next().unwrap() {
            let id: i64 = row.get(0).unwrap();
            let name: String = row.get(1).unwrap();
            let score: f64 = row.get(2).unwrap();
            let created: String = row.get(3).unwrap();
            bytes += name.len() + created.len() + usize::from(id > 0) + usize::from(score > 0.0);
            n += 1;
        }
        assert_eq!(n, ROWS);
        assert!(bytes > 0);
        times.push(started.elapsed());
    }
    let (typical, fastest) = (median(times.clone()), best(&times));
    println!(
        "{:<36} {:>54} {:>6.2} ms (best {:>6.2}) | {:>5.2} M rows/s (best {:>5.2})",
        "raw rusqlite, no conversion",
        "",
        ms(typical),
        ms(fastest),
        ROWS as f64 / typical.as_secs_f64() / 1e6,
        ROWS as f64 / fastest.as_secs_f64() / 1e6,
    );
}

/// Run in release mode:
/// `cargo test -p db-sqlite --release --test it bench:: -- --ignored --nocapture`
#[tokio::test]
#[ignore = "measurement, run on demand"]
async fn bench_fetch_100k() {
    println!();
    println!(
        "SQLite {}, {ROWS} rows of (int, text, real, timestamp text), pages of {PAGE}, median of {RUNS} runs",
        rusqlite::version()
    );

    let dir = tempfile::tempdir().unwrap();
    for (label, ts_type) in [
        ("file, timestamp column typed", "TIMESTAMP"),
        ("file, timestamp column plain text", "TEXT"),
    ] {
        let path = dir.path().join(format!("{}.db", ts_type.to_lowercase()));
        let setup = open(&with_create(file_params(&path))).await;
        fill(&setup, ts_type).await;
        setup.close().await.unwrap();
        report(label, &file_params(&path), ts_type).await;
        if ts_type == "TEXT" {
            raw_rusqlite(&path);
        }
    }
    for (label, ts_type) in [
        ("memory, timestamp column typed", "TIMESTAMP"),
        ("memory, timestamp column plain text", "TEXT"),
    ] {
        report(label, &memory_params(), ts_type).await;
    }
}
