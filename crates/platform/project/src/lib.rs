//! The project model of Pyoxide IDE (ADR 0007, 0008): a folder opened as a
//! project, its file tree loaded one directory at a time, the rules that
//! hide or exclude entries, Pyoxide's own project folder with the shared
//! project settings and the personal project state, and reading and
//! writing documents.
//!
//! Everything here is blocking and toolkit-free: callers run it on a
//! background thread (the workbench uses GPUI's background executor).

mod document;
mod filter;
pub mod idea;
pub mod store;
mod tree;
pub mod workspace;

pub use document::{LoadError, Loaded, MAX_FILE_SIZE, read_document, write_document};
pub use filter::{
    AutoExclusions, DEFAULT_IGNORED, Filter, NameMasks, auto_exclusions, set_auto_exclusions,
};
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

/// A path for display, with the home directory as `~`.
pub fn display_path(path: &Path) -> String {
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
}
