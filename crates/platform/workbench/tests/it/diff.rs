//! The diff viewer in an editor tab (#171, scenario `vcs/diff-viewer.md`)
//! against repositories made with the git executable: opening it from
//! the Commit tool window (a preview tab of the list), from the change
//! marker popup and from the project tool window; the titles, changes and
//! changed words; Next and Previous Difference, the next file after the
//! last difference, Compare Next and Previous File, Select Opposite Diff
//! Pane, Jump to Source; edits of the right side reaching the file's tab,
//! its change markers and, saved, the Commit tool window; Ignore
//! Differences. Every git call of this test binary reads a global config
//! file of its own.

use crate::support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Focusable as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use vcs_ui::diff::Side;
use workbench::{Product, SyncSettings, Workbench};

fn global_config() -> PathBuf {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let path = DIR
        .get_or_init(|| tempfile::tempdir().unwrap())
        .path()
        .join("global-config");
    git::use_global_config(path.clone());
    path
}

fn git(dir: &Path, args: &[&str]) -> String {
    let exe = git::GitExe::find(None).expect("git is installed");
    let out = Command::new(exe.path)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", global_config())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The committed `main.py` (22 lines, as the demo project's) and `b.py`.
const MAIN: &str = "def describe(items):\n    summary = {\"count\": len(items), \"sum\": 0}\n    for value in items:\n        summary[\"sum\"] += value\n    return summary\n\n\ndef fail(value):\n    if value > 2:\n        raise ValueError(value)\n    return value\n\n\ndef total(items):\n    return sum(items)\n\n\nif __name__ == \"__main__\":\n    numbers = [1, 2, 3, 4]\n    print(describe(numbers))\n    print(total(numbers))\n";

fn repository(root: &Path) {
    global_config();
    git::testing::init(root);
    for (key, value) in [
        ("commit.gpgsign", "false"),
        ("core.fsmonitor", "false"),
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
    ] {
        git(root, &["config", key, value]);
    }
    write(root, "main.py", MAIN);
    write(root, "b.py", "b = 1\nc = 2\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "initial"]);
}

/// `main.py` with a changed word on line 10 and a line added at the end;
/// `b.py` with its second line changed.
fn change(root: &Path) {
    let main =
        MAIN.replace("raise ValueError(value)", "raise ValueError(\"too big\")") + "# changed\n";
    write(root, "main.py", &main);
    write(root, "b.py", "b = 1\nc = 3\n");
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
}

impl H<'_> {
    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
        out
    }

    fn draw(&mut self) {
        self.cx.run_until_parked();
        let _ = self
            .cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear());
    }

    fn state(&mut self) -> Value {
        self.invoke("workbench.state", Value::Null)
    }

    fn diff(&mut self) -> Value {
        self.state()["diff"].clone()
    }

    fn wait<T>(&mut self, what: &str, f: impl Fn(&mut Self) -> Option<T>) -> T {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            self.draw();
            if let Some(v) = f(self) {
                return v;
            }
        }
        let state = self.state();
        panic!("timed out waiting for {what}: {:#}", state["diff"]);
    }

    /// Waits for the active diff to show `file` with its differences.
    fn wait_diff(&mut self, file: &str) -> Value {
        let file = file.to_owned();
        self.wait(&format!("the diff of {file}"), move |h| {
            let d = h.diff();
            (d["computed"] == true
                && d["path"]
                    .as_str()
                    .is_some_and(|p| Path::new(p).ends_with(&file)))
            .then_some(d)
        })
    }

    fn commit(&mut self) -> Value {
        self.invoke("vcs.state", Value::Null)["commit"].clone()
    }

    fn wait_commit(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        self.wait(what, move |h| {
            let c = h.commit();
            f(&c).then_some(c)
        })
    }

    fn focus(&mut self, f: impl FnOnce(&Workbench, &gpui::App) -> gpui::FocusHandle) {
        self.window
            .update(self.cx, |w, window, cx| {
                let focus = f(w, cx);
                window.focus(&focus);
            })
            .unwrap();
        self.draw();
    }

    fn focus_diff(&mut self, side: Side) {
        self.focus(|w, cx| {
            w.active_diff()
                .expect("a diff tab")
                .read(cx)
                .editor(side)
                .focus_handle(cx)
        });
    }

    fn focus_commit_tree(&mut self) {
        if self.commit()["visible"] != true {
            self.invoke("workbench.toggle-commit", Value::Null);
        }
        self.focus(|w, cx| w.commit_view().read(cx).tree_focus());
    }

    /// Clicks a change's action in a side's gutter (`>>`, the check box).
    fn click_action(&mut self, side: Side, index: usize) {
        let at = self
            .window
            .update(self.cx, |w, _, cx| {
                w.active_diff()
                    .expect("a diff")
                    .read(cx)
                    .action_bounds(side, index, cx)
            })
            .unwrap()
            .unwrap_or_else(|| panic!("the action of change {index} is drawn"))
            .center();
        let mut vcx = gpui::VisualTestContext::from_window(self.window.into(), self.cx);
        vcx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        vcx.simulate_click(at, gpui::Modifiers::none());
        self.draw();
    }

    fn right_text(&mut self) -> String {
        self.window
            .update(self.cx, |w, _, cx| {
                w.active_diff()
                    .expect("a diff")
                    .read(cx)
                    .editor(Side::Right)
                    .read(cx)
                    .text()
            })
            .unwrap()
    }

    fn right_caret(&mut self) -> u64 {
        self.diff()["right"]["caret"][0].as_u64().unwrap()
    }
}

