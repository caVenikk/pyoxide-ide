//! Pyoxide's own project folder ([`STORE_DIR`] in the project root) and the
//! shared project settings in it (ADR 0008).
//!
//! `project.toml` is meant to be committed with the project. It is created
//! only when there is something to store, keys it does not know survive a
//! rewrite, and writes replace the whole file at once (a temporary file and
//! a rename).
//!
//! ```toml
//! version = 1
//!
//! [exclusions]
//! paths = [
//!     ".venv",
//!     "docs/build",
//! ]
//! patterns = ["*.egg-info"]
//! ```
//!
//! Paths are relative to the project root with `/` separators on every OS;
//! patterns are file name masks matched below the root (see
//! [`NameMasks`](crate::NameMasks)).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use toml_edit::{Array, DocumentMut, Item, Table, Value};

use crate::idea;

/// Name of Pyoxide's project folder in the project root.
pub const STORE_DIR: &str = ".oxide";
/// The shared project settings in [`STORE_DIR`].
pub const SETTINGS_FILE: &str = "project.toml";
/// Version of the settings format this build reads and writes.
pub const SETTINGS_VERSION: i64 = 1;

const HEADER: &str = "# Pyoxide project settings. Paths are relative to the project folder.\n";

/// Folders and files left out of search, Go to File and indexing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exclusions {
    /// Paths relative to the project root, `/`-separated, sorted.
    pub paths: BTreeSet<String>,
    /// File name masks matched against every name below the root.
    pub patterns: Vec<String>,
}

impl Exclusions {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.patterns.is_empty()
    }

    /// Adds a relative path (any separator style is normalized); returns
    /// false if it was there or is not a path inside the project.
    pub fn add(&mut self, relative: &str) -> bool {
        normalize(relative).is_some_and(|p| self.paths.insert(p))
    }

    /// Removes a relative path; returns false if it was not there.
    pub fn remove(&mut self, relative: &str) -> bool {
        normalize(relative).is_some_and(|p| self.paths.remove(&p))
    }

    /// Adds a pattern unless it is there already.
    pub fn add_pattern(&mut self, pattern: &str) {
        let pattern = pattern.trim();
        if !pattern.is_empty() && !self.patterns.iter().any(|p| p == pattern) {
            self.patterns.push(pattern.to_owned());
        }
    }
}

/// A relative path as stored: `/`-separated, no `.` or empty parts, never
/// leaving the root. `None` for the root itself and for paths outside it.
pub fn normalize(relative: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in relative.split(['/', std::path::MAIN_SEPARATOR]) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `path` relative to `root` in the stored form, resolving `.` and `..`
/// without touching the file system.
pub fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let root = lexical(root);
    let path = lexical(path);
    let rel = path.strip_prefix(&root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `path` with `.` and `..` resolved lexically.
pub(crate) fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
    #[error(
        "{} was written by a newer version of Pyoxide (settings version {version})",
        path.display()
    )]
    Newer { path: PathBuf, version: i64 },
}

/// The settings file of a project.
pub fn settings_file(root: &Path) -> PathBuf {
    root.join(STORE_DIR).join(SETTINGS_FILE)
}

/// Whether the project has its own folder yet.
pub fn has_store(root: &Path) -> bool {
    root.join(STORE_DIR).is_dir()
}

fn read_document(path: &Path) -> Result<Option<DocumentMut>, StoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    text.parse::<DocumentMut>()
        .map(Some)
        .map_err(|e| StoreError::Invalid {
            path: path.to_owned(),
            message: e.to_string(),
        })
}

fn version(doc: &DocumentMut) -> i64 {
    doc.get("version")
        .and_then(Item::as_integer)
        .unwrap_or(SETTINGS_VERSION)
}

