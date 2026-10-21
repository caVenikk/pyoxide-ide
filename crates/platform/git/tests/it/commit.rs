// Commit, amend, rollback, Add to VCS and the user's identity against
// temporary repositories made with the git executable. Every call of
// this test binary reads a global config file of its own, so the
// person's configuration is neither read nor written.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use git::{GitExe, GitRepo, Repository};
use vcs::{Backend, Cancel, Change, CommitRequest, FileStatus};

fn exe() -> GitExe {
    GitExe::find(None).expect("git is installed")
}

/// The global config of every git call of the test binary.
fn global_config() -> &'static Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let path = dir.path().join("global-config");
    git::use_global_config(path);
    dir.path()
}

/// Runs git in `dir` for the test's own setup; panics on failure.
fn git(dir: &Path, args: &[&str]) -> String {
    let config = global_config().join("global-config");
    let out = Command::new(exe().path)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
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
    let path = native_path::canonicalize(dir.path()).unwrap();
    (dir, path)
}

/// A repository with `a.py`, `b.py`, `c.py` and `notes.txt` committed as
/// "initial", with its own identity unless `identity` is false.
fn repository(identity: bool) -> (tempfile::TempDir, PathBuf) {
    global_config();
    let (tmp, dir) = temp();
    git::testing::init(&dir);
    for (key, value) in [("core.fsmonitor", "false"), ("commit.gpgsign", "false")] {
        git(&dir, &["config", key, value]);
    }
    for f in ["a.py", "b.py", "c.py", "notes.txt"] {
        write(&dir, f, &format!("{f}\n"));
    }
    git(&dir, &["add", "."]);
    let who = [
        "-c",
        "user.name=Setup",
        "-c",
        "user.email=setup@example.invalid",
    ];
    let mut args: Vec<&str> = who.to_vec();
    args.extend(["commit", "-q", "-m", "initial"]);
    git(&dir, &args);
    if identity {
        git(&dir, &["config", "user.name", "Test"]);
        git(&dir, &["config", "user.email", "test@example.invalid"]);
    }
    (tmp, dir)
}

fn open(dir: &Path) -> GitRepo {
    GitRepo::new(Repository::detect(dir).expect("a repository"), exe())
}

fn change(dir: &Path, rel: &str, status: FileStatus) -> Change {
    Change {
        path: dir.join(rel),
        status,
        old_path: None,
    }
}

fn status_of(repo: &GitRepo, dir: &Path, rel: &str) -> FileStatus {
    repo.status(None, &Cancel::new())
        .unwrap()
        .status
        .status(&dir.join(rel))
}

#[test]
fn a_commit_takes_only_the_included_files() {
    let (_tmp, dir) = repository(true);
    write(&dir, "a.py", "a changed\n");
    write(&dir, "b.py", "b changed\n");
    // A staged change of a file that is not included stays staged.
    write(&dir, "c.py", "c staged\n");
    git(&dir, &["add", "c.py"]);
    write(&dir, "new.py", "new\n");
    let repo = open(&dir);
    let request = CommitRequest {
        message: "Fix".into(),
        changes: vec![
            change(&dir, "a.py", FileStatus::Modified),
            change(&dir, "b.py", FileStatus::Modified),
        ],
        unversioned: vec![dir.join("new.py")],
        run_hooks: true,
        ..CommitRequest::default()
    };
    let done = repo.commit(&request, &Cancel::new()).unwrap();
    assert_eq!(done.files, 3);
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]).trim(), done.oid);
    assert_eq!(git(&dir, &["log", "-1", "--format=%B"]).trim(), "Fix");
    let files = git(&dir, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        vec!["a.py", "b.py", "new.py"]
    );
    assert_eq!(
        git(&dir, &["diff", "--cached", "--name-only"]).trim(),
        "c.py"
    );
    assert_eq!(
        git(&dir, &["log", "-1", "--format=%an <%ae>"]).trim(),
        "Test <test@example.invalid>"
    );
}

#[test]
fn a_rename_and_a_deletion_are_committed_with_both_paths() {
    let (_tmp, dir) = repository(true);
    git(&dir, &["mv", "notes.txt", "notes.md"]);
    std::fs::remove_file(dir.join("b.py")).unwrap();
    let repo = open(&dir);
    let snapshot = repo.status(None, &Cancel::new()).unwrap();
    let changes = snapshot.status.changes();
    assert_eq!(changes.len(), 2, "{changes:?}");
    let request = CommitRequest {
        message: "Move".into(),
        changes,
        run_hooks: true,
        ..CommitRequest::default()
    };
    repo.commit(&request, &Cancel::new()).unwrap();
    assert!(git(&dir, &["status", "--porcelain"]).trim().is_empty());
    let files = git(&dir, &["show", "--name-status", "--format=", "-M", "HEAD"]);
    assert!(files.contains("notes.txt\tnotes.md"), "{files}");
    assert!(files.contains("D\tb.py"), "{files}");
}

