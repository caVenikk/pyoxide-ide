//! The Python debugger options of a project (the reference's
//! `PyDebuggerOptionsProvider`, the page Settings > Python > Debugger):
//! the table `[python.debugger]` of `.oxide/workspace.toml` (ADR 0009).
//! The reference keeps them per project in `.idea/workspace.xml`, so a
//! project's options are imported from there once, and `.idea` is never
//! written.
//!
//! ```toml
//! [python.debugger]
//! imported-from-idea = true
//! attach-to-subprocess = false
//! gevent = true
//! pyqt = false
//! pyqt-backend = "pyqt5"
//! evaluation-timeout = 30000
//! ```
//!
//! Options at their defaults are not written; `imported-from-idea` closes
//! the import (it is written with the first save, whatever the values are).
//! In the reference's file the component looks like this, with only the
//! options that differ from the defaults:
//!
//! ```xml
//! <component name="PyDebuggerOptionsProvider">
//!   <option name="myAttachToSubprocess" value="false" />
//!   <option name="mySupportGeventDebugging" value="true" />
//!   <option name="mySupportQtDebugging" value="false" />
//!   <option name="myPyQtBackend" value="pyqt5" />
//!   <option name="myEvaluationResponseTimeout" value="30000" />
//! </component>
//! ```
//!
//! Not kept: "Collect run-time types information", "Drop into debugger on
//! failed tests", the filter of Attach To Process and the debugger mode
//! (pydevd is not supported), which have nothing here to configure yet.
//! Blocking file I/O: callers run it off the UI thread.

use std::path::Path;
use std::time::Duration;

use project::StoreError;
use project::idea::IDEA_DIR;
use project::workspace;
use toml_edit::Value;

use crate::launch::{DEFAULT_EVALUATION_TIMEOUT, DebugOptions};

/// The table in the personal project state.
pub const TABLE: &str = "python.debugger";
/// The key that closes the one-time import from `.idea`.
pub const IMPORTED_KEY: &str = "imported-from-idea";

/// The default of "Debugger evaluation response timeout (ms)".
pub const DEFAULT_TIMEOUT_MS: u32 = DEFAULT_EVALUATION_TIMEOUT.as_millis() as u32;
/// The largest timeout the reference's spinner takes (`Int.MAX_VALUE`).
pub const MAX_TIMEOUT_MS: u32 = i32::MAX as u32;

/// The component of the reference's `workspace.xml`.
const COMPONENT: &str = "PyDebuggerOptionsProvider";

mod key {
    pub const ATTACH_TO_SUBPROCESS: &str = "attach-to-subprocess";
    pub const GEVENT: &str = "gevent";
    pub const PYQT: &str = "pyqt";
    pub const PYQT_BACKEND: &str = "pyqt-backend";
    pub const EVALUATION_TIMEOUT: &str = "evaluation-timeout";
}

/// The Qt bindings "PyQt compatible" patches for (the combo box next to
/// it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QtBackend {
    Auto,
    PyQt4,
    PyQt5,
    PyQt6,
    PySide,
    PySide2,
    PySide6,
}

impl QtBackend {
    /// In the order of the reference's combo box.
    pub const ALL: [QtBackend; 7] = [
        QtBackend::Auto,
        QtBackend::PyQt4,
        QtBackend::PyQt5,
        QtBackend::PyQt6,
        QtBackend::PySide,
        QtBackend::PySide2,
        QtBackend::PySide6,
    ];

    /// What the reference stores (`name().lowercase()`).
    pub fn id(self) -> &'static str {
        match self {
            QtBackend::Auto => "auto",
            QtBackend::PyQt4 => "pyqt4",
            QtBackend::PyQt5 => "pyqt5",
            QtBackend::PyQt6 => "pyqt6",
            QtBackend::PySide => "pyside",
            QtBackend::PySide2 => "pyside2",
            QtBackend::PySide6 => "pyside6",
        }
    }

    /// What the combo box shows.
    pub fn name(self) -> &'static str {
        match self {
            QtBackend::Auto => "Auto",
            QtBackend::PyQt4 => "PyQt4",
            QtBackend::PyQt5 => "PyQt5",
            QtBackend::PyQt6 => "PyQt6",
            QtBackend::PySide => "PySide",
            QtBackend::PySide2 => "PySide2",
            QtBackend::PySide6 => "PySide6",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|b| b.id() == text)
    }

    /// debugpy 1.8.21's pydevd patches `auto`, `pyqt4`, `pyqt5`, `pyside`
    /// and `pyside2`; for the others the run goes without Qt support.
    pub fn debugpy_supports(self) -> bool {
        !matches!(self, QtBackend::PyQt6 | QtBackend::PySide6)
    }
}

