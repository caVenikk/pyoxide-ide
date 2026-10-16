//! Source roots and what makes a folder special (ADR 0013).
//!
//! A *root* is a folder of the project marked with a kind the product
//! offers in *Mark Directory As*: a sources root or a test sources root,
//! as the reference IDE's module source folders. A folder has at most one
//! kind; the project root can be one too. Roots are stored in the project
//! settings ([`crate::store`]); before a project stores its own, the
//! product's detected roots and the reference IDE's `sourceFolder`
//! entries are in effect.
//!
//! The product also says which files make a folder a package (a Python
//! folder with `__init__.py`) or a module of its own (a folder with
//! `pyproject.toml`), with [`FolderRules`]; the tree shows them with
//! their own icons. Both come with the project's rules
//! ([`crate::Filter::with_roots`], [`crate::Filter::with_folder_rules`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use crate::store::normalize;

/// The kind of the reference IDE's source folders for code
/// (`<sourceFolder isTestSource="false">`).
pub const SOURCES: &str = "sources";
/// The kind of the reference IDE's source folders for tests
/// (`<sourceFolder isTestSource="true">`).
pub const TESTS: &str = "tests";
/// The project root as a stored root path.
pub const PROJECT_ROOT: &str = ".";

/// A stored root path: `.` for the project root, otherwise the normalized
/// relative path; `None` for a path that leaves the project.
pub fn root_key(relative: &str) -> Option<String> {
    let trimmed = relative.trim_matches(['/', std::path::MAIN_SEPARATOR]);
    if trimmed.is_empty() || trimmed == "." {
        return Some(PROJECT_ROOT.to_owned());
    }
    if trimmed
        .split(['/', std::path::MAIN_SEPARATOR])
        .any(|p| p == "..")
    {
        return None;
    }
    Some(normalize(trimmed).unwrap_or_else(|| PROJECT_ROOT.to_owned()))
}

/// The folders of a project marked with a kind of root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    /// Kind -> stored root paths (`/`-separated, `.` for the project root).
    kinds: BTreeMap<String, BTreeSet<String>>,
}

impl Roots {
    /// No folder is marked.
    pub fn is_empty(&self) -> bool {
        self.kinds.values().all(BTreeSet::is_empty)
    }

    /// The kind of the folder at `relative` (`.` or empty for the project
    /// root), if it is a root.
    pub fn kind_of(&self, relative: &str) -> Option<&str> {
        let key = root_key(relative)?;
        self.kinds
            .iter()
            .find(|(_, paths)| paths.contains(&key))
            .map(|(kind, _)| kind.as_str())
    }

    /// Marks the folder at `relative` with `kind`, replacing another kind
    /// it had; false when it had this kind or leaves the project.
    pub fn mark(&mut self, relative: &str, kind: &str) -> bool {
        let Some(key) = root_key(relative) else {
            return false;
        };
        if self.kinds.get(kind).is_some_and(|p| p.contains(&key)) {
            return false;
        }
        for paths in self.kinds.values_mut() {
            paths.remove(&key);
        }
        self.kinds.entry(kind.to_owned()).or_default().insert(key);
        true
    }

    /// Unmarks the folder at `relative`; false when it was no root.
    pub fn unmark(&mut self, relative: &str) -> bool {
        let Some(key) = root_key(relative) else {
            return false;
        };
        let mut removed = false;
        for paths in self.kinds.values_mut() {
            removed |= paths.remove(&key);
        }
        removed
    }

