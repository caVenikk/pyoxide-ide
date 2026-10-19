//! Push, Update Project, Fetch and Pull (#172) against a local bare
//! remote made with the git executable: the Push dialog and its balloons,
//! "Push Rejected" with Merge and Cancel, a new branch and its upstream,
//! "Force Push", the Update Project dialog with the local changes stashed
//! around the update, conflicts, Fetch with pruning, Pull, a branch's
//! Update, the branches popup's remote actions and the VCS Operations
//! popup. The tests wait for real processes and events.

use crate::support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, SyncSettings, Workbench};

fn git(dir: &Path, args: &[&str]) -> String {
    let exe = git::GitExe::find(None).expect("git is installed");
    let out = Command::new(exe.path)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn identity(dir: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("pull.rebase", "false"),
    ] {
        git(dir, &["config", key, value]);
    }
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// A project `demo` on `main` tracking `origin/main` of the bare
/// `demo-remote.git` beside it, and `other`, a clone that moves the
/// remote ahead.
struct Repos {
    _dir: tempfile::TempDir,
    root: PathBuf,
    remote: PathBuf,
    other: PathBuf,
}

fn repos() -> Repos {
    let dir = tempfile::tempdir().unwrap();
    let base = native_path::canonicalize(dir.path()).unwrap();
    let root = base.join("demo");
    let remote = base.join("demo-remote.git");
    let other = base.join("other");
    std::fs::create_dir_all(&root).unwrap();
    git(
        &base,
        &["init", "-q", "--bare", "-b", "main", "demo-remote.git"],
    );
    git(&root, &["init", "-q", "-b", "main"]);
    identity(&root);
    commit(&root, "a.py", "1\n2\n3\n", "first");
    commit(&root, "b.py", "b\n", "second");
    // git reads a verbatim `\\?\C:\...` URL (what `canonicalize` returns on
    // Windows) as a host name.
    let url = run_config::paths::simplified(remote.to_str().unwrap());
    git(&root, &["remote", "add", "origin", &url]);
    git(&root, &["push", "-q", "-u", "origin", "main"]);
    // The pin is part of the clone: the checkout must not convert line
    // endings (Git for Windows defaults to `autocrlf=true`).
    git(
        &base,
        &["clone", "-q", "-c", "core.autocrlf=false", &url, "other"],
    );
    identity(&other);
    Repos {
        _dir: dir,
        root,
        remote,
        other,
    }
}

/// `other` commits `text` to `file` and pushes it.
fn remote_commit(repos: &Repos, file: &str, text: &str, message: &str) {
    git(
        &repos.other,
        &["pull", "-q", "--no-rebase", "origin", "main"],
    );
    commit(&repos.other, file, text, message);
    git(&repos.other, &["push", "-q", "origin", "main"]);
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
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

    fn vcs(&mut self) -> Value {
        self.invoke("vcs.state", Value::Null)
    }

    fn remote(&mut self) -> Value {
        self.vcs()["remote"].clone()
    }

    /// The balloons as `kind: title | content`.
    fn balloons(&mut self) -> Vec<String> {
        self.invoke("workbench.state", Value::Null)["notifications"]["balloons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                format!(
                    "{}: {} | {}",
                    b["kind"].as_str().unwrap_or_default(),
                    b["title"].as_str().unwrap_or_default(),
                    b["content"].as_str().unwrap_or_default()
                )
            })
            .collect()
    }

    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            self.draw();
            let vcs = self.vcs();
            if f(&vcs) {
                return vcs;
            }
        }
        panic!("timed out waiting for {what}: {:#}", self.vcs());
    }

    fn wait_widget(&mut self, text: &str) {
        self.wait(&format!("the widget to read {text}"), |v| {
            v["widget"]["text"] == text
        });
    }

    /// Waits for a balloon, then closes them all.
    fn wait_balloon(&mut self, text: &str) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            if self.balloons().iter().any(|b| b == text) {
                self.invoke("notifications.clear-all", Value::Null);
                return;
            }
        }
        panic!("no balloon {text:?}: {:?}", self.balloons());
    }

    /// Opens the Push dialog and waits for its commits.
    fn open_push(&mut self) -> Value {
        self.invoke("vcs.push", Value::Null);
        self.wait("the Push dialog", |v| {
            let d = &v["remote"]["push_dialog"];
            let files = d["files"].as_array().is_some_and(|f| !f.is_empty());
            d["loading"] == false && (files || !d["files_text"].is_null())
        })["remote"]["push_dialog"]
            .clone()
    }
}

