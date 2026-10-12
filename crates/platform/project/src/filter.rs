//! Which entries of a project are hidden and which are excluded (ADR 0008).
//!
//! Hidden entries never appear in the tree and are never searched: the
//! reference IDE's default list of ignored files and folders (version
//! control folders, byte code, tool caches, backup files) and Pyoxide's own
//! project folder ([`STORE_DIR`]).
//!
//! Excluded entries stay in the tree, marked, are not expanded on their own,
//! and are left out of search, Go to File and indexing; everything below an
//! excluded folder is excluded too. A path is excluded when the project
//! settings name it or one of its folders, when one of its names matches an
//! exclude pattern, or when a product excludes it on its own
//! ([`AutoExclusions`], a Python virtual environment).
//!
//! Files that version control ignores are ordinary content.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::store::{self, Exclusions, STORE_DIR};

/// The reference IDE's default "Ignored Files and Folders" list: exact
/// names, `*.ext` extensions and `*`/`?` wildcards, matched against a file
/// or folder name.
pub const DEFAULT_IGNORED: &[&str] = &[
    "*.pyc",
    "*.pyo",
    "*.rbc",
    "*.yarb",
    "*~",
    ".DS_Store",
    ".git",
    ".hg",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".svn",
    "CVS",
    "__pycache__",
    "_svn",
    "vssver.scc",
    "vssver2.scc",
];

/// File name masks as the reference IDE matches them: an exact name
/// (case-sensitive), `*.ext` (the extension after the last dot, any case),
/// or a pattern with `*` and `?` (case-sensitive).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameMasks {
    exact: HashSet<String>,
    extensions: HashSet<String>,
    wildcards: Vec<String>,
}

impl NameMasks {
    pub fn new<S: AsRef<str>>(masks: impl IntoIterator<Item = S>) -> Self {
        let mut out = Self::default();
        for mask in masks {
            let mask = mask.as_ref().trim();
            if mask.is_empty() {
                continue;
            }
            let ext = mask.strip_prefix("*.");
            match ext {
                Some(ext) if !ext.contains(['*', '?', '.']) => {
                    out.extensions.insert(ext.to_lowercase());
                }
                _ if mask.contains(['*', '?']) => out.wildcards.push(mask.to_owned()),
                _ => {
                    out.exact.insert(mask.to_owned());
                }
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.extensions.is_empty() && self.wildcards.is_empty()
    }

    pub fn matches(&self, name: &str) -> bool {
        if self.exact.contains(name) {
            return true;
        }
        if self.wildcards.iter().any(|w| wildcard_match(w, name)) {
            return true;
        }
        match name.rsplit_once('.') {
            Some((_, ext)) if !self.extensions.is_empty() => {
                self.extensions.contains(&ext.to_lowercase())
            }
            _ => false,
        }
    }
}

/// `*` matches any run of characters, `?` one character.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Folders a product excludes without being asked, such as a Python
/// virtual environment. Platform crates know no language, so the product
/// registers its rules at start-up with [`set_auto_exclusions`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoExclusions {
    /// Folder names excluded wherever they are (`.venv`).
    pub dir_names: &'static [&'static str],
    /// A folder directly under the project root that contains one of these
    /// files is excluded (`pyvenv.cfg`).
    pub root_dir_markers: &'static [&'static str],
}

impl AutoExclusions {
    pub const NONE: Self = Self {
        dir_names: &[],
        root_dir_markers: &[],
    };
}

static AUTO_EXCLUSIONS: RwLock<AutoExclusions> = RwLock::new(AutoExclusions::NONE);

/// Sets the product's automatic exclusions used by [`Filter::for_root`].
pub fn set_auto_exclusions(rules: AutoExclusions) {
    *AUTO_EXCLUSIONS.write().unwrap_or_else(|e| e.into_inner()) = rules;
}