    /// The stored paths of `kind`, sorted.
    pub fn paths<'a>(&'a self, kind: &str) -> impl Iterator<Item = &'a str> + 'a {
        self.kinds
            .get(kind)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// The kinds that have roots, sorted.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.kinds
            .iter()
            .filter(|(_, paths)| !paths.is_empty())
            .map(|(kind, _)| kind.as_str())
    }

    /// Every root as (kind, stored path).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.kinds
            .iter()
            .flat_map(|(kind, paths)| paths.iter().map(move |p| (kind.as_str(), p.as_str())))
    }

    /// Adds the roots of `other` whose folders are no root here yet.
    pub fn add_missing(&mut self, other: &Roots) {
        for (kind, path) in other.iter() {
            if self.kind_of(path).is_none() {
                self.mark(path, kind);
            }
        }
    }

    /// The folders of the given kinds below `project_root`, sorted by
    /// path (the project root itself for `.`).
    pub fn folders(&self, project_root: &Path, kinds: &[&str]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self
            .iter()
            .filter(|(kind, _)| kinds.contains(kind))
            .map(|(_, path)| absolute(project_root, path))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The roots after a folder moved from `old` to `new` (relative
    /// paths): roots at or below it follow. Returns whether one did.
    pub fn moved(&mut self, old: &str, new: &str) -> bool {
        let (Some(old), Some(new)) = (root_key(old), root_key(new)) else {
            return false;
        };
        if old == PROJECT_ROOT {
            return false;
        }
        let below = format!("{old}/");
        let mut changed = false;
        for paths in self.kinds.values_mut() {
            let moved: Vec<String> = paths
                .iter()
                .filter(|p| **p == old || p.starts_with(&below))
                .cloned()
                .collect();
            for p in moved {
                paths.remove(&p);
                paths.insert(format!("{new}{}", &p[old.len()..]));
                changed = true;
            }
        }
        changed
    }

    /// The roots after the folder at `relative` was deleted: roots at or
    /// below it go. Returns whether one did.
    pub fn removed(&mut self, relative: &str) -> bool {
        let Some(gone) = root_key(relative) else {
            return false;
        };
        if gone == PROJECT_ROOT {
            return false;
        }
        let below = format!("{gone}/");
        let mut changed = false;
        for paths in self.kinds.values_mut() {
            let before = paths.len();
            paths.retain(|p| *p != gone && !p.starts_with(&below));
            changed |= paths.len() != before;
        }
        changed
    }
}

/// A stored root path as a folder below `project_root`.
pub fn absolute(project_root: &Path, stored: &str) -> PathBuf {
    let mut out = project_root.to_owned();
    if stored != PROJECT_ROOT {
        for part in stored.split('/') {
            out.push(part);
        }
    }
    out
}

/// What the product sees in a folder, from the files in it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FolderKind {
    /// An ordinary folder.
    #[default]
    Plain,
    /// A package of the product's language (a Python folder with
    /// `__init__.py`).
    Package,
    /// A module of its own (a Python project with `pyproject.toml`): the
    /// reference IDE makes it a module with the folder as its content root.
    Module,
}

/// The files that make a folder a package or a module; the product sets
/// them, as platform crates know no language.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderRules {
    /// A folder holding one of these files is a package (`__init__.py`).
    pub package_markers: &'static [&'static str],
    /// A folder holding one of these files is a module (`pyproject.toml`).
    pub module_markers: &'static [&'static str],
}

impl FolderRules {
    pub const NONE: Self = Self {
        package_markers: &[],
        module_markers: &[],
    };

    /// The kind of the folder `dir` (blocking: looks for the marker
    /// files). A module wins over a package, as the reference's content
    /// root icon wins over its package icon.
    pub fn kind_of(&self, dir: &Path) -> FolderKind {
        let has = |names: &[&str]| names.iter().any(|n| dir.join(n).is_file());
        if has(self.module_markers) {
            FolderKind::Module
        } else if has(self.package_markers) {
            FolderKind::Package
        } else {
            FolderKind::Plain
        }
    }
}

/// Finds the source roots a product marks on its own when a folder opens
/// as a project (the reference marks `src` and the root package folder of
/// `setup.py`); blocking.
pub type DetectRoots = fn(&Path) -> Vec<PathBuf>;

/// The source roots `detect` finds in `project_root` (blocking), as
/// [`SOURCES`] roots.
pub fn detected_roots(project_root: &Path, detect: Option<DetectRoots>) -> Roots {
    let mut out = Roots::default();
    for dir in detect.map(|d| d(project_root)).unwrap_or_default() {
        if let Some(rel) = crate::store::relative_or_root(project_root, &dir) {
            out.mark(&rel, SOURCES);
        }
    }
    out
}

/// The roots in effect of the open projects, by project root.
static PUBLISHED: LazyLock<RwLock<HashMap<PathBuf, Roots>>> = LazyLock::new(Default::default);

/// Makes the roots in effect of an open project known to code that has
/// only a path (the language services naming a file's module); the
/// project tool window publishes them whenever they are read or change.
pub fn publish(project_root: &Path, roots: &Roots) {
    PUBLISHED
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(project_root.to_owned(), roots.clone());
}