fn open<'a>(cx: &'a mut TestAppContext, root: &Path) -> H<'a> {
    support::allow_processes(cx);
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
    let mut h = H { cx, window };
    let path = root.display().to_string();
    h.invoke("project.open-folder", json!({ "path": path }));
    h.wait_widget("main");
    h
}

fn texts(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[gpui::test]
fn push_and_a_rejected_push(cx: &mut TestAppContext) {
    let repos = repos();
    commit(&repos.root, "a.py", "1\n2\n3\n4\n", "third");
    let mut h = open(cx, &repos.root);

    // The dialog: the repository row, the commit, its files.
    let dialog = h.open_push();
    assert_eq!(dialog["title"], "Push Commits to demo");
    assert_eq!(
        texts(&dialog["rows"]),
        vec!["main \u{2192} origin : main", "third"]
    );
    assert!(dialog["rows"][0]["label"].is_null());
    assert_eq!(
        dialog["files"],
        json!([{ "path": "a.py", "status": "modified", "old_path": null }])
    );
    assert_eq!(dialog["can_push"], true);
    assert_eq!(dialog["force_push"], "Force push to main is prohibited");
    h.invoke("push-dialog.push", Value::Null);
    assert!(h.remote()["push_dialog"].is_null(), "closed");
    h.wait_balloon("information: Pushed 1 commit to origin/main | ");
    assert_eq!(
        git(&repos.remote, &["rev-parse", "main"]),
        git(&repos.root, &["rev-parse", "main"])
    );

    // The remote moved ahead: Push Rejected, Merge, pushed again.
    remote_commit(&repos, "c.py", "c\n", "remote one");
    commit(&repos.root, "a.py", "1\n2\n3\n4\n5\n", "fourth");
    h.invoke("vcs.push", json!({ "dialog": false }));
    let vcs = h.wait("Push Rejected", |v| !v["remote"]["remote_dialog"].is_null());
    let d = &vcs["remote"]["remote_dialog"];
    assert_eq!(d["title"], "Push Rejected");
    assert_eq!(d["buttons"], json!(["Cancel", "Rebase", "Merge"]));
    assert_eq!(d["default"], "Merge");
    assert_eq!(
        d["message"],
        "Push of the current branch \"main\" was rejected. \
         Remote changes need to be merged before pushing."
    );
    assert!(
        d["option"]["label"]
            .as_str()
            .unwrap()
            .starts_with("Remember the update method")
    );
    h.invoke("dialog.choose", json!({ "button": "Merge" }));
    h.wait_balloon(
        "information: Pushed 2 commits to origin/main | 1 commit received during the push operation",
    );
    assert_eq!(
        git(&repos.remote, &["rev-parse", "main"]),
        git(&repos.root, &["rev-parse", "main"])
    );
    assert!(repos.root.join("c.py").exists(), "merged");

    // Cancel: the push stays rejected.
    remote_commit(&repos, "d.py", "d\n", "remote two");
    commit(&repos.root, "b.py", "b\nb\n", "fifth");
    h.invoke("vcs.push", json!({ "dialog": false }));
    h.wait("Push Rejected", |v| !v["remote"]["remote_dialog"].is_null());
    h.invoke("dialog.choose", json!({ "button": "Cancel" }));
    h.wait_balloon("warning: Push rejected | Push to origin/main was rejected");
    assert!(h.remote()["remote_dialog"].is_null());

    // Rebase this time, remembered: the next rejection updates silently.
    h.invoke("vcs.push", json!({ "dialog": false }));
    h.wait("Push Rejected", |v| !v["remote"]["remote_dialog"].is_null());
    h.invoke("dialog.toggle-option", Value::Null);
    h.invoke("dialog.choose", json!({ "button": "Rebase" }));
    h.wait_balloon(
        "information: Pushed 1 commit to origin/main | 1 commit received during the push operation",
    );
    assert_eq!(h.remote()["update_method"], "rebase");
    remote_commit(&repos, "e.py", "e\n", "remote three");
    commit(&repos.root, "a.py", "one\n2\n3\n4\n5\n", "sixth");
    h.invoke("vcs.push", json!({ "dialog": false }));
    h.wait_balloon(
        "information: Pushed 1 commit to origin/main | 1 commit received during the push operation",
    );
    assert!(h.remote()["remote_dialog"].is_null(), "no dialog");
    assert_eq!(
        git(
            &repos.root,
            &["rev-list", "--merges", "--count", "HEAD~1..HEAD"]
        ),
        "0",
        "rebased"
    );
}

#[gpui::test]
fn a_new_branch_and_a_force_push(cx: &mut TestAppContext) {
    let repos = repos();
    let mut h = open(cx, &repos.root);
    h.invoke("git.new-branch", json!({ "name": "feat/x" }));
    h.wait_widget("feat/x");
    commit(&repos.root, "x.py", "x\n", "feature");

    // A new branch: the `New` label, the upstream set by the push.
    let dialog = h.open_push();
    assert_eq!(texts(&dialog["rows"])[0], "feat/x \u{2192} origin : feat/x");
    assert_eq!(dialog["rows"][0]["label"], "New");
    assert_eq!(dialog["target"]["new"], true);
    assert_eq!(dialog["sets_upstream"], true);
    assert_eq!(texts(&dialog["rows"])[1..], ["feature"]);
    h.invoke("push-dialog.push", Value::Null);
    h.wait_balloon("information: Pushed feat/x to new branch origin/feat/x | ");
    assert_eq!(
        git(&repos.root, &["config", "branch.feat/x.remote"]),
        "origin"
    );

    // Rewritten history: Force Push asks first.
    git(
        &repos.root,
        &["commit", "-q", "--amend", "-m", "feature, reworded"],
    );
    let dialog = h.open_push();
    assert_eq!(dialog["force_push"], true);
    h.invoke("push-dialog.force-push", Value::Null);
    let vcs = h.wait("Force Push", |v| !v["remote"]["remote_dialog"].is_null());
    let d = &vcs["remote"]["remote_dialog"];
    assert_eq!(d["title"], "Force Push");
    assert_eq!(d["buttons"], json!(["Cancel", "Force Push"]));
    assert_eq!(
        d["message"],
        "You're going to force push to 'feat/x'. It may overwrite commits at the remote. \
         Are you sure you want to proceed?"
    );
    assert_eq!(d["option"]["label"], "Don't warn about this target");
    // Cancel goes back to the Push dialog.
    h.invoke("dialog.choose", json!({ "button": "Cancel" }));
    assert!(
        !h.remote()["push_dialog"].is_null(),
        "the Push dialog stays"
    );
    h.invoke("push-dialog.force-push", Value::Null);
    h.wait("Force Push", |v| !v["remote"]["remote_dialog"].is_null());
    h.invoke("dialog.choose", json!({ "button": "Force Push" }));
    assert!(h.remote()["push_dialog"].is_null(), "closed");
    h.wait_balloon("information: Force pushed feat/x to origin/feat/x | ");
    assert_eq!(
        git(&repos.remote, &["rev-parse", "feat/x"]),
        git(&repos.root, &["rev-parse", "feat/x"])
    );

    // Another target typed in the dialog, Set upstream offered.
    let _ = h.open_push();
    h.invoke(
        "push-dialog.set-target",
        json!({ "branch": "feat/y", "remote": "origin" }),
    );
    let dialog = h.wait("the new target", |v| {
        v["remote"]["push_dialog"]["target"]["branch"] == "feat/y"
            && v["remote"]["push_dialog"]["loading"] == false
    })["remote"]["push_dialog"]
        .clone();
    assert_eq!(dialog["target"]["new"], true);
    assert_eq!(dialog["set_upstream"], false);
    assert_eq!(dialog["rows"][0]["label"], "New");
    h.invoke("push-dialog.toggle-upstream", Value::Null);
    assert_eq!(
        h.remote()["push_dialog"]["rows"][0]["label"],
        "New & Upstream"
    );
    h.invoke("push-dialog.cancel", Value::Null);
    assert!(h.remote()["push_dialog"].is_null());

    // Force push to main is prohibited.
    h.invoke("git.checkout", json!({ "ref": "main" }));
    h.wait_widget("main");
    commit(&repos.root, "b.py", "bb\n", "main change");
    let dialog = h.open_push();
    assert_eq!(dialog["force_push"], "Force push to main is prohibited");
    h.invoke("push-dialog.force-push", Value::Null);
    assert!(h.remote()["remote_dialog"].is_null(), "nothing to confirm");
    assert!(!h.remote()["push_dialog"].is_null(), "the dialog stays");
}

#[gpui::test]
fn commit_and_push_opens_the_push_dialog(cx: &mut TestAppContext) {
    let repos = repos();
    std::fs::write(repos.root.join("a.py"), "1\n2\n3\n4\n").unwrap();
    let mut h = open(cx, &repos.root);
    h.wait("the change", |v| {
        v["commit"]["rows"].as_array().is_some_and(|r| r.len() >= 2)
    });

    // Commit and Push… commits, then runs `vcs.push`.
    h.invoke("vcs.commit", Value::Null);
    h.invoke("vcs.commit.push", json!({ "message": "Commit then push" }));
    let vcs = h.wait("the Push dialog with the new commit", |v| {
        let d = &v["remote"]["push_dialog"];
        d["loading"] == false
            && d["rows"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r["text"] == "Commit then push"))
    });
    assert_eq!(
        vcs["remote"]["push_dialog"]["title"],
        "Push Commits to demo"
    );
    h.invoke("push-dialog.push", Value::Null);
    h.wait_balloon("information: Pushed 1 commit to origin/main | ");
    assert_eq!(
        git(&repos.remote, &["log", "-1", "--format=%s", "main"]),
        "Commit then push"
    );
}

