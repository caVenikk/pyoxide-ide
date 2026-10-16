use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value as Json};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

/// The format version Pyoxide writes; a file of a newer one is read and
/// never rewritten.
pub const VERSION: i64 = 1;
const VERSION_KEY: &str = "version";

/// A typed part of the settings: a table of `settings.toml` (dotted,
/// `system.files`) read into a struct with defaults. Several types may
/// share a table when their keys differ. Use `#[serde(default)]`, so that
/// keys the file lacks take their defaults.
pub trait Section: Default + Serialize + DeserializeOwned + 'static {
    /// The table's dotted key.
    const KEY: &'static str;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("unknown setting `{0}`")]
    Unknown(String),
    #[error("invalid value for `{key}`: {message}")]
    Invalid { key: String, message: String },
}

/// Why the file is not used as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// It could not be read as TOML: the defaults apply, and the file is
    /// not written over until a setting changes.
    Broken(String),
    /// It was written by a newer Pyoxide: read, never rewritten.
    Newer(i64),
}

type Validate = fn(&Json) -> Result<(), String>;

fn validate<T: Section>(value: &Json) -> Result<(), String> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The application settings: the file's document (with what Pyoxide does
/// not know), the registered sections with their defaults, and the
/// changes not saved yet.
pub struct Store {
    file: Option<PathBuf>,
    doc: DocumentMut,
    problem: Option<Problem>,
    /// Default values by dotted key (`system.files.idle-seconds`).
    defaults: BTreeMap<String, Json>,
    /// Validators of the registered sections by their keys.
    sections: BTreeMap<&'static str, Vec<Validate>>,
    /// Keys changed since the last save: the value to write, or `None` to
    /// remove the key (back to its default).
    pending: BTreeMap<String, Option<Json>>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// A store without a file (tests; nothing is saved).
    pub fn new() -> Self {
        Self {
            file: None,
            doc: DocumentMut::new(),
            problem: None,
            defaults: BTreeMap::new(),
            sections: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }

    /// Reads `file` (none yet: the defaults) and saves there.
    pub fn load(file: PathBuf) -> Self {
        let mut store = Self::new();
        match std::fs::read_to_string(&file) {
            Ok(text) => match text.parse::<DocumentMut>() {
                Ok(doc) => {
                    if let Some(v) = version_of(&doc).filter(|v| *v > VERSION) {
                        store.problem = Some(Problem::Newer(v));
                    }
                    store.doc = doc;
                }
                Err(e) => store.problem = Some(Problem::Broken(e.to_string())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => store.problem = Some(Problem::Broken(e.to_string())),
        }
        store.file = Some(file);
        store
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub fn problem(&self) -> Option<&Problem> {
        self.problem.as_ref()
    }

    /// Makes a section known: its defaults and its keys for [`Self::get`]
    /// and [`Self::set`]. Registering it again changes nothing.
    pub fn register<T: Section>(&mut self) {
        let validators = self.sections.entry(T::KEY).or_default();
        let check = validate::<T> as Validate;
        if validators.contains(&check) {
            return;
        }
        validators.push(check);
        if let Ok(json) = serde_json::to_value(T::default()) {
            flatten(T::KEY, &json, &mut self.defaults);
        }
    }

    /// The section as stored, with defaults for the keys the file lacks or
    /// holds values of the wrong type for.
    pub fn section<T: Section>(&self) -> T {
        let Ok(Json::Object(mut value)) = serde_json::to_value(T::default()) else {
            return T::default();
        };
        if let Some(Json::Object(stored)) = self.stored(T::KEY) {
            for (key, v) in stored {
                let mut trial = value.clone();
                trial.insert(key, v);
                if serde_json::from_value::<T>(Json::Object(trial.clone())).is_ok() {
                    value = trial;
                }
            }
        }
        serde_json::from_value(Json::Object(value)).unwrap_or_default()
    }

    /// Stores a section; keys with their default values are removed from
    /// the file. True when anything changed.
    pub fn set_section<T: Section>(&mut self, value: &T) -> bool {
        self.register::<T>();
        let Ok(json) = serde_json::to_value(value) else {
            return false;
        };
        let mut keys = BTreeMap::new();
        flatten(T::KEY, &json, &mut keys);
        let mut changed = false;
        for (key, v) in keys {
            changed |= self.put(&key, v);
        }
        changed
    }

    /// A setting (dotted key) or a table of them, as stored or by default;
    /// `""` gives every setting. `None` for a key nothing knows.
    pub fn get(&self, key: &str) -> Option<Json> {
        if key.is_empty() {
            let mut all = Json::Object(Map::new());
            for (k, v) in &self.defaults {
                insert_nested(&mut all, k, v.clone());
            }
            let mut stored = table_to_json(self.doc.as_table());
            if let Json::Object(o) = &mut stored {
                o.remove(VERSION_KEY);
            }
            merge(&mut all, stored);
            return Some(all);
        }
        if let Some(default) = self.defaults.get(key) {
            return Some(self.stored(key).unwrap_or_else(|| default.clone()));
        }
        let prefix = format!("{key}.");
        let mut table = Json::Object(Map::new());
        let mut known = false;
        for (k, v) in self.defaults.range(prefix.clone()..) {
            let Some(rest) = k.strip_prefix(&prefix) else {
                break;
            };
            insert_nested(&mut table, rest, v.clone());
            known = true;
        }
        match self.stored(key) {
            Some(stored @ Json::Object(_)) => {
                merge(&mut table, stored);
                Some(table)
            }
            Some(stored) if !known => Some(stored),
            _ if known => Some(table),
            _ => None,
        }
    }

    /// Changes one setting (a key of a registered section). Returns the
    /// section's key when the value changed.
    pub fn set(&mut self, key: &str, value: Json) -> Result<Option<&'static str>, SettingsError> {
        if !self.defaults.contains_key(key) {
            return Err(SettingsError::Unknown(key.into()));
        }
        let section = self
            .section_of(key)
            .ok_or_else(|| SettingsError::Unknown(key.into()))?;
        let mut table = self
            .get(section)
            .unwrap_or_else(|| Json::Object(Map::new()));
        let rest = &key[section.len() + 1..];
        insert_nested(&mut table, rest, value.clone());
        for check in &self.sections[section] {
            check(&table).map_err(|message| SettingsError::Invalid {
                key: key.into(),
                message,
            })?;
        }
        Ok(self.put(key, value).then_some(section))
    }

    /// The registered section a key belongs to (the longest prefix).
    pub fn section_of(&self, key: &str) -> Option<&'static str> {
        self.sections
            .keys()
            .filter(|s| key.strip_prefix(**s).is_some_and(|r| r.starts_with('.')))
            .max_by_key(|s| s.len())
            .copied()
    }

    /// Changes not saved yet.
    pub fn is_dirty(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Takes the changes to save: `None` without a file, without changes,
    /// or for a file of a newer version (never rewritten).
    pub fn take_save(&mut self) -> Option<SavePlan> {
        if matches!(self.problem, Some(Problem::Newer(_))) || self.pending.is_empty() {
            return None;
        }
        let file = self.file.clone()?;
        self.problem = None;
        Some(SavePlan {
            file,
            changes: std::mem::take(&mut self.pending),
        })
    }

    /// Saves the changes now, on the calling thread.
    pub fn save_now(&mut self) -> std::io::Result<()> {
        match self.take_save() {
            Some(plan) => plan.write(),
            None => Ok(()),
        }
    }

    /// The stored value of a key (or table), without defaults.
    fn stored(&self, key: &str) -> Option<Json> {
        let mut table: &dyn toml_edit::TableLike = self.doc.as_table();
        let mut parts = key.split('.').peekable();
        while let Some(part) = parts.next() {
            let item = table.get(part)?;
            if parts.peek().is_none() {
                return item_to_json(item);
            }
            table = item.as_table_like()?;
        }
        None
    }

    /// Writes one key (its default or `null` removes it); true when the
    /// stored value changed.
    fn put(&mut self, key: &str, value: Json) -> bool {
        let value = (!value.is_null() && self.defaults.get(key) != Some(&value)).then_some(value);
        if self.stored(key) == value {
            return false;
        }
        write_key(self.doc.as_table_mut(), key, value.as_ref());
        self.pending.insert(key.into(), value);
        true
    }
}

/// Changes to write to the settings file. [`Self::write`] reads the file
/// again and changes only these keys there, so changes another window or
/// instance saved meanwhile stay; it may run on a background thread.
#[derive(Debug, Clone)]
pub struct SavePlan {
    file: PathBuf,
    changes: BTreeMap<String, Option<Json>>,
}

impl SavePlan {
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Writes the changes; a file of a newer version is left alone.
    pub fn write(&self) -> std::io::Result<()> {
        // One save at a time in this process.
        static WRITING: Mutex<()> = Mutex::new(());
        let _guard = WRITING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut doc = match std::fs::read_to_string(&self.file) {
            Ok(text) => text.parse::<DocumentMut>().unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
            Err(e) => return Err(e),
        };
        if version_of(&doc).is_some_and(|v| v > VERSION) {
            return Ok(());
        }
        if !doc.contains_key(VERSION_KEY) {
            let mut fresh = DocumentMut::new();
            fresh.insert(VERSION_KEY, toml_edit::value(VERSION));
            for (k, v) in doc.iter() {
                fresh.insert(k, v.clone());
            }
            doc = fresh;
        }
        for (key, value) in &self.changes {
            write_key(doc.as_table_mut(), key, value.as_ref());
        }
        write_atomic(&self.file, &doc.to_string())
    }
}

fn version_of(doc: &DocumentMut) -> Option<i64> {
    doc.get(VERSION_KEY).and_then(Item::as_integer)
}

/// Dotted keys of the leaves of `json` under `prefix`.
fn flatten(prefix: &str, json: &Json, out: &mut BTreeMap<String, Json>) {
    match json {
        Json::Object(o) => {
            for (k, v) in o {
                flatten(&format!("{prefix}.{k}"), v, out);
            }
        }
        v => {
            out.insert(prefix.into(), v.clone());
        }
    }
}

/// Puts `value` at the dotted `key` of an object.
fn insert_nested(target: &mut Json, key: &str, value: Json) {
    let mut current = target;
    let mut parts = key.split('.').peekable();
    while let Some(part) = parts.next() {
        if !current.is_object() {
            *current = Json::Object(Map::new());
        }
        let Json::Object(o) = current else {
            return;
        };
        if parts.peek().is_none() {
            o.insert(part.into(), value);
            return;
        }
        current = o
            .entry(part.to_string())
            .or_insert_with(|| Json::Object(Map::new()));
    }
}

/// Lays `over` onto `base`, object by object.
fn merge(base: &mut Json, over: Json) {
    match (base, over) {
        (Json::Object(b), Json::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

fn item_to_json(item: &Item) -> Option<Json> {
    match item {
        Item::None => None,
        Item::Value(v) => Some(value_to_json(v)),
        Item::Table(t) => Some(table_to_json(t)),
        Item::ArrayOfTables(a) => Some(Json::Array(a.iter().map(table_to_json).collect())),
    }
}

fn table_to_json(table: &Table) -> Json {
    Json::Object(
        table
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), item_to_json(v)?)))
            .collect(),
    )
}

fn value_to_json(value: &Value) -> Json {
    match value {
        Value::String(s) => Json::String(s.value().clone()),
        Value::Integer(i) => Json::from(*i.value()),
        Value::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Boolean(b) => Json::Bool(*b.value()),
        Value::Datetime(d) => Json::String(d.value().to_string()),
        Value::Array(a) => Json::Array(a.iter().map(value_to_json).collect()),
        Value::InlineTable(t) => Json::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), value_to_json(v)))
                .collect(),
        ),
    }
}

