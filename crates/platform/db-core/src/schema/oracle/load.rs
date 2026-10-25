//! Bulk load for Oracle: per schema the seven object queries of
//! `Introspector::objects`, then at Structure level one query per family of
//! details (tables, sequences, routines and their arguments, triggers, types)
//! and five for columns, indexes and constraints, at Full level two more
//! for source text; about 23 statements per schema however many tables it
//! holds. Every statement binds the owner once, as `:1`.
//!
//! Written from the Oracle Database Reference (`ALL_*` views). All views and
//! columns exist in 19c; nothing here needs a DBA view or a privilege beyond
//! what the per-object reads need. Not measured on a real database yet; the
//! integration tests that do are `#[ignore]` (see `docs/testing-databases.md`).

use std::collections::HashMap;

use super::{
    OracleIntrospector, column_info, fold_constraint_row, index_column_parts, join_unit_lines,
    qualified_type, sized_type, view_source,
};
use crate::schema::{
    ArgumentInfo, ArgumentMode, AttributeInfo, Cells, ColumnInfo, GeneratedColumn, IdentityKind,
    IndexInfo, Introspector, Level, ObjectDetail, ObjectInfo, ObjectKind, RelationInfo,
    RoutineDetail, SchemaInfo, SchemaSnapshot, Scope, SequenceDetail, Snapshot, SourceInfo,
    TableDetail, TriggerDetail, TypeCategory, TypeDetail, fetch_all, has_columns, has_source,
    opt_text,
};
use crate::{Connection, Dialect, Result, Value};

/// Tables with the flags of their storage and the partitioning scheme. Same
/// filter as the table branch of `objects`.
const TABLES: &str = "\
SELECT t.table_name, t.temporary, t.iot_type, p.partitioning_type, \
       CASE WHEN p.partitioning_type IS NOT NULL THEN \
            (SELECT LISTAGG(k.column_name, ', ') WITHIN GROUP (ORDER BY k.column_position) \
             FROM all_part_key_columns k \
             WHERE k.owner = t.owner AND k.name = t.table_name AND k.object_type = 'TABLE') END \
FROM all_tables t \
LEFT JOIN all_part_tables p ON p.owner = t.owner AND p.table_name = t.table_name \
WHERE t.owner = :1 AND t.nested = 'NO' AND t.secondary = 'N' AND t.dropped = 'NO'";

const SEQUENCES: &str = "\
SELECT s.sequence_name, s.min_value, s.max_value, s.increment_by, s.cycle_flag, s.order_flag, \
       s.cache_size \
FROM all_sequences s WHERE s.sequence_owner = :1";

/// Standalone functions and procedures (a row without `PROCEDURE_NAME`).
const ROUTINES: &str = "\
SELECT p.object_name, p.deterministic, p.pipelined, p.authid, p.parallel \
FROM all_procedures p \
WHERE p.owner = :1 AND p.procedure_name IS NULL AND p.object_type IN ('FUNCTION', 'PROCEDURE')";

/// Arguments of standalone routines in declaration order; position 0 is the
/// return value of a function. A routine without arguments has one row
/// with no data type.
const ARGUMENTS: &str = "\
SELECT a.object_name, a.argument_name, a.position, a.data_type, a.in_out, a.defaulted, \
       a.type_owner, a.type_name \
FROM all_arguments a \
WHERE a.owner = :1 AND a.package_name IS NULL AND a.data_level = 0 \
ORDER BY a.object_name, a.sequence";

const TRIGGERS: &str = concat!(
    "WITH ",
    px_when_fn!(),
    "SELECT t.trigger_name, t.trigger_type, t.triggering_event, t.status, \
            px_when(t.owner, t.trigger_name) \
     FROM all_triggers t WHERE t.owner = :1"
);

const TYPES: &str = "SELECT t.type_name, t.typecode FROM all_types t WHERE t.owner = :1";

const TYPE_ATTRIBUTES: &str = "\
SELECT a.type_name, a.attr_name, a.attr_type_mod, a.attr_type_owner, a.attr_type_name, \
       a.length, a.precision, a.scale, a.char_used \
FROM all_type_attrs a WHERE a.owner = :1 AND a.inherited = 'NO' \
ORDER BY a.type_name, a.attr_no";

const COLLECTION_TYPES: &str = "\
SELECT c.type_name, c.coll_type, c.elem_type_mod, c.elem_type_owner, c.elem_type_name, \
       c.length, c.precision, c.scale, c.char_used \
FROM all_coll_types c WHERE c.owner = :1";

/// The query of `columns` for every table and view of the schema; the table
/// name is the last column.
const COLUMNS: &str = concat!(
    "WITH ",
    px_default_fn!(),
    "SELECT c.column_name, c.column_id, c.data_type, c.data_length, c.data_precision, \
            c.data_scale, c.char_used, c.char_length, c.nullable, \
            CASE WHEN c.default_length > 0 \
                 THEN px_default(c.owner, c.table_name, c.column_name) END, \
            c.identity_column, cc.comments, c.data_type_owner, c.table_name \
     FROM all_tab_columns c \
     LEFT JOIN all_col_comments cc \
            ON cc.owner = c.owner AND cc.table_name = c.table_name \
           AND cc.column_name = c.column_name \
     WHERE c.owner = :1 \
     ORDER BY c.table_name, c.column_id"
);

const IDENTITY_COLUMNS: &str = "\
SELECT i.table_name, i.column_name, i.generation_type \
FROM all_tab_identity_cols i WHERE i.owner = :1";

/// Virtual columns; `ALL_TAB_COLUMNS` does not say which they are.
const VIRTUAL_COLUMNS: &str = "\
SELECT c.table_name, c.column_name \
FROM all_tab_cols c \
WHERE c.owner = :1 AND c.virtual_column = 'YES' AND c.hidden_column = 'NO'";

