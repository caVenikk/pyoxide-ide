//! Lexical helpers for Oracle SQL text shared by both Oracle drivers:
//! statement kind and terminator handling. They follow the lexical rules of
//! the Oracle SQL Language Reference (quoted strings, `q'[...]'` quoting,
//! quoted identifiers, `--` and `/* */` comments).

/// What a statement does, as far as a driver needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StmtKind {
    /// SELECT / WITH: returns rows.
    Query,
    /// INSERT, UPDATE, DELETE, MERGE.
    Dml,
    /// Anonymous PL/SQL block (`BEGIN`, `DECLARE`, `<<label>>`).
    Plsql,
    /// `CALL`: SQL that runs a stored procedure (no trailing semicolon).
    Call,
    /// DDL (implicitly commits).
    Ddl,
    /// `CREATE [OR REPLACE] PROCEDURE|FUNCTION|PACKAGE|TRIGGER|TYPE|...`:
    /// DDL whose body is PL/SQL (keeps its trailing semicolon).
    PlsqlDdl,
    AlterSession,
    Commit,
    Rollback,
    /// SAVEPOINT, SET TRANSACTION, LOCK TABLE, EXPLAIN PLAN, ...
    Other,
}

impl StmtKind {
    pub fn returns_rows(self) -> bool {
        self == StmtKind::Query
    }

    /// Statements that never take bind values.
    pub fn is_ddl(self) -> bool {
        matches!(self, StmtKind::Ddl | StmtKind::PlsqlDdl)
    }

    /// The final `;` belongs to the statement (PL/SQL blocks and units).
    pub fn keeps_semicolon(self) -> bool {
        matches!(self, StmtKind::Plsql | StmtKind::PlsqlDdl)
    }
}

/// Visit the byte ranges of `sql` that are code, not strings, quoted
/// identifiers or comments. The callback gets `(start, end)` of each run.
pub fn scan_code(sql: &str, mut on_code: impl FnMut(usize, usize)) {
    let b = sql.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut code_start = 0;
    while i < n {
        let c = b[i];
        let skip_to = if c == b'-' && i + 1 < n && b[i + 1] == b'-' {
            Some(memchr(b, i + 2, b'\n').map_or(n, |p| p + 1))
        } else if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            Some(find_seq(b, i + 2, b"*/").map_or(n, |p| p + 2))
        } else if c == b'"' {
            Some(memchr(b, i + 1, b'"').map_or(n, |p| p + 1))
        } else if (c == b'q' || c == b'Q') && i + 2 < n && b[i + 1] == b'\'' && !prev_is_ident(b, i)
        {
            Some(skip_q_quote(b, i + 2))
        } else if (c == b'n' || c == b'N')
            && i + 3 < n
            && (b[i + 1] == b'q' || b[i + 1] == b'Q')
            && b[i + 2] == b'\''
            && !prev_is_ident(b, i)
        {
            Some(skip_q_quote(b, i + 3))
        } else if c == b'\'' {
            Some(skip_string(b, i + 1))
        } else {
            None
        };
        match skip_to {
            Some(end) => {
                if code_start < i {
                    on_code(code_start, i);
                }
                i = end.max(i + 1);
                code_start = i;
            }
            None => i += 1,
        }
    }
    if code_start < n {
        on_code(code_start, n);
    }
}

fn prev_is_ident(b: &[u8], i: usize) -> bool {
    i > 0 && is_ident_byte(b[i - 1])
}

/// A byte that can be part of an unquoted identifier (non-ASCII included).
pub fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'#' || c >= 0x80
}

fn memchr(b: &[u8], from: usize, needle: u8) -> Option<usize> {
    b.get(from..)?
        .iter()
        .position(|&c| c == needle)
        .map(|p| p + from)
}

fn find_seq(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// After the opening quote of `'...'`; `''` is an escaped quote.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        if b[i] == b'\'' {
            if i + 1 < b.len() && b[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// `i` points at the delimiter after `q'`.
fn skip_q_quote(b: &[u8], i: usize) -> usize {
    let Some(&open) = b.get(i) else {
        return b.len();
    };
    let close = match open {
        b'[' => b']',
        b'{' => b'}',
        b'(' => b')',
        b'<' => b'>',
        other => other,
    };
    let mut j = i + 1;
    while j + 1 < b.len() {
        if b[j] == close && b[j + 1] == b'\'' {
            return j + 2;
        }
        j += 1;
    }
    b.len()
}

/// The code of a statement: strings, quoted identifiers and comments are
/// left out, code runs are separated by a space.
fn code_of(sql: &str) -> String {
    let mut code = String::new();
    scan_code(sql, |s, e| {
        code.push_str(&sql[s..e]);
        code.push(' ');
    });
    code
}

/// The first keyword(s) of a statement, upper case, skipping whitespace,
/// comments, strings and opening parentheses.
fn leading_words(code: &str, count: usize) -> Vec<String> {
    code.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '#'))
        .filter(|w| !w.is_empty())
        .take(count)
        .map(str::to_ascii_uppercase)
        .collect()
}

