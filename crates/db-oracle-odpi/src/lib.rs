//! Oracle backend on top of an installed Oracle Client (ODPI-C).
//!
//! Fallback for environments the pure-Rust thin driver cannot reach
//! (Kerberos, LDAP naming, native network encryption, ...). The Oracle Client
//! libraries are loaded at runtime, so this crate builds without them.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use db_core::{Cell, Column, ConnectParams, Connection, Driver, Error, ResultSet};

pub struct OdpiDriver;

pub struct OdpiConnection {
    inner: Arc<Mutex<oracle::Connection>>,
}

#[async_trait]
impl Driver for OdpiDriver {
    async fn connect(p: &ConnectParams) -> db_core::Result<Box<dyn Connection>> {
        let p = p.clone();
        let conn = tokio::task::spawn_blocking(move || {
            let dsn = format!("//{}:{}/{}", p.host, p.port, p.service);
            oracle::Connection::connect(&p.user, &p.password, dsn)
        })
        .await
        .map_err(|e| Error::Connect(e.to_string()))?
        .map_err(|e| Error::Connect(e.to_string()))?;
        Ok(Box::new(OdpiConnection {
            inner: Arc::new(Mutex::new(conn)),
        }))
    }
}

fn run(conn: &oracle::Connection, sql: &str, max_rows: usize) -> Result<ResultSet, oracle::Error> {
    let mut stmt = conn.statement(sql).build()?;
    if !stmt.is_query() {
        stmt.execute(&[])?;
        return Ok(ResultSet {
            rows_affected: stmt.row_count()?,
            ..Default::default()
        });
    }
    let rows = stmt.query(&[])?;
    let columns: Vec<Column> = rows
        .column_info()
        .iter()
        .map(|c| Column {
            name: c.name().to_string(),
            type_name: c.oracle_type().to_string(),
            nullable: c.nullable(),
        })
        .collect();
    let mut out = Vec::new();
    let mut truncated = false;
    for row in rows {
        let row = row?;
        if out.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut cells = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            let v: Option<String> = row.get(i)?;
            cells.push(v.map_or(Cell::Null, Cell::Text));
        }
        out.push(cells);
    }
    Ok(ResultSet {
        columns,
        rows: out,
        rows_affected: 0,
        truncated,
    })
}

#[async_trait]
impl Connection for OdpiConnection {
    fn backend(&self) -> &'static str {
        "oracle-odpi"
    }

    async fn query(&self, sql: &str, max_rows: usize) -> db_core::Result<ResultSet> {
        let inner = Arc::clone(&self.inner);
        let sql = sql.trim().trim_end_matches(';').to_string();
        tokio::task::spawn_blocking(move || {
            let conn = inner
                .lock()
                .map_err(|_| Error::Query("connection poisoned".into()))?;
            run(&conn, &sql, max_rows).map_err(|e| Error::Query(e.to_string()))
        })
        .await
        .map_err(|e| Error::Query(e.to_string()))?
    }

    async fn close(&self) -> db_core::Result<()> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let conn = inner
                .lock()
                .map_err(|_| Error::Query("connection poisoned".into()))?;
            conn.close().map_err(|e| Error::Query(e.to_string()))
        })
        .await
        .map_err(|e| Error::Query(e.to_string()))?
    }
}