/// The query of `indexes` for every table of the schema; the table name is
/// the last column.
const INDEXES: &str = concat!(
    "WITH ",
    px_expr_fn!(),
    "SELECT i.index_name, i.uniqueness, i.index_type, c.column_name, c.descend, \
            CASE WHEN i.index_type LIKE 'FUNCTION-BASED%' \
                 THEN px_expr(c.index_owner, c.index_name, c.column_position) END, \
            i.table_name \
     FROM all_indexes i \
     JOIN all_ind_columns c ON c.index_owner = i.owner AND c.index_name = i.index_name \
     WHERE i.table_owner = :1 AND i.index_type <> 'LOB' \
     ORDER BY i.table_name, i.index_name, c.column_position"
);

/// The query of `constraints` for every table of the schema, with the
/// deferrable, deferred, status and validated columns after the common
/// nine, and the table name last.
const CONSTRAINTS: &str = "\
SELECT c.constraint_name, c.constraint_type, c.generated, c.search_condition_vc, c.delete_rule, \
       cc.column_name, r.owner, r.table_name, rc.column_name, \
       c.deferrable, c.deferred, c.status, c.validated, c.table_name \
FROM all_constraints c \
LEFT JOIN all_cons_columns cc \
       ON cc.owner = c.owner AND cc.constraint_name = c.constraint_name \
      AND cc.table_name = c.table_name \
LEFT JOIN all_constraints r ON r.owner = c.r_owner AND r.constraint_name = c.r_constraint_name \
LEFT JOIN all_cons_columns rc \
       ON rc.owner = r.owner AND rc.constraint_name = r.constraint_name \
      AND rc.position = cc.position \
WHERE c.owner = :1 AND c.constraint_type IN ('P', 'U', 'R', 'C') \
ORDER BY c.table_name, \
         CASE c.constraint_type WHEN 'P' THEN 0 WHEN 'U' THEN 1 WHEN 'R' THEN 2 ELSE 3 END, \
         c.constraint_name, cc.position, cc.column_name";

/// Views with their text; one longer than `TEXT_VC` holds comes with its
/// `DBMS_METADATA` DDL in the last column, as `source` reads it.
const VIEW_SOURCES: &str = concat!(
    "WITH ",
    px_ddl_fn!(),
    "SELECT v.view_name, v.text_length, v.text_vc, \
            CASE WHEN v.text_length > 4000 THEN px_ddl('VIEW', v.view_name, v.owner) END \
     FROM all_views v WHERE v.owner = :1 ORDER BY v.view_name"
);

/// The stored text of PL/SQL units, line by line, a spec before its body.
const UNIT_SOURCES: &str = "\
SELECT s.name, s.type, s.text FROM all_source s \
WHERE s.owner = :1 \
  AND s.type IN ('PROCEDURE', 'FUNCTION', 'PACKAGE', 'PACKAGE BODY', 'TYPE', 'TYPE BODY', \
                 'TRIGGER') \
ORDER BY s.name, s.type, s.line";

pub(super) async fn load(
    intro: &OracleIntrospector<'_>,
    scope: &Scope,
    level: Level,
) -> Result<Snapshot> {
    let mut schemas = Vec::new();
    for info in intro.schemas().await? {
        if scope.selects(&info) {
            schemas.push(load_schema(intro, info, level).await?);
        }
    }
    Ok(Snapshot {
        level,
        token: None,
        schemas,
    })
}

async fn load_schema(
    intro: &OracleIntrospector<'_>,
    info: SchemaInfo,
    level: Level,
) -> Result<SchemaSnapshot> {
    let conn = intro.conn;
    let schema = info.name.clone();
    let arg = [Value::from(schema.as_str())];

    // The same queries as `objects`, so the base fields are the same.
    let mut objects = intro.objects(&schema, &[]).await?;
    for object in &mut objects {
        object.owner = Some(schema.clone());
    }

    let mut relations = Vec::new();
    if level >= Level::Structure {
        let mut details = Details::read(conn, &arg, &schema).await?;
        for object in &mut objects {
            object.detail = details.take(object);
        }
        relations = relations_of(conn, &arg, &schema, &objects).await?;
    }

    let mut sources = Vec::new();
    if level >= Level::Full {
        sources = read_sources(conn, &arg, &schema, &objects).await?;
    }

    Ok(SchemaSnapshot {
        info,
        level,
        objects,
        relations,
        sources,
    })
}

/// The kind-specific facts of a schema, by object name.
#[derive(Default)]
struct Details {
    tables: HashMap<String, TableDetail>,
    sequences: HashMap<String, SequenceDetail>,
    routines: HashMap<String, RoutineDetail>,
    triggers: HashMap<String, TriggerDetail>,
    types: HashMap<String, TypeDetail>,
}

