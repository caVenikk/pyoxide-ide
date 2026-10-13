//! Running Python files: the product's [`RunProvider`].

use std::path::Path;

use gpui::{App, Task};
use workbench::{RunProvider, RunRequest, RunSpec};

/// Runs a Python file with the project's interpreter.
pub struct PythonRunProvider;

/// Extensions of the files Run accepts.
const RUNNABLE: &[&str] = &["py", "pyw"];

impl RunProvider for PythonRunProvider {
    fn can_run(&self, file: &Path, _: &App) -> bool {
        file.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| RUNNABLE.contains(&e))
    }

    fn prepare(&self, _request: RunRequest, _cx: &mut App) -> Task<Result<RunSpec, String>> {
        Task::ready(Err("Running Python files is not implemented yet".to_owned()))
    }
}
