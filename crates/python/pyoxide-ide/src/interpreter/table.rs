//! The application's list of configured Python interpreters (the reference
//! IDE's SDK table, `jdk.table.xml`, with the choices `PySdkSettings`
//! remembers), kept in `interpreters.toml` of the configuration folder
//! (ADR 0017):
//!
//! ```toml
//! version = 1
//!
//! [preferred]
//! venv-base-path = "/home/u/.venvs"
//! base-python = "/usr/bin/python3"
//!
//! [import]
//! source = "PyCharm2026.2"
//!
//! [[interpreter]]
//! name = "~/proj/.venv"
//! python = "/home/u/proj/.venv/bin/python"
//! kind = "uv"
//! version = "3.13.14"
//! associated-project = "/home/u/proj"
//! uv = "/home/u/.local/bin/uv"
//! working-dir = "/home/u/proj"
//! ```
//!
//! Names are unique. A write reads the file again and applies only its
//! changes to it, so two windows keep each other's; keys and comments
//! Pyoxide does not know survive, and a file of a newer version is read
//! and never rewritten. The reference IDE's interpreters are imported once,
//! when the file does not exist yet.
//!
//! Everything here is blocking file system access: run it off the UI
//! thread.

use std::path::{Path, PathBuf};

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table as TomlTable, value};

/// The file in the configuration folder.
pub const FILE: &str = "interpreters.toml";
/// The format version written.
const VERSION: i64 = 1;
const VERSION_KEY: &str = "version";
const INTERPRETER: &str = "interpreter";
const PREFERRED: &str = "preferred";
const IMPORT: &str = "import";

/// The reference's messages of the Rename dialog
/// (`rename.python.interpreter.dialog.provide.name.error.text`,
/// `rename.python.interpreter.name.already.exists.error.text`).
pub const NAME_EMPTY: &str = "Provide a name for a Python interpreter";
pub const NAME_TAKEN: &str = "Python interpreter with the same name already exists";

/// How an interpreter is set up (the reference's `PythonSdkFlavor`): it
/// decides its icon and its group in the lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flavor {
    /// A virtual environment (venv, virtualenv).
    Venv,
    /// A virtual environment managed by uv.
    Uv,
    Conda,
    Poetry,
    Pyenv,
    /// An interpreter installation.
    System,
}

impl Flavor {
    pub const ALL: [Flavor; 6] = [
        Flavor::Venv,
        Flavor::Uv,
        Flavor::Conda,
        Flavor::Poetry,
        Flavor::Pyenv,
        Flavor::System,
    ];

    /// The `kind` of the file.
    pub fn id(self) -> &'static str {
        match self {
            Flavor::Venv => "venv",
            Flavor::Uv => "uv",
            Flavor::Conda => "conda",
            Flavor::Poetry => "poetry",
            Flavor::Pyenv => "pyenv",
            Flavor::System => "system",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    /// Listed with the virtual environments, before the other
    /// interpreters (`isNonToolVirtualEnv || isCondaVirtualEnv`).
    pub fn is_virtual_env(self) -> bool {
        matches!(self, Flavor::Venv | Flavor::Conda)
    }

    /// The flavour of an interpreter by the files around it: a virtual
    /// environment (uv's when its `pyvenv.cfg` says so), a conda
    /// environment, else an installation.
    pub fn of_layout(python: &Path) -> Flavor {
        use py_env::venv::EnvLayout;
        match py_env::venv::inspect_python(python) {
            Some(env) if env.is_uv() => Flavor::Uv,
            Some(env) => match env.layout {
                EnvLayout::Venv => Flavor::Venv,
                EnvLayout::Conda => Flavor::Conda,
                EnvLayout::Installation => Flavor::System,
            },
            None => Flavor::System,
        }
    }

    /// The reference's flavour id (`FLAVOR_ID`: `UvSdkFlavor`,
    /// `VirtualEnvSdkFlavor`, `MacPythonSdkFlavor`, …).
    fn of_reference(id: &str) -> Option<Flavor> {
        let id = id.trim();
        Some(if id.starts_with("Uv") {
            Flavor::Uv
        } else if id.starts_with("Conda") {
            Flavor::Conda
        } else if id.starts_with("Poetry") {
            Flavor::Poetry
        } else if id.starts_with("Pyenv") {
            Flavor::Pyenv
        } else if id.starts_with("VirtualEnv") {
            Flavor::Venv
        } else if id.ends_with("PythonSdkFlavor") {
            Flavor::System
        } else {
            return None;
        })
    }
}

/// A configured interpreter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Unique in the list; the reference names an environment by its path
    /// (`~/proj/.venv`).
    pub name: String,
    /// The executable ("Interpreter path").
    pub python: PathBuf,
    pub flavor: Flavor,
    /// `3.13.14`, kept for display; `None` until it is known.
    pub version: Option<String>,
    /// The project a virtual environment was made for: it is offered to
    /// that project only (`ASSOCIATED_PROJECT_PATH`).
    pub associated_project: Option<PathBuf>,
    /// The uv of a uv environment (`UV_TOOL_PATH`).
    pub uv: Option<PathBuf>,
    /// Where uv runs for a uv environment (`UV_WORKING_DIR`).
    pub working_dir: Option<PathBuf>,
}

