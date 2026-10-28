//! `db.diagnose`: find out why a connection fails, step by step, without
//! ever printing a secret. Each step reports OK/WARN/FAIL/SKIP with timing;
//! the first failure carries a plain-language cause and a next step.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use db_core::{
    Backend, ConnectParams, Connection, Dialect, Error, ErrorKind, ExecOptions, Outcome, Value,
    ValueKind, schema,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use crate::hints::{self, Hint};
use crate::profiles::Profile;
use crate::redact::Redactor;
use crate::secrets::SecretSource;
use crate::service::{DbService, Target, millis};
use crate::tls_probe;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnoseOptions {
    /// Mask host names, addresses, user, service and paths for sharing.
    pub share: bool,
    /// Also check paging and statement timeouts (takes a few seconds more).
    pub deep: bool,
    /// For Oracle, also try the other Oracle backend when login fails.
    pub try_all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "[ OK ]",
            Status::Warn => "[WARN]",
            Status::Fail => "[FAIL]",
            Status::Skip => "[SKIP]",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub millis: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<HintText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HintText {
    pub cause: String,
    pub next: String,
}

impl From<Hint> for HintText {
    fn from(h: Hint) -> Self {
        Self {
            cause: h.cause,
            next: h.next,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub tool: String,
    pub platform: String,
    pub steps: Vec<Step>,
    pub result: Status,
    /// Name of the first failing step.
    pub failed_at: Option<String>,
}

struct Diag {
    steps: Vec<Step>,
}

impl Diag {
    fn push(&mut self, name: &str, status: Status, detail: impl Into<String>, millis: Option<f64>) {
        self.steps.push(Step {
            name: name.to_owned(),
            status,
            detail: detail.into(),
            millis,
            code: None,
            hint: None,
        });
    }

    fn ok(&mut self, name: &str, detail: impl Into<String>, millis: Option<f64>) {
        self.push(name, Status::Ok, detail, millis);
    }

    fn warn(&mut self, name: &str, detail: impl Into<String>, millis: Option<f64>) {
        self.push(name, Status::Warn, detail, millis);
    }

    fn skip(&mut self, name: &str, detail: impl Into<String>) {
        self.push(name, Status::Skip, detail, None);
    }

    fn fail_with(
        &mut self,
        name: &str,
        detail: impl Into<String>,
        millis: Option<f64>,
        hint: Option<Hint>,
    ) {
        self.push(name, Status::Fail, detail, millis);
        self.steps.last_mut().expect("just pushed").hint = hint.map(Into::into);
    }

    fn fail_err(&mut self, name: &str, dialect: Option<Dialect>, err: &Error, millis: Option<f64>) {
        let hint = dialect.and_then(|d| hints::for_error(d, err));
        self.push(name, Status::Fail, err.message.clone(), millis);
        let step = self.steps.last_mut().expect("just pushed");
        step.code = err.code.clone();
        step.hint = hint.map(Into::into);
    }
}

fn platform() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Run all checks for a target. Never fails: problems become steps.
pub async fn diagnose(svc: &DbService, target: &Target, opts: &DiagnoseOptions) -> Report {
    let mut d = Diag { steps: Vec::new() };
    let mut redactor = Redactor::default();
    let mut secrets: Vec<String> = Vec::new();

    environment(&mut d);
    run(svc, target, opts, &mut d, &mut redactor, &mut secrets).await;

    let mut steps = d.steps;
    for step in &mut steps {
        let clean = |s: &str| {
            let mut s = s.to_owned();
            for secret in &secrets {
                if !secret.is_empty() {
                    s = s.replace(secret.as_str(), "<redacted>");
                }
            }
            if opts.share { redactor.apply(&s) } else { s }
        };
        step.detail = clean(&step.detail);
        if let Some(h) = &mut step.hint {
            h.cause = clean(&h.cause);
            h.next = clean(&h.next);
        }
    }
    let failed_at = steps
        .iter()
        .find(|s| s.status == Status::Fail)
        .map(|s| s.name.clone());
    let result = if failed_at.is_some() {
        Status::Fail
    } else if steps.iter().any(|s| s.status == Status::Warn) {
        Status::Warn
    } else {
        Status::Ok
    };
    Report {
        tool: format!("{} {}", app_info::db_cli_name(), env!("CARGO_PKG_VERSION")),
        platform: platform(),
        steps,
        result,
        failed_at,
    }
}

fn environment(d: &mut Diag) {
    let vars = [
        "TNS_ADMIN".to_owned(),
        "ORACLE_HOME".to_owned(),
        crate::secrets::password_env(),
        crate::profiles::profiles_env(),
    ];
    let set: Vec<String> = vars
        .iter()
        .map(|v| {
            let state = if std::env::var_os(v).is_some_and(|x| !x.is_empty()) {
                "set"
            } else {
                "unset"
            };
            format!("{v} {state}")
        })
        .collect();
    d.ok(
        "environment",
        format!(
            "version {} on {}; {}",
            env!("CARGO_PKG_VERSION"),
            platform(),
            set.join(", ")
        ),
        None,
    );
}

async fn run(
    svc: &DbService,
    target: &Target,
    opts: &DiagnoseOptions,
    d: &mut Diag,
    redactor: &mut Redactor,
    secrets: &mut Vec<String>,
) {
    // Profile.
    let (name, profile) = match svc.resolve_target(target) {
        Ok(x) => x,
        Err(e) => return d.fail_err("profile", None, &e, None),
    };
    let dialect = profile.dialect();
    mask_profile(redactor, &name, &profile);
    let ro = if profile.read_only { ", read-only" } else { "" };
    let user = if profile.user.is_empty() {
        "(external authentication)".to_owned()
    } else {
        profile.user.clone()
    };
    d.ok(
        "profile",
        format!(
            "{name}: {} {} user {user}, tls {:?}{ro}",
            profile.backend,
            profile.target(),
            profile.tls_mode()
        ),
        None,
    );
    files(d, &profile);

    // Secrets.
    let s = svc.secrets_for(&name, &profile);
    secrets.push(s.password.expose().to_owned());
    secrets.push(s.wallet_password.expose().to_owned());
    match &s.password_source {
        SecretSource::Missing => {
            let mut detail = "no password found".to_owned();
            if let Some(e) = &s.store_error {
                detail.push_str(&format!("; secret store error: {e}"));
            }
            d.fail_with(
                "secret",
                detail,
                None,
                Some(Hint {
                    cause: "the profile needs a password and none was found".into(),
                    next: format!(
                        "store it with `{} secret set {name}` or set {}",
                        app_info::db_cli_name(),
                        crate::secrets::password_env()
                    ),
                }),
            );
            return;
        }
        source => {
            let mut detail = format!("password: {source}");
            if s.wallet_source != SecretSource::NotNeeded {
                detail.push_str(&format!("; wallet password: {}", s.wallet_source));
            }
            match &s.store_error {
                Some(e) => d.warn("secret", format!("{detail}; secret store error: {e}"), None),
                None => d.ok("secret", detail, None),
            }
        }
    }

    // Client side.
    match crate::availability_for(&profile) {
        Ok(v) => d.ok("client", format!("{}: {v}", profile.backend), None),
        Err(e) => {
            d.fail_err("client", Some(dialect), &e, None);
            if !opts.try_all {
                return;
            }
        }
    }
    if dialect == Dialect::Oracle {
        let other = other_oracle(profile.backend);
        let detail = match crate::driver(other).availability() {
            Ok(v) => format!("alternative {other}: {v}"),
            Err(e) => format!("alternative {other}: not available ({})", short(&e)),
        };
        d.ok("client", detail, None);
    }

    // Network path.
    let mut tcp_ok = true;
    if dialect != Dialect::Sqlite {
        if profile.dsn.is_some() {
            d.skip("dns", "the connect string is resolved by the driver");
        } else {
            let timeout = Duration::from_secs(profile.connect_timeout_secs.unwrap_or(10));
            tcp_ok = network(d, redactor, &profile, timeout).await;
        }
    }
    if !tcp_ok && !opts.try_all {
        d.skip("login", "the network checks failed");
        return;
    }

    // Login.
    let params = match profile.to_params(s.password.clone(), s.wallet_password.clone()) {
        Ok(p) => p,
        Err(e) => return d.fail_err("login", Some(dialect), &e, None),
    };
    let mut conn = login(d, profile.backend, &params, "login").await;
    if conn.is_none() && dialect == Dialect::Oracle {
        let thin_issue = d.steps.last().is_some_and(|s| {
            s.status == Status::Fail
                && s.hint
                    .as_ref()
                    .is_some_and(|h| h.next.contains("oracle-odpi"))
        });
        if opts.try_all || thin_issue {
            let other = other_oracle(profile.backend);
            if crate::driver(other).availability().is_ok() {
                conn = login(d, other, &params, &format!("login ({other})")).await;
            } else if opts.try_all {
                d.skip(
                    &format!("login ({other})"),
                    "backend not available on this machine",
                );
            }
        }
    }
    let Some(conn) = conn else {
        d.skip("server", "login failed");
        return;
    };

    server(d, conn.as_ref(), redactor).await;
    dictionary(d, conn.as_ref()).await;
    types(d, conn.as_ref()).await;
    let conn = if opts.deep {
        paging(d, conn.as_ref()).await;
        timeouts(d, conn, profile.backend, &params).await
    } else {
        Some(conn)
    };
    if let Some(conn) = conn {
        conn.close().await.ok();
    }
}

/// First sentence of an error, with its code, for one-line details.
fn short(e: &Error) -> String {
    if e.code.as_deref() == Some("DPI-1047") || e.message.contains("DPI-1047") {
        return "Oracle Client libraries not found, DPI-1047".into();
    }
    let first = e.message.split(['\n', '.']).next().unwrap_or_default();
    let mut s: String = first.chars().take(120).collect();
    if let Some(code) = &e.code
        && !s.contains(code.as_str())
    {
        s = format!("{code}: {s}");
    }
    s
}

fn other_oracle(b: Backend) -> Backend {
    if b == Backend::OracleOdpi {
        Backend::OracleThin
    } else {
        Backend::OracleOdpi
    }
}

fn mask_profile(r: &mut Redactor, name: &str, p: &Profile) {
    r.add(&p.host, "<host>");
    r.add(&p.user, "<user>");
    if let Some(v) = &p.service {
        r.add(v, "<service>");
    }
    if let Some(v) = &p.sid {
        r.add(v, "<sid>");
    }
    if let Some(v) = &p.database {
        r.add(v, "<database>");
    }
    if let Some(v) = &p.dsn {
        r.add(v, "<dsn>");
    }
    if let Some(v) = &p.path {
        r.add(v, "<path>");
    }
    if let Some(t) = &p.tls {
        for (v, ph) in [
            (&t.wallet_dir, "<wallet-dir>"),
            (&t.ca_file, "<ca-file>"),
            (&t.client_cert, "<client-cert>"),
            (&t.client_key, "<client-key>"),
            (&t.server_name, "<server-name>"),
        ] {
            if let Some(v) = v {
                r.add(v, ph);
            }
        }
    }
    r.add(name, "<profile>");
    if let Some(home) = dirs::home_dir() {
        r.add(&home.to_string_lossy(), "~");
    }
}

/// Files the profile points to must exist before anything else.
fn files(d: &mut Diag, p: &Profile) {
    let mut checks: Vec<(String, Option<String>)> = Vec::new();
    if let Some(t) = &p.tls {
        for (label, path) in [
            ("CA file", &t.ca_file),
            ("client certificate", &t.client_cert),
            ("client key", &t.client_key),
        ] {
            if let Some(path) = path {
                let full = crate::profiles::expand_home(path);
                checks.push((
                    format!("{label} {path}"),
                    (!full.is_file()).then(|| "not found".to_owned()),
                ));
            }
        }
        if let Some(dir) = &t.wallet_dir {
            let full = crate::profiles::expand_home(dir);
            let needed = if p.backend == Backend::OracleThin {
                "ewallet.pem"
            } else {
                "cwallet.sso"
            };
            let found: Vec<&str> = [
                "ewallet.pem",
                "cwallet.sso",
                "ewallet.p12",
                "tnsnames.ora",
                "sqlnet.ora",
            ]
            .into_iter()
            .filter(|f| full.join(f).is_file())
            .collect();
            let problem = if !full.is_dir() {
                Some("not found".to_owned())
            } else if !found.contains(&needed) {
                Some(format!(
                    "has [{}] but {} needs {needed}",
                    found.join(", "),
                    p.backend
                ))
            } else {
                None
            };
            checks.push((format!("wallet {dir} [{}]", found.join(", ")), problem));
        }
    }
    if p.dialect() == Dialect::Sqlite
        && let Some(path) = &p.path
        && path != ":memory:"
    {
        let full = crate::profiles::expand_home(path);
        let problem = (!full.is_file() && !p.create)
            .then(|| "not found (set create = true to create it)".to_owned());
        checks.push((format!("database file {path}"), problem));
    }
    for (what, problem) in checks {
        match problem {
            None => d.ok("files", what, None),
            Some(problem) => d.fail_with(
                "files",
                format!("{what}: {problem}"),
                None,
                Some(Hint {
                    cause: "a file named in the profile is missing or incomplete".into(),
                    next: "fix the path in the profile".into(),
                }),
            ),
        }
    }
}

/// DNS, TCP and TLS. Returns false if the server cannot be reached at all.
async fn network(d: &mut Diag, redactor: &mut Redactor, p: &Profile, timeout: Duration) -> bool {
    let port = p.port_or_default();
    let started = Instant::now();
    let addrs: Vec<SocketAddr> =
        match tokio::time::timeout(timeout, tokio::net::lookup_host((p.host.as_str(), port))).await
        {
            Ok(Ok(a)) => a.collect(),
            Ok(Err(e)) => {
                d.fail_with(
                    "dns",
                    format!("cannot resolve {}: {e}", p.host),
                    Some(millis(started)),
                    Some(Hint {
                        cause: "the host name does not resolve".into(),
                        next: "check the spelling, the VPN connection and the DNS settings".into(),
                    }),
                );
                return false;
            }
            Err(_) => {
                d.fail_with(
                    "dns",
                    format!("resolving {} timed out", p.host),
                    Some(millis(started)),
                    Some(Hint {
                        cause: "the DNS server did not answer".into(),
                        next: "check the network and VPN".into(),
                    }),
                );
                return false;
            }
        };
    for (i, a) in addrs.iter().enumerate() {
        redactor.add(&a.ip().to_string(), &format!("<ip{}>", i + 1));
    }
    let list: Vec<String> = addrs.iter().map(|a| a.ip().to_string()).collect();
    d.ok(
        "dns",
        format!("{} -> {}", p.host, list.join(", ")),
        Some(millis(started)),
    );

    let mut connected = None;
    let mut errors = Vec::new();
    for addr in &addrs {
        let started = Instant::now();
        match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                d.ok("tcp", format!("connected to {addr}"), Some(millis(started)));
                connected = Some((*addr, stream));
                break;
            }
            Ok(Err(e)) => errors.push((
                format!("{addr}: {e}"),
                e.kind() == std::io::ErrorKind::ConnectionRefused,
            )),
            Err(_) => errors.push((
                format!("{addr}: no answer within {} s", timeout.as_secs()),
                false,
            )),
        }
    }
    let Some((addr, stream)) = connected else {
        let refused = errors.iter().any(|(_, r)| *r);
        let detail: Vec<String> = errors.into_iter().map(|(e, _)| e).collect();
        let hint = if refused {
            Hint {
                cause: format!("the host answers but nothing accepts connections on port {port}"),
                next: "check the port number and that the database listener runs".into(),
            }
        } else {
            Hint {
                cause: "no answer from the host on this port".into(),
                next: "a firewall or a missing VPN connection is the usual cause; check with the network team".into(),
            }
        };
        d.fail_with("tcp", detail.join("; "), None, Some(hint));
        return false;
    };

    let mode = p.tls_mode();
    if !mode.is_enabled() {
        drop(stream);
        d.skip("tls", "disabled in the profile");
        return true;
    }
    let tls = p.tls.clone().unwrap_or_default();
    let server_name = tls.server_name.clone().unwrap_or_else(|| p.host.clone());
    let started = Instant::now();
    let probe = tls_probe::probe(
        stream,
        p.dialect() == Dialect::Postgres,
        &server_name,
        tls.ca_file
            .as_deref()
            .map(crate::profiles::expand_home)
            .as_deref(),
        tls.wallet_dir
            .as_deref()
            .map(crate::profiles::expand_home)
            .as_deref(),
        timeout,
    )
    .await;
    let ms = Some(millis(started));
    use db_core::TlsMode::*;
    match probe {
        tls_probe::Outcome::NotOffered => {
            if mode == Prefer {
                d.warn("tls", "the server does not offer TLS; the connection will be plain", ms);
            } else {
                d.fail_with(
                    "tls",
                    "the server does not offer TLS",
                    ms,
                    Some(Hint {
                        cause: "TLS is required by the profile but the server does not support it".into(),
                        next: "set tls mode to disable or prefer, or enable TLS on the server".into(),
                    }),
                );
            }
        }
        tls_probe::Outcome::Failed(e) => d.fail_with(
            "tls",
            format!("handshake with {addr} failed: {e}"),
            ms,
            Some(Hint {
                cause: "no TLS session could be established".into(),
                next: "check that this port speaks TLS (Oracle TCPS often uses 2484) and the TLS version the server allows".into(),
            }),
        ),
        tls_probe::Outcome::Verified { version } => d.ok(
            "tls",
            format!("{version}, certificate trusted and valid for {server_name}"),
            ms,
        ),
        tls_probe::Outcome::Untrusted { version, reason, name_mismatch_only } => {
            let detail = format!("{version}, certificate not verified: {reason}");
            let acceptable = mode == Require || (mode == VerifyCa && name_mismatch_only) || mode == Prefer;
            if acceptable {
                d.warn("tls", format!("{detail} (accepted by tls mode {mode:?})"), ms);
            } else {
                d.fail_with(
                    "tls",
                    detail,
                    ms,
                    Some(Hint {
                        cause: "the server certificate is not trusted for this host".into(),
                        next: "add the issuing CA with tls.ca_file (or the wallet), fix tls.server_name, or lower the mode to require".into(),
                    }),
                );
            }
        }
    }
    true
}

async fn login(
    d: &mut Diag,
    backend: Backend,
    params: &ConnectParams,
    step: &str,
) -> Option<Box<dyn Connection>> {
    let started = Instant::now();
    match crate::driver(backend).connect(params).await {
        Ok(conn) => {
            d.ok(
                step,
                format!("connected with {backend}"),
                Some(millis(started)),
            );
            Some(conn)
        }
        Err(e) => {
            d.fail_err(step, Some(backend.dialect()), &e, Some(millis(started)));
            None
        }
    }
}

async fn first_row(conn: &dyn Connection, sql: &str) -> db_core::Result<Vec<Value>> {
    let rs = conn.query(sql, &[], 1).await?;
    Ok(rs.rows.into_iter().next().unwrap_or_default())
}

fn join_row(row: &[Value]) -> String {
    row.iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

async fn server(d: &mut Diag, conn: &dyn Connection, redactor: &mut Redactor) {
    let started = Instant::now();
    match conn.server_version().await {
        Ok(v) => d.ok("server", v.replace('\n', " "), Some(millis(started))),
        Err(e) => d.warn(
            "server",
            format!("version unknown: {}", e.message),
            Some(millis(started)),
        ),
    }
    let queries: &[(&str, &str)] = match conn.dialect() {
        Dialect::Oracle => &[
            (
                "session",
                "SELECT SYS_CONTEXT('USERENV','SESSION_USER'), SYS_CONTEXT('USERENV','CURRENT_SCHEMA'), \
                 SYS_CONTEXT('USERENV','DB_NAME'), SYS_CONTEXT('USERENV','SERVICE_NAME') FROM dual",
            ),
            (
                "charset",
                "SELECT MAX(DECODE(parameter,'NLS_CHARACTERSET',value)), MAX(DECODE(parameter,'NLS_NCHAR_CHARACTERSET',value)) \
                 FROM nls_database_parameters WHERE parameter IN ('NLS_CHARACTERSET','NLS_NCHAR_CHARACTERSET')",
            ),
            ("time zone", "SELECT DBTIMEZONE, SESSIONTIMEZONE FROM dual"),
        ],
        Dialect::Postgres => &[
            (
                "session",
                "SELECT current_user, current_schema(), current_database()",
            ),
            (
                "charset",
                "SELECT current_setting('server_encoding'), current_setting('client_encoding')",
            ),
            ("time zone", "SELECT current_setting('TimeZone')"),
        ],
        Dialect::Sqlite => &[("session", "SELECT sqlite_version()")],
    };
    for (label, sql) in queries {
        let started = Instant::now();
        match first_row(conn, sql).await {
            Ok(row) => {
                if *label == "session" {
                    for (i, v) in row.iter().enumerate().skip(1) {
                        if let Some(t) = v.to_text() {
                            redactor.add(&t, &format!("<session{i}>"));
                        }
                    }
                }
                d.ok(
                    "server",
                    format!("{label}: {}", join_row(&row)),
                    Some(millis(started)),
                );
            }
            Err(e) => d.warn(
                "server",
                format!("{label}: {}", e.message),
                Some(millis(started)),
            ),
        }
    }
}

async fn dictionary(d: &mut Diag, conn: &dyn Connection) {
    let intro = schema::introspector(conn);
    let started = Instant::now();
    let current = match intro.current_schema().await {
        Ok(s) => s,
        Err(e) => {
            let hint = hints::for_error(conn.dialect(), &e);
            d.push(
                "dictionary",
                Status::Warn,
                format!("current schema: {}", e.message),
                Some(millis(started)),
            );
            d.steps.last_mut().expect("pushed").hint = hint.map(Into::into);
            return;
        }
    };
    match intro.schemas().await {
        Ok(s) => d.ok(
            "dictionary",
            format!("{} schemas visible", s.len()),
            Some(millis(started)),
        ),
        Err(e) => d.warn(
            "dictionary",
            format!("schemas: {}", e.message),
            Some(millis(started)),
        ),
    }
    let started = Instant::now();
    let kinds = [schema::ObjectKind::Table, schema::ObjectKind::View];
    match intro.objects(&current, &kinds).await {
        Ok(objs) => {
            let tables = objs
                .iter()
                .filter(|o| o.kind == schema::ObjectKind::Table)
                .count();
            d.ok(
                "dictionary",
                format!(
                    "current schema: {tables} tables, {} views",
                    objs.len() - tables
                ),
                Some(millis(started)),
            );
            if let Some(t) = objs.iter().find(|o| o.kind == schema::ObjectKind::Table) {
                let started = Instant::now();
                match intro.columns(&current, &t.name).await {
                    Ok(cols) => d.ok(
                        "dictionary",
                        format!("columns of one table: {}", cols.len()),
                        Some(millis(started)),
                    ),
                    Err(e) => d.warn(
                        "dictionary",
                        format!("columns: {}", e.message),
                        Some(millis(started)),
                    ),
                }
            }
        }
        Err(e) => {
            let hint = hints::for_error(conn.dialect(), &e);
            d.push(
                "dictionary",
                Status::Warn,
                format!("objects: {}", e.message),
                Some(millis(started)),
            );
            d.steps.last_mut().expect("pushed").hint = hint.map(Into::into);
        }
    }
}

/// One expected value of the type check: column, kind, text prefix.
type Expect = (&'static str, Option<ValueKind>, Option<&'static str>);

fn type_probe(dialect: Dialect) -> (&'static str, Vec<Expect>) {
    use ValueKind::*;
    match dialect {
        Dialect::Oracle => (
            "SELECT 42, 12345678901234567890.0123456789, CAST(1.5 AS BINARY_DOUBLE), 'Grüße', \
             DATE '2026-01-02', TIMESTAMP '2026-01-02 03:04:05.123456', \
             TIMESTAMP '2026-01-02 03:04:05.5 +03:00', HEXTORAW('DEADBEEF'), \
             INTERVAL '1-2' YEAR TO MONTH, NULL FROM dual",
            vec![
                ("integer", Some(Int), Some("42")),
                (
                    "number(38)",
                    Some(Decimal),
                    Some("12345678901234567890.0123456789"),
                ),
                ("binary_double", Some(Float), Some("1.5")),
                ("text", Some(Text), Some("Grüße")),
                ("date", Some(Timestamp), Some("2026-01-02 00:00:00")),
                (
                    "timestamp",
                    Some(Timestamp),
                    Some("2026-01-02 03:04:05.123456"),
                ),
                (
                    "timestamp tz",
                    Some(TimestampTz),
                    Some("2026-01-02 03:04:05.5"),
                ),
                ("raw", Some(Bytes), Some("0xDEADBEEF")),
                ("interval", Some(Interval), None),
                ("null", None, None),
            ],
        ),
        Dialect::Postgres => (
            "SELECT 42::int8, 12345678901234567890.0123456789::numeric, 1.5::float8, 'Grüße'::text, \
             DATE '2026-01-02', TIMESTAMP '2026-01-02 03:04:05.123456', \
             TIMESTAMPTZ '2026-01-02 03:04:05.5+03', '\\xdeadbeef'::bytea, INTERVAL '1 year 2 mons', \
             'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid, '{\"a\": 1}'::jsonb, true, NULL",
            vec![
                ("int8", Some(Int), Some("42")),
                (
                    "numeric",
                    Some(Decimal),
                    Some("12345678901234567890.0123456789"),
                ),
                ("float8", Some(Float), Some("1.5")),
                ("text", Some(Text), Some("Grüße")),
                ("date", Some(Date), Some("2026-01-02")),
                (
                    "timestamp",
                    Some(Timestamp),
                    Some("2026-01-02 03:04:05.123456"),
                ),
                ("timestamptz", Some(TimestampTz), None),
                ("bytea", Some(Bytes), Some("0xDEADBEEF")),
                ("interval", Some(Interval), Some("1 year 2 mons")),
                (
                    "uuid",
                    Some(Uuid),
                    Some("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
                ),
                ("jsonb", Some(Json), Some("{\"a\": 1}")),
                ("bool", Some(Bool), Some("true")),
                ("null", None, None),
            ],
        ),
        Dialect::Sqlite => (
            "SELECT 42, 1.5, 'Grüße', x'DEADBEEF', NULL",
            vec![
                ("integer", Some(Int), Some("42")),
                ("real", Some(Float), Some("1.5")),
                ("text", Some(Text), Some("Grüße")),
                ("blob", Some(Bytes), Some("0xDEADBEEF")),
                ("null", None, None),
            ],
        ),
    }
}

/// Compare a row with the expectations; returns the mismatches.
fn check_types(row: &[Value], expect: &[Expect]) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, (label, kind, text)) in expect.iter().enumerate() {
        let Some(v) = row.get(i) else {
            bad.push(format!("{label}: missing"));
            continue;
        };
        if v.kind() != *kind {
            bad.push(format!("{label}: got {:?} {v}", v.kind()));
        } else if let Some(t) = text
            && !v.to_string().starts_with(t)
        {
            bad.push(format!("{label}: got {v}"));
        }
    }
    bad
}

async fn types(d: &mut Diag, conn: &dyn Connection) {
    let (sql, expect) = type_probe(conn.dialect());
    let started = Instant::now();
    match first_row(conn, sql).await {
        Ok(row) => {
            let bad = check_types(&row, &expect);
            if bad.is_empty() {
                d.ok(
                    "types",
                    format!("{} typed values decoded as expected", expect.len()),
                    Some(millis(started)),
                );
            } else {
                d.warn(
                    "types",
                    format!("unexpected decoding: {}", bad.join("; ")),
                    Some(millis(started)),
                );
            }
        }
        Err(e) => d.warn(
            "types",
            format!("type probe failed: {}", e.message),
            Some(millis(started)),
        ),
    }
}

fn series(dialect: Dialect, n: u32) -> String {
    match dialect {
        Dialect::Oracle => format!("SELECT LEVEL AS n FROM dual CONNECT BY LEVEL <= {n}"),
        Dialect::Postgres => format!("SELECT generate_series(1, {n}) AS n"),
        Dialect::Sqlite => format!(
            "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < {n}) SELECT n FROM c"
        ),
    }
}

async fn paging(d: &mut Diag, conn: &dyn Connection) {
    let opts = ExecOptions {
        fetch_size: 500,
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let mut cursor = match conn
        .execute(&series(conn.dialect(), 5000), &[], &opts)
        .await
    {
        Ok(Outcome::Rows(c)) => c,
        Ok(Outcome::Affected(_)) => {
            return d.warn("paging", "the generator returned no rows", None);
        }
        Err(e) => return d.warn("paging", e.message, Some(millis(started))),
    };
    let mut seen = 0i64;
    let mut first_ms = None;
    for _ in 0..2 {
        match cursor.next_page(500).await {
            Ok(page) => {
                first_ms.get_or_insert_with(|| millis(started));
                for row in page {
                    seen += 1;
                    // Compare as text: drivers may return NUMBER as Int or Decimal.
                    if row.first().map(Value::to_string) != Some(seen.to_string()) {
                        return d.warn("paging", format!("row {seen} out of order"), None);
                    }
                }
            }
            Err(e) => return d.warn("paging", e.message, None),
        }
    }
    let close_started = Instant::now();
    let closed = cursor.close().await;
    let ping = conn.ping().await;
    match (closed, ping) {
        (Ok(()), Ok(())) => d.ok(
            "paging",
            format!(
                "2 pages of 500 in order (first page {} ms); early close and next statement {} ms",
                first_ms.unwrap_or_default(),
                millis(close_started)
            ),
            Some(millis(started)),
        ),
        (Err(e), _) | (_, Err(e)) => d.warn("paging", format!("early close: {}", e.message), None),
    }
}

/// Run a sleep with a 1 s timeout. The connection may be lost afterwards on
/// backends without server-side cancel; reconnect then.
async fn timeouts(
    d: &mut Diag,
    conn: Box<dyn Connection>,
    backend: Backend,
    params: &ConnectParams,
) -> Option<Box<dyn Connection>> {
    let sql = match conn.dialect() {
        Dialect::Oracle => "BEGIN DBMS_SESSION.SLEEP(10); END;",
        Dialect::Postgres => "SELECT pg_sleep(10)",
        Dialect::Sqlite => {
            d.skip("timeout", "not checked for SQLite");
            return Some(conn);
        }
    };
    let opts = ExecOptions {
        timeout: Some(Duration::from_secs(1)),
        ..ExecOptions::default()
    };
    let started = Instant::now();
    let r = conn.execute(sql, &[], &opts).await;
    let ms = Some(millis(started));
    match r {
        Err(e) if e.kind == ErrorKind::Timeout => match conn.ping().await {
            Ok(()) => {
                d.ok(
                    "timeout",
                    "1 s statement timeout honoured; connection still usable",
                    ms,
                );
                Some(conn)
            }
            Err(_) => {
                d.ok(
                    "timeout",
                    "1 s statement timeout honoured; connection had to be reopened",
                    ms,
                );
                crate::driver(backend).connect(params).await.ok()
            }
        },
        Err(e) => {
            d.warn(
                "timeout",
                format!("expected a timeout, got: {}", e.message),
                ms,
            );
            Some(conn)
        }
        Ok(_) => {
            d.warn(
                "timeout",
                "the statement was not interrupted by the timeout",
                ms,
            );
            Some(conn)
        }
    }
}

/// Plain-text rendering for the CLI.
pub fn render(report: &Report) -> String {
    let mut out = format!("{} · {}\n", report.tool, report.platform);
    for s in &report.steps {
        let ms = s.millis.map(|m| format!(" ({m} ms)")).unwrap_or_default();
        let code = s
            .code
            .as_deref()
            .map(|c| format!("[{c}] "))
            .unwrap_or_default();
        out.push_str(&format!(
            "{} {:<11} {code}{}{ms}\n",
            s.status.label(),
            s.name,
            s.detail
        ));
        if let Some(h) = &s.hint {
            out.push_str(&format!(
                "       cause: {}\n       next:  {}\n",
                h.cause, h.next
            ));
        }
    }
    out.push_str(&match (&report.result, &report.failed_at) {
        (_, Some(step)) => format!("Result: FAIL at {step}"),
        (Status::Warn, None) => "Result: OK with warnings".to_owned(),
        _ => "Result: OK".to_owned(),
    });
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use secrets::MemoryStore;
    use std::path::Path;

    fn svc(dir: &Path, env: &[(&'static str, &'static str)]) -> DbService {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        DbService::new(
            dir.join("p.toml"),
            Arc::new(MemoryStore::default()),
            None,
            Arc::new(move |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())),
        )
    }

    #[test]
    fn type_check_reports_mismatches() {
        let (_, expect) = type_probe(Dialect::Sqlite);
        let good = vec![
            Value::Int(42),
            Value::Float(1.5),
            Value::from("Grüße"),
            Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
            Value::Null,
        ];
        assert!(check_types(&good, &expect).is_empty());
        let mut bad = good.clone();
        bad[0] = Value::from("42");
        assert_eq!(check_types(&bad, &expect).len(), 1);
    }

    #[tokio::test]
    async fn unknown_profile_fails_at_profile() {
        let dir = tempfile::tempdir().unwrap();
        let r = diagnose(
            &svc(dir.path(), &[]),
            &Target::Profile("nope".into()),
            &DiagnoseOptions::default(),
        )
        .await;
        assert_eq!(r.failed_at.as_deref(), Some("profile"));
        assert!(render(&r).contains("Result: FAIL at profile"));
    }

    #[tokio::test]
    async fn missing_password_fails_at_secret() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Profile::new(Backend::Postgres);
        p.host = "localhost".into();
        p.database = Some("d".into());
        p.user = "u".into();
        let r = diagnose(
            &svc(dir.path(), &[]),
            &Target::Connection(Box::new(p)),
            &DiagnoseOptions::default(),
        )
        .await;
        assert_eq!(r.failed_at.as_deref(), Some("secret"));
        assert!(
            r.steps
                .last()
                .unwrap()
                .hint
                .as_ref()
                .unwrap()
                .next
                .contains("secret set")
        );
    }

    #[tokio::test]
    async fn closed_port_fails_at_tcp_and_share_masks() {
        // Bind and drop a listener to get a port that refuses connections.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let dir = tempfile::tempdir().unwrap();
        let mut p = Profile::new(Backend::OracleThin);
        p.host = "127.0.0.1".into();
        p.port = Some(port);
        p.service = Some("salesdb".into());
        p.user = "alice_app".into();
        p.connect_timeout_secs = Some(2);
        let opts = DiagnoseOptions {
            share: true,
            ..Default::default()
        };
        let r = diagnose(
            &svc(dir.path(), &[("APP_DB_PASSWORD", "hunter2-secret")]),
            &Target::Connection(Box::new(p)),
            &opts,
        )
        .await;
        assert_eq!(r.failed_at.as_deref(), Some("tcp"), "{}", render(&r));
        let text = render(&r);
        for leak in ["hunter2-secret", "alice_app", "salesdb", "127.0.0.1"] {
            assert!(!text.contains(leak), "{leak} leaked:\n{text}");
        }
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("hunter2-secret"));
    }

    #[test]
    fn missing_wallet_dir_fails_files() {
        let mut p = Profile::new(Backend::OracleThin);
        p.host = "h".into();
        p.service = Some("s".into());
        p.tls = Some(crate::profiles::TlsProfile {
            mode: db_core::TlsMode::Require,
            wallet_dir: Some("/definitely/not/here".into()),
            ..Default::default()
        });
        let mut d = Diag { steps: Vec::new() };
        files(&mut d, &p);
        assert_eq!(d.steps[0].status, Status::Fail);
    }
}
