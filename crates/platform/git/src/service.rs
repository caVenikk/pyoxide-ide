use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use vcs::{Backend, Cancel, FileStatus, Snapshot, StatusMap, VcsError};

use crate::backend::{GitRepo, MAX_PATHSPECS};

/// Status runs this long after the last change reported (the reference's
/// `UpdateRequestsQueue`).
pub const DEBOUNCE: Duration = Duration::from_millis(300);

enum Msg {
    Paths(Vec<PathBuf>),
    Refresh,
    WriteFinished,
    Stop,
}

/// What the service tells its owner.
#[derive(Debug, Clone)]
pub enum StatusUpdate {
    /// The repository as it is now: a full status, or a limited one
    /// merged into the last.
    Snapshot(Arc<Snapshot>),
    /// A status failed (git's message).
    Failed(VcsError),
}

/// Keeps the statuses of one repository current: changed paths reported
/// by the file watcher are marked dirty, a status runs [`DEBOUNCE`] after
/// the last report, limited to the dirty paths when they are few, and the
/// result merged into the status map goes out as one update. A change of
/// `HEAD`, the index, refs, the config or a `.gitignore` makes the next
/// status a full one. While a write command of the application runs, its
/// own changes are not reported; one full status follows it.
///
/// The work runs on a thread of its own; dropping the service stops it.
pub struct StatusService {
    repo: Arc<GitRepo>,
    tx: flume::Sender<Msg>,
    updates: flume::Receiver<StatusUpdate>,
    running: Arc<Mutex<Option<Cancel>>>,
    last: Arc<Mutex<Option<Arc<Snapshot>>>>,
}

impl StatusService {
    /// Starts the service; a full status runs at once.
    pub fn start(repo: Arc<GitRepo>) -> Self {
        let (tx, rx) = flume::unbounded();
        let (out, updates) = flume::unbounded();
        let running = Arc::new(Mutex::new(None));
        let last = Arc::new(Mutex::new(None));
        let notify = tx.clone();
        repo.runner().on_write_finished(move || {
            let _ = notify.send(Msg::WriteFinished);
        });
        let worker = Worker {
            repo: repo.clone(),
            out,
            running: running.clone(),
            last: last.clone(),
            map: None,
        };
        std::thread::Builder::new()
            .name("git-status".to_owned())
            .spawn(move || worker.run(&rx))
            .expect("a thread for git status");
        Self {
            repo,
            tx,
            updates,
            running,
            last,
        }
    }

    pub fn repo(&self) -> &Arc<GitRepo> {
        &self.repo
    }

    /// The updates, in order.
    pub fn updates(&self) -> flume::Receiver<StatusUpdate> {
        self.updates.clone()
    }

    /// The last snapshot sent.
    pub fn last(&self) -> Option<Arc<Snapshot>> {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Paths changed on disk (a file watcher batch): files of the work
    /// tree and files of the repository folder. A running status is stale
    /// and stops.
    pub fn paths_changed(&self, paths: Vec<PathBuf>) {
        let repo = self.repo.repository();
        let relevant: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| p.starts_with(&repo.root) || repo.is_state_path(p))
            .collect();
        if relevant.is_empty() {
            return;
        }
        if let Some(cancel) = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            cancel.cancel();
        }
        let _ = self.tx.send(Msg::Paths(relevant));
    }

    /// Looks at the whole work tree again, without waiting.
    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }
}

impl Drop for StatusService {
    /// Stops the thread (the runner's write hook keeps a sender of its
    /// own) and a status that runs.
    fn drop(&mut self) {
        if let Some(cancel) = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            cancel.cancel();
        }
        let _ = self.tx.send(Msg::Stop);
    }
}

struct Worker {
    repo: Arc<GitRepo>,
    out: flume::Sender<StatusUpdate>,
    running: Arc<Mutex<Option<Cancel>>>,
    last: Arc<Mutex<Option<Arc<Snapshot>>>>,
    map: Option<StatusMap>,
}

impl Worker {
    fn run(mut self, rx: &flume::Receiver<Msg>) {
        let mut dirty: BTreeSet<PathBuf> = BTreeSet::new();
        let mut full = true;
        let mut deadline = Some(Instant::now());
        // The previous status was cancelled: the next one runs to the end,
        // so that a stream of changes cannot hold statuses back.
        let mut cancelled_last = false;
        loop {
            let msg = match deadline {
                Some(d) => rx.recv_deadline(d),
                None => rx.recv().map_err(|_| flume::RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(Msg::Paths(paths)) => {
                    let writing = self.repo.runner().is_writing();
                    for path in paths {
                        if writing || self.makes_full(&path) {
                            full = true;
                        } else if !self.below_ignored(&path) {
                            dirty.insert(path);
                        }
                    }
                    // A write's own changes wait for its end.
                    if !writing {
                        deadline = Some(Instant::now() + DEBOUNCE);
                    }
                }
                Ok(Msg::Refresh | Msg::WriteFinished) => {
                    full = true;
                    deadline = Some(Instant::now());
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    let paths = self.limited(&dirty, full);
                    let cancel = Cancel::new();
                    if !cancelled_last {
                        *self.running.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(cancel.clone());
                    }
                    let result = self.repo.status(paths.as_deref(), &cancel);
                    *self.running.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    match result {
                        Err(e) if e.is_cancelled() => {
                            cancelled_last = true;
                            deadline = Some(Instant::now() + DEBOUNCE);
                            continue;
                        }
                        Err(e) => {
                            if self.out.send(StatusUpdate::Failed(e)).is_err() {
                                return;
                            }
                        }
                        Ok(snapshot) => {
                            let snapshot = Arc::new(self.merge(paths.as_deref(), snapshot));
                            *self.last.lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(snapshot.clone());
                            if self.out.send(StatusUpdate::Snapshot(snapshot)).is_err() {
                                return;
                            }
                        }
                    }
                    cancelled_last = false;
                    dirty.clear();
                    full = false;
                    deadline = None;
                }
                Ok(Msg::Stop) | Err(flume::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// A change of this path can change any status.
    fn makes_full(&self, path: &Path) -> bool {
        let repo = self.repo.repository();
        repo.is_state_path(path)
            || path == repo.git_dir
            || path.file_name().is_some_and(|n| n == ".gitignore")
    }

    /// Below a folder that is ignored as a whole: its status stays.
    fn below_ignored(&self, path: &Path) -> bool {
        self.map.as_ref().is_some_and(|m| {
            path.parent()
                .is_some_and(|parent| m.status(parent) == FileStatus::Ignored)
        })
    }

    /// The paths of a limited status, `None` for a full one.
    fn limited(&self, dirty: &BTreeSet<PathBuf>, full: bool) -> Option<Vec<PathBuf>> {
        if full || self.map.is_none() || dirty.len() > MAX_PATHSPECS {
            return None;
        }
        let map = self.map.as_ref()?;
        let mut paths: Vec<PathBuf> = dirty.iter().cloned().collect();
        // Both ends of a rename, so that it stays one.
        for change in map.changes() {
            if let Some(old) = &change.old_path
                && (dirty.contains(&change.path) || dirty.contains(old))
            {
                paths.push(change.path.clone());
                paths.push(old.clone());
            }
        }
        paths.sort();
        paths.dedup();
        Some(paths)
    }

    fn merge(&mut self, paths: Option<&[PathBuf]>, mut snapshot: Snapshot) -> Snapshot {
        if let (Some(paths), Some(mut map)) = (paths, self.map.take()) {
            map.merge(paths, snapshot.status);
            snapshot.status = map;
        }
        self.map = Some(snapshot.status.clone());
        snapshot
    }
}
