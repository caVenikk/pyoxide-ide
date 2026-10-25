//! What a name in a document stands for.

use std::hash::{Hash, Hasher};
use std::ops::Range;

use db_model::NodeId;

/// What a reference names, by its position in the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    Schema,
    /// A table, view, CTE or alias: in `FROM`, as a DML target, as the
    /// qualifier of a column.
    Table,
    Column,
    Routine,
}

impl RefKind {
    /// The word for it in messages.
    pub fn noun(self) -> &'static str {
        match self {
            RefKind::Schema => "schema",
            RefKind::Table => "table",
            RefKind::Column => "column",
            RefKind::Routine => "routine",
        }
    }
}

/// What a resolved name stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A schema, table, view, synonym, routine or column of the model.
    Object(NodeId),
    /// A definition in the same statement: a CTE, a table alias, a
    /// select-list alias (the range of its name).
    Local(Range<usize>),
    /// An object an earlier statement of the script creates (catalog
    /// names).
    ScriptObject {
        schema: Option<String>,
        name: String,
    },
    /// A column of a source of the `FROM` clause: the qualifier that names
    /// the source as written (its alias, else its table name), the range of
    /// that name, and the model's column when the source is a table of the
    /// model.
    SourceColumn {
        qualifier: String,
        source: Range<usize>,
        column: Option<NodeId>,
    },
    /// Something the dialect has by itself: `DUAL`, a system column,
    /// `current_date`, a pseudo-column.
    Builtin,
}

/// The result of resolving a name (ADR 0022). Only [`Resolution::Unresolved`]
/// is reported as an error: a name is [`Resolution::Unknown`] whenever the
/// answer cannot be known (no scope, a schema outside the model or loaded
/// to names only, a hole, procedural code, the generic dialect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved(Target),
    /// An unqualified column found in more than one source of the same
    /// `FROM`.
    Ambiguous(Vec<Target>),
    Unresolved,
    Unknown,
}

impl Resolution {
    pub fn is_resolved(&self) -> bool {
        matches!(self, Resolution::Resolved(_))
    }
}

/// A fix offered for an unresolved name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Suggestion {
    /// A similar name that resolves, as it is to be written.
    Rename(String),
    /// The object exists in this schema (as it is to be written) outside
    /// the search path.
    Qualify(String),
    /// The qualifier names a table that has this alias in `FROM`.
    UseAlias(String),
}

/// A name of a statement and what it resolves to. Ranges are byte offsets
/// in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The range of the name itself (the last part of a qualified name).
    pub range: Range<usize>,
    pub kind: RefKind,
    /// The name as written.
    pub written: String,
    pub resolution: Resolution,
    /// For an unresolved name: fixes, best first.
    pub suggestions: Vec<Suggestion>,
}

/// The columns of a source, as far as they are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Columns {
    Known(Vec<Col>),
    /// The source's columns cannot be known (a schema loaded to names only,
    /// a function, a hole): no column of it is reported.
    Unknown,
}

impl Columns {
    pub fn names<'a>(names: impl IntoIterator<Item = &'a str>) -> Columns {
        Columns::Known(
            names
                .into_iter()
                .map(|n| Col {
                    name: n.to_string(),
                    id: None,
                })
                .collect(),
        )
    }

    pub fn len(&self) -> Option<usize> {
        match self {
            Columns::Known(c) => Some(c.len()),
            Columns::Unknown => None,
        }
    }

    pub fn hash_into<H: Hasher>(&self, h: &mut H) {
        match self {
            Columns::Known(cols) => {
                cols.len().hash(h);
                for c in cols {
                    c.name.hash(h);
                }
            }
            Columns::Unknown => u64::MAX.hash(h),
        }
    }
}

/// A column: its catalog name (empty for an unnamed expression) and the
/// model's node when it is a column of a table of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Col {
    pub name: String,
    pub id: Option<NodeId>,
}
