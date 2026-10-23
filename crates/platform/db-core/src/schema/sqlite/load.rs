//! Bulk load for SQLite: per schema one query over `sqlite_schema` and one
//! join of it with each table-valued pragma (`pragma_table_xinfo`,
//! `pragma_index_list`, `pragma_index_xinfo`, `pragma_foreign_key_list`).

use std::collections::HashMap;

use super::sql_text;
use super::{
    ForeignKey, RawColumn, SqliteIntrospector, TableFacts, build_constraints, column_infos,
    foreign_key_groups, primary_key, quote_ident,
};
use crate::schema::{
    IndexInfo, Introspector, Level, ObjectDetail, ObjectInfo, ObjectKind, RelationInfo, SchemaInfo,
    SchemaSnapshot, Scope, Snapshot, SourceInfo, TableDetail, TriggerDetail, fetch_all, has_source,
    opt_text, text,
};
use crate::{Connection, Dialect, Result};

/// Tables SQLite keeps for itself (`sqlite_sequence`, `sqlite_stat1`, ...).
const NOT_INTERNAL: &str = "NOT (m.type = 'table' AND m.name LIKE 'sqlite\\_%' ESCAPE '\\')";

pub(super) async fn load(
    intro: &SqliteIntrospector<'_>,
    scope: &Scope,
    level: Level,
) -> Result<Snapshot> {
    let mut schemas = Vec::new();
    for info in intro.schemas().await? {
        if scope.selects(&info) {
            schemas.push(load_schema(intro.conn, info, level).await?);
        }
    }
    Ok(Snapshot {
        level,
        token: None,
        schemas,
    })
}

/// One row of `sqlite_schema`.
struct Entry {
    kind: ObjectKind,
    name: String,
    table: String,
    sql: Option<String>,
}

async fn load_schema(
    conn: &dyn Connection,
    info: SchemaInfo,
    level: Level,
) -> Result<SchemaSnapshot> {
    let schema = info.name.clone();
    let quoted = quote_ident(&schema);
    // Same set and order as `objects`.
    let master = fetch_all(
        conn,
        &format!(
            "SELECT m.type, m.name, m.tbl_name, m.sql FROM {quoted}.sqlite_schema AS m \
             WHERE m.type IN ('table', 'view', 'index', 'trigger') AND {NOT_INTERNAL} \
             ORDER BY CASE m.type WHEN 'table' THEN 0 WHEN 'view' THEN 1 \
                                  WHEN 'index' THEN 2 ELSE 3 END, m.name"
        ),
        &[],
    )
    .await?;
    let entries: Vec<Entry> = master
        .iter()
        .filter_map(|row| {
            let kind = match text(&row[0]).as_str() {
                "table" => ObjectKind::Table,
                "view" => ObjectKind::View,
                "index" => ObjectKind::Index,
                "trigger" => ObjectKind::Trigger,
                _ => return None,
            };
            Some(Entry {
                kind,
                name: text(&row[1]),
                table: text(&row[2]),
                sql: opt_text(&row[3]),
            })
        })
        .collect();

    let mut objects: Vec<ObjectInfo> = entries
        .iter()
        .map(|e| ObjectInfo {
            schema: schema.clone(),
            name: e.name.clone(),
            kind: e.kind,
            parent: matches!(e.kind, ObjectKind::Index | ObjectKind::Trigger)
                .then(|| e.table.clone()),
            ..ObjectInfo::default()
        })
        .collect();

    let mut relations = Vec::new();
    if level >= Level::Structure {
        let bulk = Bulk::read(conn, &schema, &quoted).await?;
        relations = bulk.relations(&schema, &entries);
        for (object, entry) in objects.iter_mut().zip(&entries) {
            object.detail = detail(&schema, entry);
        }
    }

    let mut sources = Vec::new();
    if level >= Level::Full {
        for entry in &entries {
            if let (true, Some(sql)) = (has_source(Dialect::Sqlite, entry.kind), &entry.sql) {
                sources.push(SourceInfo {
                    name: entry.name.clone(),
                    kind: entry.kind,
                    signature: String::new(),
                    text: sql.clone(),
                });
            }
        }
    }

    Ok(SchemaSnapshot {
        info,
        level,
        objects,
        relations,
        sources,
    })
}