fn open(cx: &mut TestAppContext, prepare: impl FnOnce(&Path)) -> (H<'_>, tempfile::TempDir) {
    global_config();
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    prepare(&root);
    let product = Product::default();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
        .update(cx, |w, _, _| {
            w.set_sync_settings(SyncSettings {
                watch: true,
                ..SyncSettings::default()
            })
        })
        .unwrap();
    let mut h = H { cx, window, root };
    let path = h.root.display().to_string();
    h.invoke("project.open-folder", json!({ "path": path }));
    h.wait_commit("the changes", |c| {
        c["state"] == "ready" && c["rows"].as_array().is_some_and(|r| r.len() >= 2)
    });
    (h, dir)
}

#[gpui::test]
fn the_commit_window_opens_a_preview_diff_of_its_list(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
    });
    let head = git(&h.root, &["rev-parse", "HEAD"]);
    h.focus_commit_tree();
    h.invoke("commit-tree.select", json!({ "path": "main.py" }));
    h.invoke("commit-tree.open", Value::Null);
    let d = h.wait_diff("main.py");
    let state = h.state();
    let tabs = state["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 1, "{tabs:#?}");
    assert_eq!(tabs[0]["title"], "Commit: main.py");
    assert_eq!(tabs[0]["preview"], true);
    assert_eq!(state["active_tab"], 0);
    // The active file is none (no run of Current File, no breadcrumbs).
    assert_eq!(
        h.window.update(h.cx, |w, _, cx| w.active_file(cx)).unwrap(),
        None
    );
    // b.py and main.py are the Changes; main.py is the second.
    assert_eq!(d["files"], json!([1, 2]));
    assert_eq!(d["files_text"], "2/2 files");
    assert_eq!(d["left_title"]["label"], head[..8].to_owned());
    assert_eq!(d["left_title"]["path"], "main.py");
    assert_eq!(d["left_title"]["read_only"], true);
    assert_eq!(d["right_title"]["label"], "Current version");
    assert_eq!(d["right_title"]["path"], Value::Null);
    assert_eq!(d["included"], true);
    assert_eq!(d["left"]["read_only"], true);
    assert_eq!(d["right"]["read_only"], false);
    assert_eq!(d["status"], "2 differences");
    assert_eq!(
        d["changes"],
        json!([
            { "left": [9, 10], "right": [9, 10], "kind": "modified", "words": 1, "excluded": false },
            { "left": [21, 21], "right": [21, 22], "kind": "inserted", "words": null, "excluded": false },
        ])
    );
    // The changed word on both sides.
    assert_eq!(d["left"]["words"], 1);
    assert_eq!(d["right"]["words"], 1);
    // The views are at the first change, the caret at its start.
    assert_eq!(d["right"]["caret"], json!([9, 0]));
    assert_eq!(d["left"]["caret"], json!([9, 0]));
    assert_eq!(d["current_side"], "right");

    // Enter on b.py: the same preview tab shows it.
    h.focus_commit_tree();
    h.invoke("commit-tree.select", json!({ "path": "b.py" }));
    h.invoke("commit-tree.show-diff", Value::Null);
    let d = h.wait_diff("b.py");
    assert_eq!(d["files"], json!([0, 2]));
    assert_eq!(h.state()["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(d["changes"][0]["left"], json!([1, 2]));
}

#[gpui::test]
fn differences_files_and_sides_are_navigated(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
    });
    h.invoke("vcs.show-diff", json!({ "path": "b.py" }));
    h.wait_diff("b.py");
    h.invoke("diff.next-file", Value::Null);
    let d = h.wait_diff("main.py");
    assert_eq!(d["files"], json!([1, 2]));
    h.focus_diff(Side::Right);
    assert_eq!(h.right_caret(), 9);

    // F7: the next difference; at the last one the hint, then the next
    // file (there is none after main.py: nothing happens).
    h.invoke("diff.next", Value::Null);
    assert_eq!(h.right_caret(), 21);
    assert_eq!(h.diff()["left"]["caret"], json!([21, 0]));
    h.invoke("diff.next", Value::Null);
    assert_eq!(h.right_caret(), 21);
    assert_eq!(h.diff()["hint"], Value::Null);
    // Shift+F7: back to the first; at the first one the hint, then the
    // previous file at its last difference.
    h.invoke("diff.previous", Value::Null);
    assert_eq!(h.right_caret(), 9);
    h.invoke("diff.previous", Value::Null);
    assert_eq!(
        h.diff()["hint"],
        "Press again to go to the previous file",
        "{:#}",
        h.diff()
    );
    h.invoke("diff.previous", Value::Null);
    let d = h.wait_diff("b.py");
    assert_eq!(d["files"], json!([0, 2]));
    assert_eq!(d["hint"], Value::Null);
    assert_eq!(d["right"]["caret"], json!([1, 0]));

    // Select Opposite Diff Pane.
    h.focus_diff(Side::Right);
    h.invoke("diff.focus-opposite", Value::Null);
    assert_eq!(h.diff()["current_side"], "left");
    h.invoke("diff.focus-opposite", Value::Null);
    assert_eq!(h.diff()["current_side"], "right");

    // Jump to Source: the file at the caret.
    h.invoke("diff.jump-to-source", Value::Null);
    let root = h.root.clone();
    h.wait("b.py's tab", move |h| {
        let active = h.window.update(h.cx, |w, _, cx| w.active_file(cx)).unwrap();
        (active == Some(root.join("b.py"))).then_some(())
    });
    let state = h.state();
    assert_eq!(state["editor"]["caret"]["line"], 1, "{:#}", state["editor"]);
    // The diff tab stays.
    assert_eq!(state["tabs"].as_array().unwrap().len(), 2);
}

#[gpui::test]
fn next_difference_goes_on_to_the_unversioned_files(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
        write(root, "new.py", "new = 1\n");
    });
    h.wait_commit("the unversioned file", |c| {
        c["rows"]
            .as_array()
            .is_some_and(|r| r.iter().any(|r| r["group"] == "unversioned"))
    });
    h.focus_commit_tree();
    h.invoke("commit-tree.select", json!({ "path": "main.py" }));
    h.invoke("commit-tree.open", Value::Null);
    let d = h.wait_diff("main.py");
    // The counter counts the Changes; the files after them are a step away.
    assert_eq!(d["files"], json!([1, 2]));
    assert_eq!(d["files_text"], "2/2 files");
    assert_eq!(d["steps"], json!([true, true]));

    // F7 at the last difference: the hint, then the unversioned file.
    h.focus_diff(Side::Right);
    h.invoke("diff.next", Value::Null);
    assert_eq!(h.right_caret(), 21);
    h.invoke("diff.next", Value::Null);
    assert_eq!(h.diff()["hint"], "Press again to go to the next file");
    h.invoke("diff.next", Value::Null);
    let d = h.wait_diff("new.py");
    assert_eq!(d["one_side"], "right");
    assert_eq!(d["files"], json!([0, 1]));
    assert_eq!(d["files_text"], "1 file");
    assert_eq!(d["steps"], json!([true, false]));
    // The Commit tool window selects it (its group expanded).
    let commit = h.commit();
    let selected: Vec<&Value> = commit["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["selected"] == true)
        .map(|r| &r["path"])
        .collect();
    assert_eq!(selected, [&json!("new.py")]);

    // Compare Previous File: back to the last of the Changes.
    h.invoke("diff.previous-file", Value::Null);
    let d = h.wait_diff("main.py");
    assert_eq!(d["files"], json!([1, 2]));
    assert_eq!(d["steps"], json!([true, true]));
}

