//! Bulk load for PostgreSQL: per schema four queries for the objects (one
//! per catalog family: relations, triggers, routines, types), three for the
//! structure (columns, indexes, constraints) and three for the source text,
//! whatever the number of tables.
//!
//! Written from the PostgreSQL documentation (System Catalogs, System
//! Information Functions). Needs PostgreSQL 12 or later (`attgenerated`).

use super::{PostgresIntrospector, flag, on_delete_action, relation_kind, text_list};
use crate::schema::{
    ArgumentInfo, ArgumentMode, AttributeInfo, ColumnInfo, ConstraintInfo, ConstraintKind,
    GeneratedColumn, IdentityKind, IndexInfo, Level, ObjectDetail, ObjectInfo, ObjectKind,
    PartitionOf, RelationInfo, RoutineDetail, SchemaInfo, SchemaSnapshot, Scope, SequenceDetail,
    Snapshot, SourceInfo, TableDetail, TriggerDetail, TypeCategory, TypeDetail, ViewDetail,
    fetch_all, opt_text, text,
};
use crate::{Connection, Result, Row, Value};

const SCHEMAS: &str = "\
SELECT n.nspname, \
       (n.nspname = 'information_schema' OR n.nspname LIKE 'pg\\_%') AS is_system, \
       (n.nspname = current_schema()) AS is_current, \
       pg_catalog.pg_get_userbyid(n.nspowner), \
       pg_catalog.obj_description(n.oid, 'pg_namespace') \
FROM pg_catalog.pg_namespace n \
ORDER BY n.nspname";

/// Tables, views, materialized views, sequences and indexes, with the
/// partition parent and bounds of a partition and the parameters of a
/// sequence.
const RELATIONS: &str = "\
SELECT c.relname, c.relkind::text, d.description, pt.relname, \
       pg_catalog.pg_get_userbyid(c.relowner), c.relpersistence::text, \
       pn.nspname, pc.relname, pg_catalog.pg_get_expr(c.relpartbound, c.oid), \
       CASE WHEN c.relkind = 'p' THEN pg_catalog.pg_get_partkeydef(c.oid) END, \
       CASE WHEN c.relkind = 'm' THEN c.relispopulated END, \
       pg_catalog.format_type(sq.seqtypid, NULL), sq.seqstart::text, sq.seqmin::text, \
       sq.seqmax::text, sq.seqincrement::text, sq.seqcycle, sq.seqcache::int8 \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = c.oid AND d.classoid = 'pg_catalog.pg_class'::regclass AND d.objsubid = 0 \
LEFT JOIN pg_catalog.pg_index i ON i.indexrelid = c.oid \
LEFT JOIN pg_catalog.pg_class pt ON pt.oid = i.indrelid \
LEFT JOIN pg_catalog.pg_inherits inh ON inh.inhrelid = c.oid AND c.relispartition \
LEFT JOIN pg_catalog.pg_class pc ON pc.oid = inh.inhparent \
LEFT JOIN pg_catalog.pg_namespace pn ON pn.oid = pc.relnamespace \
LEFT JOIN pg_catalog.pg_sequence sq ON sq.seqrelid = c.oid \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm', 'S', 'i', 'I') \
ORDER BY c.relname";

const TRIGGERS: &str = "\
SELECT t.tgname, c.relname, d.description, t.tgtype::int4, t.tgenabled::text, \
       fn.nspname, p.proname, \
       CASE WHEN t.tgqual IS NOT NULL THEN pg_catalog.pg_get_triggerdef(t.oid, true) END \
FROM pg_catalog.pg_trigger t \
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid \
LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid = p.pronamespace \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = t.oid AND d.classoid = 'pg_catalog.pg_trigger'::regclass AND d.objsubid = 0 \
WHERE n.nspname = $1 AND NOT t.tgisinternal \
ORDER BY t.tgname, c.relname";

