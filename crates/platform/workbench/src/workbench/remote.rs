//! Push, Update Project, Fetch and Pull (#172): the Push dialog and the
//! push's balloons ("Pushed 1 commit to origin/main", "Push rejected",
//! "Push failed"), "Push Rejected" (Merge, Rebase, Cancel) for a push of
//! the current branch to its tracked branch that needs the remote's
//! commits, the "Force Push" confirmation, the Update Project dialog and
//! its balloons ("1 file updated in 1 commit", "All files are up to date",
//! "Cannot update"), Fetch and the branch actions Update and Pull, and the
//! VCS Operations popup.
//!
//! Every git call runs off the UI thread through the project's
//! [`git::GitRepo`]; git never asks for credentials (they come from
//! credential helpers or SSH agents only), so an authentication failure
//! is a "Push failed" or "Fetch failed" balloon. What the dialogs remember
//! (the update method, Don't show again, the Push Rejected choice, targets
//! force pushed without a warning) lasts while the IDE runs.

use std::collections::HashSet;
use std::sync::Arc;

use git::{FetchResult, GitRepo, RemoteFetch, UpdateResult};
use gpui::{
    AnyElement, App, AppContext as _, Context, DismissEvent, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Subscription, Window,
    anchored, div, point, px,
};
use serde_json::{Value as Json, json};
use ui::deferred;
use ui::icons::IconName;
use ui::menu::{ContextMenu, PopupOptions};
use ui::notification::{Notification, NotificationKind};
use vcs::{
    Cancel, CommitFile, ForceMode, OutgoingCommit, PushOutcome, PushResult, PushSpec, PushTarget,
    TagMode, UpdateMethod, VcsError,
};
use vcs_ui::push::{PushData, PushDialog, PushDialogEvent, PushSetup};
use vcs_ui::update::{UpdateDialog, UpdateDialogEvent};
use vcs_ui::{Fetch, Pull, ShowOperations, UpdateBranch, UpdateProject, VcsPush};

use super::Workbench;
use crate::dialog::{Dialog, DialogButton, DialogEvent};

/// The open dialogs and popup of the remote commands and what they
/// remember.
#[derive(Default)]
pub(super) struct RemoteUi {
    push: Option<OpenPush>,
    update: Option<OpenUpdate>,
    message: Option<OpenMessage>,
    operations: Option<(Entity<ContextMenu>, Subscription)>,
    /// The update method last chosen (`GitVcsSettings.updateMethod`).
    method: UpdateMethod,
    /// Update Project runs without its dialog (Don't show again).
    update_silently: bool,
    /// A rejected push updates with `method` and pushes again without
    /// asking (`autoUpdateIfPushRejected`).
    update_rejected_silently: bool,
    /// Targets (remote, branch) force pushed without the warning.
    silent_force: HashSet<(String, String)>,
}

struct OpenPush {
    dialog: Entity<PushDialog>,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

struct OpenUpdate {
    dialog: Entity<UpdateDialog>,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// What a message dialog asks.
enum Asking {
    /// "Push Rejected": update with Merge or Rebase and push again.
    Rejected(PushSpec),
    /// "Force Push": push the Push dialog's commits with force.
    ForcePush(PushSpec),
}

struct OpenMessage {
    dialog: Entity<Dialog>,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// The Push dialog's reads.
struct GitPushData(Arc<GitRepo>);

impl PushData for GitPushData {
    fn target(&self, remote: &str, branch: &str) -> Result<PushTarget, VcsError> {
        self.0.push_target(remote, branch, &Cancel::new())
    }

    fn outgoing(&self, source: &str, target: &PushTarget) -> Result<Vec<OutgoingCommit>, VcsError> {
        self.0.outgoing(source, target, &Cancel::new())
    }

