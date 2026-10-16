// Branch operations against temporary repositories made with the git
// executable; CI has git on all three systems.

use std::path::{Path, PathBuf};
use std::process::Command;

use git::{CheckedOut, CheckoutError, CheckoutMode, GitExe, GitRepo, Repository};
use vcs::{Backend, Cancel, Head};

fn exe() -> GitExe {
    GitExe::find(None).expect("git is installed")
}

/// Runs git in `dir` for the test's own setup; panics on failure.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(&exe().path)
        .args(args)
        .current_dir(dir)
        .env("GIT_EDITOR", "true")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path()).unwrap();
    #[cfg(windows)]
    let path = match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => path,
    };
    (dir, path)
}

/// A repository on `main` with one commit of `a.py`, a branch `other`
/// whose `a.py` differs, and a tag `v1` on the first commit. The
/// repository's own identity makes stashes and commits work anywhere.
fn repository() -> (tempfile::TempDir, PathBuf) {
    let (tmp, dir) = temp();
    git(&dir, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.fsmonitor", "false"),
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
    ] {
        git(&dir, &["config", key, value]);
    }
    write(&dir, "a.py", "print(1)\n");
    write(&dir, "b.py", "print(2)\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    git(&dir, &["tag", "v1"]);
    git(&dir, &["checkout", "-q", "-b", "other"]);
    write(&dir, "a.py", "print('other')\n");
    git(&dir, &["commit", "-q", "-am", "other"]);
    git(&dir, &["checkout", "-q", "main"]);
    (tmp, dir)
}

fn open(dir: &Path) -> GitRepo {
    GitRepo::new(Repository::detect(dir).expect("a repository"), exe())
}

fn head(repo: &GitRepo) -> Head {
    repo.info().unwrap().head
}

#[test]
fn checkout_of_a_branch_a_tag_and_a_revision() {
    let (_tmp, dir) = repository();
    let repo = open(&dir);
    let c = Cancel::new();
    let done = repo.checkout("other", CheckoutMode::Normal, &c).unwrap();
    assert_eq!(
        done.checked_out,
        CheckedOut::Branch {
            name: "other".into()
        }
    );
    assert_eq!(head(&repo).branch(), Some("other"));
    assert_eq!(read(&dir, "a.py"), "print('other')\n");

    let done = repo.checkout("v1", CheckoutMode::Normal, &c).unwrap();
    assert_eq!(
        done.checked_out,
        CheckedOut::Detached {
            target: "v1".into()
        }
    );
    assert!(matches!(head(&repo), Head::Detached { .. }));

    let main = repo.resolve_commit("main", &c).unwrap().unwrap();
    repo.checkout(&main[..8], CheckoutMode::Normal, &c).unwrap();
    assert_eq!(head(&repo).oid(), Some(main.as_str()));

    assert_eq!(
        repo.checkout("no-such-ref", CheckoutMode::Normal, &c),
        Err(CheckoutError::NotFound("no-such-ref".into()))
    );

    // Checkout Tag or Revision… detaches HEAD even at a branch.
    let done = repo
        .checkout_detached("other", CheckoutMode::Normal, &c)
        .unwrap();
    assert_eq!(
        done.checked_out,
        CheckedOut::Detached {
            target: "other".into()
        }
    );
    assert!(matches!(head(&repo), Head::Detached { .. }));
}

#[test]
fn local_changes_in_the_way_stop_a_checkout_and_smart_checkout_keeps_them() {
    let (_tmp, dir) = repository();
    let repo = open(&dir);
    let c = Cancel::new();
    write(&dir, "a.py", "print('mine')\n");
    write(&dir, "b.py", "print('mine too')\n");
    assert_eq!(
        repo.checkout("other", CheckoutMode::Normal, &c),
        Err(CheckoutError::LocalChanges(vec!["a.py".into()]))
    );
    assert_eq!(head(&repo).branch(), Some("main"));

    // b.py does not differ between the branches: it comes back cleanly.
    git(&dir, &["checkout", "--", "a.py"]);
    let done = repo.checkout("other", CheckoutMode::Smart, &c).unwrap();
    assert_eq!(head(&repo).branch(), Some("other"));
    assert!(!done.conflicts_on_restore);
    assert_eq!(read(&dir, "b.py"), "print('mine too')\n");
    assert_eq!(git(&dir, &["stash", "list"]), "");
}

#[test]
fn smart_checkout_with_conflicting_changes_keeps_the_stash() {
    let (_tmp, dir) = repository();
    let repo = open(&dir);
    let c = Cancel::new();
    write(&dir, "a.py", "print('mine')\n");
    let done = repo.checkout("other", CheckoutMode::Smart, &c).unwrap();
    assert_eq!(head(&repo).branch(), Some("other"));
    assert!(done.conflicts_on_restore);
    assert!(!git(&dir, &["stash", "list"]).is_empty());
}

#[test]
fn force_checkout_overwrites_local_changes() {
    let (_tmp, dir) = repository();
    let repo = open(&dir);
    write(&dir, "a.py", "print('mine')\n");
    repo.checkout("other", CheckoutMode::Force, &Cancel::new())
        .unwrap();
    assert_eq!(read(&dir, "a.py"), "print('other')\n");
}

#[test]
fn a_remote_branch_is_checked_out_as_a_new_tracking_branch() {
    let (_tmp, dir) = repository();
    let (_rtmp, remote) = temp();
    git(&remote, &["init", "-q", "--bare"]);
    let url = remote.display().to_string();
    git(&dir, &["remote", "add", "origin", &url]);
    git(&dir, &["push", "-q", "origin", "other:feat/remote"]);
    git(&dir, &["fetch", "-q", "origin"]);
    let repo = open(&dir);
    let done = repo
        .checkout("origin/feat/remote", CheckoutMode::Normal, &Cancel::new())
        .unwrap();
    assert_eq!(
        done.checked_out,
        CheckedOut::NewBranch {
            name: "feat/remote".into(),
            from: "origin/feat/remote".into()
        }
    );
    assert_eq!(head(&repo).branch(), Some("feat/remote"));
    let upstream = git(&dir, &["rev-parse", "--abbrev-ref", "@{upstream}"]);
    assert_eq!(upstream.trim(), "origin/feat/remote");
    // Once it exists, the local branch is checked out.
    git(&dir, &["checkout", "-q", "main"]);
    let done = repo
        .checkout("origin/feat/remote", CheckoutMode::Normal, &Cancel::new())
        .unwrap();
    assert_eq!(
        done.checked_out,
        CheckedOut::Branch {
            name: "feat/remote".into()
        }
    );
}

#[test]
fn new_rename_delete_and_restore() {
    let (_tmp, dir) = repository();
    let repo = open(&dir);
    let c = Cancel::new();
    repo.new_branch("feat/x", None, true, false, &c).unwrap();
    assert_eq!(head(&repo).branch(), Some("feat/x"));
    repo.new_branch("from-other", Some("other"), false, false, &c)
        .unwrap();
    assert_eq!(head(&repo).branch(), Some("feat/x"));
    assert_eq!(
        repo.resolve_commit("from-other", &c).unwrap(),
        repo.resolve_commit("other", &c).unwrap()
    );
    assert!(repo.new_branch("main", None, false, false, &c).is_err());
    repo.new_branch("main", Some("other"), false, true, &c)
        .unwrap();
    assert_eq!(
        repo.resolve_commit("main", &c).unwrap(),
        repo.resolve_commit("other", &c).unwrap()
    );

    repo.checkout("main", CheckoutMode::Normal, &c).unwrap();
    repo.rename_branch("feat/x", "feat/y", &c).unwrap();
    let names: Vec<String> = repo
        .branches(&c)
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.contains(&"feat/y".to_owned()) && !names.contains(&"feat/x".to_owned()));

    let oid = repo.delete_branch("feat/y", &c).unwrap();
    assert_eq!(repo.resolve_commit("feat/y", &c).unwrap(), None);
    repo.restore_branch("feat/y", &oid, &c).unwrap();
    assert_eq!(repo.resolve_commit("feat/y", &c).unwrap(), Some(oid));
}

