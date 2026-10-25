//! The rows of the object tree of the Database tool window, from the
//! metadata models of the data sources (`db-model`): data source >
//! database > schema > groups by kind > objects > groups of their children
//! (columns, keys, foreign keys, indexes, checks, triggers, partitions).
//! Empty groups are hidden; a group whose content is not loaded (columns of
//! a schema read to names only) is hidden too. Partitions are nested under
//! their table, as the model keeps them.
//!
//! Expanding a node reads the model only, never the database: the rows are
//! a flat list of the visible nodes, rebuilt when the expansion or a model
//! changes, and the texts of a row are made when it is drawn.

use std::collections::HashSet;
use std::sync::Arc;

use db_core::schema::{ArgumentMode, ConstraintKind, ObjectDetail};
use db_model::{Group, Model, NodeId, NodeKind, NodeRef};

/// A group of children as the tree shows it. Routines are functions and
/// procedures together; the others are one group of the model each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TreeGroup {
    Tables,
    MaterializedViews,
    Views,
    Routines,
    Sequences,
    ObjectTypes,
    Packages,
    Synonyms,
    Columns,
    Keys,
    ForeignKeys,
    Indexes,
    Checks,
    Triggers,
    Partitions,
}

impl TreeGroup {
    /// The row's text (the reference writes groups in lower case).
    pub fn label(self) -> &'static str {
        match self {
            TreeGroup::Tables => "tables",
            TreeGroup::MaterializedViews => "materialized views",
            TreeGroup::Views => "views",
            TreeGroup::Routines => "routines",
            TreeGroup::Sequences => "sequences",
            TreeGroup::ObjectTypes => "object types",
            TreeGroup::Packages => "packages",
            TreeGroup::Synonyms => "synonyms",
            TreeGroup::Columns => "columns",
            TreeGroup::Keys => "keys",
            TreeGroup::ForeignKeys => "foreign keys",
            TreeGroup::Indexes => "indexes",
            TreeGroup::Checks => "checks",
            TreeGroup::Triggers => "triggers",
            TreeGroup::Partitions => "partitions",
        }
    }

    /// The groups of the model it shows.
    fn model_groups(self) -> &'static [Group] {
        match self {
            TreeGroup::Tables => &[Group::Tables],
            TreeGroup::MaterializedViews => &[Group::MaterializedViews],
            TreeGroup::Views => &[Group::Views],
            TreeGroup::Routines => &[Group::Functions, Group::Procedures],
            TreeGroup::Sequences => &[Group::Sequences],
            TreeGroup::ObjectTypes => &[Group::Types],
            TreeGroup::Packages => &[Group::Packages],
            TreeGroup::Synonyms => &[Group::Synonyms],
            TreeGroup::Columns => &[Group::Columns],
            TreeGroup::Keys => &[Group::Keys],
            TreeGroup::ForeignKeys => &[Group::ForeignKeys],
            TreeGroup::Indexes => &[Group::Indexes],
            TreeGroup::Checks => &[Group::Checks],
            TreeGroup::Triggers => &[Group::Triggers],
            TreeGroup::Partitions => &[Group::Partitions],
        }
    }

    /// The groups under a node of `kind`, in the reference's order.
    pub fn of(kind: NodeKind) -> &'static [TreeGroup] {
        match kind {
            NodeKind::Schema => &[
                TreeGroup::Tables,
                TreeGroup::MaterializedViews,
                TreeGroup::Views,
                TreeGroup::Routines,
                TreeGroup::Sequences,
                TreeGroup::ObjectTypes,
                TreeGroup::Packages,
                TreeGroup::Synonyms,
                // Indexes and triggers whose table is not in the schema.
                TreeGroup::Indexes,
                TreeGroup::Triggers,
            ],
            NodeKind::Table => &[
                TreeGroup::Columns,
                TreeGroup::Keys,
                TreeGroup::ForeignKeys,
                TreeGroup::Indexes,
                TreeGroup::Checks,
                TreeGroup::Triggers,
                TreeGroup::Partitions,
            ],
            NodeKind::View => &[TreeGroup::Columns, TreeGroup::Triggers],
            NodeKind::MaterializedView => &[TreeGroup::Columns, TreeGroup::Indexes],
            _ => &[],
        }
    }

    /// The nodes of the group under `parent`, in tree order (routines by
    /// name, the others as the model keeps them).
    pub fn nodes<'a>(self, parent: NodeRef<'a>) -> Vec<NodeRef<'a>> {
        let mut out = Vec::new();
        for g in self.model_groups() {
            let group = parent.group(*g);
            if group.is_loaded() {
                out.extend(group.nodes());
            }
        }
        if self.model_groups().len() > 1 {
            out.sort_by_cached_key(|n| (n.name().to_lowercase(), n.signature().to_owned()));
        }
        out
    }

    /// How many nodes the group has under `parent` (none when its content
    /// is not loaded). Tables count their nested partitions too, as the
    /// reference's count does.
    pub fn len(self, parent: NodeRef<'_>) -> usize {
        let nested = matches!(self, TreeGroup::Tables | TreeGroup::Partitions);
        self.model_groups()
            .iter()
            .map(|g| parent.group(*g))
            .filter(|g| g.is_loaded())
            .map(|g| {
                if nested {
                    g.nodes().map(|n| 1 + partitions(n)).sum()
                } else {
                    g.len()
                }
            })
            .sum()
    }

    /// A stable name for selectors and the state report.
    pub fn id(self) -> &'static str {
        match self {
            TreeGroup::Tables => "tables",
            TreeGroup::MaterializedViews => "materialized-views",
            TreeGroup::Views => "views",
            TreeGroup::Routines => "routines",
            TreeGroup::Sequences => "sequences",
            TreeGroup::ObjectTypes => "object-types",
            TreeGroup::Packages => "packages",
            TreeGroup::Synonyms => "synonyms",
            TreeGroup::Columns => "columns",
            TreeGroup::Keys => "keys",
            TreeGroup::ForeignKeys => "foreign-keys",
            TreeGroup::Indexes => "indexes",
            TreeGroup::Checks => "checks",
            TreeGroup::Triggers => "triggers",
            TreeGroup::Partitions => "partitions",
        }
    }
}

