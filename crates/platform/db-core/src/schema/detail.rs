//! Kind-specific facts about schema objects. They are optional parts of the
//! model: the per-object reads (`objects`, `columns`, ...) leave them empty,
//! the bulk [`load`](super::Introspector::load) fills them as far as the
//! level and the database allow. Every field has a default and is left out of
//! the JSON when empty, so output that did not have them is unchanged.

use serde::{Deserialize, Serialize};

pub(super) fn is_false(b: &bool) -> bool {
    !*b
}

/// What an [`ObjectInfo`](super::ObjectInfo) of a given kind carries beyond
/// its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObjectDetail {
    Table(TableDetail),
    View(ViewDetail),
    Sequence(SequenceDetail),
    Routine(RoutineDetail),
    Trigger(TriggerDetail),
    Type(TypeDetail),
}

/// A table: its storage flavour and its place in a partition tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableDetail {
    /// Temporary (PostgreSQL `TEMPORARY`, Oracle global temporary, the
    /// SQLite `temp` schema).
    #[serde(skip_serializing_if = "is_false")]
    pub temporary: bool,
    /// PostgreSQL `UNLOGGED`.
    #[serde(skip_serializing_if = "is_false")]
    pub unlogged: bool,
    /// PostgreSQL foreign table.
    #[serde(skip_serializing_if = "is_false")]
    pub foreign: bool,
    /// The table is split into partitions (PostgreSQL partitioned table,
    /// Oracle partitioned table).
    #[serde(skip_serializing_if = "is_false")]
    pub partitioned: bool,
    /// How a partitioned table is split: `RANGE (day)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<String>,
    /// The table is a partition of another table (PostgreSQL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_of: Option<PartitionOf>,
    /// SQLite `WITHOUT ROWID`.
    #[serde(skip_serializing_if = "is_false")]
    pub without_rowid: bool,
    /// SQLite virtual table.
    #[serde(skip_serializing_if = "is_false")]
    pub virtual_table: bool,
    /// Oracle index-organized table.
    #[serde(skip_serializing_if = "is_false")]
    pub index_organized: bool,
}

/// The parent of a partition and its bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionOf {
    pub schema: String,
    pub table: String,
    /// As the database prints them: `FOR VALUES FROM ('a') TO ('b')`,
    /// `DEFAULT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewDetail {
    /// Materialized views: the data has been loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub populated: Option<bool>,
}

/// Sequence parameters. Numbers stay text: Oracle sequences go up to 28
/// digits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SequenceDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub increment: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub cycle: bool,
    /// Numbers preallocated per session; `None` when not cached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<i64>,
    /// Oracle `ORDER`.
    #[serde(skip_serializing_if = "is_false")]
    pub ordered: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentMode {
    #[default]
    In,
    Out,
    InOut,
    Variadic,
    /// A column of a `RETURNS TABLE (...)` function.
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgumentInfo {
    /// `None` for unnamed arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub data_type: String,
    #[serde(default)]
    pub mode: ArgumentMode,
    /// The argument has a default value.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_default: bool,
}

/// A function or procedure. `ObjectInfo::name` is shared by overloads;
/// [`signature`](Self::signature) tells them apart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutineDetail {
    /// The identity arguments, `integer, text` (PostgreSQL); empty when the
    /// name alone identifies the routine.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signature: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<ArgumentInfo>,
    /// As the database prints it: `bigint`, `SETOF text`,
    /// `TABLE(a integer, b text)`. `None` for procedures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `IMMUTABLE`, `STABLE`, `SECURITY DEFINER`, `DETERMINISTIC`,
    /// `STRICT`, `PARALLEL SAFE`, ...
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub characteristics: Vec<String>,
}

/// A trigger. The table it belongs to is `ObjectInfo::parent`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggerDetail {
    /// `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// `BEFORE`, `AFTER`, `INSTEAD OF`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<String>,
    /// `FOR EACH ROW` (otherwise per statement).
    #[serde(skip_serializing_if = "is_false")]
    pub each_row: bool,
    /// The function a PostgreSQL trigger calls, schema-qualified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// The `WHEN` condition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub disabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeCategory {
    #[default]
    Other,
    Enum,
    Composite,
    Domain,
    Range,
    /// Oracle object type.
    Object,
    /// Oracle `VARRAY` or nested table type.
    Collection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeInfo {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypeDetail {
    pub category: TypeCategory,
    /// Enum labels in sort order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Attributes of a composite or object type.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeInfo>,
    /// Domain: the underlying type; range: the subtype; collection: the
    /// element type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_type: Option<String>,
    /// Domain constraints, as `CHECK (...)` text.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub not_null: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    /// `GENERATED ALWAYS AS IDENTITY`.
    Always,
    /// `GENERATED BY DEFAULT AS IDENTITY`.
    ByDefault,
}

/// A generated (computed, virtual) column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedColumn {
    /// The expression, when the database exposes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Stored on disk (otherwise computed when read).
    #[serde(default)]
    pub stored: bool,
}