    fn files(&self, commits: &[String]) -> Result<Vec<CommitFile>, VcsError> {
        self.0.commit_files(commits, &Cancel::new())
    }
}

/// How the update of a rejected push ended.
enum UpdateOfPush {
    /// The incoming commits are in the branch (`commits` of them).
    Updated(u32),
    /// The update stopped on conflicts.
    Incomplete,
    /// The update failed or could not start.
    Failed,
}

/// `1 commit` / `2 commits`.
fn commits_text(n: u32) -> String {
    git::remote::commits_text(n)
}

/// The balloon of a finished push (`GitPushResultNotification`): its
/// kind, title and text and the action it offers.
fn push_notification(result: &PushResult, update: Option<&UpdateOfPush>) -> Notification {
    let description = result.description();
    let rejected = |text: String| {
        let mut n = Notification::new(NotificationKind::Warning, "Push rejected", text);
        n.sticky = true;
        n
    };
    match &result.outcome {
        o if o.is_success() => {
            let received = match update {
                Some(UpdateOfPush::Updated(n)) if *n > 0 => {
                    format!("{} received during the push operation", commits_text(*n))
                }
                _ => String::new(),
            };
            Notification::new(NotificationKind::Information, description, received)
        }
        PushOutcome::RejectedNoFastForward => rejected(match update {
            None | Some(UpdateOfPush::Updated(_)) => description,
            Some(UpdateOfPush::Incomplete) => {
                "Push was canceled, because not all conflicts were resolved during the update.\n\
                 Resolve the conflicts and try pushing again"
                    .to_owned()
            }
            Some(UpdateOfPush::Failed) => {
                "Push was rejected, and update failed with an error".to_owned()
            }
        }),
        PushOutcome::RejectedStaleInfo => rejected(description).with_action(
            "Force push anyway",
            "vcs.push",
            json!({
                "branch": result.source,
                "target": result.target.short_name(),
                "force": ForceMode::Force.id(),
                "dialog": false,
            }),
        ),
        PushOutcome::RejectedOther { .. } => rejected(description),
        _ => Notification::new(NotificationKind::Error, "Push failed", description),
    }
}

/// The balloons of a finished update (`GitUpdateSession`,
/// `GitConflictResolver`, `GitUpdateProcess`).
fn update_notifications(result: &UpdateResult) -> Vec<Notification> {
    let warning = |title: &str, text: &str| {
        let mut n = Notification::new(NotificationKind::Warning, title.to_owned(), text.to_owned());
        n.sticky = true;
        n
    };
    match result {
        UpdateResult::UpToDate => vec![Notification::new(
            NotificationKind::Information,
            "",
            "All files are up to date",
        )],
        UpdateResult::Updated {
            commits,
            files,
            restore_conflicts,
        } => {
            let mut out = vec![Notification::new(
                NotificationKind::Information,
                format!(
                    "{} updated in {}",
                    git::remote::files_text(*files),
                    commits_text(*commits)
                ),
                "",
            )];
            if *restore_conflicts {
                out.push(warning(
                    "Local changes were restored with conflicts",
                    "Your uncommitted changes were saved to stash.\n\
                     Unstash is not complete, you have unresolved merges in your working tree\n\
                     Resolve conflicts and drop the stash.",
                ));
            }
            out
        }
        UpdateResult::Incomplete { method, stashed } => {
            let mut out = vec![match method {
                UpdateMethod::Merge => warning(
                    "Cannot complete update",
                    "There are pending unresolved conflicts.",
                ),
                UpdateMethod::Rebase => warning(
                    "Cannot continue rebase",
                    "There are pending unresolved conflicts.\n\
                     Then you may continue rebase.\n\
                     You also may abort rebase to restore the original branch and stop rebasing.",
                ),
            }];
            if *stashed {
                out.push(warning(
                    "Local changes were not restored",
                    "Before update your uncommitted changes were saved to stash.",
                ));
            }
            out
        }
        UpdateResult::NotReady { title, message } | UpdateResult::Failed { title, message } => {
            vec![Notification::new(
                NotificationKind::Error,
                title.clone(),
                message.clone(),
            )]
        }
    }
}

/// The balloon of a fetch (`GitFetchSupportImpl`).
fn fetch_notification(result: &FetchResult) -> Notification {
    if result.is_success() {
        Notification::new(
            NotificationKind::Information,
            "Fetch successful",
            result.message(),
        )
    } else {
        Notification::new(NotificationKind::Error, "Fetch failed", result.message())
    }
}

/// The message of "Push Rejected" for one repository
/// (`push.rejected.only.one.git.repo`, `push.rejected.merge.needed`).
fn rejected_message(branch: &str) -> String {
    format!(
        "Push of the current branch \"{branch}\" was rejected. \
         Remote changes need to be merged before pushing."
    )
}

/// The message of "Force Push" (`action.force.push.to.confirmation.text`).
fn force_message(target: &PushTarget) -> String {
    format!(
        "You're going to force push to '{}'. It may overwrite commits at the remote. \
         Are you sure you want to proceed?",
        target.branch
    )
}

impl Workbench {
    /// The remote commands on the window's root.
    pub(super) fn remote_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        root.on_action(cx.listener(Self::vcs_push))
            .on_action(cx.listener(Self::vcs_update_project))
            .on_action(cx.listener(Self::git_fetch))
            .on_action(cx.listener(Self::git_update_branch))
            .on_action(cx.listener(Self::git_pull))
            .on_action(cx.listener(Self::vcs_show_operations))
    }

