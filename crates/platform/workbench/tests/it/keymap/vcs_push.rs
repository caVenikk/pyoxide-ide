//! Push, Update Project and the VCS Operations popup: the bindings of `keymaps/*/vcs-push.toml`.

use super::*;

use super::vcs::{open_branches, wait_vcs};

area_tests!("vcs-push");

/// The repository of [`open_branches`] with a bare `origin` inside its
/// `.git` folder (out of the tree and the searches) and a commit nobody
/// pushed yet.
fn with_remote(h: &mut H) {
    open_branches(h);
    h.invoke("vcs.branches-popup", json!(null));
    let git = |h: &H, args: &[&str]| {
        let exe = git::GitExe::find(None).expect("git is installed");
        let out = std::process::Command::new(exe.path)
            .args(args)
            .current_dir(&h.root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let remote = h.root.join(".git/test-remote.git");
    if !remote.exists() {
        // Without the verbatim prefix of Windows' `canonicalize`: git reads
        // `\\?\C:\...` as a host name.
        let path = run_config::paths::simplified(&remote.display().to_string());
        git(h, &["init", "-q", "--bare", &path]);
        git(h, &["remote", "add", "origin", &path]);
    }
    git(h, &["commit", "-q", "--allow-empty", "-m", "outgoing"]);
}

/// Opens the Push dialog of `main` and waits for its commits.
fn open_push(h: &mut H) -> Value {
    with_remote(h);
    h.invoke("vcs.push", json!(null));
    wait_vcs(h, "the Push dialog", |v| {
        v["remote"]["push_dialog"]["loading"] == false
    })["remote"]["push_dialog"]
        .clone()
}

fn check_remote(h: &mut H, command: &str, keys: &str, what: &str) {
    match command {
        "vcs.push" => {
            with_remote(h);
            h.press(keys);
            let vcs = wait_vcs(h, "the Push dialog", |v| {
                !v["remote"]["push_dialog"].is_null()
            });
            assert_eq!(
                vcs["remote"]["push_dialog"]["source"], "main",
                "{what}: the current branch"
            );
        }
        "vcs.update-project" => {
            with_remote(h);
            h.press(keys);
            let vcs = h.invoke("vcs.state", json!(null));
            assert_eq!(
                vcs["remote"]["update_dialog"]["title"], "Update Project",
                "{what}"
            );
        }
        "vcs.operations-popup" => {
            h.press(keys);
            let popup = remote_ui(h)["operations_popup"].clone();
            assert_eq!(popup["title"], "VCS Operations", "{what}: {popup}");
        }
        "update-dialog.ok" | "update-dialog.cancel" => {
            with_remote(h);
            h.invoke("vcs.update-project", json!({ "dialog": true }));
            assert!(!remote_ui(h)["update_dialog"].is_null(), "{what}: open");
            h.press(keys);
            assert!(remote_ui(h)["update_dialog"].is_null(), "{what}: closed");
            if command == "update-dialog.ok" {
                // `main` tracks nothing: the update cannot start.
                wait_vcs(h, "the update's balloon", |_| true);
            }
        }
        "push-dialog.cancel" => {
            open_push(h);
            h.press(keys);
            assert!(remote_ui(h)["push_dialog"].is_null(), "{what}");
        }
        "push-dialog.down" | "push-dialog.up" => {
            let dialog = open_push(h);
            assert!(
                dialog["rows"].as_array().unwrap().len() > 1,
                "{what}: {dialog}"
            );
            if command == "push-dialog.up" {
                h.invoke("push-dialog.select", json!({ "row": 1 }));
            }
            h.press(keys);
            let expected = if command == "push-dialog.down" { 1 } else { 0 };
            assert_eq!(remote_ui(h)["push_dialog"]["selected"], expected, "{what}");
        }
        "push-dialog.confirm-target" | "push-dialog.cancel-target" => {
            open_push(h);
            h.invoke("push-dialog.edit-target", json!(null));
            assert_eq!(remote_ui(h)["push_dialog"]["editing"], "main", "{what}");
            h.press(keys);
            let dialog = remote_ui(h)["push_dialog"].clone();
            assert!(dialog["editing"].is_null(), "{what}: {dialog}");
            assert_eq!(dialog["target"]["branch"], "main", "{what}");
        }
        "push-dialog.push" => {
            open_push(h);
            h.press(keys);
            assert!(remote_ui(h)["push_dialog"].is_null(), "{what}: closed");
            let pushed = h.root.join(".git/test-remote.git/refs/heads/main");
            for _ in 0..3000 {
                if pushed.exists() {
                    break;
                }
                support::tick(h.cx);
            }
            assert!(pushed.exists(), "{what}: main reached the remote");
        }
        _ => panic!("no check for {command}"),
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "vcs.push" | "vcs.update-project" | "vcs.operations-popup" => {
            check_remote(h, &b.command, keys, &what)
        }
        c if c.starts_with("push-dialog.") || c.starts_with("update-dialog.") => {
            check_remote(h, c, keys, &what)
        }
        _ => return false,
    }
    true
}
