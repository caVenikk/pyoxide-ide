//! JDBC URLs as written by PyCharm data sources, turned into neutral
//! connection fields. Passwords embedded in a URL are dropped, never
//! returned.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

/// Which kind of database a URL points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendHint {
    Oracle,
    Postgres,
    Sqlite,
}

impl BackendHint {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendHint::Oracle => "oracle",
            BackendHint::Postgres => "postgres",
            BackendHint::Sqlite => "sqlite",
        }
    }
}

/// Connection fields from a JDBC URL. Empty parts are `None`; defaults
/// (ports, `localhost`) are left to the consumer unless the URL form
/// implies them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedUrl {
    pub backend_hint: BackendHint,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Oracle service name.
    pub service: Option<String>,
    /// Oracle SID.
    pub sid: Option<String>,
    /// PostgreSQL database.
    pub database: Option<String>,
    /// SQLite file, macros expanded (`:memory:` for an in-memory database).
    pub path: Option<String>,
    /// Oracle connect string to pass on as is: a full descriptor, a TNS
    /// alias or an LDAP URL.
    pub dsn: Option<String>,
    /// TLS mode in the names of PostgreSQL's `sslmode`: `disable`,
    /// `prefer`, `require`, `verify-ca`, `verify-full`. `None` when the URL
    /// does not say.
    pub tls_mode: Option<String>,
    /// User name found in the URL itself.
    pub user: Option<String>,
    /// Remaining URL parameters (never secrets).
    pub params: BTreeMap<String, String>,
    /// Things worth telling the user: values dropped, alternatives ignored.
    pub notes: Vec<String>,
}

impl ParsedUrl {
    fn new(backend_hint: BackendHint) -> Self {
        Self {
            backend_hint,
            host: None,
            port: None,
            service: None,
            sid: None,
            database: None,
            path: None,
            dsn: None,
            tls_mode: None,
            user: None,
            params: BTreeMap::new(),
            notes: Vec::new(),
        }
    }
}

/// Values for the IDE path macros used in URLs.
#[derive(Debug, Clone, Copy, Default)]
pub struct UrlContext<'a> {
    /// `$PROJECT_DIR$`: the directory that contains `.idea`.
    pub project_dir: Option<&'a Path>,
    /// `$USER_HOME$`.
    pub user_home: Option<&'a Path>,
}

/// Why a URL could not be turned into connection fields.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct UrlError {
    /// JDBC sub-protocol, e.g. `mysql`; empty if the URL is not JDBC.
    pub subprotocol: String,
    pub reason: String,
}

fn fail(subprotocol: &str, reason: impl Into<String>) -> UrlError {
    UrlError {
        subprotocol: subprotocol.to_owned(),
        reason: reason.into(),
    }
}

/// Parameter names whose values are secrets: dropped with a note.
pub(crate) fn is_secret_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["password", "passwd", "pwd", "secret", "token", "credential"]
        .iter()
        .any(|s| n.contains(s))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let digit = |at: usize| bytes.get(at).copied().and_then(hex);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (digit(i + 1), digit(i + 2))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        c @ b'0'..=b'9' => Some(c - b'0'),
        c @ b'a'..=b'f' => Some(c - b'a' + 10),
        c @ b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Split `a=b&c=d` into the result, dropping secrets.
fn parse_params(query: &str, p: &mut ParsedUrl) {
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (percent_decode(k), percent_decode(v));
        if is_secret_name(&k) {
            p.notes.push(format!(
                "URL parameter {k} holds a secret; it was not imported"
            ));
            continue;
        }
        p.params.insert(k, v);
    }
}

fn parse_port(s: &str, sub: &str) -> Result<u16, UrlError> {
    s.parse()
        .map_err(|_| fail(sub, format!("invalid port {s:?}")))
}

