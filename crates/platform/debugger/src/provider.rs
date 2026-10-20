//! What a product supplies to debug its programs (ADR 0005, 0015): the
//! adapter and its launch arguments for a run configuration, how its
//! exception breakpoints map to the adapter's filters, its default
//! breakpoints, what it imports from the reference, its breakpoint types
//! (titles, stoppable lines, exception options) and how its values are
//! shown.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dap_client::{Transport, Variable};
use process::CommandLine;
use run_config::RunConfiguration;
use serde_json::Value;

use crate::breakpoints::{ExceptionBreakpoint, SuspendPolicy};
use crate::idea::IdeaTypes;
use crate::values::ValueNode;

pub use dap_client::{ExceptionFilter, ExceptionFilterSetting};

/// A debug run: a run configuration of the project.
#[derive(Clone)]
pub struct DebugRequest {
    pub root: PathBuf,
    pub configuration: RunConfiguration,
    /// The project's source roots.
    pub source_roots: Vec<PathBuf>,
    /// What the product found for this run on the UI thread before the
    /// session starts (the interpreter it picked for it), for its own
    /// [`DebugAdapterProvider::prepare`] to read back; `None` when the
    /// product has no such step.
    pub resolved: Option<Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for DebugRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugRequest")
            .field("root", &self.root)
            .field("configuration", &self.configuration.name)
            .field("source_roots", &self.source_roots)
            .field("resolved", &self.resolved.is_some())
            .finish()
    }
}

impl DebugRequest {
    /// A request without source roots or resolved data.
    pub fn new(root: impl Into<PathBuf>, configuration: RunConfiguration) -> Self {
        Self {
            root: root.into(),
            configuration,
            source_roots: Vec::new(),
            resolved: None,
        }
    }

    /// What the product resolved for the request, when it is a `T`.
    pub fn resolved<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.resolved.as_deref()?.downcast_ref()
    }

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
    /// A connection made already (a socket to an adapter that listens,
    /// tests, adapters in the process).
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
    /// The program's standard input comes from this file ("Redirect input
    /// from"); its console then takes no input.
    pub input_file: Option<PathBuf>,
    /// How long an expression evaluation may wait for the adapter's answer
    /// before it fails (watches, Evaluate Expression, the console); `None`:
    /// as long as the session lives.
    pub evaluation_timeout: Option<Duration>,
}

/// Requests sent around one Step Into My Code: `before` is answered before
/// the step is sent, `after` is sent at the next stop and answered before
/// the stop's threads and frames are asked for (debugpy: its library
/// filter on for the one step).
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

    /// A run configuration can be debugged: its type has a launch here
    /// (the Debug buttons are enabled for it). Called while drawing: no
    /// I/O.
    fn can_debug(&self, configuration: &RunConfiguration) -> bool {
        let _ = configuration;
        true
    }

    /// Step Into My Code; `None` makes it a plain Step Into.
    fn step_into_my_code(&self) -> Option<StepIntoMyCode> {
        None
    }

    /// A request sent to each process of a session before its first step,
    /// answered before that step is sent: it prepares the adapter's
    /// stepping (debugpy: a step keeps to the asyncio task it starts in).
    /// `None`: nothing is sent.
    fn prepare_steps(&self) -> Option<(String, Value)> {
        None
    }

    /// The product debugs the child sessions its adapter asks for (DAP
    /// `startDebugging`: subprocesses of the program); the client offers
    /// them at `initialize`.
    fn child_sessions(&self) -> bool {
        false
    }

    /// How to reach the adapter for a child session, given the child's
    /// `attach` or `launch` configuration (from `startDebugging`). An
    /// error is answered to the adapter, and the child runs on without the
    /// debugger. Blocking (connects a socket): called off the UI thread.
    fn child_adapter(&self, configuration: &Value) -> Result<Adapter, String> {
        let _ = configuration;
        Err("child sessions are not supported".to_owned())
    }

    /// The title of the balloon of a session whose debugger could not
    /// connect or failed; `None`: the platform's own.
    fn connection_failed_title(&self) -> Option<String> {
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

    /// How the variables view shows a child the adapter reports: its name
    /// and whether it is a value, a group or more children. Called while
    /// drawing: no I/O.
    fn present_value(&self, variable: &Variable) -> ValueNode {
        ValueNode::plain(variable)
    }

    /// An expression that evaluates, in a frame, to the sizes of the
    /// values of `expressions` (the length shown with a container's type,
    /// `{list: 4}`); `None` when the product shows no sizes.
    fn sizes_expression(&self, expressions: &[&str]) -> Option<String> {
        let _ = expressions;
        None
    }

    /// The sizes read from the value of [`Self::sizes_expression`], one per
    /// expression (`None`: the value has no size).
    fn parse_sizes(&self, value: &str, count: usize) -> Vec<Option<String>> {
        let _ = value;
        vec![None; count]
    }

    /// The variables view shows the children of a frame's first scope at
    /// its root and leaves the other scopes out (the reference's Python
    /// frames show their locals); otherwise every scope is a group.
    fn first_scope_only(&self) -> bool {
        false
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
