//! The project's Git repository (ADR 0016): found when the folder opens,
//! its statuses in `workbench.state`, an unsaved edit of a clean tracked
//! file modified at once, changes made outside (the file watcher, `.git`
//! included) followed, and a project mapped to no version control in the
//! reference's `.idea/vcs.xml` left alone. The repositories are made with
//! the git executable; the tests wait for real processes and events.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, SyncSettings, Workbench};

fn git(dir: &Path, args: &[&str]) {
    let exe = git::GitExe::find(None).expect("git is installed");
    let out = Command::new(exe.path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
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
            .unwrap();
        self.cx.run_until_parked();
        out
    }

    fn vcs(&mut self) -> Value {
        self.invoke("workbench.state", Value::Null)["vcs"].clone()
    }

    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            let vcs = self.vcs();
            if f(&vcs) {
                return vcs;
            }
        }
        panic!("timed out waiting for {what}: {}", self.vcs());
    }
}

/// A repository with a committed `a.py` and an unversioned `b.py`,
/// opened as the project with `a.py` in a tab.
fn open(cx: &mut TestAppContext, prepare: impl FnOnce(&Path)) -> (H<'_>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.py"), "a = 1\n").unwrap();
    git(&root, &["add", "a.py"]);
    git(&root, &["commit", "-q", "-m", "first"]);
    std::fs::write(root.join("b.py"), "b = 1\n").unwrap();
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
    h.invoke("project.open-file", json!({ "path": "a.py" }));
    (h, dir)
}

#[gpui::test]
fn statuses_follow_edits_and_changes_outside(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |_| {});
    let vcs = h.wait("the first status", |v| !v["status"].is_null());
    assert_eq!(vcs["vcs"], "Git");
    assert_eq!(vcs["status"]["repository"]["branch"], "main");
    assert_eq!(vcs["status"]["unversioned"], json!(["b.py"]));
    assert_eq!(vcs["open_files"][0]["path"], "a.py");
    assert_eq!(vcs["open_files"][0]["status"], "not_changed");

    // Typing in the clean tracked file: modified at once, before any
    // save or status.
    h.invoke("workbench.focus-editor", Value::Null);
    h.cx.simulate_input(h.window.into(), "x");
    assert_eq!(h.vcs()["open_files"][0]["status"], "modified");

    // `git add` outside the IDE: the index changes, the watcher passes it
    // on, b.py turns added.
    git(&h.root, &["add", "b.py"]);
    let vcs = h.wait("b.py to be added", |v| {
        v["status"]["changes"].as_array().is_some_and(|c| {
            c.iter()
                .any(|c| c["path"] == "b.py" && c["status"] == "added")
        })
    });
    assert_eq!(vcs["status"]["unversioned"], json!([]));

    // A new file in the project: unversioned.
    std::fs::write(h.root.join("c.py"), "c = 1\n").unwrap();
    h.wait("c.py to be unversioned", |v| {
        v["status"]["unversioned"] == json!(["c.py"])
    });
}

#[gpui::test]
fn a_project_mapped_to_no_version_control_keeps_git_off(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        std::fs::create_dir(root.join(".idea")).unwrap();
        std::fs::write(
            root.join(".idea").join("vcs.xml"),
            r#"<project version="4"><component name="VcsDirectoryMappings"><mapping directory="" vcs="" /></component></project>"#,
        )
        .unwrap();
    });
    h.wait("the mapping to be read", |v| v["off"] == true);
    assert!(h.vcs()["status"].is_null());
}

// ---- change markers and status colours (#169) ---------------------------

/// Ten committed lines in `m.py`, an ignored `logs/` folder with a file
/// and an ignored `.env`.
fn prepare_markers(root: &Path) {
    let text: String = (0..10).map(|i| format!("line_{i} = {i}\n")).collect();
    std::fs::write(root.join("m.py"), text).unwrap();
    std::fs::write(root.join(".gitignore"), "logs/\n.env\n").unwrap();
    std::fs::create_dir(root.join("logs")).unwrap();
    std::fs::write(root.join("logs").join("app.log"), "x\n").unwrap();
    std::fs::write(root.join(".env"), "A=1\n").unwrap();
    git(root, &["add", "m.py", ".gitignore"]);
    git(root, &["commit", "-q", "-m", "second"]);
}

/// The `workbench.state` entry of an open file.
fn open_file(vcs: &Value, path: &str) -> Value {
    vcs["open_files"]
        .as_array()
        .and_then(|files| files.iter().find(|f| f["path"] == path).cloned())
        .unwrap_or(Value::Null)
}

/// The kinds and lines of a file's changed ranges, once computed.
fn ranges(vcs: &Value, path: &str) -> Option<Vec<(String, u64, u64)>> {
    let markers = &open_file(vcs, path)["markers"];
    if markers["computing"] != false {
        return None;
    }
    Some(
        markers["ranges"]
            .as_array()?
            .iter()
            .map(|r| {
                (
                    r["kind"].as_str().unwrap().to_owned(),
                    r["lines"][0].as_u64().unwrap(),
                    r["lines"][1].as_u64().unwrap(),
                )
            })
            .collect(),
    )
}