impl Entry {
    pub fn new(name: impl Into<String>, python: impl Into<PathBuf>, flavor: Flavor) -> Self {
        Self {
            name: name.into(),
            python: python.into(),
            flavor,
            version: None,
            associated_project: None,
            uv: None,
            working_dir: None,
        }
    }

    /// Offered for the project at `root`: not associated with another
    /// project (`isAssociatedWithAnotherModule`); without a project every
    /// interpreter is.
    pub fn offered_for(&self, root: Option<&Path>) -> bool {
        match (&self.associated_project, root) {
            (Some(project), Some(root)) => same_path(project, root),
            _ => true,
        }
    }

    /// Associated with a project other than `root`.
    pub fn associated_elsewhere(&self, root: Option<&Path>) -> bool {
        !self.offered_for(root)
    }
}

/// What the reference keeps in `PySdkSettings` for new environments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferred {
    /// The parent folder of the last environment created.
    pub venv_base_path: Option<PathBuf>,
    /// The base interpreter of the last environment created.
    pub base_python: Option<PathBuf>,
}

/// The list as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub entries: Vec<Entry>,
    pub preferred: Preferred,
    /// The reference IDE's configuration folder the list was imported from
    /// (`PyCharm2026.2`).
    pub imported: Option<String>,
    /// The file is of a newer version: read, never written.
    pub newer: bool,
}

/// A change of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Adds an interpreter, its name made unique; an interpreter already
    /// listed (by path) is updated instead and keeps its name.
    Add(Entry),
    Remove(String),
    Rename {
        name: String,
        new_name: String,
    },
    SetPython {
        name: String,
        python: PathBuf,
    },
    SetVersion {
        name: String,
        version: Option<String>,
    },
    SetAssociation {
        name: String,
        project: Option<PathBuf>,
    },
    SetPreferred(Preferred),
    /// The list was imported from the reference's configuration folder.
    Imported(String),
}

