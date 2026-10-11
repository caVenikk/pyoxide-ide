//! Oracle introspection over the ALL_* data dictionary views, written from
//! the public Oracle Database Reference. Valid on Oracle 12.2 and later
//! (19c is the oldest supported release): it uses `ALL_USERS.ORACLE_MAINTAINED`
//! (12.1), `ALL_TAB_COLUMNS.IDENTITY_COLUMN` (12.1),
//! `ALL_CONSTRAINTS.SEARCH_CONDITION_VC` (12.1), `ALL_VIEWS.TEXT_VC` (12.1)
//! and `WITH FUNCTION` (12.1).
//!
//! LONG columns (`DATA_DEFAULT`, `COLUMN_EXPRESSION`) are read through small
//! PL/SQL functions declared in the query's `WITH` clause, which copy the
//! value into a VARCHAR2 (cut at 4000 bytes). `DBMS_METADATA.GET_DDL` is
//! called the same way so a missing privilege yields NULL instead of an
//! error. Every bind placeholder appears once, in order, so the statements
//! behave the same with positional (Oracle Client) and named binding.

use async_trait::async_trait;

use super::{
    ColumnInfo, ConstraintInfo, ConstraintKind, IndexInfo, Introspector, ObjectInfo, ObjectKind,
    SchemaInfo, fetch_all, opt_text, text, wants,
};
use crate::{Connection, Result, Value};

pub struct OracleIntrospector<'a> {
    conn: &'a dyn Connection,
}

impl<'a> OracleIntrospector<'a> {
    pub fn new(conn: &'a dyn Connection) -> Self {
        Self { conn }
    }
}

const CURRENT_SCHEMA: &str = "SELECT SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') FROM dual";

const SCHEMAS: &str = "\
SELECT u.username, u.oracle_maintained, \
       CASE WHEN u.username = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') THEN 'Y' ELSE 'N' END \
FROM all_users u \
ORDER BY u.username";

/// One branch of the objects query per kind: `(kinds, SQL with one `{}`
/// placeholder for the owner bind)`. Columns: name, kind code, parent,
/// comment.
const OBJECT_BRANCHES: &[(&[ObjectKind], &str)] = &[
    (
        &[ObjectKind::Table],
        // Nested tables, secondary (domain index) tables, recycle bin
        // entries, IOT overflow segments and materialized view containers
        // are not user tables.
        "SELECT t.table_name, 'TABLE', NULL, c.comments FROM all_tables t \
         LEFT JOIN all_tab_comments c ON c.owner = t.owner AND c.table_name = t.table_name \
              AND c.table_type = 'TABLE' \
         WHERE t.owner = {} AND t.nested = 'NO' AND t.secondary = 'N' AND t.dropped = 'NO' \
           AND (t.iot_type IS NULL OR t.iot_type = 'IOT') \
           AND NOT EXISTS (SELECT 1 FROM all_mviews m \
                           WHERE m.owner = t.owner AND m.container_name = t.table_name)",
    ),
    (
        &[ObjectKind::View],
        "SELECT v.view_name, 'VIEW', NULL, c.comments FROM all_views v \
         LEFT JOIN all_tab_comments c ON c.owner = v.owner AND c.table_name = v.view_name \
              AND c.table_type = 'VIEW' \
         WHERE v.owner = {}",
    ),
    (
        &[ObjectKind::MaterializedView],
        "SELECT m.mview_name, 'MATERIALIZED VIEW', NULL, c.comments FROM all_mviews m \
         LEFT JOIN all_mview_comments c ON c.owner = m.owner AND c.mview_name = m.mview_name \
         WHERE m.owner = {}",
    ),
    (
        &[ObjectKind::Sequence],
        "SELECT s.sequence_name, 'SEQUENCE', NULL, NULL FROM all_sequences s \
         WHERE s.sequence_owner = {}",
    ),
    (
        &[ObjectKind::Index],
        "SELECT i.index_name, 'INDEX', i.table_name, NULL FROM all_indexes i \
         WHERE i.owner = {} AND i.index_type <> 'LOB' AND i.index_name NOT LIKE 'BIN$%'",
    ),
    (
        &[ObjectKind::Trigger],
        "SELECT g.trigger_name, 'TRIGGER', g.table_name, NULL FROM all_triggers g \
         WHERE g.owner = {} AND g.trigger_name NOT LIKE 'BIN$%'",
    ),
];

