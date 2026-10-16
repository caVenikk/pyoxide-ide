//! What the client's users see of the protocol: plain values converted from
//! the private wire messages ([`crate::protocol`]). Lines and columns count
//! from 1 (the client says so at `initialize`).

use std::fmt;
use std::path::PathBuf;

/// What the adapter supports (the `initialize` answer and later
/// `capabilities` events).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub configuration_done: bool,
    pub conditional_breakpoints: bool,
    pub hit_conditional_breakpoints: bool,
    pub log_points: bool,
    pub evaluate_for_hovers: bool,
    pub set_variable: bool,
    pub set_expression: bool,
    pub terminate: bool,
    pub terminate_debuggee: bool,
    pub cancel: bool,
    pub exception_info: bool,
    pub exception_filter_options: bool,
    pub delayed_stack_trace_loading: bool,
    pub clipboard_context: bool,
    pub step_in_targets: bool,
    pub restart_frame: bool,
    pub goto_targets: bool,
    pub single_thread_execution: bool,
    /// The exception breakpoint filters the adapter offers.
    pub exception_filters: Vec<ExceptionFilter>,
}

/// An exception breakpoint filter of the adapter (`uncaught`, `raised`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExceptionFilter {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    /// On unless the user changes it.
    pub default: bool,
    pub supports_condition: bool,
}

/// A filter to turn on with `setExceptionBreakpoints`, with a condition
/// when the adapter supports one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionFilterSetting {
    pub id: String,
    pub condition: Option<String>,
}

impl ExceptionFilterSetting {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            condition: None,
        }
    }
}

/// What the client tells the adapter about itself at `initialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// The adapter's type (`debugpy`), as the client names it.
    pub adapter_id: String,
    pub client_id: String,
    pub client_name: String,
    pub locale: String,
    /// The client answers `runInTerminal`.
    pub run_in_terminal: bool,
    /// The client answers `startDebugging`.
    pub start_debugging: bool,
}

impl ClientInfo {
    /// Pyoxide as the client of an adapter of type `adapter_id`.
    pub fn new(adapter_id: impl Into<String>) -> Self {
        Self {
            adapter_id: adapter_id.into(),
            client_id: "pyoxide".to_owned(),
            client_name: "Pyoxide IDE".to_owned(),
            locale: "en".to_owned(),
            run_in_terminal: true,
            start_debugging: false,
        }
    }
}

/// A file or a text the adapter has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    pub name: Option<String>,
    pub path: Option<PathBuf>,
    /// Set when the text comes from the adapter (`source` request).
    pub reference: Option<i64>,
    /// The adapter asks to show it less prominently (`deemphasize`).
    pub deemphasized: bool,
}

/// A line breakpoint as sent with `setBreakpoints`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceBreakpoint {
    /// From 1.
    pub line: u32,
    pub column: Option<u32>,
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    /// A log message (`{expression}` parts are evaluated): the breakpoint
    /// does not suspend.
    pub log_message: Option<String>,
}

impl SourceBreakpoint {
    pub fn at(line: u32) -> Self {
        Self {
            line,
            ..Self::default()
        }
    }
}

/// A breakpoint as the adapter reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    /// Why it is not verified, or a note on it.
    pub message: Option<String>,
    pub source: Option<Source>,
    /// The line the adapter set it on: another one when it moved it.
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FramePresentation {
    #[default]
    Normal,
    /// A separator or a label, not a real frame.
    Label,
    /// A frame to show less prominently.
    Subtle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub source: Option<Source>,
    pub line: u32,
    pub column: u32,
    pub presentation: FramePresentation,
}

/// A page of a thread's frames.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackTrace {
    pub frames: Vec<StackFrame>,
    /// All frames of the thread, when the adapter says.
    pub total: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub name: String,
    /// `arguments`, `locals`, `registers` or the adapter's own.
    pub hint: Option<String>,
    pub reference: i64,
    pub named: Option<usize>,
    pub indexed: Option<usize>,
    /// Costly to fetch: not expanded on its own.
    pub expensive: bool,
}

/// How a value should be shown (`presentationHint`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presentation {
    /// `property`, `method`, `class`, `data`, `event`, `baseClass`,
    /// `innerClass`, `interface`, `mostDerivedClass`, `virtual`, ...
    pub kind: Option<String>,
    /// `static`, `constant`, `readOnly`, `rawString`, `hasObjectId`, ...
    pub attributes: Vec<String>,
    /// `public`, `private`, `protected`, `internal`, `final`.
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    pub presentation: Presentation,
    /// An expression that evaluates to the variable.
    pub evaluate_name: Option<String>,
    /// Greater than 0 when the variable has children.
    pub reference: i64,
    pub named: Option<usize>,
    pub indexed: Option<usize>,
}

