//! Adding an interpreter to the list (`python.interpreter.add`), as the
//! reference IDE's Add Python Interpreter dialog does on OK
//! (`PythonAddCustomInterpreter`): a new virtual environment (Generate new
//! / Virtualenv), a new uv environment (Generate new / uv), an existing
//! interpreter (Select existing / Python), an existing uv environment
//! (Select existing / uv). The work runs on the interpreter runtime while
//! the project is marked as being configured; the new interpreter is
//! added to the application's list and selected: at once for the project
//! (the status bar's way), or in the Interpreter page's combo until Apply.
//!
//! A virtual environment is made with uv when there is one (`uv venv
//! --python <base> --seed`), else with the base interpreter's `venv`
//! module; a uv environment as the reference makes it
//! (`UvLowLevel.initializeEnvironment`): `uv init --bare --no-project` in a
//! project without `pyproject.toml`, then `uv venv` (again with `--force`
//! when it fails), then `uv sync` in a project that has one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Task};
use py_env::RunContext;
use py_env::uv::{InitKind, InitRequest, SyncRequest, VenvRequest};
use py_env::venv::{EnvDir, EnvLayout, Location};

use super::table::{Change, Entry, Flavor, Preferred, Table};
use super::{Choice, candidates, configuring, list, runtime};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Add an interpreter.
    AddInterpreter {
        kind: String,
        project: Option<String>,
        python: Option<String>,
        base_python: Option<String>,
        location: Option<String>,
        python_version: Option<String>,
        uv_path: Option<String>,
        inherit_site_packages: Option<bool>,
        available_to_all: Option<bool>,
        override_existing: Option<bool>,
        install_python: Option<String>,
        select: Option<String>
    } => "python.interpreter.add", "Add Python Interpreter",
        "Adds an interpreter to the application's list of interpreters, as OK in the Add Python Interpreter dialog does, off the UI thread while the project is marked as being configured. `kind`: `virtualenv` (a new virtual environment at `location`, default the suggested one, made from the interpreter at `base_python`; `inherit_site_packages`, `available_to_all`), `uv` (a new uv environment at `location` with the Python `python_version` (`3.13`, default uv's) and the uv at `uv_path` or the one found; `override_existing` replaces an environment that is there), `existing-python` (an interpreter, or the environment folder holding it, at `python`) or `existing-uv` (a uv environment at `python`). `install_python` (`3.13.5`, `3.13`) first downloads and installs that Python with `uv python install` (a new virtual environment is then based on it). An interpreter already listed is reused; a new or found environment is associated with the project. `select`: `now` makes it the project's interpreter (the status bar's way), `draft` selects it in the Interpreter page until Apply (the default while that page is open). Relative paths are resolved against the project (`project`, else the active window's).",
        r#"{"kind": "uv", "location": ".venv", "python_version": "3.13"}"#;
}

/// The reference's title of a failure balloon
/// (`python.sdk.failed.to.create.interpreter.title`) and its message when
/// uv is missing (`sdk.cannot.find.uv.executable`).
pub const FAILED_TITLE: &str = "Failed to create interpreter";
pub const NO_UV: &str = "Cannot find uv executable";
/// What is wrong with the location of a new environment (PyBundle).
pub const NOT_ABSOLUTE: &str = "Path must be absolute";
pub const FOLDER_EXISTS: &str = "The specified folder already exists";
/// A virtual environment could not be made (`project.error.cant.venv`).
pub const CANT_VENV: &str = "Could not create a virtual environment";
/// uv made no environment (`python.sdk.uv.failed.to.initialize.uv.environment`).
const UV_NO_ENVIRONMENT: &str = "Failed to initialize uv environment";
/// The progress texts (`python.sdk.progress.*`).
pub const CREATING_VENV: &str = "Creating virtual environment";
pub const CREATING_UV: &str = "Creating uv environment";
pub const CONFIGURING_UV: &str = "Configuring uv environment";

/// The progress text of a Python being installed (`python.sdk.installing`
/// of the installer: "Installing {0}" with the Python's name).
pub fn installing(version: &str) -> String {
    format!("Installing Python {version}")
}

/// `python.add.sdk.already.contains.python.with.version`.
pub fn already_contains(level: &str) -> String {
    format!("Already contains Python with version {level}")
}

/// `sdk.create.custom.override.error`.
pub fn override_error(name: &str) -> String {
    format!("Environment at \"{name}\" already exists")
}