fn json_to_value(json: &Json) -> Option<Value> {
    Some(match json {
        Json::Null => return None,
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::from(i),
            None => Value::from(n.as_f64()?),
        },
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(a) => Value::Array(a.iter().filter_map(json_to_value).collect()),
        Json::Object(o) => {
            let mut t = InlineTable::new();
            for (k, v) in o {
                if let Some(v) = json_to_value(v) {
                    t.insert(k, v);
                }
            }
            Value::InlineTable(t)
        }
    })
}

/// Sets (or with `None` removes) the dotted `key` in `root`, keeping the
/// comments around an existing value; tables left empty go too.
fn write_key(root: &mut Table, key: &str, value: Option<&Json>) {
    let parts: Vec<&str> = key.split('.').collect();
    let Some((leaf, path)) = parts.split_last() else {
        return;
    };
    let Some(value) = value.and_then(json_to_value) else {
        remove_key(root, path, leaf);
        return;
    };
    let mut table = root;
    for part in path {
        let entry = table.entry(part).or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        });
        if !entry.is_table() {
            *entry = Item::Table(Table::new());
        }
        let Some(next) = entry.as_table_mut() else {
            return;
        };
        table = next;
    }
    match table.get_mut(leaf).and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            table.insert(leaf, Item::Value(value));
        }
    }
}