impl Table {
    /// Reads the list; a missing file is an empty list.
    pub fn load(path: &Path) -> Result<Table, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Table::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Table::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Table, String> {
        let doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| e.to_string())?;
        Ok(Table::of(&doc))
    }

    /// The entry named `name`.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The entry of an interpreter.
    pub fn find_python(&self, python: &Path) -> Option<&Entry> {
        self.entries.iter().find(|e| same_path(&e.python, python))
    }

    /// The entries offered for the project at `root`.
    pub fn offered_for<'a>(&'a self, root: Option<&'a Path>) -> impl Iterator<Item = &'a Entry> {
        self.entries.iter().filter(move |e| e.offered_for(root))
    }

    /// `name`, or `name (2)`, … when another entry has it.
    pub fn unique_name(&self, name: &str) -> String {
        unique_name(name, |n| self.find(n).is_some())
    }

    /// Applies `change` to the list in memory (the file is written with
    /// [`save`]).
    pub fn apply(&mut self, change: &Change) -> Result<(), String> {
        if self.newer {
            return Err(NEWER.to_owned());
        }
        let mut doc = self.document();
        apply(&mut doc, change)?;
        *self = Table::of(&doc);
        Ok(())
    }

    fn of(doc: &DocumentMut) -> Table {
        let newer = doc
            .get(VERSION_KEY)
            .and_then(Item::as_integer)
            .is_some_and(|v| v > VERSION);
        let path_of = |t: &TomlTable, key: &str| {
            t.get(key)
                .and_then(Item::as_str)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        let entries = doc
            .get(INTERPRETER)
            .and_then(Item::as_array_of_tables)
            .map(|list| list.iter().filter_map(entry_of).collect())
            .unwrap_or_default();
        let preferred = doc
            .get(PREFERRED)
            .and_then(Item::as_table)
            .map(|t| Preferred {
                venv_base_path: path_of(t, "venv-base-path"),
                base_python: path_of(t, "base-python"),
            })
            .unwrap_or_default();
        let imported = doc
            .get(IMPORT)
            .and_then(Item::as_table)
            .and_then(|t| t.get("source"))
            .and_then(Item::as_str)
            .map(str::to_owned);
        Table {
            entries,
            preferred,
            imported,
            newer,
        }
    }

    /// The list as a fresh document.
    fn document(&self) -> DocumentMut {
        let mut doc = DocumentMut::new();
        doc[VERSION_KEY] = value(VERSION);
        if self.preferred != Preferred::default() {
            let _ = apply(&mut doc, &Change::SetPreferred(self.preferred.clone()));
        }
        if let Some(source) = &self.imported {
            let _ = apply(&mut doc, &Change::Imported(source.clone()));
        }
        let mut list = ArrayOfTables::new();
        for entry in &self.entries {
            let mut table = TomlTable::new();
            write_entry(&mut table, entry);
            list.push(table);
        }
        if !list.is_empty() {
            doc[INTERPRETER] = Item::ArrayOfTables(list);
        }
        doc
    }
}

const NEWER: &str =
    "the list of interpreters was written by a newer version of Pyoxide and is not changed";

