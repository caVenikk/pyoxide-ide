//! The model of one data source: immutable, shared per schema between
//! versions, swapped whole by [`ModelHandle`].

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use db_core::DataSourceId;
use db_core::schema::{
    ChangeToken, Delta, ObjectKind, ObjectRef, SchemaDelta, SchemaSnapshot, Snapshot,
};
use name_match::Matcher;

use crate::id::{NodeId, PassThroughMap};
use crate::kind::NodeKind;
use crate::merge;
use crate::name::{NameMatch, caseless_hash, cmp_names};
use crate::schema::{Cancelled, NodeRef, SchemaModel};

fn never() -> bool {
    false
}

/// The metadata of one data source at one moment. Reading never locks and
/// never allocates for lookups; every change makes a new `Model` that shares
/// the schemas it did not touch (`Arc<SchemaModel>`), so a reader holding an
/// `Arc<Model>` keeps a consistent view while a refresh builds the next one
/// on another thread, and a cancelled or failed refresh changes nothing.
#[derive(Debug, Clone)]
pub struct Model {
    data_source: DataSourceId,
    token: Option<ChangeToken>,
    /// Sorted by name (ignoring case, then exactly).
    schemas: Vec<Arc<SchemaModel>>,
    by_hash: PassThroughMap<u32>,
    names: PassThroughMap<Vec<u32>>,
}

impl PartialEq for Model {
    fn eq(&self, other: &Self) -> bool {
        self.data_source == other.data_source
            && self.token == other.token
            && self.schemas.len() == other.schemas.len()
            && self
                .schemas
                .iter()
                .zip(&other.schemas)
                .all(|(a, b)| Arc::ptr_eq(a, b) || **a == **b)
    }
}

/// A node found by [`Model::search`].
#[derive(Debug, Clone)]
pub struct Found<'a> {
    pub node: NodeRef<'a>,
    /// Higher is better (`name_match`'s matching degree).
    pub degree: i32,
    /// Char ranges of the name that matched.
    pub fragments: Vec<Range<usize>>,
}

impl Model {
    /// A model with no schemas: a data source never loaded.
    pub fn empty(data_source: DataSourceId) -> Self {
        Self::index(data_source, None, Vec::new())
    }

    /// The model of a full pass. The snapshot's source texts are not part of
    /// the model; give the snapshot to a [`SourceStore`](crate::SourceStore)
    /// first.
    pub fn from_snapshot(data_source: DataSourceId, snapshot: Snapshot) -> Self {
        Self::from_snapshot_cancellable(data_source, snapshot, &never).expect("never cancelled")
    }

