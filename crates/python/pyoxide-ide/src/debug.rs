//! Debugging Python with debugpy (ADR 0015): the product's debug hook. A
//! debug run's interpreter is picked on the UI thread as a run's is (the
//! project's, or the one the configuration names), with `uv run`'s
//! environment for a uv project; the provider then builds the adapter and
//! its `launch` from it off the UI thread.

use std::path::PathBuf;
use std::sync::Arc;

use debugger::DebugRequest;
use gpui::{App, AppContext as _, Task};
use py_debug::{DebugTarget, PythonDebugProvider, configuration_target};
use py_run::UvRun;
use py_run::config::{LaunchContext, PythonOptions};
use run_config::paths;

use crate::interpreter::{self, ConfiguredInterpreter};
use crate::run::NO_INTERPRETER;

/// What the UI thread found for a debug run.
#[derive(Debug, Clone)]
struct Resolved {
    python: PathBuf,
    uv: Option<UvRun>,
    /// From `pyvenv.cfg`, when it tells.
    version: Option<String>,
}

/// The run of a request with the interpreter resolved for it.
fn target(request: &DebugRequest) -> Result<DebugTarget, String> {
    let resolved = request
        .resolved::<Resolved>()
        .ok_or_else(|| NO_INTERPRETER.to_owned())?;
    let cx = LaunchContext {
        root: &request.root,
        source_roots: &request.source_roots,
        python: resolved.python.clone(),
        uv: resolved.uv.clone(),
    };
    configuration_target(request, &cx, resolved.version.clone())
}

/// The interpreter of a Python configuration, as a run picks it
/// (`PythonRunProvider::prepare_configuration`).
fn resolve(mut request: DebugRequest, cx: &mut App) -> Task<Result<DebugRequest, String>> {
    let config = &request.configuration;
    if config.type_id != py_run::config::TYPE_ID {
        return Task::ready(Err(format!(
            "'{}' cannot be debugged: its type is not supported yet",
            config.name
        )));
    }
    let options = PythonOptions::read(config);
    let configured = (!options.use_project_interpreter).then(|| ConfiguredInterpreter {
        name: options.interpreter_name.clone(),
        path: paths::resolve(&options.interpreter, &request.root),
    });
    let interpreter = interpreter::for_run(&request.root, configured, cx);
    cx.background_spawn(async move {
        let (selected, _) = interpreter.await;
        let selected = selected.ok_or_else(|| NO_INTERPRETER.to_owned())?;
        let uv = if selected.uv_project.is_some()
            && py_run::config::runs_with_uv(&request.configuration)
        {
            py_env::paths::venv_root_of(&selected.python).and_then(UvRun::locate)
        } else {
            None
        };
        let version = (!selected.version.is_empty()).then(|| selected.version.clone());
        request.resolved = Some(Arc::new(Resolved {
            python: selected.python,
            uv,
            version,
        }));
        Ok(request)
    })
}

/// The Python debug hook: debugpy, the Python breakpoint types, and the
/// interpreter of each run.
pub fn debug_hook() -> workbench::DebugHook {
    workbench::DebugHook::new(PythonDebugProvider::new(target)).with_resolve(resolve)
}
