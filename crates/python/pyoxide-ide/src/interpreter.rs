//! The project's Python interpreter: the one runs use, and the interpreter
//! widget in the status bar.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, AppContext as _, Context, IntoElement, Render, Task, Window};
use workbench::StatusWidget;

/// The interpreter chosen for a project.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedInterpreter {
    /// The executable to run.
    pub python: PathBuf,
    pub kind: py_env::InterpreterKind,
    /// `3.13.1`.
    pub version: String,
    /// The text of the interpreter widget.
    pub label: String,
    /// The uv project the environment belongs to (`[project] name` of a
    /// project managed by uv), when it is one.
    pub uv_project: Option<String>,
}

/// The interpreter of the project at `root`: its virtual environment
/// (`UV_PROJECT_ENVIRONMENT`, then `.venv` or `venv` in the project or an
/// ancestor). Looked up off the UI thread.
pub fn for_project(root: &Path, cx: &App) -> Task<Option<SelectedInterpreter>> {
    let root = root.to_owned();
    cx.background_spawn(async move { project_environment(&root) })
}

fn project_environment(root: &Path) -> Option<SelectedInterpreter> {
    let env = std::env::var_os("UV_PROJECT_ENVIRONMENT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let venv = py_env::project::find_project_venv(root, env.as_deref())?;
    let python = py_env::paths::environment_python(&venv)?;
    let cfg = std::fs::read_to_string(venv.join("pyvenv.cfg")).unwrap_or_default();
    let value = |key: &str| {
        cfg.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().to_owned())
        })
    };
    let version = value("version_info")
        .or_else(|| value("version"))
        .unwrap_or_default();
    let uv_project = value("uv")
        .and_then(|_| py_env::load_project(root).ok())
        .and_then(|p| p.pyproject.and_then(|p| p.name));
    let short: String = version.split('.').take(2).collect::<Vec<_>>().join(".");
    let owner = venv
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = match &uv_project {
        Some(name) => format!("uv ({name})"),
        None => format!("Python {short} ({owner})"),
    };
    Some(SelectedInterpreter {
        python,
        kind: py_env::InterpreterKind::Venv,
        version,
        label,
        uv_project,
    })
}

/// The interpreter widget of the status bar.
pub fn status_widget() -> StatusWidget {
    StatusWidget {
        id: "status-interpreter",
        create: Rc::new(|_, _, cx| cx.new(|_| InterpreterWidget).into()),
    }
}

struct InterpreterWidget;

impl Render for InterpreterWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        workbench::chrome::status_widget("status-interpreter", "No interpreter".into(), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_environment_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(project_environment(root), None);
        let venv = root.join(".venv");
        let python = py_env::paths::venv_python(&venv);
        std::fs::create_dir_all(python.parent().unwrap()).unwrap();
        std::fs::write(&python, "").unwrap();
        std::fs::write(
            venv.join("pyvenv.cfg"),
            "home = /usr/bin\nversion_info = 3.13.1\n",
        )
        .unwrap();
        let found = project_environment(root).unwrap();
        assert_eq!(found.python, python);
        assert_eq!(found.version, "3.13.1");
        assert_eq!(found.uv_project, None);
    }
}
