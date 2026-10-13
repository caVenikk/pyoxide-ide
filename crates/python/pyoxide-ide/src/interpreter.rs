//! The project's Python interpreter: the one runs use, and the interpreter
//! widget in the status bar with its popup (the reference IDE's
//! `PySdkStatusBar` and `PySdkPopupFactory`, PyCharm 2026.2).
//!
//! The interpreters offered are the environments found in the project
//! (`py_env::venv::project_environments`): no machine-wide discovery, and
//! no table of configured interpreters yet. They are named as the
//! reference names an interpreter it sets up for a folder: by the
//! environment's path (`~/work/demo/.venv`, shown as `.venv [3.13.1]`).
//! The project's environment (`UV_PROJECT_ENVIRONMENT`, `.venv`, `venv`)
//! is the default, as the reference picks it when a folder opens; a choice
//! is kept in the personal project state (`.oxide/workspace.toml`, ADR
//! 0009) with the path relative to the project when it is inside it. The
//! reference's own interpreter of the project (`.idea` and its SDK table)
//! is imported there once, with its SDK name (`uv (demo) [3.13.1]`).
//!
//! Everything that reads files runs on the background executor. Versions
//! come from `pyvenv.cfg`; only when it holds an incomplete one (uv may
//! write `3.14`) is the interpreter asked, in the background.

mod widget;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{App, AppContext as _, Global, Task};
use py_env::InterpreterKind;
use py_env::venv::{EnvDir, EnvLayout};
use serde_json::{Value as Json, json};

pub use widget::{COMMANDS, ChooseInterpreter, InterpreterState, SelectInterpreter};

/// The interpreter chosen for a project.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedInterpreter {
    /// The executable to run.
    pub python: PathBuf,
    pub kind: py_env::InterpreterKind,
    /// `3.13.1`; empty when the files do not tell it.
    pub version: String,
    /// The text of the interpreter widget (`.venv [3.13.1]`).
    pub label: String,
    /// The uv project the environment belongs to (`[project] name` of a
    /// project managed by uv), when it is one.
    pub uv_project: Option<String>,
}

/// The interpreter of the project at `root`: the one chosen in the
/// interpreter widget (kept in the personal project state), else the
/// project's environment (`UV_PROJECT_ENVIRONMENT`, then `.venv` or `venv`
/// in the project or a parent). Looked up off the UI thread.
pub fn for_project(root: &Path, cx: &App) -> Task<Option<SelectedInterpreter>> {
    let root = root.to_owned();
    let chosen = chosen(&root, cx);
    cx.background_spawn(async move {
        let found = scan(&root, chosen.as_deref());
        found.selected.map(|s| s.to_selected(&root))
    })
}

/// Registers the interpreter widget's icons and commands (the product's
/// `init`).
pub fn init(cx: &mut App) {
    widget::init(cx);
}

/// The interpreter widget of the status bar.
pub fn status_widget() -> workbench::StatusWidget {
    widget::status_widget()
}

// ---- choices ------------------------------------------------------------------

/// Table and key of the choice in the personal project state.
const STATE_TABLE: &str = "python";
const STATE_KEY: &str = "interpreter";

/// Choices made in this session, by project root: they count at once,
/// while the personal project state is written in the background.
#[derive(Default)]
struct Chosen(HashMap<PathBuf, PathBuf>);

impl Global for Chosen {}

fn chosen(root: &Path, cx: &App) -> Option<PathBuf> {
    cx.try_global::<Chosen>()
        .and_then(|c| c.0.get(root).cloned())
}

fn set_chosen(root: &Path, python: Option<PathBuf>, cx: &mut App) {
    let chosen = cx.default_global::<Chosen>();
    match python {
        Some(python) => chosen.0.insert(root.to_owned(), python),
        None => chosen.0.remove(root),
    };
}

/// A path the person gave: relative to the project, or absolute.
fn resolve(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

/// Keeps the choice in the personal project state: relative to the
/// project with `/` separators when it is inside it.
fn store_choice(root: &Path, python: &Path) -> Result<bool, project::StoreError> {
    let value = project::store::relative_to(root, python)
        .unwrap_or_else(|| python.to_string_lossy().into_owned());
    project::workspace::save_value(root, STATE_TABLE, STATE_KEY, Some(&value))
}

// ---- found interpreters --------------------------------------------------------

/// Where the popup lists an interpreter (`PyRenderedSdkType`): plain
/// virtual environments and conda first, then the rest, uv's included
/// (`UvSdkFlavor` is a `CPythonSdkFlavor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    VirtualEnv,
    System,
}

