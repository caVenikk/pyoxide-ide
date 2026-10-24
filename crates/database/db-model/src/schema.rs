//! One schema of the model: an arena of nodes built from a
//! [`SchemaSnapshot`], its name index and the read API ([`NodeRef`],
//! [`GroupRef`]).

use std::cmp::Ordering;
use std::collections::HashMap;

use db_core::schema::{
    ArgumentInfo, ColumnInfo, ConstraintInfo, IndexInfo, Level, ObjectDetail, ObjectInfo,
    PartitionOf, RelationInfo, SchemaInfo, SchemaSnapshot,
};

use crate::id::{NodeId, PassThroughMap, local_hash, schema_hash};
use crate::kind::{Group, NodeKind};
use crate::name::{NameMatch, caseless_hash, cmp_names};

const NONE: u32 = u32::MAX;

/// How many objects are built between two looks at the cancel flag.
const CANCEL_STRIDE: usize = 512;

/// A build or refresh was cancelled; nothing was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the model update was cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    local: u64,
    kind: NodeKind,
    parent: u32,
    children: Box<[u32]>,
    /// Runs of `children` per group, in tree order: (group, start, end).
    runs: Box<[(Group, u32, u32)]>,
    data: NodeData,
}

#[derive(Debug, Clone, PartialEq)]
enum NodeData {
    Schema,
    /// `structure`: the columns and constraints of a relation, or the
    /// arguments of a routine, are loaded.
    Object {
        info: Box<ObjectInfo>,
        structure: bool,
    },
    /// An index is listed as an object, read as part of its table's
    /// structure, or both.
    Index {
        object: Option<Box<ObjectInfo>>,
        info: Option<Box<IndexInfo>>,
    },
    Column(Box<ColumnInfo>),
    Constraint(Box<ConstraintInfo>),
    /// `name` is the argument's, or `$n` for an unnamed one.
    Argument {
        name: Box<str>,
        info: Box<ArgumentInfo>,
    },
}

