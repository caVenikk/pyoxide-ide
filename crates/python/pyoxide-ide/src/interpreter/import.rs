//! The interpreters of the reference IDE as an importer of the first-run
//! window (ADR 0017, `settings_ui::first_run`): it reads the SDK table
//! (`options/jdk.table.xml`) from the snapshot the import keeps, adds the
//! interpreters that still exist to the list ([`table`]) and runs once,
//! with the import or, for an import made before this area existed, at
//! the next start (`[import.applied]`). Only without an `[import]` record
//! (a configuration older than the first-run window) the list is imported
//! from the reference's own folder when its file does not exist yet
//! ([`list::import_reference`]).

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

    /// The reference's configuration folder holds two interpreters, its
    /// snapshot (what an import keeps) one of them.
    struct Machine {
        _dir: tempfile::TempDir,
        list_file: PathBuf,
        settings_file: PathBuf,
        reference: PathBuf,
    }

    fn machine() -> Machine {
        let dir = tempfile::tempdir().unwrap();
        let base = native_path::canonicalize(dir.path()).unwrap();
        let first = venv(&base.join("envs/first"));
        let second = venv(&base.join("envs/second"));
        let table = |sdks: String| {
            format!(
                "<application>\n  <component name=\"ProjectJdkTable\">\n{sdks}  </component>\n</application>\n"
            )
        };
        let reference = base.join("JetBrains/PyCharm2026.2");
        let snapshot = base.join("config/imported/PyCharm-2026.2");
        for (folder, sdks) in [
            (&reference, sdk("first", &first) + &sdk("second", &second)),
            (&snapshot, sdk("first", &first)),
        ] {
            std::fs::create_dir_all(folder.join("options")).unwrap();
            std::fs::write(folder.join("options/jdk.table.xml"), table(sdks)).unwrap();
        }
        Machine {
            list_file: base.join("config").join(FILE),
            settings_file: base.join("config/settings.toml"),
            reference,
            _dir: dir,
        }
    }

    /// The start-up as `main` runs it: the product starts and reads the
    /// list, the settings load, the first-run window decides (`decision`,
    /// none when it does not come), the main window opens and the areas
    /// take their part of the import.
    fn start_up(cx: &mut TestAppContext, m: &Machine, decision: Option<ImportSettings>) {
        let product = std::rc::Rc::new(crate::product_with_tools(Default::default()));
        cx.update(|cx| {
            list::set_file(cx, m.list_file.clone(), vec![m.reference.clone()]);
            workbench::init(cx, &product);
            store::load(cx, m.settings_file.clone());
        });
        cx.run_until_parked();
        cx.update(|cx| {
            if let Some(mut record) = decision {
                if record.imported_from().is_some() {
                    for id in first_run::importer_ids(cx) {
                        if id != ID {
                            record.mark_applied(id);
                        }
                    }
                }
                store::set(cx, &record);
            }
            workbench::open_main_window(cx, product.clone());
        });
        cx.run_until_parked();
        cx.update(first_run::pick_up);
        cx.run_until_parked();
    }

    #[gpui::test]
    fn skip_import_brings_nothing_from_the_reference(cx: &mut TestAppContext) {
        let m = machine();
        start_up(cx, &m, Some(ImportSettings::skipped()));
        assert!(names(cx).is_empty());
        assert!(!m.list_file.exists());
    }

    #[gpui::test]
    fn import_brings_the_snapshots_interpreters_once(cx: &mut TestAppContext) {
        let m = machine();
        let record = ImportSettings::imported("PyCharm", "2026.2", SystemTime::now())
            .with_snapshot("imported/PyCharm-2026.2");
        start_up(cx, &m, Some(record));
        // The snapshot's, not the reference folder's (which has another).
        assert_eq!(names(cx), ["first"]);
        assert_eq!(table::Table::load(&m.list_file).unwrap().entries.len(), 1);
        cx.update(first_run::pick_up);
        cx.update(list::import_reference);
        cx.run_until_parked();
        assert_eq!(names(cx), ["first"]);
    }

    #[gpui::test]
    fn a_configuration_without_an_import_record_keeps_the_direct_import(cx: &mut TestAppContext) {
        let m = machine();
        std::fs::create_dir_all(m.settings_file.parent().unwrap()).unwrap();
        std::fs::write(&m.settings_file, "# written before the first-run window\n").unwrap();
        start_up(cx, &m, None);
        assert_eq!(names(cx), ["first", "second"]);
        assert_eq!(table::Table::load(&m.list_file).unwrap().entries.len(), 2);
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