fn entry_of(t: &TomlTable) -> Option<Entry> {
    let text = |key: &str| {
        t.get(key)
            .and_then(Item::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let name = text("name")?;
    let python = PathBuf::from(text("python")?);
    let flavor = text("kind")
        .and_then(|k| Flavor::parse(&k))
        .unwrap_or(Flavor::System);
    Some(Entry {
        name,
        python,
        flavor,
        version: text("version"),
        associated_project: text("associated-project").map(PathBuf::from),
        uv: text("uv").map(PathBuf::from),
        working_dir: text("working-dir").map(PathBuf::from),
    })
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Sets `key`, or removes it for `None`.
fn set(t: &mut TomlTable, key: &str, text: Option<String>) {
    match text {
        Some(text) => t[key] = value(text),
        None => {
            t.remove(key);
        }
    }
}

fn write_entry(t: &mut TomlTable, e: &Entry) {
    t["name"] = value(e.name.clone());
    t["python"] = value(path_text(&e.python));
    t["kind"] = value(e.flavor.id());
    set(t, "version", e.version.clone());
    set(
        t,
        "associated-project",
        e.associated_project.as_deref().map(path_text),
    );
    set(t, "uv", e.uv.as_deref().map(path_text));
    set(t, "working-dir", e.working_dir.as_deref().map(path_text));
}

/// The `[[interpreter]]` tables of a document, created when missing.
fn list_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, String> {
    doc.entry(INTERPRETER)
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| format!("`{INTERPRETER}` is not a list of tables"))
}

fn names(doc: &DocumentMut) -> Vec<String> {
    Table::of(doc).entries.into_iter().map(|e| e.name).collect()
}

/// The index of the table named `name`.
fn index_of(doc: &mut DocumentMut, name: &str) -> Result<usize, String> {
    let list = list_mut(doc)?;
    list.iter()
        .position(|t| t.get("name").and_then(Item::as_str) == Some(name))
        .ok_or_else(|| format!("no interpreter named `{name}`"))
}

/// Applies a change to a document of the file.
fn apply(doc: &mut DocumentMut, change: &Change) -> Result<(), String> {
    match change {
        Change::Add(entry) => {
            let existing = Table::of(doc)
                .find_python(&entry.python)
                .map(|e| e.name.clone());
            if let Some(name) = existing {
                let at = index_of(doc, &name)?;
                let table = list_mut(doc)?.get_mut(at).expect("the index found");
                write_entry(
                    table,
                    &Entry {
                        name,
                        ..entry.clone()
                    },
                );
                return Ok(());
            }
            let taken = names(doc);
            let name = unique_name(entry.name.trim(), |n| taken.iter().any(|t| t == n));
            let mut table = TomlTable::new();
            write_entry(
                &mut table,
                &Entry {
                    name,
                    ..entry.clone()
                },
            );
            list_mut(doc)?.push(table);
        }
        Change::Remove(name) => {
            if let Ok(at) = index_of(doc, name) {
                list_mut(doc)?.remove(at);
            }
        }
        Change::Rename { name, new_name } => {
            let new_name = new_name.trim();
            if new_name.is_empty() {
                return Err(NAME_EMPTY.to_owned());
            }
            if new_name != name && names(doc).iter().any(|n| n == new_name) {
                return Err(NAME_TAKEN.to_owned());
            }
            let at = index_of(doc, name)?;
            list_mut(doc)?.get_mut(at).expect("the index found")["name"] = value(new_name);
        }
        Change::SetPython { name, python } => {
            let at = index_of(doc, name)?;
            list_mut(doc)?.get_mut(at).expect("the index found")["python"] =
                value(path_text(python));
        }
        Change::SetVersion { name, version } => {
            let at = index_of(doc, name)?;
            let table = list_mut(doc)?.get_mut(at).expect("the index found");
            set(table, "version", version.clone());
        }
        Change::SetAssociation { name, project } => {
            let at = index_of(doc, name)?;
            let table = list_mut(doc)?.get_mut(at).expect("the index found");
            set(
                table,
                "associated-project",
                project.as_deref().map(path_text),
            );
        }
        Change::SetPreferred(preferred) => {
            let table = doc
                .entry(PREFERRED)
                .or_insert(Item::Table(TomlTable::new()))
                .as_table_mut()
                .ok_or_else(|| format!("`{PREFERRED}` is not a table"))?;
            set(
                table,
                "venv-base-path",
                preferred.venv_base_path.as_deref().map(path_text),
            );
            set(
                table,
                "base-python",
                preferred.base_python.as_deref().map(path_text),
            );
        }
        Change::Imported(source) => {
            let table = doc
                .entry(IMPORT)
                .or_insert(Item::Table(TomlTable::new()))
                .as_table_mut()
                .ok_or_else(|| format!("`{IMPORT}` is not a table"))?;
            table["source"] = value(source.clone());
        }
    }
    Ok(())
}

/// Writes `changes` into the file at `path`: it is read again, the changes
/// are applied to it and it is replaced at once (a temporary file renamed
/// over it). A file of a newer version or one that cannot be read is left
/// alone. Returns the list as written.
pub fn save(path: &Path, changes: &[Change]) -> Result<Table, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("{}: {e}", path.display()))?;
    if Table::of(&doc).newer {
        return Err(NEWER.to_owned());
    }
    if !doc.contains_key(VERSION_KEY) {
        let mut fresh = DocumentMut::new();
        fresh[VERSION_KEY] = value(VERSION);
        for (key, item) in doc.iter() {
            fresh.insert(key, item.clone());
        }
        doc = fresh;
    }
    for change in changes {
        apply(&mut doc, change)?;
    }
    settings::write_atomic(path, &doc.to_string())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Table::of(&doc))
}

/// `name`, or `name (2)`, `name (3)`, … when `taken`
/// (`SdkConfigurationUtil.createUniqueSdkName`).
pub fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    (2..)
        .map(|i| format!("{name} ({i})"))
        .find(|n| !taken(n))
        .expect("a free name")
}