/// PL/SQL units and synonyms come from ALL_OBJECTS.
fn plsql_object_type(kind: ObjectKind) -> Option<&'static str> {
    Some(match kind {
        ObjectKind::Procedure => "PROCEDURE",
        ObjectKind::Function => "FUNCTION",
        ObjectKind::Package => "PACKAGE",
        ObjectKind::Type => "TYPE",
        ObjectKind::Synonym => "SYNONYM",
        _ => return None,
    })
}

fn object_kind(code: &str) -> Option<ObjectKind> {
    Some(match code.trim() {
        "TABLE" => ObjectKind::Table,
        "VIEW" => ObjectKind::View,
        "MATERIALIZED VIEW" => ObjectKind::MaterializedView,
        "SEQUENCE" => ObjectKind::Sequence,
        "INDEX" => ObjectKind::Index,
        "TRIGGER" => ObjectKind::Trigger,
        "PROCEDURE" => ObjectKind::Procedure,
        "FUNCTION" => ObjectKind::Function,
        "PACKAGE" => ObjectKind::Package,
        "TYPE" => ObjectKind::Type,
        "SYNONYM" => ObjectKind::Synonym,
        _ => return None,
    })
}

/// The UNION ALL of the wanted branches; each branch binds the owner once
/// (`:1`, `:2`, ...). Returns the SQL and the number of binds.
fn objects_sql(kinds: &[ObjectKind]) -> Option<(String, usize)> {
    let mut branches = Vec::new();
    for (branch_kinds, sql) in OBJECT_BRANCHES {
        if branch_kinds.iter().any(|k| wants(kinds, *k)) {
            branches.push(sql.replace("{}", &format!(":{}", branches.len() + 1)));
        }
    }
    let types: Vec<String> = [
        ObjectKind::Procedure,
        ObjectKind::Function,
        ObjectKind::Package,
        ObjectKind::Type,
        ObjectKind::Synonym,
    ]
    .into_iter()
    .filter(|k| wants(kinds, *k))
    .filter_map(plsql_object_type)
    .map(|t| format!("'{t}'"))
    .collect();
    if !types.is_empty() {
        branches.push(format!(
            "SELECT o.object_name, o.object_type, NULL, NULL FROM all_objects o \
             WHERE o.owner = :{} AND o.object_type IN ({}) AND o.generated = 'N' \
               AND o.object_name NOT LIKE 'BIN$%'",
            branches.len() + 1,
            types.join(", ")
        ));
    }
    if branches.is_empty() {
        return None;
    }
    let n = branches.len();
    Some((format!("{} ORDER BY 1, 2", branches.join(" UNION ALL ")), n))
}

/// `DATA_DEFAULT` is LONG: the function copies it into a VARCHAR2.
const COLUMNS: &str = "\
WITH FUNCTION px_default(p_owner VARCHAR2, p_table VARCHAR2, p_column VARCHAR2) RETURN VARCHAR2 IS
  v LONG;
BEGIN
  SELECT data_default INTO v FROM all_tab_columns
   WHERE owner = p_owner AND table_name = p_table AND column_name = p_column;
  RETURN SUBSTRB(v, 1, 4000);
EXCEPTION WHEN OTHERS THEN RETURN NULL;
END;
SELECT c.column_name, c.column_id, c.data_type, c.data_length, c.data_precision, c.data_scale,
       c.char_used, c.char_length, c.nullable,
       CASE WHEN c.default_length > 0 THEN px_default(c.owner, c.table_name, c.column_name) END,
       c.identity_column, cc.comments, c.data_type_owner
FROM all_tab_columns c
LEFT JOIN all_col_comments cc
       ON cc.owner = c.owner AND cc.table_name = c.table_name AND cc.column_name = c.column_name
WHERE c.owner = :1 AND c.table_name = :2
ORDER BY c.column_id";

/// Function-based index expressions (`COLUMN_EXPRESSION`, LONG) through a
/// function; descending columns are function-based too.
const INDEXES: &str = "\
WITH FUNCTION px_expr(p_owner VARCHAR2, p_index VARCHAR2, p_pos NUMBER) RETURN VARCHAR2 IS
  v LONG;
