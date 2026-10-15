//! The file watcher of Pyoxide IDE: what changes on disk under watched
//! folders, as the operating system reports it (FSEvents on macOS, inotify
//! on Linux, ReadDirectoryChangesW on Windows, through `notify`).
//!
//! The reference IDE runs a native helper (`fsnotifier`) that marks paths
//! dirty; its virtual file system then looks at those paths on disk and
//! tells the rest of the IDE what changed. [`FileWatcher`] plays the
//! helper's part: the paths reported in a short burst (a `git checkout`,
//! a formatter run) come as one [`Batch`], each path once with a hint of
//! what happened ([`Kind`]). The hint is only a hint: systems coalesce
//! events and some report them imprecisely, so the receiver looks at the
//! disk for what a path is now. When the system lost events (a queue
//! overflow, an error) the batch asks for a rescan.
//!
//! No UI toolkit here; the watcher runs on its own threads and the batches
//! come through a channel ([`FileWatcher::events`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher as _};

/// A burst of events ends after this long without another one.
pub const QUIET: Duration = Duration::from_millis(50);
/// A batch goes out at the latest this long after its first event, even
/// while events keep coming.
pub const MAX_DELAY: Duration = Duration::from_millis(500);

/// What the system reported for a path, coalesced over a batch: the last
/// of a creation or a removal, or else a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Created, or renamed or moved to this path.
    Created,
    /// Removed, or renamed or moved away.
    Removed,
    /// Content or metadata changed.
    Changed,
}

/// A path reported in a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEvent {
    pub path: PathBuf,
    pub kind: Kind,
}

/// Paths that changed on disk in a burst of events, each once, in the
/// order they were first reported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub paths: Vec<PathEvent>,
    /// Events were lost: what is under the watched folders has to be
    /// looked at again.
    pub rescan: bool,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && !self.rescan
    }

    /// The event of `path`, when it is in the batch.
    pub fn get(&self, path: &Path) -> Option<&PathEvent> {
        self.paths.iter().find(|e| e.path == path)
    }
}

/// Watching could not start.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("{0}")]
    Notify(#[from] notify::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Watches folders and reports the paths that change under them.
///
/// Dropping the watcher stops it; the channel of [`FileWatcher::events`]
/// then ends after the last batch.
pub struct FileWatcher {
    watcher: notify::RecommendedWatcher,
    events: flume::Receiver<Batch>,
}

impl FileWatcher {
    /// A watcher that watches nothing yet. Paths for which `ignore` is
    /// true are left out of the batches (a version control folder, the
    /// application's own temporary files); it runs on the watcher's
    /// thread.
    pub fn new(ignore: impl Fn(&Path) -> bool + Send + 'static) -> Result<Self, WatchError> {
        let (raw_tx, raw_rx) = flume::unbounded();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = raw_tx.send(event);
        })?;
        let (tx, events) = flume::unbounded();
        std::thread::Builder::new()
            .name("file-watcher".to_owned())
            .spawn(move || gather(&raw_rx, &tx, &ignore))?;
        Ok(Self { watcher, events })
    }

    /// Starts watching a folder (or a file), with everything below it when
    /// `recursive`. On Linux a recursive watch sets up one watch per folder
    /// and may take a while on a large tree: call it off the UI thread.
    pub fn watch(&mut self, path: &Path, recursive: bool) -> Result<(), WatchError> {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        Ok(self.watcher.watch(path, mode)?)
    }

    /// Stops watching a path given to [`FileWatcher::watch`].
    pub fn unwatch(&mut self, path: &Path) -> Result<(), WatchError> {
        Ok(self.watcher.unwatch(path)?)
    }

    /// The batches of changed paths.
    pub fn events(&self) -> flume::Receiver<Batch> {
        self.events.clone()
    }
}

