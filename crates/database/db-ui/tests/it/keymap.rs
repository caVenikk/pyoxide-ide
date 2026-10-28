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
        "db.explorer.up" | "db.explorer.down" => {
            show_tool_window(h);
            h.press(&b.keys);
            assert_eq!(tree_selected(h).as_deref(), Some("local"));
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
        _ => false,
    }
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
