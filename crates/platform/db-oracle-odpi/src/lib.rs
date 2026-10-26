//! Oracle backend on top of an installed Oracle Client (ODPI-C).
//!
//! The fallback for environments the pure-Rust thin driver cannot reach:
//! native network encryption, Kerberos and OS authentication, LDAP and TNS
//! naming, wallets, old password verifiers. The Oracle Client libraries are
//! loaded at runtime, so this crate builds and its tests run without them;
//! see the README for installing Instant Client.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use db_core::worker::Worker;
use db_core::{Backend, ConnectParams, Connection, Driver, Error, ErrorKind, Result, Secret};

mod binds;
mod conn;
mod connect_string;
mod errors;
mod session;
mod sqltext;
#[cfg(test)]
mod testing;
mod tx;
mod values;

pub use conn::OdpiConnection;
pub use connect_string::{OPT_CONFIG_DIR, OPT_LIB_DIR};

use conn::worker_config;
use connect_string::ConnectPlan;
use session::{OracleBreaker, OracleSession};

/// Time allowed beyond `connect_timeout` before a connect that has not
/// returned is abandoned (the client enforces the timeout itself when the
/// connect string is built here; TNS aliases rely on `sqlnet.ora`).
const CONNECT_GRACE: Duration = Duration::from_secs(5);

/// Opens connections through the Oracle Client.
#[derive(Debug, Default, Clone, Copy)]
pub struct OdpiDriver;

#[async_trait]
impl Driver for OdpiDriver {
    fn backend(&self) -> Backend {
        Backend::OracleOdpi
    }

    /// The loaded Oracle Client version, or `ErrorKind::Unavailable` with the
    /// reason (`DPI-1047`: no Oracle Client libraries were found).
    fn availability(&self) -> Result<String> {
        init_client(None, None)
    }

    async fn connect(&self, params: &ConnectParams) -> Result<Box<dyn Connection>> {
        let plan = connect_string::plan(params)?;
        let password = params.password.clone();
        let limit = plan.connect_timeout;
        // The client library blocks while it resolves names and connects: not
        // on an async worker.
        let task = tokio::task::spawn_blocking(move || open(plan, password));
        let joined = if limit.is_zero() {
            task.await
        } else {
            match tokio::time::timeout(limit + CONNECT_GRACE, task).await {
                Ok(joined) => joined,
                Err(_) => {
                    return Err(Error::new(
                        ErrorKind::Network,
                        format!(
                            "the database did not answer within {} s",
                            limit.as_secs().max(1)
                        ),
                    ));
                }
            }
        };
        let (session, breaker, call_timeout_supported) =
            joined.map_err(|e| Error::internal(format!("connect task failed: {e}")))??;
        let worker = Worker::start(
            worker_config(call_timeout_supported),
            Arc::new(breaker),
            move || Ok(session),
        )
        .await?;
        Ok(Box::new(OdpiConnection::new(worker)))
    }
}

/// Load the Oracle Client (once per process) and return its version.
///
/// `lib_dir` is the directory of the Oracle Client libraries (Instant Client);
/// without it they are searched next to the executable and in the system
/// library path (`PATH`, `LD_LIBRARY_PATH`, `DYLD_LIBRARY_PATH`).
/// `config_dir` holds `tnsnames.ora`, `sqlnet.ora` and wallets; it overrides
/// `TNS_ADMIN`. Both only take effect when this is the first call that loads
/// the client in the process; later calls return the version of the client
/// that is already loaded. `connect` calls this with the profile's
/// `oracle.lib_dir` and `oracle.config_dir` options.
pub fn init_client(lib_dir: Option<&Path>, config_dir: Option<&Path>) -> Result<String> {
    let mut params = oracle::InitParams::new();
    // Rejected arguments (a path with a NUL, or one that is not representable
    // in the Windows ANSI code page) are configuration mistakes.
    let invalid = |what: &str, e: oracle::Error| {
        Error::config(format!(
            "invalid Oracle Client {what}: {}",
            errors::message_of(&e)
        ))
    };
    // Shown in V$SESSION_CONNECT_INFO.CLIENT_DRIVER.
    params
        .default_driver_name(format!(
            "{} : {}",
            app_info::slug(),
            env!("CARGO_PKG_VERSION")
        ))
        .map_err(|e| invalid("driver name", e))?;
    if let Some(dir) = lib_dir {
        params
            .oracle_client_lib_dir(dir.as_os_str())
            .map_err(|e| invalid("library directory", e))?;
    }
    if let Some(dir) = config_dir {
        params
            .oracle_client_config_dir(dir.as_os_str())
            .map_err(|e| invalid("configuration directory", e))?;
    }
    params.init().map_err(|e| errors::map_error(&e))?;
    oracle::Version::client()
        .map(|v| format!("Oracle Client {v}"))
        .map_err(|e| errors::map_error(&e))
}

