//! Checks for the bulk introspection contract (`Introspector::load`), usable
//! from the conformance cases and from a driver's own tests.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use db_core::schema::{
    ColumnInfo, ConstraintInfo, IndexInfo, Level, ObjectInfo, RelationInfo, SchemaSnapshot, Scope,
    Snapshot, SourceInfo, introspector, load_per_object,
};
use db_core::{Backend, Connection, ExecOptions, Outcome, Result, TxMode, Value};

/// A connection that counts the statements it runs. Wrap a connection to
/// show that a call costs a fixed number of round trips.
pub struct CountingConnection<'a> {
    inner: &'a dyn Connection,
    statements: AtomicUsize,
}

impl<'a> CountingConnection<'a> {
    pub fn new(inner: &'a dyn Connection) -> Self {
        Self {
            inner,
            statements: AtomicUsize::new(0),
        }
    }

    /// Statements run since the connection was wrapped or last reset.
    pub fn statements(&self) -> usize {
        self.statements.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.statements.store(0, Ordering::SeqCst);
    }
}

#[async_trait]
impl Connection for CountingConnection<'_> {
    fn backend(&self) -> Backend {
        self.inner.backend()
    }

    async fn execute(&self, sql: &str, params: &[Value], opts: &ExecOptions) -> Result<Outcome> {
        self.statements.fetch_add(1, Ordering::SeqCst);
        self.inner.execute(sql, params, opts).await
    }

    async fn cancel(&self) -> Result<()> {
        self.inner.cancel().await
    }

    async fn server_version(&self) -> Result<String> {
        self.inner.server_version().await
    }

    async fn ping(&self) -> Result<()> {
        self.inner.ping().await
    }

    async fn tx_mode(&self) -> TxMode {
        self.inner.tx_mode().await
    }

    async fn set_tx_mode(&self, mode: TxMode) -> Result<()> {
        self.inner.set_tx_mode(mode).await
    }

    async fn commit(&self) -> Result<()> {
        self.inner.commit().await
    }

    async fn rollback(&self) -> Result<()> {
        self.inner.rollback().await
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await
    }
}

/// Statements `Introspector::load` runs for `scope` at `level`.
pub async fn load_statements(conn: &dyn Connection, scope: &Scope, level: Level) -> usize {
    let counting = CountingConnection::new(conn);
    introspector(&counting)
        .load(scope, level)
        .await
        .unwrap_or_else(|e| panic!("load {level:?}: {e:?}"));
    counting.statements()
}

/// What the per-object reads do not return, removed.
fn base_object(o: &ObjectInfo) -> ObjectInfo {
    ObjectInfo {
        owner: None,
        detail: None,
        ..o.clone()
    }
}

fn base_column(c: &ColumnInfo) -> ColumnInfo {
    ColumnInfo {
        identity: None,
        generated: None,
        ..c.clone()
    }
}

fn base_index(i: &IndexInfo) -> IndexInfo {
    IndexInfo {
        predicate: None,
        include: Vec::new(),
        expressions: Vec::new(),
        ..i.clone()
    }
}

fn base_constraint(c: &ConstraintInfo) -> ConstraintInfo {
    ConstraintInfo {
        on_update: None,
        match_type: None,
        deferrable: false,
        initially_deferred: false,
        disabled: false,
        not_validated: false,
        ..c.clone()
    }
}

/// A schema reduced to the fields both kinds of read fill, in a canonical
/// order, limited to the objects `keep` accepts (by name, case-insensitive
/// comparison is up to `keep`).
fn normalized(s: &SchemaSnapshot, keep: &dyn Fn(&str) -> bool) -> NormalizedSchema {
    let mut objects: Vec<ObjectInfo> = s
        .objects
        .iter()
        .filter(|o| keep(&o.name))
        .map(base_object)
        .collect();
    objects.sort_by(|a, b| (a.kind, &a.name, &a.parent).cmp(&(b.kind, &b.name, &b.parent)));

    let mut relations: Vec<RelationInfo> = s
        .relations
        .iter()
        .filter(|r| keep(&r.name))
        .map(|r| {
            let mut columns: Vec<ColumnInfo> = r.columns.iter().map(base_column).collect();
            columns.sort_by_key(|c| c.position);
            let mut indexes: Vec<IndexInfo> = r.indexes.iter().map(base_index).collect();
            indexes.sort_by(|a, b| a.name.cmp(&b.name));
            let mut constraints: Vec<ConstraintInfo> =
                r.constraints.iter().map(base_constraint).collect();
            constraints.sort_by(|a, b| (a.kind as u8, &a.name).cmp(&(b.kind as u8, &b.name)));
            RelationInfo {
                name: r.name.clone(),
                columns,
                indexes,
                constraints,
            }
        })
        .collect();
    relations.sort_by(|a, b| a.name.cmp(&b.name));

    // Overloads and same-named triggers are one text in the per-object
    // read: join them in the order they were read.
    let mut sources: Vec<SourceInfo> = Vec::new();
    for source in s.sources.iter().filter(|x| keep(&x.name)) {
        match sources
            .iter_mut()
            .find(|x| x.name == source.name && x.kind == source.kind)
        {
            Some(joined) => {
                joined.text.push_str("\n\n");
                joined.text.push_str(&source.text);
            }
            None => sources.push(SourceInfo {
                signature: String::new(),
                ..source.clone()
            }),
        }
    }
    sources.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));

    NormalizedSchema {
        name: s.info.name.clone(),
        is_system: s.info.is_system,
        is_current: s.info.is_current,
        objects,
        relations,
        sources,
    }
}

