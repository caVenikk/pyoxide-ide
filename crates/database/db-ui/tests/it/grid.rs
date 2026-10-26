//! The data grid of a table over a scripted connection source: it opens
//! from the tree's model, reads its first page with the reference's
//! statement, pages, counts only on request, sorts on the server, keeps the
//! rows when the server rejects a condition, and Stop ends a hanging read.
//! The bindings of `keymaps/*/grid.toml` are pressed in it as well.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{Backend, DataSourceId, Purpose, Value as DbValue};
use db_model::{Model, ModelCache};
use db_session::fake::{FakeSource, Reaction};
use db_ui::fixture::demo_snapshot;
use db_ui::store::fake::FakeStore;
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::keymap::Binding;
use workbench::test_support::keymap::{Harness, check_area};

use super::*;

const NAME: &str = "pyoxide-local";

fn rows(values: &[(&str, &str)]) -> Reaction {
    Reaction::Rows {
        columns: vec!["id".into(), "email".into()],
        rows: values
            .iter()
            .map(|(a, b)| vec![DbValue::from(*a), DbValue::from(*b)])
            .collect(),
    }
}

/// A server with `demo.customers`: three rows; the page from offset 2
/// holds one; a count of 3; an unknown column; a condition that hangs.
fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    source.on(
        "nonexistent",
        Reaction::query_error("42703", "column \"nonexistent\" does not exist"),
    );
    source.on("hang_me", Reaction::Hang);
    source.on("count(*)", Reaction::column("count", &["3"]));
    source.on("offset 2", rows(&[("3", "c@example.test")]));
    source.on(
        "from demo.customers t",
        rows(&[
            ("1", "a@example.test"),
            ("2", "b@example.test"),
            ("3", "c@example.test"),
        ]),
    );
    source
}

fn store_with_model(root: &Path, server: &FakeSource) -> (Arc<FakeStore>, tempfile::TempDir) {
    let cache_dir = tempfile::tempdir().unwrap();
    ModelCache::new(cache_dir.path())
        .store(&Model::from_snapshot(
            DataSourceId::from(NAME),
            demo_snapshot(),
        ))
        .unwrap();
    let store = FakeStore::new("secret");
    store.set_model_cache(cache_dir.path());
    store.set_connections(Arc::new(server.clone()));
    store.add(root, NAME, pg());
    (store, cache_dir)
}

