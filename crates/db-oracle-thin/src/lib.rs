//! Pure-Rust Oracle backend over the `oracle-rs` thin protocol crate (no
//! Oracle Client libraries).
//!
//! oracle-rs 0.1.7 has protocol gaps against 12.2+ servers (19c, 23ai) that
//! this driver works around:
//! - A server error during a call makes oracle-rs drop the session without
//!   the error text. Every statement therefore runs inside a PL/SQL wrapper
//!   block that traps errors and returns them as data (`plsql.rs`).
//! - Only the first OUT bind of a PL/SQL call comes back, and string binds
//!   of a few KiB break the session. The driver never uses oracle-rs binds:
//!   values are embedded as typed PL/SQL variables and bound to the user's
//!   statement with `DBMS_SQL` (`literal.rs`).
//! - Several types decode wrongly (BINARY_FLOAT/DOUBLE, INTERVAL, NCHAR,
//!   ROWID, TIMESTAMP WITH TIME ZONE regions, `NULL` literal columns); the
//!   wrapper converts them on the server.
//! - One network packet per call: fetches are sized to fit (`cursor.rs`).
//! - Server cursors are never closed: the session is renewed before
//!   `OPEN_CURSORS` is reached (`ThinConnection::session_for`).
//! - No statement cancel: `cancel` is unsupported; an abandoned call marks
//!   the session broken (`ConnectionLost`).
//!
//! Read-only sessions are enforced on the client: only SELECT/WITH
//! statements run. Oracle's `SET TRANSACTION READ ONLY` applies to one
//! transaction only and would freeze the snapshot between commits.

mod config;
mod cursor;
mod literal;
mod plsql;
mod session;
mod sql;
mod types;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use db_core::timeout::with_timeout;
use db_core::{
    Backend, BusyFlag, ConnectParams, Connection, Driver, Error, ErrorKind, ExecOptions, Outcome,
    Result, TxMode, Value,
};
use oracle_rs::{BindParam, OracleType, RefCursor};

use crate::cursor::{NATIVE_PREFETCH, Source, ThinCursor};
use crate::plsql::BlockBind;
use crate::session::{LARGE_FETCH_BUDGET, SMALL_FETCH_BUDGET, Session};
use crate::sql::StmtKind;

/// Server cursors kept free for PL/SQL inside user statements.
const CURSOR_RESERVE: u32 = 10;

/// `OPEN_CURSORS` when the instance does not tell (Oracle's default).
const DEFAULT_OPEN_CURSORS: u32 = 50;

/// Rows and row size of the packet-size probe run after connecting.
const PROBE_ROWS: u32 = 15;
const PROBE_PAD: u32 = 4000;

/// The thin driver.
pub struct ThinDriver;

/// Fetch budget learned per listener address, so a server that negotiates
/// a small packet size is probed only once per process.
fn budget_cache() -> &'static Mutex<HashMap<String, usize>> {
    static CACHE: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[async_trait]
impl Driver for ThinDriver {
    fn backend(&self) -> Backend {
        Backend::OracleThin
    }

    fn availability(&self) -> Result<String> {
        Ok("built in: oracle-rs 0.1.7 (pure Rust thin protocol)".into())
    }

    async fn connect(&self, params: &ConnectParams) -> Result<Box<dyn Connection>> {
        Ok(Box::new(ThinConnection::open(params).await?))
    }
}

/// Open an oracle-rs session and run the probe block.
async fn open_session(p: &ConnectParams) -> Result<Session> {
    let secret = p.password.expose().to_owned();
    let scrub = |e: Error| e.scrub(&secret).scrub(p.tls.wallet_password.expose());
    let (cfg, target) = config::build(p).map_err(scrub)?;
    let key = target.describe();
    let cached = budget_cache()
        .lock()
        .ok()
        .and_then(|c| c.get(&key).copied());

    let mut session = Session::new(connect_raw(&cfg, &target, p).await.map_err(scrub)?);
    let probed = probe(&mut session, if cached.is_some() { 1 } else { PROBE_ROWS }).await;
    match (probed, cached) {
        (Ok(()), Some(budget)) => session.set_budget(budget),
        (Ok(()), None) => {
            session.set_budget(LARGE_FETCH_BUDGET);
            remember(&key, LARGE_FETCH_BUDGET);
        }
        (Err(e), _) if session.is_broken() && e.kind != ErrorKind::ConnectionLost => {
            // The probe page did not fit into one packet: the server uses a
            // small SDU. Reconnect and remember that.
            let mut small = Session::new(connect_raw(&cfg, &target, p).await.map_err(scrub)?);
            probe(&mut small, 1).await.map_err(scrub)?;
            small.set_budget(SMALL_FETCH_BUDGET);
            remember(&key, SMALL_FETCH_BUDGET);
            session = small;
        }
        (Err(e), _) => return Err(scrub(e)),
    }
    Ok(session)
}

