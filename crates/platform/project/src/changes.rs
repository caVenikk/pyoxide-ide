//! What changed on disk at paths a file watcher reported (blocking): the
//! paths are looked at as they are now, as the reference IDE's refresh of
//! dirty paths does, since the reports themselves are only hints.

use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::tree::list_dir;

/// A change of a file or folder on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskChange {
    /// The path is there and is new (or was replaced).
    Created { path: PathBuf, is_dir: bool },
    /// The path is there and its content changed.
    Changed { path: PathBuf, is_dir: bool },
    /// The path is gone.
    Removed(PathBuf),
}

impl DiskChange {
    pub fn path(&self) -> &Path {
        match self {
            DiskChange::Created { path, .. }
            | DiskChange::Changed { path, .. }
            | DiskChange::Removed(path) => path,
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            DiskChange::Created { is_dir, .. } | DiskChange::Changed { is_dir, .. } => *is_dir,
            DiskChange::Removed(_) => false,
        }
    }
}

/// A path that is never shown (a version control folder, byte code) or
/// lies in such a folder, relative to the project root of `filter`.
pub fn is_hidden_path(path: &Path, filter: &Filter) -> bool {
    let rel = match filter.root().and_then(|root| path.strip_prefix(root).ok()) {
        Some(rel) => rel,
        None => {
            return path
                .file_name()
                .is_some_and(|n| filter.is_hidden(&n.to_string_lossy()));
        }
    };
    rel.components()
        .any(|c| filter.is_hidden(&c.as_os_str().to_string_lossy()))
}

/// What the reported paths are now. `maybe_new` says the report was a
/// creation or the end of a rename: such a folder is listed with what is
/// below it (a folder moved into the project comes as one report), hidden
/// entries left out and excluded folders not entered. A path reported as
/// changed that is there is changed; a path that is gone is removed.
/// Hidden paths are left out.
pub fn disk_changes(
    paths: impl IntoIterator<Item = (PathBuf, bool)>,
    filter: &Filter,
) -> Vec<DiskChange> {
    let mut out = Vec::new();
    for (path, maybe_new) in paths {
        if is_hidden_path(&path, filter) {
            continue;
        }
        // The kind of what the path is now; a link counts as what it
        // points to, as in the project tree.
        let is_dir = match std::fs::metadata(&path) {
            Ok(meta) => meta.is_dir(),
            Err(_) if std::fs::symlink_metadata(&path).is_ok() => false,
            Err(_) => {
                out.push(DiskChange::Removed(path));
                continue;
            }
        };
        if !maybe_new {
            out.push(DiskChange::Changed { path, is_dir });
            continue;
        }
        let walk = is_dir
            && !filter.is_excluded(&path, true)
            && !std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
        out.push(DiskChange::Created {
            path: path.clone(),
            is_dir,
        });
        if walk {
            created_below(&path, filter, &mut out);
        }
    }
    out
}

/// Everything below a new folder, folders before their content.
fn created_below(dir: &Path, filter: &Filter, out: &mut Vec<DiskChange>) {
    let Ok(entries) = list_dir(dir, filter) else {
        return;
    };
    for entry in entries {
        let is_dir = entry.is_dir();
        let enter = is_dir && !entry.excluded && !entry.symlink;
        out.push(DiskChange::Created {
            path: entry.path.clone(),
            is_dir,
        });
        if enter {
            created_below(&entry.path, filter, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::AutoExclusions;
    use crate::store::Exclusions;

    fn filter(root: &Path) -> Filter {
        Filter::new(
            root,
            Exclusions::default(),
            AutoExclusions {
                dir_names: &[".venv"],
                root_dir_markers: &[],
            },
        )
    }

    #[test]
    fn reported_paths_are_looked_at_as_they_are_now() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.py"), "a").unwrap();
        std::fs::create_dir_all(root.join("pkg/sub")).unwrap();
        std::fs::write(root.join("pkg/m.py"), "m").unwrap();
        std::fs::write(root.join("pkg/sub/n.py"), "n").unwrap();
        std::fs::create_dir_all(root.join("pkg/.venv/lib")).unwrap();
        std::fs::create_dir_all(root.join("pkg/__pycache__")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let f = filter(root);
        let changes = disk_changes(
            [
                (root.join("a.py"), false),
                (root.join("gone.py"), true),
                (root.join("pkg"), true),
                (root.join(".git/index"), false),
            ],
            &f,
        );
        let created = |p: &str, is_dir| DiskChange::Created {
            path: root.join(p),
            is_dir,
        };
        assert_eq!(
            changes,
            vec![
                DiskChange::Changed {
                    path: root.join("a.py"),
                    is_dir: false
                },
                DiskChange::Removed(root.join("gone.py")),
                created("pkg", true),
                // Excluded folders are reported, not entered; hidden ones
                // are left out.
                created("pkg/.venv", true),
                created("pkg/sub", true),
                created("pkg/sub/n.py", false),
                created("pkg/m.py", false),
            ]
        );
        assert!(is_hidden_path(&root.join(".git/index"), &f));
        assert!(is_hidden_path(&root.join("pkg/__pycache__/m.pyc"), &f));
        assert!(!is_hidden_path(&root.join("pkg/m.py"), &f));
    }
}
