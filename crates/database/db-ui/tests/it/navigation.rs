//! Go to Declaration from SQL text: an alias moves the caret in the file, a
//! table or a column selects the object in the Database tool window (the
//! `db` scheme of object targets).

use std::time::{Duration, Instant};

use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::services::{SOURCE, attach, cached_store, open};
use super::*;

/// The path of the selected row of the Database tool window's tree.
fn selected_row(h: &mut Harness) -> Option<String> {
    state(h)["explorer"]["tree"]["rows"]
        .as_array()?
        .iter()
        .find(|r| r["selected"] == true)
        .and_then(|r| r["path"].as_str().map(str::to_owned))
}

/// Invokes Go to Declaration until the tree selects a row ending with `end`
/// (the cached model is read on a thread of its own, so the first answers
/// may find nothing).
fn go_to_declaration_selects(h: &mut Harness, end: &str) -> String {
    let started = Instant::now();
    loop {
        h.invoke("code.go-to-declaration", Value::Null);
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            h.draw();
            if let Some(row) = selected_row(h).filter(|r| r.ends_with(end)) {
                return row;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "selected {:?}, wanted {end}",
            selected_row(h)
        );
    }
}

#[gpui::test]
fn a_table_is_selected_in_the_database_tool_window(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (store, _cache) = cached_store();
    let mut h = open(cx, store, &root, "q.sql", "");
    attach(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers" }),
    );
    let row = go_to_declaration_selects(&mut h, "/tables/customers");
    assert_eq!(row, format!("{SOURCE}/pyoxide/demo/tables/customers"));
    // The tool window came up with the object in it.
    assert_eq!(state(&mut h)["explorer"]["selected"], SOURCE);
}

#[gpui::test]
fn a_column_is_selected_under_its_table(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (store, _cache) = cached_store();
    let mut h = open(cx, store, &root, "q.sql", "");
    attach(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers c where c.full_name" }),
    );
    let row = go_to_declaration_selects(&mut h, "/columns/full_name");
    assert_eq!(
        row,
        format!("{SOURCE}/pyoxide/demo/tables/customers/columns/full_name")
    );
}

#[gpui::test]
fn an_alias_moves_the_caret_in_the_file(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (store, _cache) = cached_store();
    let mut h = open(cx, store, &root, "q.sql", "");
    attach(&mut h);
    let text = "select c.id from demo.customers c where c";
    h.invoke("editor.type", json!({ "text": text }));
    let alias_column = text.find(" c where").unwrap() + 1;
    let started = Instant::now();
    loop {
        h.invoke("code.go-to-declaration", Value::Null);
        std::thread::sleep(Duration::from_millis(20));
        h.draw();
        let caret = h.invoke("workbench.state", Value::Null)["editor"]["caret"].clone();
        if caret["column"] == alias_column {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "caret {caret}");
    }
}
