//! Schema introspection: a backend-neutral model and one implementation per
//! dialect. Implementations only use [`Connection::query`], so every driver
//! of a dialect shares the same SQL.

mod change;
mod detail;
mod oracle;
mod postgres;
mod rows;
mod snapshot;
mod sqlite;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Connection, Dialect, Error, Result, Value};

pub use detail::{
    ArgumentInfo, ArgumentMode, AttributeInfo, GeneratedColumn, IdentityKind, ObjectDetail,
    PartitionOf, RoutineDetail, SequenceDetail, TableDetail, TriggerDetail, TypeCategory,
    TypeDetail, ViewDetail,
};
pub use oracle::OracleIntrospector;
pub use postgres::PostgresIntrospector;
pub use snapshot::{
    ChangeToken, Delta, Level, ObjectRef, RelationInfo, SchemaDelta, SchemaFilter, SchemaSnapshot,
    Scope, Snapshot, SourceInfo, load_per_object,
};
pub use sqlite::SqliteIntrospector;

use rows::Cells;
use snapshot::{has_columns, has_source};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    #[default]
    Table,
    View,
    MaterializedView,
    Sequence,
    Index,
    Trigger,
    Procedure,
    Function,
    Package,
    Type,
    Synonym,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub name: String,
    /// Owned by the database itself (`SYS`, `pg_catalog`, ...).
    pub is_system: bool,
    pub is_current: bool,
    /// Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
    /// Owning table for indexes and triggers.
    pub parent: Option<String>,
    pub comment: Option<String>,
    /// Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Kind-specific facts, filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<ObjectDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    /// 1-based position in the table.
    pub position: u32,
    /// Full type as the database shows it (`VARCHAR2(100 CHAR)`, `numeric(10,2)`).
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    /// Identity, serial or autoincrement column.
    pub auto_increment: bool,
    pub comment: Option<String>,
    /// Identity columns: `ALWAYS` or `BY DEFAULT`. Filled by
    /// [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<IdentityKind>,
    /// Generated columns. Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<GeneratedColumn>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    /// Column names, or expressions for function-based indexes, in order.
    pub columns: Vec<String>,
    /// Backend-specific method (`NORMAL`, `BITMAP`, `btree`, `gin`).
    pub method: Option<String>,
    /// Partial indexes: the `WHERE` condition. Filled by
    /// [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
    /// Non-key `INCLUDE` columns. Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    /// Positions (from 0) in `columns` that are expressions, not column
    /// names. Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    #[default]
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintInfo {
    pub name: String,
    pub kind: ConstraintKind,
    pub columns: Vec<String>,
    /// Foreign keys: referenced table and columns.
    pub ref_schema: Option<String>,
    pub ref_table: Option<String>,
    pub ref_columns: Vec<String>,
    pub on_delete: Option<String>,
    /// Check constraints: the condition text.
    pub check: Option<String>,
    /// Foreign keys: the `ON UPDATE` action. Filled by
    /// [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_update: Option<String>,
    /// Foreign keys: `SIMPLE` or `FULL`. Filled by [`Introspector::load`]
    /// only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_type: Option<String>,
    /// Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "detail::is_false")]
    pub deferrable: bool,
    /// Filled by [`Introspector::load`] only.
    #[serde(default, skip_serializing_if = "detail::is_false")]
    pub initially_deferred: bool,
    /// The constraint is not enforced. Filled by [`Introspector::load`]
    /// only.
    #[serde(default, skip_serializing_if = "detail::is_false")]
    pub disabled: bool,
    /// Existing rows were not checked. Filled by [`Introspector::load`]
    /// only.
    #[serde(default, skip_serializing_if = "detail::is_false")]
    pub not_validated: bool,
}

/// Read schema metadata. Names are passed exactly as stored by the database
/// (Oracle upper case for unquoted names); values are always bound, never
/// spliced into SQL.
#[async_trait]
pub trait Introspector: Send + Sync {
    async fn current_schema(&self) -> Result<String>;

    async fn schemas(&self) -> Result<Vec<SchemaInfo>>;

    /// Objects of one schema; an empty `kinds` means all kinds.
    async fn objects(&self, schema: &str, kinds: &[ObjectKind]) -> Result<Vec<ObjectInfo>>;

    async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>>;

    async fn indexes(&self, schema: &str, table: &str) -> Result<Vec<IndexInfo>>;

    async fn constraints(&self, schema: &str, table: &str) -> Result<Vec<ConstraintInfo>>;

    /// Source or DDL of a view, routine, package, trigger or table, if the
    /// database exposes it.
    async fn source(&self, schema: &str, name: &str, kind: ObjectKind) -> Result<Option<String>>;