    /// The open dialogs and popup for `vcs.state`.
    pub(super) fn vcs_remote_json(&self, cx: &App) -> Json {
        let ui = &self.remote_ui;
        json!({
            "push_dialog": ui.push.as_ref().map(|o| o.dialog.read(cx).state_json(cx)),
            "update_dialog": ui.update.as_ref().map(|o| o.dialog.read(cx).state_json()),
            "remote_dialog": ui.message.as_ref().map(|o| o.dialog.read(cx).state_json()),
            "operations_popup": ui.operations.as_ref().map(|(m, _)| m.read(cx).state_json()),
            "update_method": ui.method.id(),
        })
    }

    fn notify_all(&mut self, notifications: Vec<Notification>, cx: &mut Context<Self>) {
        for n in notifications {
            self.notify(n, cx);
        }
    }

    // ---- push -----------------------------------------------------------------

    fn vcs_push(&mut self, action: &VcsPush, window: &mut Window, cx: &mut Context<Self>) {
        self.close_branch_popups(window, cx);
        let Some(source) = action.branch.clone().or_else(|| self.current_branch()) else {
            self.error_balloon(
                "Cannot push".to_owned(),
                "HEAD is not on a branch".to_owned(),
                cx,
            );
            return;
        };
        let force = match action.force.as_deref() {
            None => ForceMode::None,
            Some(f) => match ForceMode::from_id(f) {
                Some(f) => f,
                None => {
                    self.show_notice(format!("Unknown force mode {f}").into(), cx);
                    return;
                }
            },
        };
        let tags = match action.tags.as_deref() {
            None => None,
            Some(t) => match TagMode::from_id(t) {
                Some(t) => Some(t),
                None => {
                    self.show_notice(format!("Unknown tag mode {t}").into(), cx);
                    return;
                }
            },
        };
        let target = action.target.clone();
        let set_upstream = action.set_upstream;
        if action.dialog == Some(false) {
            self.with_repo(
                window,
                cx,
                {
                    let source = source.clone();
                    move |repo| {
                        let c = Cancel::new();
                        let target = match &target {
                            Some(t) => {
                                let (remote, branch) = git::commands::split_remote_branch(repo, t)
                                    .ok_or_else(|| format!("No remote in {t}"))?;
                                repo.push_target(&remote, &branch, &c)
                                    .map_err(|e| e.message)?
                            }
                            None => repo
                                .default_push_target(&source, &c)
                                .map_err(|e| e.message)?
                                .ok_or_else(|| "The repository has no remotes".to_owned())?,
                        };
                        let upstream = repo.upstream_of(&source);
                        Ok::<_, String>(PushSpec {
                            set_upstream: set_upstream.unwrap_or(target.new && upstream.is_none()),
                            source,
                            target,
                            force,
                            tags,
                        })
                    }
                },
                move |this, spec, window, cx| match spec {
                    Ok(spec) => this.run_push(spec, window, cx),
                    Err(e) => this.error_balloon("Push failed".to_owned(), e, cx),
                },
            );
            return;
        }
        self.with_repo(
            window,
            cx,
            {
                let source = source.clone();
                move |repo| {
                    let c = Cancel::new();
                    let target = match &target {
                        Some(t) => git::commands::split_remote_branch(repo, t)
                            .and_then(|(r, b)| repo.push_target(&r, &b, &c).ok()),
                        None => repo.default_push_target(&source, &c).ok().flatten(),
                    };
                    (repo.remote_names(), target, repo.upstream_of(&source))
                }
            },
            move |this, (remotes, target, upstream), window, cx| {
                let Some(repo) = this.git_repo() else {
                    return;
                };
                let setup = PushSetup {
                    repository: repo
                        .repository()
                        .root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    source,
                    remotes,
                    target,
                    upstream,
                    data: Arc::new(GitPushData(repo)),
                };
                this.open_push_dialog(setup, window, cx);
            },
        );
    }

