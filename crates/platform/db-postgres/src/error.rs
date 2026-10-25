//! Mapping from tokio-postgres and SQLSTATE errors to [`db_core::Error`].

use std::error::Error as StdError;
use std::io;

use db_core::{Error, ErrorKind};

/// Where an error happened. A broken socket means "cannot reach the server"
/// while connecting and "the session is gone" afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Connect,
    Session,
}

/// Classify a SQLSTATE (appendix "PostgreSQL Error Codes").
pub(crate) fn kind_for_sqlstate(code: &str, message: &str) -> ErrorKind {
    match code {
        "57014" => {
            // query_canceled: a user request, or the server's own statement_timeout
            if message.to_ascii_lowercase().contains("timeout") {
                ErrorKind::Timeout
            } else {
                ErrorKind::Cancelled
            }
        }
        "57P01" | "57P02" | "57P03" => ErrorKind::ConnectionLost,
        "3D000" | "53300" | "08001" | "08004" => ErrorKind::Connect,
        "08P01" => ErrorKind::Internal,
        c if c.starts_with("28") => ErrorKind::Auth,
        c if c.starts_with("08") => ErrorKind::ConnectionLost,
        _ => ErrorKind::Query,
    }
}

/// Convert a tokio-postgres error. The message never contains the password;
/// the caller scrubs it defensively as well.
pub(crate) fn map_error(e: &tokio_postgres::Error, phase: Phase) -> Error {
    if let Some(db) = e.as_db_error() {
        let code = db.code().code();
        let mut kind = kind_for_sqlstate(code, db.message());
        if phase == Phase::Connect && kind == ErrorKind::ConnectionLost {
            kind = ErrorKind::Connect;
        }
        let mut message = db.message().to_owned();
        if let Some(detail) = db.detail() {
            message.push_str("\nDETAIL: ");
            message.push_str(detail);
        }
        if let Some(hint) = db.hint() {
            message.push_str("\nHINT: ");
            message.push_str(hint);
        }
        let mut error = Error::new(kind, message).with_code(code);
        // A 1-based character index into the statement the client sent.
        if let Some(tokio_postgres::error::ErrorPosition::Original(at)) = db.position() {
            error = error.with_position((*at as usize).saturating_sub(1));
        }
        return error;
    }

    let message = chain_message(e);
    let top = e.to_string();
    let kind = if top.starts_with("error performing TLS handshake") || has_tls_cause(e) {
        ErrorKind::Tls
    } else if top.starts_with("authentication error") || message.contains("password missing") {
        ErrorKind::Auth
    } else if top.starts_with("invalid configuration")
        || top.starts_with("invalid connection string")
    {
        ErrorKind::Config
    } else if e.is_closed() {
        ErrorKind::ConnectionLost
    } else if top.starts_with("timeout waiting for server") {
        ErrorKind::Timeout
    } else if top.starts_with("error connecting to server") {
        ErrorKind::Network
    } else if top.starts_with("error communicating with the server") {
        match find_io(e) {
            Some(kind) if phase == Phase::Session && is_disconnect(kind) => {
                ErrorKind::ConnectionLost
            }
            _ => match phase {
                Phase::Connect => ErrorKind::Network,
                Phase::Session => ErrorKind::ConnectionLost,
            },
        }
    } else if top.starts_with("error serializing parameter")
        || top.starts_with("expected ")
        || top.starts_with("invalid column")
    {
        ErrorKind::Query
    } else {
        ErrorKind::Internal
    };
    Error::new(kind, message)
}

/// `top: cause: cause ...`, since tokio-postgres only prints its own kind.
fn chain_message(e: &tokio_postgres::Error) -> String {
    let mut message = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.ends_with(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

fn has_tls_cause(e: &(dyn StdError + 'static)) -> bool {
    let mut source = e.source();
    while let Some(cause) = source {
        if cause.downcast_ref::<rustls::Error>().is_some() {
            return true;
        }
        if let Some(io) = cause.downcast_ref::<io::Error>()
            && io
                .get_ref()
                .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
        {
            return true;
        }
        source = cause.source();
    }
    false
}

fn find_io(e: &(dyn StdError + 'static)) -> Option<io::ErrorKind> {
    let mut source = e.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<io::Error>() {
            return Some(io.kind());
        }
        source = cause.source();
    }
    None
}

fn is_disconnect(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::NotConnected
    )
}

/// Map an error and remove a known secret from its message.
pub(crate) fn map_scrubbed(e: &tokio_postgres::Error, phase: Phase, secret: &str) -> Error {
    map_error(e, phase).scrub(secret)
}

/// The error for a connection that was closed on purpose.
pub(crate) fn closed() -> Error {
    Error::connection_lost("the connection is closed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlstate_classes() {
        assert_eq!(kind_for_sqlstate("28P01", ""), ErrorKind::Auth);
        assert_eq!(kind_for_sqlstate("28000", ""), ErrorKind::Auth);
        assert_eq!(kind_for_sqlstate("3D000", ""), ErrorKind::Connect);
        assert_eq!(kind_for_sqlstate("53300", ""), ErrorKind::Connect);
        assert_eq!(kind_for_sqlstate("57P01", ""), ErrorKind::ConnectionLost);
        assert_eq!(kind_for_sqlstate("57P02", ""), ErrorKind::ConnectionLost);
        assert_eq!(kind_for_sqlstate("57P03", ""), ErrorKind::ConnectionLost);
        assert_eq!(kind_for_sqlstate("08001", ""), ErrorKind::Connect);
        assert_eq!(kind_for_sqlstate("08004", ""), ErrorKind::Connect);
        assert_eq!(kind_for_sqlstate("08006", ""), ErrorKind::ConnectionLost);
        assert_eq!(kind_for_sqlstate("08003", ""), ErrorKind::ConnectionLost);
        assert_eq!(kind_for_sqlstate("08P01", ""), ErrorKind::Internal);
        assert_eq!(kind_for_sqlstate("42601", ""), ErrorKind::Query);
        assert_eq!(kind_for_sqlstate("23505", ""), ErrorKind::Query);
        assert_eq!(kind_for_sqlstate("25006", ""), ErrorKind::Query);
        assert_eq!(kind_for_sqlstate("XX000", ""), ErrorKind::Query);
    }

    #[test]
    fn cancel_versus_server_side_timeout() {
        assert_eq!(
            kind_for_sqlstate("57014", "canceling statement due to user request"),
            ErrorKind::Cancelled
        );
        assert_eq!(
            kind_for_sqlstate("57014", "canceling statement due to statement timeout"),
            ErrorKind::Timeout
        );
    }

    #[test]
    fn closed_connection_error() {
        let e = closed();
        assert_eq!(e.kind, ErrorKind::ConnectionLost);
        assert!(!e.message.is_empty());
    }

    #[test]
    fn io_disconnect_kinds() {
        assert!(is_disconnect(io::ErrorKind::BrokenPipe));
        assert!(is_disconnect(io::ErrorKind::UnexpectedEof));
        assert!(!is_disconnect(io::ErrorKind::ConnectionRefused));
        assert!(!is_disconnect(io::ErrorKind::TimedOut));
    }
}
