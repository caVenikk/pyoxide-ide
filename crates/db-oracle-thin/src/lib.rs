//! Pure-Rust Oracle backend (TNS protocol, no Oracle Client libraries).

use async_trait::async_trait;
use db_core::{
    Backend, BusyFlag, Column, ConnectParams, Connection, Driver, Error, ErrorKind, ExecOptions,
    Outcome, Result, TxMode, Value, ValueKind, VecCursor,
};
use oracle_rs::Config;

pub struct ThinDriver;

pub struct ThinConnection {
    inner: oracle_rs::Connection,
    busy: BusyFlag,
}

#[async_trait]
impl Driver for ThinDriver {
    fn backend(&self) -> Backend {
        Backend::OracleThin
    }

    fn availability(&self) -> Result<String> {
        Ok("built-in (oracle-rs)".into())
    }

    async fn connect(&self, p: &ConnectParams) -> Result<Box<dyn Connection>> {
        let cfg = Config::new(&p.host, p.port, &p.database, &p.user, p.password.expose());
        let inner = oracle_rs::Connection::connect_with_config(cfg)
            .await
            .map_err(|e| {
                Error::new(ErrorKind::Connect, e.to_string()).scrub(p.password.expose())
            })?;
        Ok(Box::new(ThinConnection {
            inner,
            busy: BusyFlag::default(),
        }))
    }
}

fn is_select(sql: &str) -> bool {
    let t = sql.trim_start().to_ascii_lowercase();
    t.starts_with("select") || t.starts_with("with") || t.starts_with('(')
}

fn cell(v: &oracle_rs::Value) -> Value {
    if v.is_null() {
        Value::Null
    } else {
        Value::Text(v.to_string())
    }
}

fn query_err(e: oracle_rs::Error) -> Error {
    Error::query(e.to_string())
}

#[async_trait]
impl Connection for ThinConnection {
    fn backend(&self) -> Backend {
        Backend::OracleThin
    }

    // Interim implementation: materializes the whole result and binds no
    // parameters. Typed values, binds, paging and cancel follow.
    async fn execute(&self, sql: &str, params: &[Value], _opts: &ExecOptions) -> Result<Outcome> {
        self.busy.check()?;
        if !params.is_empty() {
            return Err(Error::unsupported("bind parameters: not implemented yet"));
        }
        let sql = sql.trim().trim_end_matches(';');
        if !is_select(sql) {
            let r = self.inner.execute(sql, &[]).await.map_err(query_err)?;
            return Ok(Outcome::Affected(r.rows_affected));
        }
        let mut res = self.inner.query(sql, &[]).await.map_err(query_err)?;
        let columns: Vec<Column> = res
            .columns
            .iter()
            .map(|c| Column {
                name: c.name.clone(),
                type_name: format!("{:?}", c.oracle_type),
                kind: ValueKind::Text,
                nullable: c.nullable,
            })
            .collect();
        let mut rows = Vec::new();
        loop {
            rows.extend(
                res.rows
                    .iter()
                    .map(|r| r.values().iter().map(cell).collect()),
            );
            if !res.has_more_rows {
                break;
            }
            res = self
                .inner
                .fetch_more(res.cursor_id, &res.columns, 100)
                .await
                .map_err(query_err)?;
        }
        Ok(Outcome::Rows(Box::new(
            VecCursor::new(columns, rows).with_guard(self.busy.acquire()?),
        )))
    }

    async fn cancel(&self) -> Result<()> {
        Err(Error::unsupported("cancel: not implemented yet"))
    }

    async fn server_version(&self) -> Result<String> {
        Ok(format!("{:?}", self.inner.server_info().await))
    }

    async fn ping(&self) -> Result<()> {
        self.inner.ping().await.map_err(query_err)
    }

    async fn tx_mode(&self) -> TxMode {
        TxMode::Manual
    }

    async fn set_tx_mode(&self, _mode: TxMode) -> Result<()> {
        Err(Error::unsupported("transaction modes: not implemented yet"))
    }

    async fn commit(&self) -> Result<()> {
        self.inner.commit().await.map_err(query_err)
    }

    async fn rollback(&self) -> Result<()> {
        self.inner.rollback().await.map_err(query_err)
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await.map_err(query_err)
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

    /// Needs a running Oracle: `docker compose up -d oracle`, then
    /// `PYOXIDE_TEST_ORACLE=1 cargo test -p db-oracle-thin -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn select_from_dual_against_docker() {
        let p = ConnectParams {
            host: "localhost".into(),
            port: 1521,
            database: "FREEPDB1".into(),
            user: "pyoxide".into(),
            password: db_core::Secret::new(
                std::env::var("PYOXIDE_DB_PASSWORD").unwrap_or_else(|_| "pyoxide".into()),
            ),
            ..Default::default()
        };
        let c = ThinDriver.connect(&p).await.expect("connect");
        let rs = c
            .query("SELECT 1 AS n FROM dual", &[], 10)
            .await
            .expect("query");
        assert_eq!(rs.rows.len(), 1);
        c.close().await.ok();
    }
}
