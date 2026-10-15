//! Running files and run configurations: the contract with the product
//! that knows how to run its language ([`RunProvider`]), the run
//! configurations of the project (ADR 0012), the Run tool window and the
//! `run.*` commands.
//!
//! The workbench knows no language: it asks the product's provider whether
//! the current file can be run and for the command line that runs it or a
//! configuration of one of its types, then starts the process, shows its
//! output in the Run tool window and handles Stop and Rerun.

pub mod commands;
pub mod configurations;
pub mod console_view;
pub mod dropdown;
pub mod tool_window;
pub mod widget;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Task};
use run_config::{RunConfiguration, TypeInfo};
use ui::icons::IconName;

pub use configurations::{RunConfigs, RunConfigsEvent};
pub use console_view::{ConsoleEvent, ConsoleView};
pub use tool_window::{RunTab, RunToolWindow, RunToolWindowEvent, TabStatus};

/// Registers the run commands.
pub fn init(cx: &mut App) {
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::CONFIGURATION_COMMANDS);
    ui::command::register(cx, commands::TOOL_WINDOW_COMMANDS);
}

/// What the workbench asks the product to run: the current file, or a run
/// configuration.
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The file to run (absolute); for a configuration, empty.
    pub file: PathBuf,
    /// The configuration to run, instead of the file.
    pub configuration: Option<Arc<RunConfiguration>>,
    /// The project folder, when a project is open.
    pub project_root: Option<PathBuf>,
    /// The project's source roots, as the product's
    /// [`NavigationHooks::source_roots`](crate::NavigationHooks) found them
    /// when the project opened.
    pub source_roots: Arc<Vec<PathBuf>>,
}

impl RunRequest {
    /// What a launch of this request is known by while it is prepared and
    /// while it runs: the configuration, or the file.
    pub fn launch_key(&self) -> LaunchKey {
        match &self.configuration {
            Some(c) => LaunchKey::Configuration(c.key()),
            None => LaunchKey::File(self.file.clone()),
        }
    }

    /// The name of what runs: the configuration's, or the file's without
    /// its extension.
    pub fn name(&self) -> String {
        match &self.configuration {
            Some(c) => c.name.clone(),
            None => self
                .file
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// Prepares the run of a Shell Script configuration, the platform's own
/// type, off the UI thread.
pub(crate) fn prepare_shell_script(
    request: RunRequest,
    cx: &mut App,
) -> Task<Result<RunSpec, String>> {
    cx.background_spawn(async move {
        let config = request
            .configuration
            .as_ref()
            .ok_or_else(|| "No run configuration".to_owned())?;
        let root = request
            .project_root
            .clone()
            .ok_or_else(|| "No project is open".to_owned())?;
        let command = run_config::shell::command_line(config, &root)?;
        let display = command.display();
        Ok(RunSpec::new(
            config.name.clone(),
            IconName(run_config::shell::TYPE.icon),
            command,
            display,
            vec![Arc::new(console::UrlFilter)],
        ))
    })
}

/// A file run (Current File) or a configuration's run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchKey {
    File(PathBuf),
    Configuration(run_config::ConfigKey),
}

/// A process ready to start, as the product prepared it.
pub struct RunSpec {
    /// Title of the run's tab in the Run tool window: the configuration's
    /// name (for a file, its name without the extension, `main`).
    pub title: String,
    /// Icon of the run's tab.
    pub icon: IconName,
    /// The process to start.
    pub command: process::CommandLine,
    /// The console's first line: the command line as the reference prints
    /// it.
    pub display: String,
    /// Finders of links in the output (tracebacks to `file:line`, URLs), in
    /// order.
    pub link_filters: Vec<Arc<dyn console::LinkFilter>>,
    /// The process's standard input comes from this file ("Redirect input
    /// from"); the console takes no input then.
    pub input_file: Option<PathBuf>,
    /// A warning shown when the run starts (an interpreter that was not
    /// found and replaced).
    pub warning: Option<String>,
}

impl RunSpec {
    /// A spec without input file or warning.
    pub fn new(
        title: String,
        icon: IconName,
        command: process::CommandLine,
        display: String,
        link_filters: Vec<Arc<dyn console::LinkFilter>>,
    ) -> Self {
        Self {
            title,
            icon,
            command,
            display,
            link_filters,
            input_file: None,
            warning: None,
        }
    }
}

impl std::fmt::Debug for RunSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSpec")
            .field("title", &self.title)
            .field("icon", &self.icon)
            .field("display", &self.display)
            .field("link_filters", &self.link_filters.len())
            .field("input_file", &self.input_file)
            .field("warning", &self.warning)
            .finish_non_exhaustive()
    }
}

/// Runs files and run configurations of a product's languages.
pub trait RunProvider: 'static {
    /// The file can be run (Run is enabled while it is the current file).
    /// Called while drawing: no I/O.
    fn can_run(&self, file: &Path, cx: &App) -> bool;

    /// Prepares the command line that runs the request's file: finds the
    /// interpreter and builds the arguments and environment, off the UI
    /// thread. An error is a message for the user.
    fn prepare(&self, request: RunRequest, cx: &mut App) -> Task<Result<RunSpec, String>>;

    /// The run configuration types of the product (ADR 0005); the
    /// platform adds its own (Shell Script).
    fn configuration_types(&self) -> Vec<TypeInfo> {
        Vec::new()
    }

    /// Importers of the reference IDE's configurations of the product's
    /// types.
    fn importers(&self) -> Vec<Arc<dyn run_config::Importer>> {
        Vec::new()
    }

    /// Prepares the run of a configuration of one of the product's types
    /// (`request.configuration`), off the UI thread.
    fn prepare_configuration(
        &self,
        request: RunRequest,
        cx: &mut App,
    ) -> Task<Result<RunSpec, String>> {
        let _ = (request, cx);
        Task::ready(Err("This run configuration cannot run".to_owned()))
    }
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
