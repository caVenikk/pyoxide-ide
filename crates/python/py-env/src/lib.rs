//! Python environments for Pyoxide IDE.
//!
//! - [`discovery`]: interpreters from uv, `PATH`, project virtual
//!   environments, Poetry, conda and pyenv, each probed once.
//! - [`project`]: `pyproject.toml` (PEP 621, PEP 735, `[tool.uv]`), `uv.lock`,
//!   requirements files and PEP 723 script metadata.
//! - [`uv`] and [`pip`]: typed wrappers around the command-line tools, with
//!   progress lines, cancellation, timeouts and typed errors.
//! - [`search`]: package metadata from PyPI behind the [`search::PackageIndex`] trait.
//! - [`venv`]: environments on disk (`pyvenv.cfg`, the environments of a
//!   project) read without starting Python.
//! - [`ops`] and [`commands`]: the `env.*` operations and their registration
//!   in the command registry (ADR 0001).
//!
//! Nothing here blocks: every external process is spawned with tokio, bounded
//! by a timeout and killed when its future is dropped or cancelled.

pub mod commands;
pub mod discovery;
pub mod error;
pub mod interpreter;
pub mod ops;
pub mod packages;
pub mod paths;
pub mod pip;
pub mod process;
pub mod project;
pub mod search;
pub mod service;
pub mod uv;
pub mod venv;
pub mod version;

pub use commands::register;
pub use discovery::{DiscoveryOptions, DiscoveryReport, SearchEnv, ToolStatus, Tools, discover};
pub use error::{EnvError, ErrorKind, Result};
pub use interpreter::{Interpreter, InterpreterKind};
pub use packages::{Backend, Outcome, Package};
pub use process::{CancellationToken, Progress, ProgressSink, RunContext, Stream};
pub use project::{ProjectInfo, load_project};
pub use search::{IndexProject, PackageIndex, PyPi, StaticIndex};
pub use service::EnvService;
pub use uv::{Uv, UvConfig};
