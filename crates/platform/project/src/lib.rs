//! The project model of Pyoxide IDE (ADR 0007, 0008, 0013): a folder opened
//! as a project, its file tree loaded one directory at a time, the rules
//! that hide or exclude entries, its source roots and the folders a product
//! sees as packages or modules, Pyoxide's own project folder with the shared
//! project settings and the personal project state, reading and writing
//! documents, what reported paths are now on disk (for a file watcher),
//! and the file operations of the project tool window.
//!
//! Everything here is blocking and toolkit-free: callers run it on a
//! background thread (the workbench uses GPUI's background executor).

mod changes;
mod document;
mod filter;
pub mod fs_ops;
pub mod idea;
pub mod roots;
pub mod store;
mod tree;
pub mod workspace;

pub use changes::{DiskChange, disk_changes, is_hidden_path};
pub use document::{
    LoadError, Loaded, MAX_FILE_SIZE, Stamp, is_save_temp, read_document, write_document,
};
pub use filter::{
    AutoExclusions, DEFAULT_IGNORED, Filter, NameMasks, auto_exclusions, set_auto_exclusions,
    wildcard_match,
};
pub use roots::{DetectRoots, FolderKind, FolderRules, Roots};
pub use store::{Exclusions, STORE_DIR, StoreError};
pub use tree::{Entry, EntryKind, FileTree, Node, NodeId, list_dir};

use std::path::{Path, PathBuf};

/// A folder opened as a project.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    name: String,
}

impl Project {
    /// A project for `root`; the name is the folder name.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = root.into();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Self { root, name }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `path` relative to the root, if it is inside the project.
    pub fn relative<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.root).ok()
    }

    /// The root shortened for display: `~/...` under the home directory.
    pub fn display_root(&self) -> String {
        display_path(&self.root)
    }
}

/// A path for display, with the home directory as `~`. A Windows path made
/// by `canonicalize` (`\\?\C:\x`) is shown as `C:\x`.
pub fn display_path(path: &Path) -> String {
    let path = dunce::simplified(path);
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    if let Some(home) = home.map(PathBuf::from)
        && let Ok(rest) = path.strip_prefix(&home)
    {
        let sep = std::path::MAIN_SEPARATOR;
        return if rest.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~{sep}{}", rest.display())
        };
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_names_and_relative_paths() {
        let p = Project::new("/work/demo");
        assert_eq!(p.name(), "demo");
        assert_eq!(
            p.relative(Path::new("/work/demo/src/a.py")),
            Some(Path::new("src/a.py"))
        );
        assert_eq!(p.relative(Path::new("/elsewhere")), None);
    }

    /// `canonicalize` gives `\\?\C:\x` on Windows; users see `C:\x`.
    #[cfg(windows)]
    #[test]
    fn a_verbatim_path_is_shown_without_its_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\Z:\a\b")), r"Z:\a\b");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\srv\share\a")),
            r"\\srv\share\a"
        );
    }
}
