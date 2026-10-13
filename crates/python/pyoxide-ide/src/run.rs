//! Running Python files: the product's [`RunProvider`], the reference's
//! "Current File" run (`PythonRunConfigurationProducer`,
//! `PythonCommandLineState`) with the project's interpreter.

use std::path::Path;

use gpui::{App, AppContext as _, Task};
use py_run::{PythonRun, UvRun};
use ui::icons::IconName;
use workbench::{RunProvider, RunRequest, RunSpec};

use crate::interpreter;

/// Runs a Python file with the project's interpreter.
pub struct PythonRunProvider;

/// Extensions of the files Run accepts.
const RUNNABLE: &[&str] = &["py", "pyw"];

/// The reference's message when the configuration has no interpreter
/// (`runcfg.unittest.no_valid_sdk`).
pub const NO_INTERPRETER: &str = "Please select a valid Python interpreter";

impl RunProvider for PythonRunProvider {
    fn can_run(&self, file: &Path, _: &App) -> bool {
        file.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| RUNNABLE.contains(&e))
    }

    fn prepare(&self, request: RunRequest, cx: &mut App) -> Task<Result<RunSpec, String>> {
        // The project's interpreter; a file outside a project uses the
        // environment around its folder.
        let root = request
            .project_root
            .clone()
            .or_else(|| request.file.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        let interpreter = interpreter::for_project(&root, cx);
        cx.background_spawn(async move {
            let selected = interpreter.await.ok_or_else(|| NO_INTERPRETER.to_owned())?;
            let mut run = PythonRun::current_file(&selected.python, &request.file);
            // "Add content roots" and "Add source roots" to PYTHONPATH.
            run.content_roots = request.project_root.iter().cloned().collect();
            run.source_roots = request.source_roots.to_vec();
            // An environment of a uv project runs through `uv run`
            // (`UvRunToolProvider`).
            if selected.uv_project.is_some()
                && let Some(environment) = py_env::paths::venv_root_of(&selected.python)
            {
                run.uv = UvRun::locate(environment);
            }
            let title = request
                .file
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(RunSpec {
                title,
                icon: IconName("python/file"),
                command: run.command_line(),
                display: run.display(),
                link_filters: run.link_filters(),
            })
        })
    }
}
