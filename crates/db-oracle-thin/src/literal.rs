//! Values as Oracle literals.
//!
//! oracle-rs 0.1.7 cannot bind longer strings reliably and returns only the
//! first OUT value of a PL/SQL call, so the driver never uses its binds: bind
//! values become typed PL/SQL variables in the wrapper block (bound to the
//! user's statement with `DBMS_SQL.BIND_VARIABLE`, so the statement keeps
//! real binds) or, for the LOB fallback path, SQL literals. Only `'` needs
//! escaping in Oracle string literals.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use db_core::{Error, Result, Value};

/// Longest escaped text placed in one PL/SQL literal (the limit is 32767
/// bytes).
const PLSQL_PIECE: usize = 30_000;
/// Longest escaped text in one SQL literal (the limit is 4000 bytes).
const SQL_PIECE: usize = 3_900;

/// Split `s` into pieces whose escaped length stays under `max_bytes`,
/// never inside a character.
pub fn pieces(s: &str, max_bytes: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut len = 0;
    for (i, c) in s.char_indices() {
        let w = if c == '\'' { 2 } else { c.len_utf8() };
        if len + w > max_bytes && i > start {
            out.push(&s[start..i]);
            start = i;
            len = 0;
        }
        len += w;
    }
    if start < s.len() || out.is_empty() {
        out.push(&s[start..]);
    }
    out
}

/// A quoted string literal. NUL characters are spliced in with `CHR(0)`;
/// an empty string becomes `''` (which Oracle treats as NULL).
pub fn quote(s: &str) -> String {
    if !s.contains('\0') {
        return format!("'{}'", s.replace('\'', "''"));
    }
    s.split('\0')
        .map(|p| format!("'{}'", p.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(" || CHR(0) || ")
}

/// How one bind value is declared and bound inside the wrapper block.
#[derive(Debug, Clone, PartialEq)]
pub struct PlsqlBind {
    /// Declaration after the variable name (`NUMBER := 42`).
    pub decl: String,
    /// Statements run at the start of the block (temporary LOB building).
    pub init: Vec<String>,
    /// `BIND_VARIABLE` or `BIND_VARIABLE_RAW`.
    pub bind_proc: &'static str,
}

impl PlsqlBind {
    fn simple(decl: String) -> Self {
        Self {
            decl,
            init: Vec::new(),
            bind_proc: "BIND_VARIABLE",
        }
    }
}

/// Validate exact decimal text (`-12.50`, `1e5`) so it can be used as a
/// numeric literal.
pub fn decimal_literal(s: &str) -> Result<String> {
    let t = s.trim();
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(p) => (&body[..p], Some(&body[p + 1..])),
        None => (body, None),
    };
    let mut parts = mantissa.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    let digits_ok = |x: &str| x.bytes().all(|c| c.is_ascii_digit());
    let exp_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && digits_ok(e)
    });
    if (int.is_empty() && frac.is_empty()) || !digits_ok(int) || !digits_ok(frac) || !exp_ok {
        return Err(Error::query(format!("`{s}` is not a decimal number")));
    }
    Ok(t.trim_start_matches('+').to_owned())
}

fn float_literal(x: f64) -> String {
    if x.is_nan() {
        "BINARY_DOUBLE_NAN".into()
    } else if x.is_infinite() {
        if x > 0.0 {
            "BINARY_DOUBLE_INFINITY".into()
        } else {
            "-BINARY_DOUBLE_INFINITY".into()
        }
    } else {
        // Shortest representation that round-trips.
        format!("{x:e}d")
    }
}

fn year_text(year: i32) -> String {
    if year < 0 {
        format!("-{:04}", -year)
    } else {
        format!("{year:04}")
    }
}

fn date_text(d: NaiveDate) -> String {
    format!("{}-{:02}-{:02}", year_text(d.year()), d.month(), d.day())
}

fn timestamp_text(t: NaiveDateTime) -> String {
    format!(
        "{} {:02}:{:02}:{:02}.{:09}",
        date_text(t.date()),
        t.hour(),
        t.minute(),
        t.second(),
        t.nanosecond().min(999_999_999)
    )
}

