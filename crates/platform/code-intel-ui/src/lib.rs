//! The code intelligence UI of Pyoxide IDE (ADR 0010): one
//! [`CodeIntelView`] per main window connects the editors of its tabs to
//! the project's language services (a [`code_intel::CodeIntel`] the
//! product builds through its [`CodeIntelProvider`]), and owns the popups
//! of completion, documentation, parameter info, navigation and intention
//! actions, the Language Services widget and the `code.*` commands.
//!
//! - [`document`]: the documents (tab editors with a language id), their
//!   version counter and the recent texts that map late answers into the
//!   current text.
//! - [`popup`]: popups at a position in an editor that leave the focus in
//!   the editor; while one shows, the editor's container gets its key
//!   context (`Completion > Editor`).
//! - [`completion`]: the completion popup, opened by Basic Completion or
//!   by typing.
//! - [`services`]: the Language Services widget of the status bar and its
//!   popup.
//!
//! The UI depends on the `code-intel` traits only, never on the LSP
//! client; tests use `code_intel::fake::FakeCodeIntel`.

// Matched parts of names are lists of ranges, often of one.
#![allow(clippy::single_range_in_vec_init)]

pub mod commands;
pub mod completion;
pub mod document;
mod hook;
mod marks;
pub mod popup;
pub mod services;
mod view;

pub use code_intel;
pub use hook::{CodeIntelHook, CodeIntelProvider};
pub use view::{
    CaretQuery, CodeIntelEvent, CodeIntelView, HINT_CONTEXT, NO_SUGGESTIONS, editor_container,
    forward_actions,
};

/// Registers the code intelligence commands with the command index.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::COMPLETION_COMMANDS);
    ui::command::register(cx, commands::SERVICE_COMMANDS);
}