fn kinds(list: &[(&str, u64, u64)]) -> Vec<(String, u64, u64)> {
    list.iter()
        .map(|(k, a, b)| ((*k).to_owned(), *a, *b))
        .collect()
}

impl H<'_> {
    fn caret(&mut self, line: usize, column: usize) {
        let at = json!({ "line": line, "column": column });
        self.invoke("editor.set-selection", json!({ "anchor": at, "head": at }));
    }

    fn caret_line(&mut self) -> u64 {
        self.invoke("editor.state", Value::Null)["caret"]["line"]
            .as_u64()
            .unwrap()
    }

    fn popup(&mut self) -> Value {
        self.vcs()["marker_popup"].clone()
    }

    fn tree_row(&mut self, path: &str) -> Value {
        let state = self.invoke("workbench.state", Value::Null);
        state["tree"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["path"] == path)
            .cloned()
            .unwrap_or_else(|| panic!("no row {path}: {}", state["tree"]["rows"]))
    }

    fn tab_color(&mut self, path: &str) -> Value {
        let state = self.invoke("workbench.state", Value::Null);
        state["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["path"] == path)
            .map(|t| t["color"].clone())
            .unwrap()
    }
}

#[gpui::test]
fn change_markers_follow_edits_and_go_back(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, prepare_markers);
    h.invoke("project.open-file", json!({ "path": "m.py" }));
    h.wait("the base text", |v| {
        open_file(v, "m.py")["markers"]["base"] == true && ranges(v, "m.py").is_some()
    });
    assert_eq!(ranges(&h.vcs(), "m.py"), Some(Vec::new()));

    // Line 3 changed, two lines inserted after line 5, line 9 deleted.
    h.invoke("workbench.focus-editor", Value::Null);
    h.invoke(
        "editor.set-selection",
        json!({"anchor": {"line": 3, "column": 0}, "head": {"line": 3, "column": 6}}),
    );
    h.invoke("editor.type", json!({ "text": "LINE_3" }));
    h.caret(5, 10);
    h.invoke("editor.type", json!({ "text": "\nnew_a = 0\nnew_b = 0" }));
    h.caret(11, 0);
    h.invoke("editor.delete-line", Value::Null);
    let expected = kinds(&[("modified", 3, 4), ("added", 6, 8), ("deleted", 11, 11)]);
    let vcs = h.wait("three ranges", |v| {
        ranges(v, "m.py") == Some(expected.clone())
    });
    assert_eq!(open_file(&vcs, "m.py")["status"], "modified");

    // The editor draws them: a marker per range, a thin stripe mark.
    let editor = h.invoke("editor.state", Value::Null);
    let markers: Vec<(String, Value, String)> = editor["decorations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["group"] == "vcs")
        .map(|d| {
            (
                d["change"].as_str().unwrap().to_owned(),
                d["lines"].clone(),
                d["stripe"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        markers,
        vec![
            ("modified".to_owned(), json!([3, 4]), "thin".to_owned()),
            ("added".to_owned(), json!([6, 8]), "thin".to_owned()),
            ("deleted".to_owned(), json!([11, 11]), "thin".to_owned()),
        ]
    );

    // Next Change from the top: the caret goes to each change and its
    // popup shows the committed text with the changed words.
    h.caret(0, 0);
    h.invoke("vcs.next-change", Value::Null);
    assert_eq!(h.caret_line(), 3);
    let popup = h.popup();
    assert_eq!(popup["lines"], json!([3, 4]));
    assert_eq!(popup["text"], json!(["line_3 = 3"]));
    assert_eq!(
        popup["words"],
        json!([[{"text": "line_3", "kind": "modified"}]])
    );
    assert_eq!(popup["actions"][0]["enabled"], false, "no previous change");
    h.invoke("vcs.next-change", Value::Null);
    assert_eq!(h.caret_line(), 7);
    let popup = h.popup();
    assert_eq!(popup["text"], json!([]), "added lines have no old text");
    assert_eq!(popup["actions"][4]["title"], "Copy");
    assert_eq!(popup["actions"][4]["enabled"], false);
    h.invoke("vcs.next-change", Value::Null);
    assert_eq!(h.caret_line(), 11);
    assert_eq!(h.popup()["text"], json!(["line_9 = 9"]));
    assert_eq!(h.popup()["actions"][1]["enabled"], false, "no next change");
    h.invoke("vcs.previous-change", Value::Null);
    assert_eq!(h.caret_line(), 7);

    // Rollback Lines of the popup's change, as one undoable edit; the
    // popup closes with the change of the text.
    h.invoke("vcs.rollback-lines", Value::Null);
    let after = kinds(&[("modified", 3, 4), ("deleted", 9, 9)]);
    h.wait("the added lines rolled back", |v| {
        ranges(v, "m.py") == Some(after.clone())
    });
    assert!(h.popup().is_null());
    h.invoke("editor.undo", Value::Null);
    h.wait("the added lines back", |v| {
        ranges(v, "m.py") == Some(expected.clone())
    });

    // Copy puts the old text of the change on the clipboard.
    h.caret(3, 0);
    h.invoke("vcs.copy-change-lines", Value::Null);
    let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("line_3 = 3\n"));

    // Rollback Lines at the caret; then everything: the file equals its
    // committed text and is not modified.
    h.invoke("vcs.rollback-lines", Value::Null);
    h.invoke("editor.select-all", Value::Null);
    h.invoke("vcs.rollback-lines", Value::Null);
    let vcs = h.wait("no changes", |v| ranges(v, "m.py") == Some(Vec::new()));
    assert_eq!(open_file(&vcs, "m.py")["status"], "not_changed");
    let text = h.invoke("editor.text", Value::Null);
    let committed: String = (0..10).map(|i| format!("line_{i} = {i}\n")).collect();
    assert_eq!(text, json!(committed));

    // An indented line: the whitespace colour.
    h.caret(2, 0);
    h.invoke("editor.type", json!({ "text": "    " }));
    let vcs = h.wait("a whitespace change", |v| {
        ranges(v, "m.py") == Some(kinds(&[("modified", 2, 3)]))
    });
    assert_eq!(
        open_file(&vcs, "m.py")["markers"]["ranges"][0]["parts"],
        json!([{"lines": [2, 3], "kind": "whitespace"}])
    );
}

#[gpui::test]
fn names_have_the_colour_of_their_status(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, prepare_markers);
    h.wait("the first status", |v| !v["status"].is_null());
    h.wait("the base text", |v| {
        open_file(v, "a.py")["markers"]["base"] == true
    });
    assert_eq!(h.tree_row("b.py")["color"], "git-untracked");
    assert_eq!(h.tree_row(".env")["color"], "git-ignored");
    // An ignored folder is coloured; others are not.
    assert_eq!(h.tree_row("logs")["color"], "git-ignored");
    assert_eq!(h.tree_row("a.py")["color"], Value::Null);
    assert_eq!(h.tab_color("a.py"), Value::Null);

    // Edited: modified at once, in the tree and the tab.
    h.invoke("workbench.focus-editor", Value::Null);
    h.cx.simulate_input(h.window.into(), "x");
    assert_eq!(h.tab_color("a.py"), "git-modified");
    assert_eq!(h.tree_row("a.py")["color"], "git-modified");
    // Back to the committed text without saving: not modified.
    h.invoke("editor.backspace", Value::Null);
    h.wait("the committed text again", |v| {
        open_file(v, "a.py")["status"] == "not_changed"
    });
    assert_eq!(h.tab_color("a.py"), Value::Null);

    // `git add`: added.
    git(&h.root, &["add", "b.py"]);
    h.wait("b.py to be added", |v| {
        v["status"]["changes"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c["path"] == "b.py"))
    });
    assert_eq!(h.tree_row("b.py")["color"], "git-added");
}