/// Two paths name the same file or folder: equal, or equal once their
/// parent folders are resolved. The last part is not followed: the
/// interpreter of a virtual environment is a link to its base
/// interpreter, and two environments made from one base are not the same
/// interpreter.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.parent(), b.parent(), a.file_name(), b.file_name()) {
        (Some(pa), Some(pb), Some(na), Some(nb)) if na == nb => native_path::canonicalize(pa)
            .ok()
            .is_some_and(|pa| native_path::canonicalize(pb).is_ok_and(|pb| pa == pb)),
        _ => false,
    }
}

// ---- the reference IDE's interpreters ---------------------------------------------

/// The SDK type of Python interpreters in the reference IDE.
const PYTHON_SDK: &str = "Python SDK";

/// The Python interpreters of the reference IDE's SDK table: from
/// `options/jdk.table.xml` of the first (newest) folder of `config_dirs`
/// that has one, those whose interpreter exists, with the folder's name.
pub fn reference_entries(config_dirs: &[PathBuf]) -> Option<(String, Vec<Entry>)> {
    let (dir, table) = config_dirs.iter().find_map(|dir| {
        let file = dir.join("options").join("jdk.table.xml");
        file.is_file()
            .then(|| (dir, project::idea::read_sdk_table(&file)))
    })?;
    let source = dir.file_name()?.to_string_lossy().into_owned();
    let path = |key: &str, sdk: &project::idea::SdkEntry| {
        sdk.additional(key)
            .filter(|v| !v.is_empty())
            .and_then(project::idea::expand_user_home)
    };
    let entries = table
        .iter()
        .filter(|sdk| sdk.type_id == PYTHON_SDK)
        .filter_map(|sdk| {
            let python = sdk.home.clone().filter(|h| h.is_file())?;
            let flavor = if sdk.additional("IS_UV") == Some("true") {
                Flavor::Uv
            } else {
                sdk.setting("FLAVOR_ID")
                    .and_then(Flavor::of_reference)
                    .unwrap_or_else(|| Flavor::of_layout(&python))
            };
            Some(Entry {
                name: sdk.name.clone(),
                flavor,
                version: sdk.version.as_deref().and_then(super::version_in),
                associated_project: path("ASSOCIATED_PROJECT_PATH", sdk),
                uv: path("UV_TOOL_PATH", sdk),
                working_dir: path("UV_WORKING_DIR", sdk).or_else(|| path("WORKING_DIRECTORY", sdk)),
                python,
            })
        })
        .collect();
    Some((source, entries))
}

/// Adds the reference IDE's interpreters ([`reference_entries`]) to the
/// list at `path` and records where they came from; interpreters already
/// listed (by path) are skipped. Returns how many were added.
pub fn import_reference(path: &Path, config_dirs: &[PathBuf]) -> Result<usize, String> {
    let Some((source, entries)) = reference_entries(config_dirs) else {
        return Ok(0);
    };
    let before = Table::load(path)?;
    let added: Vec<Change> = entries
        .into_iter()
        .filter(|e| before.find_python(&e.python).is_none())
        .map(Change::Add)
        .collect();
    let count = added.len();
    let mut changes = added;
    changes.push(Change::Imported(source));
    save(path, &changes)?;
    Ok(count)
}

