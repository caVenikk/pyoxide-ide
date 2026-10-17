//! Debugger UI of Pyoxide IDE on GPUI (ADR 0015). It knows the debugger
//! model (`debugger`), never the DAP client.
//!
//! - [`DebuggerView`]: the window's controller of the project's debugger:
//!   it opens the [`debugger::Debugger`] of a project, keeps the
//!   breakpoints of the editor tabs in their gutters and handles the
//!   `debug.*` breakpoint commands;
//! - [`breakpoints`]: line breakpoints in the gutter (the hover icon,
//!   clicks, icons by state, line backgrounds, stripe marks, tooltips),
//!   the breakpoint properties popup and the Breakpoints dialog;
//! - [`icons`]: the breakpoint icons;
//! - [`DebugHook`]: what a product supplies (its debug adapter provider).

pub mod breakpoints;
mod hook;
pub mod icons;
mod view;

pub use breakpoints::commands::{
    AddExceptionBreakpoint, EditBreakpoint, MuteBreakpoints, ToggleBreakpoint,
    ToggleBreakpointEnabled, ToggleTemporaryBreakpoint, ViewBreakpoints,
};
pub use hook::DebugHook;
pub use view::{DebuggerView, DebuggerViewEvent, Highlighters, forward_actions};

/// Registers the icons and the commands.
pub fn init(cx: &mut gpui::App) {
    icons::register(cx);
    breakpoints::commands::register(cx);
}
