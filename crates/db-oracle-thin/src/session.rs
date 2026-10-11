//! One oracle-rs session plus the state the driver keeps about it, and the
//! mapping of oracle-rs errors to db-core errors.

use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use db_core::oracle::error_from_message;
use db_core::{Error, ErrorKind, Result};
use oracle_rs::Error as OraError;

/// How large one server response may be. oracle-rs 0.1.7 reads a single
/// network packet per call, so every fetch must fit into one packet of the
/// negotiated session data unit (SDU).
pub const LARGE_FETCH_BUDGET: usize = 58 * 1024;
/// Budget when the server negotiated a small (8 KiB) SDU.
pub const SMALL_FETCH_BUDGET: usize = 6 * 1024;
/// oracle-rs also sends every request in one packet: the largest request
/// (statement text, wrapper and inline values) for each case.
pub const LARGE_REQUEST: usize = 65_000;
pub const SMALL_REQUEST: usize = 7_800;

/// An open oracle-rs connection and what the driver tracks about it.
pub struct Session {
    pub conn: oracle_rs::Connection,
    broken: AtomicBool,
    reason: Mutex<Option<String>>,
    /// Server cursors opened on this session. oracle-rs 0.1.7 never closes
    /// them; the session is renewed before `OPEN_CURSORS` is reached.
    cursors: AtomicU32,
    /// `OPEN_CURSORS` of the instance.
    pub cursor_limit: u32,
    /// Version banner from the connect probe.
    pub banner: String,
    /// Whether the wrapper blocks can use DBMS_SQL.
    pub dbms_sql: bool,
    /// Bytes per fetch round trip (see [`LARGE_FETCH_BUDGET`]).
    pub fetch_budget: usize,
    /// Largest request in bytes (see [`LARGE_REQUEST`]).
    pub max_request: usize,
}

impl Session {
    pub fn new(conn: oracle_rs::Connection) -> Self {
        Self {
            conn,
            broken: AtomicBool::new(false),
            reason: Mutex::new(None),
            cursors: AtomicU32::new(0),
            cursor_limit: 50,
            banner: String::new(),
            dbms_sql: true,
            fetch_budget: SMALL_FETCH_BUDGET,
            max_request: SMALL_REQUEST,
        }
    }

    /// Set the fetch budget and the matching request limit.
    pub fn set_budget(&mut self, fetch_budget: usize) {
        self.fetch_budget = fetch_budget;
        self.max_request = if fetch_budget >= LARGE_FETCH_BUDGET {
            LARGE_REQUEST
        } else {
            SMALL_REQUEST
        };
    }

    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    /// Fail with `ConnectionLost` once the session is unusable.
    pub fn check(&self) -> Result<()> {
        if self.is_broken() {
            let reason = self
                .reason
                .lock()
                .ok()
                .and_then(|r| r.clone())
                .unwrap_or_else(|| "the connection is closed".into());
            return Err(Error::connection_lost(reason));
        }
        Ok(())
    }

    /// Mark the session unusable; later calls fail with `ConnectionLost`.
    pub fn mark_broken(&self, reason: &str) {
        if !self.broken.swap(true, Ordering::AcqRel)
            && let Ok(mut r) = self.reason.lock()
        {
            *r = Some(format!(
                "the connection is no longer usable: {reason}; reconnect"
            ));
        }
        self.conn.mark_closed();
    }

    /// Refuse a request that does not fit into one network packet.
    pub fn check_request(&self, text: &str) -> Result<()> {
        if text.len() > self.max_request {
            return Err(Error::unsupported(format!(
                "the statement with its bind values needs a {} KiB request, but the thin driver can send at most {} KiB (oracle-rs 0.1.7 sends each request in one network packet of the negotiated size); shorten it or use the oracle-odpi backend",
                text.len().div_ceil(1024),
                self.max_request / 1024
            )));
        }
        Ok(())
    }

    pub fn add_cursors(&self, n: u32) {
        self.cursors.fetch_add(n, Ordering::AcqRel);
    }

    pub fn cursors(&self) -> u32 {
        self.cursors.load(Ordering::Acquire)
    }

    /// Run one oracle-rs call. If the future is dropped before it finishes
    /// (timeout, cancelled task) or fails in a way that leaves the protocol
    /// state unknown, the session is marked broken.
    pub async fn call<T>(&self, fut: impl Future<Output = oracle_rs::Result<T>>) -> Result<T> {
        self.check()?;
        let guard = CallGuard {
            session: self,
            armed: true,
        };
        let result = fut.await;
        let mut guard = guard;
        guard.armed = false;
        result.map_err(|e| {
            let (err, broken) = map_call_error(e);
            if broken {
                self.mark_broken(&err.message);
            }
            err
        })
    }
}