#[test]
fn amend_replaces_the_last_commit() {
    let (_tmp, dir) = repository(true);
    write(&dir, "a.py", "first\n");
    git(&dir, &["commit", "-q", "-am", "First"]);
    let parent = git(&dir, &["rev-parse", "HEAD~1"]);
    let repo = open(&dir);
    let last = repo.last_commit(&Cancel::new()).unwrap().unwrap();
    assert_eq!(last.subject, "First");
    assert_eq!(last.message, "First");
    assert_eq!(last.author, "Test <test@example.invalid>");
    write(&dir, "b.py", "amended\n");
    let request = CommitRequest {
        message: "First, amended".into(),
        changes: vec![change(&dir, "b.py", FileStatus::Modified)],
        amend: true,
        run_hooks: true,
        ..CommitRequest::default()
    };
    repo.commit(&request, &Cancel::new()).unwrap();
    assert_eq!(
        git(&dir, &["log", "-1", "--format=%B"]).trim(),
        "First, amended"
    );
    assert_eq!(git(&dir, &["rev-parse", "HEAD~1"]), parent);
    let files = git(&dir, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(files.lines().collect::<Vec<_>>(), vec!["a.py", "b.py"]);
    // An amend without files changes the message only, leaving staged
    // changes staged.
    write(&dir, "c.py", "staged\n");
    git(&dir, &["add", "c.py"]);
    let request = CommitRequest {
        message: "Reworded".into(),
        amend: true,
        run_hooks: true,
        ..CommitRequest::default()
    };
    repo.commit(&request, &Cancel::new()).unwrap();
    assert_eq!(git(&dir, &["log", "-1", "--format=%B"]).trim(), "Reworded");
    assert_eq!(git(&dir, &["rev-parse", "HEAD~1"]), parent);
    assert_eq!(
        git(&dir, &["diff", "--cached", "--name-only"]).trim(),
        "c.py"
    );
}

#[test]
fn a_partial_commit_takes_the_included_changes_only() {
    let (_tmp, dir) = repository(true);
    write(&dir, "a.py", "one\ntwo\nthree\nfour\nfive\n");
    git(&dir, &["commit", "-q", "-am", "Lines"]);
    // Two changes of a.py, the second one left out; b.py whole; c.py
    // staged and not included stays staged.
    write(&dir, "a.py", "ONE\ntwo\nthree\nfour\nFIVE\n");
    write(&dir, "b.py", "b changed\n");
    write(&dir, "c.py", "c staged\n");
    git(&dir, &["add", "c.py"]);
    write(&dir, "new.py", "new\n");
    let repo = open(&dir);
    let request = CommitRequest {
        message: "Part".into(),
        changes: vec![
            change(&dir, "a.py", FileStatus::Modified),
            change(&dir, "b.py", FileStatus::Modified),
        ],
        unversioned: vec![dir.join("new.py")],
        partial: vec![vcs::PartialFile {
            path: dir.join("a.py"),
            // The fifth line's change.
            excluded: (4..5).map(|l| l..l + 1).collect(),
        }],
        run_hooks: true,
        ..CommitRequest::default()
    };
    let done = repo.commit(&request, &Cancel::new()).unwrap();
    assert_eq!(done.files, 3);
    assert_eq!(git(&dir, &["log", "-1", "--format=%B"]).trim(), "Part");
    assert_eq!(
        git(&dir, &["show", "HEAD:a.py"]),
        "ONE\ntwo\nthree\nfour\nfive\n"
    );
    assert_eq!(git(&dir, &["show", "HEAD:b.py"]), "b changed\n");
    assert_eq!(git(&dir, &["show", "HEAD:new.py"]), "new\n");
    let files = git(&dir, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        vec!["a.py", "b.py", "new.py"]
    );
    // The change left out stays a local change; the work tree is as it
    // was; the staged file stays staged.
    assert_eq!(read(&dir, "a.py"), "ONE\ntwo\nthree\nfour\nFIVE\n");
    assert_eq!(git(&dir, &["diff", "--name-only"]).trim(), "a.py");
    let diff = git(&dir, &["diff", "-U0", "a.py"]);
    assert!(diff.contains("-five\n+FIVE"), "{diff}");
    assert_eq!(
        git(&dir, &["diff", "--cached", "--name-only"]).trim(),
        "c.py"
    );
    assert_eq!(status_of(&repo, &dir, "a.py"), FileStatus::Modified);
    // No temporary index is left behind.
    let git_dir = dir.join(".git");
    let left: Vec<_> = std::fs::read_dir(&git_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("pyoxide-index"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn sign_off_and_author_go_into_the_commit() {
    let (_tmp, dir) = repository(true);
    write(&dir, "a.py", "x\n");
    let repo = open(&dir);
    let request = CommitRequest {
        message: "Signed".into(),
        changes: vec![change(&dir, "a.py", FileStatus::Modified)],
        sign_off: true,
        author: Some("Other <other@example.invalid>".into()),
        run_hooks: true,
        ..CommitRequest::default()
    };
    repo.commit(&request, &Cancel::new()).unwrap();
    let body = git(&dir, &["log", "-1", "--format=%B"]);
    assert!(
        body.contains("Signed-off-by: Test <test@example.invalid>"),
        "{body}"
    );
    assert_eq!(
        git(&dir, &["log", "-1", "--format=%an <%ae>"]).trim(),
        "Other <other@example.invalid>"
    );
}

#[test]
fn the_identity_is_read_and_set() {
    let (_tmp, dir) = repository(false);
    let repo = open(&dir);
    let c = Cancel::new();
    let identity = repo.identity(&c).unwrap();
    assert!(!identity.is_complete(), "{identity:?}");
    repo.set_identity("Local", "local@example.invalid", false, &c)
        .unwrap();
    let identity = repo.identity(&c).unwrap();
    assert_eq!(identity.name.as_deref(), Some("Local"));
    assert_eq!(identity.email.as_deref(), Some("local@example.invalid"));
    assert_eq!(
        git(&dir, &["config", "--local", "user.name"]).trim(),
        "Local"
    );
    // Globally: the test binary's own global config file.
    let (_tmp2, other) = repository(false);
    let repo = open(&other);
    repo.set_identity("Global", "global@example.invalid", true, &c)
        .unwrap();
    let global = std::fs::read_to_string(global_config().join("global-config")).unwrap();
    assert!(global.contains("Global"), "{global}");
    assert!(repo.identity(&c).unwrap().is_complete());
}

#[test]
fn rollback_reverts_unindexes_and_deletes() {
    let (_tmp, dir) = repository(true);
    write(&dir, "a.py", "changed\n");
    std::fs::remove_file(dir.join("b.py")).unwrap();
    write(&dir, "kept.py", "kept\n");
    write(&dir, "gone.py", "gone\n");
    git(&dir, &["add", "kept.py", "gone.py"]);
    git(&dir, &["mv", "notes.txt", "notes.md"]);
    let repo = open(&dir);
    let c = Cancel::new();
    repo.rollback(
        &[
            change(&dir, "a.py", FileStatus::Modified),
            change(&dir, "b.py", FileStatus::Deleted),
            change(&dir, "kept.py", FileStatus::Added),
        ],
        false,
        &c,
    )
    .unwrap();
    assert_eq!(read(&dir, "a.py"), "a.py\n");
    assert_eq!(read(&dir, "b.py"), "b.py\n");
    // Out of the index, still on disk.
    assert_eq!(read(&dir, "kept.py"), "kept\n");
    assert_eq!(status_of(&repo, &dir, "kept.py"), FileStatus::Unversioned);
    let renamed = Change {
        path: dir.join("notes.md"),
        status: FileStatus::Renamed,
        old_path: Some(dir.join("notes.txt")),
    };
    repo.rollback(
        &[change(&dir, "gone.py", FileStatus::Added), renamed],
        true,
        &c,
    )
    .unwrap();
    assert!(!dir.join("gone.py").exists());
    assert!(!dir.join("notes.md").exists());
    assert_eq!(read(&dir, "notes.txt"), "notes.txt\n");
    let left = git(&dir, &["status", "--porcelain"]);
    assert_eq!(left.trim(), "?? kept.py");
}

#[test]
fn add_to_vcs_stages_new_files() {
    let (_tmp, dir) = repository(true);
    write(&dir, "pkg/new.py", "new\n");
    let repo = open(&dir);
    assert_eq!(
        status_of(&repo, &dir, "pkg/new.py"),
        FileStatus::Unversioned
    );
    repo.add(&[dir.join("pkg/new.py")], &Cancel::new()).unwrap();
    assert_eq!(status_of(&repo, &dir, "pkg/new.py"), FileStatus::Added);
}

#[cfg(unix)]
#[test]
fn a_failing_hook_stops_the_commit_unless_hooks_are_off() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_tmp, dir) = repository(true);
    let repo = open(&dir);
    assert!(!repo.has_commit_hooks());
    let hook = dir.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-commit says no'\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(repo.has_commit_hooks());
    write(&dir, "a.py", "hooked\n");
    let mut request = CommitRequest {
        message: "Hooked".into(),
        changes: vec![change(&dir, "a.py", FileStatus::Modified)],
        run_hooks: true,
        ..CommitRequest::default()
    };
    let e = repo.commit(&request, &Cancel::new()).unwrap_err();
    assert!(e.message.contains("pre-commit says no"), "{e:?}");
    assert_eq!(git(&dir, &["log", "-1", "--format=%s"]).trim(), "initial");
    request.run_hooks = false;
    repo.commit(&request, &Cancel::new()).unwrap();
    assert_eq!(git(&dir, &["log", "-1", "--format=%s"]).trim(), "Hooked");
}