fn remember(key: &str, budget: usize) {
    if let Ok(mut c) = budget_cache().lock() {
        c.insert(key.to_owned(), budget);
    }
}

async fn connect_raw(
    cfg: &oracle_rs::Config,
    target: &config::Target,
    p: &ConnectParams,
) -> Result<oracle_rs::Connection> {
    let limit = if p.connect_timeout.is_zero() {
        Duration::from_secs(10)
    } else {
        p.connect_timeout
    };
    match tokio::time::timeout(
        limit,
        oracle_rs::Connection::connect_with_config(cfg.clone()),
    )
    .await
    {
        Err(_) => Err(Error::new(
            ErrorKind::Network,
            format!(
                "connecting to {} timed out after {limit:?}",
                target.describe()
            ),
        )),
        Ok(Ok(conn)) => Ok(conn),
        Ok(Err(e)) => {
            let refused = matches!(
                e,
                oracle_rs::Error::ConnectionRefused {
                    error_code: None,
                    ..
                }
            );
            let mut err = session::map_connect_error(e, target.tls);
            if refused
                && !target.tls
                && let Some(code) = refusal_code(cfg, limit).await
            {
                let what = match code {
                    12514 => format!(
                        "the listener does not know service `{}`",
                        target.service.as_deref().unwrap_or_default()
                    ),
                    12505 => format!(
                        "the listener does not know SID `{}`",
                        target.sid.as_deref().unwrap_or_default()
                    ),
                    _ => "the listener refused the connection".to_owned(),
                };
                err = db_core::oracle::error_from_message(
                    format!("ORA-{code:05}: {what} ({})", target.describe()),
                    ErrorKind::Connect,
                );
            }
            Err(err)
        }
    }
}

/// oracle-rs drops the listener's refusal reason. Repeat the CONNECT
/// exchange with its public message types to read the `(ERR=...)` code.
async fn refusal_code(cfg: &oracle_rs::Config, limit: Duration) -> Option<u32> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let exchange = async {
        let msg = oracle_rs::messages::ConnectMessage::from_config(cfg);
        let (packet, continuation) = msg.build_with_continuation().ok()?;
        let mut stream = tokio::net::TcpStream::connect(cfg.socket_addr())
            .await
            .ok()?;
        stream.write_all(&packet).await.ok()?;
        if let Some(c) = continuation {
            stream.write_all(&c).await.ok()?;
        }
        let mut header = [0u8; 8];
        stream.read_exact(&mut header).await.ok()?;
        let len = usize::from(u16::from_be_bytes([header[0], header[1]]));
        let mut data = header.to_vec();
        data.resize(len.max(8), 0);
        stream.read_exact(&mut data[8..]).await.ok()?;
        let packet = oracle_rs::packet::Packet::from_bytes(data.into()).ok()?;
        if !packet.is_refuse() {
            return None;
        }
        oracle_rs::messages::RefuseMessage::parse(&packet)
            .ok()?
            .error_code
    };
    tokio::time::timeout(limit, exchange).await.ok().flatten()
}

fn cursor_out(values: Vec<oracle_rs::Value>) -> Result<RefCursor> {
    match values.into_iter().next() {
        Some(oracle_rs::Value::Cursor(rc)) if rc.cursor_id() != 0 => Ok(rc),
        other => Err(Error::internal(format!(
            "the wrapper block returned no cursor ({other:?})"
        ))),
    }
}

