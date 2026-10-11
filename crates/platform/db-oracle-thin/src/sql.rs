//! Bind placeholders and `FOR UPDATE` handling for Oracle SQL text, on top
//! of the shared lexical helpers in `db_core::oracle::sql` (statement kind,
//! terminators, code runs outside literals and comments).

use db_core::oracle::sql::{is_ident_byte, scan_code};
use db_core::{Error, Result};

pub use db_core::oracle::sql::{StmtKind, classify, normalize};

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
