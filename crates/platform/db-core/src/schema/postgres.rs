//! PostgreSQL introspection over pg_catalog, written from the public PostgreSQL documentation (System Catalogs and System Information Functions). Needs PostgreSQL 11 or later.
//!
//! Every name is passed as a bound parameter. Overloaded functions share one
//! name: `objects` lists each overload, `source` returns all of them.

mod load;

use async_trait::async_trait;

use super::change::{self, SchemaStamps, Stamper, Units};
use super::{
    Cells, ChangeToken, ColumnInfo, ConstraintInfo, ConstraintKind, Delta, IndexInfo, Introspector,
    Level, ObjectInfo, ObjectKind, ObjectRef, SchemaDelta, SchemaInfo, SchemaSnapshot, Scope,
    Snapshot, fetch_all, text, wants,
};
use crate::{Connection, Dialect, Result, Value};

pub struct PostgresIntrospector<'a> {
    conn: &'a dyn Connection,
}

impl<'a> PostgresIntrospector<'a> {
    pub fn new(conn: &'a dyn Connection) -> Self {
        Self { conn }
    }
}

const CURRENT_SCHEMA: &str = "SELECT current_schema()";

const SCHEMAS: &str = "\
SELECT n.nspname, \
       (n.nspname = 'information_schema' OR n.nspname LIKE 'pg\\_%') AS is_system, \
       (n.nspname = current_schema()) AS is_current \
FROM pg_catalog.pg_namespace n \
ORDER BY n.nspname";

/// Relations: tables, views, materialized views, sequences and indexes.
/// The parent is the table of an index.
const RELATIONS: &str = "\
SELECT c.relname, c.relkind::text, d.description, pt.relname \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = c.oid AND d.classoid = 'pg_catalog.pg_class'::regclass AND d.objsubid = 0 \
LEFT JOIN pg_catalog.pg_index i ON i.indexrelid = c.oid \
LEFT JOIN pg_catalog.pg_class pt ON pt.oid = i.indrelid \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm', 'S', 'i', 'I') \
ORDER BY c.relname";

const TRIGGERS: &str = "\
SELECT t.tgname, c.relname, d.description \
FROM pg_catalog.pg_trigger t \
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = t.oid AND d.classoid = 'pg_catalog.pg_trigger'::regclass AND d.objsubid = 0 \
WHERE n.nspname = $1 AND NOT t.tgisinternal \
ORDER BY t.tgname, c.relname";

/// Functions, procedures and window functions; aggregates are skipped.
const ROUTINES: &str = "\
SELECT p.proname, p.prokind::text, d.description \
FROM pg_catalog.pg_proc p \
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = p.oid AND d.classoid = 'pg_catalog.pg_proc'::regclass AND d.objsubid = 0 \
WHERE n.nspname = $1 AND p.prokind IN ('f', 'p', 'w') \
ORDER BY p.proname, p.oid";

/// Enums, composite types that are not table row types, domains and ranges.
const TYPES: &str = "\
SELECT t.typname, d.description \
FROM pg_catalog.pg_type t \
JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = t.oid AND d.classoid = 'pg_catalog.pg_type'::regclass AND d.objsubid = 0 \
WHERE n.nspname = $1 \
  AND (t.typtype IN ('e', 'd', 'r') \
       OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class rc \
                                       WHERE rc.oid = t.typrelid AND rc.relkind = 'c'))) \
ORDER BY t.typname";

const COLUMNS: &str = "\
SELECT a.attname, a.attnum::int4, pg_catalog.format_type(a.atttypid, a.atttypmod), a.attnotnull, \
       pg_catalog.pg_get_expr(ad.adbin, ad.adrelid), a.attidentity::text, d.description \
FROM pg_catalog.pg_attribute a \
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = a.attrelid AND d.classoid = 'pg_catalog.pg_class'::regclass \
      AND d.objsubid = a.attnum \
WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
ORDER BY a.attnum";

/// Key columns only (INCLUDE columns are left out). `pg_get_indexdef` with a
/// column number returns the column name or the indexed expression.
const INDEXES: &str = "\
SELECT ic.relname, i.indisunique, am.amname, \
       ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid, k + 1, true) \
             FROM generate_series(0, i.indnkeyatts - 1) k ORDER BY k) \
FROM pg_catalog.pg_index i \
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
JOIN pg_catalog.pg_class tc ON tc.oid = i.indrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
WHERE n.nspname = $1 AND tc.relname = $2 \
ORDER BY ic.relname";