fn string_value(v: Option<&oracle_rs::Value>) -> Option<String> {
    match v {
        Some(oracle_rs::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Run the probe block: DBMS_SQL availability, OPEN_CURSORS, banner, and
/// `rows` rows of padding that must arrive in one packet.
async fn probe(session: &mut Session, rows: u32) -> Result<()> {
    let block = plsql::probe_block(rows, if rows > 1 { PROBE_PAD } else { 1 });
    let r = session
        .call(
            session
                .conn
                .execute_plsql(&block, &[BindParam::output_cursor()]),
        )
        .await?;
    session.add_cursors(2);
    let rc = cursor_out(r.out_values)?;
    let res = session
        .call(session.conn.fetch_cursor_with_size(&rc, rows))
        .await?;
    let first = res
        .rows
        .first()
        .ok_or_else(|| Error::internal("the connect probe returned no rows"))?;
    let values = first.values();
    session.dbms_sql = string_value(values.first()).as_deref() == Some("Y");
    session.cursor_limit = string_value(values.get(1))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_OPEN_CURSORS);
    session.banner = string_value(values.get(2))
        .map(|b| b.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "Oracle Database (version not visible to this user)".into());
    Ok(())
}

/// A connection over the thin driver.
pub struct ThinConnection {
    /// Kept to renew the session (see [`ThinConnection::session_for`]).
    params: ConnectParams,
    session: Mutex<Arc<Session>>,
    renew: tokio::sync::Mutex<()>,
    busy: BusyFlag,
    /// 0 = auto, 1 = manual.
    tx_mode: AtomicU8,
    /// Manual mode: changes may be pending.
    dirty: AtomicBool,
    /// `ALTER SESSION` statements to replay on a renewed session.
    session_sql: Mutex<Vec<String>>,
    closed: AtomicBool,
    renewals: std::sync::atomic::AtomicU32,
}

impl ThinConnection {
    /// Connect; the same as [`ThinDriver::connect`] with the concrete type.
    pub async fn open(params: &ConnectParams) -> Result<Self> {
        let session = open_session(params).await?;
        Ok(ThinConnection {
            params: params.clone(),
            session: Mutex::new(Arc::new(session)),
            renew: tokio::sync::Mutex::new(()),
            busy: BusyFlag::default(),
            tx_mode: AtomicU8::new(0),
            dirty: AtomicBool::new(false),
            session_sql: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
            renewals: std::sync::atomic::AtomicU32::new(0),
        })
    }

    fn current(&self) -> Arc<Session> {
        self.session
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::connection_lost("the connection is closed"));
        }
        Ok(())
    }

    fn auto(&self) -> bool {
        self.tx_mode.load(Ordering::Acquire) == 0
    }

    /// How many times the session was renewed because of the server cursor
    /// budget (see the module docs).
    pub fn session_renewals(&self) -> u32 {
        self.renewals.load(Ordering::Acquire)
    }

    /// Server cursors held by the current session (oracle-rs cannot close
    /// them) and the instance's `OPEN_CURSORS`.
    pub fn cursor_usage(&self) -> (u32, u32) {
        let s = self.current();
        (s.cursors(), s.cursor_limit)
    }

    /// The session for a call that opens `cost` server cursors, renewed
    /// first when the cursor budget would run out.
    async fn session_for(&self, cost: u32) -> Result<Arc<Session>> {
        let s = self.current();
        s.check()?;
        if s.cursors() + cost + CURSOR_RESERVE <= s.cursor_limit {
            return Ok(s);
        }
        let _g = self.renew.lock().await;
        let s = self.current();
        s.check()?;
        if s.cursors() + cost + CURSOR_RESERVE <= s.cursor_limit {
            return Ok(s);
        }
        if !self.auto() && self.dirty.load(Ordering::Acquire) {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "this session has used {} of {} server cursors (oracle-rs 0.1.7 cannot close cursors) and has uncommitted changes; commit or roll back so the driver can renew the session",
                    s.cursors(),
                    s.cursor_limit
                ),
            ));
        }
        let fresh = Arc::new(open_session(&self.params).await?);
        let replay: Vec<String> = self
            .session_sql
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default();
        for stmt in replay {
            if let Ok(block) = plsql::exec_block(&stmt, &[], false) {
                let _ = fresh
                    .call(fresh.conn.execute_plsql(&block, &[status_out()]))
                    .await;
                fresh.add_cursors(1);
            }
        }
        if let Ok(mut slot) = self.session.lock() {
            *slot = fresh.clone();
        }
        self.renewals.fetch_add(1, Ordering::AcqRel);
        if !s.is_broken() {
            let _ = tokio::time::timeout(Duration::from_secs(2), s.conn.close()).await;
        }
        Ok(fresh)
    }

    async fn run(
        &self,
        kind: StmtKind,
        sql: String,
        params: &[Value],
        opts: &ExecOptions,
    ) -> Result<Outcome> {
        if kind.returns_rows() {
            let guard = self.busy.acquire()?;
            let session = self.session_for(3).await?;
            let outcome = if session.dbms_sql {
                self.open_query(&session, &sql, params, opts, guard).await
            } else {
                let sql = substitute(&sql, params)?;
                self.open_native(&session, &sql, opts, guard).await
            };
            if outcome.is_ok() && !self.auto() && sql::has_for_update(&sql) {
                self.dirty.store(true, Ordering::Release);
            }
            return outcome;
        }
        self.busy.check()?;
        let session = self.session_for(1).await?;
        let n = if session.dbms_sql {
            let binds = block_binds(kind, &sql, params)?;
            let commit = self.auto() && !matches!(kind, StmtKind::Commit | StmtKind::Rollback);
            let block = plsql::exec_block(&sql, &binds, commit)?;
            session.check_request(&block)?;
            let r = session
                .call(session.conn.execute_plsql(&block, &[status_out()]))
                .await?;
            session.add_cursors(1);
            let status = string_value(r.out_values.first())
                .ok_or_else(|| Error::internal("the wrapper block returned no status"))?;
            plsql::parse_exec_status(&status)?
        } else {
            let sql = substitute(&sql, params)?;
            session.check_request(&sql)?;
            let n = if matches!(kind, StmtKind::Plsql | StmtKind::PlsqlDdl) {
                session
                    .call(session.conn.execute_plsql(&sql, &[]))
                    .await?
                    .rows_affected
            } else {
                session
                    .call(session.conn.execute(&sql, &[]))
                    .await?
                    .rows_affected
            };
            session.add_cursors(1);
            if self.auto() {
                session.call(session.conn.commit()).await?;
            }
            n
        };
        match kind {
            StmtKind::Ddl | StmtKind::PlsqlDdl | StmtKind::Commit | StmtKind::Rollback => {
                self.dirty.store(false, Ordering::Release);
            }
            StmtKind::AlterSession => {
                if let Ok(mut v) = self.session_sql.lock() {
                    v.push(sql.clone());
                }
            }
            _ if !self.auto() => self.dirty.store(true, Ordering::Release),
            _ => {}
        }
        Ok(Outcome::Affected(if kind == StmtKind::Dml { n } else { 0 }))
    }

    async fn open_query(
        &self,
        session: &Arc<Session>,
        sql: &str,
        params: &[Value],
        opts: &ExecOptions,
        guard: db_core::BusyGuard,
    ) -> Result<Outcome> {
        // Long text values go into the statement as literals: a temporary
        // LOB bound here would be freed with the block before the fetch.
        let found = sql::placeholders(sql);
        let plan = sql::bind_plan(&found, params.len())?;
        let mut inline = Vec::new();
        for (name, idx) in &plan {
            if literal::needs_inline_for_query(&params[*idx])? {
                inline.push((name.clone(), *idx));
            }
        }
        let mut text = sql.to_owned();
        for p in found.iter().rev() {
            if let Some((_, idx)) = inline.iter().find(|(n, _)| *n == p.name) {
                text.replace_range(
                    p.start..p.end,
                    &format!(" {} ", literal::sql_literal(&params[*idx])?),
                );
            }
        }
        let mut binds = Vec::new();
        for (name, idx) in plan
            .iter()
            .filter(|(n, _)| !inline.iter().any(|(m, _)| m == n))
        {
            binds.push(BlockBind {
                placeholder: name.clone(),
                bind: literal::plsql_bind(&format!("pxb{}", binds.len() + 1), &params[*idx])?,
            });
        }
        let (body, tail) = sql::split_for_update(&text);
        let occurrences = sql::placeholders(body)
            .iter()
            .map(|p| {
                binds
                    .iter()
                    .position(|b| b.placeholder == p.name)
                    .ok_or_else(|| Error::query(format!("placeholder :{} has no value", p.name)))
            })
            .collect::<Result<Vec<_>>>()?;
        let block = plsql::query_block(body, tail, &binds, &occurrences)?;
        session.check_request(&block)?;
        let r = session
            .call(
                session
                    .conn
                    .execute_plsql(&block, &[BindParam::output_cursor()]),
            )
            .await?;
        session.add_cursors(2);
        let rc = cursor_out(r.out_values)?;
        let first_name = rc.columns().first().map(|c| c.name.as_str()).unwrap_or("");
        if first_name == plsql::ERROR_COLUMN {
            let res = session
                .call(session.conn.fetch_cursor_with_size(&rc, 1))
                .await?;
            let row = res.rows.first();
            let code = row
                .and_then(|r| string_value(r.values().first()))
                .and_then(|c| c.trim().parse::<i64>().ok())
                .unwrap_or(0);
            let msg = row
                .and_then(|r| string_value(r.values().get(1)))
                .unwrap_or_default();
            return Err(plsql::trapped_error(code, &msg));
        }
        if first_name == plsql::NATIVE_COLUMN {
            let mut chunks = Vec::new();
            loop {
                let res = session
                    .call(session.conn.fetch_cursor_with_size(&rc, 8))
                    .await?;
                let n = res.rows.len();
                chunks.extend(
                    res.rows
                        .iter()
                        .filter_map(|r| string_value(r.values().first())),
                );
                if n < 8 {
                    break;
                }
            }
            let native = substitute_plan(&plsql::join_native_chunks(&chunks), &plan, params)?;
            return self.open_native(session, &native, opts, guard).await;
        }
        let columns = rc.columns().to_vec();
        Ok(Outcome::Rows(Box::new(ThinCursor::new(
            session.clone(),
            guard,
            &columns,
            Source::Ref(rc),
            Vec::new(),
            false,
            opts,
        ))))
    }

    /// Run a query through oracle-rs `query` (needed for LOB columns, which
    /// require column defines).
    async fn open_native(
        &self,
        session: &Arc<Session>,
        sql: &str,
        opts: &ExecOptions,
        guard: db_core::BusyGuard,
    ) -> Result<Outcome> {
        session.check_request(sql)?;
        let res = session.call(session.conn.query(sql, &[])).await?;
        session.add_cursors(1);
        let done = res.rows.len() < NATIVE_PREFETCH;
        let columns = res.columns.clone();
        Ok(Outcome::Rows(Box::new(ThinCursor::new(
            session.clone(),
            guard,
            &columns,
            Source::Native {
                cursor_id: res.cursor_id,
                columns: res.columns,
            },
            res.rows,
            done,
            opts,
        ))))
    }
}

