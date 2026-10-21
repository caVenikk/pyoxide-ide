//! File status colours (the reference's `FILESTATUS_*` scheme keys): the
//! names of files in the project tree, the editor tabs, Recent Files and
//! Go to File are drawn in the colour of their status. Folders are not
//! coloured, except an ignored folder (and everything below it, which is
//! ignored too). Ignored names are one grey in every theme, except in an
//! excluded folder of the tree, where they take the scheme's colour of
//! ignored files (orange in the Islands themes). An open document counts with its unsaved text: a clean
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

/// The colour of a name ignored by version control, outside excluded
/// folders: one grey in every theme (the reference draws it with a fixed
/// grey, not with the scheme's `FILESTATUS_IDEA_FILESTATUS_IGNORED`).
pub const IGNORED_NAME: u32 = 0x80_80_80;

/// How the name of a tree row or list entry is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameColor {
    /// The colour of a theme role.
    Role(UiRole),
    /// The grey of ignored names, [`IGNORED_NAME`].
    Ignored,
}

impl NameColor {
    /// The name the state dump shows.
    pub fn name(self) -> &'static str {
        match self {
            Self::Role(role) => role.name(),
            Self::Ignored => "ignored",
        }
    }

    pub fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Role(role) => cx.theme().color(role),
            Self::Ignored => gpui::rgb(IGNORED_NAME).into(),
        }
    }
}

/// The colour of a tree row or list entry: folders only when they are
/// ignored. An ignored name is grey; one in an excluded folder takes the
/// scheme's colour of ignored files instead, and an excluded name that is
/// not ignored keeps the normal text colour.
pub fn entry_color(status: FileStatus, is_dir: bool, excluded: bool) -> Option<NameColor> {
    if excluded {
        return (status == FileStatus::Ignored).then_some(NameColor::Role(UiRole::GitIgnored));
    }
    match status {
        FileStatus::Ignored => Some(NameColor::Ignored),
        _ if is_dir => None,
        _ => status_role(status).map(NameColor::Role),
    }
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

    /// The colour of a name (`excluded`: it is in an excluded folder).
    pub fn name_color(&self, path: &Path, is_dir: bool, excluded: bool) -> Option<NameColor> {
        entry_color(self.status(path, is_dir), is_dir, excluded)
    }

    /// The colour of a name, `None` for the normal text colour.
    pub fn color(&self, path: &Path, is_dir: bool, cx: &App) -> Option<Hsla> {
        self.name_color(path, is_dir, false).map(|c| c.color(cx))
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
        let role = |r| Some(NameColor::Role(r));
        assert_eq!(
            entry_color(FileStatus::Modified, false, false),
            role(UiRole::GitModified)
        );
        assert_eq!(entry_color(FileStatus::Unversioned, true, false), None);
        assert_eq!(
            entry_color(FileStatus::Ignored, true, false),
            Some(NameColor::Ignored)
        );
        assert_eq!(entry_color(FileStatus::NotChanged, false, false), None);
    }

    #[test]
    fn ignored_names_are_grey_and_excluded_ones_take_the_scheme_colour() {
        // Outside excluded folders: one grey, whatever the theme says.
        assert_eq!(
            entry_color(FileStatus::Ignored, false, false),
            Some(NameColor::Ignored)
        );
        assert_eq!(IGNORED_NAME, 0x808080);
        // Inside: the colour of the scheme's ignored files.
        assert_eq!(
            entry_color(FileStatus::Ignored, false, true),
            Some(NameColor::Role(UiRole::GitIgnored))
        );
        assert_eq!(
            entry_color(FileStatus::Ignored, true, true),
            Some(NameColor::Role(UiRole::GitIgnored))
        );
        // An excluded name that is not ignored keeps the text colour.
        assert_eq!(entry_color(FileStatus::NotChanged, true, true), None);
        assert_eq!(entry_color(FileStatus::Modified, false, true), None);
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
        let role = |r| Some(NameColor::Role(r));
        assert_eq!(colors.name_color(&p("a.py"), false, false), None);
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
        assert_eq!(
            colors.name_color(&p("b.py"), false, false),
            role(UiRole::GitAdded)
        );
        assert_eq!(
            colors.name_color(&p("logs"), true, false),
            Some(NameColor::Ignored)
        );
        assert_eq!(
            colors.name_color(&p("logs/x.log"), false, false),
            Some(NameColor::Ignored)
        );
        assert_eq!(
            colors.name_color(&p("logs/x.log"), false, true),
            role(UiRole::GitIgnored)
        );
        let doc = OpenDocument {
            unsaved: true,
            changed: None,
        };
        assert!(colors.set_document(&p("a.py"), Some(doc)));
        assert!(!colors.set_document(&p("a.py"), Some(doc)));
        assert_eq!(
            colors.name_color(&p("a.py"), false, false),
            role(UiRole::GitModified)
        );
        assert!(colors.set_document(&p("a.py"), None));
        assert_eq!(colors.name_color(&p("a.py"), false, false), None);
    }
}
