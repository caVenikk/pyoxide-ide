//! Storage of run configurations (ADR 0012).
//!
//! - A shared configuration ("Store as project file") is one file in
//!   `.oxide/run/`, meant to be committed: `<name>.toml` with a `version`.
//!   Environment variables are listed by name; a value is written only for
//!   a variable marked shared, the others' values are personal.
//! - Personal configurations (everything imported from the reference's
//!   `workspace.xml`, temporary ones), the personal values of shared
//!   configurations' variables, the selection, the custom order, the
//!   recent, pinned and history lists and the last executors are the
//!   `[run]` table of the personal project state (`.oxide/workspace.toml`,
//!   ADR 0009), never committed.
//!
//! Keys this version does not know survive a rewrite; a file of a newer
//! version is read as far as it is understood and never rewritten; writes
//! replace a file at once.
//!
//! ```toml
//! # .oxide/run/server.toml
//! version = 1
//! name = "server"
//! type = "python"
//! env = [
//!     { name = "PYTHONUNBUFFERED", value = "1" },
//!     { name = "API_TOKEN" },
//! ]
//!
//! [options]
//! script = "app/server.py"
//! ```

use std::path::{Path, PathBuf};

use toml::Value;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item};

use crate::list::RunConfigurations;
use crate::model::{BeforeLaunchTask, ConfigKey, EnvVar, Reference, RunConfiguration, Storage};

/// The folder of shared configurations in Pyoxide's project folder.
pub const SHARED_DIR: &str = "run";
/// The table of the personal project state.
pub const PERSONAL_TABLE: &str = "run";
/// Version of the configuration format this build reads and writes.
pub const VERSION: i64 = 1;
/// The key in the shared settings (`.oxide/project.toml`, table `[run]`)
/// and in the personal table that marks the import from the reference as
/// done for the shared and the personal configurations.
pub const IMPORTED_KEY: &str = "imported-from-idea";

const HEADER: &str = "# Pyoxide run configuration, shared with the project. Paths are relative to the project folder.\n";

/// Keys of a stored configuration.
const KNOWN: &[&str] = &[
    "version",
    "name",
    "type",
    "folder",
    "temporary",
    "allow-parallel",
    "include-system-env",
    "env",
    "before-launch",
    "activate-tool-window",
    "focus-tool-window",
    "edit-before-run",
    "name-is-generated",
    "options",
    "reference",
];

/// Keys of the personal table.
const KNOWN_PERSONAL: &[&str] = &[
    IMPORTED_KEY,
    "selected",
    "order",
    "recent-temporary",
    "pinned",
    "history",
    "all-expanded",
    "executors",
    "configurations",
    "shared-env",
];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Project(#[from] project::StoreError),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The shared folder of a project.
pub fn shared_dir(root: &Path) -> PathBuf {
    root.join(project::STORE_DIR).join(SHARED_DIR)
}

/// What is stored for a project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stored {
    pub list: RunConfigurations,
    /// The shared configurations of the reference were imported (by
    /// anyone: the marker is in the shared settings).
    pub shared_imported: bool,
    /// Files that could not be read, with the reason.
    pub errors: Vec<String>,
}

