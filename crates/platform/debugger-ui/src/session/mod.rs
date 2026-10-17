//! Debug sessions in the window (the reference's `XDebugSessionTab` in the
//! New UI, `XDebugSessionTabNewUI`): the Debug tool window with a tab per
//! session, its toolbar and its Threads & Variables and Console tabs
//! ([`tool_window`]), the line the program is suspended on in the editor
//! ([`execution_point`]), the session commands ([`commands`]) and the
//! [`crate::DebuggerView`]'s side of starting, stepping and stopping
//! sessions ([`control`]).

pub mod commands;
pub(crate) mod control;
pub(crate) mod execution_point;
pub mod tool_window;

pub use tool_window::{Content, DebugToolWindow};
