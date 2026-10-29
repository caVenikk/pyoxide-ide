//! Values as SQL literals and as plain text, per dialect.

use std::borrow::Cow;

use db_core::{Dialect, Value};

/// The text of a value in the tabular formats: what the display form
/// shows, a cut value without its mark (exports read values whole).
pub fn value_text(value: &Value) -> Cow<'_, str> {
    match value {
        Value::Text(s)
        | Value::Decimal(s)
        | Value::Json(s)
        | Value::Uuid(s)
        | Value::Interval(s) => Cow::Borrowed(s),
        Value::Other { text, .. } => Cow::Borrowed(text),
        Value::Truncated { prefix, .. } => value_text(prefix),
        other => Cow::Owned(other.to_string()),
    }
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

fn is_number(text: &str) -> bool {
    let t = text.strip_prefix(['-', '+']).unwrap_or(text);
    !t.is_empty()
        && t.chars().filter(|c| *c == '.').count() <= 1
        && t.chars().all(|c| c.is_ascii_digit() || c == '.')
        && t.chars().any(|c| c.is_ascii_digit())
}

/// The literal of a value in `dialect`'s SQL.
pub fn sql_literal(value: &Value, dialect: Dialect) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Bool(b) => match dialect {
            Dialect::Postgres => if *b { "TRUE" } else { "FALSE" }.into(),
            Dialect::Oracle | Dialect::Sqlite => if *b { "1" } else { "0" }.into(),
        },
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.is_finite() => f.to_string(),
        Value::Float(f) => quote(&f.to_string()),
        Value::Decimal(s) if is_number(s) => s.clone(),
        Value::Bytes(b) => match dialect {
            Dialect::Postgres => format!("'\\x{}'", hex(b)),
            Dialect::Sqlite => format!("X'{}'", hex(b)),
            Dialect::Oracle => format!("HEXTORAW('{}')", hex(b)),
        },
        Value::Date(_) if dialect == Dialect::Oracle => {
            format!("DATE {}", quote(&value_text(value)))
        }
        Value::Timestamp(_) | Value::TimestampTz(_) if dialect == Dialect::Oracle => {
            format!("TIMESTAMP {}", quote(&value_text(value)))
        }
        Value::Truncated { prefix, .. } => sql_literal(prefix, dialect),
        other => quote(&value_text(other)),
    }
}

/// `name = literal`, `name IS NULL` for NULL.
pub fn sql_condition(name: &str, value: &Value, dialect: Dialect) -> String {
    match value {
        Value::Null => format!("{name} IS NULL"),
        v => format!("{name} = {}", sql_literal(v, dialect)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn text_is_quoted_and_escaped() {
        assert_eq!(
            sql_literal(&Value::Text("it's, \"x\"\nz".into()), Dialect::Postgres),
            "'it''s, \"x\"\nz'"
        );
        assert_eq!(sql_literal(&Value::Null, Dialect::Oracle), "NULL");
        assert_eq!(
            sql_literal(&Value::Text("héllo 日本".into()), Dialect::Sqlite),
            "'héllo 日本'"
        );
    }

    #[test]
    fn engines_differ_in_booleans_bytes_and_dates() {
        let d = Value::Date(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap());
        assert_eq!(sql_literal(&d, Dialect::Postgres), "'2026-09-01'");
        assert_eq!(sql_literal(&d, Dialect::Oracle), "DATE '2026-09-01'");
        let b = Value::Bytes(vec![0xde, 0xad]);
        assert_eq!(sql_literal(&b, Dialect::Postgres), "'\\xDEAD'");
        assert_eq!(sql_literal(&b, Dialect::Sqlite), "X'DEAD'");
        assert_eq!(sql_literal(&b, Dialect::Oracle), "HEXTORAW('DEAD')");
        assert_eq!(sql_literal(&Value::Bool(true), Dialect::Postgres), "TRUE");
        assert_eq!(sql_literal(&Value::Bool(true), Dialect::Oracle), "1");
    }

    #[test]
    fn decimals_that_are_not_numbers_are_quoted() {
        assert_eq!(
            sql_literal(&Value::Decimal("-12.50".into()), Dialect::Postgres),
            "-12.50"
        );
        assert_eq!(
            sql_literal(&Value::Decimal("NaN".into()), Dialect::Postgres),
            "'NaN'"
        );
    }

    #[test]
    fn nulls_compare_with_is() {
        assert_eq!(
            sql_condition("a", &Value::Null, Dialect::Postgres),
            "a IS NULL"
        );
        assert_eq!(
            sql_condition("a", &Value::Int(1), Dialect::Postgres),
            "a = 1"
        );
    }
}