/// `sdk.create.custom.override.warning`.
pub fn override_warning(name: &str) -> String {
    format!("Existing environment at \"{name}\" will be overridden")
}

/// `sdk.create.custom.venv.unsupported.base.version` (two lines).
pub fn unsupported_base() -> String {
    let (major, minor) = py_env::interpreter::OLDEST_FOR_NEW_ENVIRONMENTS;
    format!(
        "New virtual environments require Python {major}.{minor}+.\nFor older versions, select an existing environment from disk."
    )
}

/// `sdk.create.existing.unsupported.python.management.warning` (two lines).
pub fn unmanaged_python_warning() -> String {
    let (major, minor) = py_env::interpreter::OLDEST_FOR_NEW_ENVIRONMENTS;
    format!(
        "Make sure pip and setuptools are installed.\nThe IDE only provides them for Python {major}.{minor} and newer."
    )
}
/// How long probing an interpreter may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// What is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Generate new / Virtualenv.
    Virtualenv,
    /// Generate new / uv.
    Uv,
    /// Select existing / Python.
    ExistingPython,
    /// Select existing / uv.
    ExistingUv,
}

impl Kind {
    pub(crate) fn parse(id: &str) -> Result<Kind, String> {
        match id {
            "virtualenv" => Ok(Kind::Virtualenv),
            "uv" => Ok(Kind::Uv),
            "existing-python" => Ok(Kind::ExistingPython),
            "existing-uv" => Ok(Kind::ExistingUv),
            _ => Err(format!(
                "`kind` takes `virtualenv`, `uv`, `existing-python` or `existing-uv`, not `{id}`"
            )),
        }
    }

    /// The text of the progress while it is added.
    fn progress(self) -> &'static str {
        match self {
            Kind::Virtualenv => CREATING_VENV,
            Kind::Uv => CREATING_UV,
            Kind::ExistingPython | Kind::ExistingUv => CONFIGURING_UV,
        }
    }
}

/// How the new interpreter is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Select {
    /// The project's interpreter at once.
    Now,
    /// In the Interpreter page's combo, until Apply.
    Draft,
    /// In the open Python Interpreters dialog (its Add button).
    Dialog,
}

impl Select {
    pub(crate) fn parse(id: &str) -> Result<Select, String> {
        match id {
            "now" => Ok(Select::Now),
            "draft" => Ok(Select::Draft),
            _ => Err(format!("`select` takes `now` or `draft`, not `{id}`")),
        }
    }
}

/// An add request with its paths resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub kind: Kind,
    pub root: PathBuf,
    /// The interpreter (Select existing), or the base of a new virtual
    /// environment.
    pub python: Option<PathBuf>,
    pub uv: Option<PathBuf>,
    /// uv was looked for already (the dialog): without `uv` there is none,
    /// a new virtual environment comes from the base's `venv` module.
    pub uv_located: bool,
    /// Where a new environment goes; `None`: the suggested location.
    pub location: Option<PathBuf>,
    /// The Python of a new uv environment (`3.13`); `None`: uv's choice.
    pub python_version: Option<String>,
    pub inherit_site_packages: bool,
    pub available_to_all: bool,
    /// Replace the uv environment at the location.
    pub override_existing: bool,
    /// The Python uv downloads and installs first (`3.13.5`; a new
    /// virtual environment is based on it, a new uv environment uses it).
    pub install_python: Option<String>,
}

impl Request {
    pub fn new(kind: Kind, root: PathBuf) -> Self {
        Self {
            kind,
            root,
            python: None,
            uv: None,
            uv_located: false,
            location: None,
            python_version: None,
            inherit_site_packages: false,
            available_to_all: false,
            override_existing: false,
            install_python: None,
        }
    }
}

/// What was found at the location of a new environment
/// (`FileSystem.validateVenv`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocationCheck {
    Free,
    NotAbsolute,
    /// A folder (or file) without a working interpreter.
    Exists,
    /// An environment: its interpreter, `3.13` and the folder's name.
    HasPython {
        python: PathBuf,
        level: String,
        name: String,
    },
}

