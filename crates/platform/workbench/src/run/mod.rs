//! Running files: the contract with the product that knows how to run a
//! file of its language ([`RunProvider`]), the Run tool window and the
//! `run.*` commands.
//!
//! The workbench knows no language: it asks the product's provider whether
//! the current file can be run and for the command line that runs it, then
//! starts the process, shows its output in the Run tool window and handles
//! Stop and Rerun.

mod tool_window;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Task};
use ui::icons::IconName;

pub use tool_window::{RunToolWindow, RunToolWindowEvent};

// Stand-ins for the `process` and `console` crates until they land.
#[doc(hidden)]
pub mod process {
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// A command line to start.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct CommandLine {
        pub program: PathBuf,
        pub args: Vec<OsString>,
        pub cwd: Option<PathBuf>,
        pub env: Vec<(OsString, Option<OsString>)>,
    }
}

#[doc(hidden)]
pub mod console {
    /// Finds links in a line of output.
    pub trait LinkFilter: Send + Sync {}
}

/// What the workbench asks the product to run: the current file.
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The file to run (absolute).
    pub file: PathBuf,
    /// The project folder, when a project is open.
    pub project_root: Option<PathBuf>,
    /// The project's source roots, as the product's
    /// [`NavigationHooks::source_roots`](crate::NavigationHooks) found them
    /// when the project opened.
    pub source_roots: Arc<Vec<PathBuf>>,
}

/// A process ready to start, as the product prepared it.
pub struct RunSpec {
    /// Title of the run's tab in the Run tool window (for a file, its name).
    pub title: String,
    /// Icon of the run's tab and of the run widget.
    pub icon: IconName,
    /// The process to start.
    pub command: process::CommandLine,
    /// Finders of links in the output (for example tracebacks to
    /// `file:line`), applied after the platform's URL links.
    pub link_filters: Vec<Arc<dyn console::LinkFilter>>,
}

impl std::fmt::Debug for RunSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSpec")
            .field("title", &self.title)
            .field("icon", &self.icon)
            .field("command", &self.command)
            .field("link_filters", &self.link_filters.len())
            .finish()
    }
}

/// Runs files of a product's languages.
pub trait RunProvider: 'static {
    /// The file can be run (Run is enabled while it is the current file).
    /// Called while drawing: no I/O.
    fn can_run(&self, file: &Path, cx: &App) -> bool;

    /// Prepares the command line that runs the request's file: finds the
    /// interpreter and builds the arguments and environment, off the UI
    /// thread. An error is a message for the user.
    fn prepare(&self, request: RunRequest, cx: &mut App) -> Task<Result<RunSpec, String>>;
}

/// The product's [`RunProvider`] in a [`Product`](crate::Product).
#[derive(Clone)]
pub struct RunHook(pub Rc<dyn RunProvider>);

impl RunHook {
    pub fn new(provider: impl RunProvider) -> Self {
        Self(Rc::new(provider))
    }
}

impl std::fmt::Debug for RunHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RunHook")
    }
}
