use db_core::oracle::sql::{classify, normalize};
use db_core::{Error, Result};
use oracle::StatementType;

/// The text sent to the server for one user statement: without the
/// terminator that editors hand over (`;` after SQL, a `/` line after a
/// block), keeping the `;` that ends a PL/SQL block or stored unit (see
/// `db_core::oracle::sql::normalize`).
pub(crate) fn prepare_sql(sql: &str) -> Result<&str> {
    let s = normalize(sql, classify(sql));
    if s.is_empty() {
        return Err(Error::query("the statement is empty"));
    }
    Ok(s)
}

/// Statements a read-only connection may run: queries, and the statements
/// that end a transaction (they change nothing).
pub(crate) fn allowed_when_read_only(t: StatementType) -> bool {
    matches!(
        t,
        StatementType::Select | StatementType::Commit | StatementType::Rollback
    )
}

/// Whether a statement in `TxMode::Auto` is followed by a commit. DDL commits
/// by itself, queries have nothing to commit.
pub(crate) fn commits_in_auto_mode(t: StatementType) -> bool {
    !matches!(
        t,
        StatementType::Select
            | StatementType::Create
            | StatementType::Alter
            | StatementType::Drop
            | StatementType::Commit
            | StatementType::Rollback
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(sql: &str) -> String {
        prepare_sql(sql).unwrap().to_owned()
    }

    #[test]
    fn terminators_follow_the_statement_kind() {
        assert_eq!(p("select 1 from dual;"), "select 1 from dual");
        assert_eq!(p("call pkg.proc(1);"), "call pkg.proc(1)");
        assert_eq!(p("begin null; end;\n/"), "begin null; end;");
        assert_eq!(p("<<l>> begin null; end;"), "<<l>> begin null; end;");
        assert_eq!(
            p("create or replace procedure p is begin null; end;"),
            "create or replace procedure p is begin null; end;"
        );
    }

    #[test]
    fn empty_statements_are_rejected() {
        for sql in ["", "   ", ";", " ; ", "/", "\n/", "-- only a comment\n/"] {
            let r = prepare_sql(sql);
            // A bare comment is passed on; the server answers it. Everything
            // else is rejected here.
            if sql.contains("comment") {
                assert!(r.is_ok(), "{sql:?}");
            } else {
                assert_eq!(r.unwrap_err().kind, db_core::ErrorKind::Query, "{sql:?}");
            }
        }
    }

    #[test]
    fn read_only_allows_queries_only() {
        assert!(allowed_when_read_only(StatementType::Select));
        assert!(allowed_when_read_only(StatementType::Commit));
        assert!(allowed_when_read_only(StatementType::Rollback));
        for t in [
            StatementType::Insert,
            StatementType::Update,
            StatementType::Delete,
            StatementType::Merge,
            StatementType::Create,
            StatementType::Alter,
            StatementType::Drop,
            StatementType::Begin,
            StatementType::Declare,
            StatementType::Call,
            StatementType::ExplainPlan,
            StatementType::Unknown,
        ] {
            assert!(!allowed_when_read_only(t), "{t:?}");
        }
    }

    #[test]
    fn auto_mode_commits_changes_only() {
        for t in [
            StatementType::Insert,
            StatementType::Update,
            StatementType::Delete,
            StatementType::Merge,
            StatementType::Begin,
            StatementType::Declare,
            StatementType::Call,
            StatementType::Unknown,
        ] {
            assert!(commits_in_auto_mode(t), "{t:?}");
        }
        for t in [
            StatementType::Select,
            StatementType::Create,
            StatementType::Alter,
            StatementType::Drop,
            StatementType::Commit,
            StatementType::Rollback,
        ] {
            assert!(!commits_in_auto_mode(t), "{t:?}");
        }
    }
}