/// A value from `evaluate`, `setVariable` or `setExpression`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evaluated {
    pub value: String,
    pub type_name: Option<String>,
    pub presentation: Presentation,
    pub reference: i64,
    pub named: Option<usize>,
    pub indexed: Option<usize>,
}

/// Where an expression is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluateContext {
    Watch,
    Repl,
    Hover,
    Clipboard,
}

impl EvaluateContext {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EvaluateContext::Watch => "watch",
            EvaluateContext::Repl => "repl",
            EvaluateContext::Hover => "hover",
            EvaluateContext::Clipboard => "clipboard",
        }
    }
}

/// Which children `variables` returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VariablesFilter {
    #[default]
    All,
    Indexed,
    Named,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExceptionInfo {
    pub id: String,
    pub description: Option<String>,
    /// `never`, `always`, `unhandled` or `userUnhandled`.
    pub break_mode: String,
    pub message: Option<String>,
    pub type_name: Option<String>,
    pub stack_trace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stopped {
    /// `step`, `breakpoint`, `exception`, `pause`, `entry`, `goto`,
    /// `function breakpoint`, ...
    pub reason: String,
    pub description: Option<String>,
    pub thread_id: Option<i64>,
    pub preserve_focus: bool,
    pub text: Option<String>,
    pub all_threads_stopped: bool,
    pub hit_breakpoint_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// `console`, `important`, `stdout`, `stderr`, `telemetry`, ...;
    /// `console` when the adapter names none.
    pub category: String,
    pub text: String,
    pub source: Option<Source>,
    pub line: Option<u32>,
}

/// What the adapter sends on its own.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The adapter takes the configuration (breakpoints, then
    /// `configurationDone`).
    Initialized,
    Stopped(Stopped),
    Continued {
        thread_id: i64,
        all_threads: bool,
    },
    Exited {
        exit_code: i64,
    },
    Terminated,
    ThreadStarted(i64),
    ThreadExited(i64),
    Output(Output),
    Breakpoint {
        /// `changed`, `new` or `removed`.
        reason: String,
        breakpoint: Breakpoint,
    },
    Process {
        name: String,
        pid: Option<u32>,
    },
    /// Capabilities changed; [`crate::Client::capabilities`] has them
    /// already.
    Capabilities(Capabilities),
    /// Shown state is out of date (threads, stacks, variables).
    Invalidated,
    /// An event this client does not read.
    Other {
        name: String,
        body: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TerminalKind {
    #[default]
    Integrated,
    External,
}

/// The adapter asks the client to start a process (the debuggee with its
/// launcher) in a terminal: in Pyoxide, the Debug console.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunInTerminal {
    pub kind: TerminalKind,
    pub title: Option<String>,
    pub cwd: PathBuf,
    /// The command line, the program first.
    pub args: Vec<String>,
    /// Variables to set (`Some`) or remove (`None`).
    pub env: Vec<(String, Option<String>)>,
    pub args_can_be_interpreted_by_shell: bool,
}

/// A request the adapter sends to the client.
#[derive(Debug, Clone, PartialEq)]
pub enum ReverseRequest {
    RunInTerminal(RunInTerminal),
    /// A child session (a subprocess): `launch` or `attach` with a
    /// configuration.
    StartDebugging {
        request: String,
        configuration: serde_json::Value,
    },
    Other {
        command: String,
        arguments: serde_json::Value,
    },
}

/// Why a request has no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DapError {
    /// The adapter answered with an error: its text.
    Adapter(String),
    /// The adapter answered that the request was cancelled.
    Cancelled,
    /// The connection is closed: the adapter exited or is being stopped.
    Closed,
    /// The adapter does not support the request.
    Unsupported(&'static str),
    /// The answer did not have the expected form.
    Malformed(String),
}

impl fmt::Display for DapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DapError::Adapter(message) => f.write_str(message),
            DapError::Cancelled => f.write_str("the request was cancelled"),
            DapError::Closed => f.write_str("the connection to the debugger is closed"),
            DapError::Unsupported(what) => write!(f, "the debugger does not support {what}"),
            DapError::Malformed(message) => write!(f, "a malformed answer: {message}"),
        }
    }
}

impl std::error::Error for DapError {}
