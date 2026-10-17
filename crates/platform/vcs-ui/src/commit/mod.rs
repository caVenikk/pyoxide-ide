//! The Commit tool window (#170): non-modal commit as the reference's
//! `ChangesViewCommitPanel` in the Commit tool window of the left area.
//!
//! - The header ("Commit", Options, Hide) and the changes toolbar:
//!   Refresh, Rollback…, Show Diff, Shelve Silently (not yet), View
//!   Options, Select Opened File, Expand All, Collapse All.
//! - The changes tree ([`tree`]): "Changes" and "Unversioned Files" with
//!   check boxes, file names in their status colour with the parent folder
//!   and "- renamed from …"; the context menu's subset.
//! - The status row: Amend with the "last commit" drop-down link, the
//!   commit message history button (the popup comes later) and the legend
//!   ([`legend`]).
//! - The message field ("Commit Message"), the errors "Select files to
//!   commit" and "Specify commit message", Commit and Commit and Push…
//!   (Amend Commit and Amend Commit and Push… while amending) and the
//!   commit options popup (Author, Sign-off commit, Run Git hooks).
//!
//! The view knows no backend: it emits [`CommitEvent`]s (commit, load the
//! last commit, rollback, add, open) and the workbench runs them with the
//! project's repository, then tells the view ([`CommitView::committed`],
//! [`CommitView::commit_failed`], [`CommitView::set_last_commit`]).

pub mod dialogs;
pub mod legend;
mod render;
pub mod tree;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use editor::{Editor, EditorConfig, EditorEvent};
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    Point, SharedString, Subscription, UniformListScrollHandle, Window,
};
use serde_json::{Value as Json, json};
use text::{Buffer, Selection, SelectionSet};
use ui::icons::IconName;
use ui::menu::ContextMenu;
use vcs::{CommitRequest, LastCommit, Snapshot};

use legend::Legend;
use tree::{ChangesTree, Group, Inclusion, Row};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Commit… (`CheckinProject`).
    Commit => "vcs.commit", "Commit\u{2026}",
        "Opens the Commit tool window with every change included and the focus in the commit message.";
    /// The Commit tool window (`ActivateCommitToolWindow`).
    ToggleCommitWindow => "workbench.toggle-commit", "Commit",
        "Shows the Commit tool window and moves the focus there, or hides it when it has the focus.";
    /// The Commit button.
    RunCommit { message: Option<String> } => "vcs.commit.run", "Commit",
        "Commits the included files with the message (replaced by `message` when given), as the Commit button (Amend Commit while amending) does; an empty message or no included file shows the error instead.",
        r#"{"message": "Fix"}"#;
    /// Commit and Push….
    RunCommitAndPush { message: Option<String> } => "vcs.commit.push", "Commit and Push\u{2026}",
        "Commits as the Commit button does, then pushes (when push is available).",
        r#"{"message": "Fix"}"#;
    /// Amend.
    ToggleAmend => "vcs.commit.toggle-amend", "Amend Commit",
        "Turns amending the last commit on (its message goes into the field) or off (the message before comes back).";
    /// "last commit" of the amend drop-down.
    AmendLast => "vcs.commit.amend-last", "Amend Last Commit",
        "Turns amending the last commit on (the item of the \"last commit\" drop-down).";
    /// The gear of the Commit tool window.
    ShowCommitOptions => "vcs.commit.options", "Show Commit Options",
        "Opens or closes the commit options popup: Author, Sign-off commit, Run Git hooks.";
    /// Type a commit message.
    SetCommitMessage { text: String } => "vcs.commit.set-message", "Set Commit Message",
        "Replaces the text of the commit message field.",
        r#"{"text": "Fix"}"#;
    /// Include files in the commit.
    SetIncluded { paths: Option<Vec<String>>, included: bool } => "vcs.commit.include", "Include into Commit",
        "Ticks (or with included false unticks) the check boxes of files of the Commit tool window, relative to the repository root; without paths every listed file.",
        r#"{"paths": ["main.py"], "included": true}"#;
    /// Rollback….
    Rollback { paths: Option<Vec<String>>, delete_added: Option<bool> } => "vcs.rollback", "Rollback\u{2026}",
        "Without paths opens the Rollback Changes dialog for the selected changes (the active file outside the Commit tool window); with paths (relative to the repository root) rolls them back at once, deleting added files when delete_added is true.",
        r#"{"paths": ["main.py"], "delete_added": false}"#;
    /// Add to VCS.
    AddToVcs { paths: Option<Vec<String>> } => "vcs.add", "Add to VCS",
        "Adds unversioned files to Git: the given paths (relative to the repository root), else the selected files of the Commit tool window or the active file.",
        r#"{"paths": ["new.py"]}"#;
    /// Refresh.
    RefreshChanges => "vcs.refresh", "Refresh",
        "Looks at the statuses of the work tree again.";
}