#[gpui::test]
fn the_gutters_arrow_reverts_a_change_as_one_edit(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
    });
    h.invoke("vcs.show-diff", json!({ "path": "main.py" }));
    let d = h.wait_diff("main.py");
    assert_eq!(d["revert"], true);
    assert_eq!(d["changes"].as_array().unwrap().len(), 2);
    // `>>` of the first change: its left lines replace the right ones.
    h.click_action(Side::Left, 0);
    let d = h.wait("one difference", |h| {
        let d = h.diff();
        (d["status"] == "1 difference").then_some(d)
    });
    assert_eq!(d["changes"][0]["kind"], "inserted");
    let changed =
        MAIN.replace("raise ValueError(value)", "raise ValueError(\"too big\")") + "# changed\n";
    assert_eq!(h.right_text(), MAIN.to_owned() + "# changed\n");
    // One undo brings it back.
    h.focus_diff(Side::Right);
    h.invoke("editor.undo", Value::Null);
    h.wait("two differences", |h| {
        (h.diff()["status"] == "2 differences").then_some(())
    });
    assert_eq!(h.right_text(), changed);
    // Accept Left Side at the caret: the change there.
    h.invoke("diff.next", Value::Null);
    assert_eq!(h.right_caret(), 21);
    h.invoke("diff.apply-left-side", Value::Null);
    h.wait("one difference", |h| {
        (h.diff()["status"] == "1 difference").then_some(())
    });
    assert_eq!(
        h.right_text(),
        MAIN.replace("raise ValueError(value)", "raise ValueError(\"too big\")")
    );
}

