//! Pyoxide's own project folder ([`store_dir`] in the project root) and the
//! shared project settings in it (ADR 0008). The personal project state
//! next to them is [`crate::workspace`] (ADR 0009).
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
//!
//! [roots]
//! sources = [
//!     ".",
//!     "services/api",
//! ]
//! tests = ["tests"]
//! ```
//!
//! Paths are relative to the project root with `/` separators on every OS;
//! patterns are file name masks matched below the root (see
//! [`NameMasks`](crate::NameMasks)). `[roots]` lists the folders marked
//! as roots by kind, `.` for the project root ([`Roots`], ADR 0013); once
//! the table is there it is the only source of the project's roots.
//!
//! Other settings are values by table and key ([`load_table`],
//! [`load_value`], [`save_values`]); tables are dotted paths, so a product
//! keeps its settings in its own tables (`[python.tools.ruff]`, ADR 0010)
//! under the same rules.

use std::collections::{BTreeSet, HashSet};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex, PoisonError};

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use crate::idea;
use crate::roots::{DetectRoots, Roots, detected_roots};

/// Name of the product's project folder in the project root.
pub fn store_dir() -> &'static str {
    app_info::project_dir()
}
/// The shared project settings in [`store_dir`].
pub const SETTINGS_FILE: &str = "project.toml";
/// Version of the settings format this build reads and writes.
pub const SETTINGS_VERSION: i64 = 1;

fn header() -> String {
    format!(
        "# {} project settings. Paths are relative to the project folder.\n",
        app_info::name()
    )
}

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

/// `path` relative to `root` in the stored form of a root: `.` for the
/// root itself.
pub fn relative_or_root(root: &Path, path: &Path) -> Option<String> {
    if lexical(root) == lexical(path) {
        return Some(crate::roots::PROJECT_ROOT.to_owned());
    }
    relative_to(root, path)
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
        "{} was written by a newer version of {} (settings version {version})",
        path.display(),
        app_info::name()
    )]
    Newer { path: PathBuf, version: i64 },
}

/// The settings file of a project.
pub fn settings_file(root: &Path) -> PathBuf {
    root.join(store_dir()).join(SETTINGS_FILE)
}

/// Whether the project has shared settings yet. Only the shared settings
/// count: the personal state ([`crate::workspace`]) in the same folder
/// does not stop the reference IDE's exclusions being read (ADR 0009).
pub fn has_store(root: &Path) -> bool {
    settings_file(root).is_file()
}

