//! The Commit tool window (#170) against repositories made with the git
//! executable (scenarios `vcs/commit-basic.md`, `commit-edge.md`,
//! `amend.md`, `rollback.md`): ⌘K, the tree with its check boxes and
//! legend, commit of the included files only, the errors, unversioned
//! files, "Git User Name Is Not Defined", a failing hook and Run Git
//! hooks, "Commit in Detached HEAD", Amend with the message before it, the
//! Rollback Changes dialog, Add to VCS and the draft kept in the personal
//! state. Every git call of this test binary reads a global config file
//! of its own.

use crate::support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, SyncSettings, Workbench};

/// The global git config of the test binary (empty: no identity).
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

/// A repository on `main` with `a.py`, `b.py`, `c.py` and `notes.txt` in
/// its first commit "initial"; its own identity unless `identity` is
/// false.
fn repository(root: &Path, identity: bool) {
    global_config();
    git::testing::init(root);
    for (key, value) in [("commit.gpgsign", "false"), ("core.fsmonitor", "false")] {
        git(root, &["config", key, value]);
    }
    for f in ["a.py", "b.py", "c.py", "notes.txt"] {
        write(root, f, &format!("{f}\n"));
    }
    git(root, &["add", "."]);
    git(
        root,
        &[
            "-c",
            "user.name=Setup",
            "-c",
            "user.email=setup@example.invalid",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    );
    if identity {
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "user.email", "test@example.invalid"]);
    }
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

    fn commit(&mut self) -> Value {
        self.invoke("vcs.state", Value::Null)["commit"].clone()
    }

    fn timeline(&mut self) -> Vec<Value> {
        self.invoke("workbench.state", Value::Null)["notifications"]["timeline"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            self.draw();
            let commit = self.commit();
            if f(&commit) {
                return commit;
            }
        }
        let state = self.commit();
        let on_disk = git(&self.root, &["status", "--porcelain", "-uall"]);
        let store: Vec<_> = std::fs::read_dir(self.root.join(".oxide"))
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        panic!(
            "timed out waiting for {what}: {state:#}\ngit status:\n{on_disk}\n.oxide: {store:?}"
        );
    }

    /// Waits for a notification whose title is `title`.
    fn wait_notification(&mut self, title: &str) -> Value {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            if let Some(n) = self.timeline().into_iter().find(|n| n["title"] == title) {
                return n;
            }
        }
        panic!("no notification {title:?}: {:#?}", self.timeline());
    }

    fn message_focused(&mut self) -> bool {
        self.window
            .update(self.cx, |w, window, cx| {
                w.commit_view()
                    .read(cx)
                    .message_focus(cx)
                    .is_focused(window)
            })
            .unwrap()
    }

    fn focus_tree(&mut self) {
        self.window
            .update(self.cx, |w, window, cx| {
                let focus = w.commit_view().read(cx).tree_focus();
                window.focus(&focus);
            })
            .unwrap();
        self.draw();
    }
}

/// The rows as short texts: `v Changes 3 files [all]`, `main.py [x]`.
fn rows(commit: &Value) -> Vec<String> {
    commit["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| match r["kind"].as_str().unwrap() {
            "group" => format!(
                "{} {} {} [{}]",
                if r["expanded"] == true { "v" } else { ">" },
                r["title"].as_str().unwrap(),
                r["count"].as_str().unwrap(),
                r["included"].as_str().unwrap()
            ),
            _ => format!(
                "{}{} [{}]",
                r["path"].as_str().unwrap(),
                r["origin"]
                    .as_str()
                    .map(|o| format!(" {o}"))
                    .unwrap_or_default(),
                if r["included"] == true { "x" } else { " " }
            ),
        })
        .collect()
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
    h.wait("the repository", |c| c["state"] == "ready");
    (h, dir)
}

