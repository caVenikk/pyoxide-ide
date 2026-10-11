use serde::{Deserialize, Serialize};

/// What went wrong, independent of the backend. Diagnostics and the UI branch
/// on this, never on message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Invalid connection parameters or profile.
    Config,
    /// Name resolution, TCP connect, socket I/O.
    Network,
    /// TLS handshake, certificates, wallets.
    Tls,
    /// The server rejected the credentials or the account.
    Auth,
    /// The server refused the connection for another reason (unknown
    /// service or database, listener problems, protocol mismatch).
    Connect,
    /// The server rejected a statement (syntax, permissions, constraints).
    Query,
    /// A per-call timeout elapsed.
    Timeout,
    /// The statement was cancelled on request.
    Cancelled,
    /// The connection has an open cursor; close it or use another connection.
    Busy,
    /// The connection is unusable; reconnect.
    ConnectionLost,
    /// The backend does not support this operation.
    Unsupported,
    /// A client-side component is missing (e.g. Oracle Client libraries).
    Unavailable,
    /// A bug or an unexpected response.
    Internal,
}

/// Error surfaced by any backend.
///
/// `message` must never contain secrets. `code` is the backend's own error
/// code when there is one (`ORA-01017`, SQLSTATE `28P01`, `SQLITE_BUSY`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub code: Option<String>,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: None,
            message: message.into(),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Config, message)
    }

    pub fn query(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Query, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    pub fn busy() -> Self {
        Self::new(
            ErrorKind::Busy,
            "a cursor is open on this connection; close it or use another connection",
        )
    }

    pub fn timeout() -> Self {
        Self::new(ErrorKind::Timeout, "the statement timed out")
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "the statement was cancelled")
    }

    pub fn connection_lost(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ConnectionLost, message)
    }

    /// Replace every occurrence of `secret` in the message. Drivers call this
    /// on messages that might echo connection strings.
    pub fn scrub(mut self, secret: &str) -> Self {
        if !secret.is_empty() && self.message.contains(secret) {
            self.message = self.message.replace(secret, "<redacted>");
        }
        self
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_message_only() {
        let e = Error::new(ErrorKind::Auth, "invalid username/password").with_code("ORA-01017");
        assert_eq!(e.to_string(), "invalid username/password");
        assert_eq!(e.code.as_deref(), Some("ORA-01017"));
    }

    #[test]
    fn scrub_removes_secret() {
        let e = Error::query("login user/hunter2@db failed").scrub("hunter2");
        assert!(!e.message.contains("hunter2"));
    }
}
