//! Adding an interpreter to the list (`python.interpreter.add`), as the
//! reference IDE's Add Python Interpreter dialog does on OK
//! (`PythonAddCustomInterpreter`): an existing interpreter (Select
//! existing / Python), an existing uv environment (Select existing / uv).
//! The work runs on the interpreter runtime while the project is marked as
//! being configured; the new interpreter is added to the application's
//! list and selected: at once for the project (the status bar's way), or
//! in the Interpreter page's combo until Apply.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Task};
use py_env::RunContext;
use py_env::venv::{EnvDir, EnvLayout};

use super::table::{Change, Entry, Flavor, Table};
use super::{Choice, configuring, list, runtime};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Add an interpreter.
    AddInterpreter {
        kind: String,
        project: Option<String>,
        python: Option<String>,
        uv_path: Option<String>,
        select: Option<String>
    } => "python.interpreter.add", "Add Python Interpreter",
        "Adds an interpreter to the application's list of interpreters, as OK in the Add Python Interpreter dialog does, off the UI thread while the project is marked as being configured: `kind` `existing-python` (an interpreter, or the environment folder holding it, at `python`) or `existing-uv` (a uv environment at `python`, with the uv at `uv_path` or the one found). An interpreter already listed is reused; a virtual environment is associated with the project. `select`: `now` makes it the project's interpreter (the status bar's way), `draft` selects it in the Interpreter page until Apply (the default while that page is open). Relative paths are resolved against the project (`project`, else the active window's).",
        r#"{"kind": "existing-python", "python": ".venv"}"#;
}

/// The reference's title of a failure balloon
/// (`python.sdk.failed.to.create.interpreter.title`) and its message when
/// uv is missing (`sdk.cannot.find.uv.executable`).
pub const FAILED_TITLE: &str = "Failed to create interpreter";
pub const NO_UV: &str = "Cannot find uv executable";
/// How long probing an interpreter may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// What is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Select existing / Python.
    ExistingPython,
    /// Select existing / uv.
    ExistingUv,
}

impl Kind {
    pub(crate) fn parse(id: &str) -> Result<Kind, String> {
        match id {
            "existing-python" => Ok(Kind::ExistingPython),
            "existing-uv" => Ok(Kind::ExistingUv),
            "virtualenv" | "uv" => Err(format!(
                "creating a `{id}` environment is not available yet"
            )),
            _ => Err(format!(
                "`kind` takes `existing-python` or `existing-uv`, not `{id}`"
            )),
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
    pub python: Option<PathBuf>,
    pub uv: Option<PathBuf>,
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
    if entry.version.is_none() {
        entry.version = py_env::interpreter::probe(&python, "uv", PROBE_TIMEOUT, &ctx)
            .await
            .ok()
            .map(|p| p.version);
    }
    entry.uv = Some(uv.program().to_owned());
    entry.working_dir = Some(root);
    Ok(entry)
}

/// Adds the interpreter `request` asks for: the project is marked as
/// being configured meanwhile; the entry goes into the list, and is
/// selected as `select` says. Resolves to the new interpreter's name.
pub(crate) fn add(request: Request, select: Select, cx: &mut App) -> Task<Result<String, String>> {
    let root = request.root.clone();
    if !configuring::begin(&root, cx) {
        return Task::ready(Err(configuring::BUSY_TOOLTIP.to_owned()));
    }
    let table = list::table(cx);
    let work = runtime::spawn(cx, move |token| {
        let ctx = RunContext::default().with_cancel(token);
        async move {
            let python = request
                .python
                .clone()
                .ok_or("name the interpreter or environment with `python`")?;
            match request.kind {
                Kind::ExistingPython => existing_python(request.root, python, table, ctx).await,
                Kind::ExistingUv => existing_uv(request.root, python, request.uv, ctx).await,
            }
        }
    });
    cx.spawn(async move |cx| {
        let result = async {
            let entry = work.await?;
            let python = entry.python.clone();
            let written = cx
                .update(|cx| list::change(vec![Change::Add(entry)], cx))
                .map_err(|e| e.to_string())?
                .await?;
            let name = written
                .find_python(&python)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            cx.update(|cx| match select {
                Select::Now => super::choose(&root, Choice::Python(python), cx).detach(),
                Select::Draft => super::page::select_draft(&root, &python, cx),
            })
            .map_err(|e| e.to_string())?;
            Ok(name)
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
        assert!(Kind::parse("uv").unwrap_err().contains("not available yet"));
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
