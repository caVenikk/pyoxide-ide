//! The interpreters of the reference IDE as an importer of the first-run
//! window (ADR 0017, `settings_ui::first_run`): it reads the SDK table
//! (`options/jdk.table.xml`) from the snapshot the import keeps, adds the
//! interpreters that still exist to the list ([`table`]) and runs once,
//! with the import or, for an import made before this area existed, at
//! the next start (`[import.applied]`). Without a snapshot the list is
//! imported from the reference's own folder when its file does not exist
//! yet ([`table::load_or_import`]).

use gpui::App;
use settings_ui::first_run::{self, Applied, Importer, Outcome, Source, nothing};

use super::{list, table};

/// The importer's id in `[import.applied]`.
pub(super) const ID: &str = "python-interpreters";

/// Registers the importer (the product's `init`).
pub(super) fn register(cx: &mut App) {
    first_run::register_importer(
        cx,
        Importer {
            id: ID,
            work: import_interpreters,
        },
    );
}

/// Writes the list in the configuration folder; the list in memory is
/// read again on the UI thread.
fn import_interpreters(source: &Source) -> Applied {
    let Some(config_dir) = &source.config_dir else {
        return nothing();
    };
    let file = config_dir.join(table::FILE);
    let reference = source.folder.name.clone();
    let imported =
        list::exclusive(|| table::import_snapshot(&file, source.snapshot.dir(), &reference));
    match imported {
        Ok(0) => nothing(),
        Ok(count) => Box::new(move |cx| {
            list::reload(cx);
            Outcome::imported(if count == 1 {
                "1 Python interpreter".to_owned()
            } else {
                format!("{count} Python interpreters")
            })
        }),
        Err(e) => {
            let problem = Outcome::problem(format!(
                "The Python interpreters could not be imported: {e}."
            ));
            Box::new(move |_| problem)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use gpui::TestAppContext;
    use settings::ImportSettings;
    use settings_ui::{first_run, store};

    use super::super::table::{Change, FILE};
    use super::*;

    fn venv(dir: &Path) -> PathBuf {
        let python = py_env::paths::venv_python(dir);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        std::fs::write(dir.join("pyvenv.cfg"), "version = 3.12.13\n").unwrap();
        python
    }

    fn sdk(name: &str, home: &Path) -> String {
        format!(
            "    <jdk version=\"2\">\n      <name value=\"{name}\" />\n      <type value=\"Python SDK\" />\n      <version value=\"Python 3.12.13\" />\n      <homePath value=\"{}\" />\n    </jdk>\n",
            home.to_string_lossy().replace('\\', "/")
        )
    }

    /// A start-up with the import recorded, the snapshot holding an SDK
    /// table with two interpreters (one is gone from the disk) and every
    /// importer but this one already applied.
    fn start(cx: &mut TestAppContext) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = native_path::canonicalize(dir.path()).unwrap();
        let config = base.join("config");
        let python = venv(&base.join("envs/present"));
        let table = base.join("config/imported/PyCharm-2026.2/options/jdk.table.xml");
        std::fs::create_dir_all(table.parent().unwrap()).unwrap();
        std::fs::write(
            &table,
            format!(
                "<application>\n  <component name=\"ProjectJdkTable\">\n{}{}  </component>\n</application>\n",
                sdk("present", &python),
                sdk("gone", &base.join("envs/gone/bin/python")),
            ),
        )
        .unwrap();
        let list_file = config.join(FILE);
        let settings_file = config.join("settings.toml");
        cx.update(|cx| {
            first_run::init(cx);
            register(cx);
            store::load(cx, settings_file);
            let mut record = ImportSettings::imported("PyCharm", "2026.2", SystemTime::now())
                .with_snapshot("imported/PyCharm-2026.2");
            for id in first_run::importer_ids(cx) {
                if id != ID {
                    record.mark_applied(id);
                }
            }
            store::set(cx, &record);
            list::set_file(cx, list_file.clone(), Vec::new());
            list::ensure_loaded(cx);
        });
        cx.run_until_parked();
        (dir, list_file)
    }

    fn names(cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| {
            list::table(cx)
                .entries
                .iter()
                .map(|e| e.name.clone())
                .collect()
        })
    }

    #[gpui::test]
    fn the_snapshot_brings_the_interpreters_that_exist_once(cx: &mut TestAppContext) {
        let (_dir, file) = start(cx);
        assert!(names(cx).is_empty());
        cx.update(first_run::pick_up);
        cx.run_until_parked();
        assert_eq!(names(cx), ["present"]);
        assert_eq!(table::Table::load(&file).unwrap().entries.len(), 1);
        assert_eq!(
            table::Table::load(&file).unwrap().imported.as_deref(),
            Some("PyCharm2026.2")
        );
        let applied = cx.update(|cx| store::get_key(cx, &format!("import.applied.{ID}")));
        assert_eq!(applied, Some(serde_json::json!(true)));

        // Once: the interpreter removed afterwards does not come back.
        let removed = cx.update(|cx| list::change(vec![Change::Remove("present".into())], cx));
        cx.run_until_parked();
        drop(removed);
        cx.update(first_run::pick_up);
        cx.run_until_parked();
        assert!(names(cx).is_empty());
        assert!(table::Table::load(&file).unwrap().entries.is_empty());
    }

    #[test]
    fn a_snapshot_without_an_sdk_table_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        assert_eq!(
            table::import_snapshot(&file, &dir.path().join("snapshot"), "PyCharm2026.2"),
            Ok(0)
        );
        assert!(!file.exists());
    }
}