/// The list at `path`; when the file does not exist yet, the reference
/// IDE's interpreters are imported into it first (once: the file then
/// exists).
pub fn load_or_import(path: &Path, config_dirs: &[PathBuf]) -> Result<Table, String> {
    if !path.exists() {
        import_reference(path, config_dirs)?;
    }
    Table::load(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn venv(dir: &Path, cfg: &str) -> PathBuf {
        let python = py_env::paths::venv_python(dir);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        std::fs::write(dir.join("pyvenv.cfg"), cfg).unwrap();
        python
    }

    #[test]
    fn round_trip_keeps_what_pyoxide_does_not_know() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let text = "version = 1\n# mine\nfuture = \"x\"\n\n[[interpreter]]\n# the work one\nname = \"~/proj/.venv\"\npython = \"/home/u/proj/.venv/bin/python\"\nkind = \"uv\"\nversion = \"3.13.14\"\nassociated-project = \"/home/u/proj\"\nuv = \"/home/u/.local/bin/uv\"\nworking-dir = \"/home/u/proj\"\ncolour = \"red\"\n";
        std::fs::write(&file, text).unwrap();
        let table = Table::load(&file).unwrap();
        assert_eq!(table.entries.len(), 1);
        let e = &table.entries[0];
        assert_eq!(e.name, "~/proj/.venv");
        assert_eq!(e.flavor, Flavor::Uv);
        assert_eq!(e.version.as_deref(), Some("3.13.14"));
        assert_eq!(e.uv.as_deref(), Some(Path::new("/home/u/.local/bin/uv")));
        assert!(e.offered_for(Some(Path::new("/home/u/proj"))));
        assert!(!e.offered_for(Some(Path::new("/home/u/other"))));
        assert!(e.offered_for(None));

        let written = save(
            &file,
            &[
                Change::SetVersion {
                    name: "~/proj/.venv".into(),
                    version: Some("3.13.15".into()),
                },
                Change::Add(Entry::new(
                    "Python 3.12",
                    "/usr/bin/python3.12",
                    Flavor::System,
                )),
            ],
        )
        .unwrap();
        let after = std::fs::read_to_string(&file).unwrap();
        for kept in [
            "# mine",
            "future = \"x\"",
            "# the work one",
            "colour = \"red\"",
        ] {
            assert!(after.contains(kept), "{kept} lost:\n{after}");
        }
        assert!(after.contains("version = \"3.13.15\""), "{after}");
        assert_eq!(written.entries.len(), 2);
        assert_eq!(Table::load(&file).unwrap(), written);
    }

    #[test]
    fn names_stay_unique() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let a = Entry::new("Python 3.13", "/a/python3", Flavor::System);
        let b = Entry::new("Python 3.13", "/b/python3", Flavor::System);
        let table = save(&file, &[Change::Add(a.clone()), Change::Add(b)]).unwrap();
        let names: Vec<_> = table.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Python 3.13", "Python 3.13 (2)"]);
        // The same interpreter again is updated, not added.
        let again = Entry {
            version: Some("3.13.1".into()),
            ..a
        };
        let table = save(&file, &[Change::Add(again)]).unwrap();
        assert_eq!(table.entries.len(), 2);
        assert_eq!(table.entries[0].version.as_deref(), Some("3.13.1"));

        // Rename: refused for a blank or taken name, the list unchanged.
        let rename = |new_name: &str| Change::Rename {
            name: "Python 3.13".into(),
            new_name: new_name.into(),
        };
        assert_eq!(save(&file, &[rename("  ")]), Err(NAME_EMPTY.to_owned()));
        assert_eq!(
            save(&file, &[rename("Python 3.13 (2)")]),
            Err(NAME_TAKEN.to_owned())
        );
        let mut memory = Table::load(&file).unwrap();
        assert_eq!(
            memory.apply(&rename("Python 3.13 (2)")),
            Err(NAME_TAKEN.into())
        );
        memory.apply(&rename("work")).unwrap();
        assert_eq!(memory.entries[0].name, "work");
        // Unchanged on disk until saved.
        assert_eq!(Table::load(&file).unwrap().entries[0].name, "Python 3.13");
        let table = save(
            &file,
            &[rename("work"), Change::Remove("Python 3.13 (2)".into())],
        )
        .unwrap();
        let names: Vec<_> = table.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["work"]);
        // Removing drops the entry only; removing again is no error.
        save(&file, &[Change::Remove("Python 3.13 (2)".into())]).unwrap();
    }

    #[test]
    fn association_decides_where_an_interpreter_is_offered() {
        let mut table = Table::default();
        let mut a = Entry::new("~/a/.venv", "/w/a/.venv/bin/python", Flavor::Venv);
        a.associated_project = Some("/w/a".into());
        let mut b = Entry::new("~/b/.venv", "/w/b/.venv/bin/python", Flavor::Uv);
        b.associated_project = Some("/w/b".into());
        let shared = Entry::new("Python 3.14", "/usr/bin/python3.14", Flavor::System);
        for e in [a, b, shared] {
            table.apply(&Change::Add(e)).unwrap();
        }
        let offered = |table: &Table, root: &str| -> Vec<String> {
            table
                .offered_for(Some(Path::new(root)))
                .map(|e| e.name.clone())
                .collect()
        };
        assert_eq!(offered(&table, "/w/a"), ["~/a/.venv", "Python 3.14"]);
        assert_eq!(offered(&table, "/w/b"), ["~/b/.venv", "Python 3.14"]);
        assert_eq!(offered(&table, "/w/c"), ["Python 3.14"]);
        table
            .apply(&Change::SetAssociation {
                name: "~/a/.venv".into(),
                project: None,
            })
            .unwrap();
        assert_eq!(offered(&table, "/w/c"), ["~/a/.venv", "Python 3.14"]);
        assert!(Flavor::Venv.is_virtual_env() && !Flavor::Uv.is_virtual_env());
    }

    #[test]
    fn a_newer_file_is_read_and_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let text = "version = 2\n[[interpreter]]\nname = \"x\"\npython = \"/x/python\"\nkind = \"hatch\"\n";
        std::fs::write(&file, text).unwrap();
        let mut table = Table::load(&file).unwrap();
        assert!(table.newer);
        // An unknown kind reads as an installation.
        assert_eq!(table.entries[0].flavor, Flavor::System);
        assert!(save(&file, &[Change::Remove("x".into())]).is_err());
        assert!(table.apply(&Change::Remove("x".into())).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        // A file that cannot be read is not written over.
        std::fs::write(&file, "version = = 1\n").unwrap();
        assert!(Table::load(&file).is_err());
        assert!(save(&file, &[Change::Imported("x".into())]).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "version = = 1\n");
    }

    #[test]
    fn a_new_file_gets_the_version_first() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join(FILE);
        save(
            &file,
            &[Change::SetPreferred(Preferred {
                venv_base_path: Some("/home/u/.venvs".into()),
                base_python: None,
            })],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "version = 1\n\n[preferred]\nvenv-base-path = \"/home/u/.venvs\"\n"
        );
        assert_eq!(
            Table::load(&file).unwrap().preferred.venv_base_path,
            Some(PathBuf::from("/home/u/.venvs"))
        );
    }

    /// A configuration folder of the reference IDE with an SDK table.
    fn reference_config(dir: &Path, sdks: &str) {
        std::fs::create_dir_all(dir.join("options")).unwrap();
        std::fs::write(
            dir.join("options/jdk.table.xml"),
            format!("<application>\n  <component name=\"ProjectJdkTable\">\n{sdks}  </component>\n</application>\n"),
        )
        .unwrap();
    }

    fn sdk(name: &str, home: &str, version: &str, additional: &str) -> String {
        format!(
            "    <jdk version=\"2\">\n      <name value=\"{name}\" />\n      <type value=\"Python SDK\" />\n      <version value=\"{version}\" />\n      <homePath value=\"{home}\" />\n      {additional}\n    </jdk>\n"
        )
    }

    #[test]
    fn the_reference_interpreters_are_imported_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .unwrap();
        let project = root.join("demo");
        let uv_python = venv(
            &project.join(".venv"),
            "uv = 0.9.2\nversion_info = 3.13.14\n",
        );
        let plain_python = venv(&root.join("envs/plain"), "version = 3.12.13\n");
        let system = root.join("bin/python3.11");
        std::fs::create_dir_all(system.parent().unwrap()).unwrap();
        std::fs::write(&system, "").unwrap();
        let p = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let config = root.join("PyCharm2026.2");
        let older = root.join("PyCharm2025.3");
        reference_config(&older, &sdk("old", &p(&system), "Python 3.11.9", ""));
        reference_config(
            &config,
            &[
                sdk(
                    "uv (demo)",
                    &p(&uv_python),
                    "Python 3.13.14",
                    &format!(
                        "<additional IS_UV=\"true\" UV_WORKING_DIR=\"{}\" UV_TOOL_PATH=\"$USER_HOME$/.local/bin/uv\" ASSOCIATED_PROJECT_PATH=\"{}\"><setting name=\"FLAVOR_ID\" value=\"UvSdkFlavor\" /></additional>",
                        p(&project),
                        p(&project)
                    ),
                ),
                sdk(
                    "Python 3.12 (plain)",
                    &p(&plain_python),
                    "Python 3.12.13",
                    "<additional ASSOCIATED_PROJECT_PATH=\"$USER_HOME$/work/x\"><setting name=\"FLAVOR_ID\" value=\"VirtualEnvSdkFlavor\" /></additional>",
                ),
                sdk("Python 3.11", &p(&system), "Python 3.11.9", "<additional><setting name=\"FLAVOR_ID\" value=\"MacPythonSdkFlavor\" /></additional>"),
                sdk("gone", "/nowhere/bin/python", "Python 3.10.1", ""),
            ]
            .concat(),
        );
        let dirs = vec![config.clone(), older];
        let file = root.join("config").join(FILE);
        let table = load_or_import(&file, &dirs).unwrap();
        assert_eq!(table.imported.as_deref(), Some("PyCharm2026.2"));
        let names: Vec<_> = table.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["uv (demo)", "Python 3.12 (plain)", "Python 3.11"]);
        let uv = &table.entries[0];
        assert_eq!(uv.flavor, Flavor::Uv);
        assert_eq!(uv.version.as_deref(), Some("3.13.14"));
        assert_eq!(uv.associated_project.as_deref(), Some(project.as_path()));
        assert_eq!(uv.working_dir.as_deref(), Some(project.as_path()));
        assert_eq!(uv.uv, Some(home.join(".local/bin/uv")));
        let plain = &table.entries[1];
        assert_eq!(plain.flavor, Flavor::Venv);
        assert_eq!(plain.associated_project, Some(home.join("work/x")));
        assert_eq!(table.entries[2].flavor, Flavor::System);

        // Once: the file exists now, and the import is not repeated.
        save(&file, &[Change::Remove("Python 3.11".into())]).unwrap();
        let table = load_or_import(&file, &dirs).unwrap();
        assert_eq!(table.entries.len(), 2);
        // An explicit import adds only what is missing.
        assert_eq!(import_reference(&file, &dirs).unwrap(), 1);
        assert_eq!(import_reference(&file, &dirs).unwrap(), 0);
        // Without a reference configuration nothing is written.
        let none = root.join("none").join(FILE);
        assert_eq!(load_or_import(&none, &[]).unwrap(), Table::default());
        assert!(!none.exists());
    }

    /// Environments made from one base interpreter link to it: they are
    /// still different interpreters.
    #[cfg(unix)]
    #[test]
    fn environments_of_one_base_are_different_interpreters() {
        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        let base = root.join("base/python3.13");
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        std::fs::write(&base, "").unwrap();
        let link = |env: &str| {
            let python = root.join(env).join("bin/python");
            std::fs::create_dir_all(python.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&base, &python).unwrap();
            python
        };
        let (a, b) = (link("a"), link("b"));
        assert!(!same_path(&a, &b));
        assert!(same_path(&a, &root.join("b/../a/bin/python")));
        let mut table = Table::default();
        table
            .apply(&Change::Add(Entry::new("a", &a, Flavor::Venv)))
            .unwrap();
        table
            .apply(&Change::Add(Entry::new("b", &b, Flavor::Venv)))
            .unwrap();
        assert_eq!(table.entries.len(), 2);
        assert_eq!(table.find_python(&b).unwrap().name, "b");
    }

    #[test]
    fn flavours_of_the_reference() {
        assert_eq!(Flavor::of_reference("UvSdkFlavor"), Some(Flavor::Uv));
        assert_eq!(
            Flavor::of_reference("VirtualEnvSdkFlavor"),
            Some(Flavor::Venv)
        );
        assert_eq!(
            Flavor::of_reference("CondaEnvSdkFlavor"),
            Some(Flavor::Conda)
        );
        assert_eq!(
            Flavor::of_reference("UnixPythonSdkFlavor"),
            Some(Flavor::System)
        );
        assert_eq!(Flavor::of_reference("Other"), None);
        for f in Flavor::ALL {
            assert_eq!(Flavor::parse(f.id()), Some(f));
        }
    }
}
