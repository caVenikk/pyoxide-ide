//! The Commit tool window (#170) in the workbench: the view of
//! `vcs_ui::commit` as the content of the left area's Commit tool window,
//! fed with the repository's statuses, and what it asks for done with the
//! project's [`git::GitRepo`] off the UI thread: commit (after "Git User
//! Name Is Not Defined" and "Commit in Detached HEAD" when they apply),
//! the last commit for Amend, rollback ("Rollback Changes"), Add to VCS,
//! opening a file or its diff. The commit message draft and the recent
//! messages live in the personal project state (`[vcs]`, ADR 0009).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement, IntoElement, Subscription, Task, Window,
};
use serde_json::{Value as Json, json};
use ui::icons::IconName;
use ui::notification::{Notification, NotificationKind};
use vcs::{Cancel, Change, CommitRequest, FileStatus, Head};
use vcs_ui::commit::dialogs::{RollbackDialog, RollbackEvent, UserNameDialog, UserNameEvent};
use vcs_ui::commit::{
    AddToVcs, Commit, CommitEvent, CommitView, RECENT_MESSAGES, Rollback, ToggleCommitWindow,
    action_name,
};

use super::Workbench;
use crate::dialog::{Dialog, DialogButton, DialogEvent};
use crate::tool_windows::{
    Area, ToolWindow, ToolWindowContent, ToolWindowSpec, ToolWindowView, ToolWindows,
};

/// The Commit tool window's id.
pub const COMMIT: ToolWindow = ToolWindow("commit");

/// The personal state's table and keys.
const TABLE: &str = "vcs";
const DRAFT_KEY: &str = "commit-message";
const RECENT_KEY: &str = "recent-commit-messages";
const SIGN_OFF_KEY: &str = "sign-off-commit";
/// Edits of the message are written this long after the last one.
const DRAFT_DELAY: Duration = Duration::from_millis(500);

impl ToolWindowView for CommitView {
    fn content_focus(&self, _: &App) -> FocusHandle {
        self.tree_focus()
    }
}

/// A dialog of the commit window and what it is for.
enum OpenDialog {
    Rollback {
        dialog: Entity<RollbackDialog>,
        return_focus: Option<FocusHandle>,
        _subscription: Subscription,
    },
    UserName {
        dialog: Entity<UserNameDialog>,
        request: CommitRequest,
        push: bool,
        return_focus: Option<FocusHandle>,
        _subscription: Subscription,
    },
    Detached {
        dialog: Entity<Dialog>,
        request: CommitRequest,
        push: bool,
        return_focus: Option<FocusHandle>,
        _subscription: Subscription,
    },
}

/// The Commit tool window's state in the workbench.
pub(super) struct CommitUi {
    pub(super) view: Entity<CommitView>,
    dialog: Option<OpenDialog>,
    /// The last state of "Delete local copies of added files".
    delete_added: bool,
    /// The `HEAD` whose last commit the view has.
    head: Option<String>,
    _draft_save: Option<Task<()>>,
    _subscription: Subscription,
}

impl CommitUi {
    /// The view, registered as the content of the Commit tool window.
    pub(super) fn new(
        file_types: Rc<crate::file_types::FileTypes>,
        tool_windows: &mut ToolWindows,
        window: &mut Window,
        cx: &mut Context<Workbench>,
    ) -> Self {
        let icons = file_types.clone();
        let icon: Rc<dyn Fn(&Path) -> IconName> = Rc::new(move |p: &Path| icons.icon(p));
        // The reference's "Ignored Files and Folders" (byte code, caches):
        // a file below such a folder is not listed either.
        let ignored = project::NameMasks::new(project::DEFAULT_IGNORED);
        let hidden: Rc<dyn Fn(&Path) -> bool> = Rc::new(move |p: &Path| {
            p.components()
                .any(|c| ignored.matches(&c.as_os_str().to_string_lossy()))
        });
        let view = cx.new(|cx| CommitView::new(icon, hidden, window, cx));
        tool_windows.register(
            ToolWindowSpec {
                id: COMMIT,
                area: Area::Left,
                order: 10,
                icon: IconName::TOOL_COMMIT,
                title: "Commit",
                command: Some("workbench.toggle-commit"),
                registered: true,
            },
            Some(ToolWindowContent::new(view.clone())),
        );
        let subscription = cx.subscribe_in(&view, window, |this, _, event, window, cx| {
            this.commit_event(event.clone(), window, cx)
        });
        Self {
            view,
            dialog: None,
            delete_added: false,
            head: None,
            _draft_save: None,
            _subscription: subscription,
        }
    }
}

