//! Source texts: lazy, beside the cache, replaced by passes and deltas.
use std::fs;

use db_core::DataSourceId;
use db_core::schema::{Delta, Level, ObjectKind, ObjectRef, SchemaDelta, SourceInfo};
use db_model::{Model, ModelCache, NameMatch, SourceKey, SourceStore};

use crate::fixture::{object, schema_info, shop, token};

fn source(name: &str, kind: ObjectKind, signature: &str, text: &str) -> SourceInfo {
    SourceInfo {
        name: name.into(),
        kind,
        signature: signature.into(),
        text: text.into(),
    }
}

fn full_shop() -> db_core::schema::Snapshot {
    let mut snapshot = shop();
    let public = &mut snapshot.schemas[0];
    public.level = Level::Full;
    public.sources = vec![
        source("order_totals", ObjectKind::View, "", "select 1"),
        source("total", ObjectKind::Function, "integer", "body one"),
        source("total", ObjectKind::Function, "integer, text", "body two"),
        source("orders_audit", ObjectKind::Trigger, "", "trigger body"),
    ];
    snapshot
}

#[test]
fn texts_of_a_pass_are_found_by_node_and_read_back_lazily() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ModelCache::new(dir.path());
    let ds = DataSourceId::new("shop");
    let snapshot = full_shop();
    let store = cache.sources(&ds);
    store.absorb_snapshot(&snapshot).unwrap();
    let model = Model::from_snapshot(ds.clone(), snapshot);

    // A fresh store (a restart) reads files only when asked.
    let store = cache.sources(&ds);
    let view = model
        .find("public", "order_totals", NameMatch::Exact, &[])
        .unwrap();
    let key = SourceKey::of(view).unwrap();
    assert_eq!(&*store.get(&key).unwrap().unwrap(), "select 1");
    let overloads: Vec<String> = model
        .lookup("public", "total", NameMatch::Exact, &[])
        .map(|n| {
            store
                .get(&SourceKey::of(n).unwrap())
                .unwrap()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(overloads, ["body one", "body two"]);
    // A trigger hangs under its table and is still found by its key.
    let trigger = model
        .find("public", "orders_audit", NameMatch::Exact, &[])
        .unwrap();
    assert_eq!(
        &*store
            .get(&SourceKey::of(trigger).unwrap())
            .unwrap()
            .unwrap(),
        "trigger body"
    );
    // A table of PostgreSQL has no text; a column has no key.
    let orders = model
        .find("public", "orders", NameMatch::Exact, &[])
        .unwrap();
    assert!(
        store
            .get(&SourceKey::of(orders).unwrap())
            .unwrap()
            .is_none()
    );
    assert!(SourceKey::of(orders.children().next().unwrap()).is_none());
}

#[test]
fn a_full_pass_replaces_the_texts_of_its_schemas() {
    let store = SourceStore::in_memory();
    store.absorb_snapshot(&full_shop()).unwrap();
    let mut again = full_shop();
    again.schemas[0].sources = vec![source("order_totals", ObjectKind::View, "", "select 2")];
    store.absorb_snapshot(&again).unwrap();
    let view = SourceKey::new("public", ObjectKind::View, "order_totals", "");
    assert_eq!(&*store.get(&view).unwrap().unwrap(), "select 2");
    let gone = SourceKey::new("public", ObjectKind::Function, "total", "integer");
    assert!(store.get(&gone).unwrap().is_none());
    // A pass below the full level keeps what was read.
    store.absorb_snapshot(&shop()).unwrap();
    assert!(store.get(&view).unwrap().is_some());
}

#[test]
fn a_delta_drops_changed_and_dropped_texts() {
    let dir = tempfile::tempdir().unwrap();
    let store = SourceStore::on_disk(dir.path().join("sources"));
    store.absorb_snapshot(&full_shop()).unwrap();
    let delta = Delta {
        token: token("5"),
        schemas: vec![SchemaDelta {
            info: schema_info("public"),
            objects: vec![object("public", "order_totals", ObjectKind::View)],
            relations: vec![],
            sources: vec![source("new_view", ObjectKind::View, "", "select 3")],
            dropped: vec![ObjectRef {
                schema: "public".into(),
                name: "total".into(),
                kind: ObjectKind::Function,
            }],
        }],
        dropped_schemas: vec![],
    };
    store.absorb_delta(&delta).unwrap();
    let reread = SourceStore::on_disk(dir.path().join("sources"));
    for s in [&store, &reread] {
        let get = |kind, name: &str, sig: &str| {
            s.get(&SourceKey::new("public", kind, name, sig)).unwrap()
        };
        assert!(get(ObjectKind::View, "order_totals", "").is_none());
        assert!(get(ObjectKind::Function, "total", "integer").is_none());
        assert!(get(ObjectKind::Function, "total", "integer, text").is_none());
        assert_eq!(&*get(ObjectKind::View, "new_view", "").unwrap(), "select 3");
        assert!(get(ObjectKind::Trigger, "orders_audit", "").is_some());
    }
    let dropped = Delta {
        token: token("6"),
        schemas: vec![],
        dropped_schemas: vec!["public".into()],
    };
    store.absorb_delta(&dropped).unwrap();
    assert_eq!(fs::read_dir(dir.path().join("sources")).unwrap().count(), 0);
}

#[test]
fn a_damaged_text_file_is_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let store = SourceStore::on_disk(dir.path());
    let key = SourceKey::new("s", ObjectKind::View, "v", "");
    store.put(key.clone(), "select 1").unwrap();
    for entry in fs::read_dir(dir.path()).unwrap() {
        fs::write(entry.unwrap().path(), b"{oops").unwrap();
    }
    assert!(
        SourceStore::on_disk(dir.path())
            .get(&key)
            .unwrap()
            .is_none()
    );
    store.remove(&key).unwrap();
    assert!(store.get(&key).unwrap().is_none());
}