ui::ui_commands! {
    TREE_COMMANDS, scope = "CommitTree";
    /// Up.
    TreeUp => "commit-tree.up", "Up", "Selects the row above in the Commit tool window's tree.";
    /// Down.
    TreeDown => "commit-tree.down", "Down", "Selects the row below in the Commit tool window's tree.";
    /// Left.
    TreeLeft => "commit-tree.left", "Collapse or Go to Group",
        "Collapses the selected group, or selects the group of the selected file.";
    /// Right.
    TreeRight => "commit-tree.right", "Expand or Down",
        "Expands the selected group, or selects the row below.";
    /// Space.
    TreeToggle => "commit-tree.toggle", "Include or Exclude",
        "Ticks the check boxes of the selected rows, or unticks them when all are ticked.";
    /// Enter, double-click.
    TreeOpen => "commit-tree.open", "Open Diff",
        "Opens the diff of the selected file when a diff viewer is available, else the file.";
    /// Show Diff.
    TreeShowDiff => "commit-tree.show-diff", "Show Diff",
        "Opens the diff of the selected file when a diff viewer is available, else the file.";
    /// Jump to Source.
    TreeJumpToSource => "commit-tree.jump-to-source", "Jump to Source",
        "Opens the selected file in an editor.";
    /// Expand All.
    TreeExpandAll => "commit-tree.expand-all", "Expand All", "Expands every group of the Commit tool window's tree.";
    /// Collapse All.
    TreeCollapseAll => "commit-tree.collapse-all", "Collapse All", "Collapses every group of the Commit tool window's tree.";
    /// Select Opened File.
    TreeSelectOpened => "commit-tree.select-opened-file", "Select Opened File",
        "Selects the active editor's file in the Commit tool window's tree.";
    /// Select a row.
    TreeSelect { path: String } => "commit-tree.select", "Select",
        "Selects the row of a file (relative to the repository root) or a group (`changes`, `unversioned`).",
        r#"{"path": "main.py"}"#;
}

ui::ui_commands! {
    OPTIONS_COMMANDS, scope = "CommitOptions";
    /// Close the options popup.
    CloseOptions => "commit-options.close", "Close", "Closes the commit options popup.";
    /// Sign-off commit.
    ToggleSignOff => "commit-options.toggle-sign-off", "Sign-off Commit",
        "Toggles Sign-off commit of the commit options.";
    /// Run Git hooks.
    ToggleRunHooks => "commit-options.toggle-run-hooks", "Run Git Hooks",
        "Toggles Run Git hooks of the commit options (off commits with --no-verify).";
    /// Author.
    SetAuthor { author: String } => "commit-options.set-author", "Set Author",
        "Sets the Author field of the commit options (`Name <email>`, empty for the configured user).",
        r#"{"author": "Name <name@example.com>"}"#;
}

/// Key contexts of the tool window and its parts.
pub const KEY_CONTEXT: &str = "CommitToolWindow";
pub const TREE_KEY_CONTEXT: &str = "CommitTree";
pub const MESSAGE_KEY_CONTEXT: &str = "CommitMessage";
pub const OPTIONS_KEY_CONTEXT: &str = "CommitOptions";

/// `commit.message.placeholder`.
pub const PLACEHOLDER: &str = "Commit Message";
/// `error.no.changes.to.commit`, `error.no.commit.message`,
/// `error.no.changes.no.commit.message`.
pub const NO_CHANGES: &str = "Select files to commit";
pub const NO_MESSAGE: &str = "Specify commit message";
pub const NO_CHANGES_NO_MESSAGE: &str = "Select files to commit and specify commit message";
/// The empty tree (`diff.count.differences.status.text` with 0).
pub const EMPTY_TREE: &str = "No differences";
/// Recent commit messages kept (`VcsConfiguration.MAX_STORED_MESSAGES`).
pub const RECENT_MESSAGES: usize = 20;

