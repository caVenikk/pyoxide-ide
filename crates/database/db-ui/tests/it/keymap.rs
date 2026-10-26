//! The bindings of `keymaps/*/db.toml`, pressed in the Database tool
//! window on the workbench's keymap harness (ADR 0022).

use std::path::Path;

use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::keymap::Binding;
use workbench::test_support::keymap::{Harness, check_area, check_joined_once};

use super::*;

fn check(h: &mut Harness, b: &Binding) -> bool {
    match b.command.as_str() {
        "db.explorer.new-data-source" => {
            show_tool_window(h);
            h.press(&b.keys);
            let s = state(h);
            assert!(!s["explorer"]["menu"].is_null(), "the New menu: {s}");
            true
        }
        "db.explorer.properties" => {
            show_tool_window(h);
            h.press(&b.keys);
            let s = state(h);
            assert_eq!(s["dialog"]["selected"]["name"], "local", "{s}");
            h.invoke("db.data-sources.cancel", Value::Null);
            let s = state(h);
            assert!(s["dialog"].is_null(), "{s}");
            true
        }
        "db.explorer.up" => {
            show_tool_window(h);
            h.invoke("db.explorer.select", json!({ "name": "other" }));
            h.press(&b.keys);
            assert_eq!(tree_selected(h).as_deref(), Some("local"));
            true
        }
        "db.explorer.down" => {
            select_local(h);
            h.press(&b.keys);
            assert_eq!(tree_selected(h).as_deref(), Some("other"));
            true
        }
        "db.explorer.expand-or-next" | "db.explorer.toggle" => {
            show_tool_window(h);
            h.invoke("db.explorer.select", json!({ "name": "local" }));
            h.press(&b.keys);
            assert_eq!(tree_row(h, "local")["expanded"], true);
            true
        }
        "db.explorer.collapse-or-parent" => {
            show_tool_window(h);
            h.invoke("db.explorer.expand", json!({ "path": "local" }));
            h.press(&b.keys);
            assert_eq!(tree_row(h, "local")["expanded"], false);
            true
        }
        // The context menus' actions, on the selected data source.
        "db.explorer.context-menu" => {
            select_local(h);
            h.press(&b.keys);
            let s = state(h);
            assert!(!s["explorer"]["menu"].is_null(), "the context menu: {s}");
            h.invoke("menu.cancel", Value::Null);
            true
        }
        "db.explorer.refresh" | "db.explorer.force-refresh" => {
            select_local(h);
            h.press(&b.keys);
            let s = state(h);
            assert!(
                !s["explorer"]["tree"]["data_sources"]["local"].is_null(),
                "the data source is open: {s}"
            );
            true
        }
        "db.explorer.deactivate" => {
            select_local(h);
            h.press(&b.keys);
            true
        }
        "db.explorer.rename" => {
            select_local(h);
            h.press(&b.keys);
            let s = state(h);
            assert_eq!(s["sql_dialog"]["title"], "Rename", "{s}");
            assert_eq!(s["sql_dialog"]["name"], "local", "{s}");
            invoke_in_modal(h, "db.sql-dialog.cancel", Value::Null);
            assert!(state(h)["sql_dialog"].is_null());
            true
        }
        "db.explorer.move-down" | "db.explorer.move-up" => {
            select_local(h);
            let down = b.command.ends_with("down");
            if !down {
                h.invoke("db.explorer.move-down", Value::Null);
            }
            h.press(&b.keys);
            let names = source_names(h);
            if down {
                assert_eq!(names, ["other", "local"]);
                h.invoke("db.explorer.move-up", Value::Null);
            } else {
                assert_eq!(names, ["local", "other"]);
            }
            true
        }
        "db.explorer.quick-doc" => {
            select_local(h);
            h.press(&b.keys);
            let s = state(h);
            assert_eq!(
                s["explorer"]["actions"]["doc"]["title"], "data source local",
                "{s}"
            );
            // Another selection closes the popup.
            h.invoke("db.explorer.select", json!({ "name": "other" }));
            assert!(state(h)["explorer"]["actions"]["doc"].is_null());
            true
        }
        "db.explorer.copy-reference" => {
            select_local(h);
            h.press(&b.keys);
            let text = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert_eq!(text.as_deref(), Some("local"));
            true
        }
        "db.explorer.generate-ddl" => {
            show_tool_window(h);
            h.invoke("db.explorer.expand", json!({ "path": "local" }));
            h.invoke("db.explorer.select", json!({ "name": "local" }));
            h.press(&b.keys);
            let text = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert!(
                text.as_deref()
                    .is_some_and(|t| t.contains("create table demo.customers")),
                "{text:?}"
            );
            true
        }
        "db.explorer.duplicate" => {
            select_local(h);
            h.press(&b.keys);
            assert_eq!(source_names(h), ["local", "local (2)", "other"]);
            true
        }
        "db.explorer.delete" => {
            select_local(h);
            h.press(&b.keys);
            let s = state(h);
            assert_eq!(
                s["explorer"]["actions"]["confirm"]["title"], "Remove Data Source",
                "{s}"
            );
            invoke_in_modal(h, "dialog.cancel", Value::Null);
            assert!(state(h)["explorer"]["actions"]["confirm"].is_null());
            true
        }
        _ => false,
    }
}

fn select_local(h: &mut Harness) {
    show_tool_window(h);
    h.invoke("db.explorer.select", json!({ "name": "local" }));
}

fn source_names(h: &mut Harness) -> Vec<String> {
    state(h)["explorer"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect()
}

fn tree_row(h: &mut Harness, path: &str) -> Value {
    let s = state(h);
    s["explorer"]["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == path)
        .cloned()
        .unwrap_or_else(|| panic!("no row {path}: {s}"))
}

fn tree_selected(h: &mut Harness) -> Option<String> {
    state(h)["explorer"]["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["selected"] == true)
        .map(|r| r["path"].as_str().unwrap().to_owned())
}

fn run(cx: &mut TestAppContext, macos: bool) {
    let (_dir, root) = project();
    let store = FakeStore::new("pw");
    store.add(&root, "local", pg());
    store.add(&root, "other", pg());
    // A cached model for Generate DDL to Clipboard.
    let cache_dir = tempfile::tempdir().unwrap();
    db_model::ModelCache::new(cache_dir.path())
        .store(&db_model::Model::from_snapshot(
            db_core::DataSourceId::from("local"),
            db_ui::fixture::demo_snapshot(),
        ))
        .unwrap();
    store.set_model_cache(cache_dir.path());
    let area = if macos {
        include_str!("../../keymaps/macos/db.toml")
    } else {
        include_str!("../../keymaps/default/db.toml")
    };
    check_area(cx, product(store), macos, area, Some(&root), check);
}

#[gpui::test]
fn every_macos_binding_works(cx: &mut TestAppContext) {
    run(cx, true);
}

#[gpui::test]
fn every_windows_and_linux_binding_works(cx: &mut TestAppContext) {
    run(cx, false);
}

#[test]
fn the_keymap_folders_are_joined_once() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("keymaps");
    check_joined_once(&folder.join("macos"), db_ui::KEYMAPS.macos).unwrap();
    check_joined_once(&folder.join("default"), db_ui::KEYMAPS.default).unwrap();
}
