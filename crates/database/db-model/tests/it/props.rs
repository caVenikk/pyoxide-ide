//! Property tests on random snapshots: the model does not depend on the
//! order of the input, survives its own round trip, and a delta from one
//! snapshot to another gives the model of the other, idempotently, with ids
//! kept.
use std::collections::HashMap;
use std::sync::Arc;

use db_core::DataSourceId;
use db_core::schema::{Delta, Level, ObjectKind, ObjectRef, SchemaDelta, Snapshot};
use db_model::{Model, NodeId, NodeKind};

use crate::fixture::{Rng, mutate, random_snapshot};

const CASES: u64 = 300;

fn ds() -> DataSourceId {
    DataSourceId::new("props")
}

/// What a correct introspector reports between `old` and `new`.
fn delta(old: &Snapshot, new: &Snapshot) -> Delta {
    let dropped_schemas = old
        .schemas
        .iter()
        .filter(|s| new.schema(&s.info.name).is_none())
        .map(|s| s.info.name.clone())
        .collect();
    let schemas = new
        .schemas
        .iter()
        .map(|s1| {
            let s0 = old.schema(&s1.info.name);
            let changed: Vec<_> = s1
                .objects
                .iter()
                .filter(|o| !s0.is_some_and(|s0| s0.objects.contains(o)))
                .collect();
            // A table comes whole, with all its indexes and triggers.
            let tables: Vec<&str> = changed
                .iter()
                .filter(|o| !matches!(o.kind, ObjectKind::Index | ObjectKind::Trigger))
                .map(|o| o.name.as_str())
                .collect();
            let objects: Vec<_> = s1
                .objects
                .iter()
                .filter(|o| {
                    changed.contains(o)
                        || (matches!(o.kind, ObjectKind::Index | ObjectKind::Trigger)
                            && o.parent.as_deref().is_some_and(|p| tables.contains(&p)))
                })
                .cloned()
                .collect();
            let dropped = s0
                .map(|s0| {
                    s0.objects
                        .iter()
                        .filter(|p| {
                            !s1.objects
                                .iter()
                                .any(|o| o.kind == p.kind && o.name == p.name)
                        })
                        .map(|p| ObjectRef {
                            schema: p.schema.clone(),
                            name: p.name.clone(),
                            kind: p.kind,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let relations = s1
                .relations
                .iter()
                .filter(|r| !s0.is_some_and(|s0| s0.relations.contains(r)))
                .cloned()
                .collect();
            SchemaDelta {
                info: s1.info.clone(),
                objects,
                relations,
                sources: Vec::new(),
                dropped,
            }
        })
        .collect();
    Delta {
        token: new.token.clone().expect("random snapshots have tokens"),
        schemas,
        dropped_schemas,
        unseen: vec![],
    }
}

/// The same snapshot with every list in another order.
fn shuffled(rng: &mut Rng, snapshot: &Snapshot) -> Snapshot {
    let mut s = snapshot.clone();
    rng.shuffle(&mut s.schemas);
    for schema in &mut s.schemas {
        rng.shuffle(&mut schema.objects);
        rng.shuffle(&mut schema.relations);
        for r in &mut schema.relations {
            rng.shuffle(&mut r.columns);
            rng.shuffle(&mut r.indexes);
        }
    }
    s
}

/// A node by what a user would call it: schema, kind, name, the name of the
/// object it belongs to, signature. Keys that occur twice (unnamed checks)
/// are left out.
fn keys(model: &Model) -> HashMap<(String, NodeKind, String, String, String), NodeId> {
    let mut map = HashMap::new();
    let mut twice = Vec::new();
    for schema in model.schemas() {
        for node in schema.nodes() {
            let owner = node
                .parent()
                .filter(|p| p.kind() != NodeKind::Schema && node.kind() != NodeKind::Table)
                .map(|p| p.name().to_string())
                .unwrap_or_default();
            let key = (
                schema.name().to_string(),
                node.kind(),
                node.name().to_string(),
                owner,
                node.signature().to_string(),
            );
            if map.insert(key.clone(), node.id()).is_some() {
                twice.push(key);
            }
        }
    }
    for key in twice {
        map.remove(&key);
    }
    map
}

fn each_case(mut check: impl FnMut(u64, &mut Rng, Level)) {
    for seed in 0..CASES {
        let mut rng = Rng::new(seed);
        let level = if seed % 4 == 3 {
            Level::Names
        } else {
            Level::Structure
        };
        check(seed, &mut rng, level);
    }
}

#[test]
fn the_model_does_not_depend_on_input_order() {
    each_case(|seed, rng, level| {
        let snapshot = random_snapshot(rng, level);
        let a = Model::from_snapshot(ds(), snapshot.clone());
        let b = Model::from_snapshot(ds(), shuffled(rng, &snapshot));
        assert_eq!(a, b, "seed {seed}");
    });
}

#[test]
fn a_model_survives_its_own_round_trip() {
    each_case(|seed, rng, level| {
        let model = Model::from_snapshot(ds(), random_snapshot(rng, level));
        let again = Model::from_snapshot(ds(), model.to_snapshot());
        assert_eq!(model, again, "seed {seed}");
        let mut ids: Vec<NodeId> = model
            .schemas()
            .flat_map(|s| s.nodes().map(|n| n.id()))
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count, "unique ids, seed {seed}");
    });
}

#[test]
fn a_delta_gives_the_model_of_the_new_snapshot() {
    let (mut changed, mut dropping) = (0, 0);
    each_case(|seed, rng, level| {
        let old = random_snapshot(rng, level);
        let new = if rng.chance(50) {
            mutate(rng, &old)
        } else {
            random_snapshot(rng, level)
        };
        let base = Model::from_snapshot(ds(), old.clone());
        let full = Model::from_snapshot(ds(), new.clone());
        let d = delta(&old, &new);

        let merged = base.with_delta(d.clone());
        assert_eq!(merged, full, "delta, seed {seed}");
        changed += usize::from(merged != base);
        dropping += usize::from(d.schemas.iter().any(|s| !s.dropped.is_empty()));

        let twice = merged.with_delta(d.clone());
        assert_eq!(twice, full, "idempotent, seed {seed}");
        for s in merged.schemas() {
            assert!(
                Arc::ptr_eq(
                    merged.schema_arc(s.name()).unwrap(),
                    twice.schema_arc(s.name()).unwrap()
                ),
                "a delta merged again shares every schema, seed {seed}"
            );
        }

        // A delta, then the full snapshot: the full snapshot.
        assert_eq!(
            merged.with_snapshot(new.clone()),
            full,
            "then full, seed {seed}"
        );
        assert_eq!(base.with_delta(d).with_snapshot(new), full, "seed {seed}");

        // Schemas the change did not touch are shared with the old model.
        for s in base.schemas() {
            if let Some(n) = full.schema_arc(s.name())
                && **n == *s
            {
                assert!(
                    Arc::ptr_eq(
                        base.schema_arc(s.name()).unwrap(),
                        merged.schema_arc(s.name()).unwrap()
                    ),
                    "unchanged schema {} shared, seed {seed}",
                    s.name()
                );
            }
        }
    });
    // The generator does produce changes and drops.
    assert!(changed > CASES as usize * 3 / 4, "{changed} changed");
    assert!(dropping > CASES as usize / 4, "{dropping} with drops");
}

#[test]
fn ids_survive_a_refresh() {
    each_case(|seed, rng, level| {
        let old = random_snapshot(rng, level);
        let new = mutate(rng, &old);
        let before = keys(&Model::from_snapshot(ds(), old));
        let after = keys(&Model::from_snapshot(ds(), new));
        let mut common = 0;
        for (key, id) in &after {
            if let Some(old_id) = before.get(key) {
                assert_eq!(id, old_id, "{key:?}, seed {seed}");
                common += 1;
            }
        }
        assert!(common > 0, "seed {seed}");
    });
}
