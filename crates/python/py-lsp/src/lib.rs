//! Python language services of Pyoxide IDE (ADR 0010, 0011): which servers
//! run for a project (ruff, and ty or basedpyright as the type server),
//! where their executables are, how they are configured, and the one-time
//! import of the reference's tool settings from `.idea`.
//!
//! - [`settings`]: the shared settings in `.oxide/project.toml`
//!   (`[python.tools.<tool>]`, `[python.type-engine]`) and the personal
//!   executable paths and `uvx` answers in `.oxide/workspace.toml`.
//! - [`lookup`]: finding a tool by its lookup mode (Sdk, Path, uvx) and
//!   asking it for its version.
//! - [`servers`]: the server specs for `lsp-client`: features,
//!   initialization options and `workspace/configuration` settings.
//! - [`completion`]: the type servers' completion items as the reference
//!   presents Python items (`check(self, value)`, `Point`).
//! - [`ruff`]: ruff's adjustments (rule names, nested and added actions).
//! - [`idea`]: the import of `.idea/pyLspTools.xml` and
//!   `.idea/pyTypeEngine.xml`.
//! - [`services`]: a project as a whole: imports, settings, every tool's
//!   lookup, version and problem, and its server specs.
//!
//! No UI toolkit here, and nothing that waits runs on the caller's behalf:
//! lookups read the file system and the version check runs a process, so
//! the product calls them off the UI thread.

pub mod completion;
pub mod idea;
pub mod lookup;
pub mod ruff;
pub mod servers;
pub mod services;
pub mod settings;

pub use lookup::{Executable, Lookup, LookupContext, Resolution, SearchPaths};
pub use services::{ProjectState, SpecTarget};
pub use settings::{Settings, Tool, ToolSettings, TypeEngine};