impl Node {
    /// The name, kept once: in the facts the node carries.
    fn name<'a>(&'a self, schema: &'a str) -> &'a str {
        match &self.data {
            NodeData::Schema => schema,
            NodeData::Object { info, .. } => &info.name,
            NodeData::Index {
                object: Some(o), ..
            } => &o.name,
            NodeData::Index { info: Some(i), .. } => &i.name,
            NodeData::Index { .. } => "",
            NodeData::Column(c) => &c.name,
            NodeData::Constraint(c) => &c.name,
            NodeData::Argument { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone)]
enum Slots {
    One(u32),
    Many(Vec<u32>),
}

impl Slots {
    fn as_slice(&self) -> &[u32] {
        match self {
            Slots::One(i) => std::slice::from_ref(i),
            Slots::Many(v) => v,
        }
    }

    fn push(&mut self, i: u32) {
        match self {
            Slots::One(first) => *self = Slots::Many(vec![*first, i]),
            Slots::Many(v) => v.push(i),
        }
    }
}

/// One schema: immutable, shared between model versions that did not
/// change it.
#[derive(Debug, Clone)]
pub struct SchemaModel {
    info: SchemaInfo,
    level: Level,
    hash: u64,
    /// `nodes[0]` is the schema node; the rest in tree order.
    nodes: Vec<Node>,
    by_local: PassThroughMap<u32>,
    /// Nodes that are schema objects, by their name ignoring case.
    names: PassThroughMap<Slots>,
}

/// The content decides; the indexes follow from it.
impl PartialEq for SchemaModel {
    fn eq(&self, other: &Self) -> bool {
        self.info == other.info && self.level == other.level && self.nodes == other.nodes
    }
}

impl SchemaModel {
    pub fn info(&self) -> &SchemaInfo {
        &self.info
    }

    pub fn name(&self) -> &str {
        &self.info.name
    }

    /// How much of the schema is loaded.
    pub fn level(&self) -> Level {
        self.level
    }

    pub fn id(&self) -> NodeId {
        NodeId::new(self.hash, 0)
    }

    pub(crate) fn hash(&self) -> u64 {
        self.hash
    }

    /// The schema node: the root of the schema's subtree.
    pub fn root(&self) -> NodeRef<'_> {
        NodeRef {
            schema: self,
            index: 0,
        }
    }

    /// Every node, the schema node first, parents before children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = NodeRef<'_>> + '_ {
        (0..self.nodes.len() as u32).map(move |index| NodeRef {
            schema: self,
            index,
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    pub fn node(&self, id: NodeId) -> Option<NodeRef<'_>> {
        if id.schema_part() != self.hash {
            return None;
        }
        let index = if id.is_schema() {
            0
        } else {
            *self.by_local.get(&id.local_part())?
        };
        Some(NodeRef {
            schema: self,
            index,
        })
    }

    /// The schema objects named `name` (tables, partitions, views,
    /// routines with every overload, indexes, triggers, ...), of one of
    /// `kinds` (all when empty). Does not allocate.
    pub fn lookup<'a, 'q>(
        &'a self,
        name: &'q str,
        mode: NameMatch,
        kinds: &'q [NodeKind],
    ) -> impl Iterator<Item = NodeRef<'a>> + 'q
    where
        'a: 'q,
    {
        let slots = self
            .names
            .get(&caseless_hash(name))
            .map_or(&[][..], Slots::as_slice);
        slots
            .iter()
            .map(move |&index| NodeRef {
                schema: self,
                index,
            })
            .filter(move |n| n.has_kind(kinds) && mode.matches(n.name(), name))
    }

    /// The first of [`lookup`](Self::lookup).
    pub fn find(&self, name: &str, mode: NameMatch, kinds: &[NodeKind]) -> Option<NodeRef<'_>> {
        self.lookup(name, mode, kinds).next()
    }

    /// The objects of a group of the schema node ("tables", "views", ...).
    pub fn group(&self, group: Group) -> GroupRef<'_> {
        self.root().group(group)
    }

    /// The schema as a snapshot, in the canonical order of the model,
    /// without source texts.
    pub fn to_snapshot(&self) -> SchemaSnapshot {
        let mut objects = Vec::new();
        let mut relations = Vec::new();
        for node in &self.nodes {
            match &node.data {
                NodeData::Object { info, structure } => {
                    objects.push((**info).clone());
                    if *structure && node.kind.is_relation() {
                        relations.push(self.relation_of(node));
                    }
                }
                NodeData::Index {
                    object: Some(info), ..
                } => objects.push((**info).clone()),
                _ => {}
            }
        }
        SchemaSnapshot {
            info: self.info.clone(),
            level: self.level,
            objects,
            relations,
            sources: Vec::new(),
        }
    }

    fn relation_of(&self, node: &Node) -> RelationInfo {
        let mut relation = RelationInfo {
            name: node.name(&self.info.name).to_string(),
            ..RelationInfo::default()
        };
        for &child in &node.children {
            match &self.nodes[child as usize].data {
                NodeData::Column(c) => relation.columns.push((**c).clone()),
                NodeData::Constraint(c) => relation.constraints.push((**c).clone()),
                NodeData::Index { info: Some(i), .. } => relation.indexes.push((**i).clone()),
                _ => {}
            }
        }
        relation
    }

    /// Builds the schema from a snapshot. The result does not depend on the
    /// order of the snapshot's lists: objects are sorted, duplicates (same
    /// kind, name, signature and table) are dropped, structure without an
    /// object is dropped.
    pub(crate) fn build(
        snapshot: SchemaSnapshot,
        cancel: &dyn Fn() -> bool,
    ) -> Result<SchemaModel, Cancelled> {
        Builder::run(snapshot, cancel)
    }
}

/// A node of a schema, borrowed from the model.
#[derive(Clone, Copy)]
pub struct NodeRef<'a> {
    schema: &'a SchemaModel,
    index: u32,
}