/// Loads the configurations of a project: the shared files, then the
/// personal table. Blocking.
pub fn load(root: &Path) -> Result<Stored, StoreError> {
    let mut out = Stored::default();
    let personal = project::workspace::load_table(root, PERSONAL_TABLE)?.unwrap_or_default();
    // Personal values of shared configurations' variables.
    let mut personal_values: Vec<(ConfigKey, Vec<(String, String)>)> = Vec::new();
    for entry in array_of_tables(&personal, "shared-env") {
        let Some(key) = key_of(&Value::Table(entry.clone())) else {
            continue;
        };
        let values = entry
            .get("values")
            .and_then(Value::as_table)
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        personal_values.push((key, values));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(shared_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml") && p.is_file())
        .collect();
    files.sort();
    let mut configs = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) => {
                out.errors.push(format!("{}: {e}", file.display()));
                continue;
            }
        };
        let table: toml::Table = match toml::from_str(&text) {
            Ok(t) => t,
            Err(e) => {
                out.errors.push(format!("{}: {e}", file.display()));
                continue;
            }
        };
        match from_table(&table, Storage::Shared) {
            Ok(mut config) => {
                config.file = Some(name);
                let values = personal_values
                    .iter()
                    .find(|(k, _)| *k == config.key())
                    .map(|(_, v)| v.as_slice())
                    .unwrap_or(&[]);
                for var in config.env.iter_mut().filter(|v| !v.shared) {
                    var.value = values
                        .iter()
                        .find(|(k, _)| *k == var.name)
                        .map(|(_, v)| v.clone());
                }
                configs.push(config);
            }
            Err(e) => out.errors.push(format!("{}: {e}", file.display())),
        }
    }
    for entry in array_of_tables(&personal, "configurations") {
        match from_table(entry, Storage::Personal) {
            Ok(config) => configs.push(config),
            Err(e) => out.errors.push(format!("personal run configuration: {e}")),
        }
    }
    let list = &mut out.list;
    for config in configs {
        // A duplicate (a shared file copied by hand) keeps the first.
        if !list.contains(&config.key()) {
            list.add(config);
        }
    }
    list.selected = personal.get("selected").and_then(key_of);
    list.order = keys(&personal, "order");
    list.recent_temporary = keys(&personal, "recent-temporary");
    list.pinned = keys(&personal, "pinned");
    list.history = keys(&personal, "history");
    list.all_expanded = personal
        .get("all-expanded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    list.executors = personal
        .get("executors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| Some((key_of(e)?, e.get("executor")?.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    list.extra = personal
        .iter()
        .filter(|(k, _)| !KNOWN_PERSONAL.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    list.personal_imported = personal
        .get(IMPORTED_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    out.shared_imported = project::store::load_value(root, PERSONAL_TABLE, IMPORTED_KEY)?
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(out)
}

/// Writes the configurations of a project: each shared configuration's
/// file (only when it changed; a new one gets a file name), and the
/// personal table (with the personal import marker). Returns the files of
/// shared configurations that were written.
pub fn save(root: &Path, list: &mut RunConfigurations) -> Result<Vec<String>, StoreError> {
    let dir = shared_dir(root);
    let mut written = Vec::new();
    let mut taken: Vec<String> = list.all().iter().filter_map(|c| c.file.clone()).collect();
    let shared: Vec<ConfigKey> = list
        .all()
        .iter()
        .filter(|c| c.is_shared() && !c.newer)
        .map(RunConfiguration::key)
        .collect();
    for key in shared {
        let config = list.get_mut(&key).expect("listed above");
        let file = match &config.file {
            Some(file) => file.clone(),
            None => {
                let file = file_name(&config.name, &taken);
                taken.push(file.clone());
                config.file = Some(file.clone());
                file
            }
        };
        let path = dir.join(&file);
        let table = to_table(config, true);
        let existing = std::fs::read_to_string(&path).ok();
        let unchanged = existing
            .as_deref()
            .and_then(|t| toml::from_str::<toml::Table>(t).ok())
            .is_some_and(|old| old == table);
        if unchanged {
            continue;
        }
        let text = shared_text(existing.as_deref(), &table);
        std::fs::create_dir_all(&dir).map_err(|source| StoreError::Io {
            path: dir.clone(),
            source,
        })?;
        write_atomic(&path, &text).map_err(|source| StoreError::Io {
            path: path.clone(),
            source,
        })?;
        written.push(file);
    }
    let table = personal_table(list);
    let value = (!table.is_empty()).then_some(table);
    project::workspace::save_table(root, PERSONAL_TABLE, value)?;
    Ok(written)
}

/// Removes a shared configuration's file.
pub fn delete_shared(root: &Path, file: &str) -> Result<(), StoreError> {
    let path = shared_dir(root).join(file);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io { path, source }),
    }
}

/// Marks the shared configurations of the reference as imported, in the
/// shared settings. The reference's exclusions are imported first (ADR
/// 0008): they are imported only while the shared settings do not exist.
pub fn mark_shared_imported(root: &Path) -> Result<(), StoreError> {
    let _ = project::store::import_idea(root)?;
    project::store::save_values(
        root,
        PERSONAL_TABLE,
        &[(IMPORTED_KEY, Some(toml::Value::Boolean(true)))],
    )?;
    Ok(())
}

/// A file name for a shared configuration: its name with the characters
/// file names cannot hold replaced, then ` (2)`, ` (3)`, … when taken
/// (case ignored, for case-insensitive file systems).
pub fn file_name(name: &str, taken: &[String]) -> String {
    let mut base: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    base = base.trim().trim_end_matches('.').to_owned();
    if base.is_empty() || base.starts_with('.') {
        base = format!("_{base}");
    }
    let free = |n: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    let first = format!("{base}.toml");
    if free(&first) {
        return first;
    }
    (2..)
        .map(|i| format!("{base} ({i}).toml"))
        .find(|n| free(n))
        .expect("a free name")
}

// ---- the stored form -----------------------------------------------------------

/// A configuration's table. In a shared file the values of personal
/// variables are left out (`shared_file`).
pub fn to_table(c: &RunConfiguration, shared_file: bool) -> toml::Table {
    let mut t = toml::Table::new();
    if shared_file {
        t.insert("version".into(), Value::Integer(VERSION));
    }
    t.insert("name".into(), Value::String(c.name.clone()));
    t.insert("type".into(), Value::String(c.type_id.clone()));
    if let Some(folder) = &c.folder {
        t.insert("folder".into(), Value::String(folder.clone()));
    }
    if c.temporary && !shared_file {
        t.insert("temporary".into(), Value::Boolean(true));
    }
    if let Some(parallel) = c.allow_parallel {
        t.insert("allow-parallel".into(), Value::Boolean(parallel));
    }
    if !c.include_system_env {
        t.insert("include-system-env".into(), Value::Boolean(false));
    }
    if !c.env.is_empty() {
        let env = c
            .env
            .iter()
            .map(|v| {
                let mut e = toml::Table::new();
                e.insert("name".into(), Value::String(v.name.clone()));
                let write_value = !shared_file || v.shared;
                if let (true, Some(value)) = (write_value, &v.value) {
                    e.insert("value".into(), Value::String(value.clone()));
                }
                if !shared_file && v.shared {
                    e.insert("shared".into(), Value::Boolean(true));
                }
                Value::Table(e)
            })
            .collect();
        t.insert("env".into(), Value::Array(env));
    }
    if !c.before_launch.is_empty() {
        let tasks = c
            .before_launch
            .iter()
            .map(|task| {
                let mut e = toml::Table::new();
                e.insert("provider".into(), Value::String(task.provider.clone()));
                if !task.enabled {
                    e.insert("enabled".into(), Value::Boolean(false));
                }
                e.insert("xml".into(), Value::String(task.xml.clone()));
                Value::Table(e)
            })
            .collect();
        t.insert("before-launch".into(), Value::Array(tasks));
    }
    if !c.activate_tool_window {
        t.insert("activate-tool-window".into(), Value::Boolean(false));
    }
    if c.focus_tool_window {
        t.insert("focus-tool-window".into(), Value::Boolean(true));
    }
    if c.edit_before_run {
        t.insert("edit-before-run".into(), Value::Boolean(true));
    }
    if c.name_is_generated {
        t.insert("name-is-generated".into(), Value::Boolean(true));
    }
    if !c.options.is_empty() {
        t.insert("options".into(), Value::Table(c.options.clone()));
    }
    if let Some(r) = &c.reference {
        let mut e = toml::Table::new();
        e.insert("type".into(), Value::String(r.type_id.clone()));
        if let Some(f) = &r.factory {
            e.insert("factory".into(), Value::String(f.clone()));
        }
        if let Some(n) = &r.type_name {
            e.insert("type-name".into(), Value::String(n.clone()));
        }
        e.insert("xml".into(), Value::String(r.xml.clone()));
        t.insert("reference".into(), Value::Table(e));
    }
    for (k, v) in &c.extra {
        t.entry(k.clone()).or_insert_with(|| v.clone());
    }
    t
}

/// A configuration from its table.
pub fn from_table(t: &toml::Table, storage: Storage) -> Result<RunConfiguration, String> {
    let string = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_owned);
    let flag = |k: &str| t.get(k).and_then(Value::as_bool);
    let name = string("name")
        .filter(|n| !n.is_empty())
        .ok_or("no `name`")?;
    let type_id = string("type")
        .filter(|n| !n.is_empty())
        .ok_or("no `type`")?;
    let mut c = RunConfiguration::new(type_id, name, storage);
    c.newer = t
        .get("version")
        .and_then(Value::as_integer)
        .is_some_and(|v| v > VERSION);
    c.folder = string("folder").filter(|f| !f.is_empty());
    c.temporary = storage == Storage::Personal && flag("temporary").unwrap_or(false);
    c.allow_parallel = flag("allow-parallel");
    c.include_system_env = flag("include-system-env").unwrap_or(true);
    c.env = t
        .get("env")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let name = e.get("name")?.as_str()?.to_owned();
                    let value = e.get("value").and_then(Value::as_str).map(str::to_owned);
                    // In a shared file a written value is a shared one.
                    let shared = match storage {
                        Storage::Shared => value.is_some(),
                        Storage::Personal => e.get("shared").and_then(Value::as_bool) == Some(true),
                    };
                    Some(EnvVar {
                        name,
                        value,
                        shared,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    c.before_launch = t
        .get("before-launch")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(BeforeLaunchTask {
                        provider: e.get("provider")?.as_str()?.to_owned(),
                        enabled: e.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                        xml: e
                            .get("xml")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    c.activate_tool_window = flag("activate-tool-window").unwrap_or(true);
    c.focus_tool_window = flag("focus-tool-window").unwrap_or(false);
    c.edit_before_run = flag("edit-before-run").unwrap_or(false);
    c.name_is_generated = flag("name-is-generated").unwrap_or(false);
    c.options = t
        .get("options")
        .and_then(Value::as_table)
        .cloned()
        .unwrap_or_default();
    c.reference = t.get("reference").and_then(Value::as_table).and_then(|r| {
        Some(Reference {
            type_id: r.get("type")?.as_str()?.to_owned(),
            factory: r.get("factory").and_then(Value::as_str).map(str::to_owned),
            type_name: r
                .get("type-name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            xml: r
                .get("xml")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        })
    });
    c.extra = t
        .iter()
        .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok(c)
}

/// The personal table: personal configurations, personal values of
/// shared ones, and the lists.
fn personal_table(list: &RunConfigurations) -> toml_edit::Table {
    let mut t = toml_edit::Table::new();
    if list.personal_imported {
        t.insert(IMPORTED_KEY, toml_edit::value(true));
    }
    let key_value = |k: &ConfigKey| -> toml_edit::Value {
        let mut e = InlineTable::new();
        e.insert("type", k.type_id.as_str().into());
        e.insert("name", k.name.as_str().into());
        toml_edit::Value::InlineTable(e)
    };
    let key_list = |keys: &[ConfigKey]| -> Item {
        let mut a = Array::new();
        for k in keys {
            a.push(key_value(k));
        }
        Item::Value(toml_edit::Value::Array(multiline(a)))
    };
    if list.all_expanded {
        t.insert("all-expanded", toml_edit::value(true));
    }
    if let Some(selected) = &list.selected {
        t.insert("selected", Item::Value(key_value(selected)));
    }
    for (name, keys) in [
        ("order", &list.order),
        ("recent-temporary", &list.recent_temporary),
        ("pinned", &list.pinned),
        ("history", &list.history),
    ] {
        if !keys.is_empty() {
            t.insert(name, key_list(keys));
        }
    }
    if !list.executors.is_empty() {
        let mut a = Array::new();
        for (k, executor) in &list.executors {
            let toml_edit::Value::InlineTable(mut e) = key_value(k) else {
                unreachable!()
            };
            e.insert("executor", executor.as_str().into());
            a.push(toml_edit::Value::InlineTable(e));
        }
        t.insert(
            "executors",
            Item::Value(toml_edit::Value::Array(multiline(a))),
        );
    }
    let mut shared_env = ArrayOfTables::new();
    let mut configurations = ArrayOfTables::new();
    for c in list.all() {
        if c.is_shared() {
            let values: Vec<(&str, &str)> = c
                .env
                .iter()
                .filter(|v| !v.shared)
                .filter_map(|v| Some((v.name.as_str(), v.value.as_deref()?)))
                .collect();
            if !values.is_empty() {
                let mut e = toml_edit::Table::new();
                e.insert("type", toml_edit::value(c.type_id.as_str()));
                e.insert("name", toml_edit::value(c.name.as_str()));
                let mut v = InlineTable::new();
                for (k, value) in values {
                    v.insert(k, value.into());
                }
                e.insert("values", Item::Value(toml_edit::Value::InlineTable(v)));
                shared_env.push(e);
            }
        } else {
            configurations.push(edit_table(&to_table(c, false)));
        }
    }
    if !configurations.is_empty() {
        t.insert("configurations", Item::ArrayOfTables(configurations));
    }
    if !shared_env.is_empty() {
        t.insert("shared-env", Item::ArrayOfTables(shared_env));
    }
    for (k, v) in &list.extra {
        if t.get(k).is_none() {
            t.insert(k, to_item(v));
        }
    }
    t
}

/// The text of a shared file: the existing document with the new values
/// (unchanged keys keep their formatting and comments), or a new one.
fn shared_text(existing: Option<&str>, table: &toml::Table) -> String {
    let new = edit_table(table);
    let mut doc = match existing.and_then(|t| t.parse::<DocumentMut>().ok()) {
        Some(doc) => doc,
        None => {
            let mut doc = DocumentMut::new();
            for (k, item) in new.iter() {
                doc.insert(k, item.clone());
            }
            return format!("{HEADER}{doc}");
        }
    };
    let old_keys: Vec<String> = doc.iter().map(|(k, _)| k.to_owned()).collect();
    for k in old_keys {
        if !table.contains_key(&k) {
            doc.remove(&k);
        }
    }
    for (k, item) in new.iter() {
        let same = doc
            .get(k)
            .is_some_and(|old| item_value(old) == table.get(k).cloned());
        if !same {
            doc.insert(k, item.clone());
        }
    }
    doc.to_string()
}

/// A `toml_edit` item's value.
fn item_value(item: &Item) -> Option<Value> {
    let mut doc = DocumentMut::new();
    doc.insert("v", item.clone());
    toml::from_str::<toml::Table>(&doc.to_string())
        .ok()?
        .remove("v")
}

/// A table for `toml_edit`: sub-tables as sections, arrays of tables
/// (`env`, `before-launch`) as inline tables one per line.
fn edit_table(t: &toml::Table) -> toml_edit::Table {
    let mut out = toml_edit::Table::new();
    // The known keys in their order, then the others.
    let mut entries: Vec<(&String, &Value)> = t.iter().collect();
    entries.sort_by_key(|(k, _)| KNOWN.iter().position(|n| n == k).unwrap_or(KNOWN.len()));
    for (k, v) in entries {
        let item = match v {
            Value::Table(sub) => Item::Table(edit_table(sub)),
            v => to_item(v),
        };
        out.insert(k, item);
    }
    out
}

fn to_item(v: &Value) -> Item {
    Item::Value(to_value(v))
}

fn to_value(v: &Value) -> toml_edit::Value {
    match v {
        Value::String(s) => s.as_str().into(),
        Value::Integer(i) => (*i).into(),
        Value::Float(f) => (*f).into(),
        Value::Boolean(b) => (*b).into(),
        Value::Datetime(d) => d
            .to_string()
            .parse::<toml_edit::Datetime>()
            .map(toml_edit::Value::from)
            .unwrap_or_else(|_| d.to_string().into()),
        Value::Array(a) => {
            let mut out = Array::new();
            for v in a {
                out.push(to_value(v));
            }
            if a.iter().any(Value::is_table) {
                out = multiline(out);
            }
            toml_edit::Value::Array(out)
        }
        Value::Table(t) => {
            let mut out = InlineTable::new();
            for (k, v) in t {
                out.insert(k, to_value(v));
            }
            toml_edit::Value::InlineTable(out)
        }
    }
}

/// An array with one element per line.
fn multiline(mut a: Array) -> Array {
    if a.is_empty() {
        return a;
    }
    for v in a.iter_mut() {
        v.decor_mut().set_prefix("\n    ");
        v.decor_mut().set_suffix("");
    }
    a.set_trailing("\n");
    a.set_trailing_comma(true);
    a
}

fn array_of_tables<'a>(t: &'a toml::Table, key: &str) -> Vec<&'a toml::Table> {
    t.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_table).collect())
        .unwrap_or_default()
}

fn key_of(v: &Value) -> Option<ConfigKey> {
    Some(ConfigKey::new(
        v.get("type")?.as_str()?,
        v.get("name")?.as_str()?,
    ))
}

fn keys(t: &toml::Table, name: &str) -> Vec<ConfigKey> {
    t.get(name)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(key_of).collect())
        .unwrap_or_default()
}