#[gpui::test]
fn a_change_left_out_by_its_check_box_stays_out_of_the_commit(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
    });
    h.focus_commit_tree();
    h.invoke("commit-tree.select", json!({ "path": "main.py" }));
    h.invoke("commit-tree.open", Value::Null);
    let d = h.wait_diff("main.py");
    assert_eq!(d["status"], "2 differences");
    assert_eq!(d["included"], true);
    // The second change's check box: left out, the file partly included.
    h.click_action(Side::Right, 1);
    let d = h.wait("one included", |h| {
        let d = h.diff();
        (d["status"] == "2 differences, 1 included").then_some(d)
    });
    assert_eq!(d["changes"][0]["excluded"], false);
    assert_eq!(d["changes"][1]["excluded"], true);
    let row = |c: &Value, path: &str| {
        c["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["path"] == path)
            .cloned()
            .unwrap()
    };
    let commit = h.commit();
    assert_eq!(row(&commit, "main.py")["inclusion"], "some");
    assert_eq!(row(&commit, "b.py")["inclusion"], "all");
    // Both out, then the first one in again by the command.
    h.invoke(
        "diff.include-change",
        json!({ "index": 0, "included": false }),
    );
    h.wait("none included", |h| {
        (h.diff()["status"] == "2 differences, 0 included").then_some(())
    });
    assert_eq!(row(&h.commit(), "main.py")["inclusion"], "none");
    h.invoke(
        "diff.include-change",
        json!({ "index": 0, "included": true }),
    );
    h.wait("one included", |h| {
        (h.diff()["status"] == "2 differences, 1 included").then_some(())
    });

    // The commit takes the first change of main.py and all of b.py; the
    // second change stays a local change.
    h.invoke("vcs.commit.run", json!({ "message": "Part" }));
    let root = h.root.clone();
    h.wait("the commit", move |_| {
        (git(&root, &["log", "-1", "--format=%s"]) == "Part").then_some(())
    });
    let first = MAIN.replace("raise ValueError(value)", "raise ValueError(\"too big\")");
    // (`git` trims what it prints.)
    assert_eq!(git(&h.root, &["show", "HEAD:main.py"]), first.trim_end());
    assert_eq!(git(&h.root, &["show", "HEAD:b.py"]), "b = 1\nc = 3");
    assert_eq!(
        std::fs::read_to_string(h.root.join("main.py")).unwrap(),
        first.clone() + "# changed\n"
    );
    assert_eq!(git(&h.root, &["diff", "--name-only"]), "main.py");
    assert_eq!(git(&h.root, &["diff", "--cached", "--name-only"]), "");
}

