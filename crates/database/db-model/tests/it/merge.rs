//! Snapshots and deltas merged into a model, removal by live list, cancel,
//! and the handle readers and refreshes share.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use db_core::DataSourceId;
use db_core::schema::{Delta, Level, ObjectKind, ObjectRef, SchemaDelta, Snapshot};
use db_model::{Cancelled, Group, Model, ModelHandle, NameMatch, NodeKind};

use crate::fixture::{column, object, relation, schema, schema_info, shop, synthetic, token};

fn ds() -> DataSourceId {
    DataSourceId::new("shop")
}

fn table_names(model: &Model, schema: &str) -> Vec<String> {
    model
        .schema(schema, NameMatch::Exact)
        .unwrap()
        .group(Group::Tables)
        .nodes()
        .map(|n| n.name().to_string())
        .collect()
}

fn add_column_delta() -> Delta {
    Delta {
        token: token("2"),
        schemas: vec![SchemaDelta {
            info: schema_info("public"),
            objects: vec![object("public", "invoices", ObjectKind::Table)],
            relations: vec![
                relation("invoices", vec![column("id", 1, "integer")], vec![], vec![]),
                relation(
                    "customers",
                    vec![
                        column("id", 1, "integer"),
                        column("name", 2, "text"),
                        column("email", 3, "text"),
                    ],
                    vec![],
                    vec![],
                ),
            ],
            sources: vec![],
            dropped: vec![ObjectRef {
                schema: "public".into(),
                name: "orders".into(),
                kind: ObjectKind::Table,
            }],
        }],
        dropped_schemas: vec![],
    }
}

#[test]
fn a_delta_adds_changes_and_drops() {
    let before = Model::from_snapshot(ds(), shop());
    let after = before.with_delta(add_column_delta());
    assert_eq!(after.token(), Some(&token("2")));
    assert_eq!(
        table_names(&after, "public"),
        ["customers", "events", "invoices"]
    );
    // Dropping a table drops its indexes and triggers.
    assert!(
        after
            .find("public", "orders_customer_idx", NameMatch::Exact, &[])
            .is_none()
    );
    assert!(
        after
            .find("public", "orders_audit", NameMatch::Exact, &[])
            .is_none()
    );
    let customers = after
        .find("public", "customers", NameMatch::Exact, &[])
        .unwrap();
    let columns: Vec<_> = customers
        .group(Group::Columns)
        .nodes()
        .map(|n| n.name())
        .collect();
    assert_eq!(columns, ["id", "name", "email"]);
    // Kept nodes keep their ids.
    let id_before = before
        .find("public", "customers", NameMatch::Exact, &[])
        .unwrap()
        .id();
    assert_eq!(customers.id(), id_before);
    // The schema the delta does not name is shared, not copied.
    assert!(Arc::ptr_eq(
        before.schema_arc("app").unwrap(),
        after.schema_arc("app").unwrap()
    ));
    assert!(!Arc::ptr_eq(
        before.schema_arc("public").unwrap(),
        after.schema_arc("public").unwrap()
    ));
    // The model it came from is untouched.
    assert!(
        before
            .find("public", "orders", NameMatch::Exact, &[])
            .is_some()
    );
}

#[test]
fn a_delta_merged_twice_changes_nothing_more() {
    let model = Model::from_snapshot(ds(), shop());
    let once = model.with_delta(add_column_delta());
    let twice = once.with_delta(add_column_delta());
    assert_eq!(once, twice);
    assert!(Arc::ptr_eq(
        once.schema_arc("public").unwrap(),
        twice.schema_arc("public").unwrap()
    ));
}

#[test]
fn a_delta_drops_and_creates_schemas() {
    let model = Model::from_snapshot(ds(), shop());
    let delta = Delta {
        token: token("3"),
        schemas: vec![SchemaDelta {
            info: schema_info("new"),
            objects: vec![object("new", "t", ObjectKind::Table)],
            relations: vec![relation("t", vec![column("a", 1, "int")], vec![], vec![])],
            sources: vec![],
            dropped: vec![],
        }],
        dropped_schemas: vec!["app".into()],
    };
    let after = model.with_delta(delta);
    let names: Vec<_> = after.schemas().map(|s| s.name()).collect();
    assert_eq!(names, ["new", "public"]);
    assert_eq!(
        after.schema("new", NameMatch::Exact).unwrap().level(),
        Level::Structure
    );
}