/// Blocking: load the client and connect. Returns the session, its breaker
/// and whether the client supports call timeouts.
fn open(plan: ConnectPlan, password: Secret) -> Result<(OracleSession, OracleBreaker, bool)> {
    init_client(plan.lib_dir.as_deref(), plan.config_dir.as_deref())?;
    let mut connector = oracle::Connector::new(
        plan.username.as_str(),
        password.expose(),
        plan.connect_string.as_str(),
    );
    if plan.external_auth {
        // OS, Kerberos or wallet credentials: no user name, no password.
        connector.external_auth(true);
    }
    let conn = connector
        .connect()
        .map_err(|e| errors::map_connect_error(&e, password.expose()))?;
    // Best effort: helps a DBA recognise the session in V$SESSION.
    let _ = conn.set_module(app_info::full_name());
    // Call timeouts need a client of 18c or later; with an older one the
    // worker's timer breaks the call itself.
    let call_timeout_supported = conn.set_call_timeout(None).is_ok();
    let conn = Arc::new(conn);
    let session = OracleSession::new(conn.clone(), plan.read_only, call_timeout_supported);
    Ok((session, OracleBreaker(conn), call_timeout_supported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_never_panics_without_a_client() {
        // On a machine without Oracle Client libraries this is Unavailable
        // (DPI-1047); with them it is the client version. Either way it
        // returns, repeatedly.
        for _ in 0..2 {
            match OdpiDriver.availability() {
                Ok(v) => assert!(v.starts_with("Oracle Client "), "{v}"),
                Err(e) => {
                    assert_eq!(e.kind, ErrorKind::Unavailable, "{e:?}");
                    assert_eq!(e.code.as_deref(), Some("DPI-1047"));
                }
            }
        }
    }

    #[tokio::test]
    async fn connect_without_a_client_or_a_listener_fails_cleanly() {
        // Nothing listens on port 1: with a client the connect is refused
        // (ORA-12541), without one it is Unavailable. Never a hang or a panic,
        // and never the password.
        let params = ConnectParams {
            host: "127.0.0.1".into(),
            port: 1,
            database: "FREEPDB1".into(),
            user: "scott".into(),
            password: Secret::new("tiger-secret"),
            connect_timeout: Duration::from_secs(2),
            ..Default::default()
        };
        let e = match OdpiDriver.connect(&params).await {
            Ok(_) => panic!("nothing listens on port 1"),
            Err(e) => e,
        };
        assert!(
            matches!(
                e.kind,
                ErrorKind::Unavailable | ErrorKind::Network | ErrorKind::Connect
            ),
            "{e:?}"
        );
        assert!(!e.message.contains("tiger-secret"));
    }

    #[tokio::test]
    async fn invalid_parameters_are_config_errors_before_any_client_is_loaded() {
        let params = ConnectParams {
            host: "h)(x=y".into(),
            database: "svc".into(),
            user: "u".into(),
            password: Secret::new("p"),
            ..Default::default()
        };
        let e = match OdpiDriver.connect(&params).await {
            Ok(_) => panic!("must be rejected"),
            Err(e) => e,
        };
        assert_eq!(e.kind, ErrorKind::Config);
    }

    #[test]
    fn unusable_client_directories_are_config_errors() {
        // A NUL cannot be passed to the client library; nothing is loaded.
        let e = init_client(Some(Path::new("/opt/oracle\0/lib")), None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Config, "{e:?}");
        let e = init_client(None, Some(Path::new("/etc/\0/net"))).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Config, "{e:?}");
    }

    #[test]
    fn driver_identity() {
        assert_eq!(OdpiDriver.backend(), Backend::OracleOdpi);
    }
}
