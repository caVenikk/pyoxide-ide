//! What a product supplies to debug its programs (ADR 0005, 0015): the
//! adapter and its launch arguments for a run configuration, how its
//! exception breakpoints map to the adapter's filters, its default
//! breakpoints, what it imports from the reference, and its breakpoint
//! types (titles, stoppable lines, exception options).

use std::path::{Path, PathBuf};

use dap_client::Transport;
use process::CommandLine;
use run_config::RunConfiguration;
use serde_json::Value;

use crate::breakpoints::{ExceptionBreakpoint, SuspendPolicy};
use crate::idea::IdeaTypes;

pub use dap_client::{ExceptionFilter, ExceptionFilterSetting};

/// A debug run: a run configuration of the project.
#[derive(Debug, Clone)]
pub struct DebugRequest {
    pub root: PathBuf,
    pub configuration: RunConfiguration,
}

impl DebugRequest {
    /// The key of the watches the session shows: the configuration's
    /// type, as the reference keys them.
    pub fn watches_key(&self) -> &str {
        &self.configuration.type_id
    }
}

/// How the adapter is reached.
pub enum Adapter {
    /// A process started with this command line, over its standard
    /// streams.
    Command(CommandLine),
    /// A connection made already (tests, adapters in the process; later
    /// sockets).
    Transport(Transport),
}

/// Whether the session starts the program or attaches to one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LaunchKind {
    #[default]
    Launch,
    Attach,
}

/// What [`DebugAdapterProvider::prepare`] returns.
pub struct AdapterLaunch {
    /// The session's name (the configuration's).
    pub title: String,
    pub adapter: Adapter,
    /// The adapter's type as sent at `initialize` (`debugpy`).
    pub adapter_id: String,
    pub kind: LaunchKind,
    /// The adapter's own `launch` or `attach` arguments.
    pub arguments: Value,
}

/// A request sent before and after one Step Into My Code (debugpy:
/// `setDebuggerProperty` with `justMyCode` on, then off).
#[derive(Debug, Clone, PartialEq)]
pub struct StepIntoMyCode {
    pub before: (String, Value),
    pub after: (String, Value),
}

/// The product hook of the debugger.
pub trait DebugAdapterProvider: Send + Sync + 'static {
    /// The adapter and the launch of a configuration. Blocking (finds the
    /// interpreter, the helper): called off the UI thread. An error is a
    /// message for the user.
    fn prepare(&self, request: &DebugRequest) -> Result<AdapterLaunch, String>;

    /// The adapter's exception filters to turn on for the enabled
    /// exception breakpoints; `available` are the adapter's filters. By
    /// default the adapter's default filters.
    fn exception_filters(
        &self,
        breakpoints: &[&ExceptionBreakpoint],
        available: &[ExceptionFilter],
    ) -> Vec<ExceptionFilterSetting> {
        let _ = breakpoints;
        available
            .iter()
            .filter(|f| f.default)
            .map(|f| ExceptionFilterSetting::new(f.id.clone()))
            .collect()
    }

    /// The exception breakpoints a project has before any is stored ("Any
    /// exception").
    fn default_exception_breakpoints(&self) -> Vec<ExceptionBreakpoint> {
        Vec::new()
    }

    /// The reference's breakpoint and configuration types to import.
    fn idea_types(&self) -> IdeaTypes {
        IdeaTypes::default()
    }

    /// Step Into My Code; `None` makes it a plain Step Into.
    fn step_into_my_code(&self) -> Option<StepIntoMyCode> {
        None
    }

    /// The product's type of line breakpoints; `None`: none can be set.
    fn line_breakpoint_type(&self) -> Option<LineBreakpointType> {
        None
    }

    /// The lines of a file's text a line breakpoint can be put on
    /// (`XLineBreakpointType.canPutAt`), by line from line 1; `None` for a
    /// file of another type. Blocking (one pass over the text): called off
    /// the UI thread.
    fn stoppable_lines(&self, file: &Path, text: &str) -> Option<Vec<bool>> {
        let _ = (file, text);
        None
    }

    /// Line breakpoints can be put in a file of this type (its stoppable
    /// lines are known), going by its path.
    fn line_breakpoint_file(&self, file: &Path) -> bool {
        self.stoppable_lines(file, "").is_some()
    }

    /// The product's type of exception breakpoints.
    fn exception_breakpoint_type(&self) -> Option<ExceptionBreakpointType> {
        None
    }

    /// The exception classes the class chooser of a new exception
    /// breakpoint offers.
    fn exception_classes(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A product's type of line breakpoints, as the reference presents it
/// (`XLineBreakpointType`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineBreakpointType {
    /// The type's id in the reference (`python-line`).
    pub id: String,
    /// Its group in the Breakpoints dialog (`Python Line Breakpoint`).
    pub title: String,
    /// What a new breakpoint suspends unless Make Default chose another
    /// policy (`getDefaultSuspendPolicy`).
    pub default_suspend: SuspendPolicy,
    /// All or Thread can be chosen (`isSuspendThreadSupported`).
    pub suspend_thread: bool,
}

/// An option of an exception breakpoint, a check box of its type's panel:
/// a `true` property while checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionOption {
    /// The property's name ([`ExceptionBreakpoint::properties`]).
    pub key: String,
    pub label: String,
}

/// A product's type of exception breakpoints (`XBreakpointType`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionBreakpointType {
    /// [`ExceptionBreakpoint::kind`] (`python-exception`).
    pub id: String,
    /// Its group in the Breakpoints dialog (`Python Exception Breakpoint`).
    pub title: String,
    /// The class of its default breakpoint and the name it is shown with
    /// (`BaseException`, "Any exception").
    pub any_class: String,
    pub any_name: String,
    /// The title of the options' group ("Activation policy") and the
    /// options.
    pub options_title: String,
    pub options: Vec<ExceptionOption>,
    /// The title of the class chooser ("Select Exception Class").
    pub chooser_title: String,
}

impl ExceptionBreakpointType {
    /// A breakpoint's name: the "any exception" name for its class, else
    /// the class (`getDisplayText`).
    pub fn display_name<'a>(&'a self, breakpoint: &'a ExceptionBreakpoint) -> &'a str {
        if breakpoint.class == self.any_class {
            &self.any_name
        } else {
            &breakpoint.class
        }
    }
}