#[gpui::test]
fn edits_of_the_right_side_reach_the_file(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
    });
    // The file's tab is open: the diff edits its text, its markers follow.
    h.invoke("project.open-file", json!({ "path": "b.py" }));
    h.invoke("vcs.show-diff", json!({ "path": "b.py" }));
    let d = h.wait_diff("b.py");
    assert_eq!(d["status"], "1 difference");
    h.focus_diff(Side::Right);
    // Back to the committed text: no differences.
    h.invoke("editor.select-line-end", Value::Null);
    h.invoke("editor.type", json!({ "text": "c = 2" }));
    h.wait("no differences", |h| {
        let d = h.diff();
        (d["status"] == "No differences").then_some(())
    });
    let state = h.state();
    let tab = state["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == "b.py")
        .cloned()
        .unwrap();
    assert_eq!(tab["modified"], true);
    let root = h.root.clone();
    let text = h
        .window
        .update(h.cx, |w, _, cx| {
            w.editor_for(&root.join("b.py")).unwrap().read(cx).text()
        })
        .unwrap();
    assert_eq!(text, "b = 1\nc = 2\n");
    // Saved: the Commit tool window lists it no more.
    h.invoke("workbench.save-all", Value::Null);
    h.invoke("vcs.refresh", Value::Null);
    h.wait_commit("b.py unchanged", |c| {
        !c["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["path"] == "b.py")
    });

    // Without a tab: the diff edits the file's own document, Save All
    // writes it, and a tab opened later takes its text.
    h.invoke("vcs.show-diff", json!({ "path": "main.py" }));
    h.wait_diff("main.py");
    h.focus_diff(Side::Right);
    h.invoke("editor.move-document-start", Value::Null);
    h.invoke("editor.type", json!({ "text": "# top\n" }));
    let d = h.wait("three differences", |h| {
        let d = h.diff();
        (d["status"] == "3 differences").then_some(d)
    });
    assert_eq!(d["changes"][0]["kind"], "inserted", "{:#}", d["changes"]);
    h.invoke("workbench.save-all", Value::Null);
    let root = h.root.clone();
    h.wait("main.py written", move |_| {
        std::fs::read_to_string(root.join("main.py"))
            .unwrap()
            .contains("# top")
            .then_some(())
    });
}

#[gpui::test]
fn the_marker_popup_and_the_project_tree_open_the_diff(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        change(root);
    });
    h.invoke("project.open-file", json!({ "path": "main.py" }));
    h.wait("main.py's markers", move |h| {
        let vcs = h.invoke("vcs.state", Value::Null);
        vcs["open_files"][0]["markers"]["ranges"]
            .as_array()
            .is_some_and(|r| r.len() == 2)
            .then_some(())
    });
    h.invoke("editor.go-to-line", json!({ "line": 22 }));
    h.invoke("vcs.show-current-change", Value::Null);
    let popup = h.invoke("vcs.state", Value::Null)["marker_popup"].clone();
    assert_eq!(
        popup["actions"][3]["enabled"], true,
        "Show Diff is enabled: {popup:#}"
    );
    h.invoke("vcs.show-diff-lines", Value::Null);
    let d = h.wait_diff("main.py");
    // At the popup's change.
    assert_eq!(d["right"]["caret"], json!([21, 0]));
    assert!(h.invoke("vcs.state", Value::Null)["marker_popup"].is_null());

    // The project tool window's selection.
    h.invoke("project.tree.reveal", json!({ "path": "b.py" }));
    h.focus(|w, cx| w.panel().focus_handle(cx));
    h.invoke("vcs.show-diff", Value::Null);
    h.wait_diff("b.py");
}

#[gpui::test]
fn whitespace_is_ignored_on_request(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root);
        write(root, "b.py", "b  =  1\nc = 2\n");
        write(root, "main.py", MAIN);
    });
    h.invoke("vcs.show-diff", json!({ "path": "b.py" }));
    let d = h.wait_diff("b.py");
    assert_eq!(d["status"], "1 difference");
    assert_eq!(d["settings"]["ignore"], "none");
    h.invoke("diff.set-ignore-policy", json!({ "policy": "whitespace" }));
    let d = h.wait("no differences", |h| {
        let d = h.diff();
        (d["computed"] == true && d["status"] == "No differences").then_some(d)
    });
    assert_eq!(d["settings"]["ignore"], "whitespace");
    h.invoke("diff.settings", Value::Null);
    assert_eq!(h.diff()["settings"]["open"], true);
}
