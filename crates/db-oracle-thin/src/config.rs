//! `ConnectParams` to an oracle-rs `Config`.
//!
//! TLS modes map to what oracle-rs 0.1.7 can do:
//! - `disable`: plain TCP.
//! - `prefer`: rejected (Oracle has no in-band TLS upgrade; the listener
//!   port decides between TCP and TCPS).
//! - `require`, `verify-ca`, `verify-full`: TCPS with full verification of
//!   the chain and the host name. oracle-rs has no switch to skip
//!   verification, so `require` and `verify-ca` are as strict as
//!   `verify-full`.
//! - Trust: the Mozilla roots bundled with oracle-rs (`webpki-roots`), not
//!   the OS store. `ca_file` *replaces* those roots; without `ca_file`, the
//!   certificates in `wallet_dir/ewallet.pem` replace them.
//! - `wallet_dir` (`ewallet.pem`, optionally with an encrypted key unlocked
//!   by `wallet_password`) also supplies the client certificate; or
//!   `client_cert` + `client_key` (PEM). `cwallet.sso` is not readable.
//! - `server_name`: the host name to verify and send as SNI. A certificate
//!   DN (`CN=...`) cannot be matched by oracle-rs and is rejected; a DN from
//!   a connect descriptor (`SSL_SERVER_CERT_DN`) is ignored and the host
//!   name is verified instead.

use std::path::Path;
use std::time::Duration;

use db_core::oracle_net::{self, OracleAddress};
use db_core::{ConnectParams, Error, ErrorKind, Result, TlsMode, opt};
use oracle_rs::{Config, TlsConfig};

/// Requested session data unit. oracle-rs reads one packet per call, so the
/// largest SDU the server accepts is the most useful.
pub const REQUESTED_SDU: u32 = 65_535;

/// Where and how to connect.
#[derive(Debug, Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub service: Option<String>,
    pub sid: Option<String>,
    pub tls: bool,
}

impl Target {
    /// `host:port/service` for messages and caches (never secrets).
    pub fn describe(&self) -> String {
        match (&self.service, &self.sid) {
            (Some(s), _) => format!("{}:{}/{}", self.host, self.port, s),
            (None, Some(s)) => format!("{}:{}:{}", self.host, self.port, s),
            _ => format!("{}:{}", self.host, self.port),
        }
    }
}

fn path_str(p: &Path, what: &str) -> Result<String> {
    p.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::config(format!("the {what} path is not valid UTF-8")))
}

/// Make sure rustls has a process-wide crypto provider. oracle-rs builds its
/// TLS configuration with `ClientConfig::builder()`, which panics when more
/// than one provider is compiled in and none is installed.
pub fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}

/// Resolve the target from the profile fields or `opt::ORACLE_DSN`.
pub fn target(p: &ConnectParams) -> Result<(Target, Option<OracleAddress>)> {
    if let Some(dsn) = p.option(opt::ORACLE_DSN).filter(|d| !d.trim().is_empty()) {
        let a = oracle_net::resolve(dsn)?;
        let tls = a.protocol_tcps || p.tls.mode.is_enabled();
        return Ok((
            Target {
                host: a.host.clone(),
                port: a.port,
                service: a.service.clone(),
                sid: a.sid.clone(),
                tls,
            },
            Some(a),
        ));
    }
    if p.host.trim().is_empty() {
        return Err(Error::config("host is required"));
    }
    if p.database.trim().is_empty() {
        return Err(Error::config(
            "database (Oracle service name, or SID with oracle.sid=true) is required",
        ));
    }
    let port = if p.port == 0 {
        oracle_net::DEFAULT_PORT
    } else {
        p.port
    };
    let (service, sid) = if p.flag(opt::ORACLE_SID) {
        (None, Some(p.database.trim().to_owned()))
    } else {
        (Some(p.database.trim().to_owned()), None)
    };
    Ok((
        Target {
            host: p.host.trim().to_owned(),
            port,
            service,
            sid,
            tls: p.tls.mode.is_enabled(),
        },
        None,
    ))
}

fn tls_config(p: &ConnectParams, from_dsn: Option<&OracleAddress>) -> Result<TlsConfig> {
    let mut t = TlsConfig::new();
    if let Some(ca) = &p.tls.ca_file {
        t = t.with_ca_cert(path_str(ca, "CA file")?);
    }
    match (&p.tls.client_cert, &p.tls.client_key) {
        (Some(c), Some(k)) => {
            t = t.with_client_cert(path_str(c, "client certificate")?, path_str(k, "client key")?);
        }
        (None, None) => {}
        _ => {
            return Err(Error::config(
                "client_cert and client_key must be given together",
            ));
        }
    }
    let wallet = match &p.tls.wallet_dir {
        Some(w) => Some(path_str(w, "wallet")?),
        None => from_dsn.and_then(|a| a.wallet_dir.clone()),
    };
    if let Some(w) = wallet {
        let pw = (!p.tls.wallet_password.is_empty())
            .then(|| p.tls.wallet_password.expose().to_owned());
        t = t.with_wallet(w, pw);
    }
    if let Some(name) = p.tls.server_name.as_deref().filter(|n| !n.trim().is_empty()) {
        if name.contains('=') {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "matching the server certificate DN is not supported by the thin driver; set server_name to the host name in the certificate or use the oracle-odpi backend",
            ));
        }
        t = t.with_server_name(name.trim());
    }
    Ok(t)
}

