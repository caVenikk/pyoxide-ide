// Fetch, push and Update Project against real local remotes: a bare
// repository and two clones of it (ours and a colleague's), made with the
// git executable; CI has git on all three systems.

use std::path::{Path, PathBuf};
use std::process::Command;

use git::remote::UpdateResult;
use git::{GitExe, GitRepo, Repository};
use vcs::{
    Backend, Cancel, FileStatus, ForceMode, PushOutcome, PushSpec, PushTarget, RepoState, TagMode,
    UpdateMethod,
};

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
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

fn configure(dir: &Path) {
    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.fsmonitor", "false"),
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("pull.rebase", "false"),
    ] {
        git(dir, &["config", key, value]);
    }
}

/// A bare remote with one commit on `main`, and two clones of it: ours
/// and a colleague's.
struct Remotes {
    _tmp: tempfile::TempDir,
    remote: PathBuf,
    ours: PathBuf,
    theirs: PathBuf,
}

impl Remotes {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        #[cfg(windows)]
        let base = match base.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(rest) => PathBuf::from(rest),
            None => base,
        };
        let remote = base.join("remote.git");
        let seed = base.join("seed");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&seed).unwrap();
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        git(&seed, &["init", "-q", "-b", "main"]);
        configure(&seed);
        write(&seed, "a.py", "1\n2\n3\n");
        write(&seed, "b.py", "b\n");
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "first"]);
        git(&seed, &["remote", "add", "origin", "../remote.git"]);
        git(&seed, &["push", "-q", "-u", "origin", "main"]);
        let ours = base.join("ours");
        let theirs = base.join("theirs");
        git(&base, &["clone", "-q", "remote.git", "ours"]);
        git(&base, &["clone", "-q", "remote.git", "theirs"]);
        configure(&ours);
        configure(&theirs);
        Self {
            _tmp: tmp,
            remote,
            ours,
            theirs,
        }
    }

    fn repo(&self) -> GitRepo {
        GitRepo::new(Repository::detect(&self.ours).expect("a repository"), exe())
    }

    /// The colleague commits `text` to `file` and pushes.
    fn their_commit(&self, file: &str, text: &str, message: &str) {
        write(&self.theirs, file, text);
        git(&self.theirs, &["add", "."]);
        git(&self.theirs, &["commit", "-q", "-m", message]);
        git(&self.theirs, &["push", "-q", "origin", "HEAD"]);
    }

    fn our_commit(&self, file: &str, text: &str, message: &str) {
        write(&self.ours, file, text);
        git(&self.ours, &["add", "."]);
        git(&self.ours, &["commit", "-q", "-m", message]);
    }

    fn remote_head(&self, branch: &str) -> String {
        git(&self.remote, &["rev-parse", branch])
    }
}

fn spec(source: &str, target: PushTarget) -> PushSpec {
    PushSpec {
        source: source.into(),
        target,
        set_upstream: false,
        force: ForceMode::None,
        tags: None,
    }
}

#[test]
fn a_push_sends_the_outgoing_commits() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    r.our_commit("a.py", "1\n2\n3\n4\n", "four");
    r.our_commit("c.py", "c\n", "add c");

    let target = repo.default_push_target("main", &c).unwrap().unwrap();
    assert_eq!(
        target,
        PushTarget {
            remote: "origin".into(),
            branch: "main".into(),
            new: false
        }
    );
    let outgoing = repo.outgoing("main", &target, &c).unwrap();
    let subjects: Vec<&str> = outgoing.iter().map(|o| o.subject.as_str()).collect();
    assert_eq!(subjects, vec!["add c", "four"]);
    assert_eq!(outgoing[0].author, "Test");
    let oids: Vec<String> = outgoing.iter().map(|o| o.oid.clone()).collect();
    let files = repo.commit_files(&oids, &c).unwrap();
    let names: Vec<(&str, FileStatus)> =
        files.iter().map(|f| (f.path.as_str(), f.status)).collect();
    assert_eq!(
        names,
        vec![("a.py", FileStatus::Modified), ("c.py", FileStatus::Added)]
    );

    let result = repo.push(&spec("main", target.clone()), &c).unwrap();
    assert_eq!(result.outcome, PushOutcome::Success { commits: 2 });
    assert_eq!(result.description(), "Pushed 2 commits to origin/main");
    assert_eq!(r.remote_head("main"), git(&r.ours, &["rev-parse", "HEAD"]));
    assert!(repo.outgoing("main", &target, &c).unwrap().is_empty());

    // Nothing more to send.
    let again = repo.push(&spec("main", target), &c).unwrap();
    assert_eq!(again.outcome, PushOutcome::UpToDate);
    assert_eq!(again.description(), "Everything is up to date");
}

