//! The VCS widget of the main toolbar and what it opens (#28): the
//! branches popup with the actions of a branch, Create Git Repository…
//! without a repository, the New Branch, Rename Branch and Checkout
//! dialogs, "Git Checkout Problem" (Smart Checkout, Force Checkout, Don't
//! Checkout) and the balloons of the branch commands ("Checked out {0}",
//! "Checked out new branch {0} from {1}", "**Deleted Branch:** {0}" with
//! Restore; the names in the editor font, as the reference's `<code>`).
//! The reference's Tasks plugin may replace the checkout balloon with
//! "Workspace associated with branch … has been restored" (branch
//! workspaces); without that feature the plain checkout text shows.
//!
//! Every git call runs off the UI thread through the project's
//! [`git::GitRepo`]; the statuses and the widget follow from the
//! repository's files (the status service refreshes after each write).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use git::{CheckedOut, Checkout as Done, CheckoutError, CheckoutMode, GitRepo};
use gpui::{
    Action, AnyElement, App, AppContext as _, Bounds, Context, Corner, DismissEvent, Entity,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, PathPromptOptions, Pixels, Point, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, anchored, canvas, div, point, prelude::*, px,
};
use ui::components::Tooltip;
use ui::deferred;

use crate::main_toolbar::{Cut, Measure, Widget};
use serde_json::{Value as Json, json};
use ui::icons::IconName;
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::notification::{Notification, NotificationKind};
use vcs::{Backend, BranchKind, Cancel, Head};
use vcs_ui::branches::{
    BranchData, BranchesPopup, BranchesPopupEvent, Options, PopupSetup, RECENT_ENTRIES, TopAction,
};
use vcs_ui::new_branch::{BranchDialog, BranchDialogEvent, DialogKind};
use vcs_ui::widget::{WidgetState, presentation};
use vcs_ui::{
    Checkout, CheckoutRevision, DeleteBranch, Init, NewBranch, RenameBranch, RestoreBranch,
    ShowBranches, UpdateBranch,
};

use super::Workbench;
use crate::dialog::{Dialog, DialogButton, DialogEvent};

/// The widget's popups and dialogs.
#[derive(Default)]
pub(super) struct BranchUi {
    /// Where the widget is (the popups open below it).
    pub anchor: Rc<Cell<Bounds<Pixels>>>,
    popup: Option<OpenPopup>,
    /// Create Git Repository… without a repository.
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    dialog: Option<OpenDialog>,
    problem: Option<OpenProblem>,
    options: Options,
    /// The tag `HEAD` is at when detached: the commit and its tag.
    tag: Option<(String, Option<String>)>,
    _tag_lookup: Option<Task<()>>,
}

struct OpenPopup {
    popup: Entity<BranchesPopup>,
    return_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
    _load: Task<()>,
}

/// What a branch dialog does when confirmed.
#[derive(Debug, Clone)]
enum Purpose {
    Create { from: Option<String> },
    Rename { from: String },
    Checkout,
}

