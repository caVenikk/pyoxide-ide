//! Shared setup for the Oracle integration tests (see
//! docs/testing-databases.md): `docker compose up -d oracle`, then
//! `cargo test -p db-oracle-thin -- --ignored`.

#![allow(dead_code)]

use db_core::{ConnectParams, Connection, Driver, ExecOptions, Outcome, ResultSet, Secret, Value};
use db_oracle_thin::ThinDriver;

pub fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

pub fn params() -> ConnectParams {
    ConnectParams {
        host: env("PYOXIDE_TEST_ORACLE_HOST", "localhost"),
        port: env("PYOXIDE_TEST_ORACLE_PORT", "1521")
            .parse()
            .unwrap_or(1521),
        database: env("PYOXIDE_TEST_ORACLE_SERVICE", "FREEPDB1"),
        user: "pyoxide".into(),
        password: Secret::new(env("PYOXIDE_DB_PASSWORD", "pyoxide")),
        ..Default::default()
    }
}

pub async fn connect() -> Box<dyn Connection> {
    ThinDriver.connect(&params()).await.expect("connect")
}

pub async fn query(c: &dyn Connection, sql: &str, params: &[Value]) -> ResultSet {
    c.query(sql, params, 1_000_000)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
}

pub async fn exec(c: &dyn Connection, sql: &str, params: &[Value]) -> u64 {
    match c
        .execute(sql, params, &ExecOptions::default())
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"))
    {
        Outcome::Affected(n) => n,
        Outcome::Rows(_) => panic!("{sql}: unexpected rows"),
    }
}

/// Run DDL, ignoring errors (cleanup of objects that may not exist).
pub async fn try_exec(c: &dyn Connection, sql: &str) {
    let _ = c.execute(sql, &[], &ExecOptions::default()).await;
}

/// A unique object name prefix per test run.
pub fn prefix(test: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("PXTHIN_{}_{:X}_", test.to_ascii_uppercase(), nanos % 0xFFFF)
}