/// The partitions under a table, at every level.
fn partitions(table: NodeRef<'_>) -> usize {
    table
        .group(Group::Partitions)
        .nodes()
        .map(|p| 1 + partitions(p))
        .sum()
}

/// The visible groups under `parent` (empty and unloaded ones hidden).
pub fn visible_groups(parent: NodeRef<'_>) -> impl Iterator<Item = TreeGroup> + '_ {
    TreeGroup::of(parent.kind())
        .iter()
        .copied()
        .filter(move |g| g.len(parent) > 0)
}

/// A node of the tree, within its data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKey {
    /// The data source itself.
    Source,
    /// The database of the data source (PostgreSQL).
    Database,
    /// A schema or an object of the model.
    Node(NodeId),
    /// A group under a schema or an object.
    Group(NodeId, TreeGroup),
    /// The state of a data source without a model: connecting, loading,
    /// failed.
    Status,
}

/// A node of the tree: its data source and the node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowKey {
    pub source: Arc<str>,
    pub node: NodeKey,
}

impl RowKey {
    pub fn source(source: &Arc<str>) -> Self {
        Self {
            source: source.clone(),
            node: NodeKey::Source,
        }
    }
}

/// What a data source shows instead of its model while it has none.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Connecting,
    Loading,
    /// The connection or the read failed: the message, and the row offers
    /// to try again.
    Failed(String),
    /// Read, and nothing is in the introspection scope.
    Empty,
}

/// What the tree knows of one data source.
pub struct SourceTree<'a> {
    pub name: &'a Arc<str>,
    /// The database row's name (PostgreSQL), `None` for engines whose data
    /// sources hold one database (Oracle, SQLite).
    pub database: Option<&'a str>,
    /// The model, `None` before one was read or loaded from the cache.
    pub model: Option<&'a Model>,
    /// Shown when there is no model (or it holds no schema).
    pub status: Option<Status>,
}

/// One visible row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: RowKey,
    pub depth: usize,
    pub expandable: bool,
    pub expanded: bool,
}

