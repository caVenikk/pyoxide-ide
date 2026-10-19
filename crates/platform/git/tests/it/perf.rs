// Measurements for `replica/perf.md` (ADR 0016 budgets), not run by default:
//
//     cargo test -p git --release --test it perf:: -- --ignored --nocapture

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use git::{GitExe, GitRepo, Repository, StatusService, StatusUpdate};
use vcs::{Backend, Cancel, FileStatus};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new(&GitExe::find(None).unwrap().path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false"])
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

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

#[test]
#[ignore]
fn status_of_a_10k_file_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = std::fs::canonicalize(tmp.path()).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    for d in 0..100 {
        let folder = dir.join(format!("pkg{d:03}"));
        std::fs::create_dir_all(&folder).unwrap();
        for f in 0..100 {
            std::fs::write(
                folder.join(format!("m{f:03}.py")),
                format!("x = {d} * {f}\n"),
            )
            .unwrap();
        }
    }
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "files"]);
    let changed: Vec<_> = (0..5)
        .map(|i| dir.join(format!("pkg{:03}", i * 20)).join("m001.py"))
        .collect();
    for path in &changed {
        std::fs::write(path, "changed = True\n").unwrap();
    }
    let repo = Arc::new(GitRepo::new(
        Repository::detect(&dir).unwrap(),
        GitExe::find(None).unwrap(),
    ));
    // Warm the file system caches and the index.
    repo.status(None, &Cancel::new()).unwrap();

    let full = median(
        (0..7)
            .map(|_| {
                let t = Instant::now();
                let s = repo.status(None, &Cancel::new()).unwrap();
                assert_eq!(s.status.changes().len(), 5);
                t.elapsed()
            })
            .collect(),
    );
    let limited = median(
        (0..7)
            .map(|_| {
                let t = Instant::now();
                repo.status(Some(&changed), &Cancel::new()).unwrap();
                t.elapsed()
            })
            .collect(),
    );

    // Through the service: from the end of the debounce after a watcher
    // batch to the merged snapshot.
    let service = StatusService::start(repo.clone());
    let updates = service.updates();
    let _ = updates.recv_timeout(Duration::from_secs(30)).unwrap();
    let mut batches = Vec::new();
    for round in 0..5 {
        let path = dir.join("pkg050").join(format!("m{round:03}.py"));
        std::fs::write(&path, format!("round = {round}\n")).unwrap();
        let t = Instant::now();
        service.paths_changed(vec![path.clone()]);
        let StatusUpdate::Snapshot(s) = updates.recv_timeout(Duration::from_secs(30)).unwrap()
        else {
            panic!("status failed");
        };
        assert_eq!(s.status.status(&path), FileStatus::Modified);
        batches.push(t.elapsed().saturating_sub(git::DEBOUNCE));
    }
    let after_batch = median(batches);
    eprintln!(
        "git {}: full status {:?}, limited status (5 paths) {:?}, watcher batch to snapshot after the debounce {:?}",
        repo.runner().exe().version,
        full,
        limited,
        after_batch
    );
}