BEGIN
  SELECT column_expression INTO v FROM all_ind_expressions
   WHERE index_owner = p_owner AND index_name = p_index AND column_position = p_pos;
  RETURN SUBSTRB(v, 1, 4000);
EXCEPTION WHEN OTHERS THEN RETURN NULL;
END;
SELECT i.index_name, i.uniqueness, i.index_type, c.column_name, c.descend,
       CASE WHEN i.index_type LIKE 'FUNCTION-BASED%'
            THEN px_expr(c.index_owner, c.index_name, c.column_position) END
FROM all_indexes i
JOIN all_ind_columns c ON c.index_owner = i.owner AND c.index_name = i.index_name
WHERE i.table_owner = :1 AND i.table_name = :2 AND i.index_type <> 'LOB'
ORDER BY i.index_name, c.column_position";

const CONSTRAINTS: &str = "\
SELECT c.constraint_name, c.constraint_type, c.generated, c.search_condition_vc, c.delete_rule, \
       cc.column_name, r.owner, r.table_name, rc.column_name \
FROM all_constraints c \
LEFT JOIN all_cons_columns cc \
       ON cc.owner = c.owner AND cc.constraint_name = c.constraint_name AND cc.table_name = c.table_name \
LEFT JOIN all_constraints r ON r.owner = c.r_owner AND r.constraint_name = c.r_constraint_name \
LEFT JOIN all_cons_columns rc \
       ON rc.owner = r.owner AND rc.constraint_name = r.constraint_name AND rc.position = cc.position \
WHERE c.owner = :1 AND c.table_name = :2 AND c.constraint_type IN ('P', 'U', 'R', 'C') \
ORDER BY CASE c.constraint_type WHEN 'P' THEN 0 WHEN 'U' THEN 1 WHEN 'R' THEN 2 ELSE 3 END, \
         c.constraint_name, cc.position, cc.column_name";

/// Stored PL/SQL source in line order; `{}` is the list of `TYPE` values.
const UNIT_SOURCE: &str = "\
SELECT s.type, s.text FROM all_source s \
WHERE s.owner = :1 AND s.name = :2 AND s.type IN ({}) \
ORDER BY CASE WHEN s.type LIKE '% BODY' THEN 1 ELSE 0 END, s.line";

/// `TEXT_VC` is VARCHAR2(4000); longer views need DBMS_METADATA.
const VIEW_SOURCE: &str = "\
SELECT v.text_length, v.text_vc FROM all_views v WHERE v.owner = :1 AND v.view_name = :2";

/// DDL from DBMS_METADATA, or NULL when it fails (object missing, no
/// privilege for another schema). Segment attributes are left out.
const METADATA_DDL: &str = "\
WITH FUNCTION px_ddl(p_type VARCHAR2, p_name VARCHAR2, p_owner VARCHAR2) RETURN CLOB IS
BEGIN
  DBMS_METADATA.SET_TRANSFORM_PARAM(DBMS_METADATA.SESSION_TRANSFORM, 'SEGMENT_ATTRIBUTES', FALSE);
  RETURN DBMS_METADATA.GET_DDL(p_type, p_name, p_owner);
EXCEPTION WHEN OTHERS THEN RETURN NULL;
END;
SELECT px_ddl(:1, :2, :3) FROM dual";

const SYNONYM_SOURCE: &str = "\
SELECT s.table_owner, s.table_name, s.db_link FROM all_synonyms s \
WHERE s.owner = :1 AND s.synonym_name = :2";

const SEQUENCE_SOURCE: &str = "\
SELECT s.min_value, s.max_value, s.increment_by, s.cycle_flag, s.order_flag, s.cache_size, \
       s.last_number \
FROM all_sequences s WHERE s.sequence_owner = :1 AND s.sequence_name = :2";