#[gpui::test]
fn a_commit_takes_the_included_files(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root, true);
        write(root, "a.py", "a changed\n");
        write(root, "b.py", "b changed\n");
        write(root, "c.py", "c staged\n");
        git(root, &["add", "c.py"]);
        write(root, "new.py", "new\n");
    });
    let commit = h.wait("the changes", |c| c["rows"].as_array().unwrap().len() == 5);
    assert_eq!(commit["visible"], false);

    h.invoke("vcs.commit", Value::Null);
    let commit = h.commit();
    assert_eq!(commit["visible"], true);
    assert!(h.message_focused(), "Commit… focuses the message");
    assert_eq!(
        rows(&commit),
        vec![
            "v Changes 3 files [all]",
            "a.py [x]",
            "b.py [x]",
            "c.py [x]",
            "> Unversioned Files 1 file [none]",
        ]
    );
    assert_eq!(commit["legend"], "3 modified");
    assert_eq!(commit["legend_compact"], "*3");
    assert_eq!(commit["placeholder"], "Commit Message");
    assert_eq!(commit["commit_button"], "Commit");
    assert_eq!(commit["push_button"], "Commit and Push\u{2026}");
    assert_eq!(commit["rows"][0]["selected"], true);

    // c.py unticked: its staged change stays staged.
    h.invoke(
        "vcs.commit.include",
        json!({ "paths": ["c.py"], "included": false }),
    );
    let commit = h.commit();
    assert_eq!(commit["rows"][0]["included"], "some");
    assert_eq!(commit["legend"], "2 modified");

    h.invoke("vcs.commit.run", json!({ "message": "Fix" }));
    let n = h.wait_notification("2 files committed");
    assert_eq!(n["content"], "Fix");
    assert_eq!(n["kind"], "information");
    // The recent messages are kept in `.oxide/workspace.toml`, which its
    // new `.oxide/.gitignore` ignores (the ignore file is unversioned).
    let commit = h.wait("the committed files to leave", |c| {
        c["rows"].as_array().unwrap().len() == 3 && c["rows"][2]["count"] == "2 files"
    });
    assert_eq!(commit["message"], "");
    assert_eq!(
        rows(&commit),
        vec![
            "v Changes 1 file [none]",
            "c.py [ ]",
            "> Unversioned Files 2 files [none]",
        ]
    );
    assert_eq!(git(&h.root, &["log", "-1", "--format=%B"]), "Fix");
    assert_eq!(
        git(&h.root, &["show", "--name-only", "--format=", "HEAD"]),
        "a.py\nb.py"
    );
    assert_eq!(git(&h.root, &["diff", "--cached", "--name-only"]), "c.py");
    // The message is a recent one now.
    let recent = project::workspace::load_list(&h.root, "vcs", "recent-commit-messages").unwrap();
    assert_eq!(recent, vec!["Fix".to_owned()]);
}

#[gpui::test]
fn renames_errors_and_unversioned_files(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root, true);
        git(root, &["mv", "notes.txt", "notes.md"]);
        write(root, "a.py", "a changed\n");
        write(root, "pkg/new.py", "new\n");
    });
    h.wait("the changes", |c| c["rows"].as_array().unwrap().len() == 4);
    h.invoke("vcs.commit", Value::Null);
    let commit = h.commit();
    assert_eq!(
        rows(&commit)[2],
        "notes.md - renamed from notes.txt [x]",
        "{:#}",
        commit["rows"]
    );
    assert_eq!(
        commit["rows"][2]["parent"],
        h.root.file_name().unwrap().to_str().unwrap()
    );

    // Nothing included.
    h.invoke("vcs.commit.include", json!({ "included": false }));
    h.invoke("vcs.commit.run", json!({ "message": "" }));
    assert_eq!(
        h.commit()["problem"],
        "Select files to commit and specify commit message"
    );
    h.invoke("vcs.commit.run", json!({ "message": "Fix" }));
    assert_eq!(h.commit()["problem"], "Select files to commit");
    // Ticking a file clears it; an empty message is the next problem.
    h.invoke(
        "vcs.commit.include",
        json!({ "paths": ["a.py"], "included": true }),
    );
    assert_eq!(h.commit()["problem"], Value::Null);
    h.invoke("vcs.commit.run", json!({ "message": "  " }));
    assert_eq!(h.commit()["problem"], "Specify commit message");
    assert_eq!(git(&h.root, &["log", "-1", "--format=%s"]), "initial");

    // Only the unversioned file: added and committed.
    h.invoke(
        "vcs.commit.include",
        json!({ "paths": ["a.py"], "included": false }),
    );
    h.invoke("commit-tree.select", json!({ "path": "unversioned" }));
    h.invoke("commit-tree.right", Value::Null);
    h.invoke(
        "vcs.commit.include",
        json!({ "paths": ["pkg/new.py"], "included": true }),
    );
    assert_eq!(h.commit()["legend"], "1 added");
    h.invoke("vcs.commit.run", json!({ "message": "Add new" }));
    h.wait_notification("1 file committed");
    assert_eq!(
        git(&h.root, &["show", "--name-only", "--format=", "HEAD"]),
        "pkg/new.py"
    );
}