struct CallGuard<'a> {
    session: &'a Session,
    armed: bool,
}

impl Drop for CallGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.mark_broken(
                "a call was abandoned before the server answered (timeout or cancellation)",
            );
        }
    }
}

const DESYNC_HINT: &str = "the session was closed to keep the protocol consistent. oracle-rs 0.1.7 reads one network packet per call, so a page of rows larger than the negotiated packet size cannot be read; use smaller pages or the oracle-odpi backend";

/// Map an error of a call on an open session. The flag says whether the
/// session must be considered broken.
pub fn map_call_error(e: OraError) -> (Error, bool) {
    match e {
        OraError::OracleError { code: 0, message } => (
            Error::connection_lost(format!(
                "the server rejected the call and closed the session ({message})"
            )),
            true,
        ),
        OraError::OracleError { code, message } => {
            let err = error_from_message(format!("ORA-{code:05}: {message}"), ErrorKind::Query);
            let broken = err.kind == ErrorKind::ConnectionLost;
            (err, broken)
        }
        OraError::NoDataFound => (
            Error::query("ORA-01403: no data found").with_code("ORA-01403"),
            false,
        ),
        OraError::ConnectionClosedByServer(_) => (
            Error::connection_lost(
                "the server reported an error and closed the session; oracle-rs 0.1.7 cannot read error details at this point (typically a run-time error while fetching rows, such as ORA-01722 or ORA-01476)",
            ),
            true,
        ),
        OraError::ConnectionClosed | OraError::ConnectionNotReady => {
            (Error::connection_lost("the connection is closed"), true)
        }
        OraError::Io(io) => (Error::connection_lost(format!("network error: {io}")), true),
        OraError::ConnectionTimeout(d) => (
            Error::connection_lost(format!("network timeout after {d:?}")),
            true,
        ),
        OraError::DataConversionError(m) => (
            Error::new(
                ErrorKind::Unsupported,
                format!("oracle-rs could not decode a value ({m}); {DESYNC_HINT}"),
            ),
            true,
        ),
        OraError::FeatureNotSupported(m) => (
            Error::unsupported(format!("not supported by oracle-rs: {m}")),
            false,
        ),
        other => (
            Error::internal(format!(
                "oracle-rs could not decode the server response ({other}); {DESYNC_HINT}"
            )),
            true,
        ),
    }
}

fn looks_like_tls(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "certificate",
        "tls",
        "handshake",
        "alert",
        "unknownissuer",
        "invalid peer",
        "notvalidforname",
        "badsignature",
        "ssl",
    ]
    .iter()
    .any(|k| m.contains(k))
}