impl std::fmt::Debug for NodeRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeRef")
            .field("schema", &self.schema.name())
            .field("kind", &self.kind())
            .field("name", &self.name())
            .finish()
    }
}

impl PartialEq for NodeRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.schema, other.schema) && self.index == other.index
    }
}

impl Eq for NodeRef<'_> {}

impl<'a> NodeRef<'a> {
    fn node(&self) -> &'a Node {
        &self.schema.nodes[self.index as usize]
    }

    fn at(&self, index: u32) -> NodeRef<'a> {
        NodeRef {
            schema: self.schema,
            index,
        }
    }

    pub fn id(&self) -> NodeId {
        NodeId::new(self.schema.hash, self.node().local)
    }

    pub fn kind(&self) -> NodeKind {
        self.node().kind
    }

    /// The name as the database stores it. Arguments without a name are
    /// `$1`, `$2`, ...
    pub fn name(&self) -> &'a str {
        self.node().name(&self.schema.info.name)
    }

    /// The schema the node belongs to.
    pub fn schema(&self) -> &'a SchemaModel {
        self.schema
    }

    pub fn parent(&self) -> Option<NodeRef<'a>> {
        match self.node().parent {
            NONE => None,
            p => Some(self.at(p)),
        }
    }

    /// The children in tree order (by group, then by position or name).
    pub fn children(&self) -> impl ExactSizeIterator<Item = NodeRef<'a>> + 'a {
        let this = *self;
        self.node().children.iter().map(move |&i| this.at(i))
    }

    /// The child named `name` of one of `kinds` (all when empty): a column
    /// of a table, an argument of a routine. Does not allocate.
    pub fn child(&self, name: &str, mode: NameMatch, kinds: &[NodeKind]) -> Option<NodeRef<'a>> {
        self.children()
            .find(|c| c.has_kind(kinds) && mode.matches(c.name(), name))
    }

    /// The groups of the node in tree order: every group its kind has,
    /// empty ones included (so the tree can hide them), and any other group
    /// the data put there.
    pub fn groups(&self) -> impl Iterator<Item = GroupRef<'a>> + 'a {
        let this = *self;
        let node = self.node();
        let listed = node.kind.groups();
        let extra = node
            .runs
            .iter()
            .filter(move |(g, _, _)| !listed.contains(g))
            .map(|(g, _, _)| *g);
        listed
            .iter()
            .copied()
            .chain(extra)
            .map(move |g| this.group(g))
    }

    /// One group of the node; empty when the node has none of it.
    pub fn group(&self, group: Group) -> GroupRef<'a> {
        let node = self.node();
        let nodes = node
            .runs
            .iter()
            .find(|(g, _, _)| *g == group)
            .map_or(&[][..], |&(_, start, end)| {
                &node.children[start as usize..end as usize]
            });
        let loaded = !group.needs_structure() || self.structure_loaded();
        GroupRef {
            schema: self.schema,
            group,
            nodes,
            loaded,
        }
    }

    /// The columns and constraints of a relation, or the arguments of a
    /// routine, are known. Always true for other nodes.
    pub fn structure_loaded(&self) -> bool {
        match &self.node().data {
            NodeData::Object { structure, .. } => *structure,
            _ => true,
        }
    }

    /// The facts of a schema object (indexes listed as objects included).
    pub fn object(&self) -> Option<&'a ObjectInfo> {
        match &self.node().data {
            NodeData::Object { info, .. } => Some(info),
            NodeData::Index { object, .. } => object.as_deref(),
            _ => None,
        }
    }

    pub fn column(&self) -> Option<&'a ColumnInfo> {
        match &self.node().data {
            NodeData::Column(c) => Some(c),
            _ => None,
        }
    }

    /// The structure of an index, read with its table.
    pub fn index(&self) -> Option<&'a IndexInfo> {
        match &self.node().data {
            NodeData::Index { info, .. } => info.as_deref(),
            _ => None,
        }
    }

    pub fn constraint(&self) -> Option<&'a ConstraintInfo> {
        match &self.node().data {
            NodeData::Constraint(c) => Some(c),
            _ => None,
        }
    }

    pub fn argument(&self) -> Option<&'a ArgumentInfo> {
        match &self.node().data {
            NodeData::Argument { info, .. } => Some(info),
            _ => None,
        }
    }

    /// The identity arguments of an overloaded routine; empty otherwise.
    pub fn signature(&self) -> &'a str {
        self.object().map_or("", signature)
    }

    pub fn comment(&self) -> Option<&'a str> {
        match &self.node().data {
            NodeData::Schema => self.schema.info.comment.as_deref(),
            NodeData::Column(c) => c.comment.as_deref(),
            _ => self.object().and_then(|o| o.comment.as_deref()),
        }
    }

    /// A table nested under the table it is a partition of.
    pub fn is_partition(&self) -> bool {
        self.kind() == NodeKind::Table && self.parent().is_some_and(|p| p.kind() == NodeKind::Table)
    }

    /// The parent and bounds of a partition, nested or not.
    pub fn partition_of(&self) -> Option<&'a PartitionOf> {
        match self.object()?.detail.as_ref()? {
            ObjectDetail::Table(t) => t.partition_of.as_ref(),
            _ => None,
        }
    }

    fn has_kind(&self, kinds: &[NodeKind]) -> bool {
        kinds.is_empty() || kinds.contains(&self.kind())
    }
}