pub(crate) fn read_document(path: &Path) -> Result<Option<DocumentMut>, StoreError> {
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

/// The `version` of a store file; `default` when it has none.
pub(crate) fn version_of(doc: &DocumentMut, default: i64) -> i64 {
    doc.get("version")
        .and_then(Item::as_integer)
        .unwrap_or(default)
}

fn version(doc: &DocumentMut) -> i64 {
    version_of(doc, SETTINGS_VERSION)
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
    let _lock = lock_root(root);
    if has_store(root) {
        return Ok(None);
    }
    match idea::read_exclusions(root) {
        Some(exclusions) if !exclusions.is_empty() => {
            save_locked(root, &exclusions)?;
            Ok(Some(exclusions))
        }
        _ => Ok(None),
    }
}

/// Writes the exclusions into the project settings, keeping everything
/// else in the file. Nothing is created while there is nothing to store.
/// Returns whether the file was written.
pub fn save(root: &Path, exclusions: &Exclusions) -> Result<bool, StoreError> {
    let _lock = lock_root(root);
    save_locked(root, exclusions)
}

fn save_locked(root: &Path, exclusions: &Exclusions) -> Result<bool, StoreError> {
    let path = settings_file(root);
    let existing = read_document(&path)?;
    if existing.is_none() && exclusions.is_empty() {
        return Ok(false);
    }
    let mut doc = match existing {
        Some(doc) => doc,
        None => format!("{}version = {SETTINGS_VERSION}\n", header())
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

// ---- roots ------------------------------------------------------------------

/// The roots of the project settings (`[roots]`); `None` when the file or
/// the table does not exist (the project has not stored its roots yet).
pub fn load_roots(root: &Path) -> Result<Option<Roots>, StoreError> {
    let path = settings_file(root);
    let Some(doc) = read_document(&path)? else {
        return Ok(None);
    };
    let Some(table) = doc.get("roots") else {
        return Ok(None);
    };
    let table = table
        .as_table_like()
        .ok_or_else(|| not_a_table(&path, "roots"))?;
    let mut out = Roots::default();
    for (kind, item) in table.iter() {
        let Some(paths) = item.as_array() else {
            continue;
        };
        for p in paths.iter().filter_map(Value::as_str) {
            out.mark(p, kind);
        }
    }
    Ok(Some(out))
}

/// The roots in effect: the project settings once they have `[roots]`;
/// before that the reference IDE's source folders (`.idea`) and the
/// roots `detect` finds (blocking).
pub fn load_or_detect_roots(root: &Path, detect: Option<DetectRoots>) -> Result<Roots, StoreError> {
    if let Some(stored) = load_roots(root)? {
        return Ok(stored);
    }
    let mut roots = idea::read_source_folders(root).unwrap_or_default();
    roots.add_missing(&detected_roots(root, detect));
    Ok(roots)
}

/// Copies the reference IDE's source folders into the project settings,
/// once: only while the settings have no `[roots]` and `.idea` names
/// source folders; the roots `detect` finds are stored with them. Returns
/// the roots when they were written.
pub fn import_idea_roots(
    root: &Path,
    detect: Option<DetectRoots>,
) -> Result<Option<Roots>, StoreError> {
    let _lock = lock_root(root);
    if load_roots(root)?.is_some() {
        return Ok(None);
    }
    match idea::read_source_folders(root) {
        Some(mut roots) if !roots.is_empty() => {
            roots.add_missing(&detected_roots(root, detect));
            save_roots_locked(root, &roots)?;
            Ok(Some(roots))
        }
        _ => Ok(None),
    }
}

/// Writes the roots into the project settings (`[roots]`, one array of
/// paths per kind), keeping everything else in the file. The table is
/// written also when it is empty: from then on it holds the project's
/// roots.
pub fn save_roots(root: &Path, roots: &Roots) -> Result<bool, StoreError> {
    let _lock = lock_root(root);
    save_roots_locked(root, roots)
}

fn save_roots_locked(root: &Path, roots: &Roots) -> Result<bool, StoreError> {
    let path = settings_file(root);
    let mut doc = match read_document(&path)? {
        Some(doc) => doc,
        None => format!("{}version = {SETTINGS_VERSION}\n", header())
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
    if doc.get("roots").is_none() {
        doc["roots"] = Item::Table(Table::new());
    }
    let table = doc["roots"]
        .as_table_like_mut()
        .ok_or_else(|| not_a_table(&path, "roots"))?;
    // Kinds without roots go; keys that are no list of paths stay.
    let gone: Vec<String> = table
        .iter()
        .filter(|(kind, item)| item.is_array() && roots.paths(kind).next().is_none())
        .map(|(kind, _)| kind.to_owned())
        .collect();
    for kind in gone {
        table.remove(&kind);
    }
    for kind in roots.kinds() {
        let paths: Vec<String> = roots.paths(kind).map(str::to_owned).collect();
        table.insert(kind, Item::Value(Value::Array(lines(&paths))));
    }
    write_atomic(&path, &doc.to_string()).map_err(|source| StoreError::Io { path, source })?;
    Ok(true)
}

// ---- values by table and key ----------------------------------------------

/// The table at a dotted path (`python.tools.ruff`) of a store file;
/// `None` when a part of the path is missing. A part that is not a table
/// is an error.
pub(crate) fn find_table<'a>(
    doc: &'a DocumentMut,
    table: &str,
    path: &Path,
) -> Result<Option<&'a dyn TableLike>, StoreError> {
    let mut current: &dyn TableLike = doc.as_table();
    for part in table.split('.') {
        let Some(item) = current.get(part) else {
            return Ok(None);
        };
        current = item
            .as_table_like()
            .ok_or_else(|| not_a_table(path, table))?;
    }
    Ok(Some(current))
}

/// The table at a dotted path, created when missing (its parents as
/// implicit tables, so only `[python.tools.ruff]` is written); `None`
/// when it is missing and `create` is false.
pub(crate) fn find_table_mut<'a>(
    doc: &'a mut DocumentMut,
    table: &str,
    path: &Path,
    create: bool,
) -> Result<Option<&'a mut dyn TableLike>, StoreError> {
    let parts: Vec<&str> = table.split('.').collect();
    let mut current: &mut dyn TableLike = doc.as_table_mut();
    for (i, part) in parts.iter().enumerate() {
        if current.get(part).is_none() {
            if !create {
                return Ok(None);
            }
            let mut new = Table::new();
            new.set_implicit(i + 1 < parts.len());
            current.insert(part, Item::Table(new));
        }
        current = current
            .get_mut(part)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| not_a_table(path, table))?;
    }
    Ok(Some(current))
}

