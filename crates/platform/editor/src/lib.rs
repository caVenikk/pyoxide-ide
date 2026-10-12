//! The code editor of Pyoxide IDE (ADR 0007): an [`Editor`] entity over a
//! `text::Buffer` and its GPUI element. Only visible lines are laid out and
//! painted; typing, caret movement, selection, clipboard, undo and the
//! platform input method (IME) are handled here. Every action is a command
//! of [`commands::COMMANDS`]. The find bar ([`find::FindBar`]) searches
//! an editor and shows the matches in it.

pub mod commands;
mod editor;
mod element;
pub mod find;
pub mod highlight;
mod input;

pub use editor::{
    BLINK_INTERVAL, Editor, EditorConfig, EditorEvent, SearchHighlights, forward_actions,
    new_editor,
};
pub use element::LONG_LINE;
pub use find::FindBar;
pub use highlight::{Highlighter, Job, JobResult, Span, SyntaxHighlighter};

/// Registers the editor and find bar commands with the command index.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, find::COMMANDS);
    ui::command::register(cx, find::BAR_COMMANDS);
}