/// Classify a statement by its leading keywords.
pub fn classify(sql: &str) -> StmtKind {
    let code = code_of(sql);
    if code.trim_start().starts_with("<<") {
        // A label before an anonymous block.
        return StmtKind::Plsql;
    }
    let w = leading_words(&code, 8);
    let first = w.first().map(String::as_str).unwrap_or("");
    match first {
        "SELECT" | "WITH" => StmtKind::Query,
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" => StmtKind::Dml,
        "BEGIN" | "DECLARE" => StmtKind::Plsql,
        "CALL" => StmtKind::Call,
        "COMMIT" => StmtKind::Commit,
        "ROLLBACK" => {
            if w.get(1).map(String::as_str) == Some("TO") {
                StmtKind::Other
            } else {
                StmtKind::Rollback
            }
        }
        "ALTER" if w.get(1).map(String::as_str) == Some("SESSION") => StmtKind::AlterSession,
        "CREATE" => {
            let mut rest = w[1..].iter().map(String::as_str).peekable();
            if rest.peek() == Some(&"OR") {
                rest.next();
                rest.next(); // REPLACE
            }
            while let Some(&m) = rest.peek() {
                if matches!(
                    m,
                    "EDITIONABLE"
                        | "NONEDITIONABLE"
                        | "EDITIONING"
                        | "AND"
                        | "COMPILE"
                        | "RESOLVE"
                        | "NOFORCE"
                        | "FORCE"
                ) {
                    rest.next();
                } else {
                    break;
                }
            }
            match rest.next() {
                Some(
                    "PROCEDURE" | "FUNCTION" | "PACKAGE" | "TRIGGER" | "TYPE" | "LIBRARY" | "JAVA",
                ) => StmtKind::PlsqlDdl,
                _ => StmtKind::Ddl,
            }
        }
        "ALTER" | "DROP" | "TRUNCATE" | "GRANT" | "REVOKE" | "RENAME" | "COMMENT" | "ANALYZE"
        | "AUDIT" | "NOAUDIT" | "PURGE" | "FLASHBACK" | "ASSOCIATE" | "DISASSOCIATE" => {
            StmtKind::Ddl
        }
        _ => StmtKind::Other,
    }
}

