// Tests against temporary repositories made with the git executable; CI has
// git on all three systems.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use git::{GitExe, GitRepo, Repository, StatusService, StatusUpdate};
use vcs::{Backend, BranchKind, Cancel, FileStatus, Head, LineRanges, RepoState, Snapshot};

fn exe() -> GitExe {
    GitExe::find(None).expect("git is installed")
}

/// Runs git in `dir` for the test's own setup; panics on failure.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(&exe().path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
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

/// A new repository on `main` with the machine's global settings that
/// change results switched off.
fn init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.fsmonitor", "false"),
        ("core.excludesFile", "no-such-file"),
        ("status.renames", "true"),
    ] {
        git(dir, &["config", key, value]);
    }
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    // The paths git writes (`gitdir:` files) are canonical: no links, long
    // names on Windows, without the `\\?\` prefix.
    let path = native_path::canonicalize(dir.path()).unwrap();
    (dir, path)
}

fn open(dir: &Path) -> GitRepo {
    GitRepo::new(Repository::detect(dir).expect("a repository"), exe())
}

fn status(repo: &GitRepo) -> Snapshot {
    repo.status(None, &Cancel::new()).unwrap()
}

/// A repository with one commit of `a.py`, `b.py`, `old.md`, `gone.py`.
fn committed() -> (tempfile::TempDir, PathBuf) {
    let (tmp, dir) = temp();
    init(&dir);
    for f in ["a.py", "b.py", "gone.py"] {
        write(&dir, f, &format!("# {f}\nprint(1)\n"));
    }
    write(&dir, "old.md", "# Notes\n\nSome text that stays.\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    (tmp, dir)
}

#[test]
fn statuses_of_a_work_tree() {
    let (_tmp, dir) = committed();
    write(&dir, "a.py", "# a.py\nprint(2)\n");
    write(&dir, "new.py", "x = 1\n");
    write(&dir, "staged.py", "y = 1\n");
    git(&dir, &["add", "staged.py"]);
    std::fs::remove_file(dir.join("gone.py")).unwrap();
    git(&dir, &["mv", "old.md", "notes.md"]);
    write(&dir, ".gitignore", "logs/\n.env\n");
    write(&dir, "logs/today/run.log", "log\n");
    write(&dir, ".env", "KEY=1\n");
    write(&dir, "src/pkg/mod.py", "z = 1\n");

    let repo = open(&dir);
    let s = status(&repo);
    let st = |rel: &str| s.status.status(&dir.join(rel));
    assert_eq!(st("a.py"), FileStatus::Modified);
    assert_eq!(st("b.py"), FileStatus::NotChanged);
    assert_eq!(st("new.py"), FileStatus::Unversioned);
    assert_eq!(st("staged.py"), FileStatus::Added);
    assert_eq!(st("gone.py"), FileStatus::Deleted);
    assert_eq!(st("notes.md"), FileStatus::Renamed);
    assert_eq!(
        s.status.old_path(&dir.join("notes.md")),
        Some(dir.join("old.md").as_path())
    );
    assert_eq!(st("logs"), FileStatus::Ignored);
    assert_eq!(st("logs/today/run.log"), FileStatus::Ignored);
    assert_eq!(st(".env"), FileStatus::Ignored);
    assert_eq!(st(".gitignore"), FileStatus::Unversioned);
    assert_eq!(
        s.status.status(&dir.join("src").join("pkg").join("mod.py")),
        FileStatus::Unversioned
    );
    assert_eq!(st("src"), FileStatus::NotChanged);
    assert_eq!(
        s.info.head,
        Head::Branch {
            name: "main".into(),
            oid: Some(git(&dir, &["rev-parse", "HEAD"]).trim().to_owned())
        }
    );
    assert_eq!(s.info.state, RepoState::Normal);

    // A status limited to some paths sees only them.
    let limited = repo
        .status(Some(&[dir.join("a.py"), dir.join("src")]), &Cancel::new())
        .unwrap();
    assert_eq!(limited.status.len(), 2);

    // An unsaved document of a clean tracked file is modified.
    assert_eq!(
        s.status.document_status(&dir.join("b.py"), true),
        FileStatus::Modified
    );
}

#[test]
fn a_detached_head_and_branches() {
    let (_tmp, dir) = committed();
    git(&dir, &["tag", "-a", "v1", "-m", "release"]);
    write(&dir, "a.py", "print(3)\n");
    git(&dir, &["commit", "-q", "-am", "second"]);
    git(&dir, &["branch", "feat/x"]);
    let first = git(&dir, &["rev-parse", "HEAD~1"]).trim().to_owned();

    let repo = open(&dir);
    let branches = repo.branches(&Cancel::new()).unwrap();
    let names: Vec<(&str, BranchKind, bool)> = branches
        .iter()
        .map(|b| (b.name.as_str(), b.kind, b.current))
        .collect();
    assert_eq!(
        names,
        vec![
            ("feat/x", BranchKind::Local, false),
            ("main", BranchKind::Local, true),
            ("v1", BranchKind::Tag, false),
        ]
    );
    // An annotated tag names the commit it points at.
    assert_eq!(branches[2].oid, first);

    git(&dir, &["checkout", "-q", "v1"]);
    let info = repo.info().unwrap();
    assert_eq!(info.head, Head::Detached { oid: first.clone() });
    assert_eq!(status(&repo).info.head.branch(), None);

    // Packed refs are read too.
    git(&dir, &["checkout", "-q", "main"]);
    git(&dir, &["pack-refs", "--all"]);
    assert!(!dir.join(".git/refs/heads/main").exists());
    assert_eq!(
        repo.info().unwrap().head.oid(),
        Some(git(&dir, &["rev-parse", "main"]).trim())
    );
}

#[test]
fn a_rebase_with_a_conflict() {
    let (_tmp, dir) = committed();
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "a.py", "print('feature')\n");
    git(&dir, &["commit", "-q", "-am", "feature"]);
    git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "a.py", "print('main')\n");
    git(&dir, &["commit", "-q", "-am", "main"]);
    git(&dir, &["checkout", "-q", "feature"]);
    let out = Command::new(&exe().path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["rebase", "main"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!out.status.success(), "the rebase stops at the conflict");

    let repo = open(&dir);
    let s = status(&repo);
    assert_eq!(s.info.state, RepoState::Rebasing);
    assert_eq!(s.info.rebasing_branch.as_deref(), Some("feature"));
    assert!(matches!(s.info.head, Head::Detached { .. }));
    assert_eq!(s.status.status(&dir.join("a.py")), FileStatus::Conflict);
}

