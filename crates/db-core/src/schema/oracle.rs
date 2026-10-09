//! Oracle introspection over the ALL_* data dictionary views, written from
//! the public Oracle Database Reference.

use async_trait::async_trait;

use super::{
    ColumnInfo, ConstraintInfo, IndexInfo, Introspector, ObjectInfo, ObjectKind, SchemaInfo,
    fetch_all, opt_text, text, wants,
};
use crate::{Connection, Error, Result, Value};

pub struct OracleIntrospector<'a> {
    conn: &'a dyn Connection,
}

impl<'a> OracleIntrospector<'a> {
    pub fn new(conn: &'a dyn Connection) -> Self {
        Self { conn }
    }
}

const CURRENT_SCHEMA: &str = "SELECT SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') FROM dual";

const TABLES: &str = "\
SELECT t.table_name, c.comments FROM all_tables t \
LEFT JOIN all_tab_comments c ON c.owner = t.owner AND c.table_name = t.table_name \
WHERE t.owner = :1 AND t.nested = 'NO' AND t.secondary = 'N' \
ORDER BY t.table_name";

const COLUMNS: &str = "\
SELECT column_name, column_id, data_type, data_length, data_precision, data_scale, \
char_used, char_length, nullable \
FROM all_tab_columns WHERE owner = :1 AND table_name = :2 ORDER BY column_id";

#[async_trait]
impl Introspector for OracleIntrospector<'_> {
    async fn current_schema(&self) -> Result<String> {
        let rows = fetch_all(self.conn, CURRENT_SCHEMA, &[]).await?;
        Ok(rows.first().map(|r| text(&r[0])).unwrap_or_default())
    }

    async fn schemas(&self) -> Result<Vec<SchemaInfo>> {
        Err(Error::unsupported("oracle schemas: not implemented yet"))
    }

    async fn objects(&self, schema: &str, kinds: &[ObjectKind]) -> Result<Vec<ObjectInfo>> {
        let mut out = Vec::new();
        if wants(kinds, ObjectKind::Table) {
            for r in fetch_all(self.conn, TABLES, &[Value::from(schema)]).await? {
                out.push(ObjectInfo {
                    schema: schema.to_owned(),
                    name: text(&r[0]),
                    kind: ObjectKind::Table,
                    parent: None,
                    comment: opt_text(&r[1]),
                });
            }
        }
        Ok(out)
    }

    async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows = fetch_all(self.conn, COLUMNS, &[schema.into(), table.into()]).await?;
        Ok(rows
            .iter()
            .map(|r| ColumnInfo {
                name: text(&r[0]),
                position: r[1].as_i64().unwrap_or(0) as u32,
                data_type: oracle_type(r),
                nullable: text(&r[8]) != "N",
                default: None,
                auto_increment: false,
                comment: None,
            })
            .collect())
    }

    async fn indexes(&self, _schema: &str, _table: &str) -> Result<Vec<IndexInfo>> {
        Err(Error::unsupported("oracle indexes: not implemented yet"))
    }

    async fn constraints(&self, _schema: &str, _table: &str) -> Result<Vec<ConstraintInfo>> {
        Err(Error::unsupported(
            "oracle constraints: not implemented yet",
        ))
    }

    async fn source(
        &self,
        _schema: &str,
        _name: &str,
        _kind: ObjectKind,
    ) -> Result<Option<String>> {
        Err(Error::unsupported("oracle source: not implemented yet"))
    }
}

/// Full type text from ALL_TAB_COLUMNS columns 2..=7
/// (`data_type, data_length, data_precision, data_scale, char_used, char_length`).
fn oracle_type(r: &[Value]) -> String {
    let ty = text(&r[2]);
    let precision = r[4].as_i64();
    let scale = r[5].as_i64();
    match ty.as_str() {
        "NUMBER" => match (precision, scale) {
            (Some(p), Some(0)) => format!("NUMBER({p})"),
            (Some(p), Some(s)) => format!("NUMBER({p},{s})"),
            (None, Some(0)) => "INTEGER".to_owned(),
            _ => ty,
        },
        "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" => {
            let unit = if text(&r[6]) == "C" { " CHAR" } else { "" };
            let len = r[7].as_i64().or(r[3].as_i64()).unwrap_or(0);
            if ty.starts_with('N') {
                format!("{ty}({len})")
            } else {
                format!("{ty}({len}{unit})")
            }
        }
        "RAW" => format!("RAW({})", r[3].as_i64().unwrap_or(0)),
        _ => ty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ty: &str, len: i64, p: Option<i64>, s: Option<i64>, unit: &str) -> Vec<Value> {
        vec![
            Value::from("C"),
            Value::Int(1),
            Value::from(ty),
            Value::Int(len),
            p.into(),
            s.into(),
            Value::from(unit),
            Value::Int(len),
            Value::from("Y"),
        ]
    }

    #[test]
    fn formats_types() {
        assert_eq!(
            oracle_type(&row("NUMBER", 22, Some(10), Some(2), "")),
            "NUMBER(10,2)"
        );
        assert_eq!(
            oracle_type(&row("NUMBER", 22, None, Some(0), "")),
            "INTEGER"
        );
        assert_eq!(oracle_type(&row("NUMBER", 22, None, None, "")), "NUMBER");
        assert_eq!(
            oracle_type(&row("VARCHAR2", 40, None, None, "C")),
            "VARCHAR2(40 CHAR)"
        );
        assert_eq!(oracle_type(&row("DATE", 7, None, None, "")), "DATE");
    }
}
