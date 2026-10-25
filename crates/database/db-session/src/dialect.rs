//! The few statements the session layer sends itself, per engine.

use db_core::Dialect;

use crate::job::syntax_dialect;

/// The one savepoint name a console uses (ADR 0021): PostgreSQL and SQLite
/// release it after every statement, Oracle moves it by issuing it again.
pub(crate) const SAVEPOINT: &str = "pyoxide_stmt";

pub(crate) fn savepoint() -> String {
    format!("SAVEPOINT {SAVEPOINT}")
}

pub(crate) fn rollback_to_savepoint() -> String {
    format!("ROLLBACK TO SAVEPOINT {SAVEPOINT}")
}

/// `None` where a savepoint cannot be released (Oracle).
pub(crate) fn release_savepoint(dialect: Dialect) -> Option<String> {
    match dialect {
        Dialect::Oracle => None,
        Dialect::Postgres | Dialect::Sqlite => Some(format!("RELEASE SAVEPOINT {SAVEPOINT}")),
    }
}

/// Whether the session has to open the transaction itself before its first
/// savepoint. SQLite would otherwise open one with the savepoint and commit
/// it with the release; PostgreSQL drivers accept an explicit `BEGIN` as
/// well. Oracle opens transactions implicitly.
pub(crate) fn explicit_begin(dialect: Dialect) -> bool {
    !matches!(dialect, Dialect::Oracle)
}

/// DDL ends the transaction (Oracle commits before and after it).
pub(crate) fn ddl_commits(dialect: Dialect) -> bool {
    matches!(dialect, Dialect::Oracle)
}

/// The server's read-only session mode, where the engine has one.
pub(crate) fn read_only(dialect: Dialect) -> Option<&'static str> {
    match dialect {
        Dialect::Postgres => Some("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY"),
        Dialect::Sqlite => Some("PRAGMA query_only = ON"),
        // No session-wide switch; the drivers refuse writes on a read-only
        // profile.
        Dialect::Oracle => None,
    }
}

fn string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn time_zone(dialect: Dialect, zone: &str) -> Option<String> {
    match dialect {
        Dialect::Postgres => Some(format!("SET TIME ZONE {}", string_literal(zone))),
        Dialect::Oracle => Some(format!(
            "ALTER SESSION SET TIME_ZONE = {}",
            string_literal(zone)
        )),
        Dialect::Sqlite => None,
    }
}

/// Switch the current schema. `None`: the engine has no schemas to switch
/// (SQLite attaches databases instead).
pub(crate) fn set_schema(dialect: Dialect, schema: &str) -> Option<String> {
    let quoted = syntax_dialect(dialect).quote(schema);
    match dialect {
        Dialect::Postgres => Some(format!("SET search_path TO {quoted}")),
        Dialect::Oracle => Some(format!("ALTER SESSION SET CURRENT_SCHEMA = {quoted}")),
        Dialect::Sqlite => None,
    }
}

/// A query returning the current schema in its first column.
pub(crate) fn current_schema(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres => "SELECT current_schema()",
        Dialect::Oracle => "SELECT SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') FROM DUAL",
        Dialect::Sqlite => "SELECT 'main'",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_per_dialect() {
        assert_eq!(release_savepoint(Dialect::Oracle), None);
        assert_eq!(
            release_savepoint(Dialect::Postgres).unwrap(),
            "RELEASE SAVEPOINT pyoxide_stmt"
        );
        assert_eq!(
            time_zone(Dialect::Postgres, "it's").unwrap(),
            "SET TIME ZONE 'it''s'"
        );
        assert_eq!(time_zone(Dialect::Sqlite, "UTC"), None);
        assert_eq!(
            set_schema(Dialect::Postgres, "Sales").unwrap(),
            "SET search_path TO \"Sales\""
        );
        assert_eq!(
            set_schema(Dialect::Oracle, "HR").unwrap(),
            "ALTER SESSION SET CURRENT_SCHEMA = HR"
        );
        assert_eq!(set_schema(Dialect::Sqlite, "main"), None);
        assert!(read_only(Dialect::Oracle).is_none());
    }
}
