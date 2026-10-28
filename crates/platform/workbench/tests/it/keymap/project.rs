//! The Project tool window: the bindings of `keymaps/*/project.toml`.

use super::*;

area_tests!("project");

impl H<'_> {
    fn tree_rows(&mut self) -> Vec<Value> {
        self.state()["tree"]["rows"].as_array().unwrap().clone()
    }

    fn tree_row(&mut self, path: &str) -> Value {
        self.tree_rows()
            .into_iter()
            .find(|r| r["path"] == json!(path))
            .unwrap_or_else(|| panic!("no tree row {path}"))
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "workbench.toggle-project" => {
            h.press(keys);
            assert!(h.tree_focused(), "{what}: first press focuses the tree");
            h.press(keys);
            let s = h.state();
            assert_eq!(
                s["tool_windows"]["project"]["visible"],
                json!(false),
                "{what}: second press hides it"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.hide-tool-window" => {
            h.focus_tree();
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["project"]["visible"], json!(false));
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.focus-editor" => {
            h.focus_tree();
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.save-all" => {
            h.set_editor("|");
            h.invoke("editor.type", json!({ "text": "x = 9" }));
            h.focus_editor();
            h.press(keys);
            let disk = std::fs::read_to_string(h.root.join("a.py")).unwrap();
            assert_eq!(disk, "x = 9", "{what}");
            std::fs::write(h.root.join("a.py"), "a = 1\n").unwrap();
        }
        "workbench.synchronize" => {
            // Saves a.py, then reloads b.py, changed outside.
            h.set_editor("|");
            h.invoke("editor.type", json!({ "text": "s = 1" }));
            std::fs::write(h.root.join("b.py"), "b = 3\n").unwrap();
            h.focus_editor();
            h.press(keys);
            let disk = std::fs::read_to_string(h.root.join("a.py")).unwrap();
            assert_eq!(disk, "s = 1", "{what}");
            h.invoke("tabs.select", json!({ "index": 1 }));
            let text = h.invoke("editor.text", json!(null));
            assert_eq!(text, json!("b = 3\n"), "{what}");
            std::fs::write(h.root.join("a.py"), "a = 1\n").unwrap();
            std::fs::write(h.root.join("b.py"), "b = 2\n").unwrap();
        }
        "project.tree.up" | "project.tree.down" => {
            h.select_in_tree("src/m.py");
            let rows = h.tree_rows();
            let i = rows
                .iter()
                .position(|r| r["path"] == json!("src/m.py"))
                .unwrap();
            let expected = if b.command.ends_with("up") {
                i - 1
            } else {
                i + 1
            };
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["selected"],
                rows[expected]["path"],
                "{what}"
            );
        }
        "project.tree.expand" => {
            h.select_in_tree("src/pkg");
            h.invoke("project.tree.collapse-all", json!(null));
            h.select_in_tree("src/pkg");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(false));
            h.press(keys);
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true), "{what}");
        }
        "project.tree.collapse" => {
            h.select_in_tree("src/pkg/n.py");
            h.select_in_tree("src/pkg");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true));
            h.press(keys);
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(false), "{what}");
        }
        "project.tree.open" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], json!("src/m.py"), "{what}");
            h.invoke("tabs.close", json!(null));
        }
        "project.open-in-right-split" => {
            // With a file open, so there is a group to split.
            h.invoke("project.open-file", json!({ "path": "src/pkg/n.py" }));
            h.select_in_tree("src/m.py");
            h.press(keys);
            let s = h.state();
            // A new group at the right shows the file and has the focus.
            let groups = s["splits"]["groups"].as_array().unwrap();
            assert_eq!(groups.len(), 2, "{what}: {s}");
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], json!("src/m.py"), "{what}");
            assert_eq!(s["splits"]["focused"], groups[1]["id"], "{what}");
            h.invoke("tabs.unsplit-all", json!(null));
            h.invoke("tabs.close-all", json!(null));
        }
        "project.tree.collapse-all" => {
            h.select_in_tree("src/pkg/n.py");
            h.press(keys);
            assert_eq!(h.tree_row("src")["expanded"], json!(false), "{what}");
        }
        "project.tree.expand-all" => {
            h.select_in_tree("src/pkg/n.py");
            h.invoke("project.tree.collapse-all", json!(null));
            h.select_in_tree("src");
            h.press(keys);
            assert_eq!(h.tree_row("src")["expanded"], json!(true), "{what}");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true), "{what}");
        }
        "project.tree.context-menu" => {
            h.select_in_tree("src");
            h.press(keys);
            let levels = h.menu_levels();
            assert_eq!(levels[0]["items"][0]["label"], json!("New"), "{what}");
            assert_eq!(
                levels[0]["items"][7]["label"],
                json!("Find in Files…"),
                "{what}"
            );
            assert_eq!(
                levels[0]["items"][19]["label"],
                json!("Mark Directory As"),
                "{what}"
            );
        }
        "project.tree.new" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let menu = h.state()["tree"]["context_menu"].clone();
            assert_eq!(menu["title"], json!("New"), "{what}");
            let items = &menu["levels"][0]["items"];
            assert_eq!(items[0]["label"], json!("File"), "{what}");
            assert_eq!(items[1]["label"], json!("Directory"), "{what}");
        }
        "project.tree.rename" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            assert_eq!(
                h.name_popup()["label"],
                json!("Rename file 'm.py' to:"),
                "{what}"
            );
        }
        "project.tree.delete" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            assert_eq!(
                h.state()["dialog"]["message"],
                json!("Delete file \"m.py\"?"),
                "{what}"
            );
            assert!(h.root.join("src/m.py").exists(), "{what}: only asked");
        }
        "name-input.confirm" => {
            h.select_in_tree("src");
            h.invoke("project.tree.new-directory", json!(null));
            h.invoke("editor.type", json!({ "text": "made" }));
            h.press(keys);
            assert!(h.name_popup().is_null(), "{what}");
            assert!(h.root.join("src/made").is_dir(), "{what}");
            h.invoke("project.tree.delete", json!({ "paths": ["src/made"] }));
            h.invoke("dialog.confirm", json!(null));
            assert!(!h.root.join("src/made").exists(), "{what}");
        }
        "name-input.next-kind" | "name-input.previous-kind" => {
            // Two kinds: either way the second one, going round.
            h.select_in_tree("src");
            h.invoke("project.tree.new-item", json!({ "item": "Python File" }));
            h.press(keys);
            assert_eq!(
                h.name_popup()["selected_kind"],
                json!("Python unit test"),
                "{what}"
            );
        }
        "name-input.next-field" | "name-input.previous-field" => {
            // The Copy dialog of one file: New name, then To directory.
            h.select_in_tree("src/m.py");
            h.invoke("project.tree.copy-element", json!(null));
            let dialog = h.workbench(|w, cx| w.panel().read(cx).transfer_dialog().cloned());
            let dialog = dialog.expect("the Copy dialog");
            h.press(keys);
            let directory =
                h.cx.update(|cx| dialog.read(cx).directory_field().focus_handle(cx));
            let focused =
                h.cx.update_window(h.window.into(), |_, window, _| directory.is_focused(window))
                    .unwrap();
            assert!(focused, "{what}: the directory field has the focus");
        }
        "project.tree.cut" | "project.tree.copy" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["clipboard"],
                json!({ "paths": ["src/m.py"], "cut": b.command.ends_with("cut") }),
                "{what}"
            );
        }
        "project.tree.paste" => {
            h.select_in_tree("src/m.py");
            h.invoke("project.tree.copy", json!(null));
            h.select_in_tree("src/pkg");
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["transfer"]["title"],
                json!("Copy"),
                "{what}"
            );
        }
        "project.tree.move" | "project.tree.copy-element" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let title = if b.command.ends_with("move") {
                "Move"
            } else {
                "Copy"
            };
            assert_eq!(
                h.state()["tree"]["transfer"]["title"],
                json!(title),
                "{what}"
            );
        }
        "project.tree.copy-paths" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let text =
                h.cx.update(|cx| cx.read_from_clipboard())
                    .and_then(|c| c.text());
            assert_eq!(
                text,
                Some(h.root.join("src").join("m.py").display().to_string()),
                "{what}"
            );
        }
        "name-input.cancel" => {
            h.select_in_tree("src");
            h.invoke("project.tree.new-file", json!(null));
            assert!(!h.name_popup().is_null(), "{what}");
            h.press(keys);
            assert!(h.name_popup().is_null(), "{what}");
            assert!(h.tree_focused(), "{what}: the tree has the focus back");
        }
        _ => return false,
    }
    true
}