    /// As [`from_snapshot`](Self::from_snapshot); `cancel` is asked between
    /// schemas and every few hundred objects.
    pub fn from_snapshot_cancellable(
        data_source: DataSourceId,
        snapshot: Snapshot,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, Cancelled> {
        Self::from_parts(data_source, snapshot.token, snapshot.schemas, cancel)
    }

    pub(crate) fn from_parts(
        data_source: DataSourceId,
        token: Option<ChangeToken>,
        schemas: Vec<SchemaSnapshot>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, Cancelled> {
        let mut built = Vec::with_capacity(schemas.len());
        for schema in schemas {
            built.push(Arc::new(SchemaModel::build(schema, cancel)?));
        }
        Ok(Self::index(data_source, token, built))
    }

    fn index(
        data_source: DataSourceId,
        token: Option<ChangeToken>,
        mut schemas: Vec<Arc<SchemaModel>>,
    ) -> Self {
        schemas.sort_by(|a, b| cmp_names(a.name(), b.name()));
        // One schema per exact name; the last one given wins.
        schemas.dedup_by(|later, earlier| {
            let same = later.name() == earlier.name();
            if same {
                std::mem::swap(later, earlier);
            }
            same
        });
        let mut by_hash = PassThroughMap::default();
        let mut names: PassThroughMap<Vec<u32>> = PassThroughMap::default();
        for (i, s) in schemas.iter().enumerate() {
            by_hash.insert(s.hash(), i as u32);
            names
                .entry(caseless_hash(s.name()))
                .or_default()
                .push(i as u32);
        }
        Self {
            data_source,
            token,
            schemas,
            by_hash,
            names,
        }
    }

    /// This model with the schemas of `snapshot` replaced (or added); the
    /// other schemas are kept as they are. A schema whose content did not
    /// change keeps its `Arc`, so readers can tell it did not change.
    pub fn with_snapshot(&self, snapshot: Snapshot) -> Self {
        self.with_snapshot_cancellable(snapshot, &never)
            .expect("never cancelled")
    }

    pub fn with_snapshot_cancellable(
        &self,
        snapshot: Snapshot,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, Cancelled> {
        let mut replaced = Vec::with_capacity(snapshot.schemas.len());
        for schema in snapshot.schemas {
            let built = SchemaModel::build(schema, cancel)?;
            replaced.push(self.reuse(built));
        }
        // The stamps of the schemas just read come from this pass, those of
        // the others stay.
        let names: Vec<&str> = replaced.iter().map(|s| s.name()).collect();
        let token = ChangeToken::merged(self.token.as_ref(), snapshot.token.as_ref(), &names);
        Ok(self.replace(token, replaced, &[]))
    }

    /// This model with `delta` merged: changed and new objects replaced or
    /// added, dropped ones and dropped schemas removed, the token updated.
    /// Idempotent: merging the same delta again changes nothing.
    pub fn with_delta(&self, delta: Delta) -> Self {
        self.with_delta_cancellable(delta, &never)
            .expect("never cancelled")
    }

    pub fn with_delta_cancellable(
        &self,
        delta: Delta,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, Cancelled> {
        let mut replaced = Vec::with_capacity(delta.schemas.len());
        for schema_delta in delta.schemas {
            let old = self
                .schema_arc(&schema_delta.info.name)
                .map(|s| s.to_snapshot());
            let merged = merge::apply_delta(old, schema_delta);
            let built = SchemaModel::build(merged, cancel)?;
            replaced.push(self.reuse(built));
        }
        let dropped: Vec<&str> = delta.dropped_schemas.iter().map(String::as_str).collect();
        Ok(self.replace(Some(delta.token), replaced, &dropped))
    }

    /// This model with the changes to one schema merged and the token left
    /// as it is: the way to apply a partial read (`load_objects`). The
    /// objects read have newer stamps than the token holds, so the next
    /// incremental pass reads them once more; nothing is lost.
    pub fn with_schema_delta(&self, delta: SchemaDelta) -> Self {
        self.with_schema_delta_cancellable(delta, &never)
            .expect("never cancelled")
    }

    pub fn with_schema_delta_cancellable(
        &self,
        delta: SchemaDelta,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self, Cancelled> {
        let old = self.schema_arc(&delta.info.name).map(|s| s.to_snapshot());
        let merged = merge::apply_delta(old, delta);
        let built = SchemaModel::build(merged, cancel)?;
        let replaced = vec![self.reuse(built)];
        Ok(self.replace(self.token.clone(), replaced, &[]))
    }

    /// This model without the objects of `schema` of `kinds` (all when
    /// empty) that are missing from `live`, with the indexes and triggers of
    /// removed tables. `live` may name objects of other schemas; they are
    /// ignored.
    pub fn retain_live(&self, schema: &str, kinds: &[ObjectKind], live: &[ObjectRef]) -> Self {
        let Some(old) = self.schema_arc(schema) else {
            return self.clone();
        };
        let kept = merge::retain_live(old.to_snapshot(), kinds, live);
        let built = SchemaModel::build(kept, &never).expect("never cancelled");
        self.replace(self.token.clone(), vec![self.reuse(built)], &[])
    }

    /// This model without the schemas named exactly `names`.
    pub fn without_schemas(&self, names: &[&str]) -> Self {
        self.replace(self.token.clone(), Vec::new(), names)
    }

    /// The old `Arc` when the rebuilt schema equals it.
    fn reuse(&self, built: SchemaModel) -> Arc<SchemaModel> {
        match self.schema_arc(built.name()) {
            Some(old) if **old == built => old.clone(),
            _ => Arc::new(built),
        }
    }

    fn replace(
        &self,
        token: Option<ChangeToken>,
        replaced: Vec<Arc<SchemaModel>>,
        dropped: &[&str],
    ) -> Self {
        let mut schemas: Vec<Arc<SchemaModel>> = self
            .schemas
            .iter()
            .filter(|s| {
                !dropped.contains(&s.name()) && !replaced.iter().any(|r| r.name() == s.name())
            })
            .cloned()
            .collect();
        schemas.extend(replaced);
        Self::index(self.data_source.clone(), token, schemas)
    }

    pub fn data_source(&self) -> &DataSourceId {
        &self.data_source
    }

    /// Where the database was at the last pass or delta merged, for the next
    /// incremental refresh.
    pub fn token(&self) -> Option<&ChangeToken> {
        self.token.as_ref()
    }

    /// The schemas in tree order.
    pub fn schemas(&self) -> impl ExactSizeIterator<Item = &SchemaModel> + '_ {
        self.schemas.iter().map(|s| &**s)
    }

    /// The shared schema named exactly `name`: compare with `Arc::ptr_eq`
    /// to tell whether a refresh changed it.
    pub fn schema_arc(&self, name: &str) -> Option<&Arc<SchemaModel>> {
        self.schemas_named(name, NameMatch::Exact)
            .next()
            .map(|i| &self.schemas[i])
    }

    fn schemas_named<'a, 'q>(
        &'a self,
        name: &'q str,
        mode: NameMatch,
    ) -> impl Iterator<Item = usize> + 'q
    where
        'a: 'q,
    {
        self.names
            .get(&caseless_hash(name))
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|&i| i as usize)
            .filter(move |&i| mode.matches(self.schemas[i].name(), name))
    }

    /// The schema named `name`; with [`NameMatch::IgnoreCase`] the first in
    /// tree order of those that match.
    pub fn schema(&self, name: &str, mode: NameMatch) -> Option<&SchemaModel> {
        self.schemas_named(name, mode)
            .next()
            .map(|i| &*self.schemas[i])
    }

    /// The node with this id, if it is in the model.
    pub fn node(&self, id: NodeId) -> Option<NodeRef<'_>> {
        let &i = self.by_hash.get(&id.schema_part())?;
        self.schemas[i as usize].node(id)
    }

    /// Schema objects named `name` in the schema named `schema`, of one of
    /// `kinds` (all when empty). Qualified-name resolution: one hash lookup
    /// for the schema, one for the name, no allocation.
    pub fn lookup<'a, 'q>(
        &'a self,
        schema: &'q str,
        name: &'q str,
        mode: NameMatch,
        kinds: &'q [NodeKind],
    ) -> impl Iterator<Item = NodeRef<'a>> + 'q
    where
        'a: 'q,
    {
        self.schemas_named(schema, mode)
            .flat_map(move |i| self.schemas[i].lookup(name, mode, kinds))
    }

    /// The first of [`lookup`](Self::lookup).
    pub fn find(
        &self,
        schema: &str,
        name: &str,
        mode: NameMatch,
        kinds: &[NodeKind],
    ) -> Option<NodeRef<'_>> {
        self.lookup(schema, name, mode, kinds).next()
    }

    /// Schema objects named `name` in every schema, in tree order.
    pub fn lookup_everywhere<'a, 'q>(
        &'a self,
        name: &'q str,
        mode: NameMatch,
        kinds: &'q [NodeKind],
    ) -> impl Iterator<Item = NodeRef<'a>> + 'q
    where
        'a: 'q,
    {
        self.schemas
            .iter()
            .flat_map(move |s| s.lookup(name, mode, kinds))
    }

    /// Nodes whose name `matcher` matches (speed search, Go to Symbol), of
    /// one of `kinds` (schema objects when empty), best first, at most
    /// `limit`.
    pub fn search<'a>(
        &'a self,
        matcher: &Matcher,
        kinds: &[NodeKind],
        limit: usize,
    ) -> Vec<Found<'a>> {
        let kinds = if kinds.is_empty() {
            &NodeKind::OBJECTS[..]
        } else {
            kinds
        };
        let mut found = Vec::new();
        let mut chars: Vec<char> = Vec::new();
        for schema in &self.schemas {
            for node in schema.nodes().filter(|n| kinds.contains(&n.kind())) {
                chars.clear();
                chars.extend(node.name().chars());
                if let Some(m) = matcher.match_chars(&chars) {
                    found.push(Found {
                        node,
                        degree: m.degree,
                        fragments: m.fragments,
                    });
                }
            }
        }
        found.sort_by(|a, b| {
            b.degree
                .cmp(&a.degree)
                .then_with(|| cmp_names(a.node.name(), b.node.name()))
                .then_with(|| cmp_names(a.node.schema().name(), b.node.schema().name()))
        });
        found.truncate(limit);
        found
    }

    /// Nodes in all schemas, schema nodes included.
    pub fn len(&self) -> usize {
        self.schemas.iter().map(|s| s.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }

    /// The model as a snapshot (canonical order, no source texts). Building
    /// a model from it gives an equal model.
    pub fn to_snapshot(&self) -> Snapshot {
        Snapshot {
            level: self
                .schemas
                .iter()
                .map(|s| s.level())
                .max()
                .unwrap_or_default(),
            token: self.token.clone(),
            schemas: self.schemas.iter().map(|s| s.to_snapshot()).collect(),
        }
    }
}