#[test]
fn a_crlf_file_is_not_changed() {
    let (_tmp, dir) = temp();
    init(&dir);
    git(&dir, &["config", "core.autocrlf", "true"]);
    write(&dir, "a.txt", "one\ntwo\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    // Checked out again with the work tree's line endings.
    std::fs::remove_file(dir.join("a.txt")).unwrap();
    git(&dir, &["checkout", "--", "a.txt"]);
    let disk = std::fs::read_to_string(dir.join("a.txt")).unwrap();
    assert_eq!(disk, "one\r\ntwo\r\n");

    let repo = open(&dir);
    assert!(status(&repo).status.is_empty());
    let base = repo
        .base_text(&dir.join("a.txt"), &Cancel::new())
        .unwrap()
        .unwrap();
    assert_eq!(&*base, disk);
    assert!(LineRanges::compute(&base, &disk).is_empty());
    let edited = disk.replace("two", "2");
    assert_eq!(LineRanges::compute(&base, &edited).ranges().len(), 1);
}

#[test]
fn base_texts_are_cached_by_blob() {
    let (_tmp, dir) = committed();
    let repo = open(&dir);
    let a = dir.join("a.py");
    let first = repo.base_text(&a, &Cancel::new()).unwrap().unwrap();
    assert_eq!(&*first, "# a.py\nprint(1)\n");
    // Another commit that leaves the file alone: the same text.
    write(&dir, "b.py", "print(9)\n");
    git(&dir, &["commit", "-q", "-am", "second"]);
    let again = repo.base_text(&a, &Cancel::new()).unwrap().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    // A file HEAD does not have has no base.
    write(&dir, "new.py", "x\n");
    assert_eq!(
        repo.base_text(&dir.join("new.py"), &Cancel::new()).unwrap(),
        None
    );
    // A commit of a new version.
    write(&dir, "a.py", "print(10)\n");
    git(&dir, &["commit", "-q", "-am", "third"]);
    assert_eq!(
        &*repo.base_text(&a, &Cancel::new()).unwrap().unwrap(),
        "print(10)\n"
    );
}

#[test]
fn repositories_are_found_at_and_above_a_folder() {
    let (_tmp, dir) = committed();
    write(&dir, "sub/inner.py", "x\n");
    write(&dir, "other.py", "y\n");
    let sub = dir.join("sub");
    let repo = Repository::detect(&sub).unwrap();
    assert_eq!(repo.root, dir);
    assert_eq!(repo.git_dir, dir.join(".git"));
    // A project inside a larger work tree sees its own files only.
    let scoped = GitRepo::new(repo, exe()).with_scope(&sub);
    let s = status(&scoped);
    assert_eq!(s.status.unversioned(), vec![sub.join("inner.py")]);

    // A linked work tree has a `gitdir:` file.
    let (_wt_tmp, wt_parent) = temp();
    let wt = wt_parent.join("wt");
    git(
        &dir,
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    let linked = Repository::detect(&wt).unwrap();
    assert_eq!(linked.root, wt);
    assert_eq!(linked.common_dir, dir.join(".git"));
    assert!(
        linked
            .git_dir
            .starts_with(dir.join(".git").join("worktrees"))
    );
    assert_eq!(linked.read_head().unwrap().branch(), Some("side"));
    assert!(linked.is_state_path(&linked.git_dir.join("HEAD")));

    let (_none_tmp, none) = temp();
    assert_eq!(Repository::detect(&none), None);
}

/// The next snapshot from the service within a few seconds.
fn next(updates: &flume::Receiver<StatusUpdate>) -> std::sync::Arc<Snapshot> {
    let deadline = Instant::now() + Duration::from_secs(10);
    match updates.recv_deadline(deadline).expect("an update") {
        StatusUpdate::Snapshot(s) => s,
        StatusUpdate::Failed(e) => panic!("status failed: {e}"),
    }
}

#[test]
fn the_service_follows_reported_changes() {
    let (_tmp, dir) = committed();
    let service = StatusService::start(std::sync::Arc::new(open(&dir)));
    let updates = service.updates();
    let first = next(&updates);
    assert!(first.status.is_empty());

    // A file edited outside: a limited status after the debounce.
    write(&dir, "a.py", "print('edited')\n");
    let reported = Instant::now();
    service.paths_changed(vec![dir.join("a.py")]);
    let s = next(&updates);
    assert!(reported.elapsed() >= git::DEBOUNCE);
    assert_eq!(s.status.status(&dir.join("a.py")), FileStatus::Modified);
    assert!(s.generation > first.generation);

    // `git add` outside changes the index: a full status.
    write(&dir, "new.py", "x\n");
    git(&dir, &["add", "new.py"]);
    service.paths_changed(vec![dir.join(".git").join("index")]);
    let s = next(&updates);
    assert_eq!(s.status.status(&dir.join("new.py")), FileStatus::Added);
    assert_eq!(s.status.status(&dir.join("a.py")), FileStatus::Modified);

    // Paths outside the repository are not its business.
    service.paths_changed(vec![PathBuf::from("/elsewhere/x.py")]);
    assert!(updates.recv_timeout(git::DEBOUNCE * 2).is_err());

    // The application's own write commands: one refresh after them.
    service
        .repo()
        .write(["add", "a.py"], &Cancel::new())
        .unwrap();
    let s = next(&updates);
    assert_eq!(s.status.status(&dir.join("a.py")), FileStatus::Modified);
    assert_eq!(service.last().unwrap().generation, s.generation);
}

#[test]
fn engine_commands_answer_for_a_folder() {
    let (_tmp, dir) = committed();
    write(&dir, "a.py", "print(5)\n");
    let mut registry = commands::Registry::new();
    git::commands::register(&mut registry);
    let path = dir.display().to_string();
    let run = |id: &str| {
        futures::executor::block_on(registry.invoke(id, serde_json::json!({ "path": path })))
            .unwrap()
    };
    let status = run("git.status");
    assert_eq!(status["changes"][0]["path"], "a.py");
    assert_eq!(status["changes"][0]["status"], "modified");
    assert_eq!(status["repository"]["branch"], "main");
    let branches = run("git.branches");
    assert_eq!(branches["branches"][0]["name"], "main");
    let state = run("git.state");
    assert_eq!(state["repository"]["state"], "normal");
    assert!(state["git"]["version"].as_str().is_some());

    let (_none_tmp, none) = temp();
    let e = futures::executor::block_on(registry.invoke(
        "git.status",
        serde_json::json!({ "path": none.display().to_string() }),
    ))
    .unwrap_err();
    assert_eq!(e.code, "git.no_repository");
}