#[gpui::test]
fn update_project_with_local_changes(cx: &mut TestAppContext) {
    let repos = repos();
    let mut h = open(cx, &repos.root);

    // Incoming b.py, a local change of a.py: stashed around the update.
    remote_commit(&repos, "b.py", "b\nfrom remote\n", "remote b");
    std::fs::write(repos.root.join("a.py"), "1\n2\n3\nlocal\n").unwrap();
    h.invoke("vcs.update-project", Value::Null);
    let d = h.remote()["update_dialog"].clone();
    assert_eq!(d["title"], "Update Project");
    assert_eq!(
        d["methods"],
        json!([
            "Merge incoming changes into the current branch",
            "Rebase the current branch on top of incoming changes"
        ])
    );
    assert_eq!(d["method"], "merge");
    h.invoke("update-dialog.set-method", json!({ "method": "rebase" }));
    h.invoke("update-dialog.toggle-dont-show", Value::Null);
    assert_eq!(h.remote()["update_dialog"]["dont_show"], true);
    h.invoke("update-dialog.ok", Value::Null);
    assert!(h.remote()["update_dialog"].is_null());
    h.wait_balloon("information: 1 file updated in 1 commit | ");
    assert_eq!(
        std::fs::read_to_string(repos.root.join("a.py")).unwrap(),
        "1\n2\n3\nlocal\n",
        "the local change came back"
    );
    assert_eq!(
        std::fs::read_to_string(repos.root.join("b.py")).unwrap(),
        "b\nfrom remote\n"
    );

    // Don't show again: the next update runs at once with rebase.
    remote_commit(&repos, "c.py", "c\n", "remote c");
    h.invoke("vcs.update-project", Value::Null);
    assert!(h.remote()["update_dialog"].is_null(), "no dialog");
    h.wait_balloon("information: 1 file updated in 1 commit | ");
    assert_eq!(h.remote()["update_method"], "rebase");
    h.invoke("vcs.update-project", Value::Null);
    h.wait_balloon("information:  | All files are up to date");

    // Conflicts stop the merge: the repository stays merging.
    git(&repos.root, &["checkout", "-q", "--", "a.py"]);
    remote_commit(&repos, "a.py", "remote\n2\n3\n", "remote a");
    commit(&repos.root, "a.py", "local\n2\n3\n", "local a");
    h.invoke("vcs.update-project", json!({ "method": "merge" }));
    h.wait_balloon("warning: Cannot complete update | There are pending unresolved conflicts.");
    assert!(repos.root.join(".git/MERGE_HEAD").exists(), "still merging");
    h.invoke("vcs.update-project", json!({ "method": "merge" }));
    h.wait_balloon(
        "error: Cannot update | You have unfinished merge. \
         These conflicts must be resolved before update.",
    );
    git(&repos.root, &["merge", "--abort"]);

    // A branch without a tracked branch.
    h.invoke("git.new-branch", json!({ "name": "lonely" }));
    h.wait_widget("lonely");
    h.invoke("vcs.update-project", json!({ "method": "merge" }));
    h.wait_balloon("error: Cannot update | lonely has no tracked branch");
}