const CONSTRAINTS: &str = "\
SELECT con.conname, con.contype::text, \
       ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       fn.nspname, fc.relname, \
       ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       con.confdeltype::text, \
       CASE WHEN con.contype = 'c' THEN pg_catalog.pg_get_expr(con.conbin, con.conrelid, true) END \
FROM pg_catalog.pg_constraint con \
JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid \
LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2 AND con.contype IN ('p', 'u', 'f', 'c') \
ORDER BY CASE con.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'f' THEN 2 ELSE 3 END, con.conname";

const VIEW_SOURCE: &str = "\
SELECT c.relkind::text, pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname), \
       pg_catalog.pg_get_viewdef(c.oid, true) \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('v', 'm')";

const ROUTINE_SOURCE: &str = "\
SELECT pg_catalog.pg_get_functiondef(p.oid) \
FROM pg_catalog.pg_proc p \
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
WHERE n.nspname = $1 AND p.proname = $2 AND p.prokind::text = ANY ($3::text[]) \
ORDER BY p.oid";

const TRIGGER_SOURCE: &str = "\
SELECT pg_catalog.pg_get_triggerdef(t.oid, true) \
FROM pg_catalog.pg_trigger t \
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND t.tgname = $2 AND NOT t.tgisinternal \
ORDER BY c.relname";

const INDEX_SOURCE: &str = "\
SELECT pg_catalog.pg_get_indexdef(i.indexrelid, 0, true) \
FROM pg_catalog.pg_index i \
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = ic.relnamespace \
WHERE n.nspname = $1 AND ic.relname = $2";

/// A `text[]` literal of `names`, to bind where `$n::text[]` is written.
fn array_literal(names: &[String]) -> String {
    let mut out = String::from("{");
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        for c in name.chars() {
            if c == '"' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
    }
    out.push('}');
    out
}

fn flag(v: &Value) -> bool {
    matches!(v, Value::Bool(true))
}

/// The elements of an array cell as text; empty for NULL.
fn text_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().map(text).collect(),
        _ => Vec::new(),
    }
}

fn relation_kind(code: &str) -> Option<ObjectKind> {
    Some(match code {
        "r" | "p" | "f" => ObjectKind::Table,
        "v" => ObjectKind::View,
        "m" => ObjectKind::MaterializedView,
        "S" => ObjectKind::Sequence,
        "i" | "I" => ObjectKind::Index,
        _ => return None,
    })
}

fn on_delete_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "a" => "NO ACTION",
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => return None,
    })
}