    fn open_push_dialog(&mut self, setup: PushSetup, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| PushDialog::new(setup, cx));
        let return_focus = self
            .remote_ui
            .push
            .take()
            .and_then(|o| o.return_focus)
            .or_else(|| window.focused(cx));
        window.focus(&dialog.focus_handle(cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            this.push_dialog_event(event, window, cx)
        });
        self.remote_ui.push = Some(OpenPush {
            dialog,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_push_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.remote_ui.push.take()
            && let Some(focus) = open.return_focus
        {
            window.focus(&focus);
        }
        cx.notify();
    }

    fn push_dialog_event(
        &mut self,
        event: &PushDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PushDialogEvent::Cancelled => self.close_push_dialog(window, cx),
            PushDialogEvent::Push(spec) => {
                let key = (spec.target.remote.clone(), spec.target.branch.clone());
                if spec.force != ForceMode::None && !self.remote_ui.silent_force.contains(&key) {
                    self.ask_force_push(spec.clone(), window, cx);
                    return;
                }
                self.close_push_dialog(window, cx);
                self.run_push(spec.clone(), window, cx);
            }
        }
    }

    /// Pushes; a push of the current branch to its tracked branch that
    /// needs the remote's commits first asks to update ("Push Rejected").
    fn run_push(&mut self, spec: PushSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.with_repo(
            window,
            cx,
            {
                let spec = spec.clone();
                move |repo| {
                    let c = Cancel::new();
                    let result = repo.push(&spec, &c);
                    // `pushingToNotTrackedBranch`, `pushingNotCurrentBranch`.
                    let can_update = matches!(
                        &result,
                        Ok(r) if r.outcome == PushOutcome::RejectedNoFastForward
                    ) && spec.force == ForceMode::None
                        && vcs::Backend::info(repo)
                            .is_ok_and(|i| i.head.branch() == Some(spec.source.as_str()))
                        && repo.upstream_of(&spec.source)
                            == Some((spec.target.remote.clone(), spec.target.branch.clone()));
                    (result, can_update)
                }
            },
            move |this, (result, can_update), window, cx| match result {
                Ok(_) if can_update && this.remote_ui.update_rejected_silently => {
                    let method = this.remote_ui.method;
                    this.update_and_push(spec, method, window, cx);
                }
                Ok(_) if can_update => this.ask_rejected(spec, window, cx),
                Ok(result) => {
                    let n = push_notification(&result, None);
                    this.notify(n, cx);
                }
                Err(e) => this.error_balloon("Push failed".to_owned(), e.message, cx),
            },
        );
    }

    /// Updates the current branch with `method`, then pushes again.
    fn update_and_push(
        &mut self,
        spec: PushSpec,
        method: UpdateMethod,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_repo(
            window,
            cx,
            move |repo| {
                let c = Cancel::new();
                let update = repo.update(method, None, &c)?;
                let push = if update.is_success() {
                    Some(repo.push(&spec, &c)?)
                } else {
                    None
                };
                // The push's result as it was rejected, for the balloon.
                let rejected = PushResult {
                    source: spec.source.clone(),
                    target: spec.target.clone(),
                    outcome: PushOutcome::RejectedNoFastForward,
                    tags: Vec::new(),
                };
                Ok::<_, VcsError>((update, push, rejected))
            },
            |this, result, _, cx| {
                let (update, push, rejected) = match result {
                    Ok(r) => r,
                    Err(e) => {
                        this.error_balloon("Push failed".to_owned(), e.message, cx);
                        return;
                    }
                };
                let of_push = match &update {
                    UpdateResult::UpToDate => UpdateOfPush::Updated(0),
                    UpdateResult::Updated { commits, .. } => UpdateOfPush::Updated(*commits),
                    UpdateResult::Incomplete { .. } => UpdateOfPush::Incomplete,
                    _ => UpdateOfPush::Failed,
                };
                // The update's own balloons, except the counts the push's
                // balloon gives.
                if !matches!(
                    update,
                    UpdateResult::UpToDate | UpdateResult::Updated { .. }
                ) {
                    this.notify_all(update_notifications(&update), cx);
                } else if let UpdateResult::Updated {
                    restore_conflicts: true,
                    ..
                } = update
                {
                    let mut all = update_notifications(&update);
                    all.remove(0);
                    this.notify_all(all, cx);
                }
                let n = push_notification(push.as_ref().unwrap_or(&rejected), Some(&of_push));
                this.notify(n, cx);
            },
        );
    }