#[test]
fn a_snapshot_replaces_its_schemas_and_keeps_the_others() {
    let model = Model::from_snapshot(ds(), shop());
    let app_again = Snapshot {
        level: Level::Names,
        token: Some(token("9")),
        schemas: vec![schema(
            "app",
            Level::Names,
            vec![object("app", "only", ObjectKind::Table)],
            vec![],
        )],
    };
    let after = model.with_snapshot(app_again);
    assert_eq!(table_names(&after, "app"), ["only"]);
    assert!(Arc::ptr_eq(
        model.schema_arc("public").unwrap(),
        after.schema_arc("public").unwrap()
    ));
    // An unchanged schema keeps its Arc through a full reload.
    let reloaded = model.with_snapshot(shop());
    for s in ["app", "public"] {
        assert!(Arc::ptr_eq(
            model.schema_arc(s).unwrap(),
            reloaded.schema_arc(s).unwrap()
        ));
    }
    assert_eq!(
        model
            .without_schemas(&["app"])
            .schemas()
            .map(|s| s.name())
            .collect::<Vec<_>>(),
        ["public"]
    );
}

#[test]
fn objects_missing_from_the_live_list_are_removed() {
    let model = Model::from_snapshot(ds(), shop());
    let live: Vec<ObjectRef> = ["customers", "events", "events_eu", "events_us"]
        .iter()
        .map(|n| ObjectRef {
            schema: "public".into(),
            name: n.to_string(),
            kind: ObjectKind::Table,
        })
        .collect();
    let after = model.retain_live("public", &[ObjectKind::Table], &live);
    assert_eq!(table_names(&after, "public"), ["customers", "events"]);
    assert!(
        after
            .find("public", "orders_customer_idx", NameMatch::Exact, &[])
            .is_none()
    );
    // Other kinds are not judged by a table list.
    assert!(
        after
            .find("public", "order_totals", NameMatch::Exact, &[])
            .is_some()
    );
    assert!(
        after
            .find("public", "total", NameMatch::Exact, &[])
            .is_some()
    );
    // An unknown schema changes nothing.
    assert_eq!(model.retain_live("nope", &[], &[]), model);
}

#[test]
fn a_cancelled_refresh_leaves_the_current_model() {
    let handle = ModelHandle::new(Model::from_snapshot(ds(), shop()));
    let before = handle.get();
    let generation = handle.generation();
    let asked = AtomicUsize::new(0);
    // Cancel after the build started: some objects are already built.
    let cancel = || asked.fetch_add(1, Ordering::Relaxed) >= 3;
    let result = handle.update(|current| {
        current
            .with_snapshot_cancellable(synthetic(5_000), &cancel)
            .map(Some)
    });
    assert_eq!(result.unwrap_err(), Cancelled);
    assert!(asked.load(Ordering::Relaxed) > 3, "the build was under way");
    assert!(Arc::ptr_eq(&before, &handle.get()));
    assert_eq!(handle.generation(), generation);
    assert!(
        handle
            .get()
            .find("app", "table_00001", NameMatch::Exact, &[])
            .is_none()
    );

    // The same refresh, not cancelled, is swapped in whole.
    let after = handle
        .update(|current| {
            current
                .with_snapshot_cancellable(synthetic(5_000), &|| false)
                .map(Some)
        })
        .unwrap();
    assert!(Arc::ptr_eq(&after, &handle.get()));
    assert_eq!(handle.generation(), generation + 1);
    assert!(
        after
            .find("app", "table_04999", NameMatch::Exact, &[])
            .is_some()
    );
    // The reader that held the old model still sees it whole.
    assert!(
        before
            .find("public", "orders", NameMatch::Exact, &[])
            .is_some()
    );
    assert!(
        before
            .find("app", "table_04999", NameMatch::Exact, &[])
            .is_none()
    );
}

#[test]
fn readers_see_whole_models_while_refreshes_swap() {
    let handle = Arc::new(ModelHandle::new(Model::from_snapshot(ds(), shop())));
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(3));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let (handle, stop, barrier) = (handle.clone(), stop.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                let mut reads = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let model = handle.get();
                    // Either version, never a mix: orders and its index go
                    // together.
                    let orders = model.find("public", "orders", NameMatch::Exact, &[]);
                    let idx = model.find("public", "orders_customer_idx", NameMatch::Exact, &[]);
                    assert_eq!(orders.is_some(), idx.is_some());
                    reads += 1;
                }
                reads
            })
        })
        .collect();
    barrier.wait();
    for i in 0..50 {
        handle
            .update(|m| -> Result<_, Cancelled> {
                Ok(Some(if i % 2 == 0 {
                    m.with_delta(add_column_delta())
                } else {
                    m.with_snapshot(shop())
                }))
            })
            .unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        assert!(r.join().unwrap() > 0);
    }
    assert_eq!(handle.generation(), 50);
    assert!(
        handle
            .get()
            .find("public", "orders", NameMatch::Exact, &NodeKind::RELATIONS)
            .is_some()
    );
}
