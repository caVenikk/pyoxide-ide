//! Running Python: the product's [`RunProvider`], the reference's
//! "Current File" run (`PythonRunConfigurationProducer`,
//! `PythonCommandLineState`) with the project's interpreter, and the
//! Python run configuration type (ADR 0012) with its import from the
//! reference IDE.

use std::path::Path;
use std::sync::Arc;

use gpui::{App, AppContext as _, Task};
use py_run::config::{LaunchContext, PythonEditor, PythonImporter, PythonOptions};
use py_run::{PythonRun, UvRun};
use run_config::{Importer, RunConfiguration, TypeEditor, TypeInfo, paths};
use ui::icons::IconName;
use workbench::{FieldText, RunProvider, RunRequest, RunSpec};

use crate::interpreter::{self, ConfiguredInterpreter};

/// Runs Python files and configurations with the project's interpreter.
pub struct PythonRunProvider;

/// Extensions of the files Run accepts.
const RUNNABLE: &[&str] = &["py", "pyw"];

/// The reference's message when the configuration has no interpreter
/// (`runcfg.unittest.no_valid_sdk`).
pub const NO_INTERPRETER: &str = "Please select a valid Python interpreter";

/// The icon of Python runs and of the Python configuration type.
const PYTHON_ICON: IconName = IconName("python/file");

/// A file Run accepts.
fn runnable(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RUNNABLE.contains(&e))
}

impl RunProvider for PythonRunProvider {
    fn can_run(&self, file: &Path, _: &App) -> bool {
        runnable(file)
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
            Ok(RunSpec::new(
                title,
                PYTHON_ICON,
                run.command_line(),
                run.display(),
                run.link_filters(),
            ))
        })
    }

    fn configuration_types(&self) -> Vec<TypeInfo> {
        vec![py_run::config::TYPE]
    }

    fn importers(&self) -> Vec<Arc<dyn Importer>> {
        vec![Arc::new(PythonImporter)]
    }

    fn editors(&self) -> Vec<Arc<dyn TypeEditor>> {
        vec![Arc::new(PythonEditor)]
    }

    fn context_configuration(&self, file: &Path, root: &Path) -> Option<RunConfiguration> {
        runnable(file)
            .then(|| py_run::config::context_configuration(file, root))
            .flatten()
    }

    fn same_context(&self, existing: &RunConfiguration, context: &RunConfiguration) -> bool {
        py_run::config::same_context(existing, context)
    }

    fn unsupported_details(&self, config: &RunConfiguration) -> Vec<(String, String)> {
        py_run::config::unsupported_details(config)
    }

    /// The interpreter a Python configuration runs with, as the reference's
    /// interpreter combo shows it: name, version, path.
    fn describe_field(
        &self,
        config: &RunConfiguration,
        field: &str,
        root: &Path,
        cx: &mut App,
    ) -> Option<Task<Option<FieldText>>> {
        if config.type_id != py_run::config::TYPE_ID || field != py_run::config::field::INTERPRETER
        {
            return None;
        }
        let options = PythonOptions::read(config);
        let configured = (!options.use_project_interpreter).then(|| ConfiguredInterpreter {
            name: options.interpreter_name.clone(),
            path: paths::resolve(&options.interpreter, root),
        });
        let interpreter = interpreter::for_run(root, configured, cx);
        Some(cx.background_spawn(async move {
            let (selected, _) = interpreter.await;
            selected.map(|s| FieldText {
                icon: Some(s.icon),
                text: s.name,
                detail: format!("{} {}", s.version, s.python.display())
                    .trim()
                    .to_owned(),
            })
        }))
    }

    /// A Python configuration: its interpreter (the project's, or the one
    /// it names, falling back to the project's with a warning), `uv run`
    /// for an environment of a uv project unless the configuration turns
    /// it off, then the run it describes.
    fn prepare_configuration(
        &self,
        request: RunRequest,
        cx: &mut App,
    ) -> Task<Result<RunSpec, String>> {
        let Some(config) = request.configuration.clone() else {
            return Task::ready(Err("No run configuration".to_owned()));
        };
        if config.type_id != py_run::config::TYPE_ID {
            return Task::ready(Err(format!(
                "'{}' cannot run: its type is not supported yet",
                config.name
            )));
        }
        let Some(root) = request.project_root.clone() else {
            return Task::ready(Err("No project is open".to_owned()));
        };
        let options = PythonOptions::read(&config);
        let configured = (!options.use_project_interpreter).then(|| ConfiguredInterpreter {
            name: options.interpreter_name.clone(),
            path: paths::resolve(&options.interpreter, &root),
        });
        let interpreter = interpreter::for_run(&root, configured, cx);
        cx.background_spawn(async move {
            let (selected, missing) = interpreter.await;
            let selected = selected.ok_or_else(|| NO_INTERPRETER.to_owned())?;
            // Until interpreters are configured in the IDE (#75): the one
            // the configuration names is replaced, and the message says by
            // which.
            let warning = missing.map(|what| {
                format!(
                    "Python interpreter '{what}' of '{}' was not found; running with the project interpreter {} ({})",
                    config.name,
                    selected.label,
                    selected.python.display()
                )
            });
            let uv = if selected.uv_project.is_some() && py_run::config::runs_with_uv(&config) {
                py_env::paths::venv_root_of(&selected.python).and_then(UvRun::locate)
            } else {
                None
            };
            let cx = LaunchContext {
                root: &root,
                source_roots: &request.source_roots,
                python: selected.python.clone(),
                uv,
            };
            let launch = py_run::config::launch(&config, &cx)?;
            let mut spec = RunSpec::new(
                config.name.clone(),
                PYTHON_ICON,
                launch.run.command_line(),
                launch.run.display(),
                launch.run.link_filters(),
            );
            spec.input_file = launch.input_file;
            spec.warning = warning;
            Ok(spec)
        })
    }
}
