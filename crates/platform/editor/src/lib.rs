//! The code editor of Pyoxide IDE (ADR 0007): an [`Editor`] entity over a
//! `text::Buffer` and its GPUI element. Only visible lines are laid out and
//! painted; typing, caret movement, selection, clipboard, undo and the
//! platform input method (IME) are handled here. Every action is a command
//! of [`commands::COMMANDS`]. The find bar ([`find::FindBar`]) searches
//! an editor and shows the matches in it.

pub mod commands;
mod diagnostics;
mod editor;
mod element;
pub mod find;
pub mod highlight;
mod input;
mod inspection;

pub use diagnostics::{Analysis, DiagnosticMark, MarkMessage, MarkSeverity, ProblemCounts};
pub use editor::{
    BLINK_INTERVAL, Editor, EditorConfig, EditorEvent, ForwardedInput, SearchHighlights,
    forward_actions, new_editor,
};
pub use element::LONG_LINE;
pub use find::FindBar;
pub use highlight::{Highlighter, InlineStyle, Job, JobResult, Span, SyntaxHighlighter};

/// Height of a one-line field in the UI font ([`Editor::set_ui_font`]),
/// for laying out its container.
pub fn field_line_height(window: &gpui::Window, cx: &gpui::App) -> gpui::Pixels {
    field_line_height_for(true, window, cx)
}

/// The line height of a single-line field in the UI font, or in the
/// editor font (`set_ui_font(false)`).
pub fn field_line_height_for(ui_font: bool, window: &gpui::Window, cx: &gpui::App) -> gpui::Pixels {
    use ui::theme::ThemeExt as _;
    element::metrics(cx.theme(), window, ui_font).line_height
}

/// Registers the editor and find bar commands with the command index.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, find::COMMANDS);
    ui::command::register(cx, find::BAR_COMMANDS);
    ui::command::register(cx, find::HISTORY_COMMANDS);
}