/// The children of a node in one group.
#[derive(Clone, Copy)]
pub struct GroupRef<'a> {
    schema: &'a SchemaModel,
    group: Group,
    nodes: &'a [u32],
    loaded: bool,
}

impl std::fmt::Debug for GroupRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupRef")
            .field("group", &self.group)
            .field("len", &self.nodes.len())
            .field("loaded", &self.loaded)
            .finish()
    }
}

impl<'a> GroupRef<'a> {
    pub fn group(&self) -> Group {
        self.group
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// No nodes. A group that is not [loaded](Self::is_loaded) is empty
    /// too, but its content is unknown rather than absent.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The content of the group is known: false for columns, keys and
    /// arguments of a schema loaded to names only.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn get(&self, i: usize) -> Option<NodeRef<'a>> {
        self.nodes.get(i).map(|&index| NodeRef {
            schema: self.schema,
            index,
        })
    }

    pub fn nodes(&self) -> impl ExactSizeIterator<Item = NodeRef<'a>> + 'a {
        let schema = self.schema;
        self.nodes
            .iter()
            .map(move |&index| NodeRef { schema, index })
    }
}

fn signature(info: &ObjectInfo) -> &str {
    match &info.detail {
        Some(ObjectDetail::Routine(r)) => &r.signature,
        _ => "",
    }
}

/// The identity of an object within a snapshot: what a delta replaces.
pub(crate) fn same_object(a: &ObjectInfo, b: &ObjectInfo) -> bool {
    a.kind == b.kind && a.name == b.name && signature(a) == signature(b) && a.parent == b.parent
}

fn cmp_objects(a: &ObjectInfo, b: &ObjectInfo) -> Ordering {
    a.kind
        .cmp(&b.kind)
        .then_with(|| cmp_names(&a.name, &b.name))
        .then_with(|| signature(a).cmp(signature(b)))
        .then_with(|| a.parent.cmp(&b.parent))
}

fn rank(parent: NodeKind, group: Group) -> usize {
    parent
        .groups()
        .iter()
        .position(|g| *g == group)
        .unwrap_or(64 + group as usize)
}

struct Builder<'c> {
    nodes: Vec<Node>,
    by_local: PassThroughMap<u32>,
    objects: Vec<Option<ObjectInfo>>,
    relations: HashMap<String, RelationInfo>,
    /// Children objects (indexes, triggers, partitions) per object.
    kids: Vec<Vec<usize>>,
    cancel: &'c dyn Fn() -> bool,
    built: usize,
}