#[gpui::test]
fn an_undefined_user_name_is_asked_for(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root, false);
        write(root, "a.py", "a changed\n");
    });
    h.wait("the changes", |c| c["rows"].as_array().unwrap().len() == 2);
    h.invoke("vcs.commit", Value::Null);
    h.invoke("vcs.commit.run", json!({ "message": "Who" }));
    let commit = h.wait("the dialog", |c| c["dialog"]["kind"] == "user_name");
    let dialog = &commit["dialog"];
    assert_eq!(dialog["title"], "Git User Name Is Not Defined");
    assert_eq!(dialog["ok"], "Set and Commit");
    assert_eq!(dialog["global"], false);
    h.invoke(
        "user-name-dialog.set",
        json!({ "name": "Someone", "email": "someone" }),
    );
    h.invoke("user-name-dialog.confirm", Value::Null);
    assert_eq!(
        h.commit()["dialog"]["error"],
        "The email address must contain the '@' symbol"
    );
    h.invoke(
        "user-name-dialog.set",
        json!({ "email": "someone@example.invalid" }),
    );
    h.invoke("user-name-dialog.confirm", Value::Null);
    h.wait_notification("1 file committed");
    assert_eq!(h.commit()["dialog"], Value::Null);
    assert_eq!(
        git(&h.root, &["log", "-1", "--format=%an <%ae>"]),
        "Someone <someone@example.invalid>"
    );
    assert_eq!(git(&h.root, &["config", "--local", "user.name"]), "Someone");
}

#[cfg(unix)]
#[gpui::test]
fn a_failing_hook_stops_the_commit_until_hooks_are_off(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (mut h, _dir) = open(cx, |root| {
        repository(root, true);
        let hook = root.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho 'the hook says no'\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        write(root, "a.py", "a changed\n");
    });
    h.wait("the hooks", |c| {
        c["options"]["hooks"] == true && c["rows"].as_array().unwrap().len() == 2
    });
    h.invoke("vcs.commit", Value::Null);
    h.invoke("vcs.commit.run", json!({ "message": "Hooked" }));
    let n = h.wait_notification("Commit failed with error");
    assert!(
        n["content"].as_str().unwrap().contains("the hook says no"),
        "{n:#}"
    );
    assert_eq!(h.commit()["busy"], false);
    assert_eq!(h.commit()["message"], "Hooked");

    h.invoke("vcs.commit.options", Value::Null);
    assert_eq!(h.commit()["options"]["open"], true);
    h.invoke("commit-options.toggle-run-hooks", Value::Null);
    h.invoke("commit-options.close", Value::Null);
    let options = h.commit()["options"].clone();
    assert_eq!(options["open"], false);
    assert_eq!(options["run_hooks"], false);
    h.invoke("vcs.commit.run", Value::Null);
    h.wait_notification("1 file committed");
    assert_eq!(git(&h.root, &["log", "-1", "--format=%s"]), "Hooked");
}

#[gpui::test]
fn a_detached_head_is_warned_about(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root, true);
        git(root, &["checkout", "-q", "--detach"]);
        write(root, "a.py", "a changed\n");
    });
    h.wait("the changes", |c| c["rows"].as_array().unwrap().len() == 2);
    h.invoke("vcs.commit", Value::Null);
    h.invoke("vcs.commit.run", json!({ "message": "Detached" }));
    let commit = h.wait("the warning", |c| c["dialog"]["kind"] == "detached");
    assert_eq!(commit["dialog"]["title"], "Commit in Detached HEAD");
    assert_eq!(commit["dialog"]["buttons"], json!(["Cancel", "Commit"]));
    // Cancelled: nothing is committed and the panel says why.
    h.invoke("dialog.cancel", Value::Null);
    let commit = h.commit();
    assert_eq!(commit["dialog"], Value::Null);
    assert!(
        commit["problem"]
            .as_str()
            .unwrap()
            .starts_with("The repository is in the detached HEAD state: "),
        "{commit:#}"
    );
    h.invoke("vcs.commit.run", Value::Null);
    h.wait("the warning", |c| c["dialog"]["kind"] == "detached");
    h.invoke("dialog.choose", json!({ "button": "Commit" }));
    h.wait_notification("1 file committed");
    assert_eq!(git(&h.root, &["log", "-1", "--format=%s"]), "Detached");
}