/// `host[:port]` with IPv6 in brackets. Returns host and what follows it.
fn take_host(s: &str) -> (&str, &str) {
    if let Some(rest) = s.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return (&rest[..end], &rest[end + 1..]);
    }
    match s.find(':') {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// Parse a JDBC URL. `ctx` expands `$PROJECT_DIR$` and `$USER_HOME$` in
/// SQLite paths.
pub fn parse_jdbc_url(url: &str, ctx: &UrlContext<'_>) -> Result<ParsedUrl, UrlError> {
    let url = url.trim();
    let Some(rest) = url.strip_prefix("jdbc:") else {
        return Err(fail("", "not a JDBC URL"));
    };
    let (sub, tail) = rest.split_once(':').unwrap_or((rest, ""));
    match sub {
        "oracle" => parse_oracle(tail),
        "postgresql" => parse_postgres(tail),
        "sqlite" => Ok(parse_sqlite(tail, ctx)),
        "" => Err(fail(sub, "not a JDBC URL")),
        other => Err(fail(other, format!("unsupported driver: {other}"))),
    }
}

// --------------------------------------------------------------- PostgreSQL

fn parse_postgres(tail: &str) -> Result<ParsedUrl, UrlError> {
    const SUB: &str = "postgresql";
    let mut p = ParsedUrl::new(BackendHint::Postgres);
    let (main, query) = tail.split_once('?').unwrap_or((tail, ""));
    parse_params(query, &mut p);

    if let Some(after) = main.strip_prefix("//") {
        let (authority, db) = after.split_once('/').unwrap_or((after, ""));
        let mut hosts = authority.split(',').filter(|h| !h.is_empty());
        if let Some(first) = hosts.next() {
            let (host, after_host) = take_host(first);
            if !host.is_empty() {
                p.host = Some(percent_decode(host));
            }
            if let Some(port) = after_host.strip_prefix(':').filter(|s| !s.is_empty()) {
                p.port = Some(parse_port(port, SUB)?);
            }
        }
        let others = hosts.count();
        if others > 0 {
            p.notes.push(format!(
                "{} more host(s) listed for failover; only the first is used",
                others
            ));
        }
        if p.host.is_none() {
            p.host = Some("localhost".into());
        }
        if !db.is_empty() {
            p.database = Some(percent_decode(db));
        }
    } else {
        // `jdbc:postgresql:database` or `jdbc:postgresql:/`: local server.
        p.host = Some("localhost".into());
        let db = main.trim_start_matches('/');
        if !db.is_empty() {
            p.database = Some(percent_decode(db));
        }
    }

    if let Some(user) = p.params.remove("user") {
        p.user = Some(user).filter(|u| !u.is_empty());
    }
    let ssl = p.params.get("ssl").map(|v| v.to_ascii_lowercase());
    if let Some(mode) = p.params.get("sslmode").map(|v| v.to_ascii_lowercase()) {
        p.tls_mode = Some(match mode.as_str() {
            "disable" | "prefer" | "require" | "verify-ca" | "verify-full" => mode,
            "allow" => {
                p.notes.push("sslmode=allow is imported as prefer".into());
                "prefer".into()
            }
            other => return Err(fail(SUB, format!("unknown sslmode {other:?}"))),
        });
    } else if matches!(ssl.as_deref(), Some("" | "true")) {
        // pgJDBC: `ssl=true` without `sslmode` means verify-full.
        p.tls_mode = Some("verify-full".into());
    } else if ssl.as_deref() == Some("false") {
        p.tls_mode = Some("disable".into());
    }
    Ok(p)
}

// ------------------------------------------------------------------- Oracle

fn parse_oracle(tail: &str) -> Result<ParsedUrl, UrlError> {
    const SUB: &str = "oracle";
    let (driver, rest) = tail.split_once(':').unwrap_or((tail, ""));
    if !matches!(driver, "thin" | "oci" | "oci8") {
        return Err(fail(
            SUB,
            format!("unsupported Oracle driver type {driver:?}"),
        ));
    }
    let mut p = ParsedUrl::new(BackendHint::Oracle);

    // Optional `user[/password]@` before the target.
    let target = if let Some(t) = rest.strip_prefix('@') {
        t
    } else if !rest.starts_with('(')
        && let Some(at) = rest.rfind('@')
    {
        let creds = &rest[..at];
        let (user, password) = creds.split_once('/').unwrap_or((creds, ""));
        if !user.is_empty() {
            p.user = Some(user.to_owned());
        }
        if !password.is_empty() {
            p.notes
                .push("the URL contains a password; it was not imported".into());
        }
        &rest[at + 1..]
    } else {
        rest
    };
    let target = target.trim();
    if target.is_empty() {
        return Err(fail(SUB, "no connect target after '@'"));
    }

    if target.starts_with('(') {
        parse_descriptor(target, &mut p);
        p.dsn = Some(target.to_owned());
        return Ok(p);
    }
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("ldap://") || lower.starts_with("ldaps://") {
        p.dsn = Some(target.to_owned());
        p.notes
            .push("LDAP directory naming needs the Oracle Client (oracle-odpi)".into());
        return Ok(p);
    }

    let (main, query) = target.split_once('?').unwrap_or((target, ""));
    parse_params(query, &mut p);
    let mut main = main;
    if let Some(rest) = strip_prefix_ci(main, "tcps://") {
        main = rest;
        p.tls_mode = Some(oracle_tls_mode(&p.params));
    } else if let Some(rest) = strip_prefix_ci(main, "tcp://") {
        main = rest;
    }
    let main = main.trim_start_matches("//");

    let (hosts, service) = match main.find('/') {
        Some(i) => (&main[..i], Some(&main[i + 1..])),
        None => (main, None),
    };
    // A bare word: a TNS alias resolved through tnsnames.ora.
    if service.is_none() && !hosts.contains(':') && !hosts.contains(',') {
        p.dsn = Some(hosts.to_owned());
        if !p.params.contains_key("TNS_ADMIN") {
            p.notes.push(
                "TNS alias: it is resolved through tnsnames.ora (TNS_ADMIN or the default location)"
                    .into(),
            );
        }
        return Ok(p);
    }

    let mut list = hosts.split(',').filter(|h| !h.is_empty());
    let first = list.next().unwrap_or("");
    let others = list.count();
    if others > 0 {
        p.notes.push(format!(
            "{others} more host(s) listed for failover; only the first is used"
        ));
    }
    let (host, after_host) = take_host(first);
    if host.is_empty() {
        return Err(fail(SUB, "no host in the connect string"));
    }
    p.host = Some(host.to_owned());
    if let Some(after) = after_host.strip_prefix(':') {
        let (port, sid) = after.split_once(':').unwrap_or((after, ""));
        if !port.is_empty() {
            p.port = Some(parse_port(port, SUB)?);
        }
        if !sid.is_empty() {
            p.sid = Some(sid.to_owned());
        }
    }
    if let Some(service) = service {
        let end = service.find([':', '/']).unwrap_or(service.len());
        if end < service.len() {
            p.notes.push(format!(
                "server type / instance name {:?} is ignored",
                &service[end..]
            ));
        }
        let name = &service[..end];
        if !name.is_empty() {
            p.service = Some(name.to_owned());
        }
    }
    if p.service.is_some() && p.sid.is_some() {
        return Err(fail(SUB, "both a SID and a service name"));
    }
    if p.service.is_none() && p.sid.is_none() {
        p.notes
            .push("no service name or SID: the listener default service is used".into());
    }
    Ok(p)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &s[prefix.len()..])
}

