//! The object tree: a data source opens on its cached model without asking
//! the database, nodes open from the model, a data source without a model
//! that cannot connect shows the error with Retry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::DataSourceId;
use db_model::{Model, ModelCache};
use db_ui::fixture::demo_snapshot;
use db_ui::store::fake::{FakeStore, NoServer};
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::*;

const NAME: &str = "pyoxide-local";

fn open<'a>(cx: &'a mut TestAppContext, store: Arc<FakeStore>, root: &Path) -> Harness<'a> {
    store.add(root, NAME, pg());
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(root));
    show_tool_window(&mut h);
    h
}

/// The tree's rows as `path [detail]` lines.
fn rows(h: &mut Harness) -> Vec<String> {
    state(h)["explorer"]["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let path = r["path"].as_str().unwrap().to_owned();
            match (r["detail"].as_str(), r["count"].as_u64()) {
                (Some(d), _) => format!("{path} [{d}]"),
                (_, Some(c)) => format!("{path} {c}"),
                _ => path,
            }
        })
        .collect()
}

/// Draws until `f` holds (the data source handles answer from threads of
/// their own).
fn wait_for(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = state(h);
        if f(&s) {
            return s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}: {s}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn a_cached_model_opens_without_the_database(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = ModelCache::new(cache_dir.path());
    cache
        .store(&Model::from_snapshot(
            DataSourceId::from(NAME),
            demo_snapshot(),
        ))
        .unwrap();
    let store = FakeStore::new("secret");
    store.set_model_cache(cache_dir.path());
    let server = Arc::new(NoServer::default());
    store.set_connections(server.clone());
    let mut h = open(cx, store, &root);
    assert_eq!(rows(&mut h), [NAME]);

    h.invoke("db.explorer.expand", json!({ "path": NAME }));
    wait_for(&mut h, "the cached model", |s| {
        s["explorer"]["tree"]["data_sources"][NAME]["has_model"] == true
    });
    h.invoke(
        "db.explorer.expand",
        json!({ "path": format!("{NAME}/pyoxide/demo/tables/customers/columns") }),
    );
    h.invoke(
        "db.explorer.expand",
        json!({ "path": format!("{NAME}/pyoxide/demo/tables/customers/indexes") }),
    );
    let got = rows(&mut h);
    let p = |s: &str| format!("{NAME}/pyoxide/demo/{s}");
    let want = [
        NAME.to_owned(),
        format!("{NAME}/pyoxide"),
        p("").trim_end_matches('/').to_owned(),
        p("tables 6"),
        p("tables/audit_log"),
        p("tables/customers"),
        p("tables/customers/columns 8"),
        p("tables/customers/columns/id [integer (auto increment)]"),
        p("tables/customers/columns/email [demo.email_t]"),
        p("tables/customers/columns/full_name [text]"),
        p("tables/customers/columns/country [char(2) = 'DE'::bpchar]"),
        p("tables/customers/columns/is_active [boolean = true]"),
        p("tables/customers/columns/profile [jsonb]"),
        p("tables/customers/columns/home [demo.address]"),
        p("tables/customers/columns/notes [text]"),
        p("tables/customers/keys 1"),
        p("tables/customers/indexes 3"),
        // The model keeps objects by name.
        p("tables/customers/indexes/customers_country_idx [(country) where is_active]"),
        p("tables/customers/indexes/customers_email_lower_idx [(lower(email::text)) UNIQUE]"),
        p("tables/customers/indexes/customers_pkey [(id) UNIQUE]"),
        p("tables/customers/triggers 1"),
        p("tables/measurements"),
        p("tables/orders"),
        p("materialized views 1"),
        p("views 1"),
        p("routines 3"),
        p("sequences 1"),
        p("object types 1"),
        format!("{NAME}/pyoxide/public"),
    ];
    assert_eq!(got, want);
    // Opening nodes asked the database nothing.
    assert_eq!(server.opened(), 0);

    // Closing a node hides its rows; the selection stays on it.
    h.invoke(
        "db.explorer.collapse",
        json!({ "path": format!("{NAME}/pyoxide/demo/tables") }),
    );
    assert_eq!(rows(&mut h).len(), 10);
    assert_eq!(server.opened(), 0);
}

#[gpui::test]
fn without_a_model_a_failed_connection_shows_the_error_and_retry(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let cache_dir = tempfile::tempdir().unwrap();
    let store = FakeStore::new("secret");
    store.set_model_cache(cache_dir.path());
    let server = Arc::new(NoServer::default());
    store.set_connections(server.clone());
    let mut h = open(cx, store, &root);
    h.invoke("db.explorer.expand", json!({ "path": NAME }));
    let s = wait_for(&mut h, "the failed refresh", |s| {
        s["explorer"]["tree"]["rows"][1]["failed"] == true
    });
    let status = &s["explorer"]["tree"]["rows"][1];
    assert_eq!(status["text"], NoServer::MESSAGE, "{s}");
    let opened = server.opened();
    assert!(opened >= 1);

    // Refresh tries again.
    h.invoke("db.explorer.select", json!({ "name": NAME }));
    h.invoke("db.explorer.refresh", Value::Null);
    wait_for(&mut h, "the second attempt", |_| server.opened() > opened);
    wait_for(&mut h, "the second failure", |s| {
        s["explorer"]["tree"]["data_sources"][NAME]["refreshing"] == false
    });
}