impl Details {
    async fn read(conn: &dyn Connection, arg: &[Value], schema: &str) -> Result<Self> {
        let mut d = Details::default();

        for r in fetch_all(conn, "tables", TABLES, arg).await? {
            d.tables.insert(r.text(0)?, table_detail(&r)?);
        }
        for r in fetch_all(conn, "sequences", SEQUENCES, arg).await? {
            d.sequences.insert(r.text(0)?, sequence_detail(&r)?);
        }

        for r in fetch_all(conn, "routines", ROUTINES, arg).await? {
            d.routines.insert(r.text(0)?, routine_detail(&r)?);
        }
        for r in fetch_all(conn, "arguments", ARGUMENTS, arg).await? {
            if let Some(routine) = d.routines.get_mut(&r.text(0)?) {
                add_argument(routine, &r, schema)?;
            }
        }

        for r in fetch_all(conn, "triggers", TRIGGERS, arg).await? {
            d.triggers.insert(r.text(0)?, trigger_detail(&r)?);
        }

        for r in fetch_all(conn, "types", TYPES, arg).await? {
            d.types.insert(
                r.text(0)?,
                TypeDetail {
                    category: match r.text(1)?.as_str() {
                        "OBJECT" => TypeCategory::Object,
                        "COLLECTION" => TypeCategory::Collection,
                        _ => TypeCategory::Other,
                    },
                    ..TypeDetail::default()
                },
            );
        }
        for r in fetch_all(conn, "type_attributes", TYPE_ATTRIBUTES, arg).await? {
            if let Some(t) = d.types.get_mut(&r.text(0)?) {
                t.attributes.push(AttributeInfo {
                    name: r.text(1)?,
                    data_type: member_type(&r, schema)?,
                });
            }
        }
        for r in fetch_all(conn, "collection_types", COLLECTION_TYPES, arg).await? {
            if let Some(t) = d.types.get_mut(&r.text(0)?) {
                let element = member_type(&r, schema)?;
                t.base_type = Some(match r.text(1)?.as_str() {
                    "VARRAY" => format!("VARRAY OF {element}"),
                    _ => format!("TABLE OF {element}"),
                });
            }
        }
        Ok(d)
    }

    /// The detail of `object`, moved out of the maps (names are unique per
    /// kind family in a schema).
    fn take(&mut self, object: &ObjectInfo) -> Option<ObjectDetail> {
        let name = &object.name;
        match object.kind {
            ObjectKind::Table => Some(ObjectDetail::Table(
                self.tables.remove(name).unwrap_or_default(),
            )),
            ObjectKind::Sequence => self.sequences.remove(name).map(ObjectDetail::Sequence),
            ObjectKind::Function | ObjectKind::Procedure => {
                self.routines.remove(name).map(ObjectDetail::Routine)
            }
            ObjectKind::Trigger => self.triggers.remove(name).map(ObjectDetail::Trigger),
            ObjectKind::Type => self.types.remove(name).map(ObjectDetail::Type),
            _ => None,
        }
    }
}

/// Columns: table name, temporary (`Y`/`N`), IOT type, partitioning type,
/// partition key columns.
fn table_detail(r: &Cells) -> Result<TableDetail> {
    let partitioning = r.opt_text(3)?;
    let partition_key = match &partitioning {
        Some(kind) => Some(format!("{kind} ({})", r.opt_text(4)?.unwrap_or_default())),
        None => None,
    };
    Ok(TableDetail {
        temporary: r.text(1)? == "Y",
        index_organized: r.text(2)? == "IOT",
        partitioned: partitioning.is_some(),
        partition_key,
        ..TableDetail::default()
    })
}

/// Columns: name, min, max, increment, cycle, order, cache.
fn sequence_detail(r: &Cells) -> Result<SequenceDetail> {
    Ok(SequenceDetail {
        min: r.opt_text(1)?,
        max: r.opt_text(2)?,
        increment: r.opt_text(3)?,
        cycle: r.text(4)? == "Y",
        cache: r.int(6)?.filter(|n| *n > 0),
        ordered: r.text(5)? == "Y",
        ..SequenceDetail::default()
    })
}

/// Columns: name, deterministic, pipelined, authid, parallel.
fn routine_detail(r: &Cells) -> Result<RoutineDetail> {
    let mut characteristics = Vec::new();
    if r.text(1)? == "YES" {
        characteristics.push("DETERMINISTIC".to_owned());
    }
    if r.text(2)? == "YES" {
        characteristics.push("PIPELINED".to_owned());
    }
    if r.text(3)? == "CURRENT_USER" {
        characteristics.push("AUTHID CURRENT_USER".to_owned());
    }
    if r.text(4)? == "YES" {
        characteristics.push("PARALLEL_ENABLE".to_owned());
    }
    Ok(RoutineDetail {
        language: Some("PL/SQL".to_owned()),
        characteristics,
        ..RoutineDetail::default()
    })
}

/// A row of [`ARGUMENTS`] into the routine: position 0 is the return type
/// of a function, the others are arguments.
fn add_argument(routine: &mut RoutineDetail, r: &Cells, schema: &str) -> Result<()> {
    let Some(data_type) = r.opt_text(3)? else {
        return Ok(());
    };
    let data_type = match r.opt_text(7)? {
        Some(name) => match r.opt_text(6)? {
            Some(owner) => qualified_type(&owner, &name, schema),
            None => name,
        },
        None => data_type,
    };
    if r.int(2)? == Some(0) {
        routine.return_type = Some(data_type);
        return Ok(());
    }
    routine.arguments.push(ArgumentInfo {
        name: r.opt_text(1)?,
        data_type,
        mode: match r.text(4)?.as_str() {
            "OUT" => ArgumentMode::Out,
            "IN/OUT" => ArgumentMode::InOut,
            _ => ArgumentMode::In,
        },
        has_default: r.text(5)? == "Y",
    });
    Ok(())
}

/// Columns: name, trigger type (`BEFORE EACH ROW`), events (`INSERT OR
/// UPDATE`), status, `WHEN` condition.
fn trigger_detail(r: &Cells) -> Result<TriggerDetail> {
    let kind = r.text(1)?;
    let kind = kind.trim();
    let (timing, each_row) = if kind == "INSTEAD OF" {
        ("INSTEAD OF".to_owned(), true)
    } else if let Some(timing) = kind.strip_suffix(" EACH ROW") {
        (timing.to_owned(), true)
    } else if let Some(timing) = kind.strip_suffix(" STATEMENT") {
        (timing.to_owned(), false)
    } else {
        (kind.to_owned(), false)
    };
    Ok(TriggerDetail {
        events: r
            .text(2)?
            .split(" OR ")
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty())
            .collect(),
        timing: Some(timing).filter(|t| !t.is_empty()),
        each_row,
        function: None,
        condition: r.opt_text(4)?.map(|c| c.trim().to_owned()),
        disabled: r.text(3)? == "DISABLED",
    })
}

