//! The Commit tool window and its dialogs: the bindings of `keymaps/*/vcs-commit.toml`.

use super::*;

use super::vcs::{close_diffs, commit_row, ensure_repository, git_in, wait_diff, wait_vcs};

area_tests!("vcs-commit");

/// The subject of the last commit.
fn last_subject(root: &Path) -> String {
    let exe = git::GitExe::find(None).unwrap();
    std::process::Command::new(exe.path)
        .args(["log", "-1", "--format=%s"])
        .current_dir(root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// The Commit tool window's state.
fn commit_state(h: &mut H) -> Value {
    h.invoke("vcs.state", json!(null))["commit"].clone()
}

/// `a.py` changed on disk, listed (and included) in the Commit tool window.
fn change_a(h: &mut H, text: &str) {
    ensure_repository(h);
    std::fs::write(h.root.join("a.py"), text).unwrap();
    // No file watcher here: the statuses are read again.
    h.invoke("vcs.refresh", json!(null));
    wait_vcs(h, "a.py listed", |v| {
        commit_row(&v["commit"], "a.py")["status"] == "modified"
    });
    h.invoke(
        "vcs.commit.include",
        json!({ "paths": ["a.py"], "included": true }),
    );
}

/// The Commit tool window shown with the focus in its tree.
fn focus_commit_tree(h: &mut H) {
    if commit_state(h)["visible"] != json!(true) {
        h.invoke("workbench.toggle-commit", json!(null));
    }
    let focus = h.workbench(|w, cx| w.commit_view().read(cx).tree_focus());
    h.focus(focus);
}

fn commit_focused(h: &mut H, which: &str) -> bool {
    h.cx.run_until_parked();
    h.window
        .update(h.cx, |w, window, cx| {
            let view = w.commit_view().read(cx);
            match which {
                "message" => view.message_focus(cx).is_focused(window),
                _ => view.tree_focus().is_focused(window),
            }
        })
        .unwrap()
}

/// Opens Rollback Changes for `a.py` from the tree.
fn open_rollback(h: &mut H) {
    focus_commit_tree(h);
    h.invoke("commit-tree.select", json!({ "path": "a.py" }));
    h.invoke("vcs.rollback", json!(null));
    wait_vcs(h, "the rollback dialog", |v| {
        v["commit"]["dialog"]["kind"] == "rollback"
    });
    let focus = h.workbench(|w, cx| w.commit_dialog_focus(cx)).unwrap();
    h.focus(focus);
}

/// Opens "Git User Name Is Not Defined": no identity, a commit of `a.py`.
fn open_user_name(h: &mut H, text: &str) {
    change_a(h, text);
    git_in(h, &["config", "--unset", "user.name"]);
    git_in(h, &["config", "--unset", "user.email"]);
    h.invoke("vcs.commit", json!(null));
    h.invoke("vcs.commit.run", json!({ "message": "Named" }));
    wait_vcs(h, "the user name dialog", |v| {
        v["commit"]["dialog"]["kind"] == "user_name"
    });
    let focus = h.workbench(|w, cx| w.commit_dialog_focus(cx)).unwrap();
    h.focus(focus);
}

fn restore_identity(h: &mut H) {
    git_in(h, &["config", "user.name", "Test"]);
    git_in(h, &["config", "user.email", "test@example.invalid"]);
}

fn check_commit(h: &mut H, command: &str, keys: &str, what: &str) {
    match command {
        "vcs.commit" => {
            change_a(h, "a = 'commit'\n");
            h.focus_editor();
            h.press(keys);
            assert_eq!(commit_state(h)["visible"], json!(true), "{what}");
            assert!(
                commit_focused(h, "message"),
                "{what}: the message has the focus"
            );
        }
        "workbench.toggle-commit" => {
            ensure_repository(h);
            h.press(keys);
            assert_eq!(commit_state(h)["visible"], json!(true), "{what}");
            assert!(commit_focused(h, "tree"), "{what}: the tree has the focus");
        }
        "vcs.commit.push" => {
            change_a(h, "a = 'push'\n");
            h.invoke("vcs.commit", json!(null));
            h.invoke("vcs.commit.set-message", json!({ "text": "From keys" }));
            h.focus_editor();
            h.press(keys);
            let root = h.root.clone();
            wait_vcs(h, "the commit", |_| last_subject(&root) == "From keys");
        }
        "vcs.rollback" => {
            change_a(h, "a = 'rollback'\n");
            focus_commit_tree(h);
            h.invoke("commit-tree.select", json!({ "path": "a.py" }));
            h.press(keys);
            let commit = commit_state(h);
            assert_eq!(commit["dialog"]["kind"], "rollback", "{what}");
            assert_eq!(commit["dialog"]["files"][0]["path"], "a.py", "{what}");
        }
        "vcs.add" => {
            ensure_repository(h);
            std::fs::write(h.root.join("by_keys.py"), "k = 1\n").unwrap();
            h.invoke("workbench.toggle-commit", json!(null));
            h.invoke("vcs.refresh", json!(null));
            wait_vcs(h, "by_keys.py listed", |v| {
                v["status"]["unversioned"]
                    .as_array()
                    .is_some_and(|u| u.iter().any(|p| p == "by_keys.py"))
            });
            focus_commit_tree(h);
            h.invoke("commit-tree.select", json!({ "path": "by_keys.py" }));
            h.press(keys);
            wait_vcs(h, "by_keys.py added", |v| {
                commit_row(&v["commit"], "by_keys.py")["status"] == "added"
            });
            h.invoke(
                "vcs.rollback",
                json!({ "paths": ["by_keys.py"], "delete_added": true }),
            );
            let root = h.root.clone();
            wait_vcs(h, "by_keys.py gone", |_| !root.join("by_keys.py").exists());
        }
        "vcs.commit.run" => {
            change_a(h, "a = 'run'\n");
            h.invoke("vcs.commit", json!(null));
            h.invoke("vcs.commit.set-message", json!({ "text": "" }));
            h.press(keys);
            assert_eq!(
                commit_state(h)["problem"],
                "Specify commit message",
                "{what}"
            );
        }
        "vcs.commit.toggle-amend" => {
            ensure_repository(h);
            h.invoke("vcs.commit", json!(null));
            h.press(keys);
            assert_eq!(commit_state(h)["amend"], json!(true), "{what}");
            h.invoke("vcs.commit.toggle-amend", json!(null));
        }
        "vcs.commit.options" => {
            ensure_repository(h);
            h.invoke("vcs.commit", json!(null));
            h.press(keys);
            assert_eq!(commit_state(h)["options"]["open"], json!(true), "{what}");
            h.invoke("commit-options.close", json!(null));
        }
        "commit-options.close" => {
            ensure_repository(h);
            h.invoke("vcs.commit", json!(null));
            h.invoke("vcs.commit.options", json!(null));
            assert_eq!(commit_state(h)["options"]["open"], json!(true), "{what}");
            h.press(keys);
            assert_eq!(commit_state(h)["options"]["open"], json!(false), "{what}");
        }
        "commit-tree.up" | "commit-tree.down" => {
            change_a(h, "a = 'tree'\n");
            focus_commit_tree(h);
            let (from, to) = if command == "commit-tree.up" {
                ("a.py", "changes")
            } else {
                ("changes", "a.py")
            };
            h.invoke("commit-tree.select", json!({ "path": from }));
            h.press(keys);
            assert_eq!(
                commit_row(&commit_state(h), to)["selected"],
                json!(true),
                "{what}"
            );
        }
        "commit-tree.left" | "commit-tree.right" => {
            change_a(h, "a = 'tree'\n");
            focus_commit_tree(h);
            h.invoke("commit-tree.select", json!({ "path": "changes" }));
            if command == "commit-tree.right" {
                h.invoke("commit-tree.left", json!(null));
            }
            h.press(keys);
            let expanded = commit_row(&commit_state(h), "changes")["expanded"].clone();
            assert_eq!(expanded, json!(command == "commit-tree.right"), "{what}");
            h.invoke("commit-tree.expand-all", json!(null));
        }
        "commit-tree.toggle" => {
            change_a(h, "a = 'tree'\n");
            focus_commit_tree(h);
            h.invoke("commit-tree.select", json!({ "path": "a.py" }));
            h.press(keys);
            assert_eq!(
                commit_row(&commit_state(h), "a.py")["included"],
                json!(false),
                "{what}"
            );
            h.invoke(
                "vcs.commit.include",
                json!({ "paths": ["a.py"], "included": true }),
            );
        }
        "commit-tree.open" | "commit-tree.show-diff" => {
            change_a(h, "a = 'open'\n");
            focus_commit_tree(h);
            h.invoke("commit-tree.select", json!({ "path": "a.py" }));
            h.press(keys);
            let diff = wait_diff(h, what, |d| {
                d["path"].as_str().is_some_and(|p| p.ends_with("a.py"))
            });
            assert_eq!(diff["title"], "Commit: a.py", "{what}");
            close_diffs(h);
        }
        "commit-tree.jump-to-source" => {
            change_a(h, "a = 'open'\n");
            while h.state()["tabs"].as_array().is_some_and(|t| !t.is_empty()) {
                h.invoke("tabs.close", json!({ "index": 0 }));
            }
            focus_commit_tree(h);
            h.invoke("commit-tree.select", json!({ "path": "a.py" }));
            h.press(keys);
            let root = h.root.clone();
            let mut opened = false;
            for _ in 0..300 {
                support::tick(h.cx);
                if h.workbench(|w, cx| w.active_file(cx)) == Some(root.join("a.py")) {
                    opened = true;
                    break;
                }
            }
            assert!(opened, "{what}: a.py opens");
        }
        "rollback-dialog.confirm" | "rollback-dialog.cancel" => {
            change_a(h, "a = 'dialog'\n");
            let committed = git_in(h, &["show", "HEAD:a.py"]);
            open_rollback(h);
            h.press(keys);
            assert!(commit_state(h)["dialog"].is_null(), "{what}: closed");
            if command == "rollback-dialog.confirm" {
                let root = h.root.clone();
                wait_vcs(h, "a.py rolled back", |_| {
                    std::fs::read_to_string(root.join("a.py")).ok().as_deref()
                        == Some(committed.as_str())
                });
            } else {
                assert_eq!(
                    commit_row(&commit_state(h), "a.py")["status"],
                    "modified",
                    "{what}: nothing rolled back"
                );
            }
        }
        "user-name-dialog.confirm" => {
            open_user_name(h, "a = 'who'\n");
            h.invoke(
                "user-name-dialog.set",
                json!({ "name": "Test", "email": "test@example.invalid" }),
            );
            h.press(keys);
            let root = h.root.clone();
            wait_vcs(h, "the commit", |_| last_subject(&root) == "Named");
            assert!(commit_state(h)["dialog"].is_null(), "{what}");
            restore_identity(h);
        }
        "user-name-dialog.cancel" => {
            open_user_name(h, "a = 'nobody'\n");
            h.press(keys);
            assert!(commit_state(h)["dialog"].is_null(), "{what}");
            restore_identity(h);
        }
        "user-name-dialog.next-field" => {
            open_user_name(h, "a = 'tab'\n");
            h.invoke("user-name-dialog.set", json!({ "email": "" }));
            h.press(keys);
            h.invoke("editor.type", json!({ "text": "x" }));
            assert_eq!(commit_state(h)["dialog"]["email"], "x", "{what}");
            h.invoke("user-name-dialog.cancel", json!(null));
            restore_identity(h);
        }
        other => panic!("{what}: no commit check for {other}"),
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        // The Commit tool window (#170).
        "vcs.commit"
        | "workbench.toggle-commit"
        | "vcs.commit.push"
        | "vcs.rollback"
        | "vcs.add"
        | "vcs.commit.run"
        | "vcs.commit.toggle-amend"
        | "vcs.commit.options" => check_commit(h, &b.command, keys, &what),
        c if c.starts_with("commit-tree.")
            || c.starts_with("commit-options.")
            || c.starts_with("rollback-dialog.")
            || c.starts_with("user-name-dialog.") =>
        {
            check_commit(h, c, keys, &what)
        }
        _ => return false,
    }
    true
}