#[test]
fn recent_checkouts_come_from_the_reflog() {
    let (_tmp, dir) = repository();
    let repo = open(&dir);
    let c = Cancel::new();
    repo.new_branch("feat/x", None, true, false, &c).unwrap();
    repo.checkout("other", CheckoutMode::Normal, &c).unwrap();
    repo.checkout("main", CheckoutMode::Normal, &c).unwrap();
    git(&dir, &["branch", "-D", "feat/x"]);
    // Newest first, each once, deleted branches left out.
    assert_eq!(
        repo.recent_checkouts(5, &c).unwrap(),
        vec!["main".to_owned(), "other".to_owned()]
    );
}

#[test]
fn init_creates_a_repository() {
    let (_tmp, dir) = temp();
    git::branches::init(&exe(), &dir, &Cancel::new()).unwrap();
    assert!(Repository::detect(&dir).is_some());
}

#[test]
fn engine_commands_for_branches() {
    let (_tmp, dir) = repository();
    let mut registry = commands::Registry::new();
    git::commands::register(&mut registry);
    let path = dir.display().to_string();
    let run = |id: &str, args: serde_json::Value| {
        let mut args = args;
        args["path"] = path.clone().into();
        futures::executor::block_on(registry.invoke(id, args))
    };
    let r = run(
        "git.new-branch",
        serde_json::json!({ "name": "feat x", "from": "other" }),
    )
    .unwrap();
    assert_eq!(r["name"], "feat-x");
    let e = run("git.new-branch", serde_json::json!({ "name": "HEAD" })).unwrap_err();
    assert_eq!(e.message, "HEAD is a reserved keyword");
    let e = run("git.new-branch", serde_json::json!({ "name": "main" })).unwrap_err();
    assert_eq!(e.message, "Branch name main already exists");

    write(&dir, "a.py", "print('mine')\n");
    let e = run("git.checkout", serde_json::json!({ "ref": "main" })).unwrap_err();
    assert_eq!(e.code, "git.local_changes");
    let r = run(
        "git.checkout",
        serde_json::json!({ "ref": "main", "mode": "force" }),
    )
    .unwrap();
    assert_eq!(r["checked_out"]["name"], "main");
    let r = run(
        "git.rename-branch",
        serde_json::json!({ "from": "feat-x", "to": "feat-y" }),
    )
    .unwrap();
    assert_eq!(r["to"], "feat-y");
    let r = run("git.delete-branch", serde_json::json!({ "name": "feat-y" })).unwrap();
    assert_eq!(r["oid"].as_str().map(str::len), Some(40));

    let (_none_tmp, none) = temp();
    let r = futures::executor::block_on(registry.invoke(
        "git.init",
        serde_json::json!({ "path": none.display().to_string() }),
    ))
    .unwrap();
    assert!(r["root"].as_str().is_some());
    assert!(Repository::detect(&none).is_some());
}
