//! Lexical helpers for Oracle SQL text: statement kind, bind placeholders
//! and normalization. They follow the lexical rules of the Oracle SQL
//! Language Reference (quoted strings, `q'[...]'` quoting, quoted
//! identifiers, `--` and `/* */` comments).

use db_core::{Error, Result};

/// What a statement does, as far as the driver needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StmtKind {
    /// SELECT / WITH: returns rows.
    Query,
    /// INSERT, UPDATE, DELETE, MERGE.
    Dml,
    /// Anonymous block or CALL.
    Plsql,
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
}

/// Byte ranges of SQL text that are code (not strings, identifiers or
/// comments) are visited by `scan`. The callback gets `(start, end)` of each
/// code run.
fn scan_code(sql: &str, mut on_code: impl FnMut(usize, usize)) {
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

fn is_ident_byte(c: u8) -> bool {
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

/// The first keyword(s) of a statement, upper case, skipping whitespace,
/// comments and opening parentheses.
fn leading_words(sql: &str, count: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut code = String::new();
    scan_code(sql, |s, e| {
        code.push_str(&sql[s..e]);
        code.push(' ');
    });
    for w in code
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '#'))
        .filter(|w| !w.is_empty())
    {
        words.push(w.to_ascii_uppercase());
        if words.len() == count {
            break;
        }
    }
    words
}