fn oracle_tls_mode(params: &BTreeMap<String, String>) -> String {
    let dn_match = params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("ssl_server_dn_match"))
        .map(|(_, v)| v.to_ascii_lowercase());
    match dn_match.as_deref() {
        Some("true" | "yes" | "on") => "verify-full".into(),
        _ => "verify-ca".into(),
    }
}

/// First value of `(KEY=value)` in a descriptor, case-insensitive key.
fn descriptor_value(desc: &str, key: &str) -> Option<String> {
    let upper = desc.to_ascii_uppercase();
    let pat = format!("({key}");
    let mut from = 0;
    while let Some(i) = upper[from..].find(&pat) {
        let start = from + i + pat.len();
        let rest = &desc[start..];
        let trimmed = rest.trim_start();
        if let Some(value) = trimmed.strip_prefix('=') {
            let end = value.find(')').unwrap_or(value.len());
            let v = value[..end].trim();
            return (!v.is_empty()).then(|| v.to_owned());
        }
        from = start;
    }
    None
}

fn parse_descriptor(desc: &str, p: &mut ParsedUrl) {
    p.host = descriptor_value(desc, "HOST");
    p.port = descriptor_value(desc, "PORT").and_then(|v| v.parse().ok());
    p.service = descriptor_value(desc, "SERVICE_NAME");
    if p.service.is_none() {
        p.sid = descriptor_value(desc, "SID");
    }
    if descriptor_value(desc, "PROTOCOL").is_some_and(|v| v.eq_ignore_ascii_case("tcps")) {
        let dn = descriptor_value(desc, "SSL_SERVER_DN_MATCH").map(|v| v.to_ascii_lowercase());
        p.tls_mode = Some(match dn.as_deref() {
            Some("true" | "yes" | "on") => "verify-full".into(),
            _ => "verify-ca".into(),
        });
    }
    if desc.to_ascii_uppercase().matches("(ADDRESS").count() > 1 {
        p.notes.push(
            "the descriptor lists several addresses; host and port show the first, the full \
             descriptor is kept as dsn"
                .into(),
        );
    }
}

