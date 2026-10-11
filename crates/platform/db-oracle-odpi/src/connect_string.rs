use std::path::PathBuf;
use std::time::Duration;

use db_core::{ConnectParams, Error, Result, TlsMode, opt};

/// Option key: directory with the Oracle Client libraries (Instant Client).
/// Only effective before the client is first loaded in the process.
pub const OPT_LIB_DIR: &str = db_core::opt::ORACLE_LIB_DIR;
/// Option key: directory with `tnsnames.ora`, `sqlnet.ora` and wallets,
/// the equivalent of `TNS_ADMIN`. Only effective before the client is first
/// loaded in the process.
pub const OPT_CONFIG_DIR: &str = db_core::opt::ORACLE_CONFIG_DIR;

const MAX_CONNECT_TIMEOUT_SECS: u64 = 3600;
const DEFAULT_PORT: u16 = 1521;

/// Everything `connect` needs that is derived from [`ConnectParams`], except
/// the password. Built by a pure function so it can be tested without a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectPlan {
    pub connect_string: String,
    pub username: String,
    /// OS, Kerberos or wallet based authentication: no user name, no password.
    pub external_auth: bool,
    pub lib_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub connect_timeout: Duration,
    pub read_only: bool,
}

pub(crate) fn plan(p: &ConnectParams) -> Result<ConnectPlan> {
    let external_auth = p.user.is_empty() && p.password.is_empty();
    Ok(ConnectPlan {
        connect_string: build_connect_string(p, external_auth)?,
        username: p.user.clone(),
        external_auth,
        lib_dir: dir_option(p, OPT_LIB_DIR),
        config_dir: dir_option(p, OPT_CONFIG_DIR),
        connect_timeout: p.connect_timeout,
        read_only: p.read_only,
    })
}

fn dir_option(p: &ConnectParams, key: &str) -> Option<PathBuf> {
    p.option(key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// The connect string handed to the Oracle Client.
///
/// - `oracle.dsn` is used verbatim (TNS alias, Easy Connect Plus or a full
///   descriptor); the client resolves it with `TNS_ADMIN`/`sqlnet.ora`.
/// - Otherwise a plain `//host:port/service` is built when nothing else is
///   needed, and a full descriptor when a SID, TLS, a wallet or a connect
///   timeout is involved. The descriptor form works with every client
///   version (12.2 and later for the timeout entries), unlike the Easy
///   Connect Plus `?key=value` parameters, which need a 19c client.
/// - With external authentication and no host or database the string is
///   empty: a local connection through `ORACLE_SID`.
pub(crate) fn build_connect_string(p: &ConnectParams, external_auth: bool) -> Result<String> {
    if let Some(dsn) = p.option(opt::ORACLE_DSN).map(str::trim)
        && !dsn.is_empty()
    {
        if dsn.chars().any(char::is_control) {
            return Err(Error::config(
                "the Oracle connect string must not contain control characters",
            ));
        }
        return Ok(dsn.to_owned());
    }
    if p.host.trim().is_empty() && p.database.trim().is_empty() && external_auth {
        return Ok(String::new());
    }

    let host = host_literal(&p.host)?;
    let port = if p.port == 0 { DEFAULT_PORT } else { p.port };
    let is_sid = p.flag(opt::ORACLE_SID);
    let target = name_literal(if is_sid { "SID" } else { "service name" }, &p.database)?;

    let tls = &p.tls;
    let tcps = tls.mode.is_enabled();
    let mut security: Vec<String> = Vec::new();
    if tcps {
        let has_pem =
            tls.ca_file.is_some() || tls.client_cert.is_some() || tls.client_key.is_some();
        if has_pem && tls.wallet_dir.is_none() {
            return Err(Error::config(
                "the Oracle Client cannot use PEM files (ca_file, client_cert, client_key); \
                 set the wallet directory (an auto-login wallet, cwallet.sso) instead",
            ));
        }
        let verify_full = tls.mode == TlsMode::VerifyFull;
        security.push(format!(
            "(SSL_SERVER_DN_MATCH={})",
            if verify_full { "yes" } else { "no" }
        ));
        if verify_full
            && let Some(name) = tls.server_name.as_deref().map(str::trim)
            && !name.is_empty()
        {
            if !name.contains('=') {
                return Err(Error::config(
                    "with the Oracle Client the expected server name must be the \
                     certificate DN (for example CN=db.example.com), not a host name",
                ));
            }
            security.push(format!("(SSL_SERVER_CERT_DN={})", value_literal(name)?));
        }
    }
    if let Some(dir) = &tls.wallet_dir {
        let dir = dir
            .to_str()
            .ok_or_else(|| Error::config("the wallet directory must be valid UTF-8"))?;
        security.push(format!("(MY_WALLET_DIRECTORY={})", value_literal(dir)?));
    }

    let timeout = timeout_secs(p.connect_timeout);
    let simple = !is_sid && !tcps && security.is_empty() && timeout.is_none();
    if simple {
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        return Ok(format!("//{host}:{port}/{target}"));
    }

    let mut s = String::from("(DESCRIPTION=");
    if let Some(secs) = timeout {
        s.push_str(&format!(
            "(CONNECT_TIMEOUT={secs})(TRANSPORT_CONNECT_TIMEOUT={secs})"
        ));
    }
    s.push_str(&format!(
        "(ADDRESS=(PROTOCOL={})(HOST={host})(PORT={port}))",
        if tcps { "tcps" } else { "tcp" }
    ));
    s.push_str(&format!(
        "(CONNECT_DATA=({}={target}))",
        if is_sid { "SID" } else { "SERVICE_NAME" }
    ));
    if !security.is_empty() {
        s.push_str("(SECURITY=");
        for item in &security {
            s.push_str(item);
        }
        s.push(')');
    }
    s.push(')');
    Ok(s)
}

/// Whole seconds, at least 1; `None` when no timeout is wanted.
fn timeout_secs(d: Duration) -> Option<u64> {
    if d.is_zero() {
        return None;
    }
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    Some(secs.clamp(1, MAX_CONNECT_TIMEOUT_SECS))
}

/// Host names and IP literals only: a character that is not allowed here
/// could end the `(HOST=...)` entry and inject descriptor syntax.
fn host_literal(host: &str) -> Result<String> {
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() {
        return Err(Error::config("the host is empty"));
    }
    if host.len() > 255
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '%'))
    {
        return Err(Error::config(
            "the host may contain only letters, digits and . - _ : % characters",
        ));
    }
    Ok(host.to_owned())
}