/// An interpreter of the project as the widget and the popup show it.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundInterpreter {
    pub python: PathBuf,
    /// The environment folder.
    pub dir: PathBuf,
    pub kind: InterpreterKind,
    pub version: Option<String>,
    /// A virtual environment created by uv.
    pub uv: bool,
    /// The name, as the reference names the interpreter.
    pub name: String,
    /// The name is a path (`~/…`), shortened to its last part.
    path_derived: bool,
}

/// Widget text and popup rows fit these lengths (`shortName`, `longName`).
const SHORT_NAME: usize = 50;
const LONG_NAME: usize = 100;
const ELLIPSIS: char = '\u{2026}';

impl FoundInterpreter {
    fn from_env(env: &EnvDir) -> Self {
        let kind = match env.layout {
            EnvLayout::Venv => InterpreterKind::Venv,
            EnvLayout::Conda => InterpreterKind::Conda,
            EnvLayout::Installation => InterpreterKind::System,
        };
        Self {
            python: env.python.clone(),
            dir: env.dir.clone(),
            kind,
            version: full_version(env.version.as_deref(), &env.python),
            uv: env.is_uv(),
            name: sdk_name(env),
            path_derived: true,
        }
    }

    /// An interpreter outside any environment folder.
    fn bare(python: &Path) -> Self {
        Self {
            python: python.to_owned(),
            dir: python.parent().unwrap_or(python).to_owned(),
            kind: InterpreterKind::System,
            version: full_version(None, python),
            uv: false,
            name: project::display_path(python),
            path_derived: true,
        }
    }

    fn group(&self) -> Group {
        match self.kind {
            InterpreterKind::Venv | InterpreterKind::Conda if !self.uv => Group::VirtualEnv,
            _ => Group::System,
        }
    }

    /// The widget's text: `name [X.Y.Z]` in at most 50 chars, a path cut
    /// to its last part (`PythonInterpreterPresentation.shortName`).
    pub fn short_name(&self) -> String {
        compact_name(
            &self.name,
            self.version.as_deref(),
            self.path_derived,
            SHORT_NAME,
            false,
        )
    }

    /// The popup row: at most 100 chars, a path shortened in the middle
    /// (`longName`).
    pub fn long_name(&self) -> String {
        compact_name(
            &self.name,
            self.version.as_deref(),
            self.path_derived,
            LONG_NAME,
            true,
        )
    }

    /// The widget's tooltip (`current.interpreter`, the SDK's home path).
    pub fn tooltip(&self) -> String {
        format!("Current Interpreter: {}", self.python.display())
    }

    /// The popup row's description (`python.sdk.switch.to`).
    pub fn description(&self) -> String {
        format!("Switch to {}", self.python.display())
    }

    fn to_selected(&self, root: &Path) -> SelectedInterpreter {
        let uv_project = self
            .uv
            .then(|| py_env::load_project(root).ok())
            .flatten()
            .and_then(|p| p.pyproject.and_then(|p| p.name));
        SelectedInterpreter {
            python: self.python.clone(),
            kind: self.kind,
            version: self.version.clone().unwrap_or_default(),
            label: self.short_name(),
            uv_project,
        }
    }

    fn json(&self) -> Json {
        json!({
            "name": self.name,
            "text": self.long_name(),
            "python": self.python.display().to_string(),
            "dir": self.dir.display().to_string(),
            "kind": self.kind.as_str(),
            "version": self.version,
            "uv": self.uv,
        })
    }
}

/// The version shown after the name (`3.14.6`): the one the files tell
/// when it is complete, else the one the interpreter reports, as the
/// reference reads it from the SDK's version string (uv may write
/// `version_info = 3.14` into `pyvenv.cfg`).
fn full_version(files: Option<&str>, python: &Path) -> Option<String> {
    let complete =
        |v: &str| v.split('.').count() >= 3 && v.starts_with(|c: char| c.is_ascii_digit());
    match files {
        Some(v) if complete(v) => Some(v.to_owned()),
        _ => reported_version(python).or_else(|| files.map(str::to_owned)),
    }
}

