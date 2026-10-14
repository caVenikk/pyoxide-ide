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
                t.insert(key, toml_edit::value(*value));
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