/// The commit's name on its buttons and in its dialogs.
pub fn action_name(amend: bool) -> &'static str {
    if amend { "Amend Commit" } else { "Commit" }
}

/// The second button (`action.commit.and.push.text`,
/// `action.amend.commit.and.push.text`).
pub fn push_action_name(amend: bool) -> &'static str {
    if amend {
        "Amend Commit and Push\u{2026}"
    } else {
        "Commit and Push\u{2026}"
    }
}

/// What the workbench is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitEvent {
    /// Commit (and push) the request.
    Commit {
        request: CommitRequest,
        push: bool,
    },
    /// The last commit is needed (Amend).
    LoadLastCommit,
    /// Rollback… for these files.
    Rollback(Vec<PathBuf>),
    /// Add to VCS.
    Add(Vec<PathBuf>),
    /// Open the file (Jump to Source).
    Open(PathBuf),
    /// Open the diff of the file, or the file without a diff viewer.
    ShowDiff(PathBuf),
    Refresh,
    /// Select Opened File: the workbench selects the active file.
    SelectOpenedFile,
    /// The message was edited (its draft is kept).
    MessageEdited,
    /// The Hide button.
    Hide,
}

/// Where the repository is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    /// The project has no repository (Create Git repository…).
    None,
    /// Statuses are being read.
    Loading,
    Ready,
}

/// A problem shown above the buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoChanges,
    NoMessage,
    NoChangesNoMessage,
    /// A commit check that stopped the commit.
    Check(String),
}

impl Problem {
    pub fn text(&self) -> &str {
        match self {
            Problem::NoChanges => NO_CHANGES,
            Problem::NoMessage => NO_MESSAGE,
            Problem::NoChangesNoMessage => NO_CHANGES_NO_MESSAGE,
            Problem::Check(text) => text,
        }
    }
}

/// The commit options (`GitCommitOptionsUi`, the skip hooks option).
pub struct CommitOptions {
    pub open: bool,
    pub author: Entity<Editor>,
    pub sign_off: bool,
    pub run_hooks: bool,
    /// The repository has commit hooks: "Run Git hooks" shows.
    pub hooks: bool,
}

/// An open menu of the view and where it is.
struct OpenMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

/// Amend: the message before it and the one loaded, to restore it.
#[derive(Debug, Clone)]
struct AmendData {
    before: String,
    amend: String,
}

/// The Commit tool window's view.
pub struct CommitView {
    focus: FocusHandle,
    tree_focus: FocusHandle,
    options_focus: FocusHandle,
    tree: ChangesTree,
    state: RepoState,
    message: Entity<Editor>,
    file_icon: Rc<dyn Fn(&Path) -> IconName>,
    /// Files the IDE ignores (byte code, tool caches): never listed.
    hidden: Rc<dyn Fn(&Path) -> bool>,
    amend: bool,
    amend_data: Option<AmendData>,
    /// Amend is on and waits for the last commit.
    amend_pending: bool,
    last_commit: Option<LastCommit>,
    problem: Option<Problem>,
    busy: bool,
    push_available: bool,
    options: CommitOptions,
    scroll: UniformListScrollHandle,
    /// Where the gear and the "last commit" link and the View Options
    /// button are (popups open next to them), and the view's width (the
    /// legend shortens when it does not fit).
    gear_bounds: Rc<Cell<Bounds<Pixels>>>,
    amend_link_bounds: Rc<Cell<Bounds<Pixels>>>,
    view_options_bounds: Rc<Cell<Bounds<Pixels>>>,
    width: Rc<Cell<Pixels>>,
    menu: Option<OpenMenu>,
    /// Text changes of our own do not count as edits.
    setting_message: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CommitEvent> for CommitView {}

impl Focusable for CommitView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A message field: an editor without line numbers, wrapping its lines,
/// with the placeholder while it is empty.
fn message_editor(placeholder: Option<&str>, window: &mut Window, cx: &mut App) -> Entity<Editor> {
    cx.new(|cx| {
        let mut e = Editor::new(Buffer::new(""), window, cx);
        e.set_console(true);
        e.set_transparent(true);
        e.set_placeholder(placeholder.map(|p| SharedString::from(p.to_owned())));
        e.set_config(EditorConfig {
            brackets: Vec::new(),
            quotes: Vec::new(),
            ..EditorConfig::default()
        });
        e.set_soft_wraps(true, cx);
        e
    })
}

impl CommitView {
    /// A view whose file rows get their icons from `file_icon`; files for
    /// which `hidden` holds are not listed (the reference's ignored files
    /// and folders, which version control never sees).
    pub fn new(
        file_icon: Rc<dyn Fn(&Path) -> IconName>,
        hidden: Rc<dyn Fn(&Path) -> bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let message = message_editor(Some(PLACEHOLDER), window, cx);
        let author = editor::field::new("", None, true, window, cx);
        let subscriptions = vec![cx.subscribe(&message, |this: &mut Self, _, event, cx| {
            if matches!(event, EditorEvent::Edited(_)) && !this.setting_message {
                this.message_edited(cx);
            }
        })];
        Self {
            focus: cx.focus_handle(),
            tree_focus: cx.focus_handle(),
            options_focus: cx.focus_handle(),
            tree: ChangesTree::default(),
            state: RepoState::None,
            message,
            file_icon,
            hidden,
            amend: false,
            amend_data: None,
            amend_pending: false,
            last_commit: None,
            problem: None,
            busy: false,
            push_available: false,
            options: CommitOptions {
                open: false,
                author,
                sign_off: false,
                run_hooks: true,
                hooks: false,
            },
            scroll: UniformListScrollHandle::new(),
            gear_bounds: Rc::default(),
            amend_link_bounds: Rc::default(),
            view_options_bounds: Rc::default(),
            width: Rc::default(),
            menu: None,
            setting_message: false,
            _subscriptions: subscriptions,
        }
    }