/// How long an interpreter may take to tell its version.
const VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The version `python --version` prints (`Python 3.14.6` gives `3.14.6`,
/// the reference's `\d+\.\S+`). Blocking (background only), bounded by
/// [`VERSION_TIMEOUT`], stdin closed; remembered per interpreter while the
/// file is unchanged.
fn reported_version(python: &Path) -> Option<String> {
    use std::io::Read as _;
    use std::sync::{LazyLock, Mutex};
    use std::time::{Instant, SystemTime};

    type Cache = HashMap<PathBuf, (Option<SystemTime>, Option<String>)>;
    static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);
    let modified = std::fs::metadata(python).and_then(|m| m.modified()).ok();
    if let Some((at, version)) = CACHE.lock().ok()?.get(python)
        && *at == modified
    {
        return version.clone();
    }
    let mut command = std::process::Command::new(python);
    command
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // No console window.
        command.creation_flags(0x0800_0000);
    }
    let version = (|| {
        let mut child = command.spawn().ok()?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < VERSION_TIMEOUT => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let mut out = String::new();
        child.stdout.take()?.read_to_string(&mut out).ok()?;
        if out.trim().is_empty() {
            // Python 2 printed its version to stderr.
            child.stderr.take()?.read_to_string(&mut out).ok()?;
        }
        version_in(&out)
    })();
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(python.to_owned(), (modified, version.clone()));
    }
    version
}

/// The first `\d+\.\S+` of a version string.
fn version_in(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let digits = word.find(|c: char| !c.is_ascii_digit())?;
        (digits > 0 && word[digits..].starts_with('.') && word.len() > digits + 1)
            .then(|| word.to_owned())
    })
}

/// The name the reference gives an interpreter it sets up for a folder it
/// opens (`PythonSdkType.suggestSdkName`): the environment's folder
/// relative to the home folder (`~/work/demo/.venv`), for an interpreter
/// outside an environment its path.
fn sdk_name(env: &EnvDir) -> String {
    match env.layout {
        EnvLayout::Venv | EnvLayout::Conda => project::display_path(&env.dir),
        EnvLayout::Installation => project::display_path(&env.python),
    }
}

/// `name [suffix]` in at most `max` chars, the suffix kept
/// (`PythonInterpreterPresentation.compactName`).
fn compact_name(
    name: &str,
    suffix: Option<&str>,
    path_derived: bool,
    max: usize,
    keep_prefix: bool,
) -> String {
    let suffix = suffix.map(|s| format!(" [{s}]")).unwrap_or_default();
    let available = max as isize - suffix.chars().count() as isize;
    if available <= 0 {
        return trim_middle(&format!("{name}{suffix}"), max);
    }
    let available = available as usize;
    let name = if path_derived {
        shorten_path(name, available, keep_prefix)
    } else {
        trim_middle(name, available)
    };
    name + &suffix
}

/// `StringUtil.trimMiddle`: the start and the end of a text that is too
/// long, joined by an ellipsis.
fn trim_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_owned();
    }
    let end = max / 2;
    let start = max.saturating_sub(end + 1);
    let mut out: String = chars[..start].iter().collect();
    out.push(ELLIPSIS);
    out.extend(&chars[chars.len() - end..]);
    out
}

/// `shortenPath` of the reference: the last part of a path, or with
/// `keep_prefix` the path cut in the middle before its last part.
fn shorten_path(path: &str, max: usize, keep_prefix: bool) -> String {
    let normalized = path.trim_end_matches(['/', '\\']);
    let last_sep = normalized.rfind(['/', '\\']);
    let count = |s: &str| s.chars().count();
    if keep_prefix {
        if count(path) <= max {
            return path.to_owned();
        }
        if let Some(sep) = last_sep.filter(|&s| s > 0) {
            let last = &normalized[sep..];
            let room = max as isize - count(last) as isize - 1;
            if room > 0 {
                let head: String = normalized.chars().take(room as usize).collect();
                return format!("{head}{ELLIPSIS}{last}");
            }
        }
    }
    let Some(sep) = last_sep else {
        let chars: Vec<char> = path.chars().collect();
        return chars[chars.len().saturating_sub(max)..].iter().collect();
    };
    let last = &normalized[sep + 1..];
    if count(last) <= max {
        last.to_owned()
    } else {
        let chars: Vec<char> = last.chars().collect();
        let keep = max.saturating_sub(1);
        let mut out = String::from(ELLIPSIS);
        out.extend(&chars[chars.len() - keep..]);
        out
    }
}