/// The type of an attribute or collection element from columns 2..=8
/// (`type_mod, type_owner, type_name, length, precision, scale, char_used`)
/// of [`TYPE_ATTRIBUTES`] and [`COLLECTION_TYPES`].
fn member_type(r: &Cells, schema: &str) -> Result<String> {
    let name = r.text(4)?;
    let owner = r.opt_text(3)?;
    let base = sized_type(&name, r.int(5)?, r.int(6)?, r.int(7)?, &r.text(8)?, None)
        .unwrap_or_else(|| match owner {
            Some(owner) => qualified_type(&owner, &name, schema),
            None => name,
        });
    Ok(match r.opt_text(2)? {
        Some(modifier) => format!("{modifier} {base}"),
        None => base,
    })
}

/// Columns, indexes and constraints of every table, view and materialized
/// view of the schema, in the order of the objects.
async fn relations_of(
    conn: &dyn Connection,
    arg: &[Value],
    schema: &str,
    objects: &[ObjectInfo],
) -> Result<Vec<RelationInfo>> {
    let mut relations: Vec<RelationInfo> = objects
        .iter()
        .filter(|o| has_columns(o.kind))
        .map(|o| RelationInfo {
            name: o.name.clone(),
            ..RelationInfo::default()
        })
        .collect();
    let index: HashMap<String, usize> = relations
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.clone(), i))
        .collect();

    let mut identity: HashMap<(String, String), IdentityKind> = HashMap::new();
    for r in fetch_all(conn, "identity_columns", IDENTITY_COLUMNS, arg).await? {
        identity.insert(
            (r.text(0)?, r.text(1)?),
            if r.text(2)? == "ALWAYS" {
                IdentityKind::Always
            } else {
                IdentityKind::ByDefault
            },
        );
    }
    let virtual_columns = fetch_all(conn, "virtual_columns", VIRTUAL_COLUMNS, arg)
        .await?
        .iter()
        .map(|r| Ok((r.text(0)?, r.text(1)?)))
        .collect::<Result<Vec<(String, String)>>>()?;

    for r in fetch_all(conn, "columns", COLUMNS, arg).await? {
        let table = r.text(13)?;
        let Some(&at) = index.get(&table) else {
            continue;
        };
        let mut column: ColumnInfo = column_info(&r, schema)?;
        let key = (table, column.name.clone());
        column.identity = identity.get(&key).copied();
        if virtual_columns.contains(&key) {
            // The expression of a virtual column is its default.
            column.generated = Some(GeneratedColumn {
                expression: column.default.clone(),
                stored: false,
            });
        }
        relations[at].columns.push(column);
    }

    for r in fetch_all(conn, "indexes", INDEXES, arg).await? {
        let Some(&at) = index.get(&r.text(6)?) else {
            continue;
        };
        add_index_row(&mut relations[at].indexes, &r)?;
    }

    for r in fetch_all(conn, "constraints", CONSTRAINTS, arg).await? {
        let Some(&at) = index.get(&r.text(13)?) else {
            continue;
        };
        fold_constraint_row(&mut relations[at].constraints, &r)?;
    }
    Ok(relations)
}

/// A row of [`INDEXES`] into the indexes of its table: a new index, or the
/// next column of the last one.
fn add_index_row(indexes: &mut Vec<IndexInfo>, r: &Cells) -> Result<()> {
    let name = r.text(0)?;
    let (column, is_expression) = index_column_parts(&r.text(3)?, r.opt_text(5)?, &r.text(4)?);
    match indexes.last_mut().filter(|i| i.name == name) {
        Some(last) => {
            if is_expression {
                last.expressions.push(last.columns.len());
            }
            last.columns.push(column);
        }
        None => indexes.push(IndexInfo {
            name,
            unique: r.text(1)? == "UNIQUE",
            expressions: if is_expression { vec![0] } else { Vec::new() },
            columns: vec![column],
            method: r.opt_text(2)?,
            ..IndexInfo::default()
        }),
    }
    Ok(())
}

/// The texts `source` returns for the objects of the schema: views, and
/// PL/SQL units (a package with its body, a type with its body).
async fn read_sources(
    conn: &dyn Connection,
    arg: &[Value],
    schema: &str,
    objects: &[ObjectInfo],
) -> Result<Vec<SourceInfo>> {
    let known = |name: &str, kind: ObjectKind| {
        has_source(Dialect::Oracle, kind)
            && objects.iter().any(|o| o.kind == kind && o.name == name)
    };
    let mut out = Vec::new();

    for r in fetch_all(conn, "view_sources", VIEW_SOURCES, arg).await? {
        let name = r.text(0)?;
        if !known(&name, ObjectKind::View) {
            continue;
        }
        let ddl = opt_text(&match r.value(3)? {
            Value::Truncated { prefix, .. } => prefix.as_ref().clone(),
            v => v.clone(),
        });
        if let Some(text) = view_source(schema, &name, r.int(1)?.unwrap_or(0), &r.text(2)?, ddl) {
            out.push(SourceInfo {
                name,
                kind: ObjectKind::View,
                signature: String::new(),
                text,
            });
        }
    }

    // (name, kind) -> the (type, line) pairs, in the order of the first row.
    type Unit = (String, ObjectKind, Vec<(String, String)>);
    let mut units: Vec<Unit> = Vec::new();
    let mut at: HashMap<(String, ObjectKind), usize> = HashMap::new();
    for r in fetch_all(conn, "unit_sources", UNIT_SOURCES, arg).await? {
        let (name, ty) = (r.text(0)?, r.text(1)?);
        let Some(kind) = unit_kind(&ty) else {
            continue;
        };
        if !known(&name, kind) {
            continue;
        }
        let slot = *at.entry((name.clone(), kind)).or_insert_with(|| {
            units.push((name, kind, Vec::new()));
            units.len() - 1
        });
        units[slot].2.push((ty, r.text(2)?));
    }
    for (name, kind, lines) in units {
        if let Some(text) = join_unit_lines(lines.into_iter()) {
            out.push(SourceInfo {
                name,
                kind,
                signature: String::new(),
                text,
            });
        }
    }
    Ok(out)
}

