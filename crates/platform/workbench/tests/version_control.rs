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
    assert_eq!(
        vcs["open_files"],
        json!([{"path": "a.py", "status": "not_changed"}])
    );

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
