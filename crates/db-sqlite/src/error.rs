use db_core::{Error, ErrorKind};
use rusqlite::ffi;

/// Name of a SQLite result code, extended codes included where they carry
/// useful information (`SQLITE_CONSTRAINT_UNIQUE`).
pub(crate) fn code_name(extended: i32) -> String {
    let named = match extended {
        ffi::SQLITE_CONSTRAINT_CHECK => Some("SQLITE_CONSTRAINT_CHECK"),
        ffi::SQLITE_CONSTRAINT_FOREIGNKEY => Some("SQLITE_CONSTRAINT_FOREIGNKEY"),
        ffi::SQLITE_CONSTRAINT_NOTNULL => Some("SQLITE_CONSTRAINT_NOTNULL"),
        ffi::SQLITE_CONSTRAINT_PRIMARYKEY => Some("SQLITE_CONSTRAINT_PRIMARYKEY"),
        ffi::SQLITE_CONSTRAINT_TRIGGER => Some("SQLITE_CONSTRAINT_TRIGGER"),
        ffi::SQLITE_CONSTRAINT_UNIQUE => Some("SQLITE_CONSTRAINT_UNIQUE"),
        ffi::SQLITE_CONSTRAINT_ROWID => Some("SQLITE_CONSTRAINT_ROWID"),
        ffi::SQLITE_BUSY_SNAPSHOT => Some("SQLITE_BUSY_SNAPSHOT"),
        ffi::SQLITE_BUSY_RECOVERY => Some("SQLITE_BUSY_RECOVERY"),
        _ => None,
    };
    if let Some(name) = named {
        return name.to_owned();
    }
    let primary = match extended & 0xff {
        ffi::SQLITE_ERROR => "SQLITE_ERROR",
        ffi::SQLITE_INTERNAL => "SQLITE_INTERNAL",
        ffi::SQLITE_PERM => "SQLITE_PERM",
        ffi::SQLITE_ABORT => "SQLITE_ABORT",
        ffi::SQLITE_BUSY => "SQLITE_BUSY",
        ffi::SQLITE_LOCKED => "SQLITE_LOCKED",
        ffi::SQLITE_NOMEM => "SQLITE_NOMEM",
        ffi::SQLITE_READONLY => "SQLITE_READONLY",
        ffi::SQLITE_INTERRUPT => "SQLITE_INTERRUPT",
        ffi::SQLITE_IOERR => "SQLITE_IOERR",
        ffi::SQLITE_CORRUPT => "SQLITE_CORRUPT",
        ffi::SQLITE_NOTFOUND => "SQLITE_NOTFOUND",
        ffi::SQLITE_FULL => "SQLITE_FULL",
        ffi::SQLITE_CANTOPEN => "SQLITE_CANTOPEN",
        ffi::SQLITE_PROTOCOL => "SQLITE_PROTOCOL",
        ffi::SQLITE_EMPTY => "SQLITE_EMPTY",
        ffi::SQLITE_SCHEMA => "SQLITE_SCHEMA",
        ffi::SQLITE_TOOBIG => "SQLITE_TOOBIG",
        ffi::SQLITE_CONSTRAINT => "SQLITE_CONSTRAINT",
        ffi::SQLITE_MISMATCH => "SQLITE_MISMATCH",
        ffi::SQLITE_MISUSE => "SQLITE_MISUSE",
        ffi::SQLITE_NOLFS => "SQLITE_NOLFS",
        ffi::SQLITE_AUTH => "SQLITE_AUTH",
        ffi::SQLITE_FORMAT => "SQLITE_FORMAT",
        ffi::SQLITE_RANGE => "SQLITE_RANGE",
        ffi::SQLITE_NOTADB => "SQLITE_NOTADB",
        _ => "SQLITE_UNKNOWN",
    };
    primary.to_owned()
}

/// `true` for SQLITE_BUSY and SQLITE_LOCKED.
pub(crate) fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(primary_code(e), Some(ffi::SQLITE_BUSY | ffi::SQLITE_LOCKED))
}

fn primary_code(e: &rusqlite::Error) -> Option<i32> {
    failure(e).map(|(extended, _)| extended & 0xff)
}

/// Extended result code and message of an error that came from SQLite.
fn failure(e: &rusqlite::Error) -> Option<(i32, String)> {
    match e {
        rusqlite::Error::SqliteFailure(f, msg) => Some((
            f.extended_code,
            msg.clone().unwrap_or_else(|| e.to_string()),
        )),
        // Syntax errors point at the offending token.
        rusqlite::Error::SqlInputError {
            error, msg, offset, ..
        } => Some((error.extended_code, format!("{msg} (at offset {offset})"))),
        _ => None,
    }
}