/// The exclusions of the project settings; `None` when the settings file
/// does not exist.
pub fn load(root: &Path) -> Result<Option<Exclusions>, StoreError> {
    let path = settings_file(root);
    let Some(doc) = read_document(&path)? else {
        return Ok(None);
    };
    let mut out = Exclusions::default();
    let Some(table) = doc.get("exclusions") else {
        return Ok(Some(out));
    };
    let table = table.as_table_like().ok_or_else(|| StoreError::Invalid {
        path: path.clone(),
        message: "`exclusions` is not a table".into(),
    })?;
    let strings = |key: &str| -> Vec<String> {
        table
            .get(key)
            .and_then(Item::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    for p in strings("paths") {
        out.add(&p);
    }
    for p in strings("patterns") {
        out.add_pattern(&p);
    }
    Ok(Some(out))
}

/// The exclusions in effect: the project settings once the project has
/// its own folder, before that the reference IDE's (`.idea`), if any.
pub fn load_or_import(root: &Path) -> Result<Exclusions, StoreError> {
    if has_store(root) {
        return Ok(load(root)?.unwrap_or_default());
    }
    Ok(idea::read_exclusions(root).unwrap_or_default())
}

/// Copies the reference IDE's exclusions into the project settings, once:
/// only when the project has no folder of its own yet and `.idea` names
/// exclusions. Returns them when they were written.
pub fn import_idea(root: &Path) -> Result<Option<Exclusions>, StoreError> {
    if has_store(root) {
        return Ok(None);
    }
    match idea::read_exclusions(root) {
        Some(exclusions) if !exclusions.is_empty() => {
            save(root, &exclusions)?;
            Ok(Some(exclusions))
        }
        _ => Ok(None),
    }
}

/// Writes the exclusions into the project settings, keeping everything
/// else in the file. Nothing is created while there is nothing to store.
/// Returns whether the file was written.
pub fn save(root: &Path, exclusions: &Exclusions) -> Result<bool, StoreError> {
    let path = settings_file(root);
    let existing = read_document(&path)?;
    if existing.is_none() && exclusions.is_empty() {
        return Ok(false);
    }
    let mut doc = match existing {
        Some(doc) => doc,
        None => format!("{HEADER}version = {SETTINGS_VERSION}\n")
            .parse()
            .expect("the header is valid TOML"),
    };
    let stored = version(&doc);
    if stored > SETTINGS_VERSION {
        return Err(StoreError::Newer {
            path,
            version: stored,
        });
    }
    if doc.get("version").and_then(Item::as_integer) != Some(SETTINGS_VERSION) {
        doc["version"] = toml_edit::value(SETTINGS_VERSION);
    }
    if doc.get("exclusions").is_none() {
        doc["exclusions"] = Item::Table(Table::new());
    }
    let table = doc["exclusions"]
        .as_table_like_mut()
        .ok_or_else(|| StoreError::Invalid {
            path: path.clone(),
            message: "`exclusions` is not a table".into(),
        })?;
    table.insert("paths", Item::Value(Value::Array(lines(&exclusions.paths))));
    if exclusions.patterns.is_empty() {
        table.remove("patterns");
    } else {
        let mut patterns = Array::new();
        for p in &exclusions.patterns {
            patterns.push(p.as_str());
        }
        table.insert("patterns", Item::Value(Value::Array(patterns)));
    }
    write_atomic(&path, &doc.to_string()).map_err(|source| StoreError::Io { path, source })?;
    Ok(true)
}

/// An array of strings, one per line.
fn lines<'a>(items: impl IntoIterator<Item = &'a String>) -> Array {
    let mut array = Array::new();
    for item in items {
        array.push(item.as_str());
    }
    if !array.is_empty() {
        for v in array.iter_mut() {
            v.decor_mut().set_prefix("\n    ");
            v.decor_mut().set_suffix("");
        }
        array.set_trailing("\n");
        array.set_trailing_comma(true);
    }
    array
}

/// Replaces `path` with `text` at once: a temporary file next to it,
/// flushed to disk, then renamed over it.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().expect("a settings file has a folder");
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(paths: &[&str]) -> Exclusions {
        let mut e = Exclusions::default();
        for p in paths {
            assert!(e.add(p), "{p}");
        }
        e
    }

    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(normalize("venv").as_deref(), Some("venv"));
        assert_eq!(normalize("./docs//build/").as_deref(), Some("docs/build"));
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("."), None);
        assert_eq!(normalize("../x"), None);
        assert_eq!(normalize("a/../b"), None);
        let root = Path::new("/w/p");
        assert_eq!(
            relative_to(root, Path::new("/w/p/./a/../b/c")).as_deref(),
            Some("b/c")
        );
        assert_eq!(relative_to(root, Path::new("/w/p")), None);
        assert_eq!(relative_to(root, Path::new("/w/other")), None);
        assert_eq!(relative_to(root, Path::new("/w/p/..")), None);
    }

    #[test]
    fn round_trip_creates_the_folder_lazily() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Nothing to store: no folder.
        assert!(!save(root, &Exclusions::default()).unwrap());
        assert!(!root.join(STORE_DIR).exists());
        assert_eq!(load(root).unwrap(), None);

        let mut e = ex(&["venv", "docs/build"]);
        e.add_pattern("*.egg-info");
        assert!(save(root, &e).unwrap());
        assert_eq!(load(root).unwrap(), Some(e.clone()));
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        assert!(
            text.contains("version = 1\n\n[exclusions]\npaths = [\n    \"docs/build\",\n    \"venv\",\n]\npatterns = [\"*.egg-info\"]\n"),
            "{text}"
        );
        // No temporary files are left behind.
        let files: Vec<_> = std::fs::read_dir(root.join(STORE_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files, [SETTINGS_FILE]);

        // Emptied: the file stays, with an empty list.
        assert!(save(root, &Exclusions::default()).unwrap());
        assert_eq!(load(root).unwrap(), Some(Exclusions::default()));
    }

    #[test]
    fn unknown_keys_and_comments_survive_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(STORE_DIR)).unwrap();
        let original = "# shared settings\nversion = 1\nfuture = \"keep\"\n\n[exclusions]\npaths = [\"old\"]\nnote = 3 # mine\n\n[run]\nname = \"app\"\n";
        std::fs::write(settings_file(root), original).unwrap();
        assert_eq!(load(root).unwrap(), Some(ex(&["old"])));
        save(root, &ex(&["new", "old"])).unwrap();
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        for kept in [
            "# shared settings",
            "future = \"keep\"",
            "note = 3 # mine",
            "[run]\nname = \"app\"",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        assert_eq!(load(root).unwrap(), Some(ex(&["new", "old"])));
    }

    #[test]
    fn refuses_newer_and_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(STORE_DIR)).unwrap();
        let newer = "version = 2\n[exclusions]\npaths = [\"a\"]\n";
        std::fs::write(settings_file(root), newer).unwrap();
        // Read what is understood, but never downgrade the file.
        assert_eq!(load(root).unwrap(), Some(ex(&["a"])));
        assert!(matches!(
            save(root, &ex(&["b"])),
            Err(StoreError::Newer { version: 2, .. })
        ));
        assert_eq!(std::fs::read_to_string(settings_file(root)).unwrap(), newer);

        let broken = "version = 1\n[exclusions\n";
        std::fs::write(settings_file(root), broken).unwrap();
        assert!(matches!(load(root), Err(StoreError::Invalid { .. })));
        assert!(save(root, &ex(&["b"])).is_err());
        assert_eq!(
            std::fs::read_to_string(settings_file(root)).unwrap(),
            broken
        );

        std::fs::write(settings_file(root), "exclusions = 3\n").unwrap();
        assert!(matches!(load(root), Err(StoreError::Invalid { .. })));
    }

    #[test]
    fn stored_paths_are_cleaned_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(STORE_DIR)).unwrap();
        std::fs::write(
            settings_file(root),
            "[exclusions]\npaths = [\"./a/\", \"../out\", \"\", \"b//c\", 7]\npatterns = [\"*.tmp\", \"*.tmp\", \" \"]\n",
        )
        .unwrap();
        let mut want = ex(&["a", "b/c"]);
        want.add_pattern("*.tmp");
        assert_eq!(load(root).unwrap(), Some(want));
    }
}