#[gpui::test]
fn fetch_pull_and_the_popups(cx: &mut TestAppContext) {
    let repos = repos();
    let mut h = open(cx, &repos.root);

    // Fetch: a new remote branch, then the same gone and pruned.
    git(&repos.other, &["push", "-q", "origin", "main:feat/remote"]);
    h.invoke("git.fetch", Value::Null);
    h.wait_balloon("information: Fetch successful | ");
    assert!(
        !git(
            &repos.root,
            &["branch", "-r", "--list", "origin/feat/remote"]
        )
        .is_empty()
    );
    git(
        &repos.other,
        &["push", "-q", "origin", "--delete", "feat/remote"],
    );
    h.invoke("git.fetch", json!({ "remote": "origin" }));
    h.wait_balloon(
        "information: Fetch successful | Pruned obsolete remote reference: origin/feat/remote",
    );

    // Pull a remote branch into the current one.
    git(&repos.root, &["branch", "side", "--track", "origin/main"]);
    remote_commit(&repos, "c.py", "c\n", "remote c");
    h.invoke(
        "git.pull",
        json!({ "from": "origin/main", "method": "merge" }),
    );
    h.wait_balloon("information: 1 file updated in 1 commit | ");
    assert!(repos.root.join("c.py").exists());

    // Update of a branch that is not checked out: fast-forwarded.
    h.invoke("git.update-branch", json!({ "name": "side" }));
    h.wait_balloon("information:  | Fetched 1 targets");
    assert_eq!(
        git(&repos.root, &["rev-parse", "side"]),
        git(&repos.root, &["rev-parse", "origin/main"])
    );

    // The branches popup: Update Project, Push and Fetch are there now,
    // a remote branch offers the pulls.
    h.invoke("vcs.branches-popup", Value::Null);
    let vcs = h.wait("the branches", |v| v["branches_popup"]["loaded"] == true);
    let popup = &vcs["branches_popup"];
    assert_eq!(popup["fetch"], true);
    let actions: Vec<(String, bool)> = popup["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "action")
        .map(|r| (r["text"].as_str().unwrap().to_owned(), r["enabled"] == true))
        .collect();
    assert!(actions.contains(&("Update Project\u{2026}".to_owned(), true)));
    assert!(actions.contains(&("Push\u{2026}".to_owned(), true)));
    h.invoke("branches-popup.search", json!({ "text": "origin/main" }));
    h.invoke("branches-popup.confirm", Value::Null);
    let submenu = h.vcs()["branches_popup"]["submenu"].to_string();
    assert!(
        submenu.contains("Pull into 'main' Using Rebase"),
        "{submenu}"
    );
    assert!(
        submenu.contains("Pull into 'main' Using Merge"),
        "{submenu}"
    );
    h.invoke("vcs.branches-popup", Value::Null);
    assert!(h.vcs()["branches_popup"].is_null(), "closed");

    // VCS Operations: Git's actions, those to come disabled.
    h.invoke("vcs.operations-popup", Value::Null);
    let popup = h.remote()["operations_popup"].clone();
    assert_eq!(popup["title"], "VCS Operations");
    let items = popup["levels"][0]["items"].as_array().unwrap().clone();
    assert_eq!(items[0]["header"], "Git");
    let enabled = |label: &str| {
        items
            .iter()
            .find(|i| i["label"] == label)
            .map(|i| i["enabled"] == true)
    };
    assert_eq!(enabled("Push\u{2026}"), Some(true));
    assert_eq!(enabled("Branches\u{2026}"), Some(true));
    assert_eq!(enabled("Show History"), Some(false));
    h.invoke("vcs.operations-popup", Value::Null);
    assert!(h.remote()["operations_popup"].is_null(), "toggled off");
}
