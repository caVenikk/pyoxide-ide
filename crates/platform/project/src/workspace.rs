//! Personal project state (ADR 0009): `workspace.toml` in Pyoxide's
//! project folder ([`STORE_DIR`]) holds what belongs to one person on one
//! machine, such as the chosen interpreter. It is never meant to be
//! committed, like the reference IDE's `.idea/workspace.xml`.
//!
//! ```toml
//! version = 1
//!
//! [python]
//! interpreter = ".venv/bin/python"
//! ```
//!
//! The file follows the conventions of the shared settings
//! ([`crate::store`]): a `version`, values by table and key, keys it does
//! not know survive a rewrite, writes replace the whole file at once, and
//! nothing is created while there is nothing to store. The platform knows
//! no language: tables and keys are the callers' (a product uses its own
//! table, `[python]` for the Python product).
//!
//! When Pyoxide creates the file it also writes `.oxide/.gitignore`
//! ignoring it, as the reference IDE writes `.idea/.gitignore` for
//! `workspace.xml`, but only when that `.gitignore` does not exist: a file
//! that is there is never rewritten.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

use crate::store::{
    STORE_DIR, StoreError, find_table, find_table_mut, lock_root, read_document, version_of,
    write_atomic,
};

/// The personal state in [`STORE_DIR`].
pub const WORKSPACE_FILE: &str = "workspace.toml";
/// Version of the personal state format this build reads and writes.
pub const WORKSPACE_VERSION: i64 = 1;
/// The ignore file Pyoxide writes next to [`WORKSPACE_FILE`].
pub const GITIGNORE_FILE: &str = ".gitignore";

const HEADER: &str =
    "# Pyoxide personal project state (this person, this machine). Not meant to be committed.\n";

/// What `.oxide/.gitignore` holds when Pyoxide creates it: the reference
/// IDE's group comment and the entry relative to the folder.
pub const GITIGNORE_TEXT: &str = "# Default ignored files\n/workspace.toml\n";

/// The personal state file of a project.
pub fn workspace_file(root: &Path) -> PathBuf {
    root.join(STORE_DIR).join(WORKSPACE_FILE)
}

/// The string value of `key` in `[table]` (a dotted path such as
/// `python.tools.ruff` for a nested table); `None` when the file, the table
/// or the key is missing, or the value is not a string.
pub fn load_value(root: &Path, table: &str, key: &str) -> Result<Option<String>, StoreError> {
    let path = workspace_file(root);
    let Some(doc) = read_document(&path)? else {
        return Ok(None);
    };
    let Some(table) = find_table(&doc, table, &path)? else {
        return Ok(None);
    };
    Ok(table.get(key).and_then(Item::as_str).map(str::to_owned))
}

/// Sets `key` in `[table]` (a dotted path) to `value`, or removes it with `None`, keeping
/// everything else in the file. Creates the file only when there is a value
/// to store, together with `.oxide/.gitignore` unless that exists. A file
/// of a newer version is not rewritten. Returns whether the file was
/// written.
pub fn save_value(
    root: &Path,
    table: &str,
    key: &str,
    value: Option<&str>,
) -> Result<bool, StoreError> {
    save_values(root, table, &[(key, value)])
}

/// Like [`save_value`] for several keys of `[table]` in one write.
pub fn save_values(
    root: &Path,
    table: &str,
    values: &[(&str, Option<&str>)],
) -> Result<bool, StoreError> {
    let items: Vec<(&str, Option<Item>)> = values
        .iter()
        .map(|(key, value)| (*key, value.map(toml_edit::value)))
        .collect();
    save_items(root, table, items)
}

/// Like [`save_values`] with TOML values of any type (booleans, numbers,
/// strings), for a product's typed settings; `None` removes the key.
pub fn save_toml_values(
    root: &Path,
    table: &str,
    values: Vec<(&str, Option<toml_edit::Value>)>,
) -> Result<bool, StoreError> {
    let items = values
        .into_iter()
        .map(|(key, value)| (key, value.map(Item::Value)))
        .collect();
    save_items(root, table, items)
}