/// A measurement for `replica/perf.md` (the budget of
/// `docs/performance.md`), not run by default:
/// `cargo test -p workbench --release --test version_control perf -- --ignored --nocapture`.
#[gpui::test]
#[ignore]
fn perf_markers_after_a_keystroke(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        let text: String = (0..10_000)
            .map(|i| format!("    value_{i} = compute({i}, \"text\")  # line {i}\n"))
            .collect();
        std::fs::write(root.join("big.py"), text).unwrap();
        git(root, &["add", "big.py"]);
        git(root, &["commit", "-q", "-m", "big"]);
    });
    h.invoke("project.open-file", json!({ "path": "big.py" }));
    h.wait("the base text", |v| {
        open_file(v, "big.py")["markers"]["base"] == true && ranges(v, "big.py").is_some()
    });
    h.invoke("workbench.focus-editor", Value::Null);
    let path = h.root.join("big.py");
    let status = h
        .window
        .update(h.cx, |w, _, _| w.vcs_line_status(&path))
        .unwrap()
        .expect("markers of big.py");
    let mut wall = Vec::new();
    let mut measured = Vec::new();
    for i in 0..9 {
        h.caret(1000 + i * 900, 4);
        let start = Instant::now();
        h.cx.simulate_input(h.window.into(), "x");
        h.cx.run_until_parked();
        wall.push(start.elapsed());
        let (computing, latency) =
            h.cx.update(|cx| (status.read(cx).is_computing(), status.read(cx).latency()));
        assert!(!computing);
        measured.push(latency.expect("ranges computed after the edit"));
    }
    wall.sort();
    measured.sort();
    let count = h.cx.update(|cx| status.read(cx).ranges().len());
    eprintln!(
        "markers after a keystroke, 10 000 lines, {count} ranges: keystroke to markers {:?} (median of 9), edit to ranges drawn {:?}",
        wall[4], measured[4]
    );
    assert!(measured[4] < Duration::from_millis(50));
}
