//! Pure-Rust Oracle backend (TNS protocol, no Oracle Client libraries).

use async_trait::async_trait;
use db_core::{Cell, Column, ConnectParams, Connection, Driver, Error, ResultSet};
use oracle_rs::{Config, Value};

pub struct ThinDriver;

pub struct ThinConnection {
    inner: oracle_rs::Connection,
}

#[async_trait]
impl Driver for ThinDriver {
    async fn connect(p: &ConnectParams) -> db_core::Result<Box<dyn Connection>> {
        let cfg = Config::new(&p.host, p.port, &p.service, &p.user, &p.password);
        let inner = oracle_rs::Connection::connect_with_config(cfg)
            .await
            .map_err(|e| Error::Connect(e.to_string()))?;
        Ok(Box::new(ThinConnection { inner }))
    }
}

fn is_select(sql: &str) -> bool {
    let t = sql.trim_start().to_ascii_lowercase();
    t.starts_with("select") || t.starts_with("with") || t.starts_with('(')
}

fn cell(v: &Value) -> Cell {
    if v.is_null() {
        Cell::Null
    } else {
        Cell::Text(v.to_string())
    }
}

#[async_trait]
impl Connection for ThinConnection {
    fn backend(&self) -> &'static str {
        "oracle-thin"
    }

    async fn query(&self, sql: &str, max_rows: usize) -> db_core::Result<ResultSet> {
        let sql = sql.trim().trim_end_matches(';');
        let map = |e: oracle_rs::Error| Error::Query(e.to_string());

        if !is_select(sql) {
            let r = self.inner.execute(sql, &[]).await.map_err(map)?;
            return Ok(ResultSet {
                rows_affected: r.rows_affected,
                ..Default::default()
            });
        }

        let mut res = self.inner.query(sql, &[]).await.map_err(map)?;
        let columns: Vec<Column> = res
            .columns
            .iter()
            .map(|c| Column {
                name: c.name.clone(),
                type_name: format!("{:?}", c.oracle_type),
                nullable: c.nullable,
            })
            .collect();

        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut truncated = false;
        loop {
            for r in &res.rows {
                if rows.len() >= max_rows {
                    truncated = true;
                    break;
                }
                rows.push(r.values().iter().map(cell).collect());
            }
            if truncated || !res.has_more_rows {
                break;
            }
            let more = self
                .inner
                .fetch_more(res.cursor_id, &res.columns, 100)
                .await
                .map_err(map)?;
            res = more;
        }
        // `has_more_rows` after the last page we read also counts as truncated.
        Ok(ResultSet {
            columns,
            rows,
            rows_affected: 0,
            truncated,
        })
    }

    async fn close(&self) -> db_core::Result<()> {
        self.inner
            .close()
            .await
            .map_err(|e| Error::Query(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_detection() {
        assert!(is_select("  SELECT 1 FROM dual"));
        assert!(is_select("with x as (select 1 from dual) select * from x"));
        assert!(!is_select("update t set a = 1"));
    }

    /// Needs a running Oracle: `docker compose up -d`, then
    /// `PYOXIDE_TEST_ORACLE=1 cargo test -p db-oracle-thin -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn select_from_dual_against_docker() {
        let p = ConnectParams {
            host: "localhost".into(),
            port: 1521,
            service: "FREEPDB1".into(),
            user: "pyoxide".into(),
            password: std::env::var("PYOXIDE_DB_PASSWORD").unwrap_or_else(|_| "pyoxide".into()),
        };
        let c = ThinDriver::connect(&p).await.expect("connect");
        let rs = c.query("SELECT 1 AS n FROM dual", 10).await.expect("query");
        assert_eq!(rs.rows.len(), 1);
        c.close().await.ok();
    }
}