/// The list of strings under `key` in `[table]` (a dotted path); empty
/// when the file, the table or the key is missing, values that are not
/// strings left out.
pub fn load_list(root: &Path, table: &str, key: &str) -> Result<Vec<String>, StoreError> {
    let path = workspace_file(root);
    let Some(doc) = read_document(&path)? else {
        return Ok(Vec::new());
    };
    let Some(table) = find_table(&doc, table, &path)? else {
        return Ok(Vec::new());
    };
    Ok(table
        .get(key)
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// Sets `key` in `[table]` to a list of strings (recent commit messages),
/// or removes it with `None`, under the rules of [`save_value`].
pub fn save_list(
    root: &Path,
    table: &str,
    key: &str,
    values: Option<&[String]>,
) -> Result<bool, StoreError> {
    let item = values.map(|values| {
        let array: toml_edit::Array = values.iter().map(String::as_str).collect();
        toml_edit::value(array)
    });
    save_items(root, table, vec![(key, item)])
}

fn save_items(
    root: &Path,
    table: &str,
    values: Vec<(&str, Option<Item>)>,
) -> Result<bool, StoreError> {
    let _lock = lock_root(root);
    let path = workspace_file(root);
    let existing = read_document(&path)?;
    let created = existing.is_none();
    if created && values.iter().all(|(_, v)| v.is_none()) {
        return Ok(false);
    }
    let mut doc = match existing {
        Some(doc) => doc,
        None => format!("{HEADER}version = {WORKSPACE_VERSION}\n")
            .parse::<DocumentMut>()
            .expect("the header is valid TOML"),
    };
    let stored = version_of(&doc, WORKSPACE_VERSION);
    if stored > WORKSPACE_VERSION {
        return Err(StoreError::Newer {
            path,
            version: stored,
        });
    }
    if doc.get("version").and_then(Item::as_integer) != Some(WORKSPACE_VERSION) {
        doc["version"] = toml_edit::value(WORKSPACE_VERSION);
    }
    let mut changed = false;
    for (key, value) in values {
        let Some(t) = find_table_mut(&mut doc, table, &path, value.is_some())? else {
            continue;
        };
        match value {
            Some(value) => {
                t.insert(key, value);
                changed = true;
            }
            None => changed |= t.remove(key).is_some(),
        }
    }
    if !changed {
        return Ok(false);
    }
    write_atomic(&path, &doc.to_string()).map_err(|source| StoreError::Io {
        path: path.clone(),
        source,
    })?;
    if created {
        write_gitignore(root)?;
    }
    Ok(true)
}

/// The table under the top-level key `table` with its values as they are
/// stored (a product or a platform feature keeping a structured state,
/// such as run configurations); `None` when the file or the table is
/// missing.
pub fn load_table(root: &Path, table: &str) -> Result<Option<toml::Table>, StoreError> {
    let path = workspace_file(root);
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

/// Replaces the table under the top-level key `table` with `value`, or
/// removes it with `None`, keeping everything else in the file, under the
/// rules of [`save_value`]: the file (and `.oxide/.gitignore`) is created
/// only when there is something to store, and a file of a newer version is
/// not rewritten. Returns whether the file was written.
pub fn save_table(
    root: &Path,
    table: &str,
    value: Option<toml_edit::Table>,
) -> Result<bool, StoreError> {
    let _lock = lock_root(root);
    let path = workspace_file(root);
    let existing = read_document(&path)?;
    let created = existing.is_none();
    if created && value.as_ref().is_none_or(|t| t.is_empty()) {
        return Ok(false);
    }
    let mut doc = match existing {
        Some(doc) => doc,
        None => format!("{HEADER}version = {WORKSPACE_VERSION}\n")
            .parse::<DocumentMut>()
            .expect("the header is valid TOML"),
    };
    let stored = version_of(&doc, WORKSPACE_VERSION);
    if stored > WORKSPACE_VERSION {
        return Err(StoreError::Newer {
            path,
            version: stored,
        });
    }
    if doc.get("version").and_then(Item::as_integer) != Some(WORKSPACE_VERSION) {
        doc["version"] = toml_edit::value(WORKSPACE_VERSION);
    }
    let before = doc.to_string();
    match value {
        Some(value) => {
            doc.insert(table, Item::Table(value));
        }
        None => {
            doc.remove(table);
        }
    }
    let text = doc.to_string();
    if !created && text == before {
        return Ok(false);
    }
    write_atomic(&path, &text).map_err(|source| StoreError::Io {
        path: path.clone(),
        source,
    })?;
    if created {
        write_gitignore(root)?;
    }
    Ok(true)
}

/// Writes `.oxide/.gitignore` unless it exists (any file there, even an
/// empty one, is the person's and stays as it is).
fn write_gitignore(root: &Path) -> Result<(), StoreError> {
    let path = root.join(STORE_DIR).join(GITIGNORE_FILE);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    let mut file = match file {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(source) => return Err(StoreError::Io { path, source }),
    };
    use std::io::Write as _;
    file.write_all(GITIGNORE_TEXT.as_bytes())
        .map_err(|source| StoreError::Io { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn gitignore(root: &Path) -> Option<String> {
        std::fs::read_to_string(root.join(STORE_DIR).join(GITIGNORE_FILE)).ok()
    }

    #[test]
    fn values_round_trip_and_the_file_is_created_lazily() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Nothing to store: nothing is created.
        assert!(!save_value(root, "python", "interpreter", None).unwrap());
        assert!(!root.join(STORE_DIR).exists());
        assert_eq!(load_value(root, "python", "interpreter").unwrap(), None);

        assert!(save_value(root, "python", "interpreter", Some(".venv/bin/python")).unwrap());
        assert_eq!(
            load_value(root, "python", "interpreter")
                .unwrap()
                .as_deref(),
            Some(".venv/bin/python")
        );
        let text = std::fs::read_to_string(workspace_file(root)).unwrap();
        assert_eq!(
            text,
            format!("{HEADER}version = 1\n\n[python]\ninterpreter = \".venv/bin/python\"\n")
        );
        // Replaced, then removed: the file stays without the key.
        save_value(root, "python", "interpreter", Some("venv/bin/python")).unwrap();
        assert_eq!(
            load_value(root, "python", "interpreter")
                .unwrap()
                .as_deref(),
            Some("venv/bin/python")
        );
        assert!(save_value(root, "python", "interpreter", None).unwrap());
        assert_eq!(load_value(root, "python", "interpreter").unwrap(), None);
        assert!(!save_value(root, "python", "interpreter", None).unwrap());
        assert!(workspace_file(root).is_file());
        // No temporary files are left behind.
        let mut files: Vec<_> = std::fs::read_dir(root.join(STORE_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files, [GITIGNORE_FILE, WORKSPACE_FILE]);
    }

    #[test]
    fn lists_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(load_list(root, "vcs", "recent").unwrap().is_empty());
        let values = vec!["Fix\nmore".to_owned(), "First".to_owned()];
        assert!(save_list(root, "vcs", "recent", Some(&values)).unwrap());
        save_value(root, "vcs", "draft", Some("wip")).unwrap();
        assert_eq!(load_list(root, "vcs", "recent").unwrap(), values);
        assert_eq!(
            load_value(root, "vcs", "draft").unwrap().as_deref(),
            Some("wip")
        );
        assert!(save_list(root, "vcs", "recent", None).unwrap());
        assert!(load_list(root, "vcs", "recent").unwrap().is_empty());
    }

    #[test]
    fn creating_the_file_writes_the_gitignore_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        save_value(root, "python", "interpreter", Some("a")).unwrap();
        assert_eq!(gitignore(root).as_deref(), Some(GITIGNORE_TEXT));
        // Deleted by the person: not written again while the state exists.
        std::fs::remove_file(root.join(STORE_DIR).join(GITIGNORE_FILE)).unwrap();
        save_value(root, "python", "interpreter", Some("b")).unwrap();
        assert_eq!(gitignore(root), None);
    }

    #[test]
    fn an_existing_gitignore_is_never_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(STORE_DIR)).unwrap();
        let own = "# mine\n*.toml\n";
        std::fs::write(root.join(STORE_DIR).join(GITIGNORE_FILE), own).unwrap();
        save_value(root, "python", "interpreter", Some("a")).unwrap();
        assert_eq!(gitignore(root).as_deref(), Some(own));
        // An empty one is the person's as well.
        std::fs::write(root.join(STORE_DIR).join(GITIGNORE_FILE), "").unwrap();
        std::fs::remove_file(workspace_file(root)).unwrap();
        save_value(root, "python", "interpreter", Some("a")).unwrap();
        assert_eq!(gitignore(root).as_deref(), Some(""));
    }

    #[test]
    fn unknown_keys_survive_and_newer_files_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(STORE_DIR)).unwrap();
        let original = "# my notes\nversion = 1\nlayout = \"wide\"\n\n[python]\ninterpreter = \"old\" # keep me\nother = 3\n\n[run]\nlast = \"app\"\n";
        std::fs::write(workspace_file(root), original).unwrap();
        save_value(root, "python", "interpreter", Some("new")).unwrap();
        let text = std::fs::read_to_string(workspace_file(root)).unwrap();
        for kept in [
            "# my notes",
            "layout = \"wide\"",
            "other = 3",
            "[run]\nlast = \"app\"",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        assert_eq!(
            load_value(root, "python", "interpreter")
                .unwrap()
                .as_deref(),
            Some("new")
        );
        assert_eq!(
            load_value(root, "run", "last").unwrap().as_deref(),
            Some("app")
        );
        // The file existed: no ignore file is added.
        assert_eq!(gitignore(root), None);

        let newer = "version = 2\n[python]\ninterpreter = \"x\"\n";
        std::fs::write(workspace_file(root), newer).unwrap();
        assert_eq!(
            load_value(root, "python", "interpreter")
                .unwrap()
                .as_deref(),
            Some("x")
        );
        assert!(matches!(
            save_value(root, "python", "interpreter", Some("y")),
            Err(StoreError::Newer { version: 2, .. })
        ));
        assert_eq!(
            std::fs::read_to_string(workspace_file(root)).unwrap(),
            newer
        );

        std::fs::write(workspace_file(root), "python = 3\n").unwrap();
        assert!(matches!(
            load_value(root, "python", "interpreter"),
            Err(StoreError::Invalid { .. })
        ));
        assert!(save_value(root, "python", "interpreter", Some("y")).is_err());
    }

    #[test]
    fn nested_tables_by_dotted_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(save_value(root, "python.tools.ruff", "path", Some("/opt/tools/ruff")).unwrap());
        save_value(root, "python", "interpreter", Some(".venv/bin/python")).unwrap();
        let text = std::fs::read_to_string(workspace_file(root)).unwrap();
        assert!(
            text.contains("[python.tools.ruff]\npath = \"/opt/tools/ruff\"\n"),
            "{text}"
        );
        assert_eq!(
            load_value(root, "python.tools.ruff", "path")
                .unwrap()
                .as_deref(),
            Some("/opt/tools/ruff")
        );
        assert_eq!(
            load_value(root, "python", "interpreter")
                .unwrap()
                .as_deref(),
            Some(".venv/bin/python")
        );
        assert_eq!(load_value(root, "python.tools.ty", "path").unwrap(), None);
        assert!(save_value(root, "python.tools.ruff", "path", None).unwrap());
        assert_eq!(load_value(root, "python.tools.ruff", "path").unwrap(), None);
        // Removing from a table that is not there writes nothing.
        assert!(!save_value(root, "python.tools.ty", "path", None).unwrap());
    }

    #[test]
    fn whole_tables_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!save_table(root, "run", None).unwrap());
        assert!(!save_table(root, "run", Some(toml_edit::Table::new())).unwrap());
        assert!(!root.join(STORE_DIR).exists());
        save_value(root, "python", "interpreter", Some("a")).unwrap();
        let mut table = toml_edit::Table::new();
        table.insert("selected", toml_edit::value("x"));
        let mut item = toml_edit::Table::new();
        item.insert("name", toml_edit::value("api"));
        let mut list = toml_edit::ArrayOfTables::new();
        list.push(item);
        table.insert("configurations", Item::ArrayOfTables(list));
        assert!(save_table(root, "run", Some(table.clone())).unwrap());
        // The same again writes nothing.
        assert!(!save_table(root, "run", Some(table)).unwrap());
        let loaded = load_table(root, "run").unwrap().unwrap();
        assert_eq!(loaded["selected"].as_str(), Some("x"));
        assert_eq!(
            loaded["configurations"].as_array().unwrap()[0]["name"].as_str(),
            Some("api")
        );
        // Other tables stay.
        assert_eq!(
            load_value(root, "python", "interpreter")
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert!(save_table(root, "run", None).unwrap());
        assert_eq!(load_table(root, "run").unwrap(), None);
        assert_eq!(load_table(root, "missing").unwrap(), None);
    }

    #[test]
    fn personal_state_does_not_end_the_import_of_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".idea")).unwrap();
        std::fs::write(
            root.join(".idea/demo.iml"),
            r#"<module><component name="NewModuleRootManager"><content url="file://$MODULE_DIR$"><excludeFolder url="file://$MODULE_DIR$/build" /></content></component></module>"#,
        )
        .unwrap();
        save_value(root, "python", "interpreter", Some("a")).unwrap();
        assert!(!store::has_store(root));
        assert!(store::load_or_import(root).unwrap().paths.contains("build"));
        assert!(store::import_idea(root).unwrap().is_some());
        assert!(store::has_store(root));
    }
}