/// What a project offers and which interpreter is selected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectInterpreters {
    /// The environments found in the project, each with a unique name;
    /// a chosen interpreter that is not among them is added.
    pub found: Vec<FoundInterpreter>,
    pub selected: Option<FoundInterpreter>,
    /// The selection is the person's choice (not the default).
    pub chosen: bool,
}

impl ProjectInterpreters {
    /// The interpreters in the popup's order: by group, then by name
    /// (`getAssignablePythonSdks`, `groupModuleSdksByTypes`).
    fn in_popup_order(&self) -> Vec<&FoundInterpreter> {
        let mut list: Vec<&FoundInterpreter> = self.found.iter().collect();
        list.sort_by(|a, b| a.group().cmp(&b.group()).then_with(|| a.name.cmp(&b.name)));
        list
    }
}

/// The project's interpreters and the selected one: `chosen` (or the
/// choice kept in the personal project state) when it still exists, else
/// the project's environment. Blocking.
fn scan(root: &Path, chosen: Option<&Path>) -> ProjectInterpreters {
    scan_with(root, chosen, pycharm_config_dirs)
}

/// [`scan`] with the reference IDE's configuration directories (newest
/// first) for the one-time import of its interpreter.
fn scan_with(
    root: &Path,
    chosen: Option<&Path>,
    config_dirs: impl FnOnce() -> Vec<PathBuf>,
) -> ProjectInterpreters {
    let uv_env = std::env::var_os("UV_PROJECT_ENVIRONMENT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let envs = py_env::venv::project_environments(root, uv_env.as_deref());
    let mut state = Stored::load(root);
    if chosen.is_none()
        && state.interpreter.is_none()
        && state.sdk_home.is_none()
        && let Some(imported) = import_idea_sdk(root, &envs, &config_dirs())
    {
        state = imported;
    }
    let mut found: Vec<FoundInterpreter> = Vec::new();
    for env in &envs {
        let mut item = FoundInterpreter::from_env(env);
        state.apply_name(&mut item);
        item.name = unique_name(&item.name, &found);
        found.push(item);
    }
    let chosen = chosen.map(Path::to_path_buf).or(state.interpreter.clone());
    let chosen = chosen.as_deref();
    let pick = chosen.and_then(|path| {
        let env = if path.is_dir() {
            py_env::venv::inspect(path)
        } else {
            py_env::venv::inspect_python(path)
        };
        match env {
            Some(env) => Some(
                found
                    .iter()
                    .find(|f| same_dir(&f.dir, &env.dir))
                    .cloned()
                    .unwrap_or_else(|| FoundInterpreter::from_env(&env)),
            ),
            None if path.is_file() => Some(
                found
                    .iter()
                    .find(|f| same_dir(&f.python, path))
                    .cloned()
                    .unwrap_or_else(|| FoundInterpreter::bare(path)),
            ),
            None => None,
        }
    });
    let chosen = pick.is_some();
    let selected = pick.or_else(|| {
        let dir = py_env::project::find_project_venv(root, uv_env.as_deref())?;
        found.iter().find(|f| same_dir(&f.dir, &dir)).cloned()
    });
    if let Some(selected) = &selected
        && !found.iter().any(|f| f == selected)
    {
        let mut item = selected.clone();
        state.apply_name(&mut item);
        item.name = unique_name(&item.name, &found);
        found.push(item);
    }
    let selected = selected.and_then(|s| found.iter().find(|f| f.python == s.python).cloned());
    ProjectInterpreters {
        found,
        selected,
        chosen,
    }
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || canonical(a) == canonical(b)
}

/// The environment folder of an interpreter: the parent of `bin`
/// (`Scripts`), else its own folder.
fn env_dir_of(python: &Path) -> PathBuf {
    let parent = python.parent().unwrap_or(python);
    match parent.file_name().and_then(|n| n.to_str()) {
        Some("bin" | "Scripts") => parent.parent().unwrap_or(parent).to_owned(),
        _ => parent.to_owned(),
    }
}

// ---- the reference IDE's interpreter ----------------------------------------------

/// Keys of the interpreter imported from the reference IDE: its SDK name
/// and home path. The name is shown for that interpreter.
const SDK_NAME_KEY: &str = "sdk-name";
const SDK_HOME_KEY: &str = "sdk-home";
/// The SDK type of Python interpreters in the reference IDE.
const PYTHON_SDK: &str = "Python SDK";

/// What the personal project state holds about the interpreter.
#[derive(Debug, Clone, Default, PartialEq)]
struct Stored {
    interpreter: Option<PathBuf>,
    sdk_name: Option<String>,
    sdk_home: Option<PathBuf>,
}

impl Stored {
    fn load(root: &Path) -> Self {
        let value = |key: &str| {
            project::workspace::load_value(root, STATE_TABLE, key)
                .ok()
                .flatten()
                .filter(|v| !v.is_empty())
        };
        Self {
            interpreter: value(STATE_KEY).map(|p| resolve(root, &p)),
            sdk_name: value(SDK_NAME_KEY),
            sdk_home: value(SDK_HOME_KEY).map(|p| resolve(root, &p)),
        }
    }

    /// The reference's SDK name for the interpreter it was imported for.
    fn apply_name(&self, item: &mut FoundInterpreter) {
        let (Some(name), Some(home)) = (&self.sdk_name, &self.sdk_home) else {
            return;
        };
        if same_dir(&item.python, home) || same_dir(&item.dir, &env_dir_of(home)) {
            item.name = name.clone();
            item.path_derived = false;
        }
    }
}

/// The interpreter of the reference IDE's project, imported once into the
/// personal project state (ADR 0009) while it holds no choice: the SDK the
/// project names (`.idea`), looked up in the SDK table of the newest
/// configuration directory that has it, when its interpreter exists. The
/// interpreter becomes the choice, and its SDK name is kept for it.
fn import_idea_sdk(root: &Path, envs: &[EnvDir], config_dirs: &[PathBuf]) -> Option<Stored> {
    let sdk = project::idea::project_sdk(root)?;
    if sdk.type_id != PYTHON_SDK {
        return None;
    }
    let entry = config_dirs.iter().find_map(|dir| {
        project::idea::read_sdk_table(&dir.join("options").join("jdk.table.xml"))
            .into_iter()
            .find(|e| e.name == sdk.name && e.type_id == sdk.type_id)
    })?;
    let home = entry.home.filter(|h| h.is_file())?;
    let python = envs
        .iter()
        .find(|e| same_dir(&e.dir, &env_dir_of(&home)))
        .map(|e| e.python.clone())
        .unwrap_or_else(|| home.clone());
    let stored_path = |p: &Path| {
        project::store::relative_to(root, p).unwrap_or_else(|| p.to_string_lossy().into_owned())
    };
    let values = [
        (STATE_KEY, stored_path(&python)),
        (SDK_NAME_KEY, sdk.name.clone()),
        (SDK_HOME_KEY, stored_path(&home)),
    ];
    let values: Vec<(&str, Option<&str>)> =
        values.iter().map(|(k, v)| (*k, Some(v.as_str()))).collect();
    if let Err(e) = project::workspace::save_values(root, STATE_TABLE, &values) {
        eprintln!("interpreter: {e}");
    }
    Some(Stored {
        interpreter: Some(python),
        sdk_name: Some(sdk.name),
        sdk_home: Some(home),
    })
}

/// The reference IDE's configuration directories on this machine
/// (`PyCharm2026.2`, `PyCharmCE2025.1`, …), newest first.
fn pycharm_config_dirs() -> Vec<PathBuf> {
    let Some(root) = dirs::config_dir().map(|d| d.join("JetBrains")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(Vec<u32>, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let version = ["PyCharmCE", "PyCharm"]
                .iter()
                .find_map(|p| name.strip_prefix(p))?;
            let key: Vec<u32> = version
                .split('.')
                .map(|p| p.parse().ok())
                .collect::<Option<_>>()?;
            (key.len() >= 2).then(|| (key, e.path()))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// `name`, or `name (2)`, `name (3)`, … when taken
/// (`SdkConfigurationUtil.createUniqueSdkName`).
fn unique_name(name: &str, taken: &[FoundInterpreter]) -> String {
    let free = |n: &str| !taken.iter().any(|f| f.name == n);
    if free(name) {
        return name.to_owned();
    }
    (2..)
        .map(|i| format!("{name} ({i})"))
        .find(|n| free(n))
        .expect("a free name")
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

    fn project(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap().join(name);
        std::fs::create_dir_all(&root).unwrap();
        (dir, root)
    }

    /// No configuration directories of the reference IDE.
    fn scan(root: &Path, chosen: Option<&Path>) -> ProjectInterpreters {
        scan_with(root, chosen, Vec::new)
    }

    #[test]
    fn names_and_texts_as_the_reference_shows_them() {
        let (_dir, root) = project("demo");
        assert_eq!(scan(&root, None), ProjectInterpreters::default());

        let uv = venv(
            &root.join(".venv"),
            "home = /opt/py\nuv = 0.9.2\nversion_info = 3.13.14\n",
        );
        let plain = venv(&root.join("venv"), "home = /usr/bin\nversion = 3.12.13\n");
        let found = scan(&root, None);
        let names: Vec<_> = found.found.iter().map(|f| f.short_name()).collect();
        assert_eq!(names, [".venv [3.13.14]", "venv [3.12.13]"]);
        // The popup shows the path with `~` for the home folder.
        let path = |dir: &str| project::display_path(&root.join(dir));
        let long: Vec<_> = found.found.iter().map(|f| f.long_name()).collect();
        assert_eq!(
            long,
            [
                format!("{} [3.13.14]", path(".venv")),
                format!("{} [3.12.13]", path("venv"))
            ]
        );
        // The project's environment is the default: `.venv` first.
        let selected = found.selected.clone().unwrap();
        assert_eq!(selected.python, uv);
        assert!(selected.uv);
        assert!(!found.chosen);
        assert_eq!(
            selected.tooltip(),
            format!("Current Interpreter: {}", uv.display())
        );
        assert_eq!(
            selected.description(),
            format!("Switch to {}", uv.display())
        );
        // Plain virtual environments first, uv's with the others.
        let order: Vec<_> = found
            .in_popup_order()
            .iter()
            .map(|f| f.short_name())
            .collect();
        assert_eq!(order, ["venv [3.12.13]", ".venv [3.13.14]"]);

        // A choice wins while it exists.
        let found = scan(&root, Some(&plain));
        assert!(found.chosen);
        assert_eq!(found.selected.unwrap().python, plain);
        let found = scan(&root, Some(&root.join("venv")));
        assert_eq!(found.selected.unwrap().python, plain);
        let found = scan(&root, Some(&root.join("gone/bin/python")));
        assert!(!found.chosen);
        assert_eq!(found.selected.unwrap().python, uv);
    }

    #[test]
    fn no_environment_and_a_plain_venv() {
        let (_dir, root) = project("plain");
        std::fs::write(root.join("main.py"), "print(1)\n").unwrap();
        let found = scan(&root, None);
        assert!(found.found.is_empty());
        assert_eq!(found.selected, None);

        let python = venv(&root.join("venv"), "version = 3.11.4\n");
        let found = scan(&root, None);
        let selected = found.selected.unwrap();
        assert_eq!(selected.python, python);
        assert_eq!(selected.kind, InterpreterKind::Venv);
        assert!(!selected.uv);
        let s = selected.to_selected(&root);
        assert_eq!(s.label, "venv [3.11.4]");
        assert_eq!(s.version, "3.11.4");
        assert_eq!(s.uv_project, None);
    }

    #[test]
    fn a_uv_project_names_its_project() {
        let (_dir, root) = project("demo");
        std::fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"demo-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.1\n");
        let s = scan(&root, None).selected.unwrap().to_selected(&root);
        assert_eq!(s.label, ".venv [3.13.1]");
        assert_eq!(s.kind, InterpreterKind::Venv);
        assert_eq!(s.uv_project.as_deref(), Some("demo-app"));
    }

    #[test]
    fn the_choice_is_kept_in_the_personal_project_state() {
        let (_dir, root) = project("demo");
        venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.1\n");
        let plain = venv(&root.join("venv"), "version = 3.12.13\n");
        assert!(store_choice(&root, &plain).unwrap());
        let text = std::fs::read_to_string(project::workspace::workspace_file(&root)).unwrap();
        let rel = if cfg!(windows) {
            "venv/Scripts/python.exe"
        } else {
            "venv/bin/python"
        };
        assert!(
            text.contains(&format!("[python]\ninterpreter = \"{rel}\"\n")),
            "{text}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".oxide/.gitignore")).unwrap(),
            "# Default ignored files\n/workspace.toml\n"
        );
        let found = scan(&root, None);
        assert!(found.chosen);
        assert_eq!(found.selected.unwrap().python, plain);
        // An interpreter outside the project is kept as an absolute path
        // and listed with the project's own.
        let (_other, elsewhere) = project("tools");
        let outside = venv(&elsewhere.join("env"), "version = 3.10.0\n");
        store_choice(&root, &outside).unwrap();
        let found = scan(&root, None);
        let selected = found.selected.clone().unwrap();
        assert_eq!(selected.python, outside);
        assert_eq!(selected.short_name(), "env [3.10.0]");
        assert_eq!(found.found.len(), 3);
    }

    /// A reference IDE's configuration directory with an SDK table.
    fn sdk_table(config: &Path, sdks: &[(&str, &Path)]) {
        let mut xml = String::from("<application>\n  <component name=\"ProjectJdkTable\">\n");
        for (name, home) in sdks {
            xml.push_str(&format!(
                "    <jdk version=\"2\">\n      <name value=\"{name}\" />\n      <type value=\"Python SDK\" />\n      <version value=\"Python 3.13.14\" />\n      <homePath value=\"{}\" />\n    </jdk>\n",
                home.display()
            ));
        }
        xml.push_str("  </component>\n</application>\n");
        std::fs::create_dir_all(config.join("options")).unwrap();
        std::fs::write(config.join("options/jdk.table.xml"), xml).unwrap();
    }

    fn idea_project_sdk(root: &Path, name: &str) {
        std::fs::create_dir_all(root.join(".idea")).unwrap();
        std::fs::write(
            root.join(".idea/misc.xml"),
            format!(
                "<project version=\"4\">\n  <component name=\"ProjectRootManager\" version=\"2\" project-jdk-name=\"{name}\" project-jdk-type=\"Python SDK\" />\n</project>\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn the_reference_interpreter_and_its_name_are_imported_once() {
        let (dir, root) = project("skin-tech-api");
        let uv = venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.14\n");
        let plain = venv(&root.join("venv"), "version = 3.12.13\n");
        idea_project_sdk(&root, "uv (skin-tech-api)");
        let newer = dir.path().join("PyCharm2026.2");
        let older = dir.path().join("PyCharm2025.3");
        sdk_table(&older, &[("uv (skin-tech-api)", &plain)]);
        sdk_table(
            &newer,
            &[
                ("Python 3.12", Path::new("/usr/bin/python3.12")),
                ("uv (skin-tech-api)", &root.join(".venv/bin/python3.13")),
            ],
        );
        // The SDK's home is `python3.13` of the environment.
        std::fs::write(root.join(".venv/bin/python3.13"), "").unwrap();
        let dirs = || vec![newer.clone(), older.clone()];
        let found = scan_with(&root, None, dirs);
        assert!(found.chosen);
        let selected = found.selected.clone().unwrap();
        assert_eq!(selected.python, uv);
        assert_eq!(selected.short_name(), "uv (skin-tech-api) [3.13.14]");
        assert_eq!(selected.long_name(), "uv (skin-tech-api) [3.13.14]");
        // Other environments keep their path names.
        assert!(
            found
                .found
                .iter()
                .any(|f| f.short_name() == "venv [3.12.13]")
        );
        let text = std::fs::read_to_string(project::workspace::workspace_file(&root)).unwrap();
        for line in [
            "interpreter = \".venv/bin/python\"",
            "sdk-name = \"uv (skin-tech-api)\"",
            "sdk-home = \".venv/bin/python3.13\"",
        ] {
            assert!(text.contains(line), "{line} missing:\n{text}");
        }
        // Once: the stored choice counts from now on, and the name stays
        // with its interpreter when another one is chosen.
        store_choice(&root, &plain).unwrap();
        let found = scan_with(&root, None, || panic!("not read again"));
        assert_eq!(found.selected.unwrap().python, plain);
        assert!(
            found
                .found
                .iter()
                .any(|f| f.short_name() == "uv (skin-tech-api) [3.13.14]")
        );
    }

    #[test]
    fn nothing_is_imported_without_a_matching_sdk() {
        let (dir, root) = project("demo");
        venv(&root.join(".venv"), "version = 3.12.1\n");
        let config = dir.path().join("PyCharm2026.2");
        sdk_table(&config, &[("other", Path::new("/nowhere/bin/python"))]);
        // No `.idea`.
        let found = scan_with(&root, None, || vec![config.clone()]);
        assert!(!found.chosen);
        // An SDK the table does not have, then one whose interpreter is gone.
        idea_project_sdk(&root, "uv (demo)");
        assert!(!scan_with(&root, None, || vec![config.clone()]).chosen);
        idea_project_sdk(&root, "other");
        assert!(!scan_with(&root, None, || vec![config.clone()]).chosen);
        assert!(!root.join(".oxide").exists());
    }

    #[test]
    fn versions_from_the_files_or_the_interpreter() {
        assert_eq!(version_in("Python 3.14.6\n").as_deref(), Some("3.14.6"));
        assert_eq!(version_in("Python 3.15.0b3").as_deref(), Some("3.15.0b3"));
        assert_eq!(version_in("Python\n"), None);
        let missing = Path::new("/nowhere/bin/python");
        assert_eq!(
            full_version(Some("3.12.13"), missing).as_deref(),
            Some("3.12.13")
        );
        // Incomplete, and the interpreter cannot tell: what the files say.
        assert_eq!(full_version(Some("3.14"), missing).as_deref(), Some("3.14"));
        assert_eq!(full_version(None, missing), None);
    }

    /// uv may write `3.14` into `pyvenv.cfg`; the interpreter tells the
    /// full one. Needs a Python on `PATH`.
    #[test]
    #[ignore = "starts the Python interpreter found on PATH"]
    fn an_incomplete_version_is_asked_from_the_interpreter() {
        let python = py_env::paths::find_on_path("python3").expect("python3 on PATH");
        let version = full_version(Some("3"), &python).unwrap();
        assert!(version.split('.').count() >= 3, "{version}");
    }

    #[test]
    fn long_names_are_shortened_as_the_reference_does() {
        assert_eq!(trim_middle("abcdefghij", 7), "abc\u{2026}hij");
        assert_eq!(trim_middle("abc", 7), "abc");
        let long = "uv (a-project-with-a-really-long-name-that-goes-on-and-on)";
        assert_eq!(
            compact_name(long, Some("3.13.1"), false, SHORT_NAME, false),
            "uv (a-project-with-a\u{2026}that-goes-on-and-on) [3.13.1]"
        );
        assert_eq!(
            compact_name(long, Some("3.13.1"), false, SHORT_NAME, false)
                .chars()
                .count(),
            SHORT_NAME
        );
        assert_eq!(
            compact_name("~/work/demo/.venv", Some("3.12.1"), true, 50, false),
            ".venv [3.12.1]"
        );
        assert_eq!(
            compact_name("~/work/demo/.venv", None, true, 100, true),
            "~/work/demo/.venv"
        );
        assert_eq!(
            shorten_path("/a/very/long/path/env", 12, true),
            "/a/very\u{2026}/env"
        );
        assert_eq!(shorten_path("/a/b/python3.13", 50, false), "python3.13");
        assert_eq!(
            env_dir_of(Path::new("/p/.venv/bin/python3.13")),
            Path::new("/p/.venv")
        );
    }
}