struct OpenDialog {
    dialog: Entity<BranchDialog>,
    purpose: Purpose,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

struct OpenProblem {
    dialog: Entity<Dialog>,
    target: String,
    detach: bool,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

/// A name as the reference's balloons show it (`GitUIUtil.code`).
fn code(name: &str) -> String {
    format!("<code>{}</code>", ui::notification::escape(name))
}

/// A checkout's balloon (`checkout.operation.checked.out`,
/// `checked.out.new.branch.from`), as HTML ([`Notification::html`]).
fn checked_out_text(done: &CheckedOut) -> String {
    match done {
        CheckedOut::Branch { name } => format!("Checked out {}", code(name)),
        CheckedOut::NewBranch { name, from } => {
            format!("Checked out new branch {} from {}", code(name), code(from))
        }
        CheckedOut::Detached { target } => format!("Checked out {}", code(target)),
    }
}

/// The balloon of a deleted branch (`delete.branch.operation.deleted.branch.bold`).
fn deleted_text(name: &str) -> String {
    format!("<b>Deleted Branch:</b> {}", ui::notification::escape(name))
}

/// The message of "Git Checkout Problem" (`smart.operation.dialog.north.
/// panel.label.stash.text`) with the files.
fn problem_message(files: &[String]) -> String {
    let mut text = String::from(
        "Your local changes to the following files would be overwritten by checkout.\n",
    );
    text.push_str(&format!(
        "{} can stash the changes, checkout and unstash them after that.\n",
        app_info::name()
    ));
    for f in files.iter().take(20) {
        text.push('\n');
        text.push_str(f);
    }
    if files.len() > 20 {
        text.push_str(&format!("\n\u{2026} and {} more", files.len() - 20));
    }
    text
}

impl Workbench {
    /// What the VCS widget shows.
    pub(super) fn vcs_widget_state(&self) -> WidgetState {
        let vc = &self.version_control;
        if self.file_sync_root().is_none() || !vc.detected {
            return WidgetState::Hidden;
        }
        if vc.off {
            return WidgetState::NoVcs;
        }
        match (&vc.service, &vc.snapshot) {
            (_, Some(snapshot)) => {
                let info = &snapshot.info;
                let tag = match (&info.head, &self.branch_ui.tag) {
                    (Head::Detached { oid }, Some((at, tag))) if at == oid => tag.as_deref(),
                    _ => None,
                };
                WidgetState::Repository(presentation(info, tag))
            }
            (Some(_), None) => WidgetState::Loading,
            (None, None) if vc.problem.is_some() => WidgetState::Loading,
            (None, None) => WidgetState::NoVcs,
        }
    }

    /// A new snapshot: with `HEAD` detached at another commit, the tag it
    /// is at is looked up off the UI thread.
    pub(super) fn vcs_branch_snapshot(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.version_control.snapshot.clone() else {
            return;
        };
        let Head::Detached { oid } = &snapshot.info.head else {
            self.branch_ui.tag = None;
            return;
        };
        if self.branch_ui.tag.as_ref().is_some_and(|(at, _)| at == oid) {
            return;
        }
        let Some(repo) = self.git_repo() else {
            return;
        };
        let oid = oid.clone();
        let lookup = cx.background_spawn({
            let oid = oid.clone();
            async move {
                repo.branches(&Cancel::new()).ok().and_then(|list| {
                    list.into_iter()
                        .find(|b| b.kind == BranchKind::Tag && b.oid == oid)
                        .map(|b| b.name)
                })
            }
        });
        self.branch_ui._tag_lookup = Some(cx.spawn(async move |this, cx| {
            let tag = lookup.await;
            let _ = this.update(cx, |this, cx| {
                this.branch_ui.tag = Some((oid, tag));
                cx.notify();
            });
        }));
    }

    /// The open branches popup for `vcs.state`.
    pub(super) fn vcs_branches_popup_json(&self, cx: &App) -> Json {
        match (&self.branch_ui.popup, &self.branch_ui.menu) {
            (Some(open), _) => open.popup.read(cx).state_json(cx),
            (None, Some((menu, _))) => menu.read(cx).state_json(),
            (None, None) => Json::Null,
        }
    }

    /// The open branch dialog or checkout problem for `vcs.state`.
    pub(super) fn vcs_branch_dialog_json(&self, cx: &App) -> Json {
        if let Some(open) = &self.branch_ui.dialog {
            return open.dialog.read(cx).state_json(cx);
        }
        if let Some(open) = &self.branch_ui.problem {
            return open.dialog.read(cx).state_json();
        }
        Json::Null
    }

    /// The branch commands on the window's root.
    pub(super) fn branch_actions<E: InteractiveElement>(
        &self,
        root: E,
        cx: &mut Context<Self>,
    ) -> E {
        root.on_action(cx.listener(Self::vcs_show_branches))
            .on_action(cx.listener(Self::vcs_new_branch))
            .on_action(cx.listener(Self::vcs_checkout))
            .on_action(cx.listener(Self::vcs_checkout_revision))
            .on_action(cx.listener(Self::vcs_rename_branch))
            .on_action(cx.listener(Self::vcs_delete_branch))
            .on_action(cx.listener(Self::vcs_restore_branch))
            .on_action(cx.listener(Self::vcs_init))
    }