/// The roots last published for `project_root`.
pub fn published(project_root: &Path) -> Option<Roots> {
    PUBLISHED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(project_root)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(items: &[(&str, &str)]) -> Roots {
        let mut r = Roots::default();
        for (kind, path) in items {
            assert!(r.mark(path, kind), "{kind} {path}");
        }
        r
    }

    #[test]
    fn root_keys() {
        assert_eq!(root_key("").as_deref(), Some("."));
        assert_eq!(root_key(".").as_deref(), Some("."));
        assert_eq!(root_key("./").as_deref(), Some("."));
        assert_eq!(root_key("src").as_deref(), Some("src"));
        assert_eq!(root_key("./a//b/").as_deref(), Some("a/b"));
        assert_eq!(root_key("../x"), None);
        assert_eq!(root_key("a/../b"), None);
    }

    #[test]
    fn a_folder_has_one_kind() {
        let mut r = roots(&[(SOURCES, "src"), (TESTS, "tests"), (SOURCES, ".")]);
        assert_eq!(r.kind_of("src"), Some(SOURCES));
        assert_eq!(r.kind_of(""), Some(SOURCES));
        assert_eq!(r.kind_of("tests"), Some(TESTS));
        assert_eq!(r.kind_of("docs"), None);
        assert!(!r.mark("src", SOURCES));
        // Marking with another kind replaces the kind.
        assert!(r.mark("src", TESTS));
        assert_eq!(r.kind_of("src"), Some(TESTS));
        assert_eq!(r.paths(SOURCES).collect::<Vec<_>>(), ["."]);
        assert_eq!(r.paths(TESTS).collect::<Vec<_>>(), ["src", "tests"]);
        assert!(r.unmark("src"));
        assert!(!r.unmark("src"));
        assert_eq!(r.kinds().collect::<Vec<_>>(), [SOURCES, TESTS]);
        assert!(r.unmark(".") && r.unmark("tests"));
        assert!(r.is_empty());
        assert_eq!(r.kinds().count(), 0);
    }

    #[test]
    fn folders_and_moves() {
        let root = Path::new("/w/p");
        let mut r = roots(&[
            (SOURCES, "."),
            (SOURCES, "svc/api"),
            (TESTS, "svc/api/tests"),
            (TESTS, "tests"),
        ]);
        assert_eq!(
            r.folders(root, &[SOURCES]),
            [PathBuf::from("/w/p"), PathBuf::from("/w/p/svc/api")]
        );
        assert_eq!(r.folders(root, &[SOURCES, TESTS]).len(), 4);
        assert!(r.moved("svc", "services"));
        assert_eq!(r.kind_of("services/api"), Some(SOURCES));
        assert_eq!(r.kind_of("services/api/tests"), Some(TESTS));
        assert_eq!(r.kind_of("svc/api"), None);
        assert!(!r.moved("docs", "doc"));
        assert!(!r.moved(".", "x"));
        assert!(r.removed("services"));
        assert_eq!(r.kind_of("services/api"), None);
        assert!(!r.removed("."));
        assert_eq!(r.kind_of("."), Some(SOURCES));

        let mut a = roots(&[(SOURCES, "src")]);
        a.add_missing(&roots(&[(TESTS, "src"), (TESTS, "tests")]));
        assert_eq!(a, roots(&[(SOURCES, "src"), (TESTS, "tests")]));
    }

    #[test]
    fn folder_kinds_from_marker_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["pkg", "svc", "plain", "both"] {
            std::fs::create_dir(root.join(d)).unwrap();
        }
        std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
        std::fs::write(root.join("svc/pyproject.toml"), "").unwrap();
        std::fs::write(root.join("both/__init__.py"), "").unwrap();
        std::fs::write(root.join("both/pyproject.toml"), "").unwrap();
        // A folder named like a marker does not count.
        std::fs::create_dir(root.join("plain/__init__.py")).unwrap();
        let rules = FolderRules {
            package_markers: &["__init__.py"],
            module_markers: &["pyproject.toml"],
        };
        assert_eq!(rules.kind_of(&root.join("pkg")), FolderKind::Package);
        assert_eq!(rules.kind_of(&root.join("svc")), FolderKind::Module);
        assert_eq!(rules.kind_of(&root.join("both")), FolderKind::Module);
        assert_eq!(rules.kind_of(&root.join("plain")), FolderKind::Plain);
        assert_eq!(
            FolderRules::NONE.kind_of(&root.join("pkg")),
            FolderKind::Plain
        );
    }
}