/// A typed SQL expression for a value that fits in one literal; shared by
/// the PL/SQL and SQL renderings. `None` for text and bytes.
fn scalar_expr(v: &Value) -> Result<Option<(String, &'static str)>> {
    Ok(Some(match v {
        Value::Null => ("NULL".into(), "VARCHAR2(1)"),
        Value::Bool(b) => ((if *b { "1" } else { "0" }).into(), "NUMBER"),
        Value::Int(i) => (i.to_string(), "NUMBER"),
        Value::Float(x) => (float_literal(*x), "BINARY_DOUBLE"),
        Value::Decimal(s) => (decimal_literal(s)?, "NUMBER"),
        Value::Date(d) => (
            format!("TO_DATE('{}', 'SYYYY-MM-DD')", date_text(*d)),
            "DATE",
        ),
        Value::Timestamp(t) => (
            format!(
                "TO_TIMESTAMP('{}', 'SYYYY-MM-DD HH24:MI:SS.FF9')",
                timestamp_text(*t)
            ),
            "TIMESTAMP(9)",
        ),
        Value::TimestampTz(t) => (
            format!(
                "TO_TIMESTAMP_TZ('{} {}', 'SYYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM')",
                timestamp_text(t.naive_local()),
                t.format("%:z")
            ),
            "TIMESTAMP(9) WITH TIME ZONE",
        ),
        Value::Time(_) => {
            return Err(Error::unsupported(
                "Oracle has no TIME type; bind a timestamp or text instead",
            ));
        }
        Value::Array(_) => {
            return Err(Error::unsupported(
                "array bind values are not supported by the thin driver",
            ));
        }
        Value::Truncated { .. } => {
            return Err(Error::unsupported("a truncated value cannot be bound"));
        }
        Value::Text(_)
        | Value::Bytes(_)
        | Value::Interval(_)
        | Value::Uuid(_)
        | Value::Json(_)
        | Value::Other { .. } => return Ok(None),
    }))
}