impl LocationCheck {
    /// The message of a problem as a field shows it; an environment at
    /// the location is "Already contains Python with version …".
    pub fn message(&self) -> Option<String> {
        match self {
            LocationCheck::Free => None,
            LocationCheck::NotAbsolute => Some(NOT_ABSOLUTE.to_owned()),
            LocationCheck::Exists => Some(FOLDER_EXISTS.to_owned()),
            LocationCheck::HasPython { level, .. } => Some(already_contains(level)),
        }
    }
}

/// Checks the location of a new environment; an environment whose files
/// do not tell its version is asked (a broken one counts as a folder).
pub(crate) async fn check_location(path: &Path, ctx: &RunContext) -> LocationCheck {
    match py_env::venv::check_location(path) {
        Location::Free => LocationCheck::Free,
        Location::NotAbsolute => LocationCheck::NotAbsolute,
        Location::Exists => LocationCheck::Exists,
        Location::HasPython(env) => {
            let level = match env
                .version
                .as_deref()
                .and_then(py_env::interpreter::major_minor)
            {
                Some((major, minor)) => Some(format!("{major}.{minor}")),
                None => py_env::interpreter::probe(&env.python, "manual", PROBE_TIMEOUT, ctx)
                    .await
                    .ok()
                    .map(|i| format!("{}.{}", i.major, i.minor)),
            };
            match level {
                Some(level) => LocationCheck::HasPython {
                    name: env
                        .dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| py_env::project::VENV_DIR_NAMES[0].to_owned()),
                    python: env.python,
                    level,
                },
                None => LocationCheck::Exists,
            }
        }
    }
}

/// The uv at `path` (else the one found), which must run: its version.
pub(crate) async fn uv_that_runs(
    path: Option<PathBuf>,
    ctx: &RunContext,
) -> Result<(py_env::Uv, String), String> {
    let uv = match path {
        Some(path) => py_env::Uv::new(path, py_env::UvConfig::default()),
        None => py_env::Uv::locate(py_env::UvConfig::default()).ok_or(NO_UV)?,
    };
    let version = uv.version(ctx).await.map_err(|_| NO_UV.to_owned())?;
    Ok((uv, version))
}

/// A project name as `uv init --name` takes it (`normalizeProjectName`,
/// PEP 503): lower case, runs of `-`, `_` and `.` as one `-`.
pub(crate) fn normalized_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut separator = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            separator = true;
            continue;
        }
        if separator && !out.is_empty() {
            out.push('-');
        }
        separator = false;
        out.extend(c.to_lowercase());
    }
    out
}

/// A new interpreter, and a problem worth a balloon that did not stop it
/// (`uv sync` failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Created {
    pub entry: Entry,
    pub warning: Option<String>,
}

/// The interpreter of a path the person gave: the file, or the
/// interpreter of the environment folder.
fn interpreter_at(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_owned());
    }
    py_env::paths::environment_python(path)
        .ok_or_else(|| format!("{} is not a Python interpreter", path.display()))
}

/// The entry for an interpreter of the environment `env` (or outside one):
/// named after its path, associated with the project when it is a virtual
/// environment.
fn entry_for(root: &Path, python: &Path, env: Option<&EnvDir>, flavor: Flavor) -> Entry {
    let name = match env {
        Some(env) => super::sdk_name(env),
        None => project::display_path(python),
    };
    let mut entry = Entry::new(name, python, flavor);
    entry.version = env.and_then(|e| e.version.clone());
    if flavor != Flavor::System {
        entry.associated_project = Some(root.to_owned());
    }
    entry
}

/// Select existing / Python: the interpreter at `python` (already listed:
/// reused), probed for its version.
pub(crate) async fn existing_python(
    root: PathBuf,
    python: PathBuf,
    table: Arc<Table>,
    ctx: RunContext,
) -> Result<Entry, String> {
    let python = interpreter_at(&python)?;
    if let Some(entry) = table.find_python(&python) {
        return Ok(entry.clone());
    }
    let probed = py_env::interpreter::probe(&python, "manual", PROBE_TIMEOUT, &ctx)
        .await
        .map_err(|e| e.to_string())?;
    let env = py_env::venv::inspect_python(&python);
    let flavor = match &env {
        Some(env) if env.is_uv() => Flavor::Uv,
        Some(env) => match env.layout {
            EnvLayout::Venv => Flavor::Venv,
            EnvLayout::Conda => Flavor::Conda,
            EnvLayout::Installation => Flavor::System,
        },
        None => Flavor::System,
    };
    let mut entry = entry_for(&root, &python, env.as_ref(), flavor);
    entry.version = Some(probed.version);
    Ok(entry)
}

