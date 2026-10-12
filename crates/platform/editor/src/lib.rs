//! The code editor of Pyoxide IDE (ADR 0007): an [`Editor`] entity over a
//! `text::Buffer` and its GPUI element. Only visible lines are laid out and
//! painted; typing, caret movement, selection, clipboard, undo and the
//! platform input method (IME) are handled here. Every action is a command
//! of [`commands::COMMANDS`].

pub mod commands;
mod editor;
mod element;
pub mod highlight;
mod input;

pub use editor::{BLINK_INTERVAL, Editor, EditorConfig, EditorEvent, forward_actions, new_editor};
pub use element::LONG_LINE;
pub use highlight::{Highlighter, Job, JobResult, Span, SyntaxHighlighter};

/// Registers the editor commands with the command index.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
}
