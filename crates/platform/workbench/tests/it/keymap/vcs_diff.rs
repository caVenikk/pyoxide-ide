//! The diff viewer: the bindings of `keymaps/*/vcs-diff.toml`.

use super::*;

use super::vcs::{close_diffs, commit_row, ensure_repository, git_in, wait_diff, wait_vcs};

area_tests!("vcs-diff");

/// The diff of `a.py` against its committed text with a line added at
/// its start and one at its end, the focus in its right side. `a.py` is
/// open in an editor tab: the diff edits the tab's text, so the lines are
/// typed into the tab (a write to the file would not reach it).
fn open_diff(h: &mut H, what: &str) -> Value {
    ensure_repository(h);
    let base = git_in(h, &["show", "HEAD:a.py"]);
    let tab = h.state()["tabs"]
        .as_array()
        .and_then(|t| t.iter().position(|t| t["path"] == json!("a.py")))
        .expect("a.py is open");
    h.invoke("tabs.select", json!({ "index": tab }));
    h.set_editor(&format!("{CARET}first = 0\n{base}last = 9\n"));
    h.invoke("vcs.show-diff", json!({ "path": "a.py" }));
    let diff = wait_diff(h, what, |d| {
        d["path"].as_str().is_some_and(|p| p.ends_with("a.py"))
            && d["changes"].as_array().is_some_and(|c| c.len() == 2)
    });
    focus_diff(h, vcs_ui::diff::Side::Right);
    diff
}

fn focus_diff(h: &mut H, side: vcs_ui::diff::Side) {
    let focus = h.workbench(|w, cx| {
        w.active_diff()
            .expect("a diff")
            .read(cx)
            .editor(side)
            .focus_handle(cx)
    });
    h.focus(focus);
}

fn check_diff(h: &mut H, command: &str, keys: &str, what: &str) {
    match command {
        "diff.next" | "diff.previous" => {
            let diff = open_diff(h, what);
            let last = diff["changes"][1]["right"][0].clone();
            if command == "diff.previous" {
                h.invoke("diff.next", json!(null));
            }
            h.press(keys);
            let expected = if command == "diff.next" {
                last
            } else {
                json!(0)
            };
            assert_eq!(h.state()["diff"]["right"]["caret"][0], expected, "{what}");
        }
        "diff.next-file" | "diff.previous-file" => {
            // An unversioned file: the list of the unversioned files, which
            // the Commit tool window keeps (shown, with its groups open).
            ensure_repository(h);
            if h.invoke("vcs.state", json!(null))["commit"]["visible"] != json!(true) {
                h.invoke("workbench.toggle-commit", json!(null));
            }
            h.invoke("commit-tree.expand-all", json!(null));
            h.invoke("vcs.refresh", json!(null));
            wait_vcs(h, "b.py unversioned", |v| {
                commit_row(&v["commit"], "b.py")["status"] == "unversioned"
            });
            h.invoke("vcs.show-diff", json!({ "path": "b.py" }));
            let diff = wait_diff(h, what, |d| {
                d["path"].as_str().is_some_and(|p| p.ends_with("b.py"))
            });
            let index = diff["files"][0].as_u64().unwrap();
            let count = diff["files"][1].as_u64().unwrap();
            assert!(count > 1, "{what}: {diff:#}");
            let target = if command == "diff.next-file" {
                if index + 1 == count {
                    h.invoke("diff.previous-file", json!(null));
                    index
                } else {
                    index + 1
                }
            } else if index == 0 {
                h.invoke("diff.next-file", json!(null));
                0
            } else {
                index - 1
            };
            focus_diff(h, vcs_ui::diff::Side::Right);
            h.press(keys);
            wait_diff(h, what, |d| d["files"][0] == json!(target));
        }
        "diff.focus-opposite" => {
            open_diff(h, what);
            h.press(keys);
            assert_eq!(h.state()["diff"]["current_side"], "left", "{what}");
        }
        "diff.jump-to-source" => {
            open_diff(h, what);
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
            assert_eq!(h.caret_line().0, 0, "{what}: at the caret's line");
        }
        other => panic!("{what}: no diff check for {other}"),
    }
    close_diffs(h);
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        // The diff viewer (#171).
        c if c.starts_with("diff.") => check_diff(h, c, keys, &what),
        _ => return false,
    }
    true
}