/// The visible rows of the data sources with the nodes of `expanded` open.
pub fn rows(sources: &[SourceTree<'_>], expanded: &HashSet<RowKey>) -> Vec<Row> {
    let mut out = Vec::new();
    for source in sources {
        let key = RowKey::source(source.name);
        let open = expanded.contains(&key);
        out.push(Row {
            key,
            depth: 0,
            expandable: true,
            expanded: open,
        });
        if !open {
            continue;
        }
        let schemas = source.model.map_or(0, |m| m.schemas().len());
        if schemas == 0 {
            out.push(Row {
                key: RowKey {
                    source: source.name.clone(),
                    node: NodeKey::Status,
                },
                depth: 1,
                expandable: false,
                expanded: false,
            });
            continue;
        }
        let model = source.model.expect("schemas come from a model");
        let mut depth = 1;
        if source.database.is_some() {
            let key = RowKey {
                source: source.name.clone(),
                node: NodeKey::Database,
            };
            let open = expanded.contains(&key);
            out.push(Row {
                key,
                depth,
                expandable: true,
                expanded: open,
            });
            if !open {
                continue;
            }
            depth += 1;
        }
        for schema in model.schemas() {
            push_node(&mut out, source.name, schema.root(), depth, expanded);
        }
    }
    out
}

fn push_node(
    out: &mut Vec<Row>,
    source: &Arc<str>,
    node: NodeRef<'_>,
    depth: usize,
    expanded: &HashSet<RowKey>,
) {
    let key = RowKey {
        source: source.clone(),
        node: NodeKey::Node(node.id()),
    };
    let expandable = visible_groups(node).next().is_some()
        // A schema read to names only still opens (to show it is empty).
        || node.kind() == NodeKind::Schema;
    let open = expandable && expanded.contains(&key);
    out.push(Row {
        key,
        depth,
        expandable,
        expanded: open,
    });
    if !open {
        return;
    }
    for group in visible_groups(node) {
        let key = RowKey {
            source: source.clone(),
            node: NodeKey::Group(node.id(), group),
        };
        let open = expanded.contains(&key);
        out.push(Row {
            key,
            depth: depth + 1,
            expandable: true,
            expanded: open,
        });
        if open {
            for child in group.nodes(node) {
                push_node(out, source, child, depth + 2, expanded);
            }
        }
    }
}

/// The icon of a row (mapped to the module's icons by the view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIcon {
    /// The data source's: its backend's.
    Source,
    Database,
    Schema,
    Group,
    Table,
    View,
    MaterializedView,
    Sequence,
    Routine,
    Procedure,
    Package,
    Type,
    Synonym,
    Column,
    /// A column of the primary key.
    ColumnKey,
    /// A NOT NULL column.
    ColumnNotNull,
    Key,
    ForeignKey,
    Index,
    UniqueIndex,
    Check,
    Trigger,
    Status,
    Error,
}

/// The texts and the icon of a node row.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeView {
    pub icon: RowIcon,
    pub name: String,
    /// Grey text after the name: the type of a column, the columns of an
    /// index, the arguments of a routine.
    pub detail: Option<String>,
    /// The number of nodes of a group.
    pub count: Option<usize>,
}

/// The view of a schema or object node.
pub fn node_view(node: NodeRef<'_>) -> NodeView {
    let (icon, detail) = match node.kind() {
        NodeKind::Schema => (RowIcon::Schema, None),
        NodeKind::Table => (RowIcon::Table, partition_detail(node)),
        NodeKind::View => (RowIcon::View, None),
        NodeKind::MaterializedView => (RowIcon::MaterializedView, None),
        NodeKind::Sequence => (RowIcon::Sequence, None),
        NodeKind::Function => (RowIcon::Routine, routine_detail(node)),
        NodeKind::Procedure => (RowIcon::Procedure, routine_detail(node)),
        NodeKind::Package => (RowIcon::Package, None),
        NodeKind::Type => (RowIcon::Type, None),
        NodeKind::Synonym => (RowIcon::Synonym, None),
        NodeKind::Trigger => (RowIcon::Trigger, trigger_detail(node)),
        NodeKind::Index => index_view(node),
        NodeKind::Column => column_view(node),
        NodeKind::PrimaryKey | NodeKind::UniqueKey => (RowIcon::Key, constraint_detail(node)),
        NodeKind::ForeignKey => (RowIcon::ForeignKey, constraint_detail(node)),
        NodeKind::Check => (RowIcon::Check, constraint_detail(node)),
        NodeKind::Argument => (RowIcon::Column, None),
    };
    NodeView {
        icon,
        name: node.name().to_owned(),
        detail,
        count: None,
    }
}

/// The view of a group row.
pub fn group_view(parent: NodeRef<'_>, group: TreeGroup) -> NodeView {
    NodeView {
        icon: RowIcon::Group,
        name: group.label().to_owned(),
        detail: None,
        count: Some(group.len(parent)),
    }
}

