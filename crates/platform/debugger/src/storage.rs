//! Breakpoints, watches and the mute state in the personal project state
//! (ADR 0009, 0015): the `[debugger]` table of `.oxide/workspace.toml`.
//! Paths are relative to the project root with `/` (absolute outside it),
//! lines as shown (from 1).
//!
//! ```toml
//! [debugger]
//! imported-from-idea = true
//! muted = false
//! suspend-defaults = { python-line = "all" }
//!
//! [[debugger.line-breakpoints]]
//! file = "src/main.py"
//! line = 7
//! condition = "x > 1"
//!
//! [[debugger.exception-breakpoints]]
//! type = "python-exception"
//! class = "BaseException"
//! default = true
//! properties = { notifyOnTerminate = "true" }
//!
//! [[debugger.watches]]
//! key = "python"
//! expressions = ["x", "len(items)"]
//! paused = ["len(items)"]
//!
//! [debugger.view]
//! expression-history = ["len(items)", "x + 1"]
//! watch-history = ["x"]
//! set-value-history = ["5"]
//! frames-split = 0.3
//! frames-hint-closed = true
//! ```
//!
//! `paused` lists the paused watches of a key by their expressions.
//! `[debugger.view]` keeps what Threads & Variables remembers: the
//! expressions typed last into the evaluate field, a watch's editor and
//! Set Value's editor (the reference keeps them per project and editor,
//! `XDebuggerHistoryManager`), the frames' share of the width once it was
//! dragged and the closed hint below the frames.
//!
//! Blocking file I/O: callers run it off the UI thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use project::store::StoreError;
use serde::{Deserialize, Serialize};

use crate::breakpoints::{
    BreakpointId, Breakpoints, ExceptionBreakpoint, LineBreakpoint, SuspendPolicy, Watches,
};

/// The table in the personal state.
pub const TABLE: &str = "debugger";
/// The key that marks the one-time import from `.idea` as done.
pub const IMPORTED_KEY: &str = "imported-from-idea";

/// What is stored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stored {
    pub imported: bool,
    pub muted: bool,
    pub breakpoints: Breakpoints,
    /// Whether exception breakpoints were stored (else the product's
    /// defaults apply).
    pub has_exception_breakpoints: bool,
    pub watches: Watches,
    /// What new breakpoints of a type suspend when Make Default chose
    /// another policy than the type's, by type id.
    pub suspend_defaults: BTreeMap<String, SuspendPolicy>,
    pub view: ViewState,
}

/// What Threads & Variables remembers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewState {
    /// The expressions evaluated last, the latest first.
    pub history: Vec<String>,
    /// The expressions typed last into a watch's editor and Set Value's.
    pub watch_history: Vec<String>,
    pub set_value_history: Vec<String>,
    /// The frames' share of the width, once it was dragged.
    pub frames_split: Option<f32>,
    pub frames_hint_closed: bool,
}