    /// The schemas of `scope` with their objects, and at `level` Structure
    /// and above the columns, indexes and constraints of every table and
    /// view, in a number of queries per schema that does not depend on how
    /// many objects it holds (a schema of 5000 tables costs the same number
    /// of round trips as one of five). Reads and returns in one piece: the
    /// caller owns cancellation (drop the future, or [`Connection::cancel`]
    /// from another task).
    ///
    /// The base fields equal what the per-object reads return (the
    /// conformance suite checks it); the optional fields are filled where
    /// the database tells. Use a connection with no open cursor.
    async fn load(&self, scope: &Scope, level: Level) -> Result<Snapshot>;

    /// What changed in `scope` since `since`, read from the change query
    /// alone (a statement or a few per schema, whatever the number of
    /// objects) plus a read of just the objects that changed. The delta
    /// holds the new or changed objects whole (a table with its columns,
    /// keys, indexes and triggers) at the level each schema was read at, the
    /// objects that are gone, the schemas that are gone, and the schemas of
    /// the scope the token knows nothing of ([`Delta::unseen`], to be read
    /// with [`Introspector::load`]). Its token covers everything `since`
    /// did, with the schemas it looked at brought up to date. Fails with
    /// `Unsupported` when the token is not one this introspector can
    /// compare, and the caller reloads with [`Introspector::load`].
    async fn changes(&self, since: &ChangeToken, scope: &Scope) -> Result<Delta> {
        let _ = (since, scope);
        Err(Error::unsupported(
            "this database cannot report schema changes; reload the schema",
        ))
    }

    /// Reads just `objects` of `schema` at `level`: a table, view, sequence,
    /// routine, package or type each, with what belongs to it (a table's
    /// columns, keys, indexes and triggers). An object that is not there any
    /// more comes back in [`SchemaDelta::dropped`]. An index or a trigger is
    /// read with its table: name the table. The default reads the whole
    /// schema and keeps those objects; the dialects with a bulk path read
    /// only them.
    async fn load_objects(
        &self,
        schema: &str,
        objects: &[ObjectRef],
        level: Level,
    ) -> Result<SchemaDelta> {
        change::load_objects_by_load(self, schema, objects, level).await
    }
}

/// `sql` with each `/*only:CONDITION*/` marker turned into `AND CONDITION`,
/// where `@` in the condition is `placeholder`: a statement that reads every
/// object of a schema becomes one that reads the ones named in a list bound
/// to the placeholder. A statement without the marker is returned as it is.
pub(crate) fn restrict(sql: &str, placeholder: &str) -> String {
    const OPEN: &str = "/*only:";
    let mut out = String::with_capacity(sql.len() + 64);
    let mut rest = sql;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        let after = &rest[at + OPEN.len()..];
        let end = after.find("*/").unwrap_or(after.len());
        out.push_str("AND ");
        out.push_str(&after[..end].replace('@', placeholder));
        rest = after.get(end + 2..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// The introspector for a connection's dialect.
pub fn introspector(conn: &dyn Connection) -> Box<dyn Introspector + '_> {
    match conn.dialect() {
        Dialect::Oracle => Box::new(OracleIntrospector::new(conn)),
        Dialect::Postgres => Box::new(PostgresIntrospector::new(conn)),
        Dialect::Sqlite => Box::new(SqliteIntrospector::new(conn)),
    }
}

/// All rows of a metadata query; `name` labels the query in errors.
pub(crate) async fn fetch_all(
    conn: &dyn Connection,
    name: &'static str,
    sql: &str,
    params: &[Value],
) -> Result<Vec<Cells>> {
    let rows = conn.query(sql, params, usize::MAX).await?.rows;
    Ok(rows.into_iter().map(|row| Cells::new(name, row)).collect())
}

/// Text of a cell; empty for NULL.
pub(crate) fn text(v: &Value) -> String {
    v.to_text().unwrap_or_default()
}

/// Text of a cell; `None` for NULL or empty.
pub(crate) fn opt_text(v: &Value) -> Option<String> {
    v.to_text().filter(|s| !s.is_empty())
}

pub(crate) fn wants(kinds: &[ObjectKind], kind: ObjectKind) -> bool {
    kinds.is_empty() || kinds.contains(&kind)
}

#[cfg(test)]
mod restrict_tests {
    use super::restrict;

    #[test]
    fn markers_become_conditions() {
        let sql = "SELECT 1 WHERE a = $1 /*only:(x = ANY(@) OR y = ANY(@))*/ ORDER BY 1";
        assert_eq!(
            restrict(sql, "$2::text[]"),
            "SELECT 1 WHERE a = $1 AND (x = ANY($2::text[]) OR y = ANY($2::text[])) ORDER BY 1"
        );
        assert_eq!(restrict("SELECT 1", ":2"), "SELECT 1");
    }
}
