//! Bulk load for PostgreSQL: per schema four queries for the objects (one
//! per catalog family: relations, triggers, routines, types), three for the
//! structure (columns, indexes, constraints) and three for the source text,
//! whatever the number of tables.
//!
//! Written from the PostgreSQL documentation (System Catalogs, System
//! Information Functions). Needs PostgreSQL 12 or later (`attgenerated`).

use super::{array_literal, flag, on_delete_action, relation_kind, text_list};
use crate::schema::change::{SchemaStamps, Stamp, Units};
use crate::schema::{
    ArgumentInfo, ArgumentMode, AttributeInfo, ColumnInfo, ConstraintInfo, ConstraintKind,
    GeneratedColumn, IdentityKind, IndexInfo, Level, ObjectDetail, ObjectInfo, ObjectKind,
    PartitionOf, RelationInfo, RoutineDetail, SchemaInfo, SchemaSnapshot, SequenceDetail,
    SourceInfo, TableDetail, TriggerDetail, TypeCategory, TypeDetail, ViewDetail, fetch_all, text,
};
use crate::schema::{Cells, restrict};
use crate::{Connection, Error, ErrorKind, Result, Value};

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
/*only:(c.relname = ANY(@) OR pt.relname = ANY(@))*/ \
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
/*only:c.relname = ANY(@)*/ \
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
/*only:p.proname = ANY(@)*/ \
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
/*only:t.typname = ANY(@)*/ \
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
/*only:c.relname = ANY(@)*/ \
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
/*only:tc.relname = ANY(@)*/ \
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
/*only:c.relname = ANY(@)*/ \
ORDER BY c.relname, \
         CASE con.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'f' THEN 2 ELSE 3 END, con.conname";

const VIEW_SOURCES: &str = "\
SELECT c.relname, c.relkind::text, \
       pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname), \
       pg_catalog.pg_get_viewdef(c.oid, true) \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relkind IN ('v', 'm') \
/*only:c.relname = ANY(@)*/ \
ORDER BY c.relname";

const ROUTINE_SOURCES: &str = "\
SELECT p.proname, p.prokind::text, pg_catalog.pg_get_function_identity_arguments(p.oid), \
       pg_catalog.pg_get_functiondef(p.oid) \
FROM pg_catalog.pg_proc p \
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
WHERE n.nspname = $1 AND p.prokind IN ('f', 'p', 'w') \
/*only:p.proname = ANY(@)*/ \
ORDER BY p.proname, p.oid";

const TRIGGER_SOURCES: &str = "\
SELECT t.tgname, c.relname, pg_catalog.pg_get_triggerdef(t.oid, true) \
FROM pg_catalog.pg_trigger t \
JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND NOT t.tgisinternal \
/*only:c.relname = ANY(@)*/ \
ORDER BY t.tgname, c.relname";

/// The rows of a query restricted to `names`: every row when `None`, none
/// when the list is empty.
async fn rows(
    conn: &dyn Connection,
    name: &'static str,
    sql: &str,
    schema: &str,
    names: Option<&[String]>,
) -> Result<Vec<Cells>> {
    match names {
        None => fetch_all(conn, name, sql, &[Value::from(schema)]).await,
        Some([]) => Ok(Vec::new()),
        Some(list) => {
            let sql = restrict(sql, "$2::text[]");
            let args = [Value::from(schema), Value::from(array_literal(list))];
            fetch_all(conn, name, &sql, &args).await
        }
    }
}

/// Every schema with its owner and comment.
pub(super) async fn schema_infos(conn: &dyn Connection) -> Result<Vec<SchemaInfo>> {
    let mut schemas = Vec::new();
    for r in fetch_all(conn, "schemas", SCHEMAS, &[]).await? {
        schemas.push(SchemaInfo {
            name: r.text(0)?,
            is_system: flag(r.value(1)?),
            is_current: flag(r.value(2)?),
            owner: r.opt_text(3)?,
            comment: r.opt_text(4)?,
        });
    }
    Ok(schemas)
}