    // ---- state from the workbench -------------------------------------------

    /// The repository's statuses; `None` without a repository, `loading`
    /// while they are read.
    pub fn set_snapshot(
        &mut self,
        snapshot: Option<&Arc<Snapshot>>,
        loading: bool,
        cx: &mut Context<Self>,
    ) {
        match snapshot {
            Some(s) => {
                self.state = RepoState::Ready;
                // `hidden` looks at the path below the work tree's root.
                let root = s.info.root.as_path();
                let hidden = |p: &Path| (self.hidden)(p.strip_prefix(root).unwrap_or(p));
                let mut changes = s.status.changes();
                changes.retain(|c| !hidden(&c.path));
                let mut unversioned = s.status.unversioned();
                unversioned.retain(|p| !hidden(p));
                self.tree.set_changes(&s.info.root, changes, unversioned);
            }
            None => {
                self.state = if loading {
                    RepoState::Loading
                } else {
                    RepoState::None
                };
                self.tree = ChangesTree::default();
            }
        }
        cx.notify();
    }

    /// The last commit of `HEAD` (Amend's message and drop-down).
    pub fn set_last_commit(&mut self, commit: Option<LastCommit>, cx: &mut Context<Self>) {
        self.last_commit = commit;
        if self.amend_pending {
            self.amend_pending = false;
            self.apply_amend_message(cx);
        }
        cx.notify();
    }

    pub fn last_commit(&self) -> Option<&LastCommit> {
        self.last_commit.as_ref()
    }

    /// Commit and Push… can push (the push command is there); without it
    /// the button commits only.
    pub fn set_push_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.push_available != available {
            self.push_available = available;
            cx.notify();
        }
    }

    /// The repository has commit hooks ("Run Git hooks" shows).
    pub fn set_hooks(&mut self, hooks: bool, cx: &mut Context<Self>) {
        self.options.hooks = hooks;
        cx.notify();
    }

    /// The commit was made: the message is cleared, Amend goes off.
    pub fn committed(&mut self, cx: &mut Context<Self>) {
        self.busy = false;
        self.amend = false;
        self.amend_data = None;
        self.amend_pending = false;
        self.problem = None;
        self.options
            .author
            .update(cx, |e, cx| e.set_text_selected("", cx));
        self.set_message("", cx);
        cx.notify();
    }

