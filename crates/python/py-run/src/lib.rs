//! Running a Python file in Pyoxide IDE, as the reference's "Current File"
//! run does (`PythonRunConfiguration`, `PythonCommandLineState`).
//!
//! - [`PythonRun`] describes the run (interpreter, optional `uv run`, script,
//!   working directory, project roots, environment) and builds the
//!   [`process::CommandLine`] with the reference's environment: user
//!   variables, `PYTHONUNBUFFERED=1`, `PYCHARM_HOSTED=1`,
//!   `PYTHONIOENCODING=UTF-8`, `PYTHONPATH` from content and source roots,
//!   and the virtual environment's activation (`VIRTUAL_ENV`, `PATH`).
//! - [`PythonRun::display`] is the first console line exactly as the
//!   reference prints it.
//! - [`TracebackFilter`] turns `File "...", line N` (and pytest's
//!   `path:line`) into file links, the reference's `PythonTracebackFilter`.

mod run;
mod traceback;

pub use run::{PythonRun, UvRun};
pub use traceback::TracebackFilter;