/// Argument types, names and modes come as arrays of the same length (all
/// arguments, `OUT` ones included).
const ROUTINES: &str = "\
SELECT p.proname, p.prokind::text, d.description, pg_catalog.pg_get_userbyid(p.proowner), \
       pg_catalog.pg_get_function_identity_arguments(p.oid), \
       pg_catalog.pg_get_function_result(p.oid), l.lanname, \
       p.provolatile::text, p.prosecdef, p.proisstrict, p.proparallel::text, \
       ARRAY(SELECT pg_catalog.format_type(u.t, NULL) \
             FROM unnest(COALESCE(p.proallargtypes, p.proargtypes::oid[])) \
                  WITH ORDINALITY u(t, ord) ORDER BY u.ord), \
       p.proargnames, p.proargmodes::text[], p.pronargdefaults::int4 \
FROM pg_catalog.pg_proc p \
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = p.oid AND d.classoid = 'pg_catalog.pg_proc'::regclass AND d.objsubid = 0 \
WHERE n.nspname = $1 AND p.prokind IN ('f', 'p', 'w') \
ORDER BY p.proname, p.oid";

const TYPES: &str = "\
SELECT t.typname, d.description, pg_catalog.pg_get_userbyid(t.typowner), t.typtype::text, \
       ARRAY(SELECT e.enumlabel::text FROM pg_catalog.pg_enum e \
             WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder), \
       ARRAY(SELECT a.attname::text FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum), \
       ARRAY(SELECT pg_catalog.format_type(a.atttypid, a.atttypmod) \
             FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum), \
       CASE t.typtype WHEN 'd' THEN pg_catalog.format_type(t.typbasetype, t.typtypmod) \
            WHEN 'r' THEN (SELECT pg_catalog.format_type(r.rngsubtype, NULL) \
                           FROM pg_catalog.pg_range r WHERE r.rngtypid = t.oid) END, \
       ARRAY(SELECT pg_catalog.pg_get_constraintdef(con.oid, true) \
             FROM pg_catalog.pg_constraint con \
             WHERE con.contypid = t.oid AND con.contype = 'c' ORDER BY con.conname), \
       t.typnotnull, t.typdefault \
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
SELECT c.relname, a.attname, a.attnum::int4, pg_catalog.format_type(a.atttypid, a.atttypmod), \
       a.attnotnull, pg_catalog.pg_get_expr(ad.adbin, ad.adrelid), a.attidentity::text, \
       d.description, a.attgenerated::text \
FROM pg_catalog.pg_attribute a \
JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
LEFT JOIN pg_catalog.pg_description d \
       ON d.objoid = a.attrelid AND d.classoid = 'pg_catalog.pg_class'::regclass \
      AND d.objsubid = a.attnum \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm') \
  AND a.attnum > 0 AND NOT a.attisdropped \
ORDER BY c.relname, a.attnum";

/// Key columns only in the second array (`INCLUDE` columns in the third).
/// The last array lists the key positions (from 0) that are expressions:
/// `indkey` holds 0 for them.
const INDEXES: &str = "\
SELECT tc.relname, ic.relname, i.indisunique, am.amname, \
       ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid, k + 1, true) \
             FROM generate_series(0, i.indnkeyatts - 1) k ORDER BY k), \
       pg_catalog.pg_get_expr(i.indpred, i.indrelid, true), \
       ARRAY(SELECT pg_catalog.pg_get_indexdef(i.indexrelid, k + 1, true) \
             FROM generate_series(i.indnkeyatts, i.indnatts - 1) k ORDER BY k), \
       ARRAY(SELECT k::int4 FROM generate_series(0, i.indnkeyatts - 1) k \
             WHERE i.indkey[k] = 0 ORDER BY k) \
FROM pg_catalog.pg_index i \
JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
JOIN pg_catalog.pg_class tc ON tc.oid = i.indrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
WHERE n.nspname = $1 \
ORDER BY tc.relname, ic.relname";

const CONSTRAINTS: &str = "\
SELECT c.relname, con.conname, con.contype::text, \
       ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       fn.nspname, fc.relname, \
       ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord) \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum \
             ORDER BY k.ord), \
       con.confdeltype::text, \
       CASE WHEN con.contype = 'c' THEN pg_catalog.pg_get_expr(con.conbin, con.conrelid, true) END, \
       con.confupdtype::text, con.confmatchtype::text, con.condeferrable, con.condeferred, \
       con.convalidated \
FROM pg_catalog.pg_constraint con \
JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
LEFT JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid \
LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
WHERE n.nspname = $1 AND con.contype IN ('p', 'u', 'f', 'c') \
ORDER BY c.relname, \
         CASE con.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'f' THEN 2 ELSE 3 END, con.conname";

