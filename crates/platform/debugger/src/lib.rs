//! The debugger model of Pyoxide IDE without a UI toolkit (ADR 0015), as
//! the reference's XDebugger keeps it:
//!
//! - [`Debugger`]: one project's breakpoints, watches and mute state and
//!   its debug sessions; [`DebuggerEvent`]s tell listeners what changed;
//! - [`Session`]: one debug session over a DAP adapter, its state machine,
//!   threads, frames in pages, scopes, variables loaded lazily in pages,
//!   evaluation, Set Value, stepping and Stop;
//! - [`breakpoints`]: line and exception breakpoints and watches;
//! - [`storage`]: the `[debugger]` table of the personal project state;
//! - [`idea`]: the one-time import from `.idea/workspace.xml`;
//! - [`DebugAdapterProvider`]: the product hook (the adapter, its launch
//!   arguments, exception filters, default breakpoints, import types, the
//!   breakpoint types and their stoppable lines);
//! - [`values`]: how the variables view shows an adapter's values;
//! - [`commands`]: the `debug.*` engine commands.
//!
//! UI crates depend on this crate, never on `dap-client`
//! (`scripts/check-layers.py`); the client's plain value types that the
//! model shows (threads, frames, scopes, variables) are re-exported here.

pub mod breakpoints;
pub mod commands;
mod debugger;
pub mod idea;
mod provider;
mod session;
pub mod storage;
pub mod values;

pub use breakpoints::{
    BreakpointId, Breakpoints, ExceptionBreakpoint, LineBreakpoint, SuspendPolicy, Toggled, Watches,
};
pub use dap_client::{
    EvaluateContext, Evaluated, ExceptionInfo, FramePresentation, Presentation, Scope, Source,
    StackFrame, Thread, Transport, Variable,
};
pub use debugger::{Debugger, DebuggerEvent, HistoryKind};
pub use idea::IdeaTypes;
pub use provider::{
    Adapter, AdapterLaunch, DebugAdapterProvider, DebugRequest, EvaluationError,
    ExceptionBreakpointType, ExceptionFilter, ExceptionFilterSetting, ExceptionOption,
    ExtraValueAction, LaunchKind, LineBreakpointType, StepIntoMyCode,
};
pub use session::{
    BreakpointStatus, Children, DebugError, FRAME_PAGE, Pending, Phase, STOP_TIMEOUT, Session,
    SessionId, SessionSnapshot, StopInfo,
};
