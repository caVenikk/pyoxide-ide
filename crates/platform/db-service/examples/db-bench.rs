//! Database layer benchmark, the same method for every backend.
//!
//!     cargo run --release -p db-service --example db-bench -- <profile> [rows] [runs]
//!
//! Generates `rows` rows (default 100,000) of (integer, text, timestamp,
//! decimal) on the server, so no test table is needed, and reports the
//! median of `runs` (default 5): connect, first page of 500, full fetch and
//! rows per second, plus metadata calls. Profiles and passwords are resolved
//! as in `pyoxide-db`.

use std::sync::Arc;
use std::time::Instant;

use db_core::{Connection, Dialect, ExecOptions, Outcome, schema};
use db_service::{DbService, Target, profiles};

fn generator(dialect: Dialect, n: usize) -> String {
    match dialect {
        Dialect::Oracle => format!(
            "SELECT LEVEL AS id, 'row ' || LEVEL AS name, \
             TIMESTAMP '2026-01-01 00:00:00' + NUMTODSINTERVAL(LEVEL, 'SECOND') AS ts, \
             CAST(LEVEL * 1.25 AS NUMBER(12,2)) AS amount FROM dual CONNECT BY LEVEL <= {n}"
        ),
        Dialect::Postgres => format!(
            "SELECT g AS id, 'row ' || g AS name, \
             TIMESTAMPTZ '2026-01-01 00:00:00+00' + g * INTERVAL '1 second' AS ts, \
             (g * 1.25)::numeric(12,2) AS amount FROM generate_series(1, {n}) g"
        ),
        Dialect::Sqlite => format!(
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < {n}) \
             SELECT n AS id, 'row ' || n AS name, datetime('2026-01-01', '+' || n || ' seconds') AS ts, \
             n * 1.25 AS amount FROM c"
        ),
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

struct Run {
    connect: f64,
    first_page: f64,
    total: f64,
    rows: usize,
}

async fn fetch_run(svc: &DbService, target: &Target, rows: usize) -> db_core::Result<Run> {
    let started = Instant::now();
    let opened = svc.open(target).await?;
    let connect = ms(started);
    let conn = opened.conn;
    let opts = ExecOptions {
        fetch_size: 500,
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let Outcome::Rows(mut cursor) = conn
        .execute(&generator(conn.dialect(), rows), &[], &opts)
        .await?
    else {
        return Err(db_core::Error::internal("generator returned no rows"));
    };
    let mut seen = cursor.next_page(500).await?.len();
    let first_page = ms(started);
    loop {
        let page = cursor.next_page(500).await?;
        if page.is_empty() {
            break;
        }
        seen += page.len();
    }
    let total = ms(started);
    cursor.close().await?;
    conn.close().await.ok();
    Ok(Run {
        connect,
        first_page,
        total,
        rows: seen,
    })
}

async fn metadata(conn: &dyn Connection) -> Vec<(String, f64, String)> {
    let intro = schema::introspector(conn);
    let mut out = Vec::new();
    let t = Instant::now();
    let current = intro.current_schema().await.unwrap_or_default();
    out.push(("current schema".into(), ms(t), String::new()));
    let t = Instant::now();
    let r = intro.schemas().await;
    out.push((
        "schemas".into(),
        ms(t),
        r.map(|s| format!("{} schemas", s.len()))
            .unwrap_or_else(|e| e.message),
    ));
    let t = Instant::now();
    let objects = intro.objects(&current, &[]).await;
    out.push((
        "objects of current schema".into(),
        ms(t),
        objects
            .as_ref()
            .map(|o| format!("{} objects", o.len()))
            .unwrap_or_else(|e| e.message.clone()),
    ));
    if let Ok(objects) = objects
        && let Some(table) = objects.iter().find(|o| o.kind == schema::ObjectKind::Table)
    {
        let t = Instant::now();
        let r = intro.columns(&current, &table.name).await;
        out.push((
            "columns of one table".into(),
            ms(t),
            r.map(|c| format!("{} columns", c.len()))
                .unwrap_or_else(|e| e.message),
        ));
    }
    out
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(profile) = args.next() else {
        eprintln!("usage: db-bench <profile> [rows] [runs]");
        std::process::exit(2);
    };
    let rows: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let runs: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5).max(1);
    let svc = DbService::new(
        profiles::default_path(),
        Arc::new(secrets::KeyringStore),
        None,
        secrets::process_env(),
    );
    let target = Target::Profile(profile.clone());

    let mut results = Vec::new();
    for _ in 0..runs {
        match fetch_run(&svc, &target, rows).await {
            Ok(r) => results.push(r),
            Err(e) => {
                eprintln!("error: {}", e.message);
                std::process::exit(1);
            }
        }
    }
    let backend = svc
        .resolve_target(&target)
        .map(|(_, p)| p.backend.to_string())
        .unwrap_or_default();
    let fetched = results[0].rows;
    let total = median(results.iter().map(|r| r.total).collect());
    println!("backend {backend}, {fetched} rows x 4 columns, median of {runs} runs");
    println!("| metric | value |\n|---|---|");
    println!(
        "| connect | {:.1} ms |",
        median(results.iter().map(|r| r.connect).collect())
    );
    println!(
        "| first page (500 rows) | {:.1} ms |",
        median(results.iter().map(|r| r.first_page).collect())
    );
    println!("| fetch all | {total:.1} ms |");
    println!(
        "| throughput | {:.0} rows/s |",
        fetched as f64 / (total / 1000.0)
    );

    match svc.open(&target).await {
        Ok(opened) => {
            for (what, t, detail) in metadata(opened.conn.as_ref()).await {
                println!("| {what} | {t:.1} ms ({detail}) |");
            }
            opened.conn.close().await.ok();
        }
        Err(e) => eprintln!("metadata: {}", e.message),
    }
}
