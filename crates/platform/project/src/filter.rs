//! Which entries of a project are hidden and which are shown as excluded.
//!
//! Hidden entries never appear in the tree: version control folders, byte
//! code caches and editor backup files, the same kind of default list the
//! reference IDE keeps in its "ignored files and folders" setting. Excluded
//! entries are shown but marked and never expanded on their own: folders the
//! project's `.gitignore` names, and virtual environments.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Names that are never shown.
const HIDDEN_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "CVS",
    "_svn",
    "__pycache__",
    ".DS_Store",
    "Thumbs.db",
];

/// File name endings that are never shown.
const HIDDEN_SUFFIXES: &[&str] = &[".pyc", ".pyo", "~"];

/// Hide and exclude rules of one project.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    gitignore: Option<Gitignore>,
}

impl Filter {
    /// Rules for a project root: the defaults and the root `.gitignore`.
    pub fn for_root(root: &Path) -> Self {
        let mut builder = GitignoreBuilder::new(root);
        let mut any = false;
        for file in [root.join(".gitignore"), root.join(".git/info/exclude")] {
            if file.is_file() && builder.add(&file).is_none() {
                any = true;
            }
        }
        let gitignore = any.then(|| builder.build().ok()).flatten();
        Self { gitignore }
    }

    /// Never shown.
    pub fn is_hidden(&self, name: &str) -> bool {
        HIDDEN_NAMES.contains(&name) || HIDDEN_SUFFIXES.iter().any(|s| name.ends_with(s))
    }

    /// Shown but marked as excluded.
    pub fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        if is_dir && path.join("pyvenv.cfg").is_file() {
            return true;
        }
        self.gitignore.as_ref().is_some_and(|g| {
            path.starts_with(g.path()) && g.matched_path_or_any_parents(path, is_dir).is_ignore()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_defaults_and_excludes_ignored_and_virtualenvs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "build/\n*.log\n").unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::create_dir_all(root.join(".venv")).unwrap();
        std::fs::write(root.join(".venv/pyvenv.cfg"), "home = /usr\n").unwrap();
        let f = Filter::for_root(root);
        assert!(f.is_hidden(".git"));
        assert!(f.is_hidden("__pycache__"));
        assert!(f.is_hidden("mod.cpython-313.pyc"));
        assert!(!f.is_hidden(".github"));
        assert!(f.is_excluded(&root.join("build"), true));
        assert!(f.is_excluded(&root.join("out.log"), false));
        assert!(f.is_excluded(&root.join(".venv"), true));
        assert!(!f.is_excluded(&root.join("src"), true));
        assert!(!Filter::default().is_excluded(&root.join("build"), true));
    }
}
