use db_core::oracle::{classify, error_from_message};
use db_core::{Error, ErrorKind};

/// The text of an `oracle` crate error without its "OCI Error:" wrapper, so
/// that it starts with the Oracle code (`ORA-01017: ...`, `DPI-1047: ...`).
pub(crate) fn message_of(e: &oracle::Error) -> String {
    let text = match e.db_error() {
        Some(db) => db.message().to_owned(),
        None => e.to_string(),
    };
    text.trim().to_owned()
}

/// Leading `ORA-`/`TNS-`/`DPI-` code of a message, if the message starts
/// with one. ODPI-C wraps some errors (`DPI-1067: call timeout of 1000 ms
/// exceeded with ORA-03136`); the leading code names the real condition, the
/// embedded one only the low-level cause.
fn leading_code(message: &str) -> Option<String> {
    let m = message.trim_start();
    let (prefix, is_dpi) = [("ORA-", false), ("TNS-", false), ("DPI-", true)]
        .into_iter()
        .find(|(p, _)| m.starts_with(p))?;
    let digits: String = m[prefix.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    // The same normal form as `db_core::oracle::error_code`.
    Some(if is_dpi {
        format!("DPI-{digits}")
    } else {
        format!("ORA-{:05}", digits.parse::<u32>().ok()?)
    })
}

/// Build an error from an Oracle message, trusting a leading code.
pub(crate) fn from_message(message: impl Into<String>, fallback: ErrorKind) -> Error {
    let message = message.into();
    match leading_code(&message) {
        Some(code) => Error::new(classify(&code), message).with_code(code),
        None => error_from_message(message, fallback),
    }
}

fn fallback_kind(kind: oracle::ErrorKind) -> ErrorKind {
    match kind {
        oracle::ErrorKind::OciError | oracle::ErrorKind::DpiError => ErrorKind::Query,
        // Bind values and conversions the caller got wrong.
        oracle::ErrorKind::NullValue
        | oracle::ErrorKind::ParseError
        | oracle::ErrorKind::OutOfRange
        | oracle::ErrorKind::InvalidArgument
        | oracle::ErrorKind::InvalidTypeConversion
        | oracle::ErrorKind::InvalidBindIndex
        | oracle::ErrorKind::InvalidBindName
        | oracle::ErrorKind::InvalidOperation
        | oracle::ErrorKind::NoDataFound
        | oracle::ErrorKind::BatchErrors => ErrorKind::Query,
        _ => ErrorKind::Internal,
    }
}

fn is_unsupported_column_type(message: &str) -> bool {
    message.contains("unsupported Oracle type") || message.contains("unknown Oracle type")
}

/// Map an error of a statement or any other call on a live session.
pub(crate) fn map_error(e: &oracle::Error) -> Error {
    let message = message_of(e);
    // The `oracle` crate cannot fetch some column types (JSON, VECTOR).
    if is_unsupported_column_type(&message) {
        return Error::unsupported(format!(
            "{message}; select the column through a conversion such as \
             JSON_SERIALIZE(column) or TO_CLOB(column)"
        ));
    }
    from_message(message, fallback_kind(e.kind()))
}

/// Map an error raised while opening a session. Anything the shared
/// classification does not know (a statement error code such as ORA-12631 for
/// a failed Kerberos handshake) is still a connection problem. The password is
/// removed from the message in case a client library echoes the connect string.
pub(crate) fn map_connect_error(e: &oracle::Error, password: &str) -> Error {
    connect_error(message_of(e), password)
}

pub(crate) fn connect_error(message: String, password: &str) -> Error {
    let mut err = from_message(message, ErrorKind::Connect);
    if err.kind == ErrorKind::Query || err.kind == ErrorKind::Internal {
        err.kind = ErrorKind::Connect;
    }
    err.scrub(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_library_missing_is_unavailable() {
        let e = from_message(
            "DPI-1047: Cannot locate a 64-bit Oracle Client library: \"libclntsh.dylib: cannot open\". See https://oracle.github.io/odpi/doc/installation.html",
            ErrorKind::Connect,
        );
        assert_eq!(e.kind, ErrorKind::Unavailable);
        assert_eq!(e.code.as_deref(), Some("DPI-1047"));
    }

    #[test]
    fn call_timeout_wins_over_the_embedded_ora_code() {
        let e = from_message(
            "DPI-1067: call timeout of 1000 ms exceeded with ORA-03136",
            ErrorKind::Query,
        );
        assert_eq!(e.kind, ErrorKind::Timeout);
        assert_eq!(e.code.as_deref(), Some("DPI-1067"));
    }

    #[test]
    fn closed_connection_is_lost() {
        let e = from_message(
            "DPI-1080: connection was closed by ORA-03113",
            ErrorKind::Query,
        );
        assert_eq!(e.kind, ErrorKind::ConnectionLost);
    }

    #[test]
    fn common_server_errors() {
        let kind = |m: &str| from_message(m, ErrorKind::Query).kind;
        assert_eq!(
            kind("ORA-01017: invalid username/password; logon denied"),
            ErrorKind::Auth
        );
        assert_eq!(kind("ORA-28000: the account is locked"), ErrorKind::Auth);
        assert_eq!(
            kind("ORA-12514: TNS:listener does not currently know of service requested"),
            ErrorKind::Connect
        );
        assert_eq!(kind("ORA-12541: TNS:no listener"), ErrorKind::Network);
        assert_eq!(
            kind("ORA-12154: TNS:could not resolve the connect identifier specified"),
            ErrorKind::Config
        );
        assert_eq!(
            kind("ORA-01013: user requested cancel of current operation"),
            ErrorKind::Cancelled
        );
        assert_eq!(
            kind("ORA-03114: not connected to ORACLE"),
            ErrorKind::ConnectionLost
        );
        assert_eq!(
            kind("ORA-00942: table or view does not exist"),
            ErrorKind::Query
        );
        assert_eq!(kind("ORA-28759: failure to open file"), ErrorKind::Tls);
    }

    #[test]
    fn multi_line_messages_use_the_first_code() {
        let e = from_message(
            "ORA-06550: line 1, column 7:\nPLS-00201: identifier 'X' must be declared\nORA-06550: line 1",
            ErrorKind::Query,
        );
        assert_eq!(e.code.as_deref(), Some("ORA-06550"));
        assert_eq!(e.kind, ErrorKind::Query);
    }

    #[test]
    fn column_types_the_client_crate_cannot_fetch() {
        assert!(is_unsupported_column_type("unsupported Oracle type JSON"));
        assert!(is_unsupported_column_type("unknown Oracle type number 127"));
        assert!(!is_unsupported_column_type(
            "ORA-00942: table or view does not exist"
        ));
    }

    #[test]
    fn socket_errors_without_a_code_use_the_fallback() {
        let e = from_message("connection refused", ErrorKind::Network);
        assert_eq!(e.kind, ErrorKind::Network);
        assert_eq!(e.code, None);
    }

    #[test]
    fn connect_errors_default_to_connect_and_hide_the_password() {
        // Not in the shared table: a failed Kerberos or NTS handshake.
        let e = connect_error("ORA-12631: Username retrieval failed".into(), "pw");
        assert_eq!(e.kind, ErrorKind::Connect);
        assert_eq!(e.code.as_deref(), Some("ORA-12631"));
        // Known codes keep their kind.
        let e = connect_error("ORA-01017: invalid username/password".into(), "pw");
        assert_eq!(e.kind, ErrorKind::Auth);
        let e = connect_error(
            "DPI-1047: Cannot locate a 64-bit Oracle Client library".into(),
            "",
        );
        assert_eq!(e.kind, ErrorKind::Unavailable);
        // A client library that echoes the connect string must not leak.
        let e = connect_error("ORA-12154: bad scott/hunter2@db".into(), "hunter2");
        assert!(!e.message.contains("hunter2"));
        assert!(e.message.contains("<redacted>"));
        // Empty passwords scrub nothing.
        let e = connect_error("ORA-12154: x".into(), "");
        assert_eq!(e.message, "ORA-12154: x");
    }

    #[test]
    fn fallback_kinds_of_crate_errors() {
        assert_eq!(
            fallback_kind(oracle::ErrorKind::ParseError),
            ErrorKind::Query
        );
        assert_eq!(
            fallback_kind(oracle::ErrorKind::InternalError),
            ErrorKind::Internal
        );
    }
}