#[derive(Debug, PartialEq)]
struct NormalizedSchema {
    name: String,
    is_system: bool,
    is_current: bool,
    objects: Vec<ObjectInfo>,
    relations: Vec<RelationInfo>,
    sources: Vec<SourceInfo>,
}

fn normalized_snapshot(s: &Snapshot, keep: &dyn Fn(&str) -> bool) -> Vec<NormalizedSchema> {
    s.schemas.iter().map(|x| normalized(x, keep)).collect()
}

/// Lists equal, or the first difference.
fn same<T: std::fmt::Debug + PartialEq>(what: &str, bulk: &[T], reference: &[T]) {
    for (i, (x, y)) in bulk.iter().zip(reference).enumerate() {
        assert!(
            x == y,
            "{what}, item {i} differs\n  bulk:       {x:?}\n  per-object: {y:?}"
        );
    }
    assert_eq!(
        bulk.len(),
        reference.len(),
        "{what}: {} from the bulk load, {} from the per-object reads; the extra one is {:?}",
        bulk.len(),
        reference.len(),
        bulk.get(reference.len()).or(reference.get(bulk.len()))
    );
}

/// `Introspector::load` returns, for the objects `keep` accepts, what the
/// per-object reads return (the optional fields aside), and the snapshot
/// says which level it has.
pub async fn assert_load_matches_per_object(
    conn: &dyn Connection,
    scope: &Scope,
    level: Level,
    keep: &dyn Fn(&str) -> bool,
) -> Snapshot {
    let intro = introspector(conn);
    let bulk = intro
        .load(scope, level)
        .await
        .unwrap_or_else(|e| panic!("load {level:?}: {e:?}"));
    let per_object = load_per_object(&*intro, conn.dialect(), scope, level)
        .await
        .unwrap_or_else(|e| panic!("per-object read {level:?}: {e:?}"));
    assert_eq!(bulk.level, level, "level of the snapshot");
    assert!(
        bulk.schemas.iter().all(|s| s.level == level),
        "level of the schemas"
    );
    assert_eq!(
        bulk.schemas.len(),
        per_object.schemas.len(),
        "schemas of {scope:?}"
    );
    let (a, b) = (
        normalized_snapshot(&bulk, keep),
        normalized_snapshot(&per_object, keep),
    );
    for (bulk_schema, reference) in a.iter().zip(&b) {
        assert_eq!(bulk_schema.name, reference.name, "schema order");
        let at = format!("{} at {level:?}", reference.name);
        same(
            &format!("objects of {at}"),
            &bulk_schema.objects,
            &reference.objects,
        );
        assert_eq!(
            bulk_schema.relations.len(),
            reference.relations.len(),
            "number of tables and views of {at}"
        );
        for (x, y) in bulk_schema.relations.iter().zip(&reference.relations) {
            assert_eq!(x.name, y.name, "tables and views of {at}");
            let at = format!("{}.{} ({at})", reference.name, x.name);
            same(&format!("columns of {at}"), &x.columns, &y.columns);
            same(&format!("indexes of {at}"), &x.indexes, &y.indexes);
            same(
                &format!("constraints of {at}"),
                &x.constraints,
                &y.constraints,
            );
        }
        same(
            &format!("sources of {at}"),
            &bulk_schema.sources,
            &reference.sources,
        );
        assert_eq!(
            bulk_schema, reference,
            "schema {} at {level:?}",
            reference.name
        );
    }
    bulk
}
