//! Oracle backend on top of an installed Oracle Client (ODPI-C).
//!
//! Fallback for environments the pure-Rust thin driver cannot reach
//! (Kerberos, LDAP naming, native network encryption, ...). The Oracle Client
//! libraries are loaded at runtime, so this crate builds without them.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use db_core::{
    Backend, BusyFlag, Column, ConnectParams, Connection, Driver, Error, ErrorKind, ExecOptions,
    Outcome, Result, TxMode, Value, ValueKind, VecCursor,
};

#[allow(dead_code)]
mod binds;
#[allow(dead_code)]
mod connect_string;
#[allow(dead_code)]
mod errors;
#[allow(dead_code)]
mod sqltext;
#[allow(dead_code)]
mod tx;
#[allow(dead_code)]
mod values;

pub struct OdpiDriver;

pub struct OdpiConnection {
    inner: Arc<Mutex<oracle::Connection>>,
    busy: BusyFlag,
}

#[async_trait]
impl Driver for OdpiDriver {
    fn backend(&self) -> Backend {
        Backend::OracleOdpi
    }

    fn availability(&self) -> Result<String> {
        oracle::Version::client()
            .map(|v| format!("Oracle Client {v}"))
            .map_err(|e| Error::new(ErrorKind::Unavailable, e.to_string()))
    }

    async fn connect(&self, p: &ConnectParams) -> Result<Box<dyn Connection>> {
        let p = p.clone();
        let conn = tokio::task::spawn_blocking(move || {
            let dsn = format!("//{}:{}/{}", p.host, p.port, p.database);
            oracle::Connection::connect(&p.user, p.password.expose(), dsn).map_err(|e| {
                Error::new(ErrorKind::Connect, e.to_string()).scrub(p.password.expose())
            })
        })
        .await
        .map_err(|e| Error::internal(e.to_string()))??;
        Ok(Box::new(OdpiConnection {
            inner: Arc::new(Mutex::new(conn)),
            busy: BusyFlag::default(),
        }))
    }
}

enum Fetched {
    Rows(Vec<Column>, Vec<Vec<Value>>),
    Affected(u64),
}

// Interim implementation: materializes the whole result as text and binds no
// parameters. Typed values, binds, paging and cancel follow.
fn run(conn: &oracle::Connection, sql: &str) -> std::result::Result<Fetched, oracle::Error> {
    let mut stmt = conn.statement(sql).build()?;
    if !stmt.is_query() {
        stmt.execute(&[])?;
        return Ok(Fetched::Affected(stmt.row_count()?));
    }
    let rows = stmt.query(&[])?;
    let columns: Vec<Column> = rows
        .column_info()
        .iter()
        .map(|c| Column {
            name: c.name().to_string(),
            type_name: c.oracle_type().to_string(),
            kind: ValueKind::Text,
            nullable: c.nullable(),
        })
        .collect();
    let mut out = Vec::new();
    for row in rows {
        let row = row?;
        let mut cells = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            let v: Option<String> = row.get(i)?;
            cells.push(v.map_or(Value::Null, Value::Text));
        }
        out.push(cells);
    }
    Ok(Fetched::Rows(columns, out))
}

impl OdpiConnection {
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&oracle::Connection) -> std::result::Result<T, oracle::Error> + Send + 'static,
    ) -> Result<T> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let conn = inner
                .lock()
                .map_err(|_| Error::connection_lost("connection poisoned"))?;
            f(&conn).map_err(|e| Error::query(e.to_string()))
        })
        .await
        .map_err(|e| Error::internal(e.to_string()))?
    }
}

#[async_trait]
impl Connection for OdpiConnection {
    fn backend(&self) -> Backend {
        Backend::OracleOdpi
    }

    async fn execute(&self, sql: &str, params: &[Value], _opts: &ExecOptions) -> Result<Outcome> {
        self.busy.check()?;
        if !params.is_empty() {
            return Err(Error::unsupported("bind parameters: not implemented yet"));
        }
        let sql = sql.trim().trim_end_matches(';').to_string();
        match self.blocking(move |c| run(c, &sql)).await? {
            Fetched::Affected(n) => Ok(Outcome::Affected(n)),
            Fetched::Rows(columns, rows) => Ok(Outcome::Rows(Box::new(
                VecCursor::new(columns, rows).with_guard(self.busy.acquire()?),
            ))),
        }
    }

    async fn cancel(&self) -> Result<()> {
        Err(Error::unsupported("cancel: not implemented yet"))
    }

    async fn server_version(&self) -> Result<String> {
        self.blocking(|c| {
            c.server_version()
                .map(|(v, banner)| format!("{banner} ({v})"))
        })
        .await
    }

    async fn ping(&self) -> Result<()> {
        self.blocking(|c| c.ping()).await
    }

    async fn tx_mode(&self) -> TxMode {
        TxMode::Manual
    }

    async fn set_tx_mode(&self, _mode: TxMode) -> Result<()> {
        Err(Error::unsupported("transaction modes: not implemented yet"))
    }

    async fn commit(&self) -> Result<()> {
        self.blocking(|c| c.commit()).await
    }

    async fn rollback(&self) -> Result<()> {
        self.blocking(|c| c.rollback()).await
    }

    async fn close(&self) -> Result<()> {
        self.blocking(|c| c.close()).await
    }
}
