//! What a node is ([`NodeKind`]) and the groups the tree shows its children
//! in ([`Group`]).

use db_core::schema::{ConstraintKind, ObjectKind};
use serde::{Deserialize, Serialize};

/// The kind of a node. Schema objects carry the kind of `db-core`; the
/// parts of a table or routine are nodes of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Schema,
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
    Column,
    PrimaryKey,
    UniqueKey,
    ForeignKey,
    Check,
    Argument,
}

impl NodeKind {
    /// Schema objects: the kinds an introspection pass lists as objects.
    pub const OBJECTS: [NodeKind; 11] = [
        NodeKind::Table,
        NodeKind::View,
        NodeKind::MaterializedView,
        NodeKind::Sequence,
        NodeKind::Index,
        NodeKind::Trigger,
        NodeKind::Procedure,
        NodeKind::Function,
        NodeKind::Package,
        NodeKind::Type,
        NodeKind::Synonym,
    ];

    /// Kinds a `FROM` clause can name.
    pub const RELATIONS: [NodeKind; 3] =
        [NodeKind::Table, NodeKind::View, NodeKind::MaterializedView];

    pub const ROUTINES: [NodeKind; 2] = [NodeKind::Function, NodeKind::Procedure];

    pub fn from_object(kind: ObjectKind) -> Self {
        match kind {
            ObjectKind::Table => NodeKind::Table,
            ObjectKind::View => NodeKind::View,
            ObjectKind::MaterializedView => NodeKind::MaterializedView,
            ObjectKind::Sequence => NodeKind::Sequence,
            ObjectKind::Index => NodeKind::Index,
            ObjectKind::Trigger => NodeKind::Trigger,
            ObjectKind::Procedure => NodeKind::Procedure,
            ObjectKind::Function => NodeKind::Function,
            ObjectKind::Package => NodeKind::Package,
            ObjectKind::Type => NodeKind::Type,
            ObjectKind::Synonym => NodeKind::Synonym,
        }
    }

    pub fn from_constraint(kind: ConstraintKind) -> Self {
        match kind {
            ConstraintKind::PrimaryKey => NodeKind::PrimaryKey,
            ConstraintKind::Unique => NodeKind::UniqueKey,
            ConstraintKind::ForeignKey => NodeKind::ForeignKey,
            ConstraintKind::Check => NodeKind::Check,
        }
    }

    /// The `db-core` kind of a schema object.
    pub fn object_kind(self) -> Option<ObjectKind> {
        Some(match self {
            NodeKind::Table => ObjectKind::Table,
            NodeKind::View => ObjectKind::View,
            NodeKind::MaterializedView => ObjectKind::MaterializedView,
            NodeKind::Sequence => ObjectKind::Sequence,
            NodeKind::Index => ObjectKind::Index,
            NodeKind::Trigger => ObjectKind::Trigger,
            NodeKind::Procedure => ObjectKind::Procedure,
            NodeKind::Function => ObjectKind::Function,
            NodeKind::Package => ObjectKind::Package,
            NodeKind::Type => ObjectKind::Type,
            NodeKind::Synonym => ObjectKind::Synonym,
            _ => return None,
        })
    }

    /// Tables, views and materialized views: they have columns.
    pub fn is_relation(self) -> bool {
        Self::RELATIONS.contains(&self)
    }

    pub fn is_routine(self) -> bool {
        Self::ROUTINES.contains(&self)
    }

    /// The stable name that goes into node ids. Never change one.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            NodeKind::Schema => "schema",
            NodeKind::Table => "table",
            NodeKind::View => "view",
            NodeKind::MaterializedView => "materialized_view",
            NodeKind::Sequence => "sequence",
            NodeKind::Index => "index",
            NodeKind::Trigger => "trigger",
            NodeKind::Procedure => "procedure",
            NodeKind::Function => "function",
            NodeKind::Package => "package",
            NodeKind::Type => "type",
            NodeKind::Synonym => "synonym",
            NodeKind::Column => "column",
            NodeKind::PrimaryKey => "primary_key",
            NodeKind::UniqueKey => "unique_key",
            NodeKind::ForeignKey => "foreign_key",
            NodeKind::Check => "check",
            NodeKind::Argument => "argument",
        }
    }

    /// The groups the children of a node of this kind are shown in, in tree
    /// order. A group without nodes is still listed (and reported empty),
    /// so the tree can hide it or show it as the view settings say.
    pub fn groups(self) -> &'static [Group] {
        match self {
            NodeKind::Schema => &[
                Group::Tables,
                Group::Views,
                Group::MaterializedViews,
                Group::Sequences,
                Group::Functions,
                Group::Procedures,
                Group::Packages,
                Group::Types,
                Group::Synonyms,
                // Indexes and triggers whose table is not in the schema.
                Group::Indexes,
                Group::Triggers,
            ],
            NodeKind::Table => &[
                Group::Columns,
                Group::Keys,
                Group::ForeignKeys,
                Group::Checks,
                Group::Indexes,
                Group::Triggers,
                Group::Partitions,
            ],
            NodeKind::View => &[Group::Columns, Group::Triggers],
            NodeKind::MaterializedView => &[Group::Columns, Group::Indexes],
            NodeKind::Function | NodeKind::Procedure => &[Group::Arguments],
            _ => &[],
        }
    }
}