#[async_trait]
impl Introspector for PostgresIntrospector<'_> {
    async fn current_schema(&self) -> Result<String> {
        let rows = fetch_all(self.conn, "current_schema", CURRENT_SCHEMA, &[]).await?;
        rows.first()
            .map_or_else(|| Ok(String::new()), |r| r.text(0))
    }

    async fn schemas(&self) -> Result<Vec<SchemaInfo>> {
        let rows = fetch_all(self.conn, "schemas", SCHEMAS, &[]).await?;
        rows.iter()
            .map(|r| {
                Ok(SchemaInfo {
                    name: r.text(0)?,
                    is_system: flag(r.value(1)?),
                    is_current: flag(r.value(2)?),
                    ..SchemaInfo::default()
                })
            })
            .collect()
    }

    async fn objects(&self, schema: &str, kinds: &[ObjectKind]) -> Result<Vec<ObjectInfo>> {
        let schema_param = [Value::from(schema)];
        let mut out = Vec::new();
        let object = |name: String, kind, parent, comment| ObjectInfo {
            schema: schema.to_owned(),
            name,
            kind,
            parent,
            comment,
            ..ObjectInfo::default()
        };

        let wants_relations = [
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Sequence,
            ObjectKind::Index,
        ]
        .iter()
        .any(|k| wants(kinds, *k));
        if wants_relations {
            for r in fetch_all(self.conn, "relations", RELATIONS, &schema_param).await? {
                let Some(kind) = relation_kind(&r.text(1)?) else {
                    continue;
                };
                if !wants(kinds, kind) {
                    continue;
                }
                let parent = if kind == ObjectKind::Index {
                    r.opt_text(3)?
                } else {
                    None
                };
                out.push(object(r.text(0)?, kind, parent, r.opt_text(2)?));
            }
        }

        if wants(kinds, ObjectKind::Trigger) {
            for r in fetch_all(self.conn, "triggers", TRIGGERS, &schema_param).await? {
                out.push(object(
                    r.text(0)?,
                    ObjectKind::Trigger,
                    r.opt_text(1)?,
                    r.opt_text(2)?,
                ));
            }
        }

        if wants(kinds, ObjectKind::Function) || wants(kinds, ObjectKind::Procedure) {
            for r in fetch_all(self.conn, "routines", ROUTINES, &schema_param).await? {
                let kind = if r.text(1)? == "p" {
                    ObjectKind::Procedure
                } else {
                    ObjectKind::Function
                };
                if wants(kinds, kind) {
                    out.push(object(r.text(0)?, kind, None, r.opt_text(2)?));
                }
            }
        }

        if wants(kinds, ObjectKind::Type) {
            for r in fetch_all(self.conn, "types", TYPES, &schema_param).await? {
                out.push(object(r.text(0)?, ObjectKind::Type, None, r.opt_text(1)?));
            }
        }
        Ok(out)
    }

    async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows = fetch_all(
            self.conn,
            "columns",
            COLUMNS,
            &[schema.into(), table.into()],
        )
        .await?;
        rows.iter()
            .map(|r| {
                let default = r.opt_text(4)?;
                let identity = matches!(r.text(5)?.as_str(), "a" | "d");
                let serial = default
                    .as_deref()
                    .is_some_and(|d| d.starts_with("nextval("));
                Ok(ColumnInfo {
                    name: r.text(0)?,
                    position: r.int(1)?.unwrap_or(0) as u32,
                    data_type: r.text(2)?,
                    nullable: !flag(r.value(3)?),
                    default,
                    auto_increment: identity || serial,
                    comment: r.opt_text(6)?,
                    ..ColumnInfo::default()
                })
            })
            .collect()
    }

    async fn indexes(&self, schema: &str, table: &str) -> Result<Vec<IndexInfo>> {
        let rows = fetch_all(
            self.conn,
            "indexes",
            INDEXES,
            &[schema.into(), table.into()],
        )
        .await?;
        rows.iter()
            .map(|r| {
                Ok(IndexInfo {
                    name: r.text(0)?,
                    unique: flag(r.value(1)?),
                    columns: text_list(r.value(3)?),
                    method: r.opt_text(2)?,
                    ..IndexInfo::default()
                })
            })
            .collect()
    }

    async fn constraints(&self, schema: &str, table: &str) -> Result<Vec<ConstraintInfo>> {
        let rows = fetch_all(
            self.conn,
            "constraints",
            CONSTRAINTS,
            &[schema.into(), table.into()],
        )
        .await?;
        let mut out = Vec::new();
        for r in &rows {
            let kind = match r.text(1)?.as_str() {
                "p" => ConstraintKind::PrimaryKey,
                "u" => ConstraintKind::Unique,
                "f" => ConstraintKind::ForeignKey,
                "c" => ConstraintKind::Check,
                _ => continue,
            };
            let is_fk = kind == ConstraintKind::ForeignKey;
            out.push(ConstraintInfo {
                name: r.text(0)?,
                kind,
                columns: text_list(r.value(2)?),
                ref_schema: if is_fk { r.opt_text(3)? } else { None },
                ref_table: if is_fk { r.opt_text(4)? } else { None },
                ref_columns: if is_fk {
                    text_list(r.value(5)?)
                } else {
                    Vec::new()
                },
                on_delete: if is_fk {
                    on_delete_action(&r.text(6)?).map(str::to_owned)
                } else {
                    None
                },
                check: r.opt_text(7)?,
                ..ConstraintInfo::default()
            });
        }
        Ok(out)
    }

    async fn source(&self, schema: &str, name: &str, kind: ObjectKind) -> Result<Option<String>> {
        let args = [Value::from(schema), Value::from(name)];
        match kind {
            ObjectKind::View | ObjectKind::MaterializedView => {
                let rows = fetch_all(self.conn, "view_source", VIEW_SOURCE, &args).await?;
                rows.first()
                    .map(|r| {
                        let create = if r.text(0)? == "m" {
                            "CREATE MATERIALIZED VIEW"
                        } else {
                            "CREATE OR REPLACE VIEW"
                        };
                        Ok(format!("{create} {} AS\n{}", r.text(1)?, r.text(2)?))
                    })
                    .transpose()
            }
            ObjectKind::Function | ObjectKind::Procedure => {
                let kinds = if kind == ObjectKind::Procedure {
                    "{p}"
                } else {
                    "{f,w}"
                };
                let params = [args[0].clone(), args[1].clone(), Value::from(kinds)];
                let rows = fetch_all(self.conn, "routine_source", ROUTINE_SOURCE, &params).await?;
                join_sources(&rows)
            }
            ObjectKind::Trigger => {
                let rows = fetch_all(self.conn, "trigger_source", TRIGGER_SOURCE, &args).await?;
                join_sources(&rows)
            }
            ObjectKind::Index => {
                let rows = fetch_all(self.conn, "index_source", INDEX_SOURCE, &args).await?;
                join_sources(&rows)
            }
            _ => Ok(None),
        }
    }

    async fn load(&self, scope: &Scope, level: Level) -> Result<Snapshot> {
        change::load(self, scope, level).await
    }

    async fn changes(&self, since: &ChangeToken, scope: &Scope) -> Result<Delta> {
        change::changes(self, since, scope).await
    }

    async fn load_objects(
        &self,
        schema: &str,
        objects: &[ObjectRef],
        level: Level,
    ) -> Result<SchemaDelta> {
        change::load_objects(self, schema, objects, level).await
    }
}