    /// The commit did not happen; a check's text shows above the buttons.
    pub fn commit_failed(&mut self, check: Option<String>, cx: &mut Context<Self>) {
        self.busy = false;
        self.problem = check.map(Problem::Check);
        cx.notify();
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    pub fn tree(&self) -> &ChangesTree {
        &self.tree
    }

    pub fn state(&self) -> RepoState {
        self.state
    }

    pub fn is_amend(&self) -> bool {
        self.amend
    }

    pub fn problem(&self) -> Option<&Problem> {
        self.problem.as_ref()
    }

    pub fn message_editor(&self) -> &Entity<Editor> {
        &self.message
    }

    pub fn message_text(&self, cx: &App) -> String {
        self.message.read(cx).text()
    }

    /// Where Commit… (⌘K) puts the focus.
    pub fn message_focus(&self, cx: &App) -> FocusHandle {
        self.message.focus_handle(cx)
    }

    /// Where activating the tool window puts the focus: the tree.
    pub fn tree_focus(&self) -> FocusHandle {
        self.tree_focus.clone()
    }

    /// Replaces the message (a draft, Amend), the caret at its end.
    pub fn set_message(&mut self, text: &str, cx: &mut Context<Self>) {
        self.setting_message = true;
        self.message.update(cx, |e, cx| {
            e.set_text_selected(text, cx);
            let end = text.chars().count();
            e.set_selections(SelectionSet::single(Selection::caret(end)), cx);
        });
        self.setting_message = false;
        cx.notify();
    }

    /// Commit… (⌘K): every change included.
    pub fn include_all_changes(&mut self, cx: &mut Context<Self>) {
        self.tree.include_all_changes();
        self.problem = None;
        cx.notify();
    }

    /// Selects a file's row (Select Opened File).
    pub fn select_path(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let found = self.tree.select_path(path);
        if found {
            self.reveal_lead();
            cx.notify();
        }
        found
    }

    /// The files of the selected rows.
    pub fn selected_files(&self) -> Vec<PathBuf> {
        self.tree.selected_files()
    }

    /// The tree or another part of the window has the focus.
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    pub fn tree_has_focus(&self, window: &Window) -> bool {
        self.tree_focus.is_focused(window)
    }

    // ---- editing ------------------------------------------------------------

    fn message_edited(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.problem,
            Some(Problem::NoMessage | Problem::NoChangesNoMessage)
        ) {
            self.problem = None;
        }
        cx.emit(CommitEvent::MessageEdited);
        cx.notify();
    }

    fn inclusion_changed(&mut self, cx: &mut Context<Self>) {
        // `CommitProgressPanel.clearError` on inclusion changes.
        if matches!(
            self.problem,
            Some(Problem::NoChanges | Problem::NoChangesNoMessage)
        ) {
            self.problem = None;
        }
        cx.notify();
    }

    /// The legend of the included files.
    pub fn legend(&self) -> Legend {
        Legend::of(
            &self.tree.included_changes(),
            self.tree.included_unversioned().len(),
        )
    }

    // ---- commit -------------------------------------------------------------

    /// The Commit or Commit and Push… button: the request goes to the
    /// workbench unless nothing is included or the message is empty.
    fn run(
        &mut self,
        message: Option<&str>,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = message {
            self.set_message(text, cx);
        }
        if self.busy || self.state != RepoState::Ready {
            return;
        }
        let changes = self.tree.included_changes();
        let unversioned = self.tree.included_unversioned();
        let text = self.message_text(cx);
        let no_changes = changes.is_empty() && unversioned.is_empty() && !self.amend;
        let no_message = text.trim().is_empty();
        self.problem = match (no_changes, no_message) {
            (true, true) => Some(Problem::NoChangesNoMessage),
            (true, false) => Some(Problem::NoChanges),
            (false, true) => Some(Problem::NoMessage),
            (false, false) => None,
        };
        if self.problem.is_some() {
            cx.notify();
            return;
        }
        let author = self.options.author.read(cx).text();
        let request = CommitRequest {
            message: text,
            changes,
            unversioned,
            amend: self.amend,
            sign_off: self.options.sign_off,
            run_hooks: self.options.run_hooks || !self.options.hooks,
            author: (!author.trim().is_empty()).then(|| author.trim().to_owned()),
        };
        self.busy = true;
        self.close_options(window, cx);
        cx.emit(CommitEvent::Commit { request, push });
        cx.notify();
    }

    fn run_commit(&mut self, action: &RunCommit, window: &mut Window, cx: &mut Context<Self>) {
        self.run(action.message.as_deref(), false, window, cx);
    }

    fn run_commit_and_push(
        &mut self,
        action: &RunCommitAndPush,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run(action.message.as_deref(), true, window, cx);
    }

    fn set_commit_message(
        &mut self,
        action: &SetCommitMessage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_message(&action.text, cx);
        self.message_edited(cx);
    }