    /// "Push Rejected" (`GitRejectedPushUpdateDialog`): Merge, Rebase or
    /// Cancel, the last update method the default.
    fn ask_rejected(&mut self, spec: PushSpec, window: &mut Window, cx: &mut Context<Self>) {
        let default = self.remote_ui.method.id();
        let message = rejected_message(&spec.source);
        let dialog = cx.new(|cx| {
            Dialog::new(
                "Push Rejected",
                message,
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "rebase",
                        label: "&Rebase".into(),
                    },
                    DialogButton {
                        id: "merge",
                        label: "&Merge".into(),
                    },
                ],
                cx,
            )
            .with_default(default)
            .with_icon(IconName::WARNING_DIALOG)
            .with_option(
                "Remember the update method and update silently in the future. \
                 Change this later in Settings | Version Control.",
            )
        });
        self.open_message(dialog, Asking::Rejected(spec), window, cx);
    }

    /// "Force Push" (`ForcePushAction.confirmForcePush`).
    fn ask_force_push(&mut self, spec: PushSpec, window: &mut Window, cx: &mut Context<Self>) {
        let message = force_message(&spec.target);
        let dialog = cx.new(|cx| {
            Dialog::new(
                "Force Push",
                message,
                vec![
                    DialogButton {
                        id: "cancel",
                        label: "Cancel".into(),
                    },
                    DialogButton {
                        id: "force",
                        label: "&Force Push".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::WARNING_DIALOG)
            .with_option("Don't warn about this target")
        });
        self.open_message(dialog, Asking::ForcePush(spec), window, cx);
    }

    fn open_message(
        &mut self,
        dialog: Entity<Dialog>,
        asking: Asking,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = window.focused(cx);
        window.focus(&dialog.focus_handle(cx));
        let mut asking = Some(asking);
        let subscription = cx.subscribe_in(&dialog, window, move |this, _, event, window, cx| {
            let DialogEvent::Closed { button, checked } = event;
            let (button, checked) = (*button, *checked);
            if let Some(open) = this.remote_ui.message.take()
                && let Some(focus) = open.return_focus
            {
                window.focus(&focus);
            }
            cx.notify();
            let Some(asking) = asking.take() else {
                return;
            };
            this.message_closed(asking, button, checked, window, cx);
        });
        self.remote_ui.message = Some(OpenMessage {
            dialog,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn message_closed(
        &mut self,
        asking: Asking,
        button: Option<&'static str>,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match asking {
            Asking::Rejected(spec) => {
                let method = match button {
                    Some("merge") => UpdateMethod::Merge,
                    Some("rebase") => UpdateMethod::Rebase,
                    _ => {
                        let rejected = PushResult {
                            source: spec.source.clone(),
                            target: spec.target.clone(),
                            outcome: PushOutcome::RejectedNoFastForward,
                            tags: Vec::new(),
                        };
                        let n = push_notification(&rejected, None);
                        self.notify(n, cx);
                        return;
                    }
                };
                self.remote_ui.method = method;
                if checked {
                    self.remote_ui.update_rejected_silently = true;
                }
                self.update_and_push(spec, method, window, cx);
            }
            Asking::ForcePush(spec) => {
                if button != Some("force") {
                    // Back to the Push dialog.
                    if let Some(open) = &self.remote_ui.push {
                        window.focus(&open.dialog.focus_handle(cx));
                    }
                    return;
                }
                if checked {
                    self.remote_ui
                        .silent_force
                        .insert((spec.target.remote.clone(), spec.target.branch.clone()));
                }
                self.close_push_dialog(window, cx);
                self.run_push(spec, window, cx);
            }
        }
    }

    // ---- update -----------------------------------------------------------------

    fn vcs_update_project(
        &mut self,
        action: &UpdateProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branch_popups(window, cx);
        if let Some(id) = &action.method {
            match UpdateMethod::from_id(id) {
                Some(method) => self.run_update(method, None, window, cx),
                None => self.show_notice(format!("Unknown update method {id}").into(), cx),
            }
            return;
        }
        if self.remote_ui.update_silently && action.dialog != Some(true) {
            let method = self.remote_ui.method;
            self.run_update(method, None, window, cx);
            return;
        }
        if self.git_repo().is_none() {
            self.show_notice("There is no Git repository".into(), cx);
            return;
        }
        let method = self.remote_ui.method;
        let dialog = cx.new(|cx| UpdateDialog::new(method, cx));
        let return_focus = window.focused(cx);
        window.focus(&dialog.focus_handle(cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            if let Some(open) = this.remote_ui.update.take()
                && let Some(focus) = open.return_focus
            {
                window.focus(&focus);
            }
            cx.notify();
            if let UpdateDialogEvent::Confirmed { method, dont_show } = event {
                this.remote_ui.method = *method;
                this.remote_ui.update_silently = *dont_show;
                this.run_update(*method, None, window, cx);
            }
        });
        self.remote_ui.update = Some(OpenUpdate {
            dialog,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Update Project with `method`, from `from` (a remote and its branch)
    /// or the tracked branch.
    fn run_update(
        &mut self,
        method: UpdateMethod,
        from: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_repo(
            window,
            cx,
            move |repo| {
                repo.update(
                    method,
                    from.as_ref().map(|(r, b)| (r.as_str(), b.as_str())),
                    &Cancel::new(),
                )
            },
            |this, result, _, cx| match result {
                Ok(result) => this.notify_all(update_notifications(&result), cx),
                Err(e) => this.error_balloon("Update failed".to_owned(), e.message, cx),
            },
        );
    }

    fn git_pull(&mut self, action: &Pull, window: &mut Window, cx: &mut Context<Self>) {
        self.close_branch_popups(window, cx);
        let Some(method) = UpdateMethod::from_id(&action.method) else {
            self.show_notice(
                format!("Unknown update method {}", action.method).into(),
                cx,
            );
            return;
        };
        let Some(repo) = self.git_repo() else {
            self.show_notice("There is no Git repository".into(), cx);
            return;
        };
        match git::commands::split_remote_branch(&repo, &action.from) {
            Some(from) => self.run_update(method, Some(from), window, cx),
            None => self.error_balloon(
                "Cannot update".to_owned(),
                format!("{} is not a remote branch", action.from),
                cx,
            ),
        }
    }

    /// A branch's Update: the current branch as Update Project with the
    /// last method, another one by a fast-forward fetch.
    fn git_update_branch(
        &mut self,
        action: &UpdateBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branch_popups(window, cx);
        if self.current_branch().as_deref() == Some(action.name.as_str()) {
            let method = self.remote_ui.method;
            self.run_update(method, None, window, cx);
            return;
        }
        let name = action.name.clone();
        self.with_repo(
            window,
            cx,
            move |repo| repo.update_branch(&name, &Cancel::new()),
            |this, result: Result<RemoteFetch, VcsError>, _, cx| match result {
                Ok(RemoteFetch { error: None, .. }) => {
                    this.info_balloon("Fetched 1 targets".to_owned(), cx)
                }
                Ok(RemoteFetch { error: Some(e), .. }) => {
                    this.error_balloon("Update failed".to_owned(), e, cx)
                }
                Err(e) => this.error_balloon("Update failed".to_owned(), e.message, cx),
            },
        );
    }

    // ---- fetch ------------------------------------------------------------------

    fn git_fetch(&mut self, action: &Fetch, window: &mut Window, cx: &mut Context<Self>) {
        self.close_branch_popups(window, cx);
        let remote = action.remote.clone();
        self.with_repo(
            window,
            cx,
            move |repo| {
                let c = Cancel::new();
                match &remote {
                    Some(r) => repo
                        .fetch(r, None, &c)
                        .map(|r| FetchResult { remotes: vec![r] }),
                    None => repo.fetch_all(&c),
                }
            },
            |this, result, _, cx| match result {
                Ok(result) => {
                    let n = fetch_notification(&result);
                    this.notify(n, cx);
                }
                Err(e) => this.error_balloon("Fetch failed".to_owned(), e.message, cx),
            },
        );
    }

    // ---- VCS Operations -----------------------------------------------------------

    fn vcs_show_operations(
        &mut self,
        _: &ShowOperations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((menu, _)) = self.remote_ui.operations.take() {
            menu.update(cx, |m, cx| m.dismiss(window, cx));
            cx.notify();
            return;
        }
        let git = self.version_control.service.is_some();
        let items = vcs_ui::operations::items(git, cx);
        let options = PopupOptions {
            title: Some(vcs_ui::operations::TITLE.into()),
            speed_search: true,
            ..PopupOptions::default()
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this
                .remote_ui
                .operations
                .as_ref()
                .is_some_and(|(m, _)| *m == menu)
            {
                this.remote_ui.operations = None;
                cx.notify();
            }
        });
        self.remote_ui.operations = Some((menu, subscription));
        cx.notify();
    }

    // ---- drawing --------------------------------------------------------------------

    /// The dialogs and the popup, over the window.
    pub(super) fn render_remote_overlays(&self, window: &Window) -> Vec<AnyElement> {
        let ui = &self.remote_ui;
        let mut out = Vec::new();
        if let Some(open) = &ui.push {
            out.push(open.dialog.clone().into_any_element());
        }
        if let Some(open) = &ui.update {
            out.push(open.dialog.clone().into_any_element());
        }
        if let Some(open) = &ui.message {
            out.push(open.dialog.clone().into_any_element());
        }
        if let Some((menu, _)) = &ui.operations {
            // In the middle of the window (`showCenteredInCurrentWindow`),
            // a click outside closes it.
            let size = window.viewport_size();
            let backdrop = menu.clone();
            out.push(
                deferred(
                    anchored().position(point(px(0.0), px(0.0))).child(
                        div()
                            .id("operations-backdrop")
                            .occlude()
                            .w(size.width)
                            .h(size.height)
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                backdrop.update(cx, |m, cx| m.dismiss(window, cx))
                            })
                            .child(menu.clone()),
                    ),
                )
                .with_priority(1)
                .into_any_element(),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(outcome: PushOutcome) -> PushResult {
        PushResult {
            source: "main".into(),
            target: PushTarget {
                remote: "origin".into(),
                branch: "main".into(),
                new: false,
            },
            outcome,
            tags: Vec::new(),
        }
    }

    #[test]
    fn push_balloons() {
        let n = push_notification(&result(PushOutcome::Success { commits: 2 }), None);
        assert_eq!(n.kind, NotificationKind::Information);
        assert_eq!(n.title.as_ref(), "Pushed 2 commits to origin/main");
        assert_eq!(n.content.as_ref(), "");
        let n = push_notification(
            &result(PushOutcome::Success { commits: 2 }),
            Some(&UpdateOfPush::Updated(1)),
        );
        assert_eq!(
            n.content.as_ref(),
            "1 commit received during the push operation"
        );
        let n = push_notification(&result(PushOutcome::RejectedNoFastForward), None);
        assert_eq!(
            (n.kind, n.title.as_ref(), n.content.as_ref()),
            (
                NotificationKind::Warning,
                "Push rejected",
                "Push to origin/main was rejected"
            )
        );
        let n = push_notification(
            &result(PushOutcome::RejectedNoFastForward),
            Some(&UpdateOfPush::Failed),
        );
        assert_eq!(
            n.content.as_ref(),
            "Push was rejected, and update failed with an error"
        );
        let n = push_notification(&result(PushOutcome::RejectedStaleInfo), None);
        assert_eq!(n.actions[0].text.as_ref(), "Force push anyway");
        assert_eq!(n.actions[0].args["force"], "force");
        let n = push_notification(
            &result(PushOutcome::Error {
                message: "fatal: no".into(),
            }),
            None,
        );
        assert_eq!(
            (n.kind, n.title.as_ref(), n.content.as_ref()),
            (NotificationKind::Error, "Push failed", "fatal: no")
        );
    }

    #[test]
    fn update_balloons() {
        let n = update_notifications(&UpdateResult::Updated {
            commits: 1,
            files: 2,
            restore_conflicts: false,
        });
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title.as_ref(), "2 files updated in 1 commit");
        let n = update_notifications(&UpdateResult::UpToDate);
        assert_eq!(n[0].content.as_ref(), "All files are up to date");
        let n = update_notifications(&UpdateResult::Incomplete {
            method: UpdateMethod::Merge,
            stashed: true,
        });
        assert_eq!(n[0].title.as_ref(), "Cannot complete update");
        assert_eq!(n[1].title.as_ref(), "Local changes were not restored");
        assert_eq!(
            rejected_message("main"),
            "Push of the current branch \"main\" was rejected. \
             Remote changes need to be merged before pushing."
        );
    }
}
