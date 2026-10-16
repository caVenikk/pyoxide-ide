//! The VCS widget and its branch commands (#28) against repositories made
//! with the git executable: the widget's text, icon and tooltip, the
//! branches popup and its search, New Branch (dialog and errors),
//! checkout with "Git Checkout Problem" and Smart Checkout, rename,
//! delete and restore, a tag and a commit checked out, and Create Git
//! Repository… for a folder without one. The tests wait for real
//! processes and events.

mod support;

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

/// A repository on `main` with its own identity (stashes need one), a
/// tag `v1` and a branch `other` whose `a.py` differs in its last line.
fn repository(root: &Path) {
    git(root, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(root, &["config", key, value]);
    }
    std::fs::write(root.join("a.py"), "1\n2\n3\n4\n5\n").unwrap();
    git(root, &["add", "a.py"]);
    git(root, &["commit", "-q", "-m", "first"]);
    git(root, &["tag", "v1"]);
    git(root, &["branch", "other"]);
    git(root, &["checkout", "-q", "other"]);
    std::fs::write(root.join("a.py"), "1\n2\n3\n4\nfive\n").unwrap();
    git(root, &["commit", "-q", "-am", "five"]);
    git(root, &["checkout", "-q", "main"]);
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

    /// Draws the window: actions reach what the last frame laid out.
    fn draw(&mut self) {
        self.cx.run_until_parked();
        let _ = self
            .cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear());
    }

    fn vcs(&mut self) -> Value {
        self.invoke("vcs.state", Value::Null)
    }

    fn balloons(&mut self) -> Vec<String> {
        self.invoke("workbench.state", Value::Null)["notifications"]["balloons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["content"].as_str().unwrap_or_default().to_owned())
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

    fn wait_widget(&mut self, text: &str) -> Value {
        self.wait(&format!("the widget to read {text}"), |v| {
            v["widget"]["text"] == text
        })["widget"]
            .clone()
    }

    fn wait_balloon(&mut self, text: &str) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            support::tick(self.cx);
            if self.balloons().iter().any(|b| b == text) {
                return;
            }
        }
        panic!("no balloon {text:?}: {:?}", self.balloons());
    }
}

fn open(cx: &mut TestAppContext, prepare: impl FnOnce(&Path)) -> (H<'_>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
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
    (h, dir)
}

/// The rows of the popup as short texts.
fn rows(vcs: &Value) -> Vec<String> {
    vcs["branches_popup"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let pad = "  ".repeat(r["depth"].as_u64().unwrap_or(0) as usize);
            match r["kind"].as_str().unwrap() {
                "separator" => "--".to_owned(),
                "group" | "folder" => format!(
                    "{pad}{} {}",
                    if r["expanded"] == true { "v" } else { ">" },
                    r["text"].as_str().unwrap()
                ),
                "branch" => format!(
                    "{pad}{}{}",
                    r["text"].as_str().unwrap(),
                    if r["current"] == true { " *" } else { "" }
                ),
                _ => format!("{pad}{}", r["text"].as_str().unwrap()),
            }
        })
        .collect()
}

#[gpui::test]
fn the_widget_follows_branch_commands(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, repository);
    let widget = h.wait_widget("main");
    assert_eq!(widget["kind"], "repository");
    assert_eq!(widget["icon"], "branch");
    assert_eq!(widget["tooltip"], "Git Branch: main");

    // The popup: the top actions, then the groups; main is current.
    h.invoke("vcs.branches-popup", Value::Null);
    let vcs = h.wait("the branches", |v| v["branches_popup"]["loaded"] == true);
    assert_eq!(
        vcs["branches_popup"]["placeholder"],
        "Search for branches and actions"
    );
    assert_eq!(
        rows(&vcs),
        vec![
            "Update Project\u{2026}",
            "Commit\u{2026}",
            "Push\u{2026}",
            "--",
            "New Branch\u{2026}",
            "Checkout Tag or Revision\u{2026}",
            "--",
            "v Recent",
            "  main *",
            "  other",
            "> Tags",
        ]
    );
    // Typing filters; Escape clears the search, then closes.
    h.invoke("branches-popup.search", json!({ "text": "oth" }));
    assert_eq!(rows(&h.vcs()), vec!["v Recent", "  other"]);
    h.invoke("branches-popup.close", Value::Null);
    h.invoke("branches-popup.close", Value::Null);
    assert!(h.vcs()["branches_popup"].is_null());

    // New Branch with a name: created from main and checked out.
    h.invoke("git.new-branch", json!({ "name": "feat/x" }));
    h.wait_widget("feat/x");
    h.wait_balloon("Checked out new branch feat/x from main");

    // Back on main, with a local change in the way of `other`.
    h.invoke("git.checkout", json!({ "ref": "main" }));
    h.wait_widget("main");
    h.wait_balloon("Checked out main");
    std::fs::write(h.root.join("a.py"), "one\n2\n3\n4\n5\n").unwrap();
    h.invoke("git.checkout", json!({ "ref": "other" }));
    let vcs = h.wait("Git Checkout Problem", |v| {
        v["branch_dialog"]["title"] == "Git Checkout Problem"
    });
    assert_eq!(
        vcs["branch_dialog"]["buttons"],
        json!(["Force Checkout", "Don't Checkout", "Smart Checkout"])
    );
    assert!(
        vcs["branch_dialog"]["message"]
            .as_str()
            .unwrap()
            .ends_with("\n\na.py")
    );
    h.invoke("dialog.choose", json!({ "button": "Smart Checkout" }));
    h.wait_widget("other");
    // The change came back on top of the branch's text.
    assert_eq!(
        std::fs::read_to_string(h.root.join("a.py")).unwrap(),
        "one\n2\n3\n4\nfive\n"
    );

    // Rename the current branch; delete another one and restore it.
    h.invoke(
        "git.rename-branch",
        json!({ "from": "other", "to": "renamed" }),
    );
    h.wait_widget("renamed");
    h.wait_balloon("Branch other was renamed to renamed");
    let oid = git(&h.root, &["rev-parse", "feat/x"]);
    h.invoke("git.delete-branch", json!({ "name": "feat/x" }));
    h.wait_balloon("Deleted branch feat/x");
    assert!(
        git(&h.root, &["branch", "--list", "feat/x"]).is_empty(),
        "deleted"
    );
    h.invoke(
        "git.restore-branch",
        json!({ "name": "feat/x", "oid": oid }),
    );
    let start = Instant::now();
    while git(&h.root, &["branch", "--list", "feat/x"]).is_empty() {
        assert!(start.elapsed() < Duration::from_secs(30), "restored");
        support::tick(h.cx);
    }

    // A tag, then a commit: HEAD detached.
    h.invoke("git.checkout", json!({ "ref": "main", "mode": "force" }));
    h.wait_widget("main");
    h.invoke("git.checkout-revision", json!({ "ref": "v1" }));
    let widget = h.wait_widget("v1");
    assert_eq!(widget["icon"], "tag");
    assert_eq!(
        widget["tooltip"],
        "Git: Detached HEAD doesn't point to any branch"
    );
    let other = git(&h.root, &["rev-parse", "renamed"]);
    h.invoke("git.checkout", json!({ "ref": other }));
    let widget = h.wait_widget(&other[..8]);
    assert_eq!(widget["icon"], "warning");
}