    fn set_included_action(
        &mut self,
        action: &SetIncluded,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<PathBuf> = match &action.paths {
            Some(paths) => paths.iter().map(|p| self.resolve(p)).collect(),
            None => {
                let mut all = self.tree.files(Group::Changes);
                all.extend(self.tree.files(Group::Unversioned));
                all
            }
        };
        self.tree.set_included(&paths, action.included);
        self.inclusion_changed(cx);
    }

    /// A path of a command: relative to the repository's root (`/`), or
    /// absolute.
    fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            return p.to_owned();
        }
        let mut out = self.tree.root().to_path_buf();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            out.push(part);
        }
        out
    }

    // ---- amend --------------------------------------------------------------

    fn toggle_amend(&mut self, _: &ToggleAmend, window: &mut Window, cx: &mut Context<Self>) {
        self.set_amend(!self.amend, window, cx);
    }

    fn amend_last(&mut self, _: &AmendLast, window: &mut Window, cx: &mut Context<Self>) {
        self.set_amend(true, window, cx);
    }

    /// Amend on: the last commit's message replaces the one in the field
    /// (kept to come back); off: the message before comes back unless it
    /// was edited meanwhile (`AmendCommitHandlerImpl`).
    pub fn set_amend(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.amend == on || self.state != RepoState::Ready {
            return;
        }
        self.amend = on;
        if matches!(
            self.problem,
            Some(Problem::NoChanges | Problem::NoChangesNoMessage)
        ) {
            self.problem = None;
        }
        if on {
            self.amend_pending = true;
            cx.emit(CommitEvent::LoadLastCommit);
        } else {
            self.amend_pending = false;
            if let Some(data) = self.amend_data.take()
                && self.message_text(cx) == data.amend
            {
                self.set_message(&data.before, cx);
                cx.emit(CommitEvent::MessageEdited);
            }
            window.focus(&self.message.focus_handle(cx));
        }
        cx.notify();
    }

    fn apply_amend_message(&mut self, cx: &mut Context<Self>) {
        if !self.amend {
            return;
        }
        let Some(commit) = self.last_commit.clone() else {
            return;
        };
        let before = self.message_text(cx);
        let same = before
            .split_whitespace()
            .eq(commit.message.split_whitespace());
        if !same {
            self.setting_message = true;
            self.message.update(cx, |e, cx| {
                e.set_text_selected(&commit.message, cx);
                let end = commit.message.chars().count();
                e.set_selections(SelectionSet::single(Selection::caret(end)), cx);
            });
            self.setting_message = false;
            self.amend_data = Some(AmendData {
                before,
                amend: commit.message,
            });
            cx.emit(CommitEvent::MessageEdited);
        }
        cx.notify();
    }

    /// The message before Amend took the last commit's (it goes to the
    /// recent messages, as the reference saves it).
    pub fn message_before_amend(&self) -> Option<&str> {
        self.amend_data.as_ref().map(|d| d.before.as_str())
    }

    // ---- options ------------------------------------------------------------

    fn show_options(&mut self, _: &ShowCommitOptions, window: &mut Window, cx: &mut Context<Self>) {
        if self.options.open {
            self.close_options(window, cx);
        } else {
            self.options.open = true;
            window.focus(&self.options.author.focus_handle(cx));
            cx.notify();
        }
    }

    fn close_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.options.open {
            return;
        }
        self.options.open = false;
        if self.options_focus.contains_focused(window, cx) {
            window.focus(&self.message.focus_handle(cx));
        }
        cx.notify();
    }

    fn close_options_action(
        &mut self,
        _: &CloseOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_options(window, cx);
    }

    fn toggle_sign_off(&mut self, _: &ToggleSignOff, _: &mut Window, cx: &mut Context<Self>) {
        self.options.sign_off = !self.options.sign_off;
        cx.notify();
    }

    fn toggle_run_hooks(&mut self, _: &ToggleRunHooks, _: &mut Window, cx: &mut Context<Self>) {
        self.options.run_hooks = !self.options.run_hooks;
        cx.notify();
    }

    fn set_author(&mut self, action: &SetAuthor, _: &mut Window, cx: &mut Context<Self>) {
        self.options
            .author
            .update(cx, |e, cx| e.set_text_selected(&action.author, cx));
        cx.notify();
    }

    /// Sign-off commit as the repository's settings keep it.
    pub fn set_sign_off(&mut self, on: bool, cx: &mut Context<Self>) {
        self.options.sign_off = on;
        cx.notify();
    }

    pub fn sign_off(&self) -> bool {
        self.options.sign_off
    }

    // ---- the tree -----------------------------------------------------------

    fn reveal_lead(&mut self) {
        let rows = self.tree.rows();
        if let Some(i) = self
            .tree
            .lead()
            .and_then(|l| rows.iter().position(|r| r == l))
        {
            self.scroll.scroll_to_item(i, gpui::ScrollStrategy::Center);
        }
    }

    fn tree_up(&mut self, _: &TreeUp, _: &mut Window, cx: &mut Context<Self>) {
        self.tree.move_selection(-1);
        self.reveal_lead();
        cx.notify();
    }

    fn tree_down(&mut self, _: &TreeDown, _: &mut Window, cx: &mut Context<Self>) {
        self.tree.move_selection(1);
        self.reveal_lead();
        cx.notify();
    }

    fn tree_left(&mut self, _: &TreeLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.tree.left();
        self.reveal_lead();
        cx.notify();
    }

    fn tree_right(&mut self, _: &TreeRight, _: &mut Window, cx: &mut Context<Self>) {
        self.tree.right();
        self.reveal_lead();
        cx.notify();
    }

    fn tree_toggle(&mut self, _: &TreeToggle, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.tree.selected_rows();
        self.tree.toggle(&rows);
        self.inclusion_changed(cx);
    }

    /// The selected file whose diff or source opens.
    fn selected_file(&self) -> Option<PathBuf> {
        match self.tree.lead() {
            Some(Row::File(p)) => Some(p.clone()),
            _ => self.tree.selected_rows().into_iter().find_map(|r| match r {
                Row::File(p) => Some(p),
                Row::Group(_) => None,
            }),
        }
    }

    fn tree_open(&mut self, _: &TreeOpen, _: &mut Window, cx: &mut Context<Self>) {
        match self.tree.lead().cloned() {
            Some(Row::Group(g)) => {
                let expanded = self.tree.is_expanded(g);
                self.tree.set_expanded(g, !expanded);
                cx.notify();
            }
            _ => {
                if let Some(path) = self.selected_file() {
                    cx.emit(CommitEvent::ShowDiff(path));
                }
            }
        }
    }

    fn tree_show_diff(&mut self, _: &TreeShowDiff, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_file() {
            cx.emit(CommitEvent::ShowDiff(path));
        }
    }

    fn tree_jump_to_source(
        &mut self,
        _: &TreeJumpToSource,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.selected_file() {
            cx.emit(CommitEvent::Open(path));
        }
    }

    fn tree_expand_all(&mut self, _: &TreeExpandAll, _: &mut Window, cx: &mut Context<Self>) {
        self.tree.expand_all();
        cx.notify();
    }

    fn tree_collapse_all(&mut self, _: &TreeCollapseAll, _: &mut Window, cx: &mut Context<Self>) {
        self.tree.collapse_all();
        cx.notify();
    }

    fn tree_select_opened(&mut self, _: &TreeSelectOpened, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommitEvent::SelectOpenedFile);
    }

    fn tree_select(&mut self, action: &TreeSelect, _: &mut Window, cx: &mut Context<Self>) {
        let row = match action.path.as_str() {
            "changes" => Row::Group(Group::Changes),
            "unversioned" => Row::Group(Group::Unversioned),
            path => {
                let path = self.resolve(path);
                if let Some(group) = self.tree.group_of(&path) {
                    self.tree.set_expanded(group, true);
                }
                Row::File(path)
            }
        };
        if self.tree.rows().contains(&row) {
            self.tree.select(&row);
            self.reveal_lead();
            cx.notify();
        }
    }

    fn refresh(&mut self, _: &RefreshChanges, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommitEvent::Refresh);
    }

    // ---- menus --------------------------------------------------------------

    fn open_menu(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe(&menu, |this, _, _: &gpui::DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The open menu (context menu, View Options, the amend drop-down).
    pub fn menu(&self) -> Option<&Entity<ContextMenu>> {
        self.menu.as_ref().map(|m| &m.menu)
    }

    // ---- state --------------------------------------------------------------

    /// The view for `vcs.state` (`commit`).
    pub fn state_json(&self, window: Option<&Window>, cx: &App) -> Json {
        let root = self.tree.root();
        let rel = |p: &Path| {
            p.strip_prefix(root)
                .map(|r| {
                    r.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_else(|_| p.display().to_string())
        };
        let rows: Vec<Json> = self
            .tree
            .rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => json!({
                    "kind": "group",
                    "group": g.id(),
                    "title": g.title(),
                    "count": tree::file_count(self.tree.files(*g).len()),
                    "included": self.tree.inclusion(*g).id(),
                    "expanded": self.tree.is_expanded(*g),
                    "selected": self.tree.is_selected(row),
                }),
                Row::File(p) => {
                    let change = self.tree.change(p);
                    json!({
                        "kind": "file",
                        "path": rel(p),
                        "name": p.file_name().map(|n| n.to_string_lossy().into_owned()),
                        "status": change.map_or("unversioned", |c| c.status.id()),
                        "origin": change.and_then(|c| tree::origin_text(c, root)),
                        "parent": tree::parent_text(p, root),
                        "included": self.tree.row_inclusion(row) == Inclusion::All,
                        "selected": self.tree.is_selected(row),
                    })
                }
            })
            .collect();
        let legend = self.legend();
        let focused = window.map(|w| {
            if self.tree_focus.is_focused(w) {
                "tree"
            } else if self.message.focus_handle(cx).is_focused(w) {
                "message"
            } else if self.options_focus.contains_focused(w, cx)
                || self.options.author.focus_handle(cx).is_focused(w)
            {
                "options"
            } else {
                ""
            }
        });
        json!({
            "state": match self.state {
                RepoState::None => "no_repository",
                RepoState::Loading => "loading",
                RepoState::Ready => "ready",
            },
            "rows": rows,
            "empty_text": (self.state == RepoState::Ready && self.tree.is_empty()).then_some(EMPTY_TREE),
            "message": self.message_text(cx),
            "placeholder": PLACEHOLDER,
            "amend": self.amend,
            "amend_loading": self.amend_pending,
            "last_commit": self.last_commit.as_ref().map(|c| c.subject.clone()),
            "legend": legend.text(false),
            "legend_compact": legend.text(true),
            "problem": self.problem.as_ref().map(|p| p.text().to_owned()),
            "commit_button": action_name(self.amend),
            "push_button": push_action_name(self.amend),
            "push_enabled": self.push_available,
            "busy": self.busy,
            "focused": focused.filter(|f| !f.is_empty()),
            "options": {
                "open": self.options.open,
                "author": self.options.author.read(cx).text(),
                "sign_off": self.options.sign_off,
                "run_hooks": self.options.run_hooks,
                "hooks": self.options.hooks,
            },
            "menu": self.menu.as_ref().map(|m| m.menu.read(cx).state_json()),
        })
    }
}

macro_rules! commit_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl CommitView {
            fn bind_actions<E: gpui::InteractiveElement>(el: E, cx: &mut Context<Self>) -> E {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }

        /// Forwards the commands of the Commit tool window to `view` (the
        /// workbench, when the focus is elsewhere).
        pub fn forward_actions<E: gpui::InteractiveElement>(el: E, view: Entity<CommitView>) -> E {
            el $(.on_action({
                let view = view.clone();
                move |a: &$action, window, cx| view.update(cx, |v, cx| v.$method(a, window, cx))
            }))*
        }
    };
}

