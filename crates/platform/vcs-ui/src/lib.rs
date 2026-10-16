//! Version control UI of Pyoxide IDE (ADR 0016) on GPUI. It knows the
//! model (`vcs`), never a backend: the workbench gives it statuses and base
//! texts.
//!
//! - [`status_colors`]: file names in the colour of their status
//!   (`FILESTATUS_*`), for the project tree, editor tabs and file lists.
//! - [`markers`]: an editor's change markers against its base text, the
//!   error stripe marks, Next / Previous Change and Rollback Lines.
//! - [`marker_popup`]: the popup of a change marker.
//! - The `vcs.*` commands of the editor ([`COMMANDS`]) and of the popup
//!   ([`POPUP_COMMANDS`]); the workbench handles them.

pub mod marker_popup;
pub mod markers;
pub mod status_colors;

pub use marker_popup::{MarkerPopup, PopupAction, PopupLine, popup_lines, render_popup};
pub use markers::{Computed, LineStatus, LineStatusEvent, Part};
pub use status_colors::{FileColors, OpenDocument, entry_role, status_role};

ui::ui_commands! {
    COMMANDS, scope = "Editor";
    /// Go to the next change and show its popup.
    NextChange => "vcs.next-change", "Next Change",
        "Moves the caret to the next changed lines of the active file against its committed version and shows their popup.";
    /// Go to the previous change and show its popup.
    PreviousChange => "vcs.previous-change", "Previous Change",
        "Moves the caret to the previous changed lines of the active file against its committed version and shows their popup.";
    /// Show the popup of the change at the caret.
    ShowCurrentChange => "vcs.show-current-change", "Show Current Change Marker",
        "Shows the popup of the changed lines at the caret: the committed text and the actions on the change.";
    /// Put the committed text of the changed lines back.
    RollbackLines => "vcs.rollback-lines", "Rollback Lines",
        "Puts the committed text back for the changes at the carets or in the selection (or for the change of the open marker popup), as one undoable edit.";
    /// Compare the changed lines with their committed version.
    ShowDiffLines => "vcs.show-diff-lines", "Show Diff for Lines",
        "Opens the diff of the changed lines of the marker popup or at the caret against their committed version (when a diff viewer is available).";
    /// Copy the committed text of the changed lines.
    CopyChangeLines => "vcs.copy-change-lines", "Copy",
        "Copies the committed text of the changed lines of the marker popup or at the caret to the clipboard.";
}

ui::ui_commands! {
    POPUP_COMMANDS, scope = "LineStatusPopup";
    /// Close the marker popup.
    CloseMarkerPopup => "marker-popup.close", "Close",
        "Closes the popup of a change marker.";
}

/// Registers the commands.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
}
