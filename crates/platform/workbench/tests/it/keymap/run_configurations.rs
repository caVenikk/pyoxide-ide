//! The Run/Debug Configurations dialog: the bindings of `keymaps/*/run-configurations.toml`.

use super::*;

area_tests!("run-configurations");

impl H<'_> {
    /// Opens the Run/Debug Configurations dialog with a new "Script"
    /// configuration selected and the tree focused.
    fn open_run_dialog(&mut self) {
        self.invoke("run.configurations.edit", json!(null));
        self.invoke("run.configurations.dialog.add", json!({"type": "script"}));
        self.focus_run_dialog(None);
    }

    fn run_dialog_selected(&mut self) -> Value {
        self.run_dialog()["selected"]["name"].clone()
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "run.configurations.dialog.ok"
        | "run.configurations.dialog.cancel"
        | "run.configurations.dialog.modify-options" => {
            let before = h.run()["configurations"]["configurations"]
                .as_array()
                .unwrap()
                .len();
            h.open_run_dialog();
            if b.context.as_deref().is_some_and(|c| c.ends_with("Editor")) {
                h.focus_run_dialog(Some("command"));
            }
            h.press(keys);
            match b.command.as_str() {
                "run.configurations.dialog.modify-options" => {
                    assert!(!h.run_dialog()["menu"].is_null(), "{what}");
                }
                command => {
                    assert!(h.run_dialog().is_null(), "{what}");
                    let count = h.run()["configurations"]["configurations"]
                        .as_array()
                        .unwrap()
                        .len();
                    let expected = if command.ends_with(".ok") {
                        before + 1
                    } else {
                        before
                    };
                    assert_eq!(count, expected, "{what}");
                }
            }
        }
        "run.configurations.dialog.next" | "run.configurations.dialog.previous" => {
            h.open_run_dialog();
            let first = h.run_dialog_selected();
            h.invoke("run.configurations.dialog.add", json!({"type": "script"}));
            let second = h.run_dialog_selected();
            let (from, to) = if b.command.ends_with("next") {
                (first, second)
            } else {
                (second, first)
            };
            h.invoke("run.configurations.dialog.select", json!({"name": from}));
            h.focus_run_dialog(None);
            h.press(keys);
            assert_eq!(h.run_dialog_selected(), to, "{what}");
        }
        "run.configurations.dialog.remove-tag" => {
            h.open_run_dialog();
            let tagged = |h: &mut H| -> Vec<String> {
                h.run_dialog()["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|o| o["tag"] == json!(true) && o["checked"] == json!(true))
                    .map(|o| o["id"].as_str().unwrap().to_owned())
                    .collect()
            };
            let option = tagged(h).first().cloned().expect("an option tag");
            let dialog = h
                .workbench(|w, cx| w.run_window().read(cx).edit_dialog().cloned())
                .expect("the dialog is open");
            let handle =
                h.cx.update(|cx| dialog.read(cx).tag_focus(&option).unwrap());
            h.focus(handle);
            h.press(keys);
            assert!(!tagged(h).contains(&option), "{what}");
        }
        "run.configurations.dialog.toggle-option" => {
            h.open_run_dialog();
            if b.context.as_deref().is_some_and(|c| c.ends_with("Editor")) {
                h.focus_run_dialog(Some("command"));
            }
            let option = b
                .args
                .as_ref()
                .and_then(|a| a["option"].as_str())
                .unwrap_or_default()
                .to_owned();
            let checked = |h: &mut H| {
                h.run_dialog()["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|o| o["id"] == json!(option))
                    .map(|o| o["checked"].clone())
            };
            let before = checked(h);
            h.press(keys);
            // The test's Script type has the common options only.
            if option == "allow-parallel" {
                assert_ne!(checked(h), before, "{what}");
            }
        }
        "run.configurations.dialog.expand" | "run.configurations.dialog.collapse" => {
            h.open_run_dialog();
            // The type of the new configuration: collapse hides it, expand
            // shows it again.
            h.invoke(
                "run.configurations.dialog.select",
                json!({"type": "script"}),
            );
            let expand = b.command.ends_with("expand");
            if expand {
                h.invoke("run.configurations.dialog.collapse", json!(null));
            }
            h.focus_run_dialog(None);
            h.press(keys);
            let tree = h.run_dialog()["tree"].clone();
            let node = tree
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["kind"] == json!("type") && n["type"] == json!("script"))
                .cloned()
                .unwrap();
            assert_eq!(node["expanded"], json!(expand), "{what}");
        }
        "run.configurations.dialog.add" => {
            h.open_run_dialog();
            h.press(keys);
            assert_eq!(
                h.run_dialog()["menu"]["title"],
                json!("Add New Configuration"),
                "{what}"
            );
        }
        "run.configurations.dialog.remove" => {
            h.open_run_dialog();
            let name = h.run_dialog_selected();
            h.press(keys);
            let tree = h.run_dialog()["tree"].clone();
            assert!(
                !tree.as_array().unwrap().iter().any(|n| n["text"] == name),
                "{what}"
            );
        }
        "run.configurations.dialog.copy" => {
            h.open_run_dialog();
            let name = h.run_dialog_selected();
            let count = h.run_dialog()["tree"].as_array().unwrap().len();
            h.press(keys);
            assert_ne!(h.run_dialog_selected(), name, "{what}");
            assert_eq!(
                h.run_dialog()["tree"].as_array().unwrap().len(),
                count + 1,
                "{what}"
            );
        }
        _ => return false,
    }
    true
}
