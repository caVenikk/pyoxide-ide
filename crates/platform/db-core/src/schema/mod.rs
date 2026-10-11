//! Schema introspection: a backend-neutral model and one implementation per
//! dialect. Implementations only use [`Connection::query`], so every driver
//! of a dialect shares the same SQL.

mod oracle;
mod postgres;
mod sqlite;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Connection, Dialect, Result, Row, Value};

pub use oracle::OracleIntrospector;
pub use postgres::PostgresIntrospector;
pub use sqlite::SqliteIntrospector;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub name: String,
    /// Owned by the database itself (`SYS`, `pg_catalog`, ...).
    pub is_system: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
    /// Owning table for indexes and triggers.
    pub parent: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    /// Column names, or expressions for function-based indexes, in order.
    pub columns: Vec<String>,
    /// Backend-specific method (`NORMAL`, `BITMAP`, `btree`, `gin`).
    pub method: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// The introspector for a connection's dialect.
pub fn introspector(conn: &dyn Connection) -> Box<dyn Introspector + '_> {
    match conn.dialect() {
        Dialect::Oracle => Box::new(OracleIntrospector::new(conn)),
        Dialect::Postgres => Box::new(PostgresIntrospector::new(conn)),
        Dialect::Sqlite => Box::new(SqliteIntrospector::new(conn)),
    }
}

/// All rows of a metadata query.
pub(crate) async fn fetch_all(
    conn: &dyn Connection,
    sql: &str,
    params: &[Value],
) -> Result<Vec<Row>> {
    Ok(conn.query(sql, params, usize::MAX).await?.rows)
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