/// Text form of values that bind as strings.
fn text_of(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::Interval(s) | Value::Uuid(s) | Value::Json(s) => Some(s),
        Value::Other { text, .. } => Some(text),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

/// The PL/SQL variable for bind value `v`, named `var`.
pub fn plsql_bind(var: &str, v: &Value) -> Result<PlsqlBind> {
    if let Some((expr, ty)) = scalar_expr(v)? {
        return Ok(PlsqlBind::simple(format!("{ty} := {expr}")));
    }
    if let Some(s) = text_of(v) {
        let escaped_len: usize = s.len() + s.matches('\'').count();
        if escaped_len <= PLSQL_PIECE {
            return Ok(PlsqlBind::simple(format!("VARCHAR2(32767) := {}", quote(s))));
        }
        let mut init = vec![format!("DBMS_LOB.CREATETEMPORARY({var}, TRUE);")];
        for p in pieces(s, PLSQL_PIECE) {
            init.push(format!(
                "DBMS_LOB.WRITEAPPEND({var}, LENGTH({q}), {q});",
                q = quote(p)
            ));
        }
        return Ok(PlsqlBind {
            decl: "CLOB".into(),
            init,
            bind_proc: "BIND_VARIABLE",
        });
    }
    if let Value::Bytes(b) = v {
        const RAW_CHUNK: usize = PLSQL_PIECE / 2;
        if b.len() <= RAW_CHUNK {
            return Ok(PlsqlBind {
                decl: format!("RAW(32767) := HEXTORAW('{}')", hex(b)),
                init: Vec::new(),
                bind_proc: "BIND_VARIABLE_RAW",
            });
        }
        let mut init = vec![format!("DBMS_LOB.CREATETEMPORARY({var}, TRUE);")];
        for chunk in b.chunks(RAW_CHUNK) {
            init.push(format!(
                "DBMS_LOB.WRITEAPPEND({var}, {}, HEXTORAW('{}'));",
                chunk.len(),
                hex(chunk)
            ));
        }
        return Ok(PlsqlBind {
            decl: "BLOB".into(),
            init,
            bind_proc: "BIND_VARIABLE",
        });
    }
    Err(Error::unsupported("this bind value type is not supported"))
}

/// A SQL literal for bind value `v` (LOB fallback path, which substitutes
/// values into the statement text).
pub fn sql_literal(v: &Value) -> Result<String> {
    if let Some((expr, ty)) = scalar_expr(v)? {
        return Ok(match v {
            Value::Null => "NULL".into(),
            Value::Int(_) | Value::Decimal(_) | Value::Bool(_) | Value::Float(_) => {
                format!("({expr})")
            }
            _ => {
                let _ = ty;
                expr
            }
        });
    }
    if let Some(s) = text_of(v) {
        let escaped_len: usize = s.len() + s.matches('\'').count();
        if escaped_len <= SQL_PIECE {
            return Ok(quote(s));
        }
        return Ok(format!(
            "({})",
            pieces(s, SQL_PIECE)
                .into_iter()
                .map(|p| format!("TO_CLOB({})", quote(p)))
                .collect::<Vec<_>>()
                .join(" || ")
        ));
    }
    if let Value::Bytes(b) = v {
        if b.len() <= 2000 {
            return Ok(format!("HEXTORAW('{}')", hex(b)));
        }
        return Err(Error::unsupported(
            "binary bind values longer than 2000 bytes are not supported in queries that return LOB columns",
        ));
    }
    Err(Error::unsupported("this bind value type is not supported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, NaiveDate};

    #[test]
    fn quotes_strings() {
        assert_eq!(quote("it's"), "'it''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a\0b"), "'a' || CHR(0) || 'b'");
    }

    #[test]
    fn splits_pieces_on_char_boundaries() {
        let s = "aé'b".repeat(10);
        let p = pieces(&s, 7);
        assert_eq!(p.concat(), s);
        for piece in &p {
            assert!(piece.len() + piece.matches('\'').count() <= 7);
        }
        assert_eq!(pieces("", 10), vec![""]);
    }

    #[test]
    fn validates_decimals() {
        for ok in ["1", "-12.50", "+3", ".5", "5.", "1e10", "-1.5E-7"] {
            assert!(decimal_literal(ok).is_ok(), "{ok}");
        }
        assert_eq!(decimal_literal("+3").unwrap(), "3");
        for bad in ["", "-", ".", "1e", "1.2.3", "1; DROP TABLE t", "0x10", "1e+"] {
            assert!(decimal_literal(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn renders_plsql_binds() {
        assert_eq!(plsql_bind("b1", &Value::Int(-5)).unwrap().decl, "NUMBER := -5");
        assert_eq!(plsql_bind("b1", &Value::Null).unwrap().decl, "VARCHAR2(1) := NULL");
        assert_eq!(plsql_bind("b1", &Value::Bool(true)).unwrap().decl, "NUMBER := 1");
        assert_eq!(plsql_bind("b1", &Value::Float(1.5)).unwrap().decl, "BINARY_DOUBLE := 1.5e0d");
        assert_eq!(
            plsql_bind("b1", &Value::Float(f64::NEG_INFINITY)).unwrap().decl,
            "BINARY_DOUBLE := -BINARY_DOUBLE_INFINITY"
        );
        assert_eq!(
            plsql_bind("b1", &Value::Text("o'k".into())).unwrap().decl,
            "VARCHAR2(32767) := 'o''k'"
        );
        let raw = plsql_bind("b1", &Value::Bytes(vec![0xde, 0xad])).unwrap();
        assert_eq!(raw.decl, "RAW(32767) := HEXTORAW('DEAD')");
        assert_eq!(raw.bind_proc, "BIND_VARIABLE_RAW");
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(
            plsql_bind("b1", &Value::Date(d)).unwrap().decl,
            "DATE := TO_DATE('2026-01-02', 'SYYYY-MM-DD')"
        );
        let ts = d.and_hms_nano_opt(3, 4, 5, 123_456_789).unwrap();
        assert!(plsql_bind("b1", &Value::Timestamp(ts))
            .unwrap()
            .decl
            .contains("'2026-01-02 03:04:05.123456789'"));
        let tz = DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5+03:00").unwrap();
        assert!(plsql_bind("b1", &Value::TimestampTz(tz))
            .unwrap()
            .decl
            .contains("'2026-01-02 03:04:05.500000000 +03:00'"));
        assert!(plsql_bind("b1", &Value::Decimal("1;x".into())).is_err());
        assert!(plsql_bind("b1", &Value::Array(vec![])).is_err());
    }

    #[test]
    fn long_values_become_lobs() {
        let long = "x'".repeat(20_000);
        let b = plsql_bind("b7", &Value::Text(long)).unwrap();
        assert_eq!(b.decl, "CLOB");
        assert!(b.init[0].contains("CREATETEMPORARY(b7"));
        assert!(b.init.len() >= 3);
        let blob = plsql_bind("b2", &Value::Bytes(vec![1; 40_000])).unwrap();
        assert_eq!(blob.decl, "BLOB");
        assert_eq!(blob.bind_proc, "BIND_VARIABLE");
    }

    #[test]
    fn renders_sql_literals() {
        assert_eq!(sql_literal(&Value::Int(-5)).unwrap(), "(-5)");
        assert_eq!(sql_literal(&Value::Text("a'b".into())).unwrap(), "'a''b'");
        assert_eq!(sql_literal(&Value::Null).unwrap(), "NULL");
        assert_eq!(sql_literal(&Value::Bytes(vec![1])).unwrap(), "HEXTORAW('01')");
        assert!(sql_literal(&Value::Text("y".repeat(5000))).unwrap().starts_with("(TO_CLOB("));
        assert!(sql_literal(&Value::Bytes(vec![0; 3000])).is_err());
    }
}