/// Service names and SIDs.
fn name_literal(what: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::config(format!("the Oracle {what} is empty")));
    }
    if name.len() > 128
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '$' | '#'))
    {
        return Err(Error::config(format!(
            "the Oracle {what} may contain only letters, digits and . - _ $ # characters"
        )));
    }
    Ok(name.to_owned())
}

/// A free-form value (wallet directory, certificate DN). Quoted when it
/// holds anything beyond path-safe characters, so spaces and parentheses
/// (`C:\Program Files (x86)\...`) are fine. A double quote cannot be escaped
/// in a descriptor, so it is rejected, as are control characters.
fn value_literal(value: &str) -> Result<String> {
    if value.is_empty() {
        return Err(Error::config("an empty value in the connect descriptor"));
    }
    if value.chars().any(|c| c == '"' || c.is_control()) {
        return Err(Error::config(
            "a value in the connect descriptor must not contain double quotes or control characters",
        ));
    }
    let plain = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | ':' | '.' | '_' | '-'));
    Ok(if plain {
        value.to_owned()
    } else {
        format!("\"{value}\"")
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use db_core::{ErrorKind, Secret};

    use super::*;

    fn params() -> ConnectParams {
        ConnectParams {
            host: "db.example.com".into(),
            port: 1521,
            database: "FREEPDB1".into(),
            user: "scott".into(),
            password: Secret::new("tiger"),
            connect_timeout: Duration::ZERO,
            ..Default::default()
        }
    }

    fn cs(p: &ConnectParams) -> Result<String> {
        build_connect_string(p, p.user.is_empty() && p.password.is_empty())
    }

    #[test]
    fn plain_service_name_is_easy_connect() {
        assert_eq!(cs(&params()).unwrap(), "//db.example.com:1521/FREEPDB1");
    }

    #[test]
    fn default_port_and_ipv6_brackets() {
        let mut p = params();
        p.port = 0;
        p.host = "::1".into();
        assert_eq!(cs(&p).unwrap(), "//[::1]:1521/FREEPDB1");
        p.host = "[fe80::1]".into();
        assert_eq!(cs(&p).unwrap(), "//[fe80::1]:1521/FREEPDB1");
    }

    #[test]
    fn connect_timeout_switches_to_a_descriptor() {
        let mut p = params();
        p.connect_timeout = Duration::from_secs(10);
        assert_eq!(
            cs(&p).unwrap(),
            "(DESCRIPTION=(CONNECT_TIMEOUT=10)(TRANSPORT_CONNECT_TIMEOUT=10)\
             (ADDRESS=(PROTOCOL=tcp)(HOST=db.example.com)(PORT=1521))\
             (CONNECT_DATA=(SERVICE_NAME=FREEPDB1)))"
        );
        // Sub-second values round up; the value is capped.
        p.connect_timeout = Duration::from_millis(1500);
        assert!(cs(&p).unwrap().contains("(CONNECT_TIMEOUT=2)"));
        p.connect_timeout = Duration::from_millis(1);
        assert!(cs(&p).unwrap().contains("(CONNECT_TIMEOUT=1)"));
        p.connect_timeout = Duration::from_secs(1_000_000);
        assert!(cs(&p).unwrap().contains("(CONNECT_TIMEOUT=3600)"));
    }

    #[test]
    fn sid_uses_a_descriptor() {
        let mut p = params();
        p.database = "ORCL".into();
        p.options.insert(opt::ORACLE_SID.into(), "true".into());
        assert_eq!(
            cs(&p).unwrap(),
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=tcp)(HOST=db.example.com)(PORT=1521))\
             (CONNECT_DATA=(SID=ORCL)))"
        );
    }

    #[test]
    fn tls_verify_full_with_dn_and_wallet() {
        let mut p = params();
        p.tls.mode = TlsMode::VerifyFull;
        p.tls.server_name = Some("CN=db.example.com,O=Example".into());
        p.tls.wallet_dir = Some(PathBuf::from("/opt/wallet"));
        assert_eq!(
            cs(&p).unwrap(),
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=tcps)(HOST=db.example.com)(PORT=1521))\
             (CONNECT_DATA=(SERVICE_NAME=FREEPDB1))\
             (SECURITY=(SSL_SERVER_DN_MATCH=yes)\
             (SSL_SERVER_CERT_DN=\"CN=db.example.com,O=Example\")\
             (MY_WALLET_DIRECTORY=/opt/wallet)))"
        );
    }

    #[test]
    fn tls_require_and_verify_ca_skip_the_dn_match() {
        for mode in [TlsMode::Prefer, TlsMode::Require, TlsMode::VerifyCa] {
            let mut p = params();
            p.tls.mode = mode;
            p.tls.server_name = Some("CN=ignored".into());
            let s = cs(&p).unwrap();
            assert!(s.contains("(PROTOCOL=tcps)"), "{s}");
            assert!(s.contains("(SSL_SERVER_DN_MATCH=no)"), "{s}");
            assert!(!s.contains("SSL_SERVER_CERT_DN"), "{s}");
        }
    }

    #[test]
    fn verify_full_needs_a_dn_not_a_host_name() {
        let mut p = params();
        p.tls.mode = TlsMode::VerifyFull;
        p.tls.server_name = Some("db.example.com".into());
        assert_eq!(cs(&p).unwrap_err().kind, ErrorKind::Config);
        // Without server_name the client matches the connect host.
        p.tls.server_name = None;
        let s = cs(&p).unwrap();
        assert!(s.contains("(SSL_SERVER_DN_MATCH=yes)"));
        assert!(!s.contains("SSL_SERVER_CERT_DN"));
    }

    #[test]
    fn wallet_path_with_parentheses_is_quoted() {
        let mut p = params();
        p.tls.wallet_dir = Some(PathBuf::from(r"C:\Program Files (x86)\Oracle\wallet"));
        let s = cs(&p).unwrap();
        assert!(
            s.contains(
                r#"(SECURITY=(MY_WALLET_DIRECTORY="C:\Program Files (x86)\Oracle\wallet"))"#
            ),
            "{s}"
        );
        // A plain wallet directory over TCP (secure external password store).
        assert!(s.contains("(PROTOCOL=tcp)"));
    }

    #[test]
    fn pem_files_need_a_wallet() {
        let mut p = params();
        p.tls.mode = TlsMode::VerifyCa;
        p.tls.ca_file = Some(PathBuf::from("/etc/ca.pem"));
        let e = cs(&p).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Config);
        assert!(e.message.contains("wallet"));
        // With a wallet the PEM options and the wallet password are ignored.
        p.tls.wallet_dir = Some(PathBuf::from("/opt/wallet"));
        p.tls.wallet_password = Secret::new("walletpw");
        let s = cs(&p).unwrap();
        assert!(!s.contains("walletpw"));
        assert!(!s.contains("ca.pem"));
    }

    #[test]
    fn descriptor_injection_is_rejected() {
        for host in [
            "evil)(ADDRESS=(HOST=x",
            "a=b",
            "host name",
            "h\"ost",
            "h(ost)",
            "",
            "   ",
        ] {
            let mut p = params();
            p.host = host.into();
            assert_eq!(cs(&p).unwrap_err().kind, ErrorKind::Config, "{host:?}");
        }
        for service in [
            "svc)(SECURITY=(x=y",
            "a=b",
            "svc name",
            "svc\"",
            "svc/other",
            "",
        ] {
            let mut p = params();
            p.database = service.into();
            assert_eq!(cs(&p).unwrap_err().kind, ErrorKind::Config, "{service:?}");
        }
        let mut p = params();
        p.tls.mode = TlsMode::VerifyFull;
        p.tls.server_name = Some("CN=x\")(SECURITY=(".into());
        assert_eq!(cs(&p).unwrap_err().kind, ErrorKind::Config);
        let mut p = params();
        p.tls.wallet_dir = Some(PathBuf::from("/w\")(x="));
        assert_eq!(cs(&p).unwrap_err().kind, ErrorKind::Config);
    }

    #[test]
    fn dsn_is_used_verbatim() {
        let mut p = params();
        p.host = "ignored".into();
        p.options
            .insert(opt::ORACLE_DSN.into(), "  PROD_ALIAS  ".into());
        assert_eq!(cs(&p).unwrap(), "PROD_ALIAS");
        p.options.insert(
            opt::ORACLE_DSN.into(),
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=tcp)(HOST=h)(PORT=1)))".into(),
        );
        assert!(cs(&p).unwrap().starts_with("(DESCRIPTION="));
        p.options.insert(opt::ORACLE_DSN.into(), "a\nb".into());
        assert_eq!(cs(&p).unwrap_err().kind, ErrorKind::Config);
        // An empty DSN falls back to host, port and database.
        p.options.insert(opt::ORACLE_DSN.into(), "  ".into());
        p.host = "h".into();
        assert_eq!(cs(&p).unwrap(), "//h:1521/FREEPDB1");
    }

    #[test]
    fn password_never_reaches_the_connect_string() {
        let mut p = params();
        p.connect_timeout = Duration::from_secs(5);
        p.tls.mode = TlsMode::Require;
        let plan = plan(&p).unwrap();
        assert!(!plan.connect_string.contains("tiger"));
        assert!(!format!("{plan:?}").contains("tiger"));
    }

    #[test]
    fn external_auth_when_user_and_password_are_empty() {
        let mut p = params();
        p.user.clear();
        p.password = Secret::default();
        let plan = plan(&p).unwrap();
        assert!(plan.external_auth);
        assert_eq!(plan.connect_string, "//db.example.com:1521/FREEPDB1");

        // Local bequeath connection: no host, no database.
        p.host.clear();
        p.database.clear();
        let plan = super::plan(&p).unwrap();
        assert!(plan.external_auth);
        assert_eq!(plan.connect_string, "");

        // A user name without a password is a regular (failing) login.
        let mut q = params();
        q.password = Secret::default();
        assert!(!super::plan(&q).unwrap().external_auth);
        // Without external auth an empty host is an error.
        q.host.clear();
        assert_eq!(super::plan(&q).unwrap_err().kind, ErrorKind::Config);
    }

    #[test]
    fn client_directories_come_from_options() {
        let mut p = params();
        p.options
            .insert(OPT_LIB_DIR.into(), "  /opt/oracle/instantclient  ".into());
        p.options.insert(OPT_CONFIG_DIR.into(), "".into());
        let plan = plan(&p).unwrap();
        assert_eq!(
            plan.lib_dir,
            Some(PathBuf::from("/opt/oracle/instantclient"))
        );
        assert_eq!(plan.config_dir, None);
    }
}