/// Classify a statement by its leading keywords.
pub fn classify(sql: &str) -> StmtKind {
    let w = leading_words(sql, 6);
    let first = w.first().map(String::as_str).unwrap_or("");
    match first {
        "SELECT" | "WITH" => StmtKind::Query,
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" => StmtKind::Dml,
        "BEGIN" | "DECLARE" | "CALL" => StmtKind::Plsql,
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

/// Split a query at its top-level `FOR UPDATE` clause (outside strings,
/// comments and parentheses): `(body, Some(tail))` where the tail starts
/// with `FOR`.
pub fn split_for_update(sql: &str) -> (&str, Option<&str>) {
    let b = sql.as_bytes();
    let mut depth = 0i32;
    let mut found = None;
    scan_code(sql, |s, e| {
        let mut i = s;
        while i < e {
            match b[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                c if depth == 0
                    && (c == b'f' || c == b'F')
                    && (i == 0 || !is_ident_byte(b[i - 1]))
                    && sql[i..e].len() >= 3
                    && sql[i..i + 3].eq_ignore_ascii_case("for")
                    && !b.get(i + 3).is_some_and(|&c| is_ident_byte(c)) =>
                {
                    let rest = sql[i + 3..e].trim_start();
                    if rest.len() >= 6
                        && rest[..6].eq_ignore_ascii_case("update")
                        && !rest.as_bytes().get(6).is_some_and(|&c| is_ident_byte(c))
                    {
                        found = Some(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
    });
    match found {
        Some(p) => (sql[..p].trim_end(), Some(&sql[p..])),
        None => (sql, None),
    }
}

/// Whether a query has a top-level `FOR UPDATE` clause.
pub fn has_for_update(sql: &str) -> bool {
    split_for_update(sql).1.is_some()
}

/// Trim the text and drop terminators that are not part of the statement:
/// a trailing SQL*Plus `/` line, and trailing semicolons of SQL statements
/// (PL/SQL blocks and PL/SQL units keep theirs).
pub fn normalize(sql: &str, kind: StmtKind) -> String {
    let mut s = sql.trim().to_owned();
    // SQL*Plus terminator on its own line.
    loop {
        let t = s.trim_end();
        if let Some(stripped) = t.strip_suffix('/') {
            let before = stripped.trim_end_matches([' ', '\t', '\r']);
            if before.is_empty() || before.ends_with('\n') {
                s = before.trim_end().to_owned();
                continue;
            }
        }
        break;
    }
    // CALL is SQL: no semicolon, unlike anonymous blocks and PL/SQL units.
    let keeps_semicolon = match kind {
        StmtKind::PlsqlDdl => true,
        StmtKind::Plsql => classify_first(&s) != "CALL",
        _ => false,
    };
    if !keeps_semicolon {
        while let Some(stripped) = s.trim_end().strip_suffix(';') {
            s = stripped.trim_end().to_owned();
        }
    }
    s
}

fn classify_first(sql: &str) -> String {
    leading_words(sql, 1).pop().unwrap_or_default()
}

/// A bind placeholder (`:1`, `:name`) found in code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// Name without the colon, upper case unless numeric.
    pub name: String,
    /// Byte range of `:name` in the text.
    pub start: usize,
    pub end: usize,
}

/// Bind placeholders in order of appearance, outside strings, quoted
/// identifiers and comments. `:=` and `::` are not placeholders.
pub fn placeholders(sql: &str) -> Vec<Placeholder> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    scan_code(sql, |s, e| {
        let mut i = s;
        while i < e {
            if b[i] == b':' && i + 1 < e && (i == 0 || b[i - 1] != b':') {
                let c = b[i + 1];
                if c.is_ascii_alphanumeric() || c >= 0x80 {
                    let mut j = i + 1;
                    while j < e && is_ident_byte(b[j]) {
                        j += 1;
                    }
                    let raw = &sql[i + 1..j];
                    let name = if raw.bytes().all(|c| c.is_ascii_digit()) {
                        raw.trim_start_matches('0').to_owned()
                    } else {
                        raw.to_uppercase()
                    };
                    out.push(Placeholder {
                        name: if name.is_empty() { "0".into() } else { name },
                        start: i,
                        end: j,
                    });
                    i = j;
                    continue;
                }
            }
            i += 1;
        }
    });
    out
}

/// Which parameter each distinct placeholder takes, in order of first
/// appearance. Numeric names bind by number (`:2` takes `params[1]`, every
/// occurrence of `:2` gets the same value); otherwise the n-th distinct name
/// takes `params[n]`.
pub fn bind_plan(found: &[Placeholder], param_count: usize) -> Result<Vec<(String, usize)>> {
    let mut distinct: Vec<&str> = Vec::new();
    for p in found {
        if !distinct.contains(&p.name.as_str()) {
            distinct.push(&p.name);
        }
    }
    if distinct.is_empty() {
        if param_count > 0 {
            return Err(Error::query(format!(
                "{param_count} bind value(s) given, but the statement has no placeholders"
            )));
        }
        return Ok(Vec::new());
    }
    let numeric = distinct
        .iter()
        .all(|n| n.bytes().all(|c| c.is_ascii_digit()));
    if numeric {
        let mut plan = Vec::with_capacity(distinct.len());
        let mut used = vec![false; param_count];
        for name in &distinct {
            let n: usize = name.parse().unwrap_or(0);
            if n == 0 || n > param_count {
                return Err(Error::query(format!(
                    "placeholder :{name} has no value ({param_count} bind value(s) given)"
                )));
            }
            used[n - 1] = true;
            plan.push(((*name).to_owned(), n - 1));
        }
        if let Some(unused) = used.iter().position(|u| !u) {
            return Err(Error::query(format!(
                "bind value {} is not used by the statement",
                unused + 1
            )));
        }
        Ok(plan)
    } else {
        if distinct.len() != param_count {
            return Err(Error::query(format!(
                "the statement has {} distinct placeholder(s), but {param_count} bind value(s) were given",
                distinct.len()
            )));
        }
        Ok(distinct
            .iter()
            .enumerate()
            .map(|(i, n)| ((*n).to_owned(), i))
            .collect())
    }
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
        assert_eq!(classify("call p()"), StmtKind::Plsql);
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
    fn finds_placeholders_outside_literals() {
        let sql = "SELECT :1, ':2', q'[:3]', \"A:4\", :name -- :5\n /* :6 */ FROM t WHERE x := :Name2 AND y = :01";
        let names: Vec<_> = placeholders(sql).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["1", "NAME", "NAME2", "1"]);
        let p = &placeholders("x = :abc")[0];
        assert_eq!(&"x = :abc"[p.start..p.end], ":abc");
        assert!(placeholders("BEGIN x := 1; END;").is_empty());
        assert!(placeholders("SELECT 'it''s :x' FROM dual").is_empty());
        assert!(placeholders("SELECT nq'{:x}' FROM dual").is_empty());
        assert!(placeholders("SELECT a::b FROM t").len() <= 1);
        // A q right after an identifier is not a q-quote.
        assert_eq!(placeholders("SELECT seq'x' , :1 FROM dual").len(), 1);
    }

    #[test]
    fn plans_binds() {
        let p = placeholders("SELECT :2, :1, :2 FROM dual");
        assert_eq!(
            bind_plan(&p, 2).unwrap(),
            vec![("2".into(), 1), ("1".into(), 0)]
        );
        assert!(bind_plan(&p, 1).is_err());
        assert!(bind_plan(&p, 3).is_err());
        let p = placeholders("SELECT :b, :a, :b FROM dual");
        assert_eq!(
            bind_plan(&p, 2).unwrap(),
            vec![("B".into(), 0), ("A".into(), 1)]
        );
        assert!(bind_plan(&p, 3).is_err());
        assert!(bind_plan(&[], 1).is_err());
        assert!(bind_plan(&[], 0).unwrap().is_empty());
    }

    #[test]
    fn normalizes_terminators() {
        assert_eq!(
            normalize(" SELECT 1 FROM dual ; ", StmtKind::Query),
            "SELECT 1 FROM dual"
        );
        assert_eq!(
            normalize("BEGIN NULL; END;\n/\n", StmtKind::Plsql),
            "BEGIN NULL; END;"
        );
        assert_eq!(
            normalize(
                "CREATE PROCEDURE p AS BEGIN NULL; END;\n/",
                StmtKind::PlsqlDdl
            ),
            "CREATE PROCEDURE p AS BEGIN NULL; END;"
        );
        assert_eq!(normalize("CALL p();", StmtKind::Plsql), "CALL p()");
        assert_eq!(
            normalize("SELECT 4/2 FROM dual", StmtKind::Query),
            "SELECT 4/2 FROM dual"
        );
        assert_eq!(
            normalize("insert into t values (1);;", StmtKind::Dml),
            "insert into t values (1)"
        );
    }

    #[test]
    fn detects_for_update() {
        assert!(has_for_update("SELECT * FROM t FOR UPDATE"));
        assert!(has_for_update("select * from t for  update nowait"));
        assert!(!has_for_update("SELECT 'for update' FROM t"));
        assert!(!has_for_update("SELECT * FROM t -- for update"));
        assert!(!has_for_update(
            "SELECT * FROM (SELECT * FROM t FOR UPDATE)"
        ));
        assert!(!has_for_update("SELECT before_update FROM t"));
        let (body, tail) = split_for_update("SELECT ROWID, t.* FROM t\nFOR UPDATE NOWAIT");
        assert_eq!(body, "SELECT ROWID, t.* FROM t");
        assert_eq!(tail, Some("FOR UPDATE NOWAIT"));
    }
}