#[async_trait]
impl Introspector for OracleIntrospector<'_> {
    async fn current_schema(&self) -> Result<String> {
        let rows = fetch_all(self.conn, CURRENT_SCHEMA, &[]).await?;
        Ok(rows.first().map(|r| text(&r[0])).unwrap_or_default())
    }

    async fn schemas(&self) -> Result<Vec<SchemaInfo>> {
        let rows = fetch_all(self.conn, SCHEMAS, &[]).await?;
        Ok(rows
            .iter()
            .map(|r| SchemaInfo {
                name: text(&r[0]),
                is_system: text(&r[1]) == "Y",
                is_current: text(&r[2]) == "Y",
            })
            .collect())
    }

    async fn objects(&self, schema: &str, kinds: &[ObjectKind]) -> Result<Vec<ObjectInfo>> {
        let Some((sql, binds)) = objects_sql(kinds) else {
            return Ok(Vec::new());
        };
        let params = vec![Value::from(schema); binds];
        let rows = fetch_all(self.conn, &sql, &params).await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let kind = object_kind(&text(&r[1]))?;
                wants(kinds, kind).then(|| ObjectInfo {
                    schema: schema.to_owned(),
                    name: text(&r[0]),
                    kind,
                    parent: opt_text(&r[2]),
                    comment: opt_text(&r[3]),
                })
            })
            .collect())
    }

    async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows = fetch_all(self.conn, COLUMNS, &[schema.into(), table.into()]).await?;
        Ok(rows
            .iter()
            .map(|r| ColumnInfo {
                name: text(&r[0]),
                position: r[1].as_i64().unwrap_or(0) as u32,
                data_type: oracle_type(r, schema),
                nullable: text(&r[8]) != "N",
                default: opt_text(&r[9])
                    .map(|d| d.trim().to_owned())
                    .filter(|d| !d.is_empty()),
                auto_increment: text(&r[10]) == "YES",
                comment: opt_text(&r[11]),
            })
            .collect())
    }

    async fn indexes(&self, schema: &str, table: &str) -> Result<Vec<IndexInfo>> {
        let rows = fetch_all(self.conn, INDEXES, &[schema.into(), table.into()]).await?;
        let mut out: Vec<IndexInfo> = Vec::new();
        for r in &rows {
            let name = text(&r[0]);
            let column = index_column(&text(&r[3]), opt_text(&r[5]), &text(&r[4]));
            match out.last_mut() {
                Some(last) if last.name == name => last.columns.push(column),
                _ => out.push(IndexInfo {
                    name,
                    unique: text(&r[1]) == "UNIQUE",
                    columns: vec![column],
                    method: opt_text(&r[2]),
                }),
            }
        }
        Ok(out)
    }

    async fn constraints(&self, schema: &str, table: &str) -> Result<Vec<ConstraintInfo>> {
        let rows = fetch_all(self.conn, CONSTRAINTS, &[schema.into(), table.into()]).await?;
        let mut out: Vec<ConstraintInfo> = Vec::new();
        for r in &rows {
            let name = text(&r[0]);
            let column = opt_text(&r[5]);
            let ref_column = opt_text(&r[8]);
            if let Some(last) = out.last_mut().filter(|c| c.name == name) {
                if let Some(c) = column.filter(|c| !last.columns.contains(c)) {
                    last.columns.push(c);
                }
                if let Some(c) = ref_column {
                    last.ref_columns.push(c);
                }
                continue;
            }
            let kind = match text(&r[1]).as_str() {
                "P" => ConstraintKind::PrimaryKey,
                "U" => ConstraintKind::Unique,
                "R" => ConstraintKind::ForeignKey,
                "C" => ConstraintKind::Check,
                _ => continue,
            };
            let check = opt_text(&r[3]);
            if kind == ConstraintKind::Check
                && text(&r[2]) == "GENERATED NAME"
                && check.as_deref().is_some_and(is_not_null_check)
            {
                continue;
            }
            let is_fk = kind == ConstraintKind::ForeignKey;
            out.push(ConstraintInfo {
                name,
                kind,
                columns: column.into_iter().collect(),
                ref_schema: is_fk.then(|| opt_text(&r[6])).flatten(),
                ref_table: is_fk.then(|| opt_text(&r[7])).flatten(),
                ref_columns: if is_fk {
                    ref_column.into_iter().collect()
                } else {
                    Vec::new()
                },
                on_delete: is_fk.then(|| opt_text(&r[4])).flatten(),
                check: (kind == ConstraintKind::Check).then_some(check).flatten(),
            });
        }
        Ok(out)
    }

    async fn source(&self, schema: &str, name: &str, kind: ObjectKind) -> Result<Option<String>> {
        let args = [Value::from(schema), Value::from(name)];
        match kind {
            ObjectKind::Package
            | ObjectKind::Procedure
            | ObjectKind::Function
            | ObjectKind::Trigger
            | ObjectKind::Type => {
                let types = match kind {
                    ObjectKind::Package => "'PACKAGE', 'PACKAGE BODY'",
                    ObjectKind::Type => "'TYPE', 'TYPE BODY'",
                    ObjectKind::Procedure => "'PROCEDURE'",
                    ObjectKind::Function => "'FUNCTION'",
                    _ => "'TRIGGER'",
                };
                let sql = UNIT_SOURCE.replace("{}", types);
                let rows = fetch_all(self.conn, &sql, &args).await?;
                Ok(join_units(&rows))
            }
            ObjectKind::View => {
                let rows = fetch_all(self.conn, VIEW_SOURCE, &args).await?;
                let Some(r) = rows.first() else {
                    return Ok(None);
                };
                let length = r[0].as_i64().unwrap_or(0);
                if length <= 4000 {
                    return Ok(Some(format!(
                        "CREATE OR REPLACE VIEW {} AS\n{}",
                        qualified(schema, name),
                        text(&r[1]).trim_end()
                    )));
                }
                // TEXT_VC is cut at 4000 characters.
                self.metadata("VIEW", schema, name).await
            }
            ObjectKind::Synonym => {
                let rows = fetch_all(self.conn, SYNONYM_SOURCE, &args).await?;
                Ok(rows.first().map(|r| {
                    let target = match opt_text(&r[0]) {
                        Some(owner) => qualified(&owner, &text(&r[1])),
                        None => quote_ident(&text(&r[1])),
                    };
                    let link = opt_text(&r[2]).map(|l| format!("@{l}")).unwrap_or_default();
                    format!(
                        "CREATE OR REPLACE SYNONYM {} FOR {target}{link}",
                        qualified(schema, name)
                    )
                }))
            }
            ObjectKind::Sequence => {
                if let Some(ddl) = self.metadata("SEQUENCE", schema, name).await? {
                    return Ok(Some(ddl));
                }
                let rows = fetch_all(self.conn, SEQUENCE_SOURCE, &args).await?;
                Ok(rows
                    .first()
                    .map(|r| sequence_ddl(&qualified(schema, name), r)))
            }
            ObjectKind::Table => self.metadata("TABLE", schema, name).await,
            ObjectKind::MaterializedView => self.metadata("MATERIALIZED_VIEW", schema, name).await,
            ObjectKind::Index => self.metadata("INDEX", schema, name).await,
        }
    }
}