impl ViewState {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
struct Table {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    imported_from_idea: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    muted: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    suspend_defaults: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    line_breakpoints: Vec<Line>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exception_breakpoints: Option<Vec<Exception>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    watches: Vec<WatchList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<View>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
struct View {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    expression_history: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    watch_history: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    set_value_history: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames_split: Option<f32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    frames_hint_closed: bool,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_all(value: &String) -> bool {
    value == "all"
}

fn all() -> String {
    "all".to_owned()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Line {
    file: String,
    line: u32,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    enabled: bool,
    #[serde(default = "all", skip_serializing_if = "is_all")]
    suspend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    log_message: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    log_stack: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_expression: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    temporary: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Exception {
    #[serde(rename = "type")]
    kind: String,
    class: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    default: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    enabled: bool,
    #[serde(default = "all", skip_serializing_if = "is_all")]
    suspend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    properties: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WatchList {
    key: String,
    #[serde(default)]
    expressions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paused: Vec<String>,
}

/// A stored path: relative to the root with `/`, or absolute.
pub fn stored_path(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rest) => {
            let parts: Vec<String> = rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if parts.is_empty() {
                ".".to_owned()
            } else {
                parts.join("/")
            }
        }
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// A stored path on this machine.
pub fn resolve_path(stored: &str, root: &Path) -> Option<PathBuf> {
    run_config::paths::resolve(stored, root)
}

/// Reads the `[debugger]` table; `None` when it is missing.
pub fn load(root: &Path) -> Result<Option<Stored>, StoreError> {
    let Some(table) = project::workspace::load_table(root, TABLE)? else {
        return Ok(None);
    };
    let table: Table = toml::Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| StoreError::Invalid {
            path: project::workspace::workspace_file(root),
            message: e.to_string(),
        })?;
    let mut out = Stored {
        imported: table.imported_from_idea,
        muted: table.muted,
        has_exception_breakpoints: table.exception_breakpoints.is_some(),
        suspend_defaults: table
            .suspend_defaults
            .iter()
            .filter_map(|(kind, policy)| Some((kind.clone(), SuspendPolicy::parse(policy)?)))
            .collect(),
        ..Stored::default()
    };
    for line in table.line_breakpoints {
        let Some(file) = resolve_path(&line.file, root) else {
            continue;
        };
        if line.line == 0 {
            continue;
        }
        out.breakpoints.add_line(LineBreakpoint {
            id: BreakpointId(0),
            file,
            line: line.line,
            enabled: line.enabled,
            suspend: SuspendPolicy::parse(&line.suspend).unwrap_or_default(),
            condition: line.condition.filter(|c| !c.trim().is_empty()),
            log_message: line.log_message,
            log_stack: line.log_stack,
            log_expression: line.log_expression.filter(|e| !e.trim().is_empty()),
            temporary: line.temporary,
        });
    }
    for e in table.exception_breakpoints.unwrap_or_default() {
        out.breakpoints.add_exception(ExceptionBreakpoint {
            id: BreakpointId(0),
            kind: e.kind,
            class: e.class,
            default: e.default,
            enabled: e.enabled,
            suspend: SuspendPolicy::parse(&e.suspend).unwrap_or_default(),
            condition: e.condition.filter(|c| !c.trim().is_empty()),
            properties: e.properties,
        });
    }
    for list in table.watches {
        let expressions = list.expressions;
        let mut paused = list.paused;
        let flags: Vec<bool> = expressions
            .iter()
            .map(|e| match paused.iter().position(|p| p == e) {
                Some(i) => {
                    paused.remove(i);
                    true
                }
                None => false,
            })
            .collect();
        out.watches.set(&list.key, expressions);
        for (i, _) in flags.iter().enumerate().filter(|(_, p)| **p) {
            out.watches.set_paused(&list.key, i, true);
        }
    }
    if let Some(view) = table.view {
        out.view = ViewState {
            history: view.expression_history,
            watch_history: view.watch_history,
            set_value_history: view.set_value_history,
            frames_split: view
                .frames_split
                .filter(|s| s.is_finite() && *s > 0.0 && *s < 1.0),
            frames_hint_closed: view.frames_hint_closed,
        };
    }
    Ok(Some(out))
}

/// Writes the `[debugger]` table (nothing is created while there is
/// nothing to store). Temporary breakpoints are not kept, as in the
/// reference.
pub fn save(root: &Path, stored: &Stored) -> Result<bool, StoreError> {
    let b = &stored.breakpoints;
    let table = Table {
        imported_from_idea: stored.imported,
        muted: stored.muted,
        suspend_defaults: stored
            .suspend_defaults
            .iter()
            .map(|(kind, policy)| (kind.clone(), policy.as_str().to_owned()))
            .collect(),
        line_breakpoints: b
            .line()
            .iter()
            .filter(|l| !l.temporary)
            .map(|l| Line {
                file: stored_path(&l.file, root),
                line: l.line,
                enabled: l.enabled,
                suspend: l.suspend.as_str().to_owned(),
                condition: l.condition.clone(),
                log_message: l.log_message,
                log_stack: l.log_stack,
                log_expression: l.log_expression.clone(),
                temporary: false,
            })
            .collect(),
        exception_breakpoints: stored.has_exception_breakpoints.then(|| {
            b.exception()
                .iter()
                .map(|e| Exception {
                    kind: e.kind.clone(),
                    class: e.class.clone(),
                    default: e.default,
                    enabled: e.enabled,
                    suspend: e.suspend.as_str().to_owned(),
                    condition: e.condition.clone(),
                    properties: e.properties.clone(),
                })
                .collect()
        }),
        watches: stored
            .watches
            .keys()
            .map(|(key, expressions)| WatchList {
                key: key.clone(),
                expressions: expressions.clone(),
                paused: expressions
                    .iter()
                    .zip(stored.watches.paused(key))
                    .filter(|(_, p)| *p)
                    .map(|(e, _)| e.clone())
                    .collect(),
            })
            .collect(),
        view: (!stored.view.is_empty()).then(|| View {
            expression_history: stored.view.history.clone(),
            watch_history: stored.view.watch_history.clone(),
            set_value_history: stored.view.set_value_history.clone(),
            frames_split: stored.view.frames_split,
            frames_hint_closed: stored.view.frames_hint_closed,
        }),
    };
    let text = toml::to_string(&table).map_err(|e| StoreError::Invalid {
        path: project::workspace::workspace_file(root),
        message: e.to_string(),
    })?;
    let doc: toml_edit::DocumentMut =
        text.parse()
            .map_err(|e: toml_edit::TomlError| StoreError::Invalid {
                path: project::workspace::workspace_file(root),
                message: e.to_string(),
            })?;
    let value = doc.as_table().clone();
    project::workspace::save_table(root, TABLE, (!value.is_empty()).then_some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_everything_but_temporary_breakpoints() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(load(root).unwrap(), None);
        assert!(!save(root, &Stored::default()).unwrap());
        assert!(!project::workspace::workspace_file(root).exists());

        let mut stored = Stored {
            imported: true,
            muted: true,
            has_exception_breakpoints: true,
            ..Stored::default()
        };
        let main = root.join("src").join("main.py");
        let mut first = LineBreakpoint::new(&main, 7);
        first.condition = Some("x > 1".into());
        first.suspend = SuspendPolicy::Thread;
        stored.breakpoints.add_line(first);
        let mut logging = LineBreakpoint::new(&main, 9);
        logging.enabled = false;
        logging.suspend = SuspendPolicy::None;
        logging.log_message = true;
        logging.log_expression = Some("items".into());
        stored.breakpoints.add_line(logging);
        let mut temporary = LineBreakpoint::new(&main, 11);
        temporary.temporary = true;
        stored.breakpoints.add_line(temporary);
        let mut any = ExceptionBreakpoint::new("python-exception", "BaseException");
        any.default = true;
        any.properties
            .insert("notifyOnTerminate".into(), "true".into());
        stored.breakpoints.add_exception(any);
        stored.watches.add("python", "x", None);
        stored.watches.add("python", "len(items)", None);
        stored.watches.set_paused("python", 1, true);
        stored
            .suspend_defaults
            .insert("python-line".into(), SuspendPolicy::All);
        stored.view = ViewState {
            history: vec!["len(items)".into(), "x + 1".into()],
            watch_history: vec!["x".into()],
            set_value_history: vec!["5".into()],
            frames_split: Some(0.25),
            frames_hint_closed: true,
        };
        assert!(save(root, &stored).unwrap());

        let text = std::fs::read_to_string(project::workspace::workspace_file(root)).unwrap();
        assert!(text.contains("[[debugger.line-breakpoints]]"), "{text}");
        assert!(text.contains("file = \"src/main.py\""), "{text}");
        assert!(!text.contains("line = 11"), "{text}");

        let loaded = load(root).unwrap().unwrap();
        assert!(loaded.imported && loaded.muted && loaded.has_exception_breakpoints);
        let lines = loaded.breakpoints.in_file(&main);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].condition.as_deref(), Some("x > 1"));
        assert_eq!(lines[0].suspend, SuspendPolicy::Thread);
        assert!(lines[1].logs_only() && !lines[1].enabled);
        assert_eq!(lines[1].log_expression.as_deref(), Some("items"));
        let any = &loaded.breakpoints.exception()[0];
        assert!(any.default && any.flag("notifyOnTerminate"));
        assert_eq!(loaded.watches.get("python"), ["x", "len(items)"]);
        assert_eq!(loaded.watches.paused("python"), [false, true]);
        assert!(text.contains("paused = [\"len(items)\"]"), "{text}");
        assert_eq!(
            loaded.suspend_defaults.get("python-line"),
            Some(&SuspendPolicy::All)
        );
        assert!(text.contains("[debugger.view]"), "{text}");
        assert_eq!(loaded.view, stored.view);

        // Everything removed: the table goes.
        assert!(save(root, &Stored::default()).unwrap());
        assert_eq!(load(root).unwrap(), None);
    }

    #[test]
    fn paths_outside_the_project_stay_absolute() {
        let root = Path::new("/work/project");
        assert_eq!(stored_path(&root.join("a").join("b.py"), root), "a/b.py");
        let outside = Path::new("/usr/lib/python3/os.py");
        assert_eq!(stored_path(outside, root), outside.to_string_lossy());
        assert_eq!(
            resolve_path("a/b.py", root),
            Some(root.join("a").join("b.py"))
        );
    }
}