const VIEW_SOURCES: &str = "\
SELECT c.relname, c.relkind::text, \
       pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname), \
       pg_catalog.pg_get_viewdef(c.oid, true) \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relkind IN ('v', 'm') \
ORDER BY c.relname";

const ROUTINE_SOURCES: &str = "\
SELECT p.proname, p.prokind::text, pg_catalog.pg_get_function_identity_arguments(p.oid), \
       pg_catalog.pg_get_functiondef(p.oid) \
FROM pg_catalog.pg_proc p \
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
WHERE n.nspname = $1 AND p.prokind IN ('f', 'p', 'w') \
ORDER BY p.proname, p.oid";

const TRIGGER_SOURCES: &str = "\
SELECT t.tgname, c.relname, pg_catalog.pg_get_triggerdef(t.oid, true) \
FROM pg_catalog.pg_trigger t \
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND NOT t.tgisinternal \
ORDER BY t.tgname, c.relname";

pub(super) async fn load(
    intro: &PostgresIntrospector<'_>,
    scope: &Scope,
    level: Level,
) -> Result<Snapshot> {
    let conn = intro.conn;
    let mut schemas = Vec::new();
    for r in fetch_all(conn, SCHEMAS, &[]).await? {
        let info = SchemaInfo {
            name: text(&r[0]),
            is_system: flag(&r[1]),
            is_current: flag(&r[2]),
            owner: opt_text(&r[3]),
            comment: opt_text(&r[4]),
        };
        if scope.selects(&info) {
            schemas.push(load_schema(conn, info, level).await?);
        }
    }
    Ok(Snapshot {
        level,
        token: None,
        schemas,
    })
}

async fn load_schema(
    conn: &dyn Connection,
    info: SchemaInfo,
    level: Level,
) -> Result<SchemaSnapshot> {
    let schema = info.name.clone();
    let arg = [Value::from(schema.as_str())];
    let structure = level >= Level::Structure;
    let mut objects = Vec::new();

    for r in fetch_all(conn, RELATIONS, &arg).await? {
        let code = text(&r[1]);
        let Some(kind) = relation_kind(&code) else {
            continue;
        };
        let detail = relation_detail(&code, &r);
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: text(&r[0]),
            kind,
            parent: (kind == ObjectKind::Index)
                .then(|| opt_text(&r[3]))
                .flatten(),
            comment: opt_text(&r[2]),
            owner: opt_text(&r[4]),
            detail,
        });
    }

    for r in fetch_all(conn, TRIGGERS, &arg).await? {
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: text(&r[0]),
            kind: ObjectKind::Trigger,
            parent: opt_text(&r[1]),
            comment: opt_text(&r[2]),
            owner: None,
            detail: Some(ObjectDetail::Trigger(trigger_detail(&r))),
        });
    }

    for r in fetch_all(conn, ROUTINES, &arg).await? {
        let kind = if text(&r[1]) == "p" {
            ObjectKind::Procedure
        } else {
            ObjectKind::Function
        };
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: text(&r[0]),
            kind,
            parent: None,
            comment: opt_text(&r[2]),
            owner: opt_text(&r[3]),
            detail: Some(ObjectDetail::Routine(routine_detail(&r))),
        });
    }

    for r in fetch_all(conn, TYPES, &arg).await? {
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: text(&r[0]),
            kind: ObjectKind::Type,
            parent: None,
            comment: opt_text(&r[1]),
            owner: opt_text(&r[2]),
            detail: Some(ObjectDetail::Type(type_detail(&r))),
        });
    }

    if !structure {
        // Names only: the partition tree is the one detail that shapes the
        // tree of objects.
        for object in &mut objects {
            object.detail = match object.detail.take() {
                Some(ObjectDetail::Table(t)) if t.partitioned || t.partition_of.is_some() => {
                    Some(ObjectDetail::Table(TableDetail {
                        partitioned: t.partitioned,
                        partition_key: t.partition_key,
                        partition_of: t.partition_of,
                        ..TableDetail::default()
                    }))
                }
                _ => None,
            };
        }
    }

    let mut relations = Vec::new();
    if structure {
        relations = relations_of(conn, &arg, &objects).await?;
    }

    let mut sources = Vec::new();
    if level >= Level::Full {
        sources = read_sources(conn, &arg).await?;
    }

    Ok(SchemaSnapshot {
        info,
        level,
        objects,
        relations,
        sources,
    })
}