/// The current model of a data source. Readers take the `Arc` and read
/// without holding any lock; a writer builds the next model from the current
/// one and swaps it in at the end, so a refresh that is cancelled or fails
/// leaves the current model in place.
#[derive(Debug)]
pub struct ModelHandle {
    current: RwLock<Arc<Model>>,
    /// Serialises writers so that two refreshes do not lose each other's
    /// changes; readers never take it.
    writer: Mutex<()>,
    generation: AtomicU64,
}

impl ModelHandle {
    pub fn new(model: Model) -> Self {
        Self {
            current: RwLock::new(Arc::new(model)),
            writer: Mutex::new(()),
            generation: AtomicU64::new(0),
        }
    }

    /// The current model.
    pub fn get(&self) -> Arc<Model> {
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Increases with every swap: a cheap way for the UI to see a change.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Replaces the model (a cache load, a full reload).
    pub fn set(&self, model: Model) -> Arc<Model> {
        let _writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.swap(Arc::new(model))
    }

    /// Builds the next model from the current one with `f`, without holding
    /// the read side, and swaps it in when `f` returns one. `Ok(None)` and
    /// errors (a cancel, a failed read) keep the current model.
    pub fn update<E>(
        &self,
        f: impl FnOnce(&Model) -> Result<Option<Model>, E>,
    ) -> Result<Arc<Model>, E> {
        let _writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let base = self.get();
        match f(&base)? {
            Some(next) => Ok(self.swap(Arc::new(next))),
            None => Ok(base),
        }
    }

    fn swap(&self, next: Arc<Model>) -> Arc<Model> {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = next.clone();
        self.generation.fetch_add(1, Ordering::AcqRel);
        next
    }
}