/// The kind-specific facts SQLite has: how a table is stored, when a trigger
/// fires.
fn detail(schema: &str, entry: &Entry) -> Option<ObjectDetail> {
    match entry.kind {
        ObjectKind::Table => {
            let sql = entry.sql.as_deref().unwrap_or_default();
            Some(ObjectDetail::Table(TableDetail {
                temporary: schema.eq_ignore_ascii_case("temp"),
                without_rowid: sql_text::parse_table(sql).without_rowid,
                virtual_table: sql_text::is_virtual_table(sql),
                ..TableDetail::default()
            }))
        }
        ObjectKind::Trigger => {
            let parts = sql_text::parse_trigger(entry.sql.as_deref().unwrap_or_default());
            Some(ObjectDetail::Trigger(TriggerDetail {
                events: parts.events,
                timing: parts.timing,
                // SQLite has row triggers only.
                each_row: true,
                condition: parts.condition,
                ..TriggerDetail::default()
            }))
        }
        _ => None,
    }
}

/// The four pragma joins of a schema, grouped by table.
struct Bulk {
    columns: HashMap<String, Vec<RawColumn>>,
    /// Per table: index name, unique, origin, partial.
    index_list: HashMap<String, Vec<(String, bool, String, bool)>>,
    /// Per index: key entries as (seqno, cid, name).
    index_keys: HashMap<String, Vec<(i64, i64, Option<String>)>>,
    foreign_keys: HashMap<String, Vec<ForeignKey>>,
}

impl Bulk {
    async fn read(conn: &dyn Connection, schema: &str, quoted: &str) -> Result<Self> {
        let schema_param = [crate::Value::from(schema)];
        let from = format!("{quoted}.sqlite_schema AS m");

        let mut columns: HashMap<String, Vec<RawColumn>> = HashMap::new();
        let rows = fetch_all(
            conn,
            &format!(
                "SELECT m.name, x.cid, x.name, x.type, x.\"notnull\", x.dflt_value, x.pk, x.hidden \
                 FROM {from}, pragma_table_xinfo(m.name, ?1) AS x \
                 WHERE m.type IN ('table', 'view') AND {NOT_INTERNAL} \
                 ORDER BY m.name, x.cid"
            ),
            &schema_param,
        )
        .await?;
        for r in &rows {
            columns.entry(text(&r[0])).or_default().push(RawColumn {
                cid: r[1].as_i64().unwrap_or(0),
                name: text(&r[2]),
                declared: text(&r[3]),
                not_null: r[4].as_i64().unwrap_or(0) != 0,
                default: opt_text(&r[5]),
                pk: r[6].as_i64().unwrap_or(0),
                hidden: r[7].as_i64().unwrap_or(0),
            });
        }

        // The indexes of every table with their key columns. Joined from the
        // table, not from `sqlite_schema`: the index behind the primary key
        // of a WITHOUT ROWID table is not stored there.
        let mut index_list: HashMap<String, Vec<(String, bool, String, bool)>> = HashMap::new();
        let mut index_keys: HashMap<String, Vec<(i64, i64, Option<String>)>> = HashMap::new();
        let rows = fetch_all(
            conn,
            &format!(
                "SELECT m.name, il.name, il.\"unique\", il.origin, il.partial, \
                        x.seqno, x.cid, x.name \
                 FROM {from}, pragma_index_list(m.name, ?1) AS il, \
                      pragma_index_xinfo(il.name, ?1) AS x \
                 WHERE m.type = 'table' AND {NOT_INTERNAL} AND x.key = 1 \
                 ORDER BY m.name, il.seq, x.seqno"
            ),
            &schema_param,
        )
        .await?;
        for r in &rows {
            let (table, index) = (text(&r[0]), text(&r[1]));
            let list = index_list.entry(table).or_default();
            if list.last().is_none_or(|(last, ..)| *last != index) {
                list.push((
                    index.clone(),
                    r[2].as_i64().unwrap_or(0) != 0,
                    text(&r[3]),
                    r[4].as_i64().unwrap_or(0) != 0,
                ));
            }
            index_keys.entry(index).or_default().push((
                r[5].as_i64().unwrap_or(0),
                r[6].as_i64().unwrap_or(-2),
                opt_text(&r[7]),
            ));
        }

        let mut foreign_keys: HashMap<String, Vec<ForeignKey>> = HashMap::new();
        let rows = fetch_all(
            conn,
            &format!(
                "SELECT m.name, f.id, f.\"table\", f.\"from\", f.\"to\", f.on_update, f.on_delete \
                 FROM {from}, pragma_foreign_key_list(m.name, ?1) AS f \
                 WHERE m.type = 'table' AND {NOT_INTERNAL} \
                 ORDER BY m.name, f.id, f.seq"
            ),
            &schema_param,
        )
        .await?;
        let mut by_table: Vec<(String, Vec<_>)> = Vec::new();
        for r in &rows {
            let table = text(&r[0]);
            let row = (
                r[1].as_i64().unwrap_or(0),
                text(&r[2]),
                text(&r[3]),
                opt_text(&r[4]),
                opt_text(&r[5]),
                opt_text(&r[6]),
            );
            match by_table.last_mut() {
                Some((t, rows)) if *t == table => rows.push(row),
                _ => by_table.push((table, vec![row])),
            }
        }
        for (table, rows) in by_table {
            foreign_keys.insert(table, foreign_key_groups(rows.into_iter()));
        }

        Ok(Self {
            columns,
            index_list,
            index_keys,
            foreign_keys,
        })
    }