/// A group of children in the tree: "tables" under a schema, "columns"
/// under a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Tables,
    Views,
    MaterializedViews,
    Sequences,
    Functions,
    Procedures,
    Packages,
    Types,
    Synonyms,
    Columns,
    /// Primary and unique keys.
    Keys,
    ForeignKeys,
    Checks,
    Indexes,
    Triggers,
    /// Partitions of a partitioned table, nested under it.
    Partitions,
    Arguments,
}

impl Group {
    /// The group a child of kind `child` goes to under a node of kind
    /// `parent`.
    pub fn of(parent: NodeKind, child: NodeKind) -> Group {
        match child {
            NodeKind::Table if parent == NodeKind::Table => Group::Partitions,
            NodeKind::Table => Group::Tables,
            NodeKind::View => Group::Views,
            NodeKind::MaterializedView => Group::MaterializedViews,
            NodeKind::Sequence => Group::Sequences,
            NodeKind::Function => Group::Functions,
            NodeKind::Procedure => Group::Procedures,
            NodeKind::Package => Group::Packages,
            NodeKind::Type => Group::Types,
            NodeKind::Synonym => Group::Synonyms,
            NodeKind::Index => Group::Indexes,
            NodeKind::Trigger => Group::Triggers,
            NodeKind::Column => Group::Columns,
            NodeKind::PrimaryKey | NodeKind::UniqueKey => Group::Keys,
            NodeKind::ForeignKey => Group::ForeignKeys,
            NodeKind::Check => Group::Checks,
            NodeKind::Argument => Group::Arguments,
            // A schema is never a child.
            NodeKind::Schema => Group::Tables,
        }
    }

    /// Groups whose content comes with
    /// [`Level::Structure`](db_core::schema::Level::Structure): below it
    /// they are not loaded, which is not
    /// the same as empty.
    pub fn needs_structure(self) -> bool {
        matches!(
            self,
            Group::Columns | Group::Keys | Group::ForeignKeys | Group::Checks | Group::Arguments
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for kind in NodeKind::OBJECTS {
            let object = kind.object_kind().unwrap();
            assert_eq!(NodeKind::from_object(object), kind);
        }
        assert_eq!(NodeKind::Column.object_kind(), None);
        let mut tags: Vec<_> = [
            NodeKind::Schema,
            NodeKind::Column,
            NodeKind::PrimaryKey,
            NodeKind::UniqueKey,
            NodeKind::ForeignKey,
            NodeKind::Check,
            NodeKind::Argument,
        ]
        .into_iter()
        .chain(NodeKind::OBJECTS)
        .map(NodeKind::tag)
        .collect();
        let count = tags.len();
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), count, "tags are unique");
    }

    #[test]
    fn every_child_group_is_listed_under_its_parent() {
        let cases = [
            (NodeKind::Schema, NodeKind::Table),
            (NodeKind::Schema, NodeKind::Synonym),
            (NodeKind::Schema, NodeKind::Index),
            (NodeKind::Table, NodeKind::Table),
            (NodeKind::Table, NodeKind::Column),
            (NodeKind::Table, NodeKind::UniqueKey),
            (NodeKind::Table, NodeKind::Trigger),
            (NodeKind::View, NodeKind::Column),
            (NodeKind::MaterializedView, NodeKind::Index),
            (NodeKind::Function, NodeKind::Argument),
        ];
        for (parent, child) in cases {
            assert!(
                parent.groups().contains(&Group::of(parent, child)),
                "{child:?} under {parent:?}"
            );
        }
    }
}
