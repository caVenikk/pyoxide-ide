//! File status colours (the reference's `FILESTATUS_*` scheme keys): the
//! names of files in the project tree, the editor tabs, Recent Files and
//! Go to File are drawn in the colour of their status. Folders are not
//! coloured, except an ignored folder (and everything below it, which is
//! ignored too). An open document counts with its unsaved text: a clean
//! tracked file is modified as soon as it is edited and is not modified
//! again when its text equals the committed one (`FileStatusManagerImpl`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Hsla};
use themes::UiRole;
use ui::theme::ThemeExt;
use vcs::{FileStatus, Snapshot};

/// The colour role of a status; `None` keeps the normal text colour.
pub fn status_role(status: FileStatus) -> Option<UiRole> {
    match status {
        FileStatus::NotChanged => None,
        FileStatus::Modified => Some(UiRole::GitModified),
        FileStatus::Added => Some(UiRole::GitAdded),
        FileStatus::Deleted => Some(UiRole::GitDeleted),
        FileStatus::Renamed => Some(UiRole::GitRenamed),
        FileStatus::Unversioned => Some(UiRole::GitUntracked),
        FileStatus::Ignored => Some(UiRole::GitIgnored),
        FileStatus::Conflict => Some(UiRole::GitConflict),
    }
}

/// The colour role of a tree row or list entry: folders only when they
/// are ignored.
pub fn entry_role(status: FileStatus, is_dir: bool) -> Option<UiRole> {
    if is_dir && status != FileStatus::Ignored {
        return None;
    }
    status_role(status)
}

/// What is known of an open document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenDocument {
    /// The editor has unsaved changes.
    pub unsaved: bool,
    /// Its text differs from the base text (`None` while no base text is
    /// known).
    pub changed: Option<bool>,
}

/// The status of a file with its open document taken into account.
pub fn document_status(
    status: &vcs::StatusMap,
    path: &Path,
    document: Option<OpenDocument>,
) -> FileStatus {
    let Some(doc) = document.filter(|d| d.unsaved) else {
        return status.status(path);
    };
    match doc.changed {
        // The unsaved text equals the committed one.
        Some(false) => match status.status(path) {
            FileStatus::Modified => FileStatus::NotChanged,
            other => other,
        },
        _ => status.document_status(path, true),
    }
}

#[derive(Default)]
struct Inner {
    snapshot: Option<Arc<Snapshot>>,
    open: HashMap<PathBuf, OpenDocument>,
}

/// The statuses the views colour names with: the repository's last
/// snapshot and the open documents. One handle is shared by the views of
/// a window; the owner updates it and redraws them.
#[derive(Clone, Default)]
pub struct FileColors(Rc<RefCell<Inner>>);

impl FileColors {
    pub fn new() -> Self {
        Self::default()
    }

    /// The repository's statuses (`None`: no version control).
    pub fn set_snapshot(&self, snapshot: Option<Arc<Snapshot>>) {
        self.0.borrow_mut().snapshot = snapshot;
    }

    /// What is known of an open document; `None` when it was closed.
    /// Returns whether anything changed.
    pub fn set_document(&self, path: &Path, document: Option<OpenDocument>) -> bool {
        let mut inner = self.0.borrow_mut();
        match document {
            Some(d) => inner.open.insert(path.to_owned(), d) != Some(d),
            None => inner.open.remove(path).is_some(),
        }
    }

    /// Forgets the snapshot and the documents (another project).
    pub fn clear(&self) {
        let mut inner = self.0.borrow_mut();
        inner.snapshot = None;
        inner.open.clear();
    }

    /// The status of a file or folder.
    pub fn status(&self, path: &Path, is_dir: bool) -> FileStatus {
        let inner = self.0.borrow();
        let Some(snapshot) = &inner.snapshot else {
            return FileStatus::NotChanged;
        };
        if is_dir {
            return snapshot.status.status(path);
        }
        document_status(&snapshot.status, path, inner.open.get(path).copied())
    }