/// The oracle-rs configuration for `p` (password included; never log it,
/// its `Debug` output shows the password).
pub fn build(p: &ConnectParams) -> Result<(Config, Target)> {
    if p.user.trim().is_empty() {
        return Err(Error::config("user is required"));
    }
    if p.tls.mode == TlsMode::Prefer {
        return Err(Error::config(
            "TLS mode `prefer` is not available for Oracle: use `disable` for a TCP listener or `require`/`verify-full` for a TCPS listener",
        ));
    }
    let (target, from_dsn) = target(p)?;
    let password = p.password.expose();
    let mut cfg = match (&target.service, &target.sid) {
        (Some(s), _) => Config::new(&target.host, target.port, s, p.user.trim(), password),
        (None, Some(s)) => Config::with_sid(&target.host, target.port, s, p.user.trim(), password),
        (None, None) => return Err(Error::config("no service name or SID")),
    };
    cfg = cfg
        .sdu(REQUESTED_SDU)
        .stmtcachesize(0)
        .connect_timeout(if p.connect_timeout.is_zero() {
            Duration::from_secs(10)
        } else {
            p.connect_timeout
        });
    if target.tls {
        ensure_crypto_provider();
        let t = tls_config(p, from_dsn.as_ref())?;
        t.build_client_config().map_err(|e| {
            Error::new(ErrorKind::Tls, format!("TLS setup failed: {e}")).scrub(password)
        })?;
        cfg = cfg.tls_config(t);
    }
    Ok((cfg, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_core::Secret;

    fn params() -> ConnectParams {
        ConnectParams {
            host: "db".into(),
            port: 0,
            database: "FREEPDB1".into(),
            user: "scott".into(),
            password: Secret::new("tiger"),
            ..Default::default()
        }
    }

    #[test]
    fn builds_service_and_sid_targets() {
        let (cfg, t) = build(&params()).unwrap();
        assert_eq!((t.host.as_str(), t.port), ("db", 1521));
        assert_eq!(cfg.service.service_name(), Some("FREEPDB1"));
        assert!(!cfg.is_tls_enabled());
        assert_eq!(cfg.sdu, REQUESTED_SDU);

        let mut p = params();
        p.options.insert(opt::ORACLE_SID.into(), "true".into());
        let (cfg, t) = build(&p).unwrap();
        assert_eq!(cfg.service.sid(), Some("FREEPDB1"));
        assert_eq!(t.describe(), "db:1521:FREEPDB1");
    }

    #[test]
    fn dsn_overrides_fields() {
        let mut p = params();
        p.options.insert(
            opt::ORACLE_DSN.into(),
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=other)(PORT=1600))(CONNECT_DATA=(SID=XE)))".into(),
        );
        let (cfg, t) = build(&p).unwrap();
        assert_eq!((t.host.as_str(), t.port), ("other", 1600));
        assert_eq!(cfg.service.sid(), Some("XE"));
    }

    #[test]
    fn rejects_bad_params() {
        let mut p = params();
        p.user.clear();
        assert_eq!(build(&p).unwrap_err().kind, ErrorKind::Config);
        let mut p = params();
        p.database.clear();
        assert_eq!(build(&p).unwrap_err().kind, ErrorKind::Config);
        let mut p = params();
        p.tls.mode = TlsMode::Prefer;
        assert_eq!(build(&p).unwrap_err().kind, ErrorKind::Config);
        let mut p = params();
        p.tls.mode = TlsMode::VerifyFull;
        p.tls.server_name = Some("CN=db,O=Example".into());
        assert_eq!(build(&p).unwrap_err().kind, ErrorKind::Unsupported);
        let mut p = params();
        p.tls.mode = TlsMode::Require;
        p.tls.client_cert = Some("/nonexistent/c.pem".into());
        assert_eq!(build(&p).unwrap_err().kind, ErrorKind::Config);
    }

    #[test]
    fn tls_errors_never_show_the_password() {
        let mut p = params();
        p.password = Secret::new("wallet-secret-pw");
        p.tls.mode = TlsMode::VerifyFull;
        p.tls.ca_file = Some("/nonexistent/wallet-secret-pw/ca.pem".into());
        let e = build(&p).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Tls);
        assert!(!e.message.contains("wallet-secret-pw"), "{}", e.message);
    }

    #[test]
    fn tls_with_system_roots_builds() {
        let mut p = params();
        p.tls.mode = TlsMode::VerifyFull;
        p.tls.server_name = Some("db.example.com".into());
        let (cfg, t) = build(&p).unwrap();
        assert!(cfg.is_tls_enabled());
        assert!(t.tls);
    }
}
