//! Branches: the bindings of `keymaps/*/vcs-branches.toml`.

use super::*;

use super::vcs::{open_branches, wait_vcs};

area_tests!("vcs-branches");

/// The selected row of the branches popup.
fn selected_row(popup: &Value) -> Value {
    popup["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["selected"] == true)
        .cloned()
        .unwrap_or(Value::Null)
}

fn branches_popup(h: &mut H) -> Value {
    h.invoke("vcs.state", json!(null))["branches_popup"].clone()
}

fn check_branches(h: &mut H, command: &str, keys: &str, what: &str) {
    match command {
        "git.new-branch" => {
            h.press(keys);
            let vcs = wait_vcs(h, "the New Branch dialog", |v| {
                !v["branch_dialog"].is_null()
            });
            assert_eq!(vcs["branch_dialog"]["title"], "Create New Branch", "{what}");
        }
        "branch-dialog.cancel" | "branch-dialog.confirm" => {
            if command == "branch-dialog.confirm" {
                open_branches(h);
                h.invoke("vcs.branches-popup", json!(null));
            }
            h.invoke("git.new-branch", json!(null));
            wait_vcs(h, "the New Branch dialog", |v| {
                !v["branch_dialog"].is_null()
            });
            h.invoke("branch-dialog.set-name", json!({ "text": "from-keys" }));
            h.press(keys);
            let vcs = h.invoke("vcs.state", json!(null));
            assert!(vcs["branch_dialog"].is_null(), "{what}: closed");
            if command == "branch-dialog.confirm" {
                wait_vcs(h, "the new branch", |v| v["widget"]["text"] == "from-keys");
                h.invoke("git.checkout", json!({ "ref": "main" }));
                wait_vcs(h, "main", |v| v["widget"]["text"] == "main");
                h.invoke("git.delete-branch", json!({ "name": "from-keys" }));
            }
        }
        "vcs.branches-popup" => {
            h.press(keys);
            assert!(!branches_popup(h).is_null(), "{what}: the popup shows");
        }
        _ => {
            let popup = open_branches(h);
            assert_eq!(selected_row(&popup)["name"], "main", "{what}");
            match command {
                "branches-popup.down" | "branches-popup.up" => {
                    h.press(keys);
                    let row = selected_row(&branches_popup(h));
                    let expected = if command == "branches-popup.down" {
                        "folder"
                    } else {
                        "group"
                    };
                    assert_eq!(row["kind"], expected, "{what}: {row}");
                }
                "branches-popup.left" => {
                    h.press(keys);
                    let row = selected_row(&branches_popup(h));
                    assert_eq!(row["text"], "Local", "{what}");
                }
                "branches-popup.right" => {
                    h.invoke("branches-popup.left", json!(null));
                    h.invoke("branches-popup.left", json!(null));
                    assert_eq!(selected_row(&branches_popup(h))["expanded"], false);
                    h.press(keys);
                    let row = selected_row(&branches_popup(h));
                    assert_eq!(
                        (row["text"].clone(), row["expanded"].clone()),
                        (json!("Local"), json!(true)),
                        "{what}"
                    );
                }
                "branches-popup.confirm" => {
                    h.press(keys);
                    let submenu = branches_popup(h)["submenu"].clone();
                    assert!(
                        submenu.to_string().contains("New Branch from 'main'"),
                        "{what}: {submenu}"
                    );
                }
                "branches-popup.close" => {
                    h.invoke("branches-popup.search", json!({ "text": "ma" }));
                    h.press(keys);
                    let popup = branches_popup(h);
                    assert_eq!(popup["search"], "", "{what}: the search is cleared");
                }
                "branches-popup.delete" => {
                    h.invoke("branches-popup.search", json!({ "text": "ma" }));
                    h.press(keys);
                    assert_eq!(branches_popup(h)["search"], "m", "{what}");
                }
                "branches-popup.rename" => {
                    h.invoke("branches-popup.search", json!({ "text": "other" }));
                    h.press(keys);
                    let vcs = wait_vcs(h, "the rename dialog", |v| !v["branch_dialog"].is_null());
                    assert_eq!(
                        vcs["branch_dialog"]["title"], "Rename Branch other",
                        "{what}"
                    );
                }
                _ => panic!("no check for {command}"),
            }
        }
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "git.new-branch"
        | "vcs.branches-popup"
        | "branch-dialog.confirm"
        | "branch-dialog.cancel" => check_branches(h, &b.command, keys, &what),
        c if c.starts_with("branches-popup.") => check_branches(h, c, keys, &what),
        _ => return false,
    }
    true
}