    /// The colour role of a name.
    pub fn role(&self, path: &Path, is_dir: bool) -> Option<UiRole> {
        entry_role(self.status(path, is_dir), is_dir)
    }

    /// The colour of a name, `None` for the normal text colour.
    pub fn color(&self, path: &Path, is_dir: bool, cx: &App) -> Option<Hsla> {
        self.role(path, is_dir).map(|r| cx.theme().color(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs::StatusMap;

    fn p(s: &str) -> PathBuf {
        Path::new("/r").join(s)
    }

    #[test]
    fn folders_are_coloured_only_when_ignored() {
        assert_eq!(
            entry_role(FileStatus::Modified, false),
            Some(UiRole::GitModified)
        );
        assert_eq!(entry_role(FileStatus::Unversioned, true), None);
        assert_eq!(
            entry_role(FileStatus::Ignored, true),
            Some(UiRole::GitIgnored)
        );
        assert_eq!(entry_role(FileStatus::NotChanged, false), None);
    }

    #[test]
    fn an_unsaved_document_counts_by_its_text() {
        let mut map = StatusMap::new("/r");
        map.insert(p("m.py"), FileStatus::Modified);
        map.insert(p("u.py"), FileStatus::Unversioned);
        let edited = |changed| {
            Some(OpenDocument {
                unsaved: true,
                changed,
            })
        };
        // A clean tracked file: modified at once, also before the base
        // text is known.
        assert_eq!(
            document_status(&map, &p("a.py"), edited(None)),
            FileStatus::Modified
        );
        assert_eq!(
            document_status(&map, &p("a.py"), edited(Some(true))),
            FileStatus::Modified
        );
        // Back to the committed text without saving.
        assert_eq!(
            document_status(&map, &p("a.py"), edited(Some(false))),
            FileStatus::NotChanged
        );
        assert_eq!(
            document_status(&map, &p("m.py"), edited(Some(false))),
            FileStatus::NotChanged
        );
        // Saved: the snapshot decides.
        let saved = Some(OpenDocument {
            unsaved: false,
            changed: Some(false),
        });
        assert_eq!(
            document_status(&map, &p("m.py"), saved),
            FileStatus::Modified
        );
        assert_eq!(
            document_status(&map, &p("u.py"), edited(None)),
            FileStatus::Unversioned
        );
    }

    #[test]
    fn colors_follow_the_snapshot_and_the_documents() {
        let colors = FileColors::new();
        assert_eq!(colors.role(&p("a.py"), false), None);
        let mut map = StatusMap::new("/r");
        map.insert(p("b.py"), FileStatus::Added);
        map.insert_ignored_dir(p("logs"));
        colors.set_snapshot(Some(Arc::new(Snapshot {
            info: vcs::RepoInfo {
                root: p(""),
                git_dir: p(".git"),
                head: vcs::Head::Branch {
                    name: "main".into(),
                    oid: None,
                },
                state: vcs::RepoState::Normal,
                rebasing_branch: None,
                tracking: None,
                remotes: Vec::new(),
            },
            status: map,
            generation: 1,
        })));
        assert_eq!(colors.role(&p("b.py"), false), Some(UiRole::GitAdded));
        assert_eq!(colors.role(&p("logs"), true), Some(UiRole::GitIgnored));
        assert_eq!(
            colors.role(&p("logs/x.log"), false),
            Some(UiRole::GitIgnored)
        );
        let doc = OpenDocument {
            unsaved: true,
            changed: None,
        };
        assert!(colors.set_document(&p("a.py"), Some(doc)));
        assert!(!colors.set_document(&p("a.py"), Some(doc)));
        assert_eq!(colors.role(&p("a.py"), false), Some(UiRole::GitModified));
        assert!(colors.set_document(&p("a.py"), None));
        assert_eq!(colors.role(&p("a.py"), false), None);
    }
}
