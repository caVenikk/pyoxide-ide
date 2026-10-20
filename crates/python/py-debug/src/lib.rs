//! Debugging Python in Pyoxide IDE with debugpy over DAP (ADR 0015), the
//! Python product's part of the debugger (no UI toolkit):
//!
//! - [`helper`]: the debugpy 1.8.21 helper folder, looked up in the
//!   bundle, next to the binary, then in the application cache, with a
//!   message naming `scripts/fetch-debugpy.py` when it is missing;
//! - [`launch`]: the adapter (`python <helper>/debugpy/adapter`), the
//!   `launch` arguments of a [`py_run::PythonRun`] (script or module,
//!   arguments, working directory, environment, interpreter; `uv run`
//!   debugs the environment's interpreter), the debugger options that
//!   change them, the Python 3.8 check;
//! - [`lines`]: the reference's stoppable-line rule for line breakpoints;
//! - [`breakpoints`]: the Python breakpoint types and the exception
//!   breakpoint mapping ("On termination" is `uncaught`, "On raise" is
//!   `raised`);
//! - [`settings`]: the project's Python debugger options (Settings > Python >
//!   Debugger) in the personal project state, imported once from the
//!   reference's `workspace.xml`;
//! - how the variables view shows Python values (groups, return values,
//!   container sizes);
//! - [`PythonDebugProvider`]: the product's [`debugger::DebugAdapterProvider`],
//!   given the product's [`RunResolver`] (which interpreter a request runs
//!   with).

pub mod breakpoints;
pub mod helper;
pub mod launch;
pub mod lines;
mod provider;
pub mod settings;
mod values;

pub use helper::{DEBUGPY_VERSION, Helper, HelperMissing};
pub use launch::DebugOptions;
pub use lines::StoppableLines;
pub use provider::{
    ADAPTER_ID, DebugTarget, PythonDebugProvider, RunResolver, configuration_target,
};
pub use settings::{DebuggerSettings, QtBackend};