fn not_a_table(path: &Path, table: &str) -> StoreError {
    StoreError::Invalid {
        path: path.to_owned(),
        message: format!("`{table}` is not a table"),
    }
}

/// The table at a dotted path (`python.tools.ruff`) of the project
/// settings, with the values as they are stored; `None` when the file or
/// the table does not exist. Products deserialize their own settings from
/// it (`toml::Table::try_into`).
pub fn load_table(root: &Path, table: &str) -> Result<Option<toml::Table>, StoreError> {
    let path = settings_file(root);
    let Some(doc) = read_document(&path)? else {
        return Ok(None);
    };
    let Some(found) = find_table(&doc, table, &path)? else {
        return Ok(None);
    };
    let mut copy = DocumentMut::new();
    for (key, item) in found.iter() {
        copy.insert(key, item.clone());
    }
    toml::from_str(&copy.to_string())
        .map(Some)
        .map_err(|e| StoreError::Invalid {
            path,
            message: e.to_string(),
        })
}

/// The value of `key` in the table at a dotted path of the project
/// settings; `None` when the file, the table or the key is missing.
pub fn load_value(root: &Path, table: &str, key: &str) -> Result<Option<toml::Value>, StoreError> {
    Ok(load_table(root, table)?.and_then(|mut t| t.remove(key)))
}