/// Draws until `f` holds for `db.grid.state` (the pager answers from
/// threads of its own).
fn wait_grid(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = grid_state(h);
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

fn grid_state(h: &mut Harness) -> Value {
    use gpui::AppContext as _;
    let window: gpui::AnyWindowHandle = h.window.into();
    h.cx.update_window(window, |_, window, cx| {
        ui::command::invoke("db.grid.state", Value::Null, window, cx)
    })
    .unwrap()
    .unwrap_or(Value::Null)
}

/// Shows the tree with the cached model and opens `customers`.
fn open_customers(h: &mut Harness) -> Value {
    show_tool_window(h);
    h.invoke("db.explorer.expand", json!({ "path": NAME }));
    let started = Instant::now();
    while state(h)["explorer"]["tree"]["data_sources"][NAME]["has_model"] != true {
        assert!(started.elapsed() < Duration::from_secs(10), "the model");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
    h.invoke(
        "db.grid.open",
        json!({"source": NAME, "schema": "demo", "table": "customers"}),
    );
    wait_grid(h, "the first page", |s| {
        s["rows"] == 3 && s["loading"] == false
    })
}

#[gpui::test]
fn the_grid_pages_counts_sorts_and_keeps_rows_on_errors(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    let s = open_customers(&mut h);
    assert_eq!(s["range"], "3 rows", "{s}");
    assert_eq!(s["total_text"], Value::Null);
    assert_eq!(s["columns"][0]["name"], "id");
    // The tab shows the table's name.
    let tabs = h.state()["tabs"].to_string();
    assert!(tabs.contains("customers"), "{tabs}");
    let data = server.statements(Purpose::Data);
    assert!(
        data.contains(&"SELECT t.*\nFROM demo.customers t\nLIMIT 501".to_owned()),
        "{data:?}"
    );
    assert!(
        !data.iter().any(|q| q.contains("COUNT")),
        "no count before it is asked for: {data:?}"
    );

    // A page of two: there is more, the total is open.
    h.invoke("db.grid.set-page-size", json!({"size": 2}));
    let s = wait_grid(&mut h, "pages of two", |s| {
        s["rows"] == 2 && s["loading"] == false
    });
    assert_eq!(s["range"], "1-2");
    assert_eq!(s["total_text"], "of 3+");
    h.invoke("db.grid.next-page", Value::Null);
    let s = wait_grid(&mut h, "the second page", |s| {
        s["first_row"] == 2 && s["loading"] == false
    });
    assert_eq!(s["range"], "3-3");
    assert_eq!(s["has_more"], false);
    assert!(
        server
            .statements(Purpose::Data)
            .contains(&"SELECT t.*\nFROM demo.customers t\nLIMIT 3\nOFFSET 2".to_owned())
    );
    h.invoke("db.grid.first-page", Value::Null);
    wait_grid(&mut h, "the first page again", |s| {
        s["first_row"] == 0 && s["loading"] == false
    });

    // The count on request.
    h.invoke("db.grid.count", Value::Null);
    let s = wait_grid(&mut h, "the count", |s| s["total"] == 3);
    assert_eq!(s["total_text"], "of 3");
    assert!(
        server
            .statements(Purpose::Data)
            .contains(&"SELECT COUNT(*)\nFROM demo.customers t".to_owned())
    );

    // Sort by a header: ascending, then descending, on the server.
    h.invoke("db.grid.sort", json!({"column": 1}));
    let s = wait_grid(&mut h, "sorted", |s| {
        s["query"]["order"] == "email" && s["loading"] == false
    });
    assert_eq!(s["order"], "email");
    assert_eq!(s["sort"], json!([{"column": 1, "desc": false}]));
    h.invoke("db.grid.sort", json!({"column": 1}));
    let s = wait_grid(&mut h, "sorted down", |s| {
        s["query"]["order"] == "email desc" && s["loading"] == false
    });
    assert_eq!(s["sort"], json!([{"column": 1, "desc": true}]));
    assert!(
        server
            .statements(Purpose::Data)
            .iter()
            .any(|q| q.contains("ORDER BY email desc"))
    );

    // The selection's summary.
    h.invoke(
        "db.grid.select",
        json!({"row": 0, "column": 0, "to_row": 1, "to_column": 1}),
    );
    let s = grid_state(&mut h);
    assert_eq!(s["status"]["counts"], "4 cells, 2 rows", "{s}");
    assert_eq!(s["status"]["position"], "2:2");

    // A wrong condition keeps the rows and shows the server's message.
    h.invoke("db.grid.set-filter", json!({"text": "nonexistent = 1"}));
    let s = wait_grid(&mut h, "the error", |s| !s["error"].is_null());
    assert!(
        s["error"]["text"]
            .as_str()
            .unwrap()
            .starts_with("[42703] column"),
        "{s}"
    );
    assert_eq!(s["rows"], 2);
    assert_eq!(s["query"]["where"], "", "the shown rows keep their query");

    // Stop ends a read that hangs, within a second; the rows stay.
    h.invoke("db.grid.set-filter", json!({"text": "hang_me = 1"}));
    wait_grid(&mut h, "the hanging read", |s| s["loading"] == true);
    let asked = Instant::now();
    h.invoke("db.grid.stop", Value::Null);
    let s = grid_state(&mut h);
    assert_eq!(s["loading"], false);
    assert_eq!(s["rows"], 2);
    let started = Instant::now();
    while !server.log().iter().any(|e| e.sql == "<cancel>") {
        assert!(started.elapsed() < Duration::from_secs(5), "the cancel");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
}

// ---- key bindings --------------------------------------------------------------

fn check(h: &mut Harness, b: &Binding) -> bool {
    let command = b.command.as_str();
    match command {
        "db.explorer.open-table" => {
            h.invoke(
                "db.explorer.expand",
                json!({ "path": format!("{NAME}/pyoxide/demo/tables/orders") }),
            );
            show_tool_window(h);
            h.press(&b.keys);
            wait_grid(h, "the orders grid", |s| s["table"] == "orders");
            true
        }
        "db.grid.apply-fields" => {
            h.invoke("db.grid.focus-field", json!({"field": "where"}));
            h.press(&b.keys);
            true
        }
        "db.grid.reload" | "db.grid.stop" | "db.grid.dismiss" => {
            h.invoke("db.grid.focus-field", json!({"field": "cells"}));
            h.press(&b.keys);
            true
        }
        c if c.starts_with("db.grid.") => {
            h.invoke("db.grid.focus-field", json!({"field": "cells"}));
            h.invoke("db.grid.select", json!({"row": 1, "column": 1}));
            h.press(&b.keys);
            let s = grid_state(h);
            assert!(!s["selection"].is_null(), "{command}: {s}");
            true
        }
        _ => false,
    }
}

fn run(cx: &mut TestAppContext, macos: bool) {
    let (_dir, root) = project();
    let server = server();
    server.on(
        "from demo.orders t",
        rows(&[("1", "x"), ("2", "y"), ("3", "z")]),
    );
    let (store, _cache) = store_with_model(&root, &server);
    let area = if macos {
        include_str!("../../keymaps/macos/grid.toml")
    } else {
        include_str!("../../keymaps/default/grid.toml")
    };
    check_area(cx, product(store), macos, area, Some(&root), |h, b| {
        if grid_state(h).is_null() {
            open_customers(h);
        }
        check(h, b)
    });
}

#[gpui::test]
fn every_macos_grid_binding_works(cx: &mut TestAppContext) {
    run(cx, true);
}

#[gpui::test]
fn every_windows_and_linux_grid_binding_works(cx: &mut TestAppContext) {
    run(cx, false);
}