    // ---- popups -------------------------------------------------------------

    pub(super) fn close_branch_popups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.branch_ui.popup.take()
            && let Some(focus) = open.return_focus
        {
            window.focus(&focus);
        }
        // The menu gives the focus back where it was.
        if let Some((menu, _)) = self.branch_ui.menu.take() {
            menu.update(cx, |m, cx| m.dismiss(window, cx));
        }
        cx.notify();
    }

    fn vcs_show_branches(&mut self, _: &ShowBranches, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch_ui.popup.is_some() || self.branch_ui.menu.is_some() {
            self.close_branch_popups(window, cx);
            return;
        }
        match self.vcs_widget_state() {
            WidgetState::Hidden => {}
            WidgetState::NoVcs => self.open_no_vcs_menu(window, cx),
            WidgetState::Loading | WidgetState::Repository(_) => {
                self.open_branches_popup(window, cx)
            }
        }
    }

    /// The popup without a repository: Create Git Repository….
    fn open_no_vcs_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![MenuItem::action(
            "Create Git Repository\u{2026}",
            Init { path: None },
        )];
        let menu = ContextMenu::popup(
            items,
            PopupOptions {
                speed_search: true,
                ..PopupOptions::default()
            },
            window,
            cx,
        );
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.branch_ui.menu = None;
            cx.notify();
        });
        self.branch_ui.menu = Some((menu, subscription));
        cx.notify();
    }

    /// A top action of the popup that comes with another feature: shown
    /// disabled until its command is there.
    pub(super) fn optional_action(id: &str, cx: &App) -> Option<Box<dyn Action>> {
        ui::command::list(cx)
            .iter()
            .any(|c| c.meta.id == id)
            .then(|| cx.build_action(id, None).ok())
            .flatten()
    }

    fn top_actions(&self, fresh: bool, window: &Window, cx: &App) -> Vec<TopAction> {
        let shortcut = |action: &Option<Box<dyn Action>>| {
            action
                .as_ref()
                .and_then(|a| ui::keys::shortcut_in(a.as_ref(), &self.focus, window))
                .map(Into::into)
        };
        let entry = |label: &'static str,
                     icon: Option<IconName>,
                     action: Option<Box<dyn Action>>,
                     separator_after: bool| TopAction {
            label,
            icon,
            shortcut: shortcut(&action),
            action,
            separator_after,
        };
        let new_branch: Box<dyn Action> = Box::new(NewBranch {
            name: None,
            from: None,
            checkout: None,
        });
        let revision: Box<dyn Action> = Box::new(CheckoutRevision { r#ref: None });
        vec![
            entry(
                "Update Project\u{2026}",
                Some(IconName::VCS_UPDATE),
                Self::optional_action("vcs.update-project", cx),
                false,
            ),
            entry(
                "Commit\u{2026}",
                Some(IconName::COMMIT),
                Self::optional_action("vcs.commit", cx),
                false,
            ),
            entry(
                "Push\u{2026}",
                Some(IconName::PUSH),
                Self::optional_action("vcs.push", cx),
                true,
            ),
            entry(
                "New Branch\u{2026}",
                Some(IconName::ADD),
                (!fresh).then_some(new_branch),
                false,
            ),
            entry(
                "Checkout Tag or Revision\u{2026}",
                None,
                (!fresh).then_some(revision),
                true,
            ),
        ]
    }

    /// The branches of the repository and the recent checkouts, off the
    /// UI thread.
    fn load_branch_data(&self, cx: &mut Context<Self>) -> Task<Option<BranchData>> {
        let Some(repo) = self.git_repo() else {
            return Task::ready(None);
        };
        cx.background_spawn(async move {
            let c = Cancel::new();
            let branches = repo.branches(&c).ok()?;
            let recent = repo
                .recent_checkouts(RECENT_ENTRIES, &c)
                .unwrap_or_default();
            Some(BranchData {
                branches,
                recent,
                loaded: true,
            })
        })
    }

    fn open_branches_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fresh = self
            .version_control
            .snapshot
            .as_ref()
            .is_some_and(|s| s.info.head.oid().is_none());
        let setup = PopupSetup {
            actions: self.top_actions(fresh, window, cx),
            // A flag: the branch's Update carries its name.
            update: Some(Box::new(UpdateBranch {
                name: String::new(),
            })),
            push: Self::optional_action("vcs.push", cx),
            fetch: Self::optional_action("git.fetch", cx),
            options: self.branch_ui.options,
        };
        let popup = cx.new(|cx| BranchesPopup::new(setup, cx));
        let return_focus = window.focused(cx);
        window.focus(&popup.focus_handle(cx));
        let subscriptions = vec![
            cx.subscribe_in(&popup, window, |this, _, event, window, cx| match event {
                BranchesPopupEvent::Run(action) => {
                    let action = action.boxed_clone();
                    this.close_branch_popups(window, cx);
                    window.dispatch_action(action, cx);
                }
                BranchesPopupEvent::OptionsChanged(options) => this.branch_ui.options = *options,
            }),
            cx.subscribe_in(&popup, window, |this, _, _: &DismissEvent, window, cx| {
                this.close_branch_popups(window, cx)
            }),
            cx.observe(&popup, |_, _, cx| cx.notify()),
        ];
        let load = self.load_branch_data(cx);
        let target = popup.downgrade();
        let task = cx.spawn(async move |_, cx| {
            if let Some(data) = load.await {
                let _ = target.update(cx, |p, cx| p.set_data(data, cx));
            }
        });
        self.branch_ui.popup = Some(OpenPopup {
            popup,
            return_focus,
            _subscriptions: subscriptions,
            _load: task,
        });
        cx.notify();
    }

    // ---- dialogs ------------------------------------------------------------

    /// Opens a branch dialog once the branches it checks names against are
    /// read.
    fn open_branch_dialog(
        &mut self,
        kind: DialogKind,
        purpose: Purpose,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let load = self.load_branch_data(cx);
        cx.spawn_in(window, async move |this, cx| {
            let branches = load.await.map(|d| d.branches).unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                let dialog = cx.new(|cx| BranchDialog::new(kind, &initial, branches, window, cx));
                let return_focus = window.focused(cx);
                window.focus(&dialog.focus_handle(cx));
                let subscription =
                    cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
                        this.branch_dialog_closed(event, window, cx)
                    });
                this.branch_ui.dialog = Some(OpenDialog {
                    dialog,
                    purpose,
                    return_focus,
                    _subscription: subscription,
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn branch_dialog_closed(
        &mut self,
        event: &BranchDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.branch_ui.dialog.take() else {
            return;
        };
        if let Some(focus) = &open.return_focus {
            window.focus(focus);
        }
        cx.notify();
        let BranchDialogEvent::Confirmed {
            name,
            checkout,
            overwrite,
        } = event.clone()
        else {
            return;
        };
        match open.purpose {
            Purpose::Create { from } => {
                self.create_branch(name, from, checkout, overwrite, window, cx)
            }
            Purpose::Rename { from } => self.rename(from, name, cx),
            Purpose::Checkout => self.run_checkout(name, CheckoutMode::Normal, true, window, cx),
        }
    }

    /// "Git Checkout Problem": the local changes in the way of a checkout.
    fn open_checkout_problem(
        &mut self,
        target: String,
        detach: bool,
        files: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| {
            Dialog::new(
                "Git Checkout Problem",
                problem_message(&files),
                vec![
                    DialogButton {
                        id: "force",
                        label: "&Force Checkout".into(),
                    },
                    DialogButton {
                        id: "cancel",
                        label: "Don't Checkout".into(),
                    },
                    DialogButton {
                        id: "smart",
                        label: "Smart Checkout".into(),
                    },
                ],
                cx,
            )
            .with_icon(IconName::WARNING_DIALOG)
        });
        let return_focus = window.focused(cx);
        window.focus(&dialog.focus_handle(cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| {
            let DialogEvent::Closed { button, .. } = event;
            let Some(open) = this.branch_ui.problem.take() else {
                return;
            };
            if let Some(focus) = &open.return_focus {
                window.focus(focus);
            }
            cx.notify();
            let mode = match button {
                Some("smart") => CheckoutMode::Smart,
                Some("force") => CheckoutMode::Force,
                _ => return,
            };
            this.run_checkout(open.target, mode, open.detach, window, cx);
        });
        self.branch_ui.problem = Some(OpenProblem {
            dialog,
            target,
            detach,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    // ---- commands -------------------------------------------------------------

    /// A balloon without a title; `html` as [`Notification::html`] reads it.
    pub(super) fn info_balloon(&mut self, html: String, cx: &mut Context<Self>) {
        self.notify(
            Notification::html(NotificationKind::Information, "", &html),
            cx,
        );
    }

    pub(super) fn error_balloon(&mut self, title: String, text: String, cx: &mut Context<Self>) {
        self.notify(Notification::new(NotificationKind::Error, title, text), cx);
    }

    /// Runs `f` with the repository off the UI thread, then `then` with its
    /// result in the window.
    pub(super) fn with_repo<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&GitRepo) -> T + Send + 'static,
        then: impl FnOnce(&mut Workbench, T, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        let Some(repo) = self.git_repo() else {
            self.show_notice("There is no Git repository".into(), cx);
            return;
        };
        let work = cx.background_spawn(async move { f(&repo) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| then(this, result, window, cx));
        })
        .detach();
    }

    pub(super) fn current_branch(&self) -> Option<String> {
        self.version_control
            .snapshot
            .as_ref()
            .and_then(|s| s.info.head.branch().map(str::to_owned))
    }

    fn vcs_new_branch(&mut self, action: &NewBranch, window: &mut Window, cx: &mut Context<Self>) {
        self.close_branch_popups(window, cx);
        let checkout = action.checkout.unwrap_or(true);
        match &action.name {
            Some(name) => {
                let name = vcs::branch_name::clean_up(name);
                let from = action.from.clone();
                self.with_repo(
                    window,
                    cx,
                    {
                        let name = name.clone();
                        move |repo| {
                            let branches = repo.branches(&Cancel::new()).unwrap_or_default();
                            vcs::branch_name::check_new(&name, &branches, false, true)
                                .map_err(|e| e.to_string())
                        }
                    },
                    move |this, checked, window, cx| match checked {
                        Ok(()) => this.create_branch(name, from, checkout, false, window, cx),
                        Err(e) => {
                            this.error_balloon(format!("Could not create new branch {name}"), e, cx)
                        }
                    },
                );
            }
            None => {
                let (kind, initial) = match &action.from {
                    Some(from) => {
                        // A remote branch's name without its remote
                        // (`calculateNewBranchInitialName`).
                        let initial = self
                            .remote_branch_local_name(from)
                            .unwrap_or_else(|| from.clone());
                        (
                            DialogKind::Create {
                                from: Some(from.clone()),
                            },
                            initial,
                        )
                    }
                    None => (
                        DialogKind::Create { from: None },
                        self.current_branch().unwrap_or_default(),
                    ),
                };
                self.open_branch_dialog(
                    kind,
                    Purpose::Create {
                        from: action.from.clone(),
                    },
                    initial,
                    window,
                    cx,
                );
            }
        }
    }

    /// `origin/feat` as `feat` when `origin` is a remote.
    fn remote_branch_local_name(&self, name: &str) -> Option<String> {
        let snapshot = self.version_control.snapshot.as_ref()?;
        snapshot.info.remotes.iter().find_map(|r| {
            name.strip_prefix(&r.name)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(str::to_owned)
        })
    }

    fn create_branch(
        &mut self,
        name: String,
        from: Option<String>,
        checkout: bool,
        overwrite: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // From another branch the new one starts at its commit, without
        // tracking it (`GitCheckoutAsNewBranch`: `<branch>^0`).
        let start = from.as_ref().map(|f| format!("{f}^0"));
        let shown_from = from
            .clone()
            .or_else(|| self.current_branch())
            .unwrap_or_else(|| "HEAD".to_owned());
        self.with_repo(
            window,
            cx,
            {
                let name = name.clone();
                move |repo| {
                    repo.new_branch(&name, start.as_deref(), checkout, overwrite, &Cancel::new())
                }
            },
            move |this, result, _, cx| match result {
                Ok(()) if checkout => this.info_balloon(
                    checked_out_text(&CheckedOut::NewBranch {
                        name: name.clone(),
                        from: shown_from.clone(),
                    }),
                    cx,
                ),
                Ok(()) => this.info_balloon(format!("Branch {} was created", code(&name)), cx),
                Err(e) => this.error_balloon(
                    format!("Could not create new branch {name}"),
                    e.to_string(),
                    cx,
                ),
            },
        );
    }

    fn vcs_checkout(&mut self, action: &Checkout, window: &mut Window, cx: &mut Context<Self>) {
        self.close_branch_popups(window, cx);
        let mode = match action.mode.as_deref() {
            Some("force") => CheckoutMode::Force,
            Some("smart") => CheckoutMode::Smart,
            _ => CheckoutMode::Normal,
        };
        self.run_checkout(action.r#ref.clone(), mode, false, window, cx);
    }

    fn vcs_checkout_revision(
        &mut self,
        action: &CheckoutRevision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branch_popups(window, cx);
        match &action.r#ref {
            Some(target) => {
                self.run_checkout(target.clone(), CheckoutMode::Normal, true, window, cx)
            }
            None => self.open_branch_dialog(
                DialogKind::Checkout,
                Purpose::Checkout,
                String::new(),
                window,
                cx,
            ),
        }
    }

    fn run_checkout(
        &mut self,
        target: String,
        mode: CheckoutMode,
        detach: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_repo(
            window,
            cx,
            {
                let target = target.clone();
                move |repo| {
                    let c = Cancel::new();
                    if detach {
                        repo.checkout_detached(&target, mode, &c)
                    } else {
                        repo.checkout(&target, mode, &c)
                    }
                }
            },
            move |this, result: Result<Done, CheckoutError>, window, cx| match result {
                Ok(done) => {
                    this.info_balloon(checked_out_text(&done.checked_out), cx);
                    if done.conflicts_on_restore {
                        this.notify(
                            Notification::new(
                                NotificationKind::Warning,
                                "Local changes were restored with conflicts",
                                "Before checkout your uncommitted changes were saved to stash.",
                            ),
                            cx,
                        );
                    }
                }
                Err(CheckoutError::LocalChanges(files)) => {
                    this.open_checkout_problem(target, detach, files, window, cx)
                }
                Err(CheckoutError::NotFound(_)) => this.error_balloon(
                    format!("Could not checkout {target}"),
                    "Revision not found".to_owned(),
                    cx,
                ),
                Err(e) => {
                    this.error_balloon(format!("Could not checkout {target}"), e.to_string(), cx)
                }
            },
        );
    }

    fn vcs_rename_branch(
        &mut self,
        action: &RenameBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branch_popups(window, cx);
        let Some(from) = action.from.clone().or_else(|| self.current_branch()) else {
            self.show_notice("There is no branch to rename".into(), cx);
            return;
        };
        match &action.to {
            Some(to) => {
                let to = vcs::branch_name::clean_up(to);
                self.rename(from, to, cx)
            }
            None => self.open_branch_dialog(
                DialogKind::Rename { from: from.clone() },
                Purpose::Rename { from: from.clone() },
                from,
                window,
                cx,
            ),
        }
    }

    fn rename(&mut self, from: String, to: String, cx: &mut Context<Self>) {
        let Some(repo) = self.git_repo() else {
            return;
        };
        let work = cx.background_spawn({
            let (from, to) = (from.clone(), to.clone());
            async move {
                let c = Cancel::new();
                let branches = repo.branches(&c).unwrap_or_default();
                vcs::branch_name::check_new(&to, &branches, false, true)
                    .map_err(|e| e.to_string())?;
                repo.rename_branch(&from, &to, &c)
                    .map_err(|e| e.to_string())
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.info_balloon(
                    format!(
                        "Branch <b>{}</b> was renamed to <b>{}</b>",
                        code(&from),
                        code(&to)
                    ),
                    cx,
                ),
                Err(e) => this.error_balloon(format!("Could not rename {from}"), e, cx),
            });
        })
        .detach();
    }

    fn vcs_delete_branch(
        &mut self,
        action: &DeleteBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branch_popups(window, cx);
        let name = action.name.clone();
        if self.current_branch().as_deref() == Some(name.as_str()) {
            self.error_balloon(
                format!("Branch {name} wasn't deleted"),
                "The current branch cannot be deleted".to_owned(),
                cx,
            );
            return;
        }
        self.with_repo(
            window,
            cx,
            {
                let name = name.clone();
                move |repo| repo.delete_branch(&name, &Cancel::new())
            },
            move |this, result, _, cx| match result {
                Ok(oid) => {
                    this.notify(
                        Notification::html(NotificationKind::Information, "", &deleted_text(&name))
                            .with_action(
                                "Restore",
                                "git.restore-branch",
                                json!({ "name": name, "oid": oid }),
                            ),
                        cx,
                    );
                }
                Err(e) => {
                    this.error_balloon(format!("Branch {name} wasn't deleted"), e.message, cx)
                }
            },
        );
    }

    fn vcs_restore_branch(
        &mut self,
        action: &RestoreBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (name, oid) = (action.name.clone(), action.oid.clone());
        self.with_repo(
            window,
            cx,
            {
                let name = name.clone();
                move |repo| repo.restore_branch(&name, &oid, &Cancel::new())
            },
            move |this, result, _, cx| {
                if let Err(e) = result {
                    this.error_balloon(format!("Could not Restore {name}"), e.message, cx);
                }
            },
        );
    }

    fn vcs_init(&mut self, action: &Init, window: &mut Window, cx: &mut Context<Self>) {
        self.close_branch_popups(window, cx);
        let root = self.file_sync_root().map(|p| p.to_path_buf());
        match &action.path {
            Some(path) => {
                let path = PathBuf::from(path);
                let path = match (&root, path.is_relative()) {
                    (Some(root), true) => native_path::join(root, &path),
                    _ => path,
                };
                self.init_repository(path, cx);
            }
            None => {
                let chosen = cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Create Git Repository".into()),
                });
                cx.spawn_in(window, async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = chosen.await
                        && let Some(path) = paths.into_iter().next()
                    {
                        let _ = this.update(cx, |this, cx| this.init_repository(path, cx));
                    }
                })
                .detach();
            }
        }
    }

    /// `git init` in `folder`; the project's repository is looked for again.
    fn init_repository(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        let work = cx.background_spawn({
            let folder = folder.clone();
            async move {
                let exe = git::GitExe::find(None).map_err(|e| e.to_string())?;
                git::branches::init(&exe, &folder, &Cancel::new()).map_err(|e| e.message)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    if let Some(root) = this.file_sync_root().map(|p| p.to_path_buf())
                        && (root == folder || root.starts_with(&folder))
                    {
                        this.vcs_project_opened(&root, cx);
                    }
                }
                Err(e) => this.error_balloon("Git init failed".to_owned(), e, cx),
            });
        })
        .detach();
    }

    // ---- drawing --------------------------------------------------------------

    /// The VCS widget of the toolbar's left group with the room before it;
    /// nothing while it is hidden. Its text gives way like the project's.
    pub(super) fn vcs_toolbar_widget<'a>(
        &self,
        measure: &Measure<'_>,
        cx: &'a App,
    ) -> Vec<Widget<'a>> {
        let state = self.vcs_widget_state();
        let Some(text) = state.text() else {
            return Vec::new();
        };
        let measured = measure.text(text, FontWeight::NORMAL, Cut::End);
        let icon = state.icon();
        let (incoming, outgoing) = match &state {
            WidgetState::Repository(p) => (p.incoming, p.outgoing),
            _ => (false, false),
        };
        let tooltip = state.tooltip();
        let anchor = self.branch_ui.anchor.clone();
        let chrome = crate::chrome::vcs_widget_chrome(
            icon.is_some(),
            usize::from(incoming) + usize::from(outgoing),
        );
        vec![
            Widget::space(crate::chrome::VCS_WIDGET_SPACE),
            Widget::text(chrome, measured, move |text| {
                crate::chrome::vcs_widget(icon, &text, incoming, outgoing, cx)
                    .child(
                        canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    )
                    .relative()
                    .when_some(tooltip, |d, tip| d.tooltip(Tooltip::text(tip, None)))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowBranches), cx))
                    .into_any_element()
            }),
        ]
    }

    /// The popups and dialogs of the widget, over the window.
    pub(super) fn render_branch_overlays(&self, window: &Window, cx: &App) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let anchor = self.branch_ui.anchor.get();
        let below: Point<Pixels> = point(anchor.origin.x - px(4.0), anchor.bottom() + px(4.0));
        if let Some(open) = &self.branch_ui.popup {
            out.push(popup_overlay(open.popup.clone(), below, window));
            let popup = open.popup.read(cx);
            if let Some((menu, offset)) = popup.open_menu() {
                out.push(ui::menu::overlay_at(
                    &menu,
                    below + offset,
                    Corner::TopLeft,
                    window,
                ));
            }
        }
        if let Some((menu, _)) = &self.branch_ui.menu {
            out.push(ui::menu::overlay_at(menu, below, Corner::TopLeft, window));
        }
        if let Some(open) = &self.branch_ui.dialog {
            out.push(open.dialog.clone().into_any_element());
        }
        if let Some(open) = &self.branch_ui.problem {
            out.push(open.dialog.clone().into_any_element());
        }
        out
    }
}