#[gpui::test]
fn the_new_branch_dialog_checks_names(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, repository);
    h.wait_widget("main");
    h.invoke("git.new-branch", Value::Null);
    let vcs = h.wait("the dialog", |v| !v["branch_dialog"].is_null());
    let dialog = &vcs["branch_dialog"];
    assert_eq!(dialog["title"], "Create New Branch");
    assert_eq!(dialog["label"], "Branch Name:");
    assert_eq!(dialog["text"], "main");
    assert_eq!(dialog["checkout"], true);
    assert_eq!(dialog["ok"], "Create");

    let error = |h: &mut H, text: &str, confirm: bool| -> Value {
        h.invoke("branch-dialog.set-name", json!({ "text": text }));
        if confirm {
            h.invoke("branch-dialog.confirm", Value::Null);
        }
        h.vcs()["branch_dialog"].clone()
    };
    assert_eq!(
        error(&mut h, "", true)["error"],
        "Specify name for the new branch"
    );
    assert_eq!(
        error(&mut h, "HEAD", true)["error"],
        "HEAD is a reserved keyword"
    );
    let d = error(&mut h, "main", false);
    assert_eq!(
        d["error"],
        "Branch name main already exists.\nChange the name or overwrite existing branch"
    );
    assert_eq!(d["overwrite"], false);
    // Spaces become dashes while typing.
    let d = error(&mut h, "my branch", false);
    assert_eq!(d["text"], "my-branch");
    assert!(d["error"].is_null());
    h.invoke("branch-dialog.confirm", Value::Null);
    h.wait_widget("my-branch");
    assert!(h.vcs()["branch_dialog"].is_null());

    // Rename asks with the branch's name.
    h.invoke("git.rename-branch", Value::Null);
    let vcs = h.wait("the rename dialog", |v| !v["branch_dialog"].is_null());
    assert_eq!(vcs["branch_dialog"]["title"], "Rename Branch my-branch");
    assert_eq!(vcs["branch_dialog"]["ok"], "Rename");
    h.invoke("branch-dialog.cancel", Value::Null);
    assert!(h.vcs()["branch_dialog"].is_null());
}

#[gpui::test]
fn a_folder_without_git_offers_a_repository(cx: &mut TestAppContext) {
    let (mut h, _dir) = open(cx, |root| {
        std::fs::write(root.join("a.py"), "a = 1\n").unwrap();
    });
    let vcs = h.wait("the folder to be looked at", |v| {
        v["widget"]["kind"] == "no_vcs"
    });
    assert_eq!(vcs["widget"]["text"], "Version Control");
    assert!(vcs["widget"]["icon"].is_null());
    assert_eq!(
        vcs["widget"]["tooltip"],
        "Configure a version control for the project"
    );
    h.invoke("vcs.branches-popup", Value::Null);
    let menu = h.vcs()["branches_popup"].clone();
    assert!(
        menu.to_string().contains("Create Git Repository\u{2026}"),
        "{menu:#}"
    );
    h.invoke("menu.cancel", Value::Null);
    let path = h.root.display().to_string();
    h.invoke("git.init", json!({ "path": path }));
    let vcs = h.wait("the new repository", |v| {
        v["widget"]["kind"] == "repository"
    });
    assert!(!vcs["widget"]["text"].as_str().unwrap().is_empty());
    h.wait("a.py to be unversioned", |v| {
        v["status"]["unversioned"] == json!(["a.py"])
    });
}
