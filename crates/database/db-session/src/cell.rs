//! The whole value of one cell (ADR 0021, "Paged results": large values
//! arrive as `Value::Truncated`; the grid loads the full value on request by
//! a keyed query): `SELECT t.<column> FROM <table> t WHERE <key> = ...`,
//! run on a data connection without the size limit of a page.
//!
//! The key is the row's primary key (or, for a table without one, the
//! columns whose values compare reliably). Its values are written into the
//! statement as literals of the dialect, because a driver binds only some
//! types (a uuid, a timestamp or a date does not bind everywhere), never
//! text the user typed.

use std::sync::Arc;

use db_core::{Dialect, Error, Value};
use serde::Serialize;
use tokio::sync::watch;

use crate::data_source::Shared;
use crate::job::syntax_dialect;
use crate::pager::{PageError, TableRef};

/// Longest value a keyed read returns whole (bytes); a longer one stays
/// truncated.
pub const FULL_VALUE_LIMIT: usize = 256 << 20;

/// Which value to read: a column of the row of `table` that `key` names.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellQuery {
    pub table: TableRef,
    pub column: String,
    /// Column and value pairs that find the row (`NULL` compares with
    /// `IS NULL`).
    pub key: Vec<(String, Value)>,
}

impl CellQuery {
    /// The statement for `dialect`; `None` when there is no key or a key
    /// value has no literal (JSON, arrays, truncated values).
    pub fn sql(&self, dialect: Dialect) -> Option<String> {
        if self.key.is_empty() {
            return None;
        }
        let d = syntax_dialect(dialect);
        let mut conditions = Vec::new();
        for (name, value) in &self.key {
            let column = format!("t.{}", d.quote(name));
            conditions.push(match value {
                Value::Null => format!("{column} IS NULL"),
                other => format!("{column} = {}", literal(dialect, other)?),
            });
        }
        Some(format!(
            "SELECT t.{}\nFROM {} t\nWHERE {}",
            d.quote(&self.column),
            self.table.qualified(dialect),
            conditions.join("\n  AND ")
        ))
    }
}

/// Whether a value can be part of a key ([`literal`] has a form for it).
pub fn is_keyable(value: &Value) -> bool {
    matches!(
        value,
        Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Decimal(_)
            | Value::Text(_)
            | Value::Bytes(_)
            | Value::Date(_)
            | Value::Time(_)
            | Value::Timestamp(_)
            | Value::TimestampTz(_)
            | Value::Uuid(_)
    )
}

fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// `value` as a literal of `dialect`; `None` for what is no key.
pub fn literal(dialect: Dialect, value: &Value) -> Option<String> {
    Some(match value {
        Value::Bool(b) => match dialect {
            Dialect::Postgres => b.to_string(),
            _ => u8::from(*b).to_string(),
        },
        Value::Int(n) => n.to_string(),
        Value::Decimal(s) => {
            let plain = s
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'));
            if !plain || s.is_empty() {
                return None;
            }
            s.clone()
        }
        Value::Text(s) | Value::Uuid(s) => quoted(s),
        Value::Bytes(b) => match dialect {
            Dialect::Postgres => format!("'\\x{}'::bytea", hex(b)),
            Dialect::Oracle => format!("HEXTORAW('{}')", hex(b)),
            Dialect::Sqlite => format!("X'{}'", hex(b)),
        },
        Value::Date(_) => match dialect {
            Dialect::Oracle => format!("DATE {}", quoted(&value.to_string())),
            _ => quoted(&value.to_string()),
        },
        Value::Timestamp(_) | Value::TimestampTz(_) => match dialect {
            Dialect::Oracle => format!("TIMESTAMP {}", quoted(&value.to_string())),
            _ => quoted(&value.to_string()),
        },
        Value::Time(_) => quoted(&value.to_string()),
        _ => return None,
    })
}

impl Shared {
    pub(crate) async fn run_cell(
        self: &Arc<Self>,
        query: &CellQuery,
        rx: watch::Receiver<bool>,
    ) -> Result<Value, PageError> {
        // Whether there is a statement does not depend on the dialect.
        if query.sql(Dialect::Postgres).is_none() {
            return Err(PageError {
                error: Error::internal("the row has no key to find it by".to_owned()),
                sql: String::new(),
            });
        }
        let (_, rows, sql) = self
            .run_select(
                |d| query.sql(d).unwrap_or_default(),
                2,
                Some(FULL_VALUE_LIMIT),
                rx,
            )
            .await?;
        match rows.as_slice() {
            [row] => Ok(row.first().cloned().unwrap_or(Value::Null)),
            [] => Err(PageError {
                error: Error::internal("the row is not there any more".to_owned()),
                sql,
            }),
            _ => Err(PageError {
                error: Error::internal("the key finds more than one row".to_owned()),
                sql,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(key: Vec<(&str, Value)>) -> CellQuery {
        CellQuery {
            table: TableRef::new(Some("demo"), "events"),
            column: "message".into(),
            key: key.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        }
    }

    #[test]
    fn the_statement_finds_the_row_by_its_key() {
        let q = query(vec![("id", Value::Int(7))]);
        assert_eq!(
            q.sql(Dialect::Postgres).unwrap(),
            "SELECT t.message\nFROM demo.events t\nWHERE t.id = 7"
        );
        let q = query(vec![
            ("Id", Value::Text("it's".into())),
            (
                "uid",
                Value::Uuid("1f15485f-4b8a-bb91-e4cc-16e720e02c1e".into()),
            ),
            ("gone", Value::Null),
        ]);
        assert_eq!(
            q.sql(Dialect::Postgres).unwrap(),
            "SELECT t.message\nFROM demo.events t\nWHERE t.\"Id\" = 'it''s'\n  AND t.uid = '1f15485f-4b8a-bb91-e4cc-16e720e02c1e'\n  AND t.gone IS NULL"
        );
    }

    #[test]
    fn a_key_needs_literals() {
        assert_eq!(query(vec![]).sql(Dialect::Postgres), None);
        let json = query(vec![("doc", Value::Json("{}".into()))]);
        assert_eq!(json.sql(Dialect::Postgres), None);
        assert_eq!(
            literal(Dialect::Postgres, &Value::Decimal("1; drop".into())),
            None
        );
        assert_eq!(
            literal(Dialect::Oracle, &Value::Bytes(vec![1, 0xAB])).unwrap(),
            "HEXTORAW('01AB')"
        );
        assert_eq!(
            literal(Dialect::Sqlite, &Value::Bytes(vec![1, 0xAB])).unwrap(),
            "X'01AB'"
        );
        assert!(is_keyable(&Value::Int(1)));
        assert!(!is_keyable(&Value::Json(String::new())));
    }
}