/// Select existing / uv: the uv environment at `env` (a folder or its
/// interpreter) with the uv at `uv` (else the one found), which must run.
pub(crate) async fn existing_uv(
    root: PathBuf,
    env: PathBuf,
    uv: Option<PathBuf>,
    ctx: RunContext,
) -> Result<Entry, String> {
    let python = interpreter_at(&env)?;
    let uv = match uv {
        Some(uv) => py_env::Uv::new(uv, py_env::UvConfig::default()),
        None => py_env::Uv::locate(py_env::UvConfig::default()).ok_or(NO_UV)?,
    };
    uv.version(&ctx).await.map_err(|_| NO_UV.to_owned())?;
    let env = py_env::venv::inspect_python(&python);
    let mut entry = entry_for(&root, &python, env.as_ref(), Flavor::Uv);
    // uv may write an incomplete version (`3.14`): ask the interpreter.
    if entry
        .version
        .as_deref()
        .is_none_or(|v| v.split('.').count() < 3)
        && let Ok(probed) = py_env::interpreter::probe(&python, "uv", PROBE_TIMEOUT, &ctx).await
    {
        entry.version = Some(probed.version);
    }
    entry.uv = Some(uv.program().to_owned());
    entry.working_dir = Some(root);
    Ok(entry)
}

/// The location of a new environment: the one asked for, else the
/// suggested one.
fn location_of(request: &Request, table: &Table) -> PathBuf {
    request.location.clone().unwrap_or_else(|| {
        candidates::suggested_location(&request.root, table.preferred.venv_base_path.as_deref())
    })
}

/// Generate new / Virtualenv: a virtual environment at the location, made
/// from the base interpreter (Python 3.8+), with pip; associated with the
/// project unless it is for all projects.
pub(crate) async fn create_virtualenv(
    request: Request,
    table: Arc<Table>,
    ctx: RunContext,
) -> Result<Created, String> {
    let base = request
        .python
        .clone()
        .ok_or("choose the base interpreter with `base_python`")?;
    let base = interpreter_at(&base)?;
    let location = location_of(&request, &table);
    if let Some(problem) = check_location(&location, &ctx).await.message() {
        return Err(problem);
    }
    let probed = py_env::interpreter::probe(&base, "manual", PROBE_TIMEOUT, &ctx)
        .await
        .map_err(|e| e.to_string())?;
    if !py_env::interpreter::supports_new_environments(probed.major, probed.minor) {
        return Err(unsupported_base().replace('\n', " "));
    }
    let uv = match request.uv.clone() {
        Some(path) => Some(py_env::Uv::new(path, py_env::UvConfig::default())),
        None if request.uv_located => None,
        None => py_env::Uv::locate(py_env::UvConfig::default()),
    };
    let venv = VenvRequest {
        path: location.clone(),
        python: Some(base.to_string_lossy().into_owned()),
        seed: true,
        system_site_packages: request.inherit_site_packages,
        cwd: Some(request.root.clone()),
        ..VenvRequest::default()
    };
    let created = py_env::ops::venv_create_with(uv.as_ref(), venv, &ctx)
        .await
        .map_err(|e| format!("{CANT_VENV}: {e}"))?;
    let env = py_env::venv::inspect_python(&created.python);
    let mut entry = entry_for(&request.root, &created.python, env.as_ref(), Flavor::Venv);
    if let Some(interpreter) = &created.interpreter {
        entry.version = Some(interpreter.version.clone());
    }
    if request.available_to_all {
        entry.associated_project = None;
    }
    Ok(Created {
        entry,
        warning: None,
    })
}