impl OracleIntrospector<'_> {
    /// `DBMS_METADATA.GET_DDL`, `None` when the database refuses.
    async fn metadata(
        &self,
        object_type: &str,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>> {
        let rows = fetch_all(
            self.conn,
            METADATA_DDL,
            &[object_type.into(), name.into(), schema.into()],
        )
        .await?;
        Ok(rows.first().and_then(|r| {
            let v = match &r[0] {
                Value::Truncated { prefix, .. } => prefix.as_ref().clone(),
                v => v.clone(),
            };
            opt_text(&v).map(|s| s.trim().to_owned())
        }))
    }
}

/// `"OWNER"."NAME"`.
fn qualified(owner: &str, name: &str) -> String {
    format!("{}.{}", quote_ident(owner), quote_ident(name))
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// `"COL" IS NOT NULL`: the check Oracle creates for a NOT NULL column.
fn is_not_null_check(condition: &str) -> bool {
    let c = condition.trim();
    let Some(rest) = c.strip_prefix('"') else {
        return false;
    };
    let Some(end) = rest.find('"') else {
        return false;
    };
    rest[end + 1..].trim().eq_ignore_ascii_case("IS NOT NULL")
}

/// An index column: the name, or the expression of a function-based index
/// (`"NAME"` for a plain descending column), with ` DESC` when descending.
fn index_column(column: &str, expression: Option<String>, descend: &str) -> String {
    let mut out = match expression {
        Some(e) => {
            let e = e.trim();
            let inner = e.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
            match inner {
                Some(name) if !name.contains('"') => name.to_owned(),
                _ => e.to_owned(),
            }
        }
        None => column.to_owned(),
    };
    if descend.eq_ignore_ascii_case("DESC") {
        out.push_str(" DESC");
    }
    out
}

/// Stored units in order, each as `CREATE OR REPLACE <text>` terminated by
/// `/`; spec before body.
fn join_units(rows: &[Vec<Value>]) -> Option<String> {
    let mut units: Vec<(String, String)> = Vec::new();
    for r in rows {
        let ty = text(&r[0]);
        let line = text(&r[1]);
        match units.last_mut() {
            Some((t, body)) if *t == ty => body.push_str(&line),
            _ => units.push((ty, line)),
        }
    }
    if units.is_empty() {
        return None;
    }
    Some(
        units
            .iter()
            .map(|(_, body)| format!("CREATE OR REPLACE {}\n/", body.trim_end()))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

/// DDL for a sequence from ALL_SEQUENCES (fallback when DBMS_METADATA is
/// not allowed). `LAST_NUMBER` is where the sequence would continue.
fn sequence_ddl(name: &str, r: &[Value]) -> String {
    let cache = r[5].as_i64().unwrap_or(0);
    format!(
        "CREATE SEQUENCE {name} START WITH {} INCREMENT BY {} MINVALUE {} MAXVALUE {} {} {} {}",
        text(&r[6]),
        text(&r[2]),
        text(&r[0]),
        text(&r[1]),
        if cache > 0 {
            format!("CACHE {cache}")
        } else {
            "NOCACHE".into()
        },
        if text(&r[3]) == "Y" {
            "CYCLE"
        } else {
            "NOCYCLE"
        },
        if text(&r[4]) == "Y" {
            "ORDER"
        } else {
            "NOORDER"
        },
    )
}

/// Full type text from ALL_TAB_COLUMNS columns 2..=7
/// (`data_type, data_length, data_precision, data_scale, char_used,
/// char_length`) and column 12 (`data_type_owner`). TIMESTAMP and INTERVAL
/// types already carry their precision in DATA_TYPE.
fn oracle_type(r: &[Value], table_owner: &str) -> String {
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
        "FLOAT" => match precision {
            Some(p) => format!("FLOAT({p})"),
            None => ty,
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
        "RAW" | "UROWID" => format!("{ty}({})", r[3].as_i64().unwrap_or(0)),
        _ => match r.get(12).and_then(opt_text) {
            Some(owner) if owner != table_owner && owner != "SYS" && owner != "PUBLIC" => {
                format!("{owner}.{ty}")
            }
            _ => ty,
        },
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
            Value::Null,
            Value::from("NO"),
            Value::Null,
            Value::Null,
        ]
    }

    #[test]
    fn formats_types() {
        assert_eq!(
            oracle_type(&row("NUMBER", 22, Some(10), Some(2), ""), "S"),
            "NUMBER(10,2)"
        );
        assert_eq!(
            oracle_type(&row("NUMBER", 22, None, Some(0), ""), "S"),
            "INTEGER"
        );
        assert_eq!(
            oracle_type(&row("NUMBER", 22, None, None, ""), "S"),
            "NUMBER"
        );
        assert_eq!(
            oracle_type(&row("NUMBER", 22, Some(38), Some(0), ""), "S"),
            "NUMBER(38)"
        );
        assert_eq!(
            oracle_type(&row("VARCHAR2", 40, None, None, "C"), "S"),
            "VARCHAR2(40 CHAR)"
        );
        assert_eq!(
            oracle_type(&row("VARCHAR2", 40, None, None, "B"), "S"),
            "VARCHAR2(40)"
        );
        assert_eq!(
            oracle_type(&row("NVARCHAR2", 100, None, None, "C"), "S"),
            "NVARCHAR2(100)"
        );
        assert_eq!(oracle_type(&row("RAW", 16, None, None, ""), "S"), "RAW(16)");
        assert_eq!(
            oracle_type(&row("FLOAT", 22, Some(126), None, ""), "S"),
            "FLOAT(126)"
        );
        assert_eq!(oracle_type(&row("DATE", 7, None, None, ""), "S"), "DATE");
        assert_eq!(
            oracle_type(
                &row("TIMESTAMP(6) WITH TIME ZONE", 13, None, Some(6), ""),
                "S"
            ),
            "TIMESTAMP(6) WITH TIME ZONE"
        );
        let mut udt = row("ADDRESS_T", 1, None, None, "");
        udt[12] = Value::from("OTHER");
        assert_eq!(oracle_type(&udt, "S"), "OTHER.ADDRESS_T");
        udt[12] = Value::from("S");
        assert_eq!(oracle_type(&udt, "S"), "ADDRESS_T");
    }

    #[test]
    fn builds_objects_sql_with_one_bind_per_branch() {
        let (sql, n) = objects_sql(&[]).unwrap();
        assert_eq!(n, 7);
        for i in 1..=7 {
            assert_eq!(sql.matches(&format!(":{i} ")).count(), 1, "{sql}");
        }
        assert!(!sql.contains("{}"));
        let (sql, n) = objects_sql(&[ObjectKind::Table, ObjectKind::View]).unwrap();
        assert_eq!(n, 2);
        assert!(sql.contains("all_tables") && sql.contains("all_views"));
        assert!(!sql.contains("all_objects"));
        let (sql, n) = objects_sql(&[ObjectKind::Package, ObjectKind::Synonym]).unwrap();
        assert_eq!(n, 1);
        assert!(sql.contains("IN ('PACKAGE', 'SYNONYM')"), "{sql}");
    }

    #[test]
    fn maps_object_kinds() {
        assert_eq!(object_kind("TABLE"), Some(ObjectKind::Table));
        assert_eq!(
            object_kind("MATERIALIZED VIEW"),
            Some(ObjectKind::MaterializedView)
        );
        assert_eq!(object_kind("TYPE  "), Some(ObjectKind::Type));
        assert_eq!(object_kind("PACKAGE BODY"), None);
    }

    #[test]
    fn recognizes_not_null_checks() {
        assert!(is_not_null_check("\"ID\" IS NOT NULL"));
        assert!(is_not_null_check(" \"My Col\" is not null "));
        assert!(!is_not_null_check("\"ID\" > 0"));
        assert!(!is_not_null_check("ID IS NOT NULL AND X > 1"));
    }

    #[test]
    fn index_columns() {
        assert_eq!(index_column("NAME", None, "ASC"), "NAME");
        assert_eq!(
            index_column("SYS_NC00003$", Some("\"NAME\"".into()), "DESC"),
            "NAME DESC"
        );
        assert_eq!(
            index_column("SYS_NC00004$", Some("UPPER(\"NAME\")".into()), "ASC"),
            "UPPER(\"NAME\")"
        );
    }

    #[test]
    fn joins_units_in_order() {
        let rows = vec![
            vec![Value::from("PACKAGE"), Value::from("PACKAGE p AS\n")],
            vec![Value::from("PACKAGE"), Value::from("END;\n")],
            vec![
                Value::from("PACKAGE BODY"),
                Value::from("PACKAGE BODY p AS\n"),
            ],
            vec![Value::from("PACKAGE BODY"), Value::from("END;\n")],
        ];
        assert_eq!(
            join_units(&rows).unwrap(),
            "CREATE OR REPLACE PACKAGE p AS\nEND;\n/\n\nCREATE OR REPLACE PACKAGE BODY p AS\nEND;\n/"
        );
        assert_eq!(join_units(&[]), None);
    }

    #[test]
    fn quotes_identifiers() {
        assert_eq!(qualified("SCOTT", "EMP"), "\"SCOTT\".\"EMP\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn sequence_fallback_ddl() {
        let r = vec![
            Value::Int(1),
            Value::Decimal("9999999999999999999999999999".into()),
            Value::Int(1),
            Value::from("N"),
            Value::from("N"),
            Value::Int(20),
            Value::Int(41),
        ];
        assert_eq!(
            sequence_ddl("\"S\".\"Q\"", &r),
            "CREATE SEQUENCE \"S\".\"Q\" START WITH 41 INCREMENT BY 1 MINVALUE 1 MAXVALUE 9999999999999999999999999999 CACHE 20 NOCYCLE NOORDER"
        );
    }
}