/// A stamp of the schema row (owner, comment).
const SCHEMA_STAMP: &str = "\
SELECT pg_catalog.hashtextextended(pg_catalog.concat_ws(':', n.oid::text, n.xmin::text, \
         (SELECT COALESCE(sum(pg_catalog.hashtextextended( \
                    d.objsubid::text || ':' || d.xmin::text, 0)::numeric), 0) \
          FROM pg_catalog.pg_description d \
          WHERE d.objoid = n.oid AND d.classoid = 'pg_catalog.pg_namespace'::regclass)), 0)::text \
FROM pg_catalog.pg_namespace n WHERE n.nspname = $1";

/// A stamp per table, view, materialized view and sequence that covers
/// everything the load reads of it: the row versions (`xmin`) of its catalog
/// row and of its columns, defaults, constraints, indexes (and their rows),
/// triggers, comments and sequence parameters. A row that was added,
/// replaced or removed changes the sum; the rows PostgreSQL updates in place
/// (statistics) do not.
const RELATION_STAMPS: &str = "\
SELECT c.relname, c.relkind::text, \
  pg_catalog.hashtextextended(pg_catalog.concat_ws(':', c.oid::text, c.xmin::text, \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(a.attnum::text || ':' || a.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_attribute a WHERE a.attrelid = c.oid AND a.attnum > 0), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(x.adnum::text || ':' || x.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_attrdef x WHERE x.adrelid = c.oid), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(k.oid::text || ':' || k.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_constraint k WHERE k.conrelid = c.oid), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended( \
              i.indexrelid::text || ':' || i.xmin::text || ':' || ic.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
       WHERE i.indrelid = c.oid), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(t.oid::text || ':' || t.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_trigger t WHERE t.tgrelid = c.oid AND NOT t.tgisinternal), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(d.objsubid::text || ':' || d.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_description d \
       WHERE d.objoid = c.oid AND d.classoid = 'pg_catalog.pg_class'::regclass), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(s.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_sequence s WHERE s.seqrelid = c.oid)), 0)::text \
FROM pg_catalog.pg_class c \
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm', 'S')";

/// A stamp per routine overload: its row and its comment.
const ROUTINE_STAMPS: &str = "\
SELECT p.proname, p.prokind::text, pg_catalog.pg_get_function_identity_arguments(p.oid), \
  pg_catalog.hashtextextended(pg_catalog.concat_ws(':', p.oid::text, p.xmin::text, \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(d.objsubid::text || ':' || d.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_description d \
       WHERE d.objoid = p.oid AND d.classoid = 'pg_catalog.pg_proc'::regclass)), 0)::text \
FROM pg_catalog.pg_proc p \
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
WHERE n.nspname = $1 AND p.prokind IN ('f', 'p', 'w')";

/// A stamp per type: its row, enum labels, attributes of a composite, domain
/// constraints, range and comment.
const TYPE_STAMPS: &str = "\
SELECT t.typname, \
  pg_catalog.hashtextextended(pg_catalog.concat_ws(':', t.oid::text, t.xmin::text, \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(e.oid::text || ':' || e.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_enum e WHERE e.enumtypid = t.oid), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(a.attnum::text || ':' || a.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND t.typrelid <> 0), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(k.oid::text || ':' || k.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_constraint k WHERE k.contypid = t.oid), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(r.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_range r WHERE r.rngtypid = t.oid), \
    (SELECT COALESCE(sum(pg_catalog.hashtextextended(d.objsubid::text || ':' || d.xmin::text, 0)::numeric), 0) \
       FROM pg_catalog.pg_description d \
       WHERE d.objoid = t.oid AND d.classoid = 'pg_catalog.pg_type'::regclass)), 0)::text \
FROM pg_catalog.pg_type t \
JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
WHERE n.nspname = $1 \
  AND (t.typtype IN ('e', 'd', 'r') \
       OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class rc \
                                       WHERE rc.oid = t.typrelid AND rc.relkind = 'c')))";

