//! What a product supplies to debug its programs (ADR 0005, 0015): the
//! adapter and its launch arguments for a run configuration, how its
//! exception breakpoints map to the adapter's filters, its default
//! breakpoints and what it imports from the reference.

use std::path::PathBuf;

use dap_client::Transport;
use process::CommandLine;
use run_config::RunConfiguration;
use serde_json::Value;

use crate::breakpoints::ExceptionBreakpoint;
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
}