/// Generate new / uv: `uv init --bare --no-project` when the project has
/// no `pyproject.toml`, `uv venv` at the location (`--clear` to override
/// one that is there; again with `--force` when it fails), `uv sync` when
/// it has one (a failure is only reported).
pub(crate) async fn create_uv(
    request: Request,
    table: Arc<Table>,
    ctx: RunContext,
) -> Result<Created, String> {
    let (uv, _) = uv_that_runs(request.uv.clone(), &ctx).await?;
    let root = request.root.clone();
    let location = location_of(&request, &table);
    match check_location(&location, &ctx).await {
        LocationCheck::Free => {}
        LocationCheck::HasPython { .. } if request.override_existing => {}
        LocationCheck::HasPython { name, .. } => return Err(override_error(&name)),
        problem => return Err(problem.message().unwrap_or_default()),
    }
    let version = request
        .python_version
        .clone()
        .filter(|v| !v.trim().is_empty());
    if let Some(v) = &version
        && py_env::interpreter::major_minor(v).is_none()
    {
        return Err(format!(
            "`python_version` takes a version like `3.13`, not `{v}`"
        ));
    }
    let init = !root.join("pyproject.toml").exists();
    if init {
        let name = root
            .file_name()
            .map(|n| normalized_name(&n.to_string_lossy()))
            .filter(|n| !n.is_empty());
        let request = InitRequest {
            name,
            kind: InitKind::Bare,
            python: version.clone(),
            no_project: true,
            // No `--vcs`: uv's own default, as the reference leaves it.
            vcs: true,
            cwd: Some(root.clone()),
            ..InitRequest::default()
        };
        uv.init(&request, &ctx).await.map_err(|e| e.to_string())?;
    }
    let mut venv = VenvRequest {
        path: location.clone(),
        python: version.clone(),
        clear: request.override_existing,
        cwd: Some(root.clone()),
        ..VenvRequest::default()
    };
    if uv.venv(&venv, &ctx).await.is_err() {
        venv.force = true;
        uv.venv(&venv, &ctx).await.map_err(|e| e.to_string())?;
    }
    let python = py_env::paths::venv_python(&location);
    if !python.is_file() {
        return Err(UV_NO_ENVIRONMENT.to_owned());
    }
    let warning = if init {
        None
    } else {
        let sync = SyncRequest {
            project: root.clone(),
            environment: Some(location.clone()),
            ..SyncRequest::default()
        };
        uv.sync(&sync, &ctx).await.err().map(|e| e.to_string())
    };
    let env = py_env::venv::inspect_python(&python);
    let mut entry = entry_for(&root, &python, env.as_ref(), Flavor::Uv);
    if entry
        .version
        .as_deref()
        .is_none_or(|v| v.split('.').count() < 3)
        && let Ok(probed) = py_env::interpreter::probe(&python, "uv", PROBE_TIMEOUT, &ctx).await
    {
        entry.version = Some(probed.version);
    }
    entry.uv = Some(uv.program().to_owned());
    entry.working_dir = Some(root);
    Ok(Created { entry, warning })
}