/// `integer (auto increment)`, `char(2) = 'DE'::bpchar`; the icon marks the
/// primary key and NOT NULL.
fn column_view(node: NodeRef<'_>) -> (RowIcon, Option<String>) {
    let Some(column) = node.column() else {
        return (RowIcon::Column, None);
    };
    let in_primary_key = node.parent().is_some_and(|table| {
        table
            .group(Group::Keys)
            .nodes()
            .filter_map(|k| k.constraint())
            .any(|c| c.kind == ConstraintKind::PrimaryKey && c.columns.contains(&column.name))
    });
    let icon = if in_primary_key {
        RowIcon::ColumnKey
    } else if !column.nullable {
        RowIcon::ColumnNotNull
    } else {
        RowIcon::Column
    };
    let mut detail = short_type(&column.data_type);
    if column.auto_increment || column.identity.is_some() {
        detail.push_str(" (auto increment)");
    } else if let Some(default) = &column.default {
        detail.push_str(" = ");
        detail.push_str(default);
    }
    (icon, Some(detail))
}

/// PostgreSQL's long names of the character types in the reference's
/// short spelling: `char(2)`, `varchar(100)`.
fn short_type(data_type: &str) -> String {
    for (long, short) in [("character varying", "varchar"), ("character", "char")] {
        if let Some(rest) = data_type.strip_prefix(long)
            && (rest.is_empty() || rest.starts_with('(') || rest.starts_with('['))
        {
            return format!("{short}{rest}");
        }
    }
    data_type.to_owned()
}

/// `(lower(email::text)) UNIQUE`, `(country) where is_active`.
fn index_view(node: NodeRef<'_>) -> (RowIcon, Option<String>) {
    let Some(index) = node.index() else {
        return (RowIcon::Index, None);
    };
    let mut detail = format!("({})", index.columns.join(", "));
    if index.unique {
        detail.push_str(" UNIQUE");
    }
    if let Some(predicate) = &index.predicate {
        detail.push_str(" where ");
        detail.push_str(predicate);
    }
    let icon = if index.unique {
        RowIcon::UniqueIndex
    } else {
        RowIcon::Index
    };
    (icon, Some(detail))
}

/// `(id)`; a foreign key `(customer_id) → customers(id)`; a check its
/// condition.
fn constraint_detail(node: NodeRef<'_>) -> Option<String> {
    let c = node.constraint()?;
    let columns = format!("({})", c.columns.join(", "));
    Some(match c.kind {
        ConstraintKind::PrimaryKey | ConstraintKind::Unique => columns,
        ConstraintKind::ForeignKey => {
            let table = c.ref_table.as_deref().unwrap_or_default();
            let schema = c
                .ref_schema
                .as_deref()
                .filter(|s| *s != node.schema().name());
            let target = match schema {
                Some(s) => format!("{s}.{table}"),
                None => table.to_owned(),
            };
            format!("{columns} \u{2192} {target}({})", c.ref_columns.join(", "))
        }
        ConstraintKind::Check => c.check.clone().unwrap_or(columns),
    })
}

/// `(p_customer: integer): bigint`; without arguments only `: trigger`.
fn routine_detail(node: NodeRef<'_>) -> Option<String> {
    let Some(ObjectDetail::Routine(r)) = node.object()?.detail.as_ref() else {
        return None;
    };
    let mut out = String::new();
    if !r.arguments.is_empty() {
        let args: Vec<String> = r
            .arguments
            .iter()
            .map(|a| {
                let mode = match a.mode {
                    ArgumentMode::Out => "out ",
                    ArgumentMode::InOut => "inout ",
                    ArgumentMode::Variadic => "variadic ",
                    _ => "",
                };
                match &a.name {
                    Some(name) => format!("{mode}{name}: {}", a.data_type),
                    None => format!("{mode}{}", a.data_type),
                }
            })
            .collect();
        out.push_str(&format!("({})", args.join(", ")));
    }
    if let Some(ret) = &r.return_type {
        out.push_str(": ");
        out.push_str(ret);
    }
    (!out.is_empty()).then_some(out)
}

/// `before update` of a trigger.
fn trigger_detail(node: NodeRef<'_>) -> Option<String> {
    let Some(ObjectDetail::Trigger(t)) = node.object()?.detail.as_ref() else {
        return None;
    };
    let events = t.events.join(", ").to_lowercase();
    match &t.timing {
        Some(timing) => Some(format!("{} {events}", timing.to_lowercase())),
        None => (!events.is_empty()).then_some(events),
    }
}

/// The bounds of a nested partition.
fn partition_detail(node: NodeRef<'_>) -> Option<String> {
    node.is_partition()
        .then(|| node.partition_of().and_then(|p| p.bounds.clone()))
        .flatten()
}

#[cfg(test)]
mod tests;