/// Replaces `path` with `text` at once (a temporary file, then a rename).
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().expect("a file has a folder");
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

    fn shared_config() -> RunConfiguration {
        let mut c = RunConfiguration::new("python", "server", Storage::Shared);
        c.folder = Some("Servers".into());
        c.allow_parallel = Some(true);
        c.env = vec![
            EnvVar::new("PYTHONUNBUFFERED", "1", true),
            EnvVar::new("API_TOKEN", "personal-secret-1", false),
        ];
        c.set_str("script", "app/server.py");
        c.before_launch = vec![BeforeLaunchTask {
            provider: "Make".into(),
            enabled: false,
            xml: "<option name=\"Make\" enabled=\"false\" />".into(),
        }];
        c
    }

    #[test]
    fn shared_and_personal_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut list = RunConfigurations::default();
        let shared = list.add(shared_config());
        let mut personal = RunConfiguration::new("python", "api", Storage::Personal);
        personal.env = vec![EnvVar::new("DB_PASSWORD", "personal-secret-2", false)];
        personal.temporary = true;
        personal.include_system_env = false;
        let personal = list.add(personal);
        list.selected = Some(shared.clone());
        list.order = vec![personal.clone(), shared.clone()];
        list.pinned = vec![shared.clone()];
        list.executors = vec![(shared.clone(), "Run".into())];
        list.personal_imported = true;
        let written = save(root, &mut list).unwrap();
        assert_eq!(written, ["server.toml"]);
        let text = std::fs::read_to_string(shared_dir(root).join("server.toml")).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        assert!(text.contains("{ name = \"API_TOKEN\" }"), "{text}");
        assert!(
            text.contains("{ name = \"PYTHONUNBUFFERED\", value = \"1\" }"),
            "{text}"
        );
        assert!(
            text.contains("[options]\nscript = \"app/server.py\""),
            "{text}"
        );
        // Personal values are only in the personal file.
        for secret in ["personal-secret-1", "personal-secret-2"] {
            assert!(!text.contains(secret));
        }
        let personal_text =
            std::fs::read_to_string(project::workspace::workspace_file(root)).unwrap();
        assert!(
            personal_text.contains("personal-secret-1"),
            "{personal_text}"
        );
        assert!(
            personal_text.contains("[[run.configurations]]"),
            "{personal_text}"
        );

        let stored = load(root).unwrap();
        assert!(stored.errors.is_empty(), "{:?}", stored.errors);
        assert!(stored.list.personal_imported);
        assert!(!stored.shared_imported);
        let mut expected = list.clone();
        expected.get_mut(&shared).unwrap().file = Some("server.toml".into());
        assert_eq!(stored.list.get(&shared), expected.get(&shared));
        assert_eq!(stored.list.get(&personal), expected.get(&personal));
        assert_eq!(stored.list.selected, Some(shared.clone()));
        assert_eq!(stored.list.order, [personal.clone(), shared.clone()]);
        assert_eq!(stored.list.pinned, std::slice::from_ref(&shared));
        assert_eq!(stored.list.executors, [(shared.clone(), "Run".to_owned())]);

        // Unchanged: nothing is written again.
        let mut again = stored.list.clone();
        assert!(save(root, &mut again).unwrap().is_empty());
    }

    #[test]
    fn a_missing_personal_value_is_not_set_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(shared_dir(root)).unwrap();
        std::fs::write(
            shared_dir(root).join("x.toml"),
            "version = 1\nname = \"x\"\ntype = \"python\"\nenv = [{ name = \"TOKEN\" }, { name = \"A\", value = \"1\" }]\n",
        )
        .unwrap();
        let stored = load(root).unwrap();
        let c = stored.list.get(&ConfigKey::new("python", "x")).unwrap();
        assert_eq!(c.env[0].value, None);
        assert!(!c.env[0].shared);
        assert!(c.env[1].shared);
        assert_eq!(c.env_values().collect::<Vec<_>>(), [("A", "1")]);
    }

    #[test]
    fn unknown_keys_and_comments_survive_and_newer_files_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(shared_dir(root)).unwrap();
        let original = "# team notes\nversion = 1\nname = \"x\" # the name\ntype = \"python\"\nfuture = \"keep\"\n\n[options]\nscript = \"a.py\"\nfuture-option = 3\n";
        std::fs::write(shared_dir(root).join("x.toml"), original).unwrap();
        let newer = "version = 2\nname = \"y\"\ntype = \"python\"\n";
        std::fs::write(shared_dir(root).join("y.toml"), newer).unwrap();
        std::fs::write(shared_dir(root).join("broken.toml"), "name = ").unwrap();
        let mut stored = load(root).unwrap();
        assert_eq!(stored.errors.len(), 1);
        let key = ConfigKey::new("python", "x");
        stored.list.get_mut(&key).unwrap().set_str("script", "b.py");
        stored
            .list
            .get_mut(&ConfigKey::new("python", "y"))
            .unwrap()
            .set_str("script", "z.py");
        save(root, &mut stored.list).unwrap();
        let text = std::fs::read_to_string(shared_dir(root).join("x.toml")).unwrap();
        for kept in [
            "# team notes",
            "name = \"x\" # the name",
            "future = \"keep\"",
            "future-option = 3",
            "script = \"b.py\"",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        assert_eq!(
            std::fs::read_to_string(shared_dir(root).join("y.toml")).unwrap(),
            newer
        );
        assert!(
            stored
                .list
                .get(&ConfigKey::new("python", "y"))
                .unwrap()
                .newer
        );
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name("server", &[]), "server.toml");
        assert_eq!(file_name("a/b: c?", &[]), "a_b_ c_.toml");
        assert_eq!(
            file_name("server", &["Server.toml".into()]),
            "server (2).toml"
        );
        assert_eq!(file_name(".hidden", &[]), "_.hidden.toml");
        assert_eq!(file_name("...", &[]), "_.toml");
    }

    #[test]
    fn the_shared_marker_keeps_the_exclusions_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".idea")).unwrap();
        std::fs::write(
            root.join(".idea/demo.iml"),
            r#"<module><component name="NewModuleRootManager"><content url="file://$MODULE_DIR$"><excludeFolder url="file://$MODULE_DIR$/build" /></content></component></module>"#,
        )
        .unwrap();
        mark_shared_imported(root).unwrap();
        assert!(
            project::store::load(root)
                .unwrap()
                .unwrap()
                .paths
                .contains("build")
        );
        assert!(load(root).unwrap().shared_imported);
    }
}