#[test]
fn a_new_branch_is_pushed_with_its_upstream_and_tags() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    git(&r.ours, &["checkout", "-q", "-b", "feat/x"]);
    r.our_commit("x.py", "x\n", "x");
    git(&r.ours, &["tag", "-a", "v2", "-m", "v2"]);

    let target = repo.default_push_target("feat/x", &c).unwrap().unwrap();
    assert!(target.new, "the remote has no feat/x");
    assert_eq!(target.short_name(), "origin/feat/x");
    let outgoing = repo.outgoing("feat/x", &target, &c).unwrap();
    assert_eq!(outgoing.len(), 1, "only what no remote branch has");

    let mut s = spec("feat/x", target);
    s.set_upstream = true;
    s.tags = Some(TagMode::CurrentBranch);
    let result = repo.push(&s, &c).unwrap();
    assert_eq!(result.outcome, PushOutcome::NewBranch);
    assert_eq!(result.tags, vec!["v2".to_owned()]);
    assert_eq!(
        result.description(),
        "Pushed feat/x to new branch origin/feat/x, and tag v2 to origin"
    );
    assert_eq!(
        repo.upstream_of("feat/x"),
        Some(("origin".into(), "feat/x".into()))
    );
    assert!(!git(&r.remote, &["tag", "--list", "v2"]).is_empty());
}

#[test]
fn a_rejected_push_goes_through_after_a_merge_update() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    r.their_commit("b.py", "b\nmore\n", "theirs");
    r.our_commit("a.py", "1\n2\n3\nours\n", "ours");

    let target = repo.default_push_target("main", &c).unwrap().unwrap();
    let rejected = repo.push(&spec("main", target.clone()), &c).unwrap();
    assert_eq!(rejected.outcome, PushOutcome::RejectedNoFastForward);
    assert_eq!(rejected.description(), "Push to origin/main was rejected");

    let update = repo.update(UpdateMethod::Merge, None, &c).unwrap();
    assert!(
        matches!(
            update,
            UpdateResult::Updated {
                files: 1,
                restore_conflicts: false,
                ..
            }
        ),
        "{update:?}"
    );
    assert_eq!(read(&r.ours, "b.py"), "b\nmore\n");
    // A merge commit joins both lines.
    assert_eq!(
        git(&r.ours, &["rev-list", "--parents", "-1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
    let pushed = repo.push(&spec("main", target), &c).unwrap();
    assert!(matches!(pushed.outcome, PushOutcome::Success { .. }));
    assert_eq!(r.remote_head("main"), git(&r.ours, &["rev-parse", "HEAD"]));
}

#[test]
fn a_rebase_update_keeps_the_local_changes() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    r.their_commit("b.py", "b\ntheirs\n", "theirs");
    r.our_commit("c.py", "c\n", "ours");
    // An uncommitted change of a file the incoming commit changes too.
    write(&r.ours, "b.py", "b\nlocal\n");
    write(&r.ours, "a.py", "1\n2\n3\nlocal\n");
    git(&r.ours, &["checkout", "-q", "--", "b.py"]);

    let update = repo.update(UpdateMethod::Rebase, None, &c).unwrap();
    let UpdateResult::Updated {
        restore_conflicts, ..
    } = update
    else {
        panic!("{update:?}");
    };
    assert!(!restore_conflicts);
    assert_eq!(read(&r.ours, "a.py"), "1\n2\n3\nlocal\n", "restored");
    assert_eq!(read(&r.ours, "b.py"), "b\ntheirs\n");
    // Rebased: no merge commit, ours on top of theirs.
    let log = git(&r.ours, &["log", "--format=%s"]);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec!["ours", "theirs", "first"]
    );
    assert!(
        git(&r.ours, &["stash", "list"]).is_empty(),
        "the stash is gone"
    );

    // Nothing new: up to date.
    assert_eq!(
        repo.update(UpdateMethod::Rebase, None, &c).unwrap(),
        UpdateResult::UpToDate
    );
}

#[test]
fn conflicts_leave_the_repository_as_they_are() {
    let r = Remotes::new();
    let c = Cancel::new();
    r.their_commit("a.py", "1\ntheirs\n3\n", "theirs");
    r.our_commit("a.py", "1\nours\n3\n", "ours");
    let repo = r.repo();
    let merge = repo.update(UpdateMethod::Merge, None, &c).unwrap();
    assert_eq!(
        merge,
        UpdateResult::Incomplete {
            method: UpdateMethod::Merge,
            stashed: false
        }
    );
    assert_eq!(repo.info().unwrap().state, RepoState::Merging);
    // A second update refuses to start.
    let again = repo.update(UpdateMethod::Merge, None, &c).unwrap();
    assert!(
        matches!(&again, UpdateResult::NotReady { message, .. } if message.contains("unfinished merge")),
        "{again:?}"
    );
    git(&r.ours, &["merge", "--abort"]);

    // With a rebase, and a local change put aside.
    write(&r.ours, "b.py", "b\nlocal\n");
    let rebase = repo.update(UpdateMethod::Rebase, None, &c).unwrap();
    assert_eq!(
        rebase,
        UpdateResult::Incomplete {
            method: UpdateMethod::Rebase,
            stashed: true
        }
    );
    assert_eq!(repo.info().unwrap().state, RepoState::Rebasing);
    assert!(
        !git(&r.ours, &["stash", "list"]).is_empty(),
        "kept in the stash"
    );
}

