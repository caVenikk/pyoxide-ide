//! The code intelligence UI of Pyoxide IDE (ADR 0010): one
//! [`CodeIntelView`] per main window connects the editors of its tabs to
//! the project's language services (a [`code_intel::CodeIntel`] the
//! product builds through its [`CodeIntelProvider`]), and owns the popups
//! of completion, documentation, parameter info, navigation and intention
//! actions, the Language Services widget and the `code.*` commands.
//!
//! The UI depends on the `code-intel` traits only, never on the LSP
//! client; tests use `code_intel::fake::FakeCodeIntel`.
//!
//! This is the skeleton: the commands are declared and reach the view,
//! which does nothing with them yet.

pub mod commands;
mod hook;
mod view;

pub use code_intel;
pub use hook::{CodeIntelHook, CodeIntelProvider};
pub use view::{CodeIntelView, forward_actions};

/// Registers the code intelligence commands with the command index.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::COMPLETION_COMMANDS);
    ui::command::register(cx, commands::SERVICE_COMMANDS);
}
