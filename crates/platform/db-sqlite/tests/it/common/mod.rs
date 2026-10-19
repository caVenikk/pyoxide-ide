#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use db_core::{ConnectParams, Connection, Driver, ExecOptions, ResultSet, Value, opt};
use db_sqlite::SqliteDriver;

/// A recursive CTE that takes far longer than any test waits.
pub const SLOW_COUNT: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                              WHERE x < 1000000000) SELECT count(*) FROM c";

/// First row at once, the second after scanning a billion numbers.
pub const SLOW_SECOND_ROW: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                                   WHERE x < 1000000000) \
                                   SELECT x FROM c WHERE x = 1 OR x = 1000000000";

pub fn memory_params() -> ConnectParams {
    ConnectParams {
        database: ":memory:".into(),
        ..ConnectParams::default()
    }
}

pub fn file_params(path: &Path) -> ConnectParams {
    ConnectParams {
        database: path.to_string_lossy().into_owned(),
        ..ConnectParams::default()
    }
}

pub fn with_create(mut params: ConnectParams) -> ConnectParams {
    params
        .options
        .insert(opt::SQLITE_CREATE.into(), "true".into());
    params
}

pub async fn open(params: &ConnectParams) -> Arc<dyn Connection> {
    Arc::from(SqliteDriver.connect(params).await.expect("connect"))
}

pub async fn memory() -> Arc<dyn Connection> {
    open(&memory_params()).await
}

/// A new database file in `dir`, opened for reading and writing.
pub async fn new_file(dir: &Path, name: &str) -> Arc<dyn Connection> {
    open(&with_create(file_params(&dir.join(name)))).await
}

pub async fn run(conn: &Arc<dyn Connection>, sql: &str) {
    conn.execute(sql, &[], &ExecOptions::default())
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

pub async fn rows(conn: &Arc<dyn Connection>, sql: &str, params: &[Value]) -> ResultSet {
    conn.query(sql, params, usize::MAX)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

pub async fn scalar(conn: &Arc<dyn Connection>, sql: &str) -> Value {
    rows(conn, sql, &[]).await.rows[0][0].clone()
}