/// Map a rusqlite error to the backend-neutral error.
pub(crate) fn map_error(e: &rusqlite::Error) -> Error {
    if let Some((extended, text)) = failure(e) {
        let code = code_name(extended);
        let (kind, message) = match extended & 0xff {
            ffi::SQLITE_INTERRUPT => return Error::cancelled().with_code(code),
            ffi::SQLITE_BUSY => (
                ErrorKind::Query,
                format!(
                    "{text}: another connection or process holds a conflicting lock \
                     (gave up after the busy timeout)"
                ),
            ),
            ffi::SQLITE_LOCKED => (
                ErrorKind::Query,
                format!("{text}: a table is locked by another statement"),
            ),
            ffi::SQLITE_CANTOPEN => (ErrorKind::Config, text),
            ffi::SQLITE_NOTADB => (ErrorKind::Connect, text),
            ffi::SQLITE_MISUSE | ffi::SQLITE_INTERNAL => (ErrorKind::Internal, text),
            _ => (ErrorKind::Query, text),
        };
        return Error::new(kind, message).with_code(code);
    }
    match e {
        rusqlite::Error::MultipleStatement => Error::query(
            "only one statement can run per call; split the script into separate statements",
        ),
        rusqlite::Error::InvalidParameterCount(given, expected) => Error::query(format!(
            "the statement has {expected} bind parameter(s) but {given} value(s) were given"
        )),
        rusqlite::Error::NulError(_) => Error::query("the SQL text contains a NUL character"),
        rusqlite::Error::ToSqlConversionFailure(inner) => Error::unsupported(inner.to_string()),
        other => Error::query(other.to_string()),
    }
}

/// Map an error from opening a database file.
pub(crate) fn map_open_error(e: &rusqlite::Error, path: &str) -> Error {
    let mapped = map_error(e);
    match mapped.kind {
        ErrorKind::Config => Error::config(format!(
            "cannot open SQLite database '{path}': {}; check the path and permissions",
            mapped.message
        ))
        .with_code(mapped.code.unwrap_or_default()),
        ErrorKind::Connect => Error::new(
            ErrorKind::Connect,
            format!("'{path}' is not a SQLite database (or it is encrypted or corrupt)"),
        )
        .with_code(mapped.code.unwrap_or_default()),
        _ => mapped,
    }
}

pub(crate) fn worker_gone() -> Error {
    Error::connection_lost("the SQLite connection is closed or its worker thread has stopped")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(extended: i32, msg: &str) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(ffi::Error::new(extended), Some(msg.to_owned()))
    }

    #[test]
    fn kinds_and_codes() {
        let e = map_error(&failure(ffi::SQLITE_INTERRUPT, "interrupted"));
        assert_eq!(e.kind, ErrorKind::Cancelled);
        assert_eq!(e.code.as_deref(), Some("SQLITE_INTERRUPT"));

        let e = map_error(&failure(ffi::SQLITE_BUSY, "database is locked"));
        assert_eq!(e.kind, ErrorKind::Query);
        assert_eq!(e.code.as_deref(), Some("SQLITE_BUSY"));
        assert!(e.message.contains("busy timeout"));

        let e = map_error(&failure(
            ffi::SQLITE_CONSTRAINT_UNIQUE,
            "UNIQUE failed: t.a",
        ));
        assert_eq!(e.kind, ErrorKind::Query);
        assert_eq!(e.code.as_deref(), Some("SQLITE_CONSTRAINT_UNIQUE"));

        let e = map_error(&failure(
            ffi::SQLITE_READONLY,
            "attempt to write a readonly database",
        ));
        assert_eq!(e.code.as_deref(), Some("SQLITE_READONLY"));
        assert_eq!(e.kind, ErrorKind::Query);

        let e = map_open_error(
            &failure(ffi::SQLITE_CANTOPEN, "unable to open database file"),
            "x.db",
        );
        assert_eq!(e.kind, ErrorKind::Config);
        assert!(e.message.contains("x.db"));

        let e = map_open_error(
            &failure(ffi::SQLITE_NOTADB, "file is not a database"),
            "x.db",
        );
        assert_eq!(e.kind, ErrorKind::Connect);
    }
}
