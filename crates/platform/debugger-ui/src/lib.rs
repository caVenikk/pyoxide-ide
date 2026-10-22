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
//! - `inline`: the values of the selected frame after the lines of its
//!   file;
//! - [`value_hint`]: values under the mouse and the popups with their
//!   trees;
//! - [`DebugHook`]: what a product supplies (its debug adapter provider).

pub mod breakpoints;
pub mod frames;
mod hook;
pub mod icons;
mod inline;
pub mod session;
pub mod value_hint;
pub mod variables;
mod view;
pub mod watches;

pub use breakpoints::commands::{
    AddExceptionBreakpoint, EditBreakpoint, MuteBreakpoints, ToggleBreakpoint,
    ToggleBreakpointEnabled, ToggleTemporaryBreakpoint, ViewBreakpoints,
};
pub use hook::{DebugHook, Resolve};
pub use session::{Content, DebugToolWindow};
pub use view::{DebuggerView, DebuggerViewEvent, Highlighters, forward_actions};

/// Re-exported for the window's debug runs: the request and the session.
pub use debugger::{DebugRequest, Phase, Session};

/// Registers the icons and the commands.
pub fn init(cx: &mut gpui::App) {
    icons::register(cx);
    breakpoints::commands::register(cx);
    ui::command::register(cx, session::commands::COMMANDS);
    ui::command::register(cx, value_hint::COMMANDS);
}