fn relation_detail(code: &str, r: &Row) -> Option<ObjectDetail> {
    match code {
        "r" | "p" | "f" => Some(ObjectDetail::Table(TableDetail {
            temporary: text(&r[5]) == "t",
            unlogged: text(&r[5]) == "u",
            foreign: code == "f",
            partitioned: code == "p",
            partition_key: opt_text(&r[9]),
            partition_of: opt_text(&r[7]).map(|table| PartitionOf {
                schema: text(&r[6]),
                table,
                bounds: opt_text(&r[8]),
            }),
            ..TableDetail::default()
        })),
        "v" | "m" => Some(ObjectDetail::View(ViewDetail {
            populated: match &r[10] {
                Value::Bool(b) => Some(*b),
                _ => None,
            },
        })),
        "S" => Some(ObjectDetail::Sequence(SequenceDetail {
            data_type: opt_text(&r[11]),
            start: opt_text(&r[12]),
            min: opt_text(&r[13]),
            max: opt_text(&r[14]),
            increment: opt_text(&r[15]),
            cycle: flag(&r[16]),
            cache: r[17].as_i64().filter(|c| *c > 1),
            ordered: false,
        })),
        _ => None,
    }
}

/// `tgtype` bits: 1 row, 2 before, 4 insert, 8 delete, 16 update,
/// 32 truncate, 64 instead of.
fn trigger_detail(r: &Row) -> TriggerDetail {
    let bits = r[3].as_i64().unwrap_or(0);
    let mut events = Vec::new();
    for (bit, name) in [
        (4, "INSERT"),
        (8, "DELETE"),
        (16, "UPDATE"),
        (32, "TRUNCATE"),
    ] {
        if bits & bit != 0 {
            events.push(name.to_owned());
        }
    }
    let timing = if bits & 64 != 0 {
        "INSTEAD OF"
    } else if bits & 2 != 0 {
        "BEFORE"
    } else {
        "AFTER"
    };
    TriggerDetail {
        events,
        timing: Some(timing.to_owned()),
        each_row: bits & 1 != 0,
        function: opt_text(&r[6]).map(|f| match opt_text(&r[5]) {
            Some(schema) => format!("{schema}.{f}"),
            None => f,
        }),
        condition: opt_text(&r[7]).and_then(|definition| when_condition(&definition)),
        disabled: text(&r[4]) == "D",
    }
}

/// The condition of `CREATE TRIGGER ... WHEN (condition) EXECUTE ...`.
/// `pg_get_expr` cannot print it: the condition refers to `OLD` and `NEW`.
fn when_condition(definition: &str) -> Option<String> {
    let start = definition.find(" WHEN (")? + " WHEN (".len();
    let end = definition.rfind(") EXECUTE ")?;
    (end >= start).then(|| definition[start..end].to_owned())
}