/// A child before it gets its place: sorted by group, then by `key`.
struct Pending {
    rank: usize,
    group: Group,
    index: u32,
}

impl<'c> Builder<'c> {
    fn run(
        snapshot: SchemaSnapshot,
        cancel: &'c dyn Fn() -> bool,
    ) -> Result<SchemaModel, Cancelled> {
        if cancel() {
            return Err(Cancelled);
        }
        let SchemaSnapshot {
            info,
            level,
            mut objects,
            relations,
            sources: _,
        } = snapshot;
        objects.sort_by(cmp_objects);
        objects.dedup_by(|later, earlier| same_object(later, earlier));

        let mut by_relation_name: HashMap<&str, usize> = HashMap::new();
        let mut by_table_name: HashMap<&str, usize> = HashMap::new();
        for (i, o) in objects.iter().enumerate() {
            let kind = NodeKind::from_object(o.kind);
            if kind.is_relation() {
                by_relation_name.entry(o.name.as_str()).or_insert(i);
            }
            if kind == NodeKind::Table {
                by_table_name.entry(o.name.as_str()).or_insert(i);
            }
        }
        // Where each object hangs: under its table (indexes, triggers,
        // partitions of a table of this schema) or under the schema.
        let mut parent_of: Vec<Option<usize>> = objects
            .iter()
            .enumerate()
            .map(|(i, o)| match NodeKind::from_object(o.kind) {
                NodeKind::Index | NodeKind::Trigger => o
                    .parent
                    .as_deref()
                    .and_then(|p| by_relation_name.get(p).copied()),
                NodeKind::Table => match &o.detail {
                    Some(ObjectDetail::Table(t)) => t
                        .partition_of
                        .as_ref()
                        .filter(|p| p.schema == info.name)
                        .and_then(|p| by_table_name.get(p.table.as_str()).copied())
                        .filter(|&p| p != i),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // Partition chains that loop (never in a real database) are cut.
        let n = objects.len();
        for i in 0..n {
            let (mut j, mut steps) = (i, 0);
            while let Some(p) = parent_of[j] {
                j = p;
                steps += 1;
                if j == i {
                    parent_of[i] = None;
                    break;
                }
                if steps > n {
                    break;
                }
            }
        }
        let mut kids = vec![Vec::new(); n];
        let mut top = Vec::new();
        for (i, p) in parent_of.iter().enumerate() {
            match p {
                Some(p) => kids[*p].push(i),
                None => top.push(i),
            }
        }

        let mut relation_map = HashMap::with_capacity(relations.len());
        let mut estimate = 1 + n;
        for mut r in relations {
            estimate += r.columns.len() + r.constraints.len() + r.indexes.len();
            // The name is the key; the builder does not read it again.
            relation_map.entry(std::mem::take(&mut r.name)).or_insert(r);
        }

        let hash = schema_hash(&info.name);
        let mut by_local = PassThroughMap::default();
        by_local.reserve(estimate);
        let mut builder = Builder {
            nodes: Vec::with_capacity(estimate),
            by_local,
            objects: objects.into_iter().map(Some).collect(),
            relations: relation_map,
            kids,
            cancel,
            built: 0,
        };
        builder.nodes.push(Node {
            local: 0,
            kind: NodeKind::Schema,
            parent: NONE,
            children: Box::new([]),
            runs: Box::new([]),
            data: NodeData::Schema,
        });
        let mut children = Vec::with_capacity(top.len());
        for i in top {
            let index = builder.object(i, 0)?;
            children.push(index);
        }
        builder.finish_children(0, children);

        let mut names: PassThroughMap<Slots> = PassThroughMap::default();
        names.reserve(n);
        for (i, node) in builder.nodes.iter().enumerate() {
            let is_object = matches!(
                node.data,
                NodeData::Object { .. }
                    | NodeData::Index {
                        object: Some(_),
                        ..
                    }
            );
            if is_object {
                names
                    .entry(caseless_hash(node.name(&info.name)))
                    .and_modify(|s| s.push(i as u32))
                    .or_insert(Slots::One(i as u32));
            }
        }
        Ok(SchemaModel {
            info,
            level,
            hash,
            nodes: builder.nodes,
            by_local: builder.by_local,
            names,
        })
    }

    /// A free local id for `path`: the hash, or the hash with a counter
    /// when an earlier node of the schema took it.
    fn local(&self, kind: NodeKind, path: &[&str]) -> u64 {
        let mut salt = 0;
        loop {
            let h = local_hash(kind.tag(), path, salt);
            if !self.by_local.contains_key(&h) {
                return h;
            }
            salt += 1;
        }
    }

    fn insert(&mut self, kind: NodeKind, local: u64, parent: u32, data: NodeData) -> u32 {
        let index = self.nodes.len() as u32;
        self.by_local.insert(local, index);
        self.nodes.push(Node {
            local,
            kind,
            parent,
            children: Box::new([]),
            runs: Box::new([]),
            data,
        });
        index
    }

    /// Sorts the pending children by group and stores them with their runs.
    /// Within a group the order they were made in stays.
    fn finish_children(&mut self, parent: u32, children: Vec<u32>) {
        let parent_kind = self.nodes[parent as usize].kind;
        let mut pending: Vec<Pending> = children
            .into_iter()
            .map(|index| {
                let group = Group::of(parent_kind, self.nodes[index as usize].kind);
                Pending {
                    rank: rank(parent_kind, group),
                    group,
                    index,
                }
            })
            .collect();
        if parent == 0 {
            let nodes = &self.nodes;
            pending.sort_by(|a, b| {
                let (x, y) = (&nodes[a.index as usize], &nodes[b.index as usize]);
                a.rank
                    .cmp(&b.rank)
                    .then_with(|| cmp_names(x.name(""), y.name("")))
                    .then_with(|| object_signature(x).cmp(object_signature(y)))
            });
        } else {
            pending.sort_by_key(|p| p.rank);
        }
        let mut runs: Vec<(Group, u32, u32)> = Vec::new();
        for (i, p) in pending.iter().enumerate() {
            match runs.last_mut() {
                Some((g, _, end)) if *g == p.group => *end = i as u32 + 1,
                _ => runs.push((p.group, i as u32, i as u32 + 1)),
            }
        }
        let node = &mut self.nodes[parent as usize];
        node.children = pending.iter().map(|p| p.index).collect();
        node.runs = runs.into_boxed_slice();
    }

    fn object(&mut self, i: usize, parent: u32) -> Result<u32, Cancelled> {
        self.built += 1;
        if self.built.is_multiple_of(CANCEL_STRIDE) && (self.cancel)() {
            return Err(Cancelled);
        }
        let info = self.objects[i].take().expect("each object is placed once");
        let kind = NodeKind::from_object(info.kind);
        let local = match (kind, info.parent.as_deref()) {
            (NodeKind::Index | NodeKind::Trigger, Some(owner)) => {
                self.local(kind, &[owner, &info.name])
            }
            (k, _) if k.is_routine() => self.local(kind, &[&info.name, signature(&info)]),
            _ => self.local(kind, &[&info.name]),
        };
        let relation = if kind.is_relation() {
            self.relations.remove(info.name.as_str())
        } else {
            None
        };
        let arguments = match &info.detail {
            Some(ObjectDetail::Routine(r)) if kind.is_routine() => Some(r.arguments.clone()),
            _ => None,
        };
        // Names the children's ids are made of; most objects have none.
        let (name, sig) = if relation.is_some() || arguments.is_some() {
            (info.name.clone(), signature(&info).to_string())
        } else {
            (String::new(), String::new())
        };
        let data = if kind == NodeKind::Index {
            NodeData::Index {
                object: Some(Box::new(info)),
                info: None,
            }
        } else {
            NodeData::Object {
                info: Box::new(info),
                structure: relation.is_some() || arguments.is_some(),
            }
        };
        let index = self.insert(kind, local, parent, data);

        let kids = std::mem::take(&mut self.kids[i]);
        let mut children = Vec::with_capacity(
            kids.len()
                + relation.as_ref().map_or(0, |r| {
                    r.columns.len() + r.constraints.len() + r.indexes.len()
                })
                + arguments.as_ref().map_or(0, Vec::len),
        );
        if let Some(relation) = relation {
            self.relation(index, &name, relation, &kids, &mut children)?;
        } else {
            for &k in &kids {
                children.push(self.object(k, index)?);
            }
        }
        if let Some(arguments) = arguments {
            for (n, arg) in arguments.into_iter().enumerate() {
                let position = (n + 1).to_string();
                let arg_name = arg.name.clone().unwrap_or_else(|| format!("${position}"));
                let local = self.local(
                    NodeKind::Argument,
                    &[name.as_str(), sig.as_str(), position.as_str()],
                );
                let data = NodeData::Argument {
                    name: arg_name.into(),
                    info: Box::new(arg),
                };
                let child = self.insert(NodeKind::Argument, local, index, data);
                children.push(child);
            }
        }
        self.finish_children(index, children);
        Ok(index)
    }

    /// Columns, keys, indexes, triggers and partitions of a relation.
    fn relation(
        &mut self,
        index: u32,
        name: &str,
        relation: RelationInfo,
        kids: &[usize],
        children: &mut Vec<u32>,
    ) -> Result<(), Cancelled> {
        let RelationInfo {
            columns: mut cols,
            constraints: mut cons,
            indexes,
            ..
        } = relation;
        cols.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| cmp_names(&a.name, &b.name))
        });
        for c in cols {
            let local = self.local(NodeKind::Column, &[name, c.name.as_str()]);
            children.push(self.insert(
                NodeKind::Column,
                local,
                index,
                NodeData::Column(Box::new(c)),
            ));
        }
        cons.sort_by(|a, b| {
            NodeKind::from_constraint(a.kind)
                .cmp(&NodeKind::from_constraint(b.kind))
                .then_with(|| cmp_names(&a.name, &b.name))
        });
        for c in cons {
            let kind = NodeKind::from_constraint(c.kind);
            let local = self.local(kind, &[name, c.name.as_str()]);
            children.push(self.insert(kind, local, index, NodeData::Constraint(Box::new(c))));
        }
        // Indexes: the ones listed as objects and the ones read with the
        // table, joined by name.
        let mut index_infos: Vec<Option<IndexInfo>> = indexes.into_iter().map(Some).collect();
        let mut merged: Vec<(String, Option<usize>, Option<IndexInfo>)> = Vec::new();
        for &k in kids {
            let o = self.objects[k].as_ref().expect("kids are not placed yet");
            if o.kind == db_core::schema::ObjectKind::Index {
                let info = index_infos
                    .iter_mut()
                    .find(|i| i.as_ref().is_some_and(|i| i.name == o.name))
                    .and_then(Option::take);
                merged.push((o.name.clone(), Some(k), info));
            }
        }
        for info in index_infos.into_iter().flatten() {
            if !merged.iter().any(|(n, _, _)| *n == info.name) {
                merged.push((info.name.clone(), None, Some(info)));
            }
        }
        merged.sort_by(|a, b| cmp_names(&a.0, &b.0));
        for (index_name, object, info) in merged {
            let object = object.map(|k| {
                self.built += 1;
                Box::new(self.objects[k].take().expect("each object is placed once"))
            });
            let local = self.local(NodeKind::Index, &[name, index_name.as_str()]);
            let data = NodeData::Index {
                object,
                info: info.map(Box::new),
            };
            children.push(self.insert(NodeKind::Index, local, index, data));
        }
        for &k in kids {
            if self.objects[k].is_some() {
                children.push(self.object(k, index)?);
            }
        }
        Ok(())
    }
}

fn object_signature(node: &Node) -> &str {
    match &node.data {
        NodeData::Object { info, .. } => signature(info),
        _ => "",
    }
}
