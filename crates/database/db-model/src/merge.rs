//! Changes applied to the flat form of a schema ([`SchemaSnapshot`]); the
//! model rebuilds the schema from the result. Working on the flat form makes
//! every change idempotent and the result independent of how it was reached.

use std::collections::HashSet;

use db_core::schema::{Level, ObjectInfo, ObjectKind, ObjectRef, SchemaDelta, SchemaSnapshot};

use crate::kind::NodeKind;
use crate::schema::same_object;

/// Removes the objects `drop` selects, with the indexes and triggers of the
/// tables and views among them and those relations' structure: the database
/// drops them together.
fn drop_objects(snapshot: &mut SchemaSnapshot, drop: impl Fn(&ObjectInfo) -> bool) {
    let mut relations: HashSet<String> = HashSet::new();
    snapshot.objects.retain(|o| {
        let gone = drop(o);
        if gone && NodeKind::from_object(o.kind).is_relation() {
            relations.insert(o.name.clone());
        }
        !gone
    });
    if relations.is_empty() {
        return;
    }
    snapshot.objects.retain(|o| {
        !(matches!(o.kind, ObjectKind::Index | ObjectKind::Trigger)
            && o.parent.as_ref().is_some_and(|p| relations.contains(p)))
    });
    snapshot.relations.retain(|r| !relations.contains(&r.name));
}

/// The level of a schema first seen in a delta: what its content shows.
fn level_of(delta: &SchemaDelta) -> Level {
    if !delta.sources.is_empty() {
        Level::Full
    } else if !delta.relations.is_empty() {
        Level::Structure
    } else {
        Level::Names
    }
}

/// `old` with `delta` applied: dropped objects removed (by name and kind, so
/// every overload of a dropped routine and every trigger of that name),
/// changed objects replaced by identity (kind, name, signature, table), new
/// ones added, structure replaced per relation. Applying the same delta
/// twice gives the same result.
pub(crate) fn apply_delta(old: Option<SchemaSnapshot>, delta: SchemaDelta) -> SchemaSnapshot {
    let level = level_of(&delta);
    let SchemaDelta {
        info,
        objects,
        relations,
        sources: _,
        dropped,
    } = delta;
    let mut snapshot = old.unwrap_or_else(|| SchemaSnapshot {
        info: info.clone(),
        level,
        objects: Vec::new(),
        relations: Vec::new(),
        sources: Vec::new(),
    });
    snapshot.info = info;

    let dropped: HashSet<(ObjectKind, &str)> =
        dropped.iter().map(|d| (d.kind, d.name.as_str())).collect();
    if !dropped.is_empty() {
        drop_objects(&mut snapshot, |o| {
            dropped.contains(&(o.kind, o.name.as_str()))
        });
    }

    let changed: HashSet<(ObjectKind, &str)> =
        objects.iter().map(|o| (o.kind, o.name.as_str())).collect();
    snapshot.objects.retain(|o| {
        !(changed.contains(&(o.kind, o.name.as_str())) && objects.iter().any(|n| same_object(n, o)))
    });
    snapshot.objects.extend(objects);

    let replaced: HashSet<&str> = relations.iter().map(|r| r.name.as_str()).collect();
    snapshot
        .relations
        .retain(|r| !replaced.contains(r.name.as_str()));
    snapshot.relations.extend(relations);
    snapshot
}

/// `snapshot` without the objects of `kinds` (all when empty) that are not
/// in `live`: removal by the list of what still exists, for databases that
/// report deletions only that way.
pub(crate) fn retain_live(
    mut snapshot: SchemaSnapshot,
    kinds: &[ObjectKind],
    live: &[ObjectRef],
) -> SchemaSnapshot {
    let schema = snapshot.info.name.clone();
    let live: HashSet<(ObjectKind, &str)> = live
        .iter()
        .filter(|r| r.schema == schema)
        .map(|r| (r.kind, r.name.as_str()))
        .collect();
    drop_objects(&mut snapshot, |o| {
        (kinds.is_empty() || kinds.contains(&o.kind)) && !live.contains(&(o.kind, o.name.as_str()))
    });
    snapshot
}