/// `{0} files committed` (`vcs.commit.files.committed`).
fn committed_title(files: usize) -> String {
    match files {
        0 => "No files committed".to_owned(),
        1 => "1 file committed".to_owned(),
        n => format!("{n} files committed"),
    }
}

/// `warning.message.commit.with.detached.head`, as text.
fn detached_message(root: &Path) -> String {
    format!(
        "The Git repository at the following path is in the detached HEAD state:\n{}\n\
         You can look around, make experimental changes and commit them, but be sure to \
         checkout a branch not to lose your work.\nOtherwise you risk losing your changes.",
        root.display()
    )
}

/// The user's name for the user name dialog (`SystemProperties.getUserName`).
fn system_user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

impl Workbench {
    /// The commit commands on the window's root.
    pub(super) fn commit_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        let root = root
            .on_action(cx.listener(Self::vcs_commit))
            .on_action(cx.listener(Self::vcs_toggle_commit))
            .on_action(cx.listener(Self::vcs_rollback))
            .on_action(cx.listener(Self::vcs_add));
        vcs_ui::commit::forward_actions(root, self.commit_ui.view.clone())
    }

    /// The Commit tool window's view.
    pub fn commit_view(&self) -> &Entity<CommitView> {
        &self.commit_ui.view
    }

    /// The repository's statuses reached the view (or there is none).
    pub(super) fn vcs_commit_snapshot(&mut self, cx: &mut Context<Self>) {
        let vc = &self.version_control;
        let snapshot = vc.snapshot.clone();
        let loading = self.file_sync_root().is_some()
            && (!vc.detected || (vc.service.is_some() && snapshot.is_none()));
        let push = ui::command::list(cx)
            .iter()
            .any(|c| c.meta.id == "vcs.push");
        self.commit_ui.view.update(cx, |v, cx| {
            v.set_snapshot(snapshot.as_ref(), loading, cx);
            v.set_push_available(push, cx);
        });
        // The last commit and the hooks follow `HEAD`.
        let head = snapshot
            .as_ref()
            .and_then(|s| s.info.head.oid().map(str::to_owned));
        if head != self.commit_ui.head {
            self.commit_ui.head = head;
            self.load_last_commit(cx);
        }
    }

    /// The last commit and whether the repository has commit hooks, off
    /// the UI thread.
    fn load_last_commit(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git_repo() else {
            self.commit_ui
                .view
                .update(cx, |v, cx| v.set_last_commit(None, cx));
            return;
        };
        let work = cx.background_spawn(async move {
            let last = repo.last_commit(&Cancel::new()).ok().flatten();
            (last, repo.has_commit_hooks())
        });
        let view = self.commit_ui.view.downgrade();
        cx.spawn(async move |_, cx| {
            let (last, hooks) = work.await;
            let _ = view.update(cx, |v, cx| {
                v.set_hooks(hooks, cx);
                v.set_last_commit(last, cx);
            });
        })
        .detach();
    }

    /// A project was opened: its draft and Sign-off commit come back.
    pub(super) fn commit_project_opened(&mut self, root: &Path, cx: &mut Context<Self>) {
        let root = root.to_owned();
        let load = cx.background_spawn(async move {
            let draft = project::workspace::load_value(&root, TABLE, DRAFT_KEY)
                .ok()
                .flatten();
            let sign_off = project::workspace::load_value(&root, TABLE, SIGN_OFF_KEY)
                .ok()
                .flatten();
            (draft, sign_off.as_deref() == Some("true"))
        });
        let view = self.commit_ui.view.downgrade();
        cx.spawn(async move |_, cx| {
            let (draft, sign_off) = load.await;
            let _ = view.update(cx, |v, cx| {
                v.set_message(draft.as_deref().unwrap_or(""), cx);
                v.set_sign_off(sign_off, cx);
            });
        })
        .detach();
    }

    fn commit_event(&mut self, event: CommitEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            CommitEvent::Commit { request, push } => {
                self.commit_requested(request, push, window, cx)
            }
            CommitEvent::LoadLastCommit => self.load_last_commit(cx),
            CommitEvent::Rollback(paths) => self.open_rollback(paths, window, cx),
            CommitEvent::Add(paths) => self.add_to_vcs(paths, cx),
            CommitEvent::Open(path) => self.open_file(path, window, cx),
            CommitEvent::ShowDiff(path) => self.show_diff(path, window, cx),
            CommitEvent::Refresh => self.vcs_refresh(),
            CommitEvent::SelectOpenedFile => {
                if let Some(path) = self.active_file(cx) {
                    self.commit_ui
                        .view
                        .update(cx, |v, cx| v.select_path(&path, cx));
                }
            }
            CommitEvent::MessageEdited => self.save_draft_later(cx),
            CommitEvent::Hide => self.hide(COMMIT, window, cx),
        }
    }

    /// The draft is written a moment after the last edit.
    fn save_draft_later(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.file_sync_root().map(Path::to_path_buf) else {
            return;
        };
        self.commit_ui._draft_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DRAFT_DELAY).await;
            let Ok(text) =
                this.update(cx, |this, cx| this.commit_ui.view.read(cx).message_text(cx))
            else {
                return;
            };
            cx.background_spawn(async move {
                let value = (!text.is_empty()).then_some(text.as_str());
                let _ = project::workspace::save_value(&root, TABLE, DRAFT_KEY, value);
            })
            .await;
        }));
    }

    /// Writes the draft now (a commit cleared it) and puts messages first
    /// in the recent ones, the last one given first.
    fn store_messages(&mut self, messages: Vec<String>, cx: &mut Context<Self>) {
        let Some(root) = self.file_sync_root().map(Path::to_path_buf) else {
            return;
        };
        self.commit_ui._draft_save = None;
        let draft = self.commit_ui.view.read(cx).message_text(cx);
        cx.background_spawn(async move {
            let value = (!draft.is_empty()).then_some(draft.as_str());
            let _ = project::workspace::save_value(&root, TABLE, DRAFT_KEY, value);
            let messages: Vec<String> = messages
                .into_iter()
                .filter(|m| !m.trim().is_empty())
                .collect();
            if !messages.is_empty() {
                let mut recent =
                    project::workspace::load_list(&root, TABLE, RECENT_KEY).unwrap_or_default();
                for message in messages {
                    recent.retain(|m| *m != message);
                    recent.insert(0, message);
                }
                recent.truncate(RECENT_MESSAGES);
                let _ = project::workspace::save_list(&root, TABLE, RECENT_KEY, Some(&recent));
            }
        })
        .detach();
    }

    // ---- commands ---------------------------------------------------------------

    /// Commit… (⌘K): the tool window with every change included and the
    /// focus in the message; open documents are written first so that
    /// their changes are listed.
    fn vcs_commit(&mut self, _: &Commit, window: &mut Window, cx: &mut Context<Self>) {
        for task in self.save_all_tasks(cx) {
            task.detach();
        }
        self.commit_ui
            .view
            .update(cx, |v, cx| v.include_all_changes(cx));
        self.activate_tool_window(COMMIT, window, cx);
        let focus = self.commit_ui.view.read(cx).message_focus(cx);
        window.focus(&focus);
        cx.notify();
    }

    /// ⌘0: the Commit tool window shown and focused, or hidden when it has
    /// the focus.
    fn vcs_toggle_commit(
        &mut self,
        _: &ToggleCommitWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.commit_ui.view.read(cx).contains_focus(window, cx);
        if self.is_visible(COMMIT) && focused {
            self.hide(COMMIT, window, cx);
        } else {
            self.activate_tool_window(COMMIT, window, cx);
        }
    }

    /// A path of a command: relative to the repository's root, or absolute.
    fn repo_path(&self, path: &str) -> Option<PathBuf> {
        let p = Path::new(path);
        if p.is_absolute() {
            return Some(p.to_owned());
        }
        let root = self.version_control.snapshot.as_ref()?.info.root.clone();
        let mut out = root;
        for part in path.split('/').filter(|s| !s.is_empty()) {
            out.push(part);
        }
        Some(out)
    }

    /// The files a command without paths works on: the selection of the
    /// Commit tool window while it has the focus, else the active file.
    fn context_files(&self, window: &Window, cx: &App) -> Vec<PathBuf> {
        let view = self.commit_ui.view.read(cx);
        if view.contains_focus(window, cx) {
            return view.selected_files();
        }
        self.active_file(cx).into_iter().collect()
    }

    fn snapshot_changes(&self) -> Vec<Change> {
        self.version_control
            .snapshot
            .as_ref()
            .map(|s| s.status.changes())
            .unwrap_or_default()
    }

    /// Rollback… (⌘⌥Z): the Rollback Changes dialog for the selected
    /// changes, or with paths the rollback at once.
    fn vcs_rollback(&mut self, action: &Rollback, window: &mut Window, cx: &mut Context<Self>) {
        match &action.paths {
            Some(paths) => {
                let paths: Vec<PathBuf> = paths.iter().filter_map(|p| self.repo_path(p)).collect();
                let changes: Vec<Change> = self
                    .snapshot_changes()
                    .into_iter()
                    .filter(|c| paths.contains(&c.path))
                    .collect();
                self.run_rollback(changes, action.delete_added.unwrap_or(false), cx);
            }
            None => {
                let files = self.context_files(window, cx);
                self.open_rollback(files, window, cx);
            }
        }
    }

    /// Opens "Rollback Changes" with `files` ticked (every change when
    /// none of them is one).
    fn open_rollback(&mut self, files: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let changes = self.snapshot_changes();
        let Some(root) = self
            .version_control
            .snapshot
            .as_ref()
            .map(|s| s.info.root.clone())
        else {
            return;
        };
        if changes.is_empty() {
            return;
        }
        let mut included: Vec<PathBuf> = files
            .into_iter()
            .filter(|f| changes.iter().any(|c| &c.path == f))
            .collect();
        if included.is_empty() {
            included = changes.iter().map(|c| c.path.clone()).collect();
        }
        let file_types = self.file_types.clone();
        let icon: Rc<dyn Fn(&Path) -> IconName> = Rc::new(move |p: &Path| file_types.icon(p));
        let delete_added = self.commit_ui.delete_added;
        let dialog =
            cx.new(|cx| RollbackDialog::new(&root, changes, &included, delete_added, icon, cx));
        let return_focus = window.focused(cx);
        window.focus(&dialog.focus_handle(cx));
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            |this, dialog, event: &RollbackEvent, window, cx| {
                this.commit_ui.delete_added = dialog.read(cx).delete_added();
                this.close_commit_dialog(window, cx);
                if let RollbackEvent::Confirmed {
                    changes,
                    delete_added,
                } = event
                {
                    this.run_rollback(changes.clone(), *delete_added, cx);
                }
            },
        );
        self.commit_ui.dialog = Some(OpenDialog::Rollback {
            dialog,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Rolls back after writing the open documents; the open files reload.
    fn run_rollback(&mut self, changes: Vec<Change>, delete_added: bool, cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        let Some(repo) = self.git_repo() else {
            return;
        };
        let saves = self.save_all_tasks(cx);
        let mut paths: Vec<PathBuf> = changes.iter().map(|c| c.path.clone()).collect();
        paths.extend(changes.iter().filter_map(|c| c.old_path.clone()));
        cx.spawn(async move |this, cx| {
            futures::future::join_all(saves).await;
            let result = cx
                .background_spawn(
                    async move { repo.rollback(&changes, delete_added, &Cancel::new()) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.check_open_files(Some(paths), cx);
                if let Err(e) = result {
                    this.notify(
                        Notification::new(NotificationKind::Error, "Rollback failed", e.message),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Add to VCS (⌘⌥A).
    fn vcs_add(&mut self, action: &AddToVcs, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = match &action.paths {
            Some(paths) => paths.iter().filter_map(|p| self.repo_path(p)).collect(),
            None => self.context_files(window, cx),
        };
        let unversioned: Vec<PathBuf> = match &self.version_control.snapshot {
            Some(s) => paths
                .into_iter()
                .filter(|p| s.status.status(p) == FileStatus::Unversioned)
                .collect(),
            None => Vec::new(),
        };
        self.add_to_vcs(unversioned, cx);
    }

    fn add_to_vcs(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let Some(repo) = self.git_repo() else {
            return;
        };
        let work = cx.background_spawn(async move { repo.add(&paths, &Cancel::new()) });
        cx.spawn(async move |this, cx| {
            if let Err(e) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.notify(
                        Notification::new(
                            NotificationKind::Error,
                            "Adding files to Git failed",
                            e.message,
                        ),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    /// The diff of a file when a diff viewer is there (`vcs.show-diff`),
    /// else the file.
    fn show_diff(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let has_diff = ui::command::list(cx)
            .iter()
            .any(|c| c.meta.id == "vcs.show-diff");
        let rel = self
            .version_control
            .snapshot
            .as_ref()
            .map(|s| git::commands::relative_path(&s.info.root, &path));
        if has_diff
            && let Some(rel) = rel
            && let Ok(action) = cx.build_action("vcs.show-diff", Some(json!({ "path": rel })))
        {
            window.dispatch_action(action, cx);
            return;
        }
        self.open_file(path, window, cx);
    }

    // ---- commit -----------------------------------------------------------------

    /// The view asks for a commit: the user's name is checked first (unless
    /// an author is given), then a detached `HEAD` is warned about.
    fn commit_requested(
        &mut self,
        request: CommitRequest,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git_repo() else {
            self.commit_ui
                .view
                .update(cx, |v, cx| v.commit_failed(None, cx));
            return;
        };
        if request.author.is_some() {
            self.check_detached(request, push, window, cx);
            return;
        }
        let work = cx.background_spawn(async move { repo.identity(&Cancel::new()) });
        cx.spawn_in(window, async move |this, cx| {
            let identity = work.await;
            let _ = this.update_in(cx, |this, window, cx| match identity {
                Ok(identity) if !identity.is_complete() => {
                    this.open_user_name(identity, request, push, window, cx)
                }
                _ => this.check_detached(request, push, window, cx),
            });
        })
        .detach();
    }

    fn open_user_name(
        &mut self,
        identity: git::Identity,
        request: CommitRequest,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = identity.name.unwrap_or_else(system_user_name);
        let email = identity.email.unwrap_or_default();
        let action = action_name(request.amend);
        let dialog = cx.new(|cx| UserNameDialog::new(&name, &email, false, action, window, cx));
        let return_focus = window.focused(cx);
        window.focus(&dialog.focus_handle(cx));
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            |this, _, event: &UserNameEvent, window, cx| {
                let Some(OpenDialog::UserName { request, push, .. }) =
                    this.close_commit_dialog(window, cx)
                else {
                    return;
                };
                match event.clone() {
                    UserNameEvent::Confirmed {
                        name,
                        email,
                        global,
                    } => {
                        this.set_identity_and_commit(name, email, global, request, push, window, cx)
                    }
                    UserNameEvent::Cancelled => this
                        .commit_ui
                        .view
                        .update(cx, |v, cx| v.commit_failed(None, cx)),
                }
            },
        );
        self.commit_ui.dialog = Some(OpenDialog::UserName {
            dialog,
            request,
            push,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)] // the dialog's answer and the commit waiting for it
    fn set_identity_and_commit(
        &mut self,
        name: String,
        email: String,
        global: bool,
        request: CommitRequest,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git_repo() else {
            return;
        };
        let work =
            cx.background_spawn(
                async move { repo.set_identity(&name, &email, global, &Cancel::new()) },
            );
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.check_detached(request, push, window, cx),
                Err(e) => {
                    this.notify(
                        Notification::new(
                            NotificationKind::Error,
                            "Could not set user.name and user.email",
                            e.message,
                        ),
                        cx,
                    );
                    this.commit_ui
                        .view
                        .update(cx, |v, cx| v.commit_failed(None, cx));
                }
            });
        })
        .detach();
    }

    /// "Commit in Detached HEAD" before a commit on a detached `HEAD`.
    fn check_detached(
        &mut self,
        request: CommitRequest,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detached = self
            .version_control
            .snapshot
            .as_ref()
            .filter(|s| matches!(s.info.head, Head::Detached { .. }))
            .map(|s| s.info.root.clone());
        let Some(root) = detached else {
            self.run_commit(request, push, window, cx);
            return;
        };
        let action = action_name(request.amend);
        let dialog = cx.new(|cx| {
            Dialog::new(
                "Commit in Detached HEAD",
                detached_message(&root),
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "commit",
                        label: action.into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::WARNING_DIALOG)
        });
        let return_focus = window.focused(cx);
        window.focus(&dialog.focus_handle(cx));
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, event: &DialogEvent, window, cx| {
                let DialogEvent::Closed { button, .. } = event;
                let Some(OpenDialog::Detached { request, push, .. }) =
                    this.close_commit_dialog(window, cx)
                else {
                    return;
                };
                if *button == Some("commit") {
                    this.run_commit(request, push, window, cx);
                } else {
                    let text = format!(
                        "The repository is in the detached HEAD state: {}",
                        root.display()
                    );
                    this.commit_ui
                        .view
                        .update(cx, |v, cx| v.commit_failed(Some(text), cx));
                }
            },
        );
        self.commit_ui.dialog = Some(OpenDialog::Detached {
            dialog,
            request,
            push,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Writes the open documents, commits, then reports: "{n} files
    /// committed" with the message, or "Commit failed with error" with
    /// git's output; Commit and Push… pushes after a commit.
    fn run_commit(
        &mut self,
        request: CommitRequest,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git_repo() else {
            self.commit_ui
                .view
                .update(cx, |v, cx| v.commit_failed(None, cx));
            return;
        };
        let saves = self.save_all_tasks(cx);
        let before_amend = self
            .commit_ui
            .view
            .read(cx)
            .message_before_amend()
            .map(str::to_owned);
        let message = request.message.clone();
        let sign_off = request.sign_off;
        let root = self.file_sync_root().map(Path::to_path_buf);
        cx.spawn_in(window, async move |this, cx| {
            futures::future::join_all(saves).await;
            let result = cx
                .background_spawn(async move { repo.commit(&request, &Cancel::new()) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(done) => {
                    this.commit_ui.view.update(cx, |v, cx| v.committed(cx));
                    this.notify(
                        Notification::new(
                            NotificationKind::Information,
                            committed_title(done.files),
                            message.clone(),
                        ),
                        cx,
                    );
                    this.store_messages(before_amend.into_iter().chain([message]).collect(), cx);
                    if let Some(root) = root {
                        let value = sign_off.then_some("true");
                        cx.background_spawn(async move {
                            let _ =
                                project::workspace::save_value(&root, TABLE, SIGN_OFF_KEY, value);
                        })
                        .detach();
                    }
                    this.vcs_refresh();
                    if push && let Ok(action) = cx.build_action("vcs.push", None) {
                        window.dispatch_action(action, cx);
                    }
                }
                Err(e) => {
                    this.commit_ui
                        .view
                        .update(cx, |v, cx| v.commit_failed(None, cx));
                    this.notify(
                        Notification::new(
                            NotificationKind::Error,
                            "Commit failed with error",
                            e.message,
                        ),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    // ---- dialogs ----------------------------------------------------------------

    /// Closes the open dialog, the focus back where it was.
    fn close_commit_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<OpenDialog> {
        let open = self.commit_ui.dialog.take()?;
        let focus = match &open {
            OpenDialog::Rollback { return_focus, .. }
            | OpenDialog::UserName { return_focus, .. }
            | OpenDialog::Detached { return_focus, .. } => return_focus.clone(),
        };
        if let Some(focus) = focus {
            window.focus(&focus);
        }
        cx.notify();
        Some(open)
    }

    /// The open dialog of the Commit tool window, over the window.
    pub(super) fn render_commit_overlays(&self) -> Option<AnyElement> {
        self.commit_ui.dialog.as_ref().map(|d| match d {
            OpenDialog::Rollback { dialog, .. } => dialog.clone().into_any_element(),
            OpenDialog::UserName { dialog, .. } => dialog.clone().into_any_element(),
            OpenDialog::Detached { dialog, .. } => dialog.clone().into_any_element(),
        })
    }

    /// The Commit tool window for `vcs.state`.
    pub(super) fn vcs_commit_json(&self, cx: &App) -> Json {
        let mut state = self.commit_ui.view.read(cx).state_json(None, cx);
        state["visible"] = json!(self.is_visible(COMMIT));
        state["dialog"] = match &self.commit_ui.dialog {
            Some(OpenDialog::Rollback { dialog, .. }) => dialog.read(cx).state_json(),
            Some(OpenDialog::UserName { dialog, .. }) => dialog.read(cx).state_json(cx),
            Some(OpenDialog::Detached { dialog, .. }) => {
                let mut d = dialog.read(cx).state_json();
                d["kind"] = json!("detached");
                d
            }
            None => Json::Null,
        };
        state
    }

    /// The focus of the open commit dialog (its keys work there).
    pub fn commit_dialog_focus(&self, cx: &App) -> Option<FocusHandle> {
        Some(match self.commit_ui.dialog.as_ref()? {
            OpenDialog::Rollback { dialog, .. } => dialog.focus_handle(cx),
            OpenDialog::UserName { dialog, .. } => dialog.focus_handle(cx),
            OpenDialog::Detached { dialog, .. } => dialog.focus_handle(cx),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balloon_and_dialog_texts() {
        assert_eq!(committed_title(1), "1 file committed");
        assert_eq!(committed_title(3), "3 files committed");
        assert!(detached_message(Path::new("/r")).starts_with(
            "The Git repository at the following path is in the detached HEAD state:\n/r\n"
        ));
    }
}
