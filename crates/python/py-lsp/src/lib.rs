//! Python language services of Pyoxide IDE (ADR 0010, 0011): which servers
//! run for a project (ruff, and ty or basedpyright as the type server),
//! where their executables are, how they are configured, and the one-time
//! import of the reference's tool settings from `.idea`.
//!
//! - [`settings`]: the shared settings in `.oxide/project.toml`
//!   (`[python.tools.<tool>]`, `[python.type-engine]`) and the personal
//!   executable paths in `.oxide/workspace.toml`.
//! - [`lookup`]: finding a tool by its lookup mode (Sdk, Path, uvx).
//! - [`servers`]: the server specs for `lsp-client`.
//! - [`ruff`]: ruff's adjustments (rule names, nested and added actions).
//! - [`idea`]: the import of `.idea/pyLspTools.xml` and
//!   `.idea/pyTypeEngine.xml`.
//!
//! This is the skeleton: types and signatures; the bodies come with the
//! Python services package.

pub mod idea;
pub mod lookup;
pub mod ruff;
pub mod servers;
pub mod settings;

pub use lookup::{Executable, Lookup, LookupContext};
pub use settings::{Settings, Tool, ToolSettings, TypeEngine};