// ------------------------------------------------------------------- SQLite

fn expand_macros(s: &str, ctx: &UrlContext<'_>, notes: &mut Vec<String>) -> String {
    let fwd = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut out = s.to_owned();
    if let Some(dir) = ctx.project_dir {
        out = out.replace("$PROJECT_DIR$", &fwd(dir));
    }
    if let Some(home) = ctx.user_home {
        out = out.replace("$USER_HOME$", &fwd(home));
    }
    // Anything still looking like `$NAME$` stays as is.
    let mut rest = out.as_str();
    while let Some(start) = rest.find('$') {
        let after = &rest[start + 1..];
        match after.find('$') {
            Some(end)
                if end > 0
                    && after[..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                notes.push(format!("path macro ${}$ was not expanded", &after[..end]));
                rest = &after[end + 1..];
            }
            _ => break,
        }
    }
    out
}

fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

fn parse_sqlite(tail: &str, ctx: &UrlContext<'_>) -> ParsedUrl {
    let mut p = ParsedUrl::new(BackendHint::Sqlite);
    let mut path = tail.trim();
    if let Some(rest) = path.strip_prefix("file:") {
        let (file, query) = rest.split_once('?').unwrap_or((rest, ""));
        parse_params(query, &mut p);
        path = file;
    }
    if path.is_empty() || path == ":memory:" {
        p.path = Some(":memory:".into());
        return p;
    }
    let mut expanded = expand_macros(path, ctx, &mut p.notes);
    if !is_absolute(&expanded)
        && !expanded.starts_with('$')
        && let Some(dir) = ctx.project_dir
    {
        p.notes.push(format!(
            "relative path {expanded:?} resolved against the project directory"
        ));
        expanded = format!(
            "{}/{}",
            dir.to_string_lossy()
                .replace('\\', "/")
                .trim_end_matches('/'),
            expanded.trim_start_matches("./")
        );
    }
    p.path = Some(expanded);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(url: &str) -> ParsedUrl {
        let ctx = UrlContext {
            project_dir: Some(Path::new("/work/demo")),
            user_home: Some(Path::new("/home/dev")),
        };
        parse_jdbc_url(url, &ctx).unwrap_or_else(|e| panic!("{url}: {e}"))
    }

    fn reason(url: &str) -> String {
        parse_jdbc_url(url, &UrlContext::default())
            .unwrap_err()
            .reason
    }

    #[test]
    fn oracle_service_forms() {
        let p = parse("jdbc:oracle:thin:@//db.example.com:1522/ORCLPDB1");
        assert_eq!(p.backend_hint, BackendHint::Oracle);
        assert_eq!(p.host.as_deref(), Some("db.example.com"));
        assert_eq!(p.port, Some(1522));
        assert_eq!(p.service.as_deref(), Some("ORCLPDB1"));
        assert_eq!(p.sid, None);
        assert!(p.notes.is_empty(), "{:?}", p.notes);

        let p = parse("jdbc:oracle:thin:@db.example.com:1521/sales.example.com");
        assert_eq!(p.service.as_deref(), Some("sales.example.com"));
        let p = parse("jdbc:oracle:thin:@//db.example.com/FREEPDB1");
        assert_eq!((p.port, p.service.as_deref()), (None, Some("FREEPDB1")));
        let p = parse("jdbc:oracle:oci:@//db.example.com:1521/SVC:dedicated");
        assert_eq!(p.service.as_deref(), Some("SVC"));
        assert_eq!(p.notes.len(), 1);
    }

    #[test]
    fn oracle_sid() {
        let p = parse("jdbc:oracle:thin:@db.example.com:1521:ORCL");
        assert_eq!(p.host.as_deref(), Some("db.example.com"));
        assert_eq!(p.port, Some(1521));
        assert_eq!(p.sid.as_deref(), Some("ORCL"));
        assert_eq!(p.service, None);
    }

    #[test]
    fn oracle_user_and_password_in_url() {
        let p = parse("jdbc:oracle:thin:app_user/hunter2@//db.example.com:1521/SVC");
        assert_eq!(p.user.as_deref(), Some("app_user"));
        assert_eq!(p.service.as_deref(), Some("SVC"));
        let dump = format!("{p:?}");
        assert!(!dump.contains("hunter2"), "{dump}");
        assert!(p.notes.iter().any(|n| n.contains("password")));
        let p = parse("jdbc:oracle:thin:app_user@db.example.com:1521:SID1");
        assert_eq!(p.user.as_deref(), Some("app_user"));
        assert!(p.notes.is_empty());
    }

    #[test]
    fn oracle_descriptor() {
        let d = "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=db1.example.com)(PORT=1530))\
                 (CONNECT_DATA=(SERVICE_NAME=REPORTS)))";
        let p = parse(&format!("jdbc:oracle:thin:@{d}"));
        assert_eq!(p.dsn.as_deref(), Some(d));
        assert_eq!(p.host.as_deref(), Some("db1.example.com"));
        assert_eq!(p.port, Some(1530));
        assert_eq!(p.service.as_deref(), Some("REPORTS"));
        assert_eq!(p.tls_mode, None);

        let d = "(description = (address_list = (address = (protocol = tcps)(host = a.example.com)(port = 2484))\
                 (address = (protocol = tcps)(host = b.example.com)(port = 2484)))\
                 (connect_data = (sid = LEGACY))(security = (ssl_server_dn_match = yes)))";
        let p = parse(&format!("jdbc:oracle:thin:@{d}"));
        assert_eq!(p.host.as_deref(), Some("a.example.com"));
        assert_eq!(p.sid.as_deref(), Some("LEGACY"));
        assert_eq!(p.tls_mode.as_deref(), Some("verify-full"));
        assert_eq!(p.notes.len(), 1);
    }

    #[test]
    fn oracle_alias_and_tns_admin() {
        let p = parse("jdbc:oracle:thin:@REPORTS_HIGH?TNS_ADMIN=/opt/wallets/reports");
        assert_eq!(p.dsn.as_deref(), Some("REPORTS_HIGH"));
        assert_eq!(
            p.params.get("TNS_ADMIN").map(String::as_str),
            Some("/opt/wallets/reports")
        );
        assert!(p.host.is_none());
        assert!(p.notes.is_empty());
        let p = parse("jdbc:oracle:thin:@REPORTS");
        assert_eq!(p.dsn.as_deref(), Some("REPORTS"));
        assert_eq!(p.notes.len(), 1);
    }

    #[test]
    fn oracle_ezconnect_plus() {
        let p = parse(
            "jdbc:oracle:thin:@tcps://adb.example.com:1522/abc_high.example.com?wallet_location=/w&ssl_server_dn_match=true",
        );
        assert_eq!(p.host.as_deref(), Some("adb.example.com"));
        assert_eq!(p.port, Some(1522));
        assert_eq!(p.service.as_deref(), Some("abc_high.example.com"));
        assert_eq!(p.tls_mode.as_deref(), Some("verify-full"));
        assert_eq!(
            p.params.get("wallet_location").map(String::as_str),
            Some("/w")
        );

        let p = parse("jdbc:oracle:thin:@tcp://h1.example.com:1521,h2.example.com:1521/SVC");
        assert_eq!(p.host.as_deref(), Some("h1.example.com"));
        assert_eq!(p.tls_mode, None);
        assert!(p.notes[0].contains("1 more host"));

        let p = parse("jdbc:oracle:thin:@[2001:db8::1]:1521/SVC");
        assert_eq!(p.host.as_deref(), Some("2001:db8::1"));
        assert_eq!(p.port, Some(1521));

        let p = parse("jdbc:oracle:thin:@ldap://ldap.example.com:389/sales,cn=OracleContext");
        assert!(p.dsn.unwrap().starts_with("ldap://"));
    }

    #[test]
    fn oracle_errors() {
        assert!(reason("jdbc:oracle:kprb:").contains("driver type"));
        assert!(reason("jdbc:oracle:thin:@").contains("no connect target"));
        assert!(reason("jdbc:oracle:thin:@//h:port/S").contains("invalid port"));
        assert!(reason("jdbc:oracle:thin:@h:1521:SID/SVC").contains("both"));
    }

    #[test]
    fn postgres_forms() {
        let p = parse("jdbc:postgresql://pg.example.com:5433/app_db");
        assert_eq!(p.backend_hint, BackendHint::Postgres);
        assert_eq!(p.host.as_deref(), Some("pg.example.com"));
        assert_eq!(p.port, Some(5433));
        assert_eq!(p.database.as_deref(), Some("app_db"));
        assert_eq!(p.tls_mode, None);

        let p = parse("jdbc:postgresql://pg.example.com/app_db");
        assert_eq!(p.port, None);
        let p = parse("jdbc:postgresql://pg.example.com/");
        assert_eq!(p.database, None);
        let p = parse("jdbc:postgresql:app_db");
        assert_eq!(
            (p.host.as_deref(), p.database.as_deref()),
            (Some("localhost"), Some("app_db"))
        );
        let p = parse("jdbc:postgresql:/");
        assert_eq!((p.host.as_deref(), p.database), (Some("localhost"), None));
        let p = parse("jdbc:postgresql://[2001:db8::5]:5432/db");
        assert_eq!(p.host.as_deref(), Some("2001:db8::5"));
        let p = parse("jdbc:postgresql://:5432/db");
        assert_eq!(p.host.as_deref(), Some("localhost"));
    }

    #[test]
    fn postgres_params() {
        let p = parse(
            "jdbc:postgresql://pg.example.com/app?sslmode=verify-ca&user=app_user&password=hunter2&currentSchema=billing&ApplicationName=my%20app",
        );
        assert_eq!(p.tls_mode.as_deref(), Some("verify-ca"));
        assert_eq!(p.user.as_deref(), Some("app_user"));
        assert_eq!(
            p.params.get("currentSchema").map(String::as_str),
            Some("billing")
        );
        assert_eq!(
            p.params.get("ApplicationName").map(String::as_str),
            Some("my app")
        );
        assert!(!p.params.contains_key("password"));
        assert!(!p.params.contains_key("user"));
        assert!(!format!("{p:?}").contains("hunter2"));
        assert_eq!(p.notes.len(), 1);

        assert_eq!(
            parse("jdbc:postgresql://h/d?ssl=true").tls_mode.as_deref(),
            Some("verify-full")
        );
        assert_eq!(
            parse("jdbc:postgresql://h/d?ssl=true&sslmode=require")
                .tls_mode
                .as_deref(),
            Some("require")
        );
        assert_eq!(
            parse("jdbc:postgresql://h/d?sslmode=allow")
                .tls_mode
                .as_deref(),
            Some("prefer")
        );
        assert_eq!(
            parse("jdbc:postgresql://h/d?ssl=false").tls_mode.as_deref(),
            Some("disable")
        );
        assert!(reason("jdbc:postgresql://h/d?sslmode=sometimes").contains("sslmode"));
        let p = parse("jdbc:postgresql://h/d?sslpassword=x&sslkey=/k.pk8");
        assert!(!p.params.contains_key("sslpassword"));
        assert_eq!(p.params.get("sslkey").map(String::as_str), Some("/k.pk8"));
    }

    #[test]
    fn postgres_multi_host() {
        let p = parse(
            "jdbc:postgresql://pg1.example.com:5432,pg2.example.com:5433/app?targetServerType=primary",
        );
        assert_eq!(p.host.as_deref(), Some("pg1.example.com"));
        assert_eq!(p.port, Some(5432));
        assert_eq!(p.database.as_deref(), Some("app"));
        assert!(p.notes[0].contains("1 more host"));
        assert_eq!(
            p.params.get("targetServerType").map(String::as_str),
            Some("primary")
        );
    }

    #[test]
    fn sqlite_paths() {
        let p = parse("jdbc:sqlite:$PROJECT_DIR$/data/app.db");
        assert_eq!(p.backend_hint, BackendHint::Sqlite);
        assert_eq!(p.path.as_deref(), Some("/work/demo/data/app.db"));
        assert!(p.notes.is_empty());
        let p = parse("jdbc:sqlite:$USER_HOME$/dbs/x.sqlite");
        assert_eq!(p.path.as_deref(), Some("/home/dev/dbs/x.sqlite"));
        let p = parse("jdbc:sqlite:C:\\data\\app.db");
        assert_eq!(p.path.as_deref(), Some("C:\\data\\app.db"));
        let p = parse("jdbc:sqlite:/var/data/app.db");
        assert_eq!(p.path.as_deref(), Some("/var/data/app.db"));
        assert_eq!(
            parse("jdbc:sqlite::memory:").path.as_deref(),
            Some(":memory:")
        );
        assert_eq!(parse("jdbc:sqlite:").path.as_deref(), Some(":memory:"));
        let p = parse("jdbc:sqlite:file:$PROJECT_DIR$/a.db?mode=ro");
        assert_eq!(p.path.as_deref(), Some("/work/demo/a.db"));
        assert_eq!(p.params.get("mode").map(String::as_str), Some("ro"));
        let p = parse("jdbc:sqlite:local.db");
        assert_eq!(p.path.as_deref(), Some("/work/demo/local.db"));
        assert_eq!(p.notes.len(), 1);
        let p = parse("jdbc:sqlite:$MODULE_DIR$/m.db");
        assert_eq!(p.path.as_deref(), Some("$MODULE_DIR$/m.db"));
        assert!(p.notes[0].contains("MODULE_DIR"));
    }

    #[test]
    fn sqlite_without_context_keeps_macros() {
        let p = parse_jdbc_url("jdbc:sqlite:$PROJECT_DIR$/a.db", &UrlContext::default()).unwrap();
        assert_eq!(p.path.as_deref(), Some("$PROJECT_DIR$/a.db"));
        assert!(p.notes[0].contains("PROJECT_DIR"));
    }

    #[test]
    fn unsupported() {
        let e = parse_jdbc_url("jdbc:mysql://h:3306/db", &UrlContext::default()).unwrap_err();
        assert_eq!(e.subprotocol, "mysql");
        assert!(e.reason.contains("unsupported driver"));
        assert!(reason("postgresql://h/db").contains("not a JDBC URL"));
        assert!(reason("jdbc:").contains("not a JDBC URL"));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