fn status_out() -> BindParam {
    BindParam::output(OracleType::Varchar, 4200)
}

/// Bind values as wrapper-block variables, by placeholder name.
fn block_binds(kind: StmtKind, sql: &str, params: &[Value]) -> Result<Vec<BlockBind>> {
    if kind.is_ddl() {
        if !params.is_empty() {
            return Err(Error::query("DDL statements cannot take bind values"));
        }
        return Ok(Vec::new());
    }
    let plan = sql::bind_plan(&sql::placeholders(sql), params.len())?;
    plan.into_iter()
        .enumerate()
        .map(|(i, (placeholder, idx))| {
            Ok(BlockBind {
                placeholder,
                bind: literal::plsql_bind(&format!("pxb{}", i + 1), &params[idx])?,
            })
        })
        .collect()
}

/// Replace placeholders with SQL literals (LOB path and sessions without
/// DBMS_SQL).
fn substitute(sql: &str, params: &[Value]) -> Result<String> {
    let plan = sql::bind_plan(&sql::placeholders(sql), params.len())?;
    substitute_plan(sql, &plan, params)
}

/// [`substitute`] with a bind plan computed earlier (the text may have
/// fewer placeholders than the plan).
fn substitute_plan(sql: &str, plan: &[(String, usize)], params: &[Value]) -> Result<String> {
    let mut out = sql.to_owned();
    for p in sql::placeholders(sql).iter().rev() {
        let idx = plan
            .iter()
            .find(|(name, _)| *name == p.name)
            .map(|(_, i)| *i)
            .ok_or_else(|| Error::query(format!("placeholder :{} has no value", p.name)))?;
        out.replace_range(
            p.start..p.end,
            &format!(" {} ", literal::sql_literal(&params[idx])?),
        );
    }
    Ok(out)
}