fn unit_kind(ty: &str) -> Option<ObjectKind> {
    Some(match ty {
        "PROCEDURE" => ObjectKind::Procedure,
        "FUNCTION" => ObjectKind::Function,
        "PACKAGE" | "PACKAGE BODY" => ObjectKind::Package,
        "TYPE" | "TYPE BODY" => ObjectKind::Type,
        "TRIGGER" => ObjectKind::Trigger,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use super::*;
    use crate::schema::{ConstraintKind, SchemaFilter};
    use crate::{
        Backend, Column, Cursor, ExecOptions, Outcome, Result as DbResult, Row, TxMode, ValueKind,
    };

    fn v(s: &str) -> Value {
        Value::from(s)
    }

    fn row(cells: Vec<Value>) -> Cells {
        Cells::new("test", cells)
    }

    #[test]
    fn every_statement_binds_the_owner_once() {
        let all = [
            TABLES,
            SEQUENCES,
            ROUTINES,
            ARGUMENTS,
            TRIGGERS,
            TYPES,
            TYPE_ATTRIBUTES,
            COLLECTION_TYPES,
            COLUMNS,
            IDENTITY_COLUMNS,
            VIRTUAL_COLUMNS,
            INDEXES,
            CONSTRAINTS,
            VIEW_SOURCES,
            UNIT_SOURCES,
        ];
        for sql in all {
            assert_eq!(sql.matches(":1").count(), 1, "{sql}");
            assert!(!sql.contains(":2") && !sql.contains("{}"), "{sql}");
        }
    }

    #[test]
    fn statements_with_functions_declare_them_once() {
        for (sql, function) in [
            (COLUMNS, "px_default"),
            (INDEXES, "px_expr"),
            (TRIGGERS, "px_when"),
            (VIEW_SOURCES, "px_ddl"),
        ] {
            assert!(sql.starts_with("WITH FUNCTION "), "{sql}");
            assert_eq!(
                sql.matches(&format!("FUNCTION {function}(")).count(),
                1,
                "{sql}"
            );
            assert_eq!(sql.matches("END;").count(), 1, "{sql}");
            assert!(sql.contains("\nSELECT "), "{sql}");
        }
    }

    #[test]
    fn column_queries_agree_with_the_per_object_ones() {
        // The bulk query selects the per-object columns, then the table.
        let per_object_start = super::super::COLUMNS.split("SELECT c.column_name").nth(1);
        let bulk_start = COLUMNS.split("SELECT c.column_name").nth(1);
        let (per_object, bulk) = (per_object_start.unwrap(), bulk_start.unwrap());
        assert!(
            bulk.contains("c.data_type_owner, c.table_name FROM"),
            "{bulk}"
        );
        assert!(
            per_object.contains("c.data_type_owner FROM"),
            "{per_object}"
        );
    }

    #[test]
    fn describes_tables_and_sequences() {
        let plain = row(vec![v("T"), v("N"), Value::Null, Value::Null, Value::Null]);
        assert_eq!(table_detail(&plain).unwrap(), TableDetail::default());
        let partitioned = row(vec![v("T"), v("Y"), v("IOT"), v("RANGE"), v("DAY, SHOP")]);
        let detail = table_detail(&partitioned).unwrap();
        assert!(detail.temporary && detail.index_organized && detail.partitioned);
        assert_eq!(detail.partition_key.as_deref(), Some("RANGE (DAY, SHOP)"));

        let seq = row(vec![
            v("S"),
            Value::Int(1),
            Value::Decimal("9999999999999999999999999999".into()),
            Value::Int(2),
            v("Y"),
            v("N"),
            Value::Int(10),
        ]);
        let detail = sequence_detail(&seq).unwrap();
        assert_eq!(detail.max.as_deref(), Some("9999999999999999999999999999"));
        assert!(detail.cycle && !detail.ordered);
        assert_eq!(detail.cache, Some(10));
        let nocache = row(vec![
            v("S"),
            Value::Int(1),
            Value::Int(9),
            Value::Int(1),
            v("N"),
            v("Y"),
            Value::Int(0),
        ]);
        let detail = sequence_detail(&nocache).unwrap();
        assert_eq!(detail.cache, None);
        assert!(detail.ordered);
    }

    #[test]
    fn describes_routines_with_their_arguments() {
        let mut routine = routine_detail(&row(vec![
            v("ADD2"),
            v("YES"),
            v("NO"),
            v("CURRENT_USER"),
            v("NO"),
        ]))
        .unwrap();
        assert_eq!(
            routine.characteristics,
            ["DETERMINISTIC", "AUTHID CURRENT_USER"]
        );
        let arg = |name: Option<&str>, pos: i64, ty: &str, in_out: &str, defaulted: &str| {
            row(vec![
                v("ADD2"),
                name.map(v).unwrap_or(Value::Null),
                Value::Int(pos),
                v(ty),
                v(in_out),
                v(defaulted),
                Value::Null,
                Value::Null,
            ])
        };
        add_argument(&mut routine, &arg(None, 0, "NUMBER", "OUT", "N"), "S").unwrap();
        add_argument(&mut routine, &arg(Some("A"), 1, "NUMBER", "IN", "N"), "S").unwrap();
        add_argument(
            &mut routine,
            &arg(Some("B"), 2, "NUMBER", "IN/OUT", "Y"),
            "S",
        )
        .unwrap();
        // The placeholder row of a routine without arguments.
        add_argument(
            &mut routine,
            &row(vec![
                v("ADD2"),
                Value::Null,
                Value::Int(1),
                Value::Null,
                v("IN"),
                v("N"),
                Value::Null,
                Value::Null,
            ]),
            "S",
        )
        .unwrap();
        assert_eq!(routine.return_type.as_deref(), Some("NUMBER"));
        assert_eq!(routine.arguments.len(), 2);
        assert_eq!(routine.arguments[0].name.as_deref(), Some("A"));
        assert_eq!(routine.arguments[1].mode, ArgumentMode::InOut);
        assert!(routine.arguments[1].has_default && !routine.arguments[0].has_default);

        // A user type is qualified unless it belongs to the schema.
        let mut routine = RoutineDetail::default();
        let typed = |owner: &str| {
            row(vec![
                v("P"),
                v("X"),
                Value::Int(1),
                v("OBJECT"),
                v("IN"),
                v("N"),
                v(owner),
                v("ADDRESS_T"),
            ])
        };
        add_argument(&mut routine, &typed("OTHER"), "S").unwrap();
        add_argument(&mut routine, &typed("S"), "S").unwrap();
        assert_eq!(routine.arguments[0].data_type, "OTHER.ADDRESS_T");
        assert_eq!(routine.arguments[1].data_type, "ADDRESS_T");
    }

    #[test]
    fn describes_triggers() {
        let t = trigger_detail(&row(vec![
            v("TRG"),
            v("BEFORE EACH ROW"),
            v("INSERT OR UPDATE"),
            v("ENABLED"),
            v("NEW.qty > 0 "),
        ]))
        .unwrap();
        assert_eq!(t.events, ["INSERT", "UPDATE"]);
        assert_eq!(t.timing.as_deref(), Some("BEFORE"));
        assert!(t.each_row && !t.disabled);
        assert_eq!(t.condition.as_deref(), Some("NEW.qty > 0"));

        let t = trigger_detail(&row(vec![
            v("TRG"),
            v("AFTER STATEMENT"),
            v("DELETE"),
            v("DISABLED"),
            Value::Null,
        ]))
        .unwrap();
        assert_eq!(t.timing.as_deref(), Some("AFTER"));
        assert!(!t.each_row && t.disabled);
        assert_eq!(t.condition, None);

        let t = trigger_detail(&row(vec![
            v("TRG"),
            v("INSTEAD OF"),
            v("INSERT"),
            v("ENABLED"),
            Value::Null,
        ]))
        .unwrap();
        assert_eq!(t.timing.as_deref(), Some("INSTEAD OF"));
        assert!(t.each_row);
    }

    #[test]
    fn describes_member_types() {
        let attr =
            |owner: Option<&str>, ty: &str, len: Option<i64>, p: Option<i64>, s: Option<i64>| {
                vec![
                    v("T"),
                    v("A"),
                    Value::Null,
                    owner.map(v).unwrap_or(Value::Null),
                    v(ty),
                    len.into(),
                    p.into(),
                    s.into(),
                    v("B"),
                ]
            };
        assert_eq!(
            member_type(
                &row(attr(Some("SYS"), "NUMBER", Some(22), Some(9), Some(2))),
                "S"
            )
            .unwrap(),
            "NUMBER(9,2)"
        );
        assert_eq!(
            member_type(
                &row(attr(Some("SYS"), "VARCHAR2", Some(10), None, None)),
                "S"
            )
            .unwrap(),
            "VARCHAR2(10)"
        );
        assert_eq!(
            member_type(&row(attr(Some("SYS"), "DATE", Some(7), None, None)), "S").unwrap(),
            "DATE"
        );
        assert_eq!(
            member_type(
                &row(attr(Some("OTHER"), "ADDRESS_T", None, None, None)),
                "S"
            )
            .unwrap(),
            "OTHER.ADDRESS_T"
        );
        let mut by_ref = attr(Some("S"), "ADDRESS_T", None, None, None);
        by_ref[2] = v("REF");
        assert_eq!(member_type(&row(by_ref), "S").unwrap(), "REF ADDRESS_T");
    }

    fn constraint_row(
        name: &str,
        kind: &str,
        generated: &str,
        check: Option<&str>,
        column: &str,
        reference: Option<(&str, &str)>,
        flags: [&str; 4],
    ) -> Row {
        let (ref_table, ref_column) = reference.unzip();
        vec![
            v(name),
            v(kind),
            v(generated),
            check.map(v).unwrap_or(Value::Null),
            if kind == "R" {
                v("CASCADE")
            } else {
                Value::Null
            },
            v(column),
            if kind == "R" { v("S") } else { Value::Null },
            ref_table.map(v).unwrap_or(Value::Null),
            ref_column.map(v).unwrap_or(Value::Null),
            v(flags[0]),
            v(flags[1]),
            v(flags[2]),
            v(flags[3]),
            v("T"),
        ]
    }

    #[test]
    fn folds_constraint_rows() {
        let plain = ["NOT DEFERRABLE", "IMMEDIATE", "ENABLED", "VALIDATED"];
        let rows = vec![
            constraint_row("PK", "P", "USER NAME", None, "A", None, plain),
            constraint_row("PK", "P", "USER NAME", None, "B", None, plain),
            constraint_row("FK", "R", "USER NAME", None, "A", Some(("P", "X")), plain),
            constraint_row("FK", "R", "USER NAME", None, "B", Some(("P", "Y")), plain),
            constraint_row(
                "CK",
                "C",
                "USER NAME",
                Some("A > 0"),
                "A",
                None,
                ["DEFERRABLE", "DEFERRED", "DISABLED", "NOT VALIDATED"],
            ),
            constraint_row(
                "SYS_C001",
                "C",
                "GENERATED NAME",
                Some("\"A\" IS NOT NULL"),
                "A",
                None,
                plain,
            ),
        ];
        let mut out = Vec::new();
        for r in &rows {
            fold_constraint_row(&mut out, &row(r.clone())).unwrap();
        }
        assert_eq!(out.len(), 3, "the NOT NULL check is skipped: {out:?}");
        assert_eq!(out[0].kind, ConstraintKind::PrimaryKey);
        assert_eq!(out[0].columns, ["A", "B"]);
        assert!(!out[0].deferrable && !out[0].disabled && !out[0].not_validated);
        assert_eq!(out[1].ref_table.as_deref(), Some("P"));
        assert_eq!(out[1].ref_columns, ["X", "Y"]);
        assert_eq!(out[1].on_delete.as_deref(), Some("CASCADE"));
        assert!(out[2].deferrable && out[2].initially_deferred);
        assert!(out[2].disabled && out[2].not_validated);
        assert_eq!(out[2].check.as_deref(), Some("A > 0"));

        // A row of the per-object query has nine columns.
        let mut out = Vec::new();
        fold_constraint_row(&mut out, &row(rows[0][..9].to_vec())).unwrap();
        assert_eq!(out.len(), 1);
        assert!(!out[0].deferrable && !out[0].disabled);
    }

    #[test]
    fn folds_index_rows() {
        let r = |index: &str, unique: &str, column: &str, descend: &str, expr: Option<&str>| {
            row(vec![
                v(index),
                v(unique),
                v("NORMAL"),
                v(column),
                v(descend),
                expr.map(v).unwrap_or(Value::Null),
                v("T"),
            ])
        };
        let mut out = Vec::new();
        for row in [
            r(
                "I1",
                "NONUNIQUE",
                "SYS_NC1$",
                "ASC",
                Some("UPPER(\"LABEL\")"),
            ),
            r("I1", "NONUNIQUE", "SYS_NC2$", "DESC", Some("\"QTY\"")),
            r("I2", "UNIQUE", "ID", "ASC", None),
        ] {
            add_index_row(&mut out, &row).unwrap();
        }
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].columns, ["UPPER(\"LABEL\")", "QTY DESC"]);
        assert_eq!(out[0].expressions, [0]);
        assert!(!out[0].unique && out[1].unique);
        assert_eq!(out[1].columns, ["ID"]);
        assert!(out[1].expressions.is_empty());
    }

    #[test]
    fn maps_unit_types() {
        assert_eq!(unit_kind("PACKAGE BODY"), Some(ObjectKind::Package));
        assert_eq!(unit_kind("TYPE BODY"), Some(ObjectKind::Type));
        assert_eq!(unit_kind("TRIGGER"), Some(ObjectKind::Trigger));
        assert_eq!(unit_kind("JAVA SOURCE"), None);
    }

    /// Answers every statement of the introspector with canned rows and
    /// counts them.
    struct Canned {
        statements: AtomicUsize,
        tables: usize,
    }

    struct Page(Vec<Row>, Vec<Column>);

    #[async_trait]
    impl Cursor for Page {
        fn columns(&self) -> &[Column] {
            &self.1
        }
        async fn next_page(&mut self, _max_rows: usize) -> DbResult<Vec<Row>> {
            Ok(std::mem::take(&mut self.0))
        }
        fn is_exhausted(&self) -> bool {
            self.0.is_empty()
        }
        async fn close(self: Box<Self>) -> DbResult<()> {
            Ok(())
        }
    }

    impl Canned {
        fn rows(&self, sql: &str) -> Vec<Row> {
            let table = |i: usize| format!("T{i}");
            let tables = (0..self.tables).map(table);
            let per_table = |f: &dyn Fn(String) -> Vec<Row>| -> Vec<Row> {
                tables.clone().flat_map(f).collect()
            };
            let objects = super::super::objects_sql(&[]);
            if sql == super::super::SCHEMAS {
                vec![vec![v("S"), v("N"), v("Y")]]
            } else if sql == objects[0] {
                per_table(&|t| vec![vec![v(&t), v("TABLE"), Value::Null, Value::Null]])
            } else if sql == objects[3] {
                vec![vec![v("SEQ"), v("SEQUENCE"), Value::Null, Value::Null]]
            } else if sql.starts_with("SELECT o.object_name") {
                vec![vec![v("FN"), v("FUNCTION"), Value::Null, Value::Null]]
            } else if sql == TABLES {
                per_table(&|t| vec![vec![v(&t), v("N"), Value::Null, Value::Null, Value::Null]])
            } else if sql == COLUMNS {
                per_table(&|t| {
                    ["ID", "LABEL"]
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            vec![
                                v(c),
                                Value::Int(i as i64 + 1),
                                v(if i == 0 { "NUMBER" } else { "VARCHAR2" }),
                                Value::Int(22),
                                Value::Null,
                                Value::Null,
                                v("B"),
                                Value::Int(20),
                                v("Y"),
                                Value::Null,
                                v("NO"),
                                Value::Null,
                                Value::Null,
                                v(&t),
                            ]
                        })
                        .collect()
                })
            } else if sql == IDENTITY_COLUMNS {
                per_table(&|t| vec![vec![v(&t), v("ID"), v("ALWAYS")]])
            } else if sql == INDEXES {
                per_table(&|t| {
                    vec![vec![
                        v(&format!("{t}_PK")),
                        v("UNIQUE"),
                        v("NORMAL"),
                        v("ID"),
                        v("ASC"),
                        Value::Null,
                        v(&t),
                    ]]
                })
            } else if sql == CONSTRAINTS {
                per_table(&|t| {
                    vec![constraint_row(
                        &format!("{t}_PK"),
                        "P",
                        "USER NAME",
                        None,
                        "ID",
                        None,
                        ["NOT DEFERRABLE", "IMMEDIATE", "ENABLED", "VALIDATED"],
                    )]
                    .into_iter()
                    .map(|mut r| {
                        r[13] = v(&t);
                        r
                    })
                    .collect()
                })
            } else if sql == ARGUMENTS {
                vec![vec![
                    v("FN"),
                    Value::Null,
                    Value::Int(0),
                    v("NUMBER"),
                    v("OUT"),
                    v("N"),
                    Value::Null,
                    Value::Null,
                ]]
            } else if sql == ROUTINES {
                vec![vec![v("FN"), v("NO"), v("NO"), v("DEFINER"), v("NO")]]
            } else if sql == UNIT_SOURCES {
                vec![
                    vec![v("FN"), v("FUNCTION"), v("FUNCTION fn RETURN NUMBER IS\n")],
                    vec![v("FN"), v("FUNCTION"), v("BEGIN RETURN 1; END;\n")],
                    vec![
                        v("OTHER"),
                        v("PROCEDURE"),
                        v("PROCEDURE other IS BEGIN NULL; END;"),
                    ],
                ]
            } else {
                Vec::new()
            }
        }
    }

    #[async_trait]
    impl Connection for Canned {
        fn backend(&self) -> Backend {
            Backend::OracleThin
        }
        async fn execute(
            &self,
            sql: &str,
            _params: &[Value],
            _opts: &ExecOptions,
        ) -> DbResult<Outcome> {
            self.statements.fetch_add(1, Ordering::SeqCst);
            let rows = self.rows(sql);
            let width = rows.first().map_or(0, Vec::len);
            let columns = (0..width)
                .map(|i| Column {
                    name: format!("C{i}"),
                    type_name: "VARCHAR2".into(),
                    kind: ValueKind::Text,
                    nullable: true,
                })
                .collect();
            Ok(Outcome::Rows(Box::new(Page(rows, columns))))
        }
        async fn cancel(&self) -> DbResult<()> {
            Ok(())
        }
        async fn server_version(&self) -> DbResult<String> {
            Ok("Oracle Database 19c".into())
        }
        async fn ping(&self) -> DbResult<()> {
            Ok(())
        }
        async fn tx_mode(&self) -> TxMode {
            TxMode::default()
        }
        async fn set_tx_mode(&self, _mode: TxMode) -> DbResult<()> {
            Ok(())
        }
        async fn commit(&self) -> DbResult<()> {
            Ok(())
        }
        async fn rollback(&self) -> DbResult<()> {
            Ok(())
        }
        async fn close(&self) -> DbResult<()> {
            Ok(())
        }
    }

    async fn statements(tables: usize, level: Level) -> (usize, Snapshot) {
        let conn = Arc::new(Canned {
            statements: AtomicUsize::new(0),
            tables,
        });
        let intro = OracleIntrospector::new(&*conn);
        let scope = Scope {
            filter: SchemaFilter::Current,
            system: false,
        };
        let snapshot = intro.load(&scope, level).await.expect("load");
        (conn.statements.load(Ordering::SeqCst), snapshot)
    }

    #[tokio::test]
    async fn the_number_of_statements_does_not_depend_on_the_tables() {
        for level in [Level::Names, Level::Structure, Level::Full] {
            let (few, _) = statements(3, level).await;
            let (many, _) = statements(300, level).await;
            assert_eq!(few, many, "{level:?}");
        }
        let (names, _) = statements(3, Level::Names).await;
        let (structure, _) = statements(3, Level::Structure).await;
        let (full, _) = statements(3, Level::Full).await;
        assert!(names < structure && structure < full);
        assert!(full <= 30, "{full} statements");
    }

    #[tokio::test]
    async fn loads_structure_and_source_from_the_rows() {
        let (_, snapshot) = statements(2, Level::Full).await;
        let schema = snapshot.schema("S").expect("schema S");
        assert_eq!(schema.objects.len(), 4);
        let table = schema
            .objects
            .iter()
            .find(|o| o.name == "T0")
            .expect("table T0");
        assert_eq!(table.owner.as_deref(), Some("S"));
        assert!(matches!(table.detail, Some(ObjectDetail::Table(_))));

        let relation = schema.relation("T1").expect("relation T1");
        assert_eq!(relation.columns.len(), 2);
        assert_eq!(relation.columns[0].identity, Some(IdentityKind::Always));
        assert_eq!(relation.columns[1].identity, None);
        assert_eq!(relation.columns[1].data_type, "VARCHAR2(20)");
        assert_eq!(relation.indexes.len(), 1);
        assert_eq!(relation.constraints.len(), 1);
        assert_eq!(relation.constraints[0].columns, ["ID"]);

        let function = schema
            .objects
            .iter()
            .find(|o| o.name == "FN")
            .expect("function FN");
        let Some(ObjectDetail::Routine(routine)) = &function.detail else {
            panic!("routine detail: {function:?}");
        };
        assert_eq!(routine.return_type.as_deref(), Some("NUMBER"));

        // Source only for objects the schema has.
        assert_eq!(schema.sources.len(), 1);
        assert_eq!(schema.sources[0].name, "FN");
        assert_eq!(
            schema.sources[0].text,
            "CREATE OR REPLACE FUNCTION fn RETURN NUMBER IS\nBEGIN RETURN 1; END;\n/"
        );

        let (_, names) = statements(2, Level::Names).await;
        let schema = names.schema("S").expect("schema S");
        assert!(schema.relations.is_empty() && schema.sources.is_empty());
        assert!(schema.objects.iter().all(|o| o.detail.is_none()));
    }
}
