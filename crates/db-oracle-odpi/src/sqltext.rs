use db_core::{Error, Result};
use oracle::StatementType;

/// The text sent to the server for one user statement.
///
/// Editors hand over statements with their terminator: the `;` that ends a
/// SQL statement or a `/` line after a PL/SQL block. The server rejects the
/// `;` after plain SQL (ORA-00911) but needs the one that closes a PL/SQL
/// block or a stored unit, so the terminator is removed only for SQL.
pub(crate) fn prepare_sql(sql: &str) -> Result<&str> {
    let mut s = sql.trim();
    // A lone `/` on the last line ends a block in SQL*Plus style scripts.
    if let Some((rest, last)) = s.rsplit_once('\n')
        && last.trim() == "/"
    {
        s = rest.trim_end();
    } else if s == "/" {
        s = "";
    }
    if s.is_empty() {
        return Err(Error::query("the statement is empty"));
    }
    if is_plsql_unit(s) {
        return Ok(s);
    }
    let stripped = s.trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    if stripped.is_empty() {
        return Err(Error::query("the statement is empty"));
    }
    Ok(stripped)
}

/// Anonymous PL/SQL blocks and `CREATE` statements for stored units, where
/// the final `;` belongs to the statement.
pub(crate) fn is_plsql_unit(sql: &str) -> bool {
    let mut words = Words(sql);
    let Some(first) = words.next() else {
        // Starts with a non-word: a `<<label>>` before a block.
        return sql.trim_start().starts_with("<<");
    };
    if first.eq_ignore_ascii_case("begin") || first.eq_ignore_ascii_case("declare") {
        return true;
    }
    if !first.eq_ignore_ascii_case("create") {
        return false;
    }
    // CREATE [OR REPLACE] [EDITIONABLE|NONEDITIONABLE] [AND ... NOFORCE] <unit>
    for word in words.take(8) {
        let w = word.to_ascii_lowercase();
        match w.as_str() {
            "procedure" | "function" | "package" | "trigger" | "type" | "library" | "java" => {
                return true;
            }
            "or" | "replace" | "editionable" | "noneditionable" | "and" | "resolve" | "compile"
            | "force" | "noforce" => continue,
            _ => return false,
        }
    }
    false
}

/// Words at the start of a statement, skipping whitespace and comments.
/// Stops at the first character that cannot be part of a keyword.
struct Words<'a>(&'a str);

impl<'a> Iterator for Words<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        loop {
            let s = self.0.trim_start();
            if let Some(rest) = s.strip_prefix("--") {
                self.0 = rest.split_once('\n').map_or("", |(_, r)| r);
            } else if let Some(rest) = s.strip_prefix("/*") {
                self.0 = rest.split_once("*/").map_or("", |(_, r)| r);
            } else {
                self.0 = s;
                break;
            }
        }
        let end = self
            .0
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(self.0.len());
        if end == 0 {
            return None;
        }
        let (word, rest) = self.0.split_at(end);
        self.0 = rest;
        Some(word)
    }
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
    fn sql_loses_its_terminator() {
        assert_eq!(p("select 1 from dual;"), "select 1 from dual");
        assert_eq!(p("  select 1 from dual ;  \n"), "select 1 from dual");
        assert_eq!(p("select 1 from dual;;"), "select 1 from dual");
        assert_eq!(p("create table t (a int);"), "create table t (a int)");
        assert_eq!(
            p("insert into t values (';');"),
            "insert into t values (';')"
        );
        assert_eq!(p("select 1 from dual"), "select 1 from dual");
        assert_eq!(p("-- c\nselect 1;"), "-- c\nselect 1");
        assert_eq!(p("/* c */ select 1;"), "/* c */ select 1");
        assert_eq!(p("call pkg.proc(1);"), "call pkg.proc(1)");
        assert_eq!(
            p("with function f return number is begin return 1; end; select f from dual;"),
            "with function f return number is begin return 1; end; select f from dual"
        );
    }

    #[test]
    fn plsql_keeps_its_terminator() {
        assert_eq!(p("begin null; end;"), "begin null; end;");
        assert_eq!(p("BEGIN NULL; END;  "), "BEGIN NULL; END;");
        assert_eq!(
            p("declare x number; begin null; end;"),
            "declare x number; begin null; end;"
        );
        assert_eq!(p("-- hi\nbegin null; end;"), "-- hi\nbegin null; end;");
        assert_eq!(p("<<l>> begin null; end;"), "<<l>> begin null; end;");
        for sql in [
            "create or replace procedure p is begin null; end;",
            "CREATE FUNCTION f RETURN NUMBER IS BEGIN RETURN 1; END;",
            "create or replace editionable package pk as end;",
            "create package body pk as end;",
            "create or replace trigger tr before insert on t begin null; end;",
            "create or replace type ty as object (a number);",
            "create type body ty as end;",
            "create or replace and compile java source named \"S\" as class S {}",
        ] {
            assert_eq!(p(sql), sql, "{sql}");
        }
    }

    #[test]
    fn plain_ddl_is_not_a_unit() {
        for sql in [
            "create table t (a int);",
            "create or replace view v as select 1 from dual;",
            "create index i on t (a);",
            "create sequence s;",
            "create synonym s for t;",
            "create or replace directory d as '/tmp';",
        ] {
            assert!(!is_plsql_unit(sql), "{sql}");
            assert!(!p(sql).ends_with(';'), "{sql}");
        }
    }

    #[test]
    fn sqlplus_slash_terminator() {
        assert_eq!(p("begin null; end;\n/"), "begin null; end;");
        assert_eq!(p("begin null; end;\n/\n"), "begin null; end;");
        assert_eq!(p("select 1 from dual\n/"), "select 1 from dual");
        assert_eq!(p("select 4 / 2 from dual"), "select 4 / 2 from dual");
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