/// The automatic exclusions set by the product.
pub fn auto_exclusions() -> AutoExclusions {
    *AUTO_EXCLUSIONS.read().unwrap_or_else(|e| e.into_inner())
}

/// Hide and exclude rules of one project. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    root: Option<PathBuf>,
    exclusions: Exclusions,
    /// `exclusions.paths` for lookups.
    excluded: HashSet<String>,
    patterns: NameMasks,
    auto_names: HashSet<&'static str>,
    /// Folders under the root that hold a marker of `root_dir_markers`.
    auto_root_dirs: HashSet<String>,
}

impl Filter {
    /// Rules for a project root: the default ignored list, the exclusions
    /// of the project settings (or, before the project has its own
    /// settings, those of the reference IDE's `.idea` folder) and the
    /// product's automatic exclusions. Reads a few small files and the
    /// root listing (blocking).
    pub fn for_root(root: &Path) -> Self {
        let exclusions = store::load_or_import(root).unwrap_or_default();
        Self::new(root, exclusions, auto_exclusions())
    }

    /// Rules for a project root with the given exclusions (blocking: looks
    /// for the marker files of `auto` in the folders under the root).
    pub fn new(root: &Path, exclusions: Exclusions, auto: AutoExclusions) -> Self {
        let mut auto_root_dirs = HashSet::new();
        if !auto.root_dir_markers.is_empty()
            && let Ok(items) = std::fs::read_dir(root)
        {
            for item in items.flatten() {
                let path = item.path();
                if path.is_dir() && auto.root_dir_markers.iter().any(|m| path.join(m).is_file()) {
                    auto_root_dirs.insert(item.file_name().to_string_lossy().into_owned());
                }
            }
        }
        Self {
            inner: Arc::new(Inner {
                root: Some(root.to_owned()),
                excluded: exclusions.paths.iter().cloned().collect(),
                patterns: NameMasks::new(&exclusions.patterns),
                exclusions,
                auto_names: auto.dir_names.iter().copied().collect(),
                auto_root_dirs,
            }),
        }
    }

    /// The same rules with other exclusions (after the person marked or
    /// unmarked a folder); the automatic ones are kept.
    pub fn with_exclusions(&self, exclusions: Exclusions) -> Self {
        let old = &self.inner;
        Self {
            inner: Arc::new(Inner {
                root: old.root.clone(),
                excluded: exclusions.paths.iter().cloned().collect(),
                patterns: NameMasks::new(&exclusions.patterns),
                exclusions,
                auto_names: old.auto_names.clone(),
                auto_root_dirs: old.auto_root_dirs.clone(),
            }),
        }
    }

    /// The project root, unless this is the empty filter.
    pub fn root(&self) -> Option<&Path> {
        self.inner.root.as_deref()
    }

    /// The exclusions of the project settings.
    pub fn exclusions(&self) -> &Exclusions {
        &self.inner.exclusions
    }

    /// Never shown and never searched.
    pub fn is_hidden(&self, name: &str) -> bool {
        name == STORE_DIR || default_ignored().matches(name)
    }

    /// Shown but excluded: the path or one of its folders is excluded.
    /// Paths outside the project, and the root itself, are not.
    pub fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        let Some(names) = self.relative_names(path) else {
            return false;
        };
        let inner = &self.inner;
        let last = names.len() - 1;
        if let Some(first) = names.first()
            && (last > 0 || is_dir)
            && inner.auto_root_dirs.contains(first.as_str())
        {
            return true;
        }
        let mut prefix = String::new();
        for (i, name) in names.iter().enumerate() {
            let dir = i < last || is_dir;
            if dir && inner.auto_names.contains(name.as_str()) {
                return true;
            }
            if inner.patterns.matches(name) {
                return true;
            }
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(name);
            if inner.excluded.contains(&prefix) {
                return true;
            }
        }
        false
    }

    /// The path itself is in the exclusions of the project settings (so
    /// its exclusion can be cancelled).
    pub fn is_excluded_root(&self, path: &Path) -> bool {
        self.relative(path)
            .is_some_and(|rel| self.inner.excluded.contains(&rel))
    }

    /// `path` relative to the root with `/` separators, if it is strictly
    /// inside the project.
    pub fn relative(&self, path: &Path) -> Option<String> {
        self.relative_names(path).map(|names| names.join("/"))
    }

    fn relative_names(&self, path: &Path) -> Option<Vec<String>> {
        let rel = path.strip_prefix(self.inner.root.as_ref()?).ok()?;
        let mut names = Vec::new();
        for c in rel.components() {
            match c {
                Component::Normal(n) => names.push(n.to_string_lossy().into_owned()),
                Component::CurDir => {}
                _ => return None,
            }
        }
        (!names.is_empty()).then_some(names)
    }
}