fn remove_key(table: &mut Table, path: &[&str], leaf: &str) {
    match path.split_first() {
        None => {
            table.remove(leaf);
        }
        Some((first, rest)) => {
            let Some(inner) = table.get_mut(first).and_then(Item::as_table_mut) else {
                return;
            };
            remove_key(inner, rest, leaf);
            if inner.is_empty() {
                table.remove(first);
            }
        }
    }
}

/// Replaces `path` with `text` at once: a temporary file next to it,
/// flushed to disk, then renamed over it.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
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
    use serde::Deserialize;
    use serde_json::json;

    use super::*;
    use crate::{SaveSettings, SyncSettings};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(default, rename_all = "kebab-case")]
    struct Font {
        family: String,
        size: u32,
    }

    impl Default for Font {
        fn default() -> Self {
            Self {
                family: "JetBrains Mono".into(),
                size: 13,
            }
        }
    }

    impl Section for Font {
        const KEY: &'static str = "editor.font";
    }

    fn store(dir: &tempfile::TempDir, text: Option<&str>) -> Store {
        let file = dir.path().join("settings.toml");
        if let Some(text) = text {
            std::fs::write(&file, text).unwrap();
        }
        let mut s = Store::load(file);
        s.register::<SaveSettings>();
        s.register::<SyncSettings>();
        s.register::<Font>();
        s
    }

    fn read(dir: &tempfile::TempDir) -> String {
        std::fs::read_to_string(dir.path().join("settings.toml")).unwrap()
    }

    #[test]
    fn sections_take_defaults_and_stored_values() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(
            &dir,
            Some("version = 1\n[editor.font]\nsize = 15\nfamily = 3\n"),
        );
        // A value of the wrong type keeps its default, the others apply.
        assert_eq!(
            s.section::<Font>(),
            Font {
                family: "JetBrains Mono".into(),
                size: 15
            }
        );
        assert_eq!(s.section::<SaveSettings>(), SaveSettings::default());
        assert_eq!(s.get("editor.font.size"), Some(json!(15)));
        assert_eq!(s.get("system.files.idle-seconds"), Some(json!(2)));
        assert_eq!(
            s.get("system.files"),
            Some(json!({
                "idle-seconds": 2,
                "save-on-deactivation": true,
                "save-on-idle": true,
                "save-on-tab-switch": true,
                "sync-on-activation": true,
            }))
        );
        assert_eq!(s.get("nothing.here"), None);
        assert_eq!(
            s.section_of("system.files.idle-seconds"),
            Some("system.files")
        );
    }

    #[test]
    fn unknown_keys_and_comments_survive_and_defaults_are_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(
            &dir,
            Some(
                "version = 1\n# mine\nfuture = \"x\"\n\n[system.files]\n# idle\nidle-seconds = 5 # five\nother = true\n",
            ),
        );
        assert_eq!(s.section::<SaveSettings>().idle_seconds, 5);
        let section = s.set("system.files.idle-seconds", json!(30)).unwrap();
        assert_eq!(section, Some("system.files"));
        s.set("system.files.save-on-idle", json!(true)).unwrap();
        s.save_now().unwrap();
        let text = read(&dir);
        assert!(text.contains("# mine\nfuture = \"x\""), "{text}");
        assert!(text.contains("# idle\nidle-seconds = 30 # five"), "{text}");
        assert!(text.contains("other = true"), "{text}");
        assert!(!text.contains("save-on-idle"), "{text}");
        // Back to the default: the key goes; the table keeps the unknown one.
        s.set("system.files.idle-seconds", json!(2)).unwrap();
        s.save_now().unwrap();
        let text = read(&dir);
        assert!(!text.contains("idle-seconds"), "{text}");
        assert!(text.contains("[system.files]"), "{text}");
        // No temporary files are left behind.
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn a_new_file_gets_the_version_and_only_changed_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir, None);
        let changed = s.set_section(&SaveSettings {
            idle_seconds: 30,
            ..SaveSettings::default()
        });
        assert!(changed);
        assert!(!s.set_section(&SaveSettings {
            idle_seconds: 30,
            ..SaveSettings::default()
        }));
        s.save_now().unwrap();
        assert_eq!(
            read(&dir),
            "version = 1\n\n[system.files]\nidle-seconds = 30\n"
        );
    }

    #[test]
    fn a_save_keeps_what_another_instance_saved_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = store(&dir, None);
        let mut b = store(&dir, None);
        a.set("editor.font.size", json!(16)).unwrap();
        b.set("system.files.idle-seconds", json!(9)).unwrap();
        a.save_now().unwrap();
        b.save_now().unwrap();
        let c = store(&dir, None);
        assert_eq!(c.section::<Font>().size, 16);
        assert_eq!(c.section::<SaveSettings>().idle_seconds, 9);
    }

    #[test]
    fn a_newer_file_is_read_and_never_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let text = "version = 2\n[editor.font]\nsize = 20\n";
        let mut s = store(&dir, Some(text));
        assert_eq!(s.problem(), Some(&Problem::Newer(2)));
        assert_eq!(s.section::<Font>().size, 20);
        s.set("editor.font.size", json!(12)).unwrap();
        assert!(s.take_save().is_none());
        assert_eq!(read(&dir), text);
    }

    #[test]
    fn a_broken_file_gives_defaults_and_is_replaced_after_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir, Some("version = = 1\n"));
        assert!(matches!(s.problem(), Some(Problem::Broken(_))));
        assert_eq!(s.section::<Font>(), Font::default());
        assert!(s.take_save().is_none());
        assert_eq!(read(&dir), "version = = 1\n");
        s.set("editor.font.size", json!(14)).unwrap();
        s.save_now().unwrap();
        assert_eq!(read(&dir), "version = 1\n\n[editor.font]\nsize = 14\n");
        assert_eq!(s.problem(), None);
    }

    #[test]
    fn set_checks_the_key_and_the_type() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(&dir, None);
        assert_eq!(
            s.set("editor.font.weight", json!(1)),
            Err(SettingsError::Unknown("editor.font.weight".into()))
        );
        assert!(matches!(
            s.set("editor.font.size", json!("big")),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(matches!(
            s.set("system.files.idle-seconds", json!(-1)),
            Err(SettingsError::Invalid { .. })
        ));
        assert_eq!(s.set("editor.font.size", json!(13)), Ok(None));
        assert!(!s.is_dirty());
        // The test switch of `SyncSettings` is not a setting.
        assert_eq!(s.get("system.files.watch"), None);
    }
}
