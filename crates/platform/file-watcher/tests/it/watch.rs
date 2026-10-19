//! The watcher on a real file system: changes made by another writer (this
//! test, as a stand-in for `git` or a formatter) come as batches. Systems
//! deliver events late and coalesce them, so every check waits for the
//! batch it needs with a generous timeout and accepts any path form the
//! system reports (`/private/var` on macOS, verbatim paths on Windows).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use file_watcher::{Batch, FileWatcher, Kind};

/// How long a change may take to be reported.
const TIMEOUT: Duration = Duration::from_secs(20);

fn root(dir: &tempfile::TempDir) -> PathBuf {
    native_path::canonicalize(dir.path()).unwrap()
}

/// The path of an event is `rel` under the watched folder.
fn is(path: &Path, rel: &str) -> bool {
    path.ends_with(rel)
}

/// Waits for batches until one has `rel` with a kind `ok` accepts; returns
/// every batch seen.
fn wait_for(watcher: &FileWatcher, rel: &str, ok: impl Fn(Kind) -> bool) -> Vec<Batch> {
    let events = watcher.events();
    let start = Instant::now();
    let mut seen = Vec::new();
    while let Some(left) = TIMEOUT.checked_sub(start.elapsed()) {
        let Ok(batch) = events.recv_timeout(left) else {
            break;
        };
        let found = batch.paths.iter().any(|e| is(&e.path, rel) && ok(e.kind));
        seen.push(batch);
        if found {
            return seen;
        }
    }
    panic!("no event for {rel} within {TIMEOUT:?}; batches: {seen:#?}");
}

fn watcher(root: &Path) -> FileWatcher {
    let mut w = FileWatcher::new(|p: &Path| {
        p.components()
            .any(|c| c.as_os_str() == ".git" || c.as_os_str() == "ignored.tmp")
    })
    .unwrap();
    w.watch(root, true).unwrap();
    w
}

#[test]
fn reports_created_changed_and_removed_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = root(&dir);
    std::fs::write(root.join("old.py"), "x = 1\n").unwrap();
    let w = watcher(&root);

    std::fs::write(root.join("new.py"), "y = 2\n").unwrap();
    wait_for(&w, "new.py", |_| true);

    std::fs::write(root.join("old.py"), "x = 2\n").unwrap();
    wait_for(&w, "old.py", |_| true);

    std::fs::remove_file(root.join("new.py")).unwrap();
    wait_for(&w, "new.py", |k| k == Kind::Removed);
    drop(w);
}

#[test]
fn reports_both_ends_of_a_rename_and_new_folders() {
    let dir = tempfile::tempdir().unwrap();
    let root = root(&dir);
    std::fs::write(root.join("a.py"), "a\n").unwrap();
    let w = watcher(&root);

    std::fs::rename(root.join("a.py"), root.join("b.py")).unwrap();
    let mut seen = wait_for(&w, "b.py", |k| k != Kind::Removed);
    if !seen
        .iter()
        .flat_map(|b| &b.paths)
        .any(|e| is(&e.path, "a.py"))
    {
        seen = wait_for(&w, "a.py", |_| true);
    }
    assert!(!seen.is_empty());

    // A folder made after the watch started is watched too.
    std::fs::create_dir(root.join("pkg")).unwrap();
    wait_for(&w, "pkg", |_| true);
    let file = root.join("pkg").join("m.py");
    std::fs::write(&file, "m\n").unwrap();
    // Its first file may come with the folder (inotify adds the folder's
    // watch after it was made); a later change always comes.
    std::fs::write(&file, "m = 1\n").unwrap();
    wait_for(&w, "pkg/m.py", |_| true);
    drop(w);
}

#[test]
fn ignored_paths_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let root = root(&dir);
    std::fs::create_dir(root.join(".git")).unwrap();
    let w = watcher(&root);

    std::fs::write(root.join(".git").join("index"), "x").unwrap();
    std::fs::write(root.join("ignored.tmp"), "x").unwrap();
    // A later change marks the end: everything before it was reported.
    std::fs::write(root.join("marker.py"), "x").unwrap();
    let seen = wait_for(&w, "marker.py", |_| true);
    for event in seen.iter().flat_map(|b| &b.paths) {
        assert!(
            !is(&event.path, ".git/index") && !is(&event.path, "ignored.tmp"),
            "{event:?} is ignored"
        );
    }
    drop(w);
}

#[test]
fn dropping_the_watcher_ends_its_events() {
    let dir = tempfile::tempdir().unwrap();
    let root = root(&dir);
    let w = watcher(&root);
    let events = w.events();
    drop(w);
    let start = Instant::now();
    loop {
        match events.recv_timeout(TIMEOUT) {
            Err(flume::RecvTimeoutError::Disconnected) => break,
            Err(flume::RecvTimeoutError::Timeout) => panic!("the events did not end"),
            Ok(_) => assert!(start.elapsed() < TIMEOUT, "the events did not end"),
        }
    }
}