/// The branches popup at `position` above everything else, with a
/// backdrop that closes it on a click outside.
fn popup_overlay(
    popup: Entity<BranchesPopup>,
    position: Point<Pixels>,
    window: &Window,
) -> AnyElement {
    let size = window.viewport_size();
    let backdrop = popup.clone();
    deferred(
        anchored().position(point(px(0.0), px(0.0))).child(
            div()
                .id("branches-backdrop")
                .occlude()
                .w(size.width)
                .h(size.height)
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    backdrop.update(cx, |p, cx| p.dismiss(cx))
                })
                .child(
                    anchored()
                        .position(position)
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(4.0))
                        .child(popup),
                ),
        ),
    )
    .with_priority(1)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balloon_texts() {
        assert_eq!(
            checked_out_text(&CheckedOut::Branch {
                name: "main".into()
            }),
            "Checked out <code>main</code>"
        );
        assert_eq!(
            checked_out_text(&CheckedOut::NewBranch {
                name: "feat".into(),
                from: "origin/feat".into()
            }),
            "Checked out new branch <code>feat</code> from <code>origin/feat</code>"
        );
        assert_eq!(deleted_text("a<b"), "<b>Deleted Branch:</b> a&lt;b");
        let message = problem_message(&["a.py".into()]);
        assert!(message.starts_with(
            "Your local changes to the following files would be overwritten by checkout."
        ));
        assert!(message.ends_with("\n\na.py"));
    }
}
