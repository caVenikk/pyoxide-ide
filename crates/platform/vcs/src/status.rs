use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The status of a file in a repository: the reference's `FileStatus`
/// values that Git produces (renamed is separate here, so that the Commit
/// tool window can say "renamed from").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// Tracked and unchanged, or outside any repository.
    NotChanged,
    Modified,
    Added,
    Deleted,
    /// Moved or renamed (its old path is in the [`Change`]).
    Renamed,
    /// Not tracked and not ignored ("unknown" in the reference).
    Unversioned,
    Ignored,
    /// Unmerged: a merge, rebase or cherry-pick left a conflict.
    Conflict,
}

impl FileStatus {
    /// The stable name used in JSON (`modified`, `unversioned`, ...).
    pub fn id(self) -> &'static str {
        match self {
            Self::NotChanged => "not_changed",
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Unversioned => "unversioned",
            Self::Ignored => "ignored",
            Self::Conflict => "conflict",
        }
    }

    /// A change the Commit tool window lists under "Changes".
    pub fn is_change(self) -> bool {
        matches!(
            self,
            Self::Modified | Self::Added | Self::Deleted | Self::Renamed | Self::Conflict
        )
    }
}

/// A changed file of the work tree against `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Absolute path in the work tree (the old one for a deleted file).
    pub path: PathBuf,
    pub status: FileStatus,
    /// The path before a rename.
    pub old_path: Option<PathBuf>,
}

/// The statuses of a repository's files.
///
/// Only paths with a status other than [`FileStatus::NotChanged`] are kept;
/// an ignored folder is kept once and counts for everything below it (the
/// reference's `hasAncestor` rule for ignored files).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusMap {
    root: PathBuf,
    files: HashMap<PathBuf, FileStatus>,
    /// New path -> old path of renames.
    renames: HashMap<PathBuf, PathBuf>,
    ignored_dirs: HashSet<PathBuf>,
}