fn default_ignored() -> &'static NameMasks {
    static MASKS: std::sync::OnceLock<NameMasks> = std::sync::OnceLock::new();
    MASKS.get_or_init(|| NameMasks::new(DEFAULT_IGNORED))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: AutoExclusions = AutoExclusions {
        dir_names: &[".venv"],
        root_dir_markers: &["pyvenv.cfg"],
    };

    fn exclusions(paths: &[&str], patterns: &[&str]) -> Exclusions {
        Exclusions {
            paths: paths.iter().map(|p| p.to_string()).collect(),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn hides_the_reference_default_list() {
        let f = Filter::default();
        for name in [
            ".git",
            ".hg",
            ".svn",
            "CVS",
            "_svn",
            "__pycache__",
            ".mypy_cache",
            ".pytest_cache",
            ".ruff_cache",
            ".DS_Store",
            "vssver.scc",
            "vssver2.scc",
            "mod.cpython-313.pyc",
            "mod.PYC",
            "mod.pyo",
            "x.rbc",
            "x.yarb",
            "notes.txt~",
            STORE_DIR,
        ] {
            assert!(f.is_hidden(name), "{name}");
        }
        for name in [
            ".github",
            ".gitignore",
            ".idea",
            ".venv",
            "Thumbs.db",
            "cvs",
            "pyc",
            "app.py",
            "~notes",
            ".gitattributes",
        ] {
            assert!(!f.is_hidden(name), "{name}");
        }
        let root = tempfile::tempdir().unwrap();
        let f = Filter::new(root.path(), Exclusions::default(), AutoExclusions::NONE);
        assert!(f.is_hidden(".git") && f.is_hidden(STORE_DIR) && !f.is_hidden(".idea"));
    }

    #[test]
    fn name_masks_follow_the_reference_matchers() {
        let m = NameMasks::new(["*.egg-info", "build", "*.LOG", "tmp?", "a*b*c", ""]);
        assert!(m.matches("pkg.egg-info"));
        assert!(m.matches("build"));
        assert!(!m.matches("Build"));
        assert!(m.matches("x.log") && m.matches("x.Log"));
        assert!(m.matches("tmp1") && !m.matches("tmp") && !m.matches("tmp12"));
        assert!(m.matches("abc") && m.matches("a-b-c") && m.matches("aXbYc"));
        assert!(!m.matches("acb"));
        assert!(!NameMasks::new(["*.egg-info"]).matches("egg-info"));
        assert!(NameMasks::new(Vec::<String>::new()).is_empty());
    }

    #[test]
    fn gitignored_entries_are_ordinary_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "build/\n*.log\n").unwrap();
        std::fs::create_dir_all(root.join(".git/info")).unwrap();
        std::fs::write(root.join(".git/info/exclude"), "dist/\n").unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        let f = Filter::for_root(root);
        assert!(!f.is_excluded(&root.join("build"), true));
        assert!(!f.is_excluded(&root.join("out.log"), false));
        assert!(!f.is_excluded(&root.join("dist"), true));
        assert!(!f.is_hidden("build") && !f.is_hidden("out.log"));
    }

    #[test]
    fn explicit_exclusions_cover_everything_below() {
        let root = Path::new("/work/demo");
        let f = Filter::new(
            root,
            exclusions(&["venv", "docs/build", "notes.txt"], &[]),
            AutoExclusions::NONE,
        );
        assert!(f.is_excluded(&root.join("venv"), true));
        assert!(f.is_excluded(&root.join("venv/lib/site.py"), false));
        assert!(f.is_excluded(&root.join("docs/build"), true));
        assert!(f.is_excluded(&root.join("docs/build/html/index.html"), false));
        assert!(f.is_excluded(&root.join("notes.txt"), false));
        assert!(!f.is_excluded(&root.join("docs"), true));
        assert!(!f.is_excluded(&root.join("docs/index.md"), false));
        assert!(!f.is_excluded(&root.join("venv2"), true));
        assert!(!f.is_excluded(&root.join("src/venv"), true));
        assert!(!f.is_excluded(root, true));
        assert!(!f.is_excluded(Path::new("/elsewhere/venv"), true));
        assert!(f.is_excluded_root(&root.join("docs/build")));
        assert!(!f.is_excluded_root(&root.join("docs/build/html")));
        assert_eq!(
            f.relative(&root.join("docs/build")).as_deref(),
            Some("docs/build")
        );
        assert_eq!(f.relative(root), None);
        // Nothing is excluded before a project is open.
        assert!(!Filter::default().is_excluded(&root.join("venv"), true));
    }

    #[test]
    fn patterns_match_names_below_the_root() {
        let root = Path::new("/work/demo");
        let f = Filter::new(
            root,
            exclusions(&[], &["*.egg-info", "*.tmp"]),
            AutoExclusions::NONE,
        );
        assert!(f.is_excluded(&root.join("src/pkg.egg-info"), true));
        assert!(f.is_excluded(&root.join("src/pkg.egg-info/PKG-INFO"), false));
        assert!(f.is_excluded(&root.join("a.tmp"), false));
        assert!(!f.is_excluded(&root.join("src/pkg"), true));
        assert!(!f.is_excluded_root(&root.join("a.tmp")));
    }

    #[test]
    fn product_rules_exclude_virtual_environments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in [".venv", "env", "sub/.venv", "sub/env", "src"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("env/pyvenv.cfg"), "home = /usr\n").unwrap();
        std::fs::write(root.join("sub/env/pyvenv.cfg"), "home = /usr\n").unwrap();
        let f = Filter::new(root, Exclusions::default(), PY);
        assert!(f.is_excluded(&root.join(".venv"), true));
        assert!(f.is_excluded(&root.join(".venv/lib/x.py"), false));
        assert!(f.is_excluded(&root.join("sub/.venv"), true));
        assert!(f.is_excluded(&root.join("env"), true));
        assert!(f.is_excluded(&root.join("env/bin/python"), false));
        // A marker only counts in a folder directly under the root.
        assert!(!f.is_excluded(&root.join("sub/env"), true));
        // A file named like an excluded folder is not excluded.
        assert!(!f.is_excluded(&root.join("src/.venv"), false));
        assert!(!f.is_excluded(&root.join("src"), true));
        assert!(!f.is_excluded_root(&root.join(".venv")));
        // Without product rules nothing of this is excluded.
        let plain = Filter::new(root, Exclusions::default(), AutoExclusions::NONE);
        assert!(!plain.is_excluded(&root.join(".venv"), true));
        assert!(!plain.is_excluded(&root.join("env"), true));
        // Other exclusions keep the product rules.
        let marked = f.with_exclusions(exclusions(&["src"], &[]));
        assert!(marked.is_excluded(&root.join("src"), true));
        assert!(marked.is_excluded(&root.join("env"), true));
        assert_eq!(marked.exclusions().paths.len(), 1);
    }
}