fn routine_detail(r: &Row) -> RoutineDetail {
    let types = text_list(&r[11]);
    let names = text_list(&r[12]);
    let modes = text_list(&r[13]);
    let defaults = usize::try_from(r[14].as_i64().unwrap_or(0)).unwrap_or(0);
    let mode_of = |i: usize| match modes.get(i).map(String::as_str) {
        Some("o") => ArgumentMode::Out,
        Some("b") => ArgumentMode::InOut,
        Some("v") => ArgumentMode::Variadic,
        Some("t") => ArgumentMode::Table,
        _ => ArgumentMode::In,
    };
    // The last `pronargdefaults` input arguments have defaults.
    let inputs = (0..types.len())
        .filter(|i| {
            matches!(
                mode_of(*i),
                ArgumentMode::In | ArgumentMode::InOut | ArgumentMode::Variadic
            )
        })
        .count();
    let mut input_seen = 0;
    let arguments = types
        .iter()
        .enumerate()
        .map(|(i, data_type)| {
            let mode = mode_of(i);
            let is_input = matches!(
                mode,
                ArgumentMode::In | ArgumentMode::InOut | ArgumentMode::Variadic
            );
            let has_default = is_input && input_seen >= inputs.saturating_sub(defaults);
            if is_input {
                input_seen += 1;
            }
            ArgumentInfo {
                name: names.get(i).filter(|n| !n.is_empty()).cloned(),
                data_type: data_type.clone(),
                mode,
                has_default,
            }
        })
        .collect();

    let mut characteristics = Vec::new();
    if text(&r[1]) == "w" {
        characteristics.push("WINDOW".to_owned());
    }
    match text(&r[7]).as_str() {
        "i" => characteristics.push("IMMUTABLE".to_owned()),
        "s" => characteristics.push("STABLE".to_owned()),
        _ => {}
    }
    if flag(&r[8]) {
        characteristics.push("SECURITY DEFINER".to_owned());
    }
    if flag(&r[9]) {
        characteristics.push("STRICT".to_owned());
    }
    match text(&r[10]).as_str() {
        "s" => characteristics.push("PARALLEL SAFE".to_owned()),
        "r" => characteristics.push("PARALLEL RESTRICTED".to_owned()),
        _ => {}
    }

    RoutineDetail {
        signature: text(&r[4]),
        arguments,
        return_type: opt_text(&r[5]),
        language: opt_text(&r[6]),
        characteristics,
    }
}

fn type_detail(r: &Row) -> TypeDetail {
    let category = match text(&r[3]).as_str() {
        "e" => TypeCategory::Enum,
        "c" => TypeCategory::Composite,
        "d" => TypeCategory::Domain,
        "r" => TypeCategory::Range,
        _ => TypeCategory::Other,
    };
    let attribute_names = text_list(&r[5]);
    let attribute_types = text_list(&r[6]);
    TypeDetail {
        category,
        labels: text_list(&r[4]),
        attributes: attribute_names
            .into_iter()
            .zip(attribute_types)
            .map(|(name, data_type)| AttributeInfo { name, data_type })
            .collect(),
        base_type: opt_text(&r[7]),
        constraints: text_list(&r[8]),
        not_null: category == TypeCategory::Domain && flag(&r[9]),
        default: opt_text(&r[10]),
    }
}