/// Trim the text and drop terminators that are not part of the statement:
/// trailing SQL*Plus `/` lines, and trailing semicolons unless `kind` keeps
/// them (PL/SQL blocks and units). Editors hand statements over with their
/// terminator; the server rejects the `;` after plain SQL (ORA-00911) but
/// needs the one that ends a PL/SQL block. An empty result means there is
/// no statement.
pub fn normalize(sql: &str, kind: StmtKind) -> &str {
    let mut s = sql.trim();
    // SQL*Plus terminator on its own line.
    loop {
        let t = s.trim_end();
        if let Some(stripped) = t.strip_suffix('/') {
            let before = stripped.trim_end_matches([' ', '\t', '\r']);
            if before.is_empty() || before.ends_with('\n') {
                s = before.trim_end();
                continue;
            }
        }
        break;
    }
    if !kind.keeps_semicolon() {
        while let Some(stripped) = s.trim_end().strip_suffix(';') {
            s = stripped.trim_end();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_statements() {
        assert_eq!(classify("  SELECT 1 FROM dual"), StmtKind::Query);
        assert_eq!(
            classify("with x as (select 1 from dual) select * from x"),
            StmtKind::Query
        );
        assert_eq!(
            classify("-- c\n/* block */ ( (SELECT 1 FROM dual) )"),
            StmtKind::Query
        );
        assert_eq!(classify("select*from dual"), StmtKind::Query);
        assert_eq!(classify("update t set a = 1"), StmtKind::Dml);
        assert_eq!(classify("MERGE INTO t USING s ON (1=1)"), StmtKind::Dml);
        assert_eq!(classify("begin null; end;"), StmtKind::Plsql);
        assert_eq!(
            classify("DECLARE x NUMBER; BEGIN NULL; END;"),
            StmtKind::Plsql
        );
        assert_eq!(classify("<<outer>> begin null; end;"), StmtKind::Plsql);
        assert_eq!(classify("-- c\n<<l>> begin null; end;"), StmtKind::Plsql);
        assert_eq!(classify("call p()"), StmtKind::Call);
        assert_eq!(classify("create table t (a number)"), StmtKind::Ddl);
        assert_eq!(
            classify("CREATE OR REPLACE EDITIONABLE PROCEDURE p AS BEGIN NULL; END;"),
            StmtKind::PlsqlDdl
        );
        assert_eq!(
            classify("create or replace package body p as end;"),
            StmtKind::PlsqlDdl
        );
        assert_eq!(
            classify("CREATE TYPE t AS OBJECT (a NUMBER)"),
            StmtKind::PlsqlDdl
        );
        assert_eq!(
            classify("create or replace and compile java source named x as ..."),
            StmtKind::PlsqlDdl
        );
        assert_eq!(
            classify("create or replace and resolve noforce java source named x as ..."),
            StmtKind::PlsqlDdl
        );
        assert_eq!(
            classify("ALTER SESSION SET NLS_DATE_FORMAT = 'YYYY'"),
            StmtKind::AlterSession
        );
        assert_eq!(classify("alter table t add c number"), StmtKind::Ddl);
        assert_eq!(classify("COMMIT"), StmtKind::Commit);
        assert_eq!(classify("rollback"), StmtKind::Rollback);
        assert_eq!(classify("rollback to savepoint a"), StmtKind::Other);
        assert_eq!(classify("savepoint a"), StmtKind::Other);
        assert_eq!(classify("'select' "), StmtKind::Other);
        assert_eq!(classify(""), StmtKind::Other);
    }

    #[test]
    fn plain_ddl_is_not_a_unit() {
        for sql in [
            "create table t (a int);",
            "create or replace view v as select 1 from dual;",
            "create or replace editioning view v as select 1 from dual;",
            "create index i on t (a);",
            "create sequence s;",
            "create synonym s for t;",
            "create or replace directory d as '/tmp';",
        ] {
            assert_eq!(classify(sql), StmtKind::Ddl, "{sql}");
        }
    }

    fn n(sql: &str) -> &str {
        normalize(sql, classify(sql))
    }

    #[test]
    fn sql_loses_its_terminator() {
        assert_eq!(n("select 1 from dual;"), "select 1 from dual");
        assert_eq!(n("  select 1 from dual ;  \n"), "select 1 from dual");
        assert_eq!(n("select 1 from dual;;"), "select 1 from dual");
        assert_eq!(n("create table t (a int);"), "create table t (a int)");
        assert_eq!(
            n("insert into t values (';');"),
            "insert into t values (';')"
        );
        assert_eq!(n("-- c\nselect 1;"), "-- c\nselect 1");
        assert_eq!(n("/* c */ select 1;"), "/* c */ select 1");
        assert_eq!(n("call pkg.proc(1);"), "call pkg.proc(1)");
        assert_eq!(
            n("with function f return number is begin return 1; end; select f from dual;"),
            "with function f return number is begin return 1; end; select f from dual"
        );
    }

    #[test]
    fn plsql_keeps_its_terminator() {
        for sql in [
            "begin null; end;",
            "declare x number; begin null; end;",
            "-- hi\nbegin null; end;",
            "<<l>> begin null; end;",
            "create or replace procedure p is begin null; end;",
            "CREATE FUNCTION f RETURN NUMBER IS BEGIN RETURN 1; END;",
            "create or replace editionable package pk as end;",
            "create package body pk as end;",
            "create or replace trigger tr before insert on t begin null; end;",
            "create or replace type ty as object (a number);",
            "create type body ty as end;",
            "create or replace and compile java source named \"S\" as class S {}",
        ] {
            assert_eq!(n(sql), sql, "{sql}");
        }
        assert_eq!(n("BEGIN NULL; END;  "), "BEGIN NULL; END;");
    }

    #[test]
    fn sqlplus_slash_terminator() {
        assert_eq!(n("begin null; end;\n/"), "begin null; end;");
        assert_eq!(n("begin null; end;\n/\n"), "begin null; end;");
        assert_eq!(n("begin null; end;\r\n /\n/"), "begin null; end;");
        assert_eq!(n("select 1 from dual\n/"), "select 1 from dual");
        assert_eq!(n("select 4 / 2 from dual"), "select 4 / 2 from dual");
        assert_eq!(n("SELECT 4/2 FROM dual"), "SELECT 4/2 FROM dual");
        assert_eq!(
            normalize(
                "CREATE PROCEDURE p AS BEGIN NULL; END;\n/",
                StmtKind::PlsqlDdl
            ),
            "CREATE PROCEDURE p AS BEGIN NULL; END;"
        );
    }

    #[test]
    fn empty_statements_normalize_to_nothing() {
        for sql in ["", "   ", ";", " ; ", "/", "\n/", " ;\n/"] {
            assert_eq!(n(sql), "", "{sql:?}");
        }
        // A bare comment is passed on; the server answers it.
        assert_eq!(n("-- only a comment\n/"), "-- only a comment");
    }

    #[test]
    fn code_runs_skip_literals_and_comments() {
        let sql = "SELECT 'a;b', q'[x']y]', \"Q\" -- c\n/* d */ FROM t";
        let mut code = String::new();
        scan_code(sql, |s, e| code.push_str(&sql[s..e]));
        assert_eq!(code, "SELECT , ,   FROM t");
        assert!(is_ident_byte(b'#') && is_ident_byte(0xC3) && !is_ident_byte(b'('));
    }
}