impl StatusMap {
    /// An empty map of the work tree at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            ..Self::default()
        }
    }

    /// The work tree's root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Records the status of a file.
    pub fn insert(&mut self, path: PathBuf, status: FileStatus) {
        if status == FileStatus::NotChanged {
            self.files.remove(&path);
        } else {
            self.files.insert(path, status);
        }
    }

    /// Records a rename.
    pub fn insert_rename(&mut self, path: PathBuf, old_path: PathBuf) {
        self.renames.insert(path.clone(), old_path);
        self.files.insert(path, FileStatus::Renamed);
    }

    /// Records an ignored folder: everything below it is ignored.
    pub fn insert_ignored_dir(&mut self, path: PathBuf) {
        self.ignored_dirs.insert(path);
    }

    /// The status of a file or folder. Paths outside the work tree are
    /// not changed.
    pub fn status(&self, path: &Path) -> FileStatus {
        if !path.starts_with(&self.root) {
            return FileStatus::NotChanged;
        }
        if let Some(status) = self.files.get(path) {
            return *status;
        }
        if self.is_below_ignored(path) {
            return FileStatus::Ignored;
        }
        FileStatus::NotChanged
    }

    /// The status of a file whose open document may have unsaved changes:
    /// an unsaved, clean tracked file is modified at once
    /// (`FileStatusManagerImpl`).
    pub fn document_status(&self, path: &Path, unsaved: bool) -> FileStatus {
        let status = self.status(path);
        if unsaved && status == FileStatus::NotChanged && path.starts_with(&self.root) {
            FileStatus::Modified
        } else {
            status
        }
    }

    fn is_below_ignored(&self, path: &Path) -> bool {
        if self.ignored_dirs.is_empty() {
            return false;
        }
        path.ancestors()
            .take_while(|a| a.starts_with(&self.root) && *a != self.root)
            .any(|a| self.ignored_dirs.contains(a))
    }

    /// The path a renamed file had.
    pub fn old_path(&self, path: &Path) -> Option<&Path> {
        self.renames.get(path).map(PathBuf::as_path)
    }

    /// The changes against `HEAD` (no unversioned or ignored files), by
    /// path.
    pub fn changes(&self) -> Vec<Change> {
        self.with(FileStatus::is_change)
    }

    /// Files that are not tracked and not ignored, by path.
    pub fn unversioned(&self) -> Vec<PathBuf> {
        self.paths(|s| s == FileStatus::Unversioned)
    }

    /// Ignored files and folders, by path.
    pub fn ignored(&self) -> Vec<PathBuf> {
        let mut out = self.paths(|s| s == FileStatus::Ignored);
        out.extend(self.ignored_dirs.iter().cloned());
        out.sort();
        out
    }

    /// Every file with a status (ignored folders left out), by path.
    pub fn entries(&self) -> Vec<Change> {
        self.with(|_| true)
    }

    fn with(&self, keep: impl Fn(FileStatus) -> bool) -> Vec<Change> {
        let mut out: Vec<Change> = self
            .files
            .iter()
            .filter(|(_, s)| keep(**s))
            .map(|(path, status)| Change {
                path: path.clone(),
                status: *status,
                old_path: self.renames.get(path).cloned(),
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn paths(&self, keep: impl Fn(FileStatus) -> bool) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(_, s)| keep(**s))
            .map(|(p, _)| p.clone())
            .collect();
        out.sort();
        out
    }

    /// Number of files with a status.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.ignored_dirs.is_empty()
    }

    /// Takes the result of a status limited to `paths` (files or folders):
    /// what was known at or below them is replaced by `fresh`. A rename
    /// whose either end is in `paths` is replaced too.
    pub fn merge(&mut self, paths: &[PathBuf], fresh: StatusMap) {
        let covered = |p: &Path| paths.iter().any(|q| p.starts_with(q));
        let stale_renames: Vec<PathBuf> = self
            .renames
            .iter()
            .filter(|(new, old)| covered(new) || covered(old))
            .map(|(new, _)| new.clone())
            .collect();
        for new in stale_renames {
            self.renames.remove(&new);
            self.files.remove(&new);
        }
        self.files.retain(|p, _| !covered(p));
        self.ignored_dirs.retain(|p| !covered(p));
        self.files.extend(fresh.files);
        self.renames.extend(fresh.renames);
        self.ignored_dirs.extend(fresh.ignored_dirs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        Path::new("/r").join(s)
    }

    #[test]
    fn ignored_folders_count_for_everything_below_them() {
        let mut m = StatusMap::new("/r");
        m.insert_ignored_dir(p("logs"));
        m.insert(p("a.py"), FileStatus::Modified);
        m.insert(p(".env"), FileStatus::Ignored);
        assert_eq!(m.status(&p("logs")), FileStatus::Ignored);
        assert_eq!(m.status(&p("logs/x/y.log")), FileStatus::Ignored);
        assert_eq!(m.status(&p("logsx")), FileStatus::NotChanged);
        assert_eq!(m.status(&p(".env")), FileStatus::Ignored);
        assert_eq!(m.status(&p("a.py")), FileStatus::Modified);
        assert_eq!(
            m.status(Path::new("/elsewhere/a.py")),
            FileStatus::NotChanged
        );
        assert_eq!(m.ignored(), vec![p(".env"), p("logs")]);
    }

    #[test]
    fn an_unsaved_clean_tracked_file_is_modified() {
        let mut m = StatusMap::new("/r");
        m.insert(p("new.py"), FileStatus::Unversioned);
        m.insert(p("added.py"), FileStatus::Added);
        assert_eq!(m.document_status(&p("a.py"), true), FileStatus::Modified);
        assert_eq!(m.document_status(&p("a.py"), false), FileStatus::NotChanged);
        assert_eq!(
            m.document_status(&p("new.py"), true),
            FileStatus::Unversioned
        );
        assert_eq!(m.document_status(&p("added.py"), true), FileStatus::Added);
        assert_eq!(
            m.document_status(Path::new("/elsewhere/a.py"), true),
            FileStatus::NotChanged
        );
    }

    #[test]
    fn a_limited_status_replaces_what_is_below_its_paths() {
        let mut m = StatusMap::new("/r");
        m.insert(p("a.py"), FileStatus::Modified);
        m.insert(p("src/b.py"), FileStatus::Modified);
        m.insert_rename(p("new.md"), p("old.md"));
        m.insert_ignored_dir(p("src/build"));
        let mut fresh = StatusMap::new("/r");
        fresh.insert(p("src/c.py"), FileStatus::Unversioned);
        m.merge(&[p("src"), p("old.md")], fresh);
        assert_eq!(m.status(&p("a.py")), FileStatus::Modified);
        assert_eq!(m.status(&p("src/b.py")), FileStatus::NotChanged);
        assert_eq!(m.status(&p("src/c.py")), FileStatus::Unversioned);
        assert_eq!(m.status(&p("src/build/x")), FileStatus::NotChanged);
        assert_eq!(m.status(&p("new.md")), FileStatus::NotChanged);
        assert_eq!(m.old_path(&p("new.md")), None);
    }

    #[test]
    fn changes_leave_out_unversioned_and_ignored_files() {
        let mut m = StatusMap::new("/r");
        m.insert(p("b.py"), FileStatus::Added);
        m.insert(p("a.py"), FileStatus::Modified);
        m.insert(p("c.py"), FileStatus::Unversioned);
        m.insert_rename(p("d.md"), p("e.md"));
        let changes = m.changes();
        let paths: Vec<_> = changes.iter().map(|c| c.path.clone()).collect();
        assert_eq!(paths, vec![p("a.py"), p("b.py"), p("d.md")]);
        assert_eq!(changes[2].old_path, Some(p("e.md")));
        assert_eq!(m.unversioned(), vec![p("c.py")]);
    }
}