/// Columns, indexes and constraints of every table, view and materialized
/// view of the schema, in the order of the objects.
async fn relations_of(
    conn: &dyn Connection,
    arg: &[Value],
    objects: &[ObjectInfo],
) -> Result<Vec<RelationInfo>> {
    let mut relations: Vec<RelationInfo> = objects
        .iter()
        .filter(|o| crate::schema::has_columns(o.kind))
        .map(|o| RelationInfo {
            name: o.name.clone(),
            ..RelationInfo::default()
        })
        .collect();
    let index: std::collections::HashMap<String, usize> = relations
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.clone(), i))
        .collect();

    for r in fetch_all(conn, COLUMNS, arg).await? {
        let Some(&at) = index.get(&text(&r[0])) else {
            continue;
        };
        let default = opt_text(&r[5]);
        let identity = text(&r[6]);
        let generated = text(&r[8]);
        let serial = default
            .as_deref()
            .is_some_and(|d| d.starts_with("nextval("));
        relations[at].columns.push(ColumnInfo {
            name: text(&r[1]),
            position: r[2].as_i64().unwrap_or(0) as u32,
            data_type: text(&r[3]),
            nullable: !flag(&r[4]),
            auto_increment: matches!(identity.as_str(), "a" | "d") || serial,
            comment: opt_text(&r[7]),
            identity: match identity.as_str() {
                "a" => Some(IdentityKind::Always),
                "d" => Some(IdentityKind::ByDefault),
                _ => None,
            },
            // `s`: stored; `v` (PostgreSQL 18): virtual. The expression is
            // the column's default expression.
            generated: matches!(generated.as_str(), "s" | "v").then(|| GeneratedColumn {
                expression: default.clone(),
                stored: generated == "s",
            }),
            default,
        });
    }

    for r in fetch_all(conn, INDEXES, arg).await? {
        let Some(&at) = index.get(&text(&r[0])) else {
            continue;
        };
        relations[at].indexes.push(IndexInfo {
            name: text(&r[1]),
            unique: flag(&r[2]),
            columns: text_list(&r[4]),
            method: opt_text(&r[3]),
            predicate: opt_text(&r[5]),
            include: text_list(&r[6]),
            expressions: match &r[7] {
                Value::Array(items) => items
                    .iter()
                    .filter_map(|v| v.as_i64())
                    .map(|k| usize::try_from(k).unwrap_or(0))
                    .collect(),
                _ => Vec::new(),
            },
        });
    }

    for r in fetch_all(conn, CONSTRAINTS, arg).await? {
        let Some(&at) = index.get(&text(&r[0])) else {
            continue;
        };
        let kind = match text(&r[2]).as_str() {
            "p" => ConstraintKind::PrimaryKey,
            "u" => ConstraintKind::Unique,
            "f" => ConstraintKind::ForeignKey,
            "c" => ConstraintKind::Check,
            _ => continue,
        };
        let is_fk = kind == ConstraintKind::ForeignKey;
        let action = |v: &Value| {
            is_fk
                .then(|| on_delete_action(&text(v)).map(str::to_owned))
                .flatten()
        };
        relations[at].constraints.push(ConstraintInfo {
            name: text(&r[1]),
            kind,
            columns: text_list(&r[3]),
            ref_schema: is_fk.then(|| opt_text(&r[4])).flatten(),
            ref_table: is_fk.then(|| opt_text(&r[5])).flatten(),
            ref_columns: if is_fk { text_list(&r[6]) } else { Vec::new() },
            on_delete: action(&r[7]),
            check: opt_text(&r[8]),
            on_update: action(&r[9]),
            match_type: is_fk
                .then(|| match text(&r[10]).as_str() {
                    "f" => Some("FULL"),
                    "p" => Some("PARTIAL"),
                    "s" => Some("SIMPLE"),
                    _ => None,
                })
                .flatten()
                .map(str::to_owned),
            deferrable: flag(&r[11]),
            initially_deferred: flag(&r[12]),
            disabled: false,
            not_validated: !flag(&r[13]),
        });
    }
    Ok(relations)
}

/// The texts `source` returns, one per object (per overload for routines,
/// per table for triggers of the same name).
async fn read_sources(conn: &dyn Connection, arg: &[Value]) -> Result<Vec<SourceInfo>> {
    let mut out = Vec::new();
    for r in fetch_all(conn, VIEW_SOURCES, arg).await? {
        let (kind, create) = if text(&r[1]) == "m" {
            (ObjectKind::MaterializedView, "CREATE MATERIALIZED VIEW")
        } else {
            (ObjectKind::View, "CREATE OR REPLACE VIEW")
        };
        out.push(SourceInfo {
            name: text(&r[0]),
            kind,
            signature: String::new(),
            text: format!("{create} {} AS\n{}", text(&r[2]), text(&r[3])),
        });
    }
    for r in fetch_all(conn, ROUTINE_SOURCES, arg).await? {
        let Some(body) = opt_text(&r[3]) else {
            continue;
        };
        out.push(SourceInfo {
            name: text(&r[0]),
            kind: if text(&r[1]) == "p" {
                ObjectKind::Procedure
            } else {
                ObjectKind::Function
            },
            signature: text(&r[2]),
            text: body,
        });
    }
    for r in fetch_all(conn, TRIGGER_SOURCES, arg).await? {
        let Some(body) = opt_text(&r[2]) else {
            continue;
        };
        out.push(SourceInfo {
            name: text(&r[0]),
            kind: ObjectKind::Trigger,
            signature: text(&r[1]),
            text: body,
        });
    }
    Ok(out)
}