#[async_trait]
impl Stamper for PostgresIntrospector<'_> {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    async fn schema_infos(&self) -> Result<Vec<SchemaInfo>> {
        load::schema_infos(self.conn).await
    }

    async fn stamps(&self, info: &SchemaInfo, level: Level) -> Result<SchemaStamps> {
        load::stamps(self.conn, info, level).await
    }

    async fn read(
        &self,
        info: SchemaInfo,
        level: Level,
        units: Option<&Units>,
    ) -> Result<SchemaSnapshot> {
        load::load_schema_retrying(self.conn, info, level, units).await
    }
}

/// All definitions (one per overload or table) separated by a blank line.
fn join_sources(rows: &[Cells]) -> Result<Option<String>> {
    let mut parts = Vec::new();
    for r in rows {
        parts.extend(r.opt_text(0)?);
    }
    Ok((!parts.is_empty()).then(|| parts.join("\n\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_codes_map_to_kinds() {
        assert_eq!(relation_kind("r"), Some(ObjectKind::Table));
        assert_eq!(relation_kind("p"), Some(ObjectKind::Table));
        assert_eq!(relation_kind("f"), Some(ObjectKind::Table));
        assert_eq!(relation_kind("v"), Some(ObjectKind::View));
        assert_eq!(relation_kind("m"), Some(ObjectKind::MaterializedView));
        assert_eq!(relation_kind("S"), Some(ObjectKind::Sequence));
        assert_eq!(relation_kind("i"), Some(ObjectKind::Index));
        assert_eq!(relation_kind("I"), Some(ObjectKind::Index));
        assert_eq!(relation_kind("c"), None);
    }

    #[test]
    fn array_literals_quote_every_name() {
        assert_eq!(array_literal(&[]), "{}");
        assert_eq!(
            array_literal(&[
                "a".into(),
                "b c".into(),
                "q\"x".into(),
                "back\\slash".into()
            ]),
            r#"{"a","b c","q\"x","back\\slash"}"#
        );
    }

    #[test]
    fn foreign_key_actions() {
        assert_eq!(on_delete_action("c"), Some("CASCADE"));
        assert_eq!(on_delete_action("a"), Some("NO ACTION"));
        assert_eq!(on_delete_action("r"), Some("RESTRICT"));
        assert_eq!(on_delete_action("n"), Some("SET NULL"));
        assert_eq!(on_delete_action("d"), Some("SET DEFAULT"));
        assert_eq!(on_delete_action("x"), None);
    }

    #[test]
    fn text_lists_and_flags() {
        let list = Value::Array(vec![Value::from("a"), Value::from("b c")]);
        assert_eq!(text_list(&list), ["a", "b c"]);
        assert!(text_list(&Value::Null).is_empty());
        assert!(flag(&Value::Bool(true)));
        assert!(!flag(&Value::Bool(false)));
        assert!(!flag(&Value::Null));
    }

    fn cells(row: crate::Row) -> Cells {
        Cells::new("sources", row)
    }

    #[test]
    fn join_sources_skips_empty_rows() {
        assert_eq!(join_sources(&[]).unwrap(), None);
        assert_eq!(join_sources(&[cells(vec![Value::Null])]).unwrap(), None);
        let rows = vec![
            cells(vec![Value::from("one")]),
            cells(vec![Value::from("two")]),
        ];
        assert_eq!(join_sources(&rows).unwrap().as_deref(), Some("one\n\ntwo"));
        assert!(join_sources(&[cells(Vec::new())]).is_err());
    }
}
