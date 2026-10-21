//! The side-by-side diff viewer (the reference's `SimpleDiffViewer` in an
//! editor tab): the base text on the left, read-only, its line numbers
//! next to the divider; the current text on the right, editable (the
//! file's document); changed lines and words highlighted, the divider
//! joining the changes of both sides, synchronized scrolling, Next and
//! Previous Difference, the next and previous file of a list, Jump to
//! Source, Select Opposite Diff Pane, the changes' actions in the gutter
//! (`>>` reverts a change, a check box includes it into the commit) and
//! the settings popup (Ignore Differences, Highlighting Differences).
//!
//! - [`model`]: the differences and the computations on them (no UI).
//! - [`view`]: the [`DiffView`] entity and its toolbar and titles.
//! - [`divider`]: the divider between the sides.
//!
//! The view knows no repository: the workbench gives it the base text,
//! the editor of the current text and the titles ([`DiffRequest`]) and
//! acts on its [`DiffEvent`]s.

pub mod divider;
pub mod model;
pub mod view;

pub use model::{
    ChangeKind, DiffChange, DiffSettings, HighlightPolicy, IgnorePolicy, Side, WordChange,
};
pub use view::{DiffEvent, DiffRequest, DiffView, ScrollTo, SideTitle};

/// The key context of the diff viewer.
pub const CONTEXT: &str = "Diff";

ui::ui_commands! {
    COMMANDS, scope = "Diff";
    /// F7 in a diff.
    NextDifference => "diff.next", "Next Difference",
        "Moves the carets of both sides to the next difference of the active diff; at the last one, pressed again, opens the next file's diff.";
    /// Shift+F7 in a diff.
    PreviousDifference => "diff.previous", "Previous Difference",
        "Moves the carets of both sides to the previous difference of the active diff; at the first one, pressed again, opens the previous file's diff.";
    /// Compare Next File.
    NextFile => "diff.next-file", "Compare Next File",
        "Shows the diff of the next file of the list the diff was opened for (the Commit tool window's changes).";
    /// Compare Previous File.
    PreviousFile => "diff.previous-file", "Compare Previous File",
        "Shows the diff of the previous file of the list the diff was opened for.";
    /// Select Opposite Diff Pane.
    FocusOpposite => "diff.focus-opposite", "Select Opposite Diff Pane",
        "Moves the focus to the other side of the active diff.";
    /// Jump to Source.
    JumpToSource => "diff.jump-to-source", "Jump to Source",
        "Opens the file of the active diff in its editor at the caret's line (a line of the left side goes to the line that stands there now).";
    /// Ignore Differences.
    SetIgnorePolicy { policy: String } => "diff.set-ignore-policy", "Ignore Differences",
        "Sets which whitespace the diff ignores: none, trim (at the start and end of lines) or whitespace (all of it).",
        r#"{"policy": "trim"}"#;
    /// Highlighting Differences.
    SetHighlightPolicy { policy: String } => "diff.set-highlight-policy", "Highlighting Differences",
        "Sets how the diff highlights changes: lines, words (changed lines and the changed words in them) or none.",
        r#"{"policy": "lines"}"#;
    /// The gutter's `>>` (`Diff.ApplyLeftSide`).
    ApplyLeftSide { index: Option<usize> } => "diff.apply-left-side", "Accept Left Side",
        "Replaces the right side's lines of a change with the left side's, as one undoable edit: in a diff of local changes it reverts the change (the gutter's >>). The change at index, else the change at the caret of the current side.",
        r#"{"index": 0}"#;
    /// The gutter's check box of a change (`Vcs.Diff.IncludeChangedLinesIntoCommit`).
    IncludeChange { index: Option<usize>, included: Option<bool> } => "diff.include-change", "Include into Commit",
        "Includes a change of the active diff into the commit or leaves it out (the gutter's check box; the commit then takes the file in part): the change at index, else the one at the caret of the current side; included sets it, else it toggles.",
        r#"{"index": 0, "included": false}"#;
    /// The settings popup of the toolbar's gear.
    ShowSettings => "diff.settings", "Show Diff Settings Popup\u{2026}",
        "Opens the diff's settings popup: Ignore Differences and Highlighting Differences.";
}

ui::ui_commands! {
    OPEN_COMMANDS, scope = "Workbench";
    /// Show Diff.
    ShowDiff { path: Option<String> } => "vcs.show-diff", "Show Diff",
        "Opens the diff of a file against its committed version in an editor tab: the file at path (relative to the repository's root, or absolute), else the selection of the Commit tool window or the project tool window while it has the focus, else the active file. A file of the Commit tool window's list opens in its preview tab, with the list's other files a step away.",
        r#"{"path": "main.py"}"#;
}

/// Registers the commands.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, OPEN_COMMANDS);
}