/// The change query of a schema: one stamp per object, in four statements.
pub(super) async fn stamps(
    conn: &dyn Connection,
    info: &SchemaInfo,
    level: Level,
) -> Result<SchemaStamps> {
    retry_vanished(|| stamps_once(conn, info, level)).await
}

async fn stamps_once(
    conn: &dyn Connection,
    info: &SchemaInfo,
    level: Level,
) -> Result<SchemaStamps> {
    let arg = [Value::from(info.name.as_str())];
    let row = fetch_all(conn, "schema_stamp", SCHEMA_STAMP, &arg)
        .await?
        .first()
        .map(|r| r.text(0))
        .transpose()?
        .unwrap_or_default();
    let mut objects = Vec::new();
    for r in fetch_all(conn, "relation_stamps", RELATION_STAMPS, &arg).await? {
        let Some(kind) = relation_kind(&r.text(1)?) else {
            continue;
        };
        objects.push(Stamp(kind, r.text(0)?, String::new(), None, r.text(2)?));
    }
    for r in fetch_all(conn, "routine_stamps", ROUTINE_STAMPS, &arg).await? {
        let kind = if r.text(1)? == "p" {
            ObjectKind::Procedure
        } else {
            ObjectKind::Function
        };
        objects.push(Stamp(kind, r.text(0)?, r.text(2)?, None, r.text(3)?));
    }
    for r in fetch_all(conn, "type_stamps", TYPE_STAMPS, &arg).await? {
        objects.push(Stamp(
            ObjectKind::Type,
            r.text(0)?,
            String::new(),
            None,
            r.text(1)?,
        ));
    }
    Ok(SchemaStamps {
        level,
        system: info.is_system,
        row,
        objects,
    })
}

/// How many times a schema is loaded when objects vanish under the load.
const LOAD_ATTEMPTS: usize = 3;

/// The catalog functions (`pg_get_indexdef`, `pg_get_viewdef`, ...) look
/// objects up by OID at the time they run, not in the statement's snapshot:
/// an object dropped by a concurrent session after the statement started
/// fails it with `XX000 could not open relation with OID ...` (or `cache
/// lookup failed`) instead of vanishing from the result.
fn vanished_object(err: &Error) -> bool {
    err.kind == ErrorKind::Query
        && match err.code.as_deref() {
            Some("XX000") => {
                err.message.contains("could not open relation with OID")
                    || err.message.contains("cache lookup failed")
            }
            Some("42P01") => true,
            _ => false,
        }
}

/// Runs `attempt` again when an object was dropped during it: the queries
/// are bulk, so the dropped object cannot be skipped on its own, and the
/// second pass no longer sees it. The attempts are limited, so a persistent
/// catalog fault is still reported. A schema that is gone itself has no
/// objects and comes back empty.
async fn retry_vanished<T, F, Fut>(attempt: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempts = 1;
    loop {
        match attempt().await {
            Err(err) if attempts < LOAD_ATTEMPTS && vanished_object(&err) => attempts += 1,
            other => return other,
        }
    }
}

/// Reads a schema, again when an object vanished under the read. `units`
/// restricts the read to some objects and what belongs to them.
pub(super) async fn load_schema_retrying(
    conn: &dyn Connection,
    info: SchemaInfo,
    level: Level,
    units: Option<&Units>,
) -> Result<SchemaSnapshot> {
    retry_vanished(|| load_schema(conn, info.clone(), level, units)).await
}