    /// Columns, indexes and constraints of every table and view.
    fn relations(&self, schema: &str, entries: &[Entry]) -> Vec<RelationInfo> {
        let sql_of: HashMap<&str, Option<&str>> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.sql.as_deref()))
            .collect();
        let no_columns = Vec::new();
        let mut out = Vec::new();
        for entry in entries
            .iter()
            .filter(|e| matches!(e.kind, ObjectKind::Table | ObjectKind::View))
        {
            let raw = self.columns.get(&entry.name).unwrap_or(&no_columns);
            let parsed = match entry.kind {
                ObjectKind::Table => entry
                    .sql
                    .as_deref()
                    .map(sql_text::parse_table)
                    .unwrap_or_default(),
                _ => sql_text::Table::default(),
            };

            // An INTEGER PRIMARY KEY of a rowid table is the rowid itself.
            let pk: Vec<&RawColumn> = raw.iter().filter(|c| c.pk > 0).collect();
            let rowid_alias = match pk.as_slice() {
                [only]
                    if only.declared.eq_ignore_ascii_case("INTEGER") && !parsed.without_rowid =>
                {
                    Some(only.cid)
                }
                _ => None,
            };
            let columns = column_infos(raw, rowid_alias, Some(&parsed.generated));

            let list = self.index_list.get(&entry.name);
            let mut indexes = Vec::new();
            let mut unique = Vec::new();
            for (name, is_unique, origin, partial) in list.into_iter().flatten() {
                let index_sql = sql_of.get(name.as_str()).copied().flatten();
                let (keys, expressions) = self.index_columns(name, index_sql);
                if origin == "u" {
                    unique.push((name.clone(), keys.clone()));
                }
                indexes.push(IndexInfo {
                    name: name.clone(),
                    unique: *is_unique,
                    columns: keys,
                    predicate: partial
                        .then(|| index_sql.and_then(sql_text::index_predicate))
                        .flatten(),
                    expressions,
                    ..IndexInfo::default()
                });
            }

            let groups = self
                .foreign_keys
                .get(&entry.name)
                .map(Vec::as_slice)
                .unwrap_or_default();
            // `REFERENCES parent` without columns means parent's primary key.
            let mut parent_pks = HashMap::new();
            for group in groups {
                if !group.to.iter().all(Option::is_some) {
                    let parent = self.columns.get(&group.table).unwrap_or(&no_columns);
                    parent_pks.insert(group.table.clone(), primary_key(parent));
                }
            }
            let constraints = if entry.kind == ObjectKind::Table {
                build_constraints(
                    &TableFacts {
                        schema,
                        table: &entry.name,
                        raw,
                        parsed: &parsed,
                        unique: &unique,
                        groups,
                        parent_pks: &parent_pks,
                    },
                    true,
                )
            } else {
                Vec::new()
            };

            out.push(RelationInfo {
                name: entry.name.clone(),
                columns,
                indexes,
                constraints,
            });
        }
        out
    }

    /// The key columns of an index, with the positions that are
    /// expressions. Expressions come from the `CREATE INDEX` text; the
    /// pragma only says that there is one.
    fn index_columns(&self, index: &str, sql: Option<&str>) -> (Vec<String>, Vec<usize>) {
        let mut items: Option<Vec<String>> = None;
        let mut names = Vec::new();
        let mut expressions = Vec::new();
        for (seqno, cid, name) in self.index_keys.get(index).into_iter().flatten() {
            if let Some(name) = name {
                names.push(name.clone());
            } else if *cid == -1 {
                names.push("rowid".to_owned());
            } else {
                let items =
                    items.get_or_insert_with(|| sql.map(sql_text::index_items).unwrap_or_default());
                let at = usize::try_from(*seqno).unwrap_or(0);
                expressions.push(names.len());
                names.push(
                    items
                        .get(at)
                        .cloned()
                        .unwrap_or_else(|| "<expression>".to_owned()),
                );
            }
        }
        (names, expressions)
    }
}
