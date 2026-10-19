//! Fetch benchmark: `cargo test --release -p db-oracle-thin --test it bench:: --
//! --ignored --nocapture` against the Oracle container.

use std::time::Instant;

use crate::common::*;
use db_core::{Driver, ExecOptions, Outcome};
use db_oracle_thin::ThinDriver;

const SQL: &str = "SELECT LEVEL AS id, 'row ' || LEVEL AS name, \
    TIMESTAMP '2026-01-01 00:00:00' + NUMTODSINTERVAL(LEVEL, 'SECOND') AS ts, \
    CAST(LEVEL * 1.25 AS NUMBER(12,2)) AS amount FROM dual CONNECT BY LEVEL <= 100000";

#[tokio::test]
#[ignore]
async fn bench_fetch_100k() {
    let started = Instant::now();
    let c = ThinDriver.connect(&params()).await.expect("connect");
    let connect = started.elapsed();

    let started = Instant::now();
    let Outcome::Rows(mut cur) = c.execute(SQL, &[], &ExecOptions::default()).await.unwrap() else {
        panic!("expected rows");
    };
    let first = cur.next_page(500).await.unwrap();
    let first_page = started.elapsed();
    let mut rows = first.len();
    loop {
        let page = cur.next_page(500).await.unwrap();
        if page.is_empty() {
            break;
        }
        rows += page.len();
    }
    let total = started.elapsed();
    assert_eq!(rows, 100_000);
    println!(
        "oracle-thin: connect {:.1} ms, first page of 500 {:.1} ms, 100000 rows in {:.1} ms ({:.0} rows/s)",
        connect.as_secs_f64() * 1e3,
        first_page.as_secs_f64() * 1e3,
        total.as_secs_f64() * 1e3,
        rows as f64 / total.as_secs_f64()
    );
}