#[gpui::test]
fn amend_takes_the_last_message_and_gives_the_draft_back(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root, true);
        write(root, "a.py", "first\n");
        git(root, &["commit", "-q", "-am", "First"]);
        write(root, "b.py", "b changed\n");
        write(root, "c.py", "c changed\n");
    });
    h.wait("the last commit", |c| {
        c["last_commit"] == "First" && c["rows"].as_array().unwrap().len() == 3
    });
    let parent = git(&h.root, &["rev-parse", "HEAD~1"]);
    h.invoke("vcs.commit", Value::Null);
    h.invoke("vcs.commit.set-message", json!({ "text": "draft" }));
    h.invoke("vcs.commit.toggle-amend", Value::Null);
    let commit = h.wait("the amend message", |c| c["message"] == "First");
    assert_eq!(commit["amend"], true);
    assert_eq!(commit["commit_button"], "Amend Commit");
    assert_eq!(commit["push_button"], "Amend Commit and Push\u{2026}");
    h.invoke("vcs.commit.toggle-amend", Value::Null);
    let commit = h.commit();
    assert_eq!(commit["amend"], false);
    assert_eq!(commit["message"], "draft");

    h.invoke("vcs.commit.toggle-amend", Value::Null);
    h.wait("the amend message", |c| c["message"] == "First");
    h.invoke(
        "vcs.commit.include",
        json!({ "paths": ["c.py"], "included": false }),
    );
    h.invoke("vcs.commit.run", json!({ "message": "First, amended" }));
    h.wait_notification("1 file committed");
    let commit = h.wait("amend off", |c| c["amend"] == false && c["message"] == "");
    assert_eq!(commit["commit_button"], "Commit");
    assert_eq!(
        git(&h.root, &["log", "-1", "--format=%B"]),
        "First, amended"
    );
    assert_eq!(git(&h.root, &["rev-parse", "HEAD~1"]), parent);
    assert_eq!(
        git(&h.root, &["show", "--name-only", "--format=", "HEAD"]),
        "a.py\nb.py"
    );
    // The draft before Amend went to the recent messages.
    let recent = project::workspace::load_list(&h.root, "vcs", "recent-commit-messages").unwrap();
    assert_eq!(
        recent,
        vec!["First, amended".to_owned(), "draft".to_owned()]
    );
}

#[gpui::test]
fn rollback_reverts_unindexes_and_deletes(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        repository(root, true);
        write(root, "a.py", "a changed\n");
        write(root, "new.py", "new\n");
        git(root, &["add", "new.py"]);
    });
    h.invoke("project.open-file", json!({ "path": "a.py" }));
    h.wait("the changes", |c| c["rows"].as_array().unwrap().len() == 3);
    h.invoke("workbench.toggle-commit", Value::Null);
    h.focus_tree();
    // The group stands for its files.
    h.invoke("commit-tree.select", json!({ "path": "changes" }));
    h.invoke("vcs.rollback", Value::Null);
    let commit = h.wait("the dialog", |c| c["dialog"]["kind"] == "rollback");
    let dialog = &commit["dialog"];
    assert_eq!(dialog["title"], "Rollback Changes");
    assert_eq!(
        dialog["files"],
        json!([
            { "path": "a.py", "status": "modified", "included": true },
            { "path": "new.py", "status": "added", "included": true },
        ])
    );
    assert_eq!(dialog["legend"], "1 added   1 modified");
    assert_eq!(dialog["delete_added"], false);
    assert_eq!(dialog["delete_added_enabled"], true);
    h.invoke("rollback-dialog.confirm", Value::Null);
    h.wait("the rollback", |c| {
        c["dialog"].is_null()
            && c["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "group" && r["title"] == "Unversioned Files")
            && !c["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["title"] == "Changes")
    });
    assert_eq!(
        std::fs::read_to_string(h.root.join("a.py")).unwrap(),
        "a.py\n"
    );
    assert!(h.root.join("new.py").exists());
    // The open editor has the committed text again.
    let start = Instant::now();
    loop {
        let text = h
            .window
            .update(h.cx, |w, _, cx| {
                w.active_editor().map(|e| e.read(cx).text())
            })
            .unwrap();
        if text.as_deref() == Some("a.py\n") {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "editor: {text:?}"
        );
        support::tick(h.cx);
    }

    // Added again, rolled back with its local copy deleted.
    h.invoke("vcs.add", json!({ "paths": ["new.py"] }));
    h.wait("new.py added", |c| {
        c["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["path"] == "new.py" && r["status"] == "added")
    });
    h.invoke(
        "vcs.rollback",
        json!({ "paths": ["new.py"], "delete_added": true }),
    );
    h.wait("no changes", |c| c["empty_text"] == "No differences");
    assert!(!h.root.join("new.py").exists());
    assert_eq!(git(&h.root, &["status", "--porcelain"]), "");
}

#[gpui::test]
fn the_draft_is_kept_and_a_folder_without_git_offers_one(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| repository(root, true));
    let commit = h.commit();
    assert_eq!(commit["empty_text"], "No differences");
    h.invoke(
        "vcs.commit.set-message",
        json!({ "text": "Work in progress" }),
    );
    let start = Instant::now();
    loop {
        let draft = project::workspace::load_value(&h.root, "vcs", "commit-message").unwrap();
        if draft.as_deref() == Some("Work in progress") {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "draft: {draft:?}"
        );
        support::tick(h.cx);
    }

    let other = tempfile::tempdir().unwrap();
    let path = native_path::canonicalize(other.path()).unwrap();
    write(&path, "x.py", "x\n");
    h.invoke(
        "project.open-folder",
        json!({ "path": path.display().to_string() }),
    );
    let commit = h.wait("no repository", |c| c["state"] == "no_repository");
    assert_eq!(commit["message"], "");
}