/// The batch being gathered.
#[derive(Default)]
struct Pending {
    batch: Batch,
    index: HashMap<PathBuf, usize>,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Pending {
    fn add(&mut self, path: PathBuf, kind: Kind) {
        match self.index.get(&path) {
            Some(&i) => {
                let event = &mut self.batch.paths[i];
                // A creation or removal outweighs a change.
                if kind != Kind::Changed {
                    event.kind = kind;
                }
            }
            None => {
                self.index.insert(path.clone(), self.batch.paths.len());
                self.batch.paths.push(PathEvent { path, kind });
            }
        }
    }

    fn touched(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// When the batch goes out unless more events come.
    fn deadline(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        Some((last + QUIET).min(first + MAX_DELAY))
    }

    fn take(&mut self) -> Batch {
        let batch = std::mem::take(&mut self.batch);
        *self = Self::default();
        batch
    }
}

/// The hint of what happened to the paths of an event; `None` for events
/// that change nothing (reads).
fn kinds(kind: &EventKind, paths: usize) -> Option<Vec<Kind>> {
    let one = |k| Some(vec![k; paths]);
    match kind {
        EventKind::Access(_) => None,
        EventKind::Create(_) => one(Kind::Created),
        EventKind::Remove(_) => one(Kind::Removed),
        EventKind::Modify(ModifyKind::Name(mode)) => match mode {
            RenameMode::From => one(Kind::Removed),
            RenameMode::To => one(Kind::Created),
            // The old path, then the new one.
            RenameMode::Both => Some(
                (0..paths)
                    .map(|i| if i == 0 { Kind::Removed } else { Kind::Created })
                    .collect(),
            ),
            // Either end of a rename (FSEvents does not say which): the
            // path may now be new, so it counts as created.
            RenameMode::Any | RenameMode::Other => one(Kind::Created),
        },
        EventKind::Modify(_) | EventKind::Any | EventKind::Other => one(Kind::Changed),
    }
}

/// Gathers the raw events into batches until the watcher or the receiver
/// is gone.
fn gather(
    raw: &flume::Receiver<notify::Result<notify::Event>>,
    out: &flume::Sender<Batch>,
    ignore: &dyn Fn(&Path) -> bool,
) {
    let mut pending = Pending::default();
    loop {
        let next = match pending.deadline() {
            Some(deadline) => raw.recv_deadline(deadline),
            None => raw
                .recv()
                .map_err(|_| flume::RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Ok(event)) => {
                pending.touched(Instant::now());
                if event.need_rescan() {
                    pending.batch.rescan = true;
                }
                let Some(kinds) = kinds(&event.kind, event.paths.len()) else {
                    continue;
                };
                for (path, kind) in event.paths.into_iter().zip(kinds) {
                    if !ignore(&path) {
                        pending.add(path, kind);
                    }
                }
            }
            // Something went wrong in the system's watcher: events may be
            // missing.
            Ok(Err(_)) => {
                pending.touched(Instant::now());
                pending.batch.rescan = true;
            }
            Err(flume::RecvTimeoutError::Timeout) => {
                let batch = pending.take();
                if !batch.is_empty() && out.send(batch).is_err() {
                    return;
                }
            }
            Err(flume::RecvTimeoutError::Disconnected) => {
                let batch = pending.take();
                if !batch.is_empty() {
                    let _ = out.send(batch);
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_has_each_path_once_with_the_last_creation_or_removal() {
        let mut p = Pending::default();
        p.add("/p/a".into(), Kind::Changed);
        p.add("/p/b".into(), Kind::Created);
        p.add("/p/a".into(), Kind::Created);
        p.add("/p/a".into(), Kind::Changed);
        p.add("/p/b".into(), Kind::Removed);
        let batch = p.take();
        assert_eq!(
            batch.paths,
            vec![
                PathEvent {
                    path: "/p/a".into(),
                    kind: Kind::Created
                },
                PathEvent {
                    path: "/p/b".into(),
                    kind: Kind::Removed
                },
            ]
        );
        assert!(p.take().is_empty());
    }

    #[test]
    fn a_batch_goes_out_after_a_quiet_time_or_the_longest_delay() {
        let mut p = Pending::default();
        assert_eq!(p.deadline(), None);
        let t0 = Instant::now();
        p.touched(t0);
        assert_eq!(p.deadline(), Some(t0 + QUIET));
        let later = t0 + MAX_DELAY - Duration::from_millis(10);
        p.touched(later);
        assert_eq!(p.deadline(), Some(t0 + MAX_DELAY));
    }

    #[test]
    fn event_kinds_become_hints() {
        use notify::event::{AccessKind, CreateKind, DataChange, RemoveKind};
        assert_eq!(kinds(&EventKind::Access(AccessKind::Any), 1), None);
        assert_eq!(
            kinds(&EventKind::Create(CreateKind::File), 1),
            Some(vec![Kind::Created])
        );
        assert_eq!(
            kinds(&EventKind::Remove(RemoveKind::Folder), 1),
            Some(vec![Kind::Removed])
        );
        assert_eq!(
            kinds(&EventKind::Modify(ModifyKind::Name(RenameMode::Both)), 2),
            Some(vec![Kind::Removed, Kind::Created])
        );
        assert_eq!(
            kinds(&EventKind::Modify(ModifyKind::Name(RenameMode::Any)), 1),
            Some(vec![Kind::Created])
        );
        assert_eq!(
            kinds(&EventKind::Modify(ModifyKind::Data(DataChange::Any)), 1),
            Some(vec![Kind::Changed])
        );
    }
}
