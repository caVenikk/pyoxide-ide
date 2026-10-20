//! Quit, editor tabs, context menus and message dialogs: the bindings of `keymaps/*/window.toml`.

use super::*;

area_tests!("window");

impl H<'_> {
    /// Opens the context menu of a tree row; nothing is selected in it.
    fn open_menu(&mut self, path: &str) {
        self.select_in_tree(path);
        self.invoke("project.tree.context-menu", json!(null));
        assert_eq!(self.menu_levels().len(), 1, "the context menu is open");
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "app.quit" => {
            h.press(keys);
        }
        "dialog.confirm" | "dialog.cancel" => {
            let first = h.start_run(&support::long_running(), "started");
            h.invoke("run.run", json!(null));
            assert!(!h.state()["dialog"].is_null(), "{what}");
            h.press(keys);
            assert!(h.state()["dialog"].is_null(), "{what}");
            if b.command == "dialog.confirm" {
                h.wait_run("Stop and Rerun", |s| {
                    s["tabs"][0]["pid"].is_number() && s["tabs"][0]["pid"] != first
                });
            } else {
                assert_eq!(h.run()["tabs"][0]["pid"], first, "{what}");
            }
        }
        "tabs.close" => {
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tabs"].as_array().unwrap().len(), 2, "{what}");
        }
        "tabs.next" => {
            h.press(keys);
            assert_eq!(h.state()["active_tab"], json!(1), "{what}");
        }
        "tabs.previous" => {
            h.press(keys);
            assert_eq!(h.state()["active_tab"], json!(2), "{what}");
        }
        "menu.down" | "menu.up" => {
            h.open_menu("src");
            h.press(keys);
            // Down from no row selects the first one, Up the last one.
            let expected = if b.command == "menu.down" { 0 } else { 17 };
            assert_eq!(h.menu_levels()[0]["selected"], json!(expected), "{what}");
        }
        "menu.open-submenu" => {
            h.open_menu("src");
            h.invoke("menu.up", json!(null));
            h.press(keys);
            let levels = h.menu_levels();
            assert_eq!(levels.len(), 2, "{what}");
            assert_eq!(levels[1]["selected"], json!(0), "{what}");
        }
        "menu.close-submenu" => {
            h.open_menu("src");
            h.invoke("menu.up", json!(null));
            h.invoke("menu.open-submenu", json!(null));
            h.press(keys);
            assert_eq!(h.menu_levels().len(), 1, "{what}");
        }
        "menu.confirm" => {
            h.open_menu("src");
            h.invoke("menu.up", json!(null));
            h.press(keys);
            // On a submenu row: the submenu opens.
            assert_eq!(h.menu_levels().len(), 2, "{what}");
        }
        "menu.cancel" => {
            h.open_menu("src");
            h.press(keys);
            assert!(h.menu_levels().is_empty(), "{what}");
            assert!(h.tree_focused(), "{what}: the tree has the focus back");
        }
        _ => return false,
    }
    true
}