#[test]
fn an_update_needs_a_branch_with_an_upstream() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    git(&r.ours, &["checkout", "-q", "-b", "local-only"]);
    assert_eq!(
        repo.update(UpdateMethod::Merge, None, &c).unwrap(),
        UpdateResult::NotReady {
            title: "Cannot update".into(),
            message: "local-only has no tracked branch".into()
        }
    );
    // Pull into the current branch from a chosen remote branch.
    r.their_commit("b.py", "b\npulled\n", "theirs");
    let pulled = repo
        .update(UpdateMethod::Rebase, Some(("origin", "main")), &c)
        .unwrap();
    assert!(
        matches!(pulled, UpdateResult::Updated { commits: 1, .. }),
        "{pulled:?}"
    );
    assert_eq!(read(&r.ours, "b.py"), "b\npulled\n");

    git(&r.ours, &["checkout", "-q", "--detach"]);
    let detached = repo.update(UpdateMethod::Merge, None, &c).unwrap();
    assert!(
        matches!(&detached, UpdateResult::NotReady { title, .. } if title == "Cannot Update: No Current Branch"),
        "{detached:?}"
    );
}

#[test]
fn fetch_prunes_and_updates_other_branches() {
    let r = Remotes::new();
    let c = Cancel::new();
    git(&r.theirs, &["push", "-q", "origin", "HEAD:refs/heads/old"]);
    git(
        &r.theirs,
        &["push", "-q", "origin", "HEAD:refs/heads/other"],
    );
    let repo = r.repo();
    let fetched = repo.fetch_all(&c).unwrap();
    assert!(fetched.is_success());
    assert!(
        repo.resolve_commit("refs/remotes/origin/old", &c)
            .unwrap()
            .is_some()
    );

    git(&r.theirs, &["push", "-q", "origin", "--delete", "old"]);
    let fetched = repo.fetch_all(&c).unwrap();
    assert_eq!(fetched.remotes[0].pruned, vec!["origin/old".to_owned()]);
    assert_eq!(
        fetched.message(),
        "Pruned obsolete remote reference: origin/old"
    );

    // A branch that is not checked out is fast-forwarded to its upstream.
    git(
        &r.ours,
        &["branch", "-q", "other", "--track", "origin/other"],
    );
    git(
        &r.theirs,
        &["checkout", "-q", "-b", "other", "--track", "origin/other"],
    );
    r.their_commit("o.py", "o\n", "other work");
    let updated = repo.update_branch("other", &c).unwrap();
    assert_eq!(updated.error, None);
    assert_eq!(
        git(&r.ours, &["rev-parse", "other"]),
        r.remote_head("other")
    );

    // A remote that is not there: git's message.
    git(&r.ours, &["remote", "add", "gone", "../no-such.git"]);
    let failed = repo.fetch("gone", None, &c).unwrap();
    assert!(failed.error.is_some());
}

#[test]
fn force_with_lease_protects_unfetched_commits() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    r.our_commit("a.py", "1\n", "mine");
    let target = repo.default_push_target("main", &c).unwrap().unwrap();
    repo.push(&spec("main", target.clone()), &c).unwrap();
    git(&r.ours, &["commit", "-q", "--amend", "-m", "mine, amended"]);

    let mut forced = spec("main", target.clone());
    forced.force = ForceMode::WithLease;
    let result = repo.push(&forced, &c).unwrap();
    assert_eq!(result.outcome, PushOutcome::Forced);
    assert_eq!(result.description(), "Force pushed main to origin/main");

    // The colleague pushed something we never fetched: the lease fails.
    git(&r.theirs, &["fetch", "-q"]);
    git(&r.theirs, &["reset", "-q", "--hard", "origin/main"]);
    r.their_commit("t.py", "t\n", "theirs");
    git(&r.ours, &["commit", "-q", "--amend", "-m", "mine again"]);
    let stale = repo.push(&forced, &c).unwrap();
    assert_eq!(stale.outcome, PushOutcome::RejectedStaleInfo);
    forced.force = ForceMode::Force;
    assert_eq!(repo.push(&forced, &c).unwrap().outcome, PushOutcome::Forced);
}

#[test]
fn a_push_to_a_missing_remote_is_an_error() {
    let r = Remotes::new();
    let repo = r.repo();
    let c = Cancel::new();
    git(&r.ours, &["remote", "add", "gone", "../no-such.git"]);
    let target = repo.push_target("gone", "main", &c).unwrap();
    assert!(target.new);
    let result = repo.push(&spec("main", target), &c).unwrap();
    let PushOutcome::Error { message } = &result.outcome else {
        panic!("{result:?}");
    };
    assert!(!message.is_empty());
}
