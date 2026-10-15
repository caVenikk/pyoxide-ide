//! Running Python in Pyoxide IDE, as the reference's "Current File" run and
//! its Python run configurations do (`PythonRunConfiguration`,
//! `PythonCommandLineState`).
//!
//! - [`config`]: the Python run configuration type (ADR 0012): its
//!   options, the import of the reference's Python and `uv run`
//!   configurations, and the run a configuration describes.
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

pub mod config;
mod run;
mod traceback;

pub use run::{PythonRun, Target, UvRun};
pub use traceback::TracebackFilter;
