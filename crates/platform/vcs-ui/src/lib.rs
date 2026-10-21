//! Version control UI of Pyoxide IDE (ADR 0016) on GPUI. It knows the
//! model (`vcs`), never a backend: the workbench gives it statuses and base
//! texts.
//!
//! - [`status_colors`]: file names in the colour of their status
//!   (`FILESTATUS_*`), for the project tree, editor tabs and file lists.
//! - [`markers`]: an editor's change markers against its base text, the
//!   error stripe marks, Next / Previous Change and Rollback Lines.
//! - [`marker_popup`]: the popup of a change marker.
//! - [`widget`]: what the VCS widget of the main toolbar shows.
//! - [`branches`]: the branches popup of the widget.
//! - [`new_branch`]: the New Branch, Rename Branch and Checkout dialogs.
//! - [`commit`]: the Commit tool window, the Rollback Changes and Git User
//!   Name Is Not Defined dialogs.
//! - [`push`]: the Push dialog; [`update`]: the Update Project dialog;
//!   [`operations`]: the VCS Operations popup.
//! - [`diff`]: the side-by-side diff viewer of an editor tab.
//! - The `vcs.*` commands of the editor ([`COMMANDS`]) and of the popup
//!   ([`POPUP_COMMANDS`]), the branch commands ([`BRANCH_COMMANDS`]) and
//!   the remote ones ([`REMOTE_COMMANDS`]); the workbench handles them.

pub mod branches;
pub mod commit;
pub mod diff;
pub mod marker_popup;
pub mod markers;
pub mod new_branch;
pub mod operations;
pub mod push;
pub mod status_colors;
pub mod update;
pub mod widget;

pub use marker_popup::{
    MarkerPopup, PopupAction, PopupText, doc_highlights, popup_text, render_popup,
};
pub use markers::{Computed, LineStatus, LineStatusEvent, Part};
pub use status_colors::{FileColors, NameColor, OpenDocument, entry_color, status_role};

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

ui::ui_commands! {
    BRANCH_COMMANDS, scope = "Workbench";
    /// The branches popup.
    ShowBranches => "vcs.branches-popup", "Branches\u{2026}",
        "Opens the popup of the VCS widget: the branches and their actions, or, without a repository, Create Git Repository\u{2026}.";
    /// New Branch….
    NewBranch {
        name: Option<String>,
        from: Option<String>,
        checkout: Option<bool>,
    } => "git.new-branch", "New Branch\u{2026}",
        "Creates a branch from the current one (or from `from`) and checks it out unless checkout is false; without a name the New Branch dialog asks for it.",
        r#"{"name": "feat/x", "from": "main"}"#;
    /// Checkout.
    Checkout { r#ref: String, mode: Option<String> } => "git.checkout", "Checkout",
        "Checks out a local branch, a remote branch as a new local branch tracking it, a tag or a commit; local changes in the way ask what to do (Git Checkout Problem) unless mode is force or smart.",
        r#"{"ref": "main"}"#;
    /// Checkout Tag or Revision….
    CheckoutRevision { r#ref: Option<String> } => "git.checkout-revision", "Checkout Tag or Revision\u{2026}",
        "Checks out a tag, a branch or a commit with HEAD detached; without a reference the Checkout dialog asks for it.",
        r#"{"ref": "v1.0"}"#;
    /// Rename….
    RenameBranch { from: Option<String>, to: Option<String> } => "git.rename-branch", "Rename Branch\u{2026}",
        "Renames a local branch (the current one by default); without a new name the Rename Branch dialog asks for it.",
        r#"{"from": "feat/x", "to": "feat/y"}"#;
    /// Delete.
    DeleteBranch { name: String } => "git.delete-branch", "Delete Branch",
        "Deletes a local branch, merged or not; the balloon offers Restore.",
        r#"{"name": "feat/x"}"#;
    /// Restore a deleted branch.
    RestoreBranch { name: String, oid: String } => "git.restore-branch", "Restore Branch",
        "Creates a deleted branch again at its commit (the Restore link of the Deleted Branch balloon).",
        r#"{"name": "feat/x", "oid": "0123456789abcdef0123456789abcdef01234567"}"#;
    /// Create Git Repository….
    Init { path: Option<String> } => "git.init", "Create Git Repository\u{2026}",
        "Creates a Git repository (git init) in a folder; without a path a folder chooser opens at the project folder.",
        r#"{"path": "."}"#;
    /// Describe version control.
    State => "vcs.state", "Version Control State",
        "Returns the project's version control as JSON: the repository and its statuses, the VCS widget (text, icon, tooltip, arrows), the open branches popup (search, rows) and dialog.";
}

ui::ui_commands! {
    REMOTE_COMMANDS, scope = "Workbench";
    /// Push….
    VcsPush {
        branch: Option<String>,
        target: Option<String>,
        force: Option<String>,
        tags: Option<String>,
        set_upstream: Option<bool>,
        dialog: Option<bool>,
    } => "vcs.push", "Push\u{2026}",
        "Opens the Push dialog for the current branch (or `branch`); with dialog false pushes at once to `target` (origin/main) or the default target, force none, with_lease or force, tags all or current_branch.",
        r#"{"branch": "main", "dialog": false}"#;
    /// Update Project….
    UpdateProject { method: Option<String>, dialog: Option<bool> } => "vcs.update-project", "Update Project\u{2026}",
        "Fetches the tracked branch of the current branch and merges or rebases the incoming commits (local changes stashed around it); the Update Project dialog asks for the method unless Don't show again was chosen or a method is given.",
        r#"{"method": "rebase"}"#;
    /// Fetch.
    Fetch { remote: Option<String> } => "git.fetch", "Fetch",
        "Fetches every remote of the repository (or one), pruning the remote branches that are gone.",
        r#"{"remote": "origin"}"#;
    /// A branch's Update.
    UpdateBranch { name: String } => "git.update-branch", "Update",
        "Updates a local branch from its tracked branch: the current branch as Update Project does with the chosen method, another one by a fast-forward fetch.",
        r#"{"name": "main"}"#;
    /// Pull into the current branch.
    Pull { from: String, method: String } => "git.pull", "Pull",
        "Pulls a remote branch into the current branch using merge or rebase (Pull into '…' Using Merge of a remote branch).",
        r#"{"from": "origin/main", "method": "rebase"}"#;
    /// VCS Operations.
    ShowOperations => "vcs.operations-popup", "VCS Operations Popup\u{2026}",
        "Opens the VCS Operations popup: commit, rollback, branches, push and the other version control actions.";
}

/// Registers the commands.
pub fn init(cx: &mut gpui::App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, POPUP_COMMANDS);
    ui::command::register(cx, BRANCH_COMMANDS);
    ui::command::register(cx, branches::COMMANDS);
    ui::command::register(cx, branches::INTERNAL_COMMANDS);
    ui::command::register(cx, new_branch::COMMANDS);
    commit::init(cx);
    ui::command::register(cx, REMOTE_COMMANDS);
    ui::command::register(cx, push::COMMANDS);
    ui::command::register(cx, update::COMMANDS);
    diff::init(cx);
}
