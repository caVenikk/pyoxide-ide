use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Branch, RepoInfo, StatusMap};

/// Stops a running operation: a backend checks it and stops its process.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A failed version control operation. `message` is the backend's own
/// text (git's error output), shown to people as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VcsError {
    pub kind: VcsErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsErrorKind {
    /// The operation was cancelled.
    Cancelled,
    /// The backend's program could not run.
    Unavailable,
    /// The backend reported an error.
    Failed,
}

impl VcsError {
    pub fn cancelled() -> Self {
        Self {
            kind: VcsErrorKind::Cancelled,
            message: "cancelled".to_owned(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: VcsErrorKind::Unavailable,
            message: message.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: VcsErrorKind::Failed,
            message: message.into(),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.kind == VcsErrorKind::Cancelled
    }
}

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VcsError {}

/// What a repository is now: its state and the statuses of its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub info: RepoInfo,
    pub status: StatusMap,
    /// Counts the snapshots of a repository, so that a later one is known.
    pub generation: u64,
}

/// A version control backend for one repository. Every method may run
/// external processes: call them off the UI thread.
pub trait Backend: Send + Sync {
    /// The name shown for the version control (`Git`).
    fn name(&self) -> &'static str;

    /// The work tree's root.
    fn root(&self) -> &Path;

    /// Where the repository is now, from its files.
    fn info(&self) -> Result<RepoInfo, VcsError>;

    /// The statuses of the work tree's files, or of the files at or below
    /// `paths` only, with the current branch's upstream.
    fn status(&self, paths: Option<&[PathBuf]>, cancel: &Cancel) -> Result<Snapshot, VcsError>;

    /// The text of a file in `HEAD` as it would be checked out (line
    /// endings and filters applied), `None` when `HEAD` has no such file.
    fn base_text(&self, path: &Path, cancel: &Cancel) -> Result<Option<Arc<str>>, VcsError>;

    /// Local and remote branches and tags.
    fn branches(&self, cancel: &Cancel) -> Result<Vec<Branch>, VcsError>;
}