/// The options of the page, with the reference's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebuggerSettings {
    /// "Attach to subprocess automatically while debugging" (on).
    pub attach_to_subprocess: bool,
    /// "Gevent compatible" (off).
    pub gevent: bool,
    /// "PyQt compatible" (on).
    pub pyqt: bool,
    /// The combo box next to it (Auto).
    pub pyqt_backend: QtBackend,
    /// "Debugger evaluation response timeout (ms)" (60000; 0 waits as long
    /// as the session lives).
    pub evaluation_timeout_ms: u32,
}

impl Default for DebuggerSettings {
    fn default() -> Self {
        Self {
            attach_to_subprocess: true,
            gevent: false,
            pyqt: true,
            pyqt_backend: QtBackend::Auto,
            evaluation_timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl DebuggerSettings {
    /// The launch options these settings give: the rest of `options`
    /// (other pages') stays.
    pub fn apply(&self, options: &mut DebugOptions) {
        options.attach_to_subprocess = self.attach_to_subprocess;
        options.gevent = self.gevent;
        options.qt = self.pyqt.then(|| self.pyqt_backend.id().to_owned());
        options.evaluation_timeout = (self.evaluation_timeout_ms > 0)
            .then(|| Duration::from_millis(u64::from(self.evaluation_timeout_ms)));
    }

    /// The options of `[python.debugger]`; what is missing or of another
    /// type is the default.
    fn from_table(table: &toml::Table) -> Self {
        let defaults = Self::default();
        let flag = |key: &str, default: bool| {
            table
                .get(key)
                .and_then(toml::Value::as_bool)
                .unwrap_or(default)
        };
        Self {
            attach_to_subprocess: flag(key::ATTACH_TO_SUBPROCESS, defaults.attach_to_subprocess),
            gevent: flag(key::GEVENT, defaults.gevent),
            pyqt: flag(key::PYQT, defaults.pyqt),
            pyqt_backend: table
                .get(key::PYQT_BACKEND)
                .and_then(toml::Value::as_str)
                .and_then(QtBackend::parse)
                .unwrap_or(defaults.pyqt_backend),
            evaluation_timeout_ms: table
                .get(key::EVALUATION_TIMEOUT)
                .and_then(toml::Value::as_integer)
                .and_then(|n| u32::try_from(n).ok())
                .map_or(defaults.evaluation_timeout_ms, |n| n.min(MAX_TIMEOUT_MS)),
        }
    }

    /// The values to write: those that differ from the defaults, the
    /// others removed; `imported-from-idea` always.
    fn items(&self) -> Vec<(&'static str, Option<Value>)> {
        let defaults = Self::default();
        let differs = |value: Value, differs: bool| differs.then_some(value);
        vec![
            (IMPORTED_KEY, Some(Value::from(true))),
            (
                key::ATTACH_TO_SUBPROCESS,
                differs(
                    Value::from(self.attach_to_subprocess),
                    self.attach_to_subprocess != defaults.attach_to_subprocess,
                ),
            ),
            (
                key::GEVENT,
                differs(Value::from(self.gevent), self.gevent != defaults.gevent),
            ),
            (
                key::PYQT,
                differs(Value::from(self.pyqt), self.pyqt != defaults.pyqt),
            ),
            (
                key::PYQT_BACKEND,
                differs(
                    Value::from(self.pyqt_backend.id()),
                    self.pyqt_backend != defaults.pyqt_backend,
                ),
            ),
            (
                key::EVALUATION_TIMEOUT,
                differs(
                    Value::from(i64::from(self.evaluation_timeout_ms)),
                    self.evaluation_timeout_ms != defaults.evaluation_timeout_ms,
                ),
            ),
        ]
    }
}

/// The project's options: those of `[python.debugger]`, else the
/// reference's of `.idea/workspace.xml` (written to `[python.debugger]`
/// once, [`import_idea`]), else the defaults.
pub fn load(root: &Path) -> Result<DebuggerSettings, StoreError> {
    if let Some(table) = workspace::load_table(root, TABLE)? {
        return Ok(DebuggerSettings::from_table(&table));
    }
    Ok(import_idea(root)?.unwrap_or_default())
}

/// Writes the options to `[python.debugger]` (defaults are not written, the
/// import from `.idea` is closed).
pub fn save(root: &Path, settings: &DebuggerSettings) -> Result<(), StoreError> {
    workspace::save_toml_values(root, TABLE, settings.items())?;
    Ok(())
}

/// Imports the reference's options of the project once: while
/// `[python.debugger]` does not exist, and `.idea/workspace.xml` has the
/// component, its options become `[python.debugger]`. Returns what was
/// imported.
pub fn import_idea(root: &Path) -> Result<Option<DebuggerSettings>, StoreError> {
    if workspace::load_table(root, TABLE)?.is_some() {
        return Ok(None);
    }
    let path = root.join(IDEA_DIR).join("workspace.xml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let Some(imported) = read_workspace(&text) else {
        return Ok(None);
    };
    save(root, &imported)?;
    Ok(Some(imported))
}

/// The options of a `workspace.xml` text; `None` without the component.
pub fn read_workspace(text: &str) -> Option<DebuggerSettings> {
    let doc = run_config::xml::parse(text).ok()?;
    let component = doc
        .children_named("component")
        .find(|c| c.attr("name") == Some(COMPONENT))?;
    let defaults = DebuggerSettings::default();
    let flag = |name: &str, default: bool| {
        component
            .field(name)
            .map_or(default, |v| v.trim().eq_ignore_ascii_case("true"))
    };
    Some(DebuggerSettings {
        attach_to_subprocess: flag("myAttachToSubprocess", defaults.attach_to_subprocess),
        gevent: flag("mySupportGeventDebugging", defaults.gevent),
        pyqt: flag("mySupportQtDebugging", defaults.pyqt),
        pyqt_backend: component
            .field("myPyQtBackend")
            .and_then(QtBackend::parse)
            .unwrap_or(defaults.pyqt_backend),
        evaluation_timeout_ms: component
            .field("myEvaluationResponseTimeout")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .map_or(defaults.evaluation_timeout_ms, |n| n.min(MAX_TIMEOUT_MS)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(workspace_xml: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if let Some(text) = workspace_xml {
            let idea = dir.path().join(IDEA_DIR);
            std::fs::create_dir_all(&idea).unwrap();
            std::fs::write(idea.join("workspace.xml"), text).unwrap();
        }
        dir
    }

    const REFERENCE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PyDebuggerOptionsProvider">
    <option name="myAttachToSubprocess" value="false" />
    <option name="mySupportGeventDebugging" value="true" />
    <option name="myPyQtBackend" value="pyside2" />
    <option name="myEvaluationResponseTimeout" value="30000" />
  </component>
</project>"#;

    #[test]
    fn the_reference_defaults() {
        let settings = DebuggerSettings::default();
        assert!(settings.attach_to_subprocess && settings.pyqt && !settings.gevent);
        assert_eq!(settings.pyqt_backend, QtBackend::Auto);
        assert_eq!(settings.evaluation_timeout_ms, 60_000);
        let mut options = DebugOptions::default();
        let before = options.clone();
        settings.apply(&mut options);
        assert_eq!(options, before, "the defaults are the launch's defaults");
    }

    #[test]
    fn settings_become_launch_options() {
        let settings = DebuggerSettings {
            attach_to_subprocess: false,
            gevent: true,
            pyqt: true,
            pyqt_backend: QtBackend::PyQt5,
            evaluation_timeout_ms: 0,
        };
        let mut options = DebugOptions {
            show_return_values: true,
            ..DebugOptions::default()
        };
        settings.apply(&mut options);
        assert!(!options.attach_to_subprocess && options.gevent);
        assert_eq!(options.qt.as_deref(), Some("pyqt5"));
        assert_eq!(options.evaluation_timeout, None, "0 ms waits for ever");
        assert!(options.show_return_values, "other pages' options stay");
        let off = DebuggerSettings {
            pyqt: false,
            evaluation_timeout_ms: 500,
            ..settings
        };
        off.apply(&mut options);
        assert_eq!(options.qt, None);
        assert_eq!(options.evaluation_timeout, Some(Duration::from_millis(500)));
    }

    #[test]
    fn saved_options_keep_only_what_differs() {
        let dir = project(None);
        assert_eq!(load(dir.path()).unwrap(), DebuggerSettings::default());
        let settings = DebuggerSettings {
            attach_to_subprocess: false,
            evaluation_timeout_ms: 1500,
            ..DebuggerSettings::default()
        };
        save(dir.path(), &settings).unwrap();
        let text = std::fs::read_to_string(workspace::workspace_file(dir.path())).unwrap();
        assert!(text.contains("[python.debugger]"), "{text}");
        assert!(text.contains("attach-to-subprocess = false"), "{text}");
        assert!(text.contains("evaluation-timeout = 1500"), "{text}");
        assert!(!text.contains("gevent") && !text.contains("pyqt"), "{text}");
        assert_eq!(load(dir.path()).unwrap(), settings);
        // Back to the defaults: the table stays and closes the import.
        save(dir.path(), &DebuggerSettings::default()).unwrap();
        let text = std::fs::read_to_string(workspace::workspace_file(dir.path())).unwrap();
        assert!(text.contains("imported-from-idea = true"), "{text}");
        assert!(!text.contains("attach-to-subprocess"), "{text}");
        assert_eq!(load(dir.path()).unwrap(), DebuggerSettings::default());
    }

    #[test]
    fn other_state_of_the_file_survives() {
        let dir = project(None);
        workspace::save_value(
            dir.path(),
            "python",
            "interpreter",
            Some(".venv/bin/python"),
        )
        .unwrap();
        save(dir.path(), &DebuggerSettings::default()).unwrap();
        assert_eq!(
            workspace::load_value(dir.path(), "python", "interpreter").unwrap(),
            Some(".venv/bin/python".to_owned())
        );
    }

    #[test]
    fn a_broken_value_is_the_default() {
        let dir = project(None);
        let path = workspace::workspace_file(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "version = 1\n[python.debugger]\ngevent = \"yes\"\npyqt-backend = \"qt9\"\nevaluation-timeout = -5\n",
        )
        .unwrap();
        assert_eq!(load(dir.path()).unwrap(), DebuggerSettings::default());
    }

    #[test]
    fn the_reference_options_are_imported_once() {
        let dir = project(Some(REFERENCE));
        let loaded = load(dir.path()).unwrap();
        assert_eq!(
            loaded,
            DebuggerSettings {
                attach_to_subprocess: false,
                gevent: true,
                pyqt: true,
                pyqt_backend: QtBackend::PySide2,
                evaluation_timeout_ms: 30_000,
            }
        );
        // Written: later changes in the reference do not reach it.
        let changed = REFERENCE.replace("30000", "45000");
        std::fs::write(dir.path().join(IDEA_DIR).join("workspace.xml"), changed).unwrap();
        assert_eq!(load(dir.path()).unwrap(), loaded);
        assert_eq!(import_idea(dir.path()).unwrap(), None);
        // A reset to the defaults in Pyoxide is not undone by the import.
        save(dir.path(), &DebuggerSettings::default()).unwrap();
        assert_eq!(load(dir.path()).unwrap(), DebuggerSettings::default());
    }

    #[test]
    fn no_component_nothing_imported() {
        let dir = project(Some(
            r#"<project version="4"><component name="Other"/></project>"#,
        ));
        assert_eq!(load(dir.path()).unwrap(), DebuggerSettings::default());
        assert!(!workspace::workspace_file(dir.path()).exists());
        let dir = project(None);
        assert_eq!(import_idea(dir.path()).unwrap(), None);
    }

    #[test]
    fn qt_backends() {
        assert_eq!(QtBackend::parse(" PyQt5 "), Some(QtBackend::PyQt5));
        assert_eq!(QtBackend::parse("none"), None);
        assert!(
            QtBackend::ALL
                .iter()
                .all(|b| QtBackend::parse(b.id()) == Some(*b))
        );
        assert!(QtBackend::PySide2.debugpy_supports());
        assert!(!QtBackend::PyQt6.debugpy_supports());
    }
}