async fn load_schema(
    conn: &dyn Connection,
    info: SchemaInfo,
    level: Level,
    units: Option<&Units>,
) -> Result<SchemaSnapshot> {
    let schema = info.name.clone();
    let structure = level >= Level::Structure;
    let mut objects = Vec::new();
    let relation_names = units.map(|u| u.relations.as_slice());
    let routine_names = units.map(|u| u.routines.as_slice());
    let type_names = units.map(|u| u.types.as_slice());

    for r in rows(conn, "relations", RELATIONS, &schema, relation_names).await? {
        let code = r.text(1)?;
        let Some(kind) = relation_kind(&code) else {
            continue;
        };
        let detail = relation_detail(&code, &r)?;
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: r.text(0)?,
            kind,
            parent: if kind == ObjectKind::Index {
                r.opt_text(3)?
            } else {
                None
            },
            comment: r.opt_text(2)?,
            owner: r.opt_text(4)?,
            detail,
        });
    }

    for r in rows(conn, "triggers", TRIGGERS, &schema, relation_names).await? {
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: r.text(0)?,
            kind: ObjectKind::Trigger,
            parent: r.opt_text(1)?,
            comment: r.opt_text(2)?,
            owner: None,
            detail: Some(ObjectDetail::Trigger(trigger_detail(&r)?)),
        });
    }

    for r in rows(conn, "routines", ROUTINES, &schema, routine_names).await? {
        let kind = if r.text(1)? == "p" {
            ObjectKind::Procedure
        } else {
            ObjectKind::Function
        };
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: r.text(0)?,
            kind,
            parent: None,
            comment: r.opt_text(2)?,
            owner: r.opt_text(3)?,
            detail: Some(ObjectDetail::Routine(routine_detail(&r)?)),
        });
    }

    for r in rows(conn, "types", TYPES, &schema, type_names).await? {
        objects.push(ObjectInfo {
            schema: schema.clone(),
            name: r.text(0)?,
            kind: ObjectKind::Type,
            parent: None,
            comment: r.opt_text(1)?,
            owner: r.opt_text(2)?,
            detail: Some(ObjectDetail::Type(type_detail(&r)?)),
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
        relations = relations_of(conn, &schema, relation_names, &objects).await?;
    }

    let mut sources = Vec::new();
    if level >= Level::Full {
        sources = read_sources(conn, &schema, relation_names, routine_names).await?;
    }

    Ok(SchemaSnapshot {
        info,
        level,
        objects,
        relations,
        sources,
    })
}

fn relation_detail(code: &str, r: &Cells) -> Result<Option<ObjectDetail>> {
    Ok(match code {
        "r" | "p" | "f" => {
            let partition_of = match r.opt_text(7)? {
                Some(table) => Some(PartitionOf {
                    schema: r.text(6)?,
                    table,
                    bounds: r.opt_text(8)?,
                }),
                None => None,
            };
            Some(ObjectDetail::Table(TableDetail {
                temporary: r.text(5)? == "t",
                unlogged: r.text(5)? == "u",
                foreign: code == "f",
                partitioned: code == "p",
                partition_key: r.opt_text(9)?,
                partition_of,
                ..TableDetail::default()
            }))
        }
        "v" | "m" => Some(ObjectDetail::View(ViewDetail {
            populated: match r.value(10)? {
                Value::Bool(b) => Some(*b),
                _ => None,
            },
        })),
        "S" => Some(ObjectDetail::Sequence(SequenceDetail {
            data_type: r.opt_text(11)?,
            start: r.opt_text(12)?,
            min: r.opt_text(13)?,
            max: r.opt_text(14)?,
            increment: r.opt_text(15)?,
            cycle: flag(r.value(16)?),
            cache: r.int(17)?.filter(|c| *c > 1),
            ordered: false,
        })),
        _ => None,
    })
}

/// `tgtype` bits: 1 row, 2 before, 4 insert, 8 delete, 16 update,
/// 32 truncate, 64 instead of.
fn trigger_detail(r: &Cells) -> Result<TriggerDetail> {
    let bits = r.int(3)?.unwrap_or(0);
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
    let function = match r.opt_text(6)? {
        Some(f) => Some(match r.opt_text(5)? {
            Some(schema) => format!("{schema}.{f}"),
            None => f,
        }),
        None => None,
    };
    Ok(TriggerDetail {
        events,
        timing: Some(timing.to_owned()),
        each_row: bits & 1 != 0,
        function,
        condition: r
            .opt_text(7)?
            .and_then(|definition| when_condition(&definition)),
        disabled: r.text(4)? == "D",
    })
}

/// The condition of `CREATE TRIGGER ... WHEN (condition) EXECUTE ...`.
/// `pg_get_expr` cannot print it: the condition refers to `OLD` and `NEW`.
fn when_condition(definition: &str) -> Option<String> {
    let start = definition.find(" WHEN (")? + " WHEN (".len();
    let end = definition.rfind(") EXECUTE ")?;
    (end >= start).then(|| definition[start..end].to_owned())
}

fn routine_detail(r: &Cells) -> Result<RoutineDetail> {
    let types = text_list(r.value(11)?);
    let names = text_list(r.value(12)?);
    let modes = text_list(r.value(13)?);
    let defaults = usize::try_from(r.int(14)?.unwrap_or(0)).unwrap_or(0);
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
    if r.text(1)? == "w" {
        characteristics.push("WINDOW".to_owned());
    }
    match r.text(7)?.as_str() {
        "i" => characteristics.push("IMMUTABLE".to_owned()),
        "s" => characteristics.push("STABLE".to_owned()),
        _ => {}
    }
    if flag(r.value(8)?) {
        characteristics.push("SECURITY DEFINER".to_owned());
    }
    if flag(r.value(9)?) {
        characteristics.push("STRICT".to_owned());
    }
    match r.text(10)?.as_str() {
        "s" => characteristics.push("PARALLEL SAFE".to_owned()),
        "r" => characteristics.push("PARALLEL RESTRICTED".to_owned()),
        _ => {}
    }

    Ok(RoutineDetail {
        signature: r.text(4)?,
        arguments,
        return_type: r.opt_text(5)?,
        language: r.opt_text(6)?,
        characteristics,
    })
}

fn type_detail(r: &Cells) -> Result<TypeDetail> {
    let category = match r.text(3)?.as_str() {
        "e" => TypeCategory::Enum,
        "c" => TypeCategory::Composite,
        "d" => TypeCategory::Domain,
        "r" => TypeCategory::Range,
        _ => TypeCategory::Other,
    };
    let attribute_names = text_list(r.value(5)?);
    let attribute_types = text_list(r.value(6)?);
    Ok(TypeDetail {
        category,
        labels: text_list(r.value(4)?),
        attributes: attribute_names
            .into_iter()
            .zip(attribute_types)
            .map(|(name, data_type)| AttributeInfo { name, data_type })
            .collect(),
        base_type: r.opt_text(7)?,
        constraints: text_list(r.value(8)?),
        not_null: category == TypeCategory::Domain && flag(r.value(9)?),
        default: r.opt_text(10)?,
    })
}

/// Columns, indexes and constraints of every table, view and materialized
/// view of the schema, in the order of the objects.
async fn relations_of(
    conn: &dyn Connection,
    schema: &str,
    names: Option<&[String]>,
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

    for r in rows(conn, "columns", COLUMNS, schema, names).await? {
        let Some(&at) = index.get(&r.text(0)?) else {
            continue;
        };
        let default = r.opt_text(5)?;
        let identity = r.text(6)?;
        let generated = r.text(8)?;
        let serial = default
            .as_deref()
            .is_some_and(|d| d.starts_with("nextval("));
        relations[at].columns.push(ColumnInfo {
            name: r.text(1)?,
            position: r.int(2)?.unwrap_or(0) as u32,
            data_type: r.text(3)?,
            nullable: !flag(r.value(4)?),
            auto_increment: matches!(identity.as_str(), "a" | "d") || serial,
            comment: r.opt_text(7)?,
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

    for r in rows(conn, "indexes", INDEXES, schema, names).await? {
        let Some(&at) = index.get(&r.text(0)?) else {
            continue;
        };
        relations[at].indexes.push(IndexInfo {
            name: r.text(1)?,
            unique: flag(r.value(2)?),
            columns: text_list(r.value(4)?),
            method: r.opt_text(3)?,
            predicate: r.opt_text(5)?,
            include: text_list(r.value(6)?),
            expressions: match r.value(7)? {
                Value::Array(items) => items
                    .iter()
                    .filter_map(|v| v.as_i64())
                    .map(|k| usize::try_from(k).unwrap_or(0))
                    .collect(),
                _ => Vec::new(),
            },
        });
    }

    for r in rows(conn, "constraints", CONSTRAINTS, schema, names).await? {
        let Some(&at) = index.get(&r.text(0)?) else {
            continue;
        };
        let kind = match r.text(2)?.as_str() {
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
            name: r.text(1)?,
            kind,
            columns: text_list(r.value(3)?),
            ref_schema: if is_fk { r.opt_text(4)? } else { None },
            ref_table: if is_fk { r.opt_text(5)? } else { None },
            ref_columns: if is_fk {
                text_list(r.value(6)?)
            } else {
                Vec::new()
            },
            on_delete: action(r.value(7)?),
            check: r.opt_text(8)?,
            on_update: action(r.value(9)?),
            match_type: if is_fk {
                match r.text(10)?.as_str() {
                    "f" => Some("FULL"),
                    "p" => Some("PARTIAL"),
                    "s" => Some("SIMPLE"),
                    _ => None,
                }
                .map(str::to_owned)
            } else {
                None
            },
            deferrable: flag(r.value(11)?),
            initially_deferred: flag(r.value(12)?),
            disabled: false,
            not_validated: !flag(r.value(13)?),
        });
    }
    Ok(relations)
}

/// The texts `source` returns, one per object (per overload for routines,
/// per table for triggers of the same name).
async fn read_sources(
    conn: &dyn Connection,
    schema: &str,
    relation_names: Option<&[String]>,
    routine_names: Option<&[String]>,
) -> Result<Vec<SourceInfo>> {
    let mut out = Vec::new();
    for r in rows(conn, "view_sources", VIEW_SOURCES, schema, relation_names).await? {
        let (kind, create) = if r.text(1)? == "m" {
            (ObjectKind::MaterializedView, "CREATE MATERIALIZED VIEW")
        } else {
            (ObjectKind::View, "CREATE OR REPLACE VIEW")
        };
        out.push(SourceInfo {
            name: r.text(0)?,
            kind,
            signature: String::new(),
            text: format!("{create} {} AS\n{}", r.text(2)?, r.text(3)?),
        });
    }
    for r in rows(
        conn,
        "routine_sources",
        ROUTINE_SOURCES,
        schema,
        routine_names,
    )
    .await?
    {
        let Some(body) = r.opt_text(3)? else {
            continue;
        };
        out.push(SourceInfo {
            name: r.text(0)?,
            kind: if r.text(1)? == "p" {
                ObjectKind::Procedure
            } else {
                ObjectKind::Function
            },
            signature: r.text(2)?,
            text: body,
        });
    }
    for r in rows(
        conn,
        "trigger_sources",
        TRIGGER_SOURCES,
        schema,
        relation_names,
    )
    .await?
    {
        let Some(body) = r.opt_text(2)? else {
            continue;
        };
        out.push(SourceInfo {
            name: r.text(0)?,
            kind: ObjectKind::Trigger,
            signature: r.text(1)?,
            text: body,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::schema::{PostgresIntrospector, Scope, Snapshot};
    use crate::{Backend, Column, Cursor, ExecOptions, Outcome, Row, TxMode};

    /// A server that answers the schema list, fails the relations query
    /// with the given errors (one per call, the last one repeats) or
    /// answers it with `relations`, and anything else with no rows.
    struct Scripted {
        failures: Vec<Option<Error>>,
        relations: Vec<Row>,
        calls: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(failures: Vec<Option<Error>>, relations: Vec<Row>) -> Self {
            Self {
                failures,
                relations,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn relation_calls(&self) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|sql| sql.as_str() == RELATIONS)
                .count()
        }
    }

    struct Page(Vec<Row>);

    #[async_trait]
    impl Cursor for Page {
        fn columns(&self) -> &[Column] {
            &[]
        }
        async fn next_page(&mut self, _max_rows: usize) -> Result<Vec<Row>> {
            Ok(std::mem::take(&mut self.0))
        }
        fn is_exhausted(&self) -> bool {
            self.0.is_empty()
        }
        async fn close(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl Connection for Scripted {
        fn backend(&self) -> Backend {
            Backend::Postgres
        }
        async fn execute(
            &self,
            sql: &str,
            _params: &[Value],
            _opts: &ExecOptions,
        ) -> Result<Outcome> {
            let call = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(sql.to_owned());
                calls.iter().filter(|c| c.as_str() == RELATIONS).count()
            };
            let rows = if sql == SCHEMAS {
                vec![vec![
                    Value::from("s"),
                    Value::Bool(false),
                    Value::Bool(true),
                    Value::Null,
                    Value::Null,
                ]]
            } else if sql == RELATIONS {
                let failure = self
                    .failures
                    .get(call - 1)
                    .or(self.failures.last())
                    .cloned()
                    .flatten();
                if let Some(error) = failure {
                    return Err(error);
                }
                self.relations.clone()
            } else {
                Vec::new()
            };
            Ok(Outcome::Rows(Box::new(Page(rows))))
        }
        async fn cancel(&self) -> Result<()> {
            Ok(())
        }
        async fn server_version(&self) -> Result<String> {
            Ok("PostgreSQL 17".into())
        }
        async fn ping(&self) -> Result<()> {
            Ok(())
        }
        async fn tx_mode(&self) -> TxMode {
            TxMode::default()
        }
        async fn set_tx_mode(&self, _mode: TxMode) -> Result<()> {
            Ok(())
        }
        async fn commit(&self) -> Result<()> {
            Ok(())
        }
        async fn rollback(&self) -> Result<()> {
            Ok(())
        }
        async fn close(&self) -> Result<()> {
            Ok(())
        }
    }

    fn vanished() -> Error {
        Error::query("could not open relation with OID 16421").with_code("XX000")
    }

    async fn load_all(conn: &Scripted) -> Result<Snapshot> {
        let intro = PostgresIntrospector::new(conn);
        crate::schema::Introspector::load(&intro, &Scope::current(), Level::Structure).await
    }

    #[tokio::test]
    async fn a_row_narrower_than_the_query_is_an_error() {
        let conn = Scripted::new(vec![None], vec![vec![Value::from("t")]]);
        let err = load_all(&conn).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("`relations`"), "{}", err.message);
        assert!(err.message.contains("column 1"), "{}", err.message);
    }

    #[tokio::test]
    async fn a_schema_is_loaded_again_when_an_object_vanished() {
        let conn = Scripted::new(vec![Some(vanished()), None], Vec::new());
        let snapshot = load_all(&conn).await.expect("the second pass succeeds");
        assert_eq!(snapshot.schemas.len(), 1);
        assert_eq!(conn.relation_calls(), 2);
    }

    #[tokio::test]
    async fn a_persistent_catalog_error_is_reported_after_the_attempts() {
        let conn = Scripted::new(vec![Some(vanished())], Vec::new());
        let err = load_all(&conn).await.unwrap_err();
        assert_eq!(err.code.as_deref(), Some("XX000"));
        assert_eq!(conn.relation_calls(), LOAD_ATTEMPTS);
    }

    #[tokio::test]
    async fn other_errors_are_not_retried() {
        let denied = Error::query("permission denied for table t").with_code("42501");
        let conn = Scripted::new(vec![Some(denied)], Vec::new());
        assert!(load_all(&conn).await.is_err());
        assert_eq!(conn.relation_calls(), 1);
    }

    #[test]
    fn only_vanished_objects_are_retried() {
        assert!(vanished_object(&vanished()));
        assert!(vanished_object(
            &Error::query("cache lookup failed for type 5").with_code("XX000")
        ));
        assert!(vanished_object(
            &Error::query("relation \"t\" does not exist").with_code("42P01")
        ));
        assert!(!vanished_object(&Error::query(
            "could not open relation with OID 1"
        )));
        assert!(!vanished_object(
            &Error::query("syntax error").with_code("42601")
        ));
    }
}