#[async_trait]
impl Connection for ThinConnection {
    fn backend(&self) -> Backend {
        Backend::OracleThin
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        self.ensure_open()?;
        self.busy.check()?;
        let kind = sql::classify(sql);
        let text = sql::normalize(sql, kind);
        if text.is_empty() {
            return Err(Error::query("empty statement"));
        }
        if self.params.read_only && kind != StmtKind::Query {
            return Err(Error::query(
                "the connection is read-only: only SELECT and WITH queries are allowed",
            ));
        }
        with_timeout(opts.timeout, self.run(kind, text, params, opts), async {}).await
    }

    async fn cancel(&self) -> Result<()> {
        Err(Error::unsupported(
            "the thin driver cannot cancel a running statement (oracle-rs 0.1.7 has no break/reset); drop the running call instead: the session is then closed and must be reopened",
        ))
    }

    async fn server_version(&self) -> Result<String> {
        self.ensure_open()?;
        let s = self.current();
        s.check()?;
        Ok(s.banner.clone())
    }

    async fn ping(&self) -> Result<()> {
        self.ensure_open()?;
        let s = self.current();
        s.call(s.conn.ping()).await
    }

    async fn tx_mode(&self) -> TxMode {
        if self.auto() {
            TxMode::Auto
        } else {
            TxMode::Manual
        }
    }

    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.ensure_open()?;
        if mode == TxMode::Auto && !self.auto() && self.dirty.load(Ordering::Acquire) {
            // Like JDBC: switching to auto-commit commits pending work.
            self.commit().await?;
        }
        self.tx_mode
            .store(if mode == TxMode::Auto { 0 } else { 1 }, Ordering::Release);
        Ok(())
    }

    async fn commit(&self) -> Result<()> {
        self.ensure_open()?;
        let s = self.current();
        s.call(s.conn.commit()).await?;
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    async fn rollback(&self) -> Result<()> {
        self.ensure_open()?;
        let s = self.current();
        s.call(s.conn.rollback()).await?;
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let s = self.current();
        if !s.is_broken() {
            let _ = tokio::time::timeout(Duration::from_secs(5), s.conn.close()).await;
        }
        s.mark_broken("the connection was closed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_literals() {
        let sql = "SELECT :1, ':1', :2 FROM t WHERE a = :1";
        let out = substitute(sql, &[Value::Int(-5), Value::Text("o'k".into())]).unwrap();
        assert_eq!(out, "SELECT  (-5) , ':1',  'o''k'  FROM t WHERE a =  (-5) ");
        assert!(substitute("SELECT :1 FROM dual", &[]).is_err());
        assert_eq!(
            substitute("SELECT 1 FROM dual", &[]).unwrap(),
            "SELECT 1 FROM dual"
        );
    }

    #[test]
    fn ddl_takes_no_binds() {
        assert!(block_binds(StmtKind::Ddl, "CREATE TABLE t (a NUMBER)", &[Value::Int(1)]).is_err());
        assert!(
            block_binds(StmtKind::Ddl, "CREATE TABLE t (a NUMBER)", &[])
                .unwrap()
                .is_empty()
        );
        let b = block_binds(
            StmtKind::Dml,
            "UPDATE t SET a = :1 WHERE b = :1",
            &[Value::Int(1)],
        )
        .unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].placeholder, "1");
    }

    #[test]
    fn driver_reports_availability() {
        assert!(ThinDriver.availability().unwrap().contains("oracle-rs"));
        assert_eq!(ThinDriver.backend(), Backend::OracleThin);
    }
}