/// Sets keys of the table at a dotted path of the project settings
/// (`None` removes a key), keeping everything else in the file, as
/// [`save`] does: the file is created only when there is a value to
/// store, a file of a newer version is not rewritten, and the write
/// replaces the file at once. Returns whether the file was written.
pub fn save_values(
    root: &Path,
    table: &str,
    values: &[(&str, Option<toml::Value>)],
) -> Result<bool, StoreError> {
    let _lock = lock_root(root);
    let path = settings_file(root);
    let existing = read_document(&path)?;
    if existing.is_none() && values.iter().all(|(_, v)| v.is_none()) {
        return Ok(false);
    }
    let mut doc = match existing {
        Some(doc) => doc,
        None => format!("{}version = {SETTINGS_VERSION}\n", header())
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
    let mut changed = false;
    for (key, value) in values {
        let create = value.is_some();
        let Some(t) = find_table_mut(&mut doc, table, &path, create)? else {
            continue;
        };
        match value {
            Some(value) => {
                t.insert(key, to_item(value));
                changed = true;
            }
            None => changed |= t.remove(key).is_some(),
        }
    }
    if !changed {
        return Ok(false);
    }
    write_atomic(&path, &doc.to_string()).map_err(|source| StoreError::Io { path, source })?;
    Ok(true)
}

/// A value in the form `toml_edit` keeps (a table becomes a table).
fn to_item(value: &toml::Value) -> Item {
    let mut wrapper = toml::Table::new();
    wrapper.insert("v".to_owned(), value.clone());
    let text = toml::to_string(&wrapper).expect("a TOML value serializes");
    let doc: DocumentMut = text.parse().expect("serialized TOML parses");
    doc.get("v").cloned().expect("the value is there")
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

/// The project roots whose store files are being written in this process.
static WRITING: LazyLock<(Mutex<HashSet<PathBuf>>, Condvar)> = LazyLock::new(Default::default);

/// Holds the right to write the store files of one project root.
pub(crate) struct RootLock(PathBuf);

/// Waits until no other thread of this process writes the store files of
/// `root`, and holds them until the lock is dropped. Writes read, change
/// and replace a whole file: two writers at once (the import of the
/// reference IDE's exclusions and a product's settings, on background
/// threads) would each drop the other's change.
pub(crate) fn lock_root(root: &Path) -> RootLock {
    let key = native_path::canonicalize(root).unwrap_or_else(|_| lexical(root));
    let (writing, released) = &*WRITING;
    let mut busy = writing.lock().unwrap_or_else(PoisonError::into_inner);
    while busy.contains(&key) {
        busy = released.wait(busy).unwrap_or_else(PoisonError::into_inner);
    }
    busy.insert(key.clone());
    RootLock(key)
}

impl Drop for RootLock {
    fn drop(&mut self) {
        let (writing, released) = &*WRITING;
        writing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
        released.notify_all();
    }
}

/// Replaces `path` with `text` at once: a temporary file next to it,
/// flushed to disk, then renamed over it.
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().expect("a store file has a folder");
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
        assert!(!root.join(store_dir()).exists());
        assert_eq!(load(root).unwrap(), None);

        let mut e = ex(&["venv", "docs/build"]);
        e.add_pattern("*.egg-info");
        assert!(save(root, &e).unwrap());
        assert_eq!(load(root).unwrap(), Some(e.clone()));
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        assert!(text.starts_with(&header()), "{text}");
        assert!(
            text.contains("version = 1\n\n[exclusions]\npaths = [\n    \"docs/build\",\n    \"venv\",\n]\npatterns = [\"*.egg-info\"]\n"),
            "{text}"
        );
        // No temporary files are left behind.
        let files: Vec<_> = std::fs::read_dir(root.join(store_dir()))
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
        std::fs::create_dir_all(root.join(store_dir())).unwrap();
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
        std::fs::create_dir_all(root.join(store_dir())).unwrap();
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
    fn values_by_dotted_table_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Nothing to store: nothing is created.
        assert!(!save_values(root, "python.tools.ruff", &[("enabled", None)]).unwrap());
        assert!(!root.join(store_dir()).exists());
        assert_eq!(load_table(root, "python.tools.ruff").unwrap(), None);
        assert_eq!(
            load_value(root, "python.tools.ruff", "enabled").unwrap(),
            None
        );

        assert!(
            save_values(
                root,
                "python.tools.ruff",
                &[
                    ("enabled", Some(true.into())),
                    ("lookup", Some("sdk".into()))
                ],
            )
            .unwrap()
        );
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        assert!(text.starts_with(&header()), "{text}");
        // Parents are implicit: only the table itself gets a header.
        assert!(
            text.ends_with(
                "version = 1\n\n[python.tools.ruff]\nenabled = true\nlookup = \"sdk\"\n"
            ),
            "{text}"
        );
        assert_eq!(
            load_value(root, "python.tools.ruff", "lookup").unwrap(),
            Some(toml::Value::from("sdk"))
        );
        save_values(root, "python.type-engine", &[("name", Some("ty".into()))]).unwrap();
        let args = toml::Value::Array(vec!["--preview".into(), 3.into()]);
        save_values(root, "python.tools.ruff", &[("args", Some(args.clone()))]).unwrap();
        let ruff = load_table(root, "python.tools.ruff").unwrap().unwrap();
        assert_eq!(ruff.get("enabled"), Some(&toml::Value::Boolean(true)));
        assert_eq!(ruff.get("args"), Some(&args));
        let python = load_table(root, "python").unwrap().unwrap();
        assert_eq!(python["type-engine"]["name"], toml::Value::from("ty"));
        assert_eq!(python["tools"]["ruff"]["lookup"], toml::Value::from("sdk"));

        // Removing keys; missing tables and keys write nothing.
        assert!(save_values(root, "python.tools.ruff", &[("lookup", None)]).unwrap());
        assert_eq!(
            load_value(root, "python.tools.ruff", "lookup").unwrap(),
            None
        );
        assert!(!save_values(root, "python.tools.ruff", &[("lookup", None)]).unwrap());
        assert!(!save_values(root, "python.tools.ty", &[("enabled", None)]).unwrap());

        // Exclusions and values live side by side.
        save(root, &ex(&["venv"])).unwrap();
        assert_eq!(load(root).unwrap(), Some(ex(&["venv"])));
        assert_eq!(
            load_value(root, "python.tools.ruff", "enabled").unwrap(),
            Some(toml::Value::Boolean(true))
        );
    }

    #[test]
    fn values_keep_the_rest_of_the_file_and_refuse_newer_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(store_dir())).unwrap();
        let original = "# shared\nversion = 1\n\n[exclusions]\npaths = [\"a\"]\n\n[python.tools.ruff]\nenabled = false # off for now\nfuture = \"keep\"\n";
        std::fs::write(settings_file(root), original).unwrap();
        save_values(root, "python.tools.ruff", &[("enabled", Some(true.into()))]).unwrap();
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        for kept in [
            "# shared",
            "[exclusions]\npaths = [\"a\"]",
            "future = \"keep\"",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        assert_eq!(load(root).unwrap(), Some(ex(&["a"])));
        assert_eq!(
            load_value(root, "python.tools.ruff", "enabled").unwrap(),
            Some(toml::Value::Boolean(true))
        );

        let newer = "version = 2\n[python.tools.ruff]\nenabled = true\n";
        std::fs::write(settings_file(root), newer).unwrap();
        assert_eq!(
            load_value(root, "python.tools.ruff", "enabled").unwrap(),
            Some(toml::Value::Boolean(true))
        );
        assert!(matches!(
            save_values(
                root,
                "python.tools.ruff",
                &[("enabled", Some(false.into()))]
            ),
            Err(StoreError::Newer { version: 2, .. })
        ));
        assert_eq!(std::fs::read_to_string(settings_file(root)).unwrap(), newer);

        std::fs::write(settings_file(root), "python = 3\n").unwrap();
        assert!(matches!(
            load_table(root, "python.tools"),
            Err(StoreError::Invalid { .. })
        ));
        assert!(matches!(
            save_values(root, "python.tools", &[("x", Some(1.into()))]),
            Err(StoreError::Invalid { .. })
        ));
    }

    #[test]
    fn stored_paths_are_cleaned_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(store_dir())).unwrap();
        std::fs::write(
            settings_file(root),
            "[exclusions]\npaths = [\"./a/\", \"../out\", \"\", \"b//c\", 7]\npatterns = [\"*.tmp\", \"*.tmp\", \" \"]\n",
        )
        .unwrap();
        let mut want = ex(&["a", "b/c"]);
        want.add_pattern("*.tmp");
        assert_eq!(load(root).unwrap(), Some(want));
    }

    fn marked(items: &[(&str, &str)]) -> Roots {
        let mut r = Roots::default();
        for (kind, path) in items {
            r.mark(path, kind);
        }
        r
    }

    #[test]
    fn roots_round_trip_beside_the_exclusions() {
        use crate::roots::{SOURCES, TESTS};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(load_roots(root).unwrap(), None);
        save(root, &ex(&["venv"])).unwrap();
        // A store without `[roots]` has stored none yet.
        assert_eq!(load_roots(root).unwrap(), None);
        let roots = marked(&[(SOURCES, "."), (SOURCES, "svc/api"), (TESTS, "tests")]);
        assert!(save_roots(root, &roots).unwrap());
        assert_eq!(load_roots(root).unwrap(), Some(roots.clone()));
        assert_eq!(load(root).unwrap(), Some(ex(&["venv"])));
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        assert!(
            text.ends_with(
                "[roots]\nsources = [\n    \".\",\n    \"svc/api\",\n]\ntests = [\n    \"tests\",\n]\n"
            ),
            "{text}"
        );
        // Unmarking everything keeps an empty table: the project's roots
        // are none (detected ones no longer apply).
        assert!(save_roots(root, &Roots::default()).unwrap());
        assert_eq!(load_roots(root).unwrap(), Some(Roots::default()));
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        assert!(text.ends_with("[roots]\n"), "{text}");

        // Keys that are no list of paths survive; a broken table is an
        // error.
        std::fs::write(
            settings_file(root),
            "version = 1\n[roots]\nnote = 3\nsources = [\"src\"]\n",
        )
        .unwrap();
        save_roots(root, &marked(&[(TESTS, "t")])).unwrap();
        let text = std::fs::read_to_string(settings_file(root)).unwrap();
        assert!(text.contains("note = 3"), "{text}");
        assert!(!text.contains("sources"), "{text}");
        std::fs::write(settings_file(root), "roots = 3\n").unwrap();
        assert!(matches!(load_roots(root), Err(StoreError::Invalid { .. })));
        assert!(save_roots(root, &Roots::default()).is_err());
    }

    #[test]
    fn roots_are_imported_once_with_the_detected_ones() {
        use crate::roots::{SOURCES, TESTS};
        fn detect(root: &Path) -> Vec<PathBuf> {
            vec![root.join("src")]
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Nothing stored, no `.idea`: the detected roots, nothing written.
        assert_eq!(
            load_or_detect_roots(root, Some(detect)).unwrap(),
            marked(&[(SOURCES, "src")])
        );
        assert_eq!(import_idea_roots(root, Some(detect)).unwrap(), None);
        assert!(!root.join(store_dir()).exists());

        std::fs::create_dir_all(root.join(".idea")).unwrap();
        std::fs::write(
            root.join(".idea/demo.iml"),
            r#"<module type="PYTHON_MODULE" version="4"><component name="NewModuleRootManager">
  <content url="file://$MODULE_DIR$">
    <sourceFolder url="file://$MODULE_DIR$" isTestSource="false" />
    <sourceFolder url="file://$MODULE_DIR$/tests" isTestSource="true" />
    <sourceFolder url="file://$MODULE_DIR$/src" isTestSource="true" />
  </content>
</component></module>"#,
        )
        .unwrap();
        // The reference's kind wins for a folder both name.
        let want = marked(&[(SOURCES, "."), (TESTS, "tests"), (TESTS, "src")]);
        assert_eq!(load_or_detect_roots(root, Some(detect)).unwrap(), want);
        assert_eq!(
            import_idea_roots(root, Some(detect)).unwrap(),
            Some(want.clone())
        );
        assert_eq!(load_roots(root).unwrap(), Some(want.clone()));
        // Once: later changes in `.idea` are not read.
        std::fs::remove_file(root.join(".idea/demo.iml")).unwrap();
        assert_eq!(import_idea_roots(root, Some(detect)).unwrap(), None);
        assert_eq!(load_or_detect_roots(root, None).unwrap(), want);
    }

    /// Writers on other threads (the exclusions import of the project
    /// panel, a product's settings) do not lose each other's changes.
    #[test]
    fn concurrent_writes_keep_every_table() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        let writer = |table: &'static str| {
            let root = root.clone();
            std::thread::spawn(move || {
                for i in 0..40 {
                    let key = format!("k{i}");
                    save_values(&root, table, &[(&key, Some(toml::Value::from(i)))]).unwrap();
                }
            })
        };
        let exclusions = {
            let root = root.clone();
            std::thread::spawn(move || {
                for i in 0..40 {
                    let mut e = load(&root).unwrap().unwrap_or_default();
                    e.add(&format!("dir{i}"));
                    save(&root, &e).unwrap();
                }
            })
        };
        let threads = [
            writer("python.tools.ruff"),
            writer("other.product"),
            exclusions,
        ];
        for t in threads {
            t.join().unwrap();
        }
        for table in ["python.tools.ruff", "other.product"] {
            assert_eq!(
                load_table(&root, table).unwrap().unwrap().len(),
                40,
                "{table}"
            );
        }
        assert!(!load(&root).unwrap().unwrap().is_empty());
        // The workspace file follows the same lock.
        let personal = |n: usize| {
            let root = root.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    let key = format!("t{n}k{i}");
                    crate::workspace::save_value(&root, "python", &key, Some("x")).unwrap();
                }
            })
        };
        let threads = [personal(1), personal(2)];
        for t in threads {
            t.join().unwrap();
        }
        for n in 1..=2 {
            for i in 0..20 {
                let key = format!("t{n}k{i}");
                assert!(
                    crate::workspace::load_value(&root, "python", &key)
                        .unwrap()
                        .is_some(),
                    "{key}"
                );
            }
        }
    }
}