/// Map an error raised while connecting.
pub fn map_connect_error(e: OraError, tls: bool) -> Error {
    match e {
        OraError::Io(io) => {
            let text = io.to_string();
            if tls && looks_like_tls(&text) {
                Error::new(ErrorKind::Tls, format!("TLS handshake failed: {text}"))
            } else {
                Error::new(ErrorKind::Network, format!("cannot reach the listener: {text}"))
            }
        }
        OraError::ConnectionTimeout(d) => Error::new(
            ErrorKind::Network,
            format!("connect timed out after {d:?}"),
        ),
        OraError::InvalidCredentials => Error::new(
            ErrorKind::Auth,
            "ORA-01017: invalid username/password; logon denied",
        )
        .with_code("ORA-01017"),
        // A rejection after the password was sent: oracle-rs does not read
        // the server's error here; Oracle answers ORA-01017 for a wrong
        // user name or password.
        OraError::AuthenticationFailed(m) if m.contains("MARKER") => Error::new(
            ErrorKind::Auth,
            "ORA-01017: invalid username/password; logon denied (the thin driver cannot read the server's exact reason; a locked or expired account is also possible)",
        )
        .with_code("ORA-01017"),
        OraError::AuthenticationFailed(m) => {
            Error::new(ErrorKind::Auth, format!("authentication failed: {m}"))
        }
        OraError::UnsupportedVerifierType(v) => Error::new(
            ErrorKind::Unsupported,
            format!(
                "the account's password verifier type {v:#x} is not supported by oracle-rs (it supports the 11g SHA-1 and 12c PBKDF2 verifiers); use the oracle-odpi backend or reset the password"
            ),
        ),
        OraError::NativeNetworkEncryptionRequired => Error::new(
            ErrorKind::Connect,
            "ORA-12660: the server requires Oracle native network encryption, which the thin driver does not support; use TLS (TCPS) or the oracle-odpi backend",
        )
        .with_code("ORA-12660"),
        OraError::ConnectionRedirect(_) | OraError::ConnectionRedirected { .. } => Error::new(
            ErrorKind::Connect,
            "the listener redirected the connection (RAC SCAN listener, shared server or Connection Manager); oracle-rs 0.1.7 cannot follow redirects. Connect to a node listener or use the oracle-odpi backend",
        ),
        OraError::ConnectionRefused { error_code, message } => {
            let text = message.unwrap_or_else(|| "the listener refused the connection".into());
            match error_code {
                Some(code) => error_from_message(format!("ORA-{code:05}: {text}"), ErrorKind::Connect),
                None => Error::new(ErrorKind::Connect, text),
            }
        }
        OraError::InvalidServiceName { service_name, .. } => Error::new(
            ErrorKind::Connect,
            format!(
                "ORA-12514: the listener does not know service `{}`",
                service_name.unwrap_or_default()
            ),
        )
        .with_code("ORA-12514"),
        OraError::InvalidSid { sid, .. } => Error::new(
            ErrorKind::Connect,
            format!("ORA-12505: the listener does not know SID `{}`", sid.unwrap_or_default()),
        )
        .with_code("ORA-12505"),
        OraError::OracleError { code, message } => {
            error_from_message(format!("ORA-{code:05}: {message}"), ErrorKind::Connect)
        }
        OraError::Internal(m) if tls => Error::new(ErrorKind::Tls, format!("TLS setup failed: {m}")),
        OraError::FeatureNotSupported(m) => {
            Error::unsupported(format!("not supported by oracle-rs: {m}"))
        }
        OraError::InvalidConnectionString(m) => Error::config(m),
        OraError::ConnectionClosedByServer(m) => Error::new(
            ErrorKind::Connect,
            format!("the server closed the connection during logon: {m}"),
        ),
        OraError::ConnectionClosed => Error::new(
            ErrorKind::Connect,
            "the server closed the connection during logon",
        ),
        other => Error::new(
            ErrorKind::Connect,
            format!("protocol error while connecting: {other}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_call_errors() {
        let (e, broken) = map_call_error(OraError::OracleError {
            code: 942,
            message: "table or view does not exist".into(),
        });
        assert_eq!(
            (e.kind, e.code.as_deref(), broken),
            (ErrorKind::Query, Some("ORA-00942"), false)
        );
        let (e, broken) = map_call_error(OraError::OracleError {
            code: 3113,
            message: "end-of-file on communication channel".into(),
        });
        assert_eq!((e.kind, broken), (ErrorKind::ConnectionLost, true));
        let (e, broken) = map_call_error(OraError::BufferUnderflow {
            needed: 4,
            available: 1,
        });
        assert_eq!((e.kind, broken), (ErrorKind::Internal, true));
        let (e, broken) = map_call_error(OraError::ConnectionClosedByServer("x".into()));
        assert_eq!((e.kind, broken), (ErrorKind::ConnectionLost, true));
        let (e, _) = map_call_error(OraError::OracleError {
            code: 0,
            message: "x".into(),
        });
        assert_eq!(e.kind, ErrorKind::ConnectionLost);
    }

    #[test]
    fn maps_connect_errors() {
        let refused = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert_eq!(
            map_connect_error(OraError::Io(refused), false).kind,
            ErrorKind::Network
        );
        let tls = std::io::Error::other("invalid peer certificate: UnknownIssuer");
        assert_eq!(
            map_connect_error(OraError::Io(tls), true).kind,
            ErrorKind::Tls
        );
        let e = map_connect_error(OraError::InvalidCredentials, false);
        assert_eq!(
            (e.kind, e.code.as_deref()),
            (ErrorKind::Auth, Some("ORA-01017"))
        );
        let e = map_connect_error(
            OraError::ConnectionRefused {
                error_code: Some(12514),
                message: Some("no service".into()),
            },
            false,
        );
        assert_eq!(
            (e.kind, e.code.as_deref()),
            (ErrorKind::Connect, Some("ORA-12514"))
        );
        let e = map_connect_error(OraError::NativeNetworkEncryptionRequired, false);
        assert_eq!(e.code.as_deref(), Some("ORA-12660"));
        let e = map_connect_error(OraError::Internal("bad wallet".into()), true);
        assert_eq!(e.kind, ErrorKind::Tls);
    }
}