/// Downloads and installs the Python `version` with uv (`uv python
/// install`) and, when `find`, finds its executable (`uv python find`).
pub(crate) async fn install_python(
    uv: Option<PathBuf>,
    version: &str,
    find: bool,
    ctx: &RunContext,
) -> Result<Option<PathBuf>, String> {
    let (uv, _) = uv_that_runs(uv, ctx).await?;
    uv.python_install(&[version.to_owned()], false, ctx)
        .await
        .map_err(|e| e.to_string())?;
    if !find {
        return Ok(None);
    }
    uv.python_find(version, ctx)
        .await
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Adds the interpreter `request` asks for: the project is marked as
/// being configured meanwhile; the entry goes into the list, and is
/// selected as `select` says. Resolves to the new interpreter's name and
/// a problem that did not stop it.
pub(crate) fn add(
    request: Request,
    select: Select,
    cx: &mut App,
) -> Task<Result<(String, Option<String>), String>> {
    let root = request.root.clone();
    if !configuring::begin(&root, cx) {
        return Task::ready(Err(configuring::BUSY_TOOLTIP.to_owned()));
    }
    let kind = request.kind;
    let first = match &request.install_python {
        Some(version) => installing(version),
        None => kind.progress().to_owned(),
    };
    configuring::set_progress(&root, Some(first), cx);
    cx.spawn(async move |cx| {
        let result = async {
            let mut request = request;
            // A Python to download first: a new virtual environment is
            // based on it (`installBaseSdk`).
            if let Some(version) = request.install_python.take() {
                let uv = request.uv.clone();
                let install = cx
                    .update(|cx| {
                        runtime::spawn(cx, move |token| async move {
                            let ctx = RunContext::default().with_cancel(token);
                            // A new uv environment names the version.
                            install_python(uv, &version, kind == Kind::Virtualenv, &ctx).await
                        })
                    })
                    .map_err(|e| e.to_string())?;
                if let Some(python) = install.await? {
                    request.python = Some(python);
                }
                cx.update(|cx| configuring::set_progress(&root, Some(kind.progress().into()), cx))
                    .map_err(|e| e.to_string())?;
            }
            let base = request.python.clone();
            let table = cx.update(|cx| list::table(cx)).map_err(|e| e.to_string())?;
            let work = cx
                .update(|cx| {
                    runtime::spawn(cx, move |token| {
                        let ctx = RunContext::default().with_cancel(token);
                        async move {
                            let existing = || {
                                request
                                    .python
                                    .clone()
                                    .ok_or("name the interpreter or environment with `python`")
                            };
                            let plain = |entry: Entry| Created {
                                entry,
                                warning: None,
                            };
                            match request.kind {
                                Kind::Virtualenv => create_virtualenv(request, table, ctx).await,
                                Kind::Uv => create_uv(request, table, ctx).await,
                                Kind::ExistingPython => {
                                    let python = existing()?;
                                    existing_python(request.root, python, table, ctx)
                                        .await
                                        .map(plain)
                                }
                                Kind::ExistingUv => {
                                    let python = existing()?;
                                    existing_uv(request.root, python, request.uv, ctx)
                                        .await
                                        .map(plain)
                                }
                            }
                        }
                    })
                })
                .map_err(|e| e.to_string())?;
            let Created { entry, warning } = work.await?;
            let python = entry.python.clone();
            let mut changes = vec![Change::Add(entry)];
            // The base of the next virtual environment
            // (`preferredVirtualEnvBaseSdk`).
            if kind == Kind::Virtualenv {
                let preferred = cx
                    .update(|cx| list::table(cx).preferred.clone())
                    .map_err(|e| e.to_string())?;
                changes.push(Change::SetPreferred(Preferred {
                    base_python: base,
                    ..preferred
                }));
            }
            let written = cx
                .update(|cx| list::change(changes, cx))
                .map_err(|e| e.to_string())?
                .await?;
            let name = written
                .find_python(&python)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            cx.update(|cx| match select {
                Select::Now => super::choose(&root, Choice::Python(python), cx).detach(),
                Select::Draft => super::page::select_draft(&root, &python, cx),
                Select::Dialog => super::list_dialog::select_new(&root, &name, cx),
            })
            .map_err(|e| e.to_string())?;
            Ok((name, warning))
        }
        .await;
        let _ = cx.update(|cx| configuring::end(&root, cx));
        result
    })
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
    fn kinds_and_selections() {
        assert_eq!(Kind::parse("existing-uv"), Ok(Kind::ExistingUv));
        assert_eq!(Kind::parse("uv"), Ok(Kind::Uv));
        assert_eq!(Kind::parse("virtualenv"), Ok(Kind::Virtualenv));
        assert!(Kind::parse("conda").is_err());
        assert_eq!(Select::parse("draft"), Ok(Select::Draft));
        assert!(Select::parse("later").is_err());
    }

    #[test]
    fn entries_are_named_and_associated_as_the_reference_does() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let python = venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.14\n");
        let env = py_env::venv::inspect_python(&python).unwrap();
        let entry = entry_for(&root, &python, Some(&env), Flavor::Uv);
        assert_eq!(entry.name, project::display_path(&root.join(".venv")));
        assert_eq!(entry.version.as_deref(), Some("3.13.14"));
        assert_eq!(entry.associated_project.as_deref(), Some(root.as_path()));
        // An installation is offered to every project.
        let system = entry_for(&root, Path::new("/usr/bin/python3"), None, Flavor::System);
        assert_eq!(system.associated_project, None);
        assert_eq!(interpreter_at(&root.join(".venv")), Ok(python.clone()));
        assert!(interpreter_at(&root.join("nothing")).is_err());
    }

    /// A listed interpreter is reused without starting it.
    #[test]
    fn a_listed_interpreter_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let python = venv(&root.join("env"), "version = 3.12.1\n");
        let mut table = Table::default();
        table
            .apply(&Change::Add(Entry::new("mine", &python, Flavor::Venv)))
            .unwrap();
        let entry = futures_lite_block_on(existing_python(
            root.clone(),
            root.join("env"),
            Arc::new(table),
            RunContext::default(),
        ))
        .unwrap();
        assert_eq!(entry.name, "mine");
    }

    fn futures_lite_block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }
}