commit_actions! {
    RunCommit => run_commit,
    RunCommitAndPush => run_commit_and_push,
    ToggleAmend => toggle_amend,
    AmendLast => amend_last,
    ShowCommitOptions => show_options,
    SetCommitMessage => set_commit_message,
    SetIncluded => set_included_action,
    RefreshChanges => refresh,
    TreeUp => tree_up,
    TreeDown => tree_down,
    TreeLeft => tree_left,
    TreeRight => tree_right,
    TreeToggle => tree_toggle,
    TreeOpen => tree_open,
    TreeShowDiff => tree_show_diff,
    TreeJumpToSource => tree_jump_to_source,
    TreeExpandAll => tree_expand_all,
    TreeCollapseAll => tree_collapse_all,
    TreeSelectOpened => tree_select_opened,
    TreeSelect => tree_select,
    CloseOptions => close_options_action,
    ToggleSignOff => toggle_sign_off,
    ToggleRunHooks => toggle_run_hooks,
    SetAuthor => set_author,
}

/// Registers the commands.
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    ui::command::register(cx, TREE_COMMANDS);
    ui::command::register(cx, OPTIONS_COMMANDS);
    ui::command::register(cx, dialogs::ROLLBACK_COMMANDS);
    ui::command::register(cx, dialogs::USER_NAME_COMMANDS);
}
