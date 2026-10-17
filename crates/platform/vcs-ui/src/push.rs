//! The Push dialog (`VcsPushDialog` with the Git push support): "Push
//! Commits to {repository}", the push log on the left (the repository row
//! `main → origin : main` with the `New` / `Upstream` label, then the
//! commits to push) and the files of the selected commits on the right;
//! Push tags with its mode at the bottom left; Cancel and Push, whose
//! arrow offers Force Push (`--force-with-lease`; disabled for the
//! protected branches `main` and `master`).
//!
//! The remote of the repository row is a link that lists the remotes; a
//! click on the target branch (or Edit Target) edits it in place with Set
//! upstream beside the field, Enter keeps the name, Escape restores it.
//! The dialog loads the commits and the files off the UI thread through
//! the [`PushData`] its owner gives it, and reports what to push with
//! [`PushDialogEvent`]; the owner pushes.

use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Corner, DismissEvent, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, IntoElement, MouseButton, Pixels, Point, Render, SharedString,
    Subscription, Task, Window, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use text::Buffer;
use themes::UiRole;
use ui::components::{Tooltip, checkbox, input_frame, popup_shadow};
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;
use vcs::{
    CommitFile, ForceMode, OutgoingCommit, PushSpec, PushTarget, TagMode, VcsError,
    branch_name::clean_up,
};

use editor::{Editor, EditorConfig};

use crate::status_colors::status_role;

ui::ui_commands! {
    COMMANDS, scope = "PushDialog";
    /// Push.
    Push => "push-dialog.push", "Push",
        "Pushes the commits of the Push dialog to its target and closes it.";
    /// Force Push.
    ForcePush => "push-dialog.force-push", "Force Push",
        "Force pushes (with lease) the commits of the Push dialog after a confirmation; not for the protected branches main and master.";
    /// Cancel.
    CancelPush => "push-dialog.cancel", "Cancel", "Closes the Push dialog without pushing.";
    /// The next row of the push log.
    Down => "push-dialog.down", "Next", "Selects the next row of the push log.";
    /// The previous row of the push log.
    Up => "push-dialog.up", "Previous", "Selects the previous row of the push log.";
    /// Select a row.
    Select { row: usize } => "push-dialog.select", "Select Row",
        "Selects a row of the push log: 0 is the repository (all commits), then the commits newest first; the files of the selection show on the right.",
        r#"{"row": 1}"#;
    /// Edit the target branch.
    EditTarget => "push-dialog.edit-target", "Edit Target",
        "Edits the target branch of the Push dialog in place (as a click on it does).";
    /// Keep the edited target.
    ConfirmTarget => "push-dialog.confirm-target", "Confirm Target",
        "Keeps the target branch typed in the Push dialog's field.";
    /// Restore the target.
    CancelTarget => "push-dialog.cancel-target", "Cancel Editing",
        "Stops editing the target branch of the Push dialog and restores it.";
    /// Set the target.
    SetTarget { branch: String, remote: Option<String> } => "push-dialog.set-target", "Set Target",
        "Sets the target branch (and remote) of the Push dialog.",
        r#"{"branch": "feat/x", "remote": "origin"}"#;
    /// Push tags.
    ToggleTags => "push-dialog.toggle-tags", "Push Tags",
        "Toggles Push tags of the Push dialog.";
    /// The tags to push.
    SetTagMode { mode: String } => "push-dialog.set-tag-mode", "Push Tags Mode",
        "Sets which tags Push tags pushes: all or current_branch (annotated tags of the pushed commits).",
        r#"{"mode": "current_branch"}"#;
    /// Set upstream.
    ToggleUpstream => "push-dialog.toggle-upstream", "Set Upstream",
        "Toggles Set upstream of the Push dialog (shown while the target differs from the branch's upstream).";
}

/// Key context of the dialog.
pub const KEY_CONTEXT: &str = "PushDialog";

/// The push log's size (`CENTER_PANEL_WIDTH`, `CENTER_PANEL_HEIGHT`).
pub const WIDTH: f32 = 800.0;
const LOG_HEIGHT: f32 = 450.0;
const TITLE_HEIGHT: f32 = 40.0;
const ROW: f32 = 24.0;
/// Indent of commit rows (`"   "` before the subject) and of the rows.
const ROW_X: f32 = 12.0;
const COMMIT_X: f32 = 36.0;
const FIELD_HEIGHT: f32 = 24.0;

/// Branches force push is prohibited for by default
/// (`FORCE_PUSH_PROHIBITED_PATTERNS`).
pub const PROTECTED_BRANCHES: &[&str] = &["master", "main"];

/// The `New` / `Upstream` label (`BranchLabels`): green on a pale green.
const LABEL_FG: (u32, u32) = (0x00b53d, 0x6ba65d);
const LABEL_BG: (u32, u32) = (0xebfcf1, 0x313b32);

/// What the dialog reads from the repository, off the UI thread.
pub trait PushData: Send + Sync {
    /// A branch of a remote as a target (new or not).
    fn target(&self, remote: &str, branch: &str) -> Result<PushTarget, VcsError>;
    /// The commits a push of `source` to `target` sends, newest first.
    fn outgoing(&self, source: &str, target: &PushTarget) -> Result<Vec<OutgoingCommit>, VcsError>;
    /// The files the commits change, as one change.
    fn files(&self, commits: &[String]) -> Result<Vec<CommitFile>, VcsError>;
}

/// What the dialog opens with.
pub struct PushSetup {
    /// The repository's short name (its folder).
    pub repository: String,
    /// The local branch pushed.
    pub source: String,
    pub remotes: Vec<String>,
    /// `None` without remotes.
    pub target: Option<PushTarget>,
    /// The source's upstream (remote, branch): Set upstream shows for
    /// other targets.
    pub upstream: Option<(String, String)>,
    pub data: Arc<dyn PushData>,
}

/// What the dialog asks of its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum PushDialogEvent {
    /// Push (force pushes ask first, the dialog stays until then).
    Push(PushSpec),
    Cancelled,
}

enum Commits {
    Loading,
    Loaded(Vec<OutgoingCommit>),
    Failed(String),
}

enum Files {
    None,
    Loading,
    Loaded(Vec<CommitFile>),
}

pub struct PushDialog {
    focus: FocusHandle,
    repository: String,
    source: String,
    remotes: Vec<String>,
    target: Option<PushTarget>,
    upstream: Option<(String, String)>,
    data: Arc<dyn PushData>,
    commits: Commits,
    files: Files,
    /// 0: the repository row; then the commits.
    selected: usize,
    /// The field of the target branch while it is edited.
    editing: Option<Entity<Editor>>,
    set_upstream: bool,
    push_tags: bool,
    tag_mode: TagMode,
    /// The remotes' list, the tags' modes or Push's options, where it is.
    menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    _commits_task: Option<Task<()>>,
    _files_task: Option<Task<()>>,
}

impl EventEmitter<PushDialogEvent> for PushDialog {}

impl Focusable for PushDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.editing {
            Some(field) => field.focus_handle(cx),
            None => self.focus.clone(),
        }
    }
}

/// The push target's default upstream marks: a new branch, a new upstream.
fn label_text(new: bool, upstream: bool) -> Option<&'static str> {
    match (new, upstream) {
        (true, true) => Some("New & Upstream"),
        (true, false) => Some("New"),
        (false, true) => Some("Upstream"),
        (false, false) => None,
    }
}

impl PushDialog {
    pub fn new(setup: PushSetup, cx: &mut Context<Self>) -> Self {
        let mut dialog = Self {
            focus: cx.focus_handle(),
            repository: setup.repository,
            source: setup.source,
            remotes: setup.remotes,
            target: setup.target,
            upstream: setup.upstream,
            data: setup.data,
            commits: Commits::Loading,
            files: Files::None,
            selected: 0,
            editing: None,
            set_upstream: false,
            push_tags: false,
            tag_mode: TagMode::All,
            menu: None,
            _commits_task: None,
            _files_task: None,
        };
        dialog.load_commits(cx);
        dialog
    }

    pub fn title(&self) -> String {
        format!("Push Commits to {}", self.repository)
    }

    pub fn target(&self) -> Option<&PushTarget> {
        self.target.as_ref()
    }

    /// The target is not the source's upstream: Set upstream is offered.
    fn upstream_offered(&self) -> bool {
        match (&self.target, &self.upstream) {
            (Some(t), Some((r, b))) => !(t.remote == *r && t.branch == *b),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Whether the push sets the upstream (`shouldSetUpstream`): a new
    /// branch without an upstream, or Set upstream checked.
    fn sets_upstream(&self) -> bool {
        let Some(t) = &self.target else {
            return false;
        };
        (t.new && self.upstream.is_none()) || (self.upstream_offered() && self.set_upstream)
    }

    fn commits(&self) -> &[OutgoingCommit] {
        match &self.commits {
            Commits::Loaded(c) => c,
            _ => &[],
        }
    }

    /// Something to push: commits, a new branch, or tags.
    pub fn can_push(&self) -> bool {
        let Some(t) = &self.target else {
            return false;
        };
        if self.editing.is_some() || matches!(self.commits, Commits::Loading) {
            return false;
        }
        !self.commits().is_empty() || t.new || self.push_tags
    }

    /// Force push is prohibited for the target (`isForcePushAllowed`).
    fn force_prohibited(&self) -> Option<String> {
        let t = self.target.as_ref()?;
        PROTECTED_BRANCHES
            .contains(&t.branch.as_str())
            .then(|| format!("Force push to {} is prohibited", t.branch))
    }

    fn spec(&self, force: ForceMode) -> Option<PushSpec> {
        Some(PushSpec {
            source: self.source.clone(),
            target: self.target.clone()?,
            set_upstream: self.sets_upstream(),
            force,
            tags: self.push_tags.then_some(self.tag_mode),
        })
    }

    fn load_commits(&mut self, cx: &mut Context<Self>) {
        self.commits = Commits::Loading;
        self.files = Files::None;
        let Some(target) = self.target.clone() else {
            self.commits = Commits::Loaded(Vec::new());
            return;
        };
        let data = self.data.clone();
        let source = self.source.clone();
        let work = cx.background_spawn(async move { data.outgoing(&source, &target) });
        self._commits_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.commits = match result {
                    Ok(c) => Commits::Loaded(c),
                    Err(e) => Commits::Failed(e.message),
                };
                this.selected = 0;
                this.load_files(cx);
                cx.notify();
            });
        }));
    }

    /// The commits of the selection: all for the repository row.
    fn selected_commits(&self) -> Vec<String> {
        let commits = self.commits();
        match self.selected {
            0 => commits.iter().map(|c| c.oid.clone()).collect(),
            i => commits
                .get(i - 1)
                .map(|c| vec![c.oid.clone()])
                .unwrap_or_default(),
        }
    }

    fn load_files(&mut self, cx: &mut Context<Self>) {
        let commits = self.selected_commits();
        if commits.is_empty() {
            self.files = Files::None;
            self._files_task = None;
            return;
        }
        self.files = Files::Loading;
        let data = self.data.clone();
        let work = cx.background_spawn(async move { data.files(&commits) });
        self._files_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.files = Files::Loaded(result.unwrap_or_default());
                cx.notify();
            });
        }));
    }

    fn rows(&self) -> usize {
        1 + self.commits().len()
    }

    fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let row = row.min(self.rows() - 1);
        if row != self.selected {
            self.selected = row;
            self.load_files(cx);
        }
        cx.notify();
    }

    /// A new target: its commits are read again.
    fn change_target(&mut self, remote: String, branch: String, cx: &mut Context<Self>) {
        let data = self.data.clone();
        let work = cx.background_spawn(async move { data.target(&remote, &branch) });
        self._commits_task = Some(cx.spawn(async move |this, cx| {
            let target = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(target) = target {
                    this.target = Some(target);
                    this.load_commits(cx);
                }
                cx.notify();
            });
        }));
        self.commits = Commits::Loading;
        cx.notify();
    }

    // ---- actions --------------------------------------------------------

    fn push(&mut self, _: &Push, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.apply_target_edit(cx);
            window.focus(&self.focus);
            return;
        }
        if self.can_push()
            && let Some(spec) = self.spec(ForceMode::None)
        {
            cx.emit(PushDialogEvent::Push(spec));
        }
    }

    fn force_push(&mut self, _: &ForcePush, _: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        if self.can_push()
            && self.force_prohibited().is_none()
            && let Some(spec) = self.spec(ForceMode::WithLease)
        {
            cx.emit(PushDialogEvent::Push(spec));
        }
    }

    fn cancel(&mut self, _: &CancelPush, _: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.close_menu(cx);
            return;
        }
        if self.editing.is_some() {
            self.stop_editing(cx);
            return;
        }
        cx.emit(PushDialogEvent::Cancelled);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.select_row(self.selected + 1, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.select_row(self.selected.saturating_sub(1), cx);
    }

    fn select(&mut self, action: &Select, _: &mut Window, cx: &mut Context<Self>) {
        self.select_row(action.row, cx);
    }

    fn edit_target(&mut self, _: &EditTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.start_editing(window, cx);
    }

    fn start_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = &self.target else {
            return;
        };
        let text = target.branch.clone();
        let field = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.set_single_line(true);
            e.set_ui_font(true);
            e.set_transparent(true);
            e.set_config(EditorConfig {
                brackets: Vec::new(),
                ..EditorConfig::default()
            });
            e.set_text_selected(&text, cx);
            e
        });
        window.focus(&field.focus_handle(cx));
        self.editing = Some(field);
        cx.notify();
    }

    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    /// The typed target branch is kept.
    fn apply_target_edit(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.editing.take() else {
            return;
        };
        let name = clean_up(&field.read(cx).text());
        let Some(remote) = self.target.as_ref().map(|t| t.remote.clone()) else {
            return;
        };
        if !name.is_empty() && self.target.as_ref().is_some_and(|t| t.branch != name) {
            self.change_target(remote, name, cx);
        }
        cx.notify();
    }

    fn confirm_target(&mut self, _: &ConfirmTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_target_edit(cx);
        window.focus(&self.focus);
    }

    fn cancel_target(&mut self, _: &CancelTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_editing(cx);
        window.focus(&self.focus);
    }

    fn set_target(&mut self, action: &SetTarget, _: &mut Window, cx: &mut Context<Self>) {
        let remote = action
            .remote
            .clone()
            .or_else(|| self.target.as_ref().map(|t| t.remote.clone()));
        let Some(remote) = remote.filter(|r| self.remotes.contains(r)) else {
            return;
        };
        self.editing = None;
        self.change_target(remote, clean_up(&action.branch), cx);
    }

    fn toggle_tags(&mut self, _: &ToggleTags, _: &mut Window, cx: &mut Context<Self>) {
        self.push_tags = !self.push_tags;
        cx.notify();
    }

    fn set_tag_mode(&mut self, action: &SetTagMode, _: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        if let Some(mode) = TagMode::from_id(&action.mode) {
            self.tag_mode = mode;
        }
        cx.notify();
    }

    fn toggle_upstream(&mut self, _: &ToggleUpstream, _: &mut Window, cx: &mut Context<Self>) {
        if self.upstream_offered() {
            self.set_upstream = !self.set_upstream;
            cx.notify();
        }
    }

    // ---- menus -------------------------------------------------------------

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    fn open_menu(
        &mut self,
        items: Vec<MenuItem>,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = ContextMenu::popup(items, PopupOptions::default(), window, cx);
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                this.menu = None;
                window.focus(&this.focus);
                cx.notify();
            });
        self.menu = Some((menu, at, subscription));
        cx.notify();
    }

    fn open_remotes(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.target.as_ref().map(|t| t.branch.clone()) else {
            return;
        };
        let mut items: Vec<MenuItem> = self
            .remotes
            .iter()
            .map(|r| {
                MenuItem::action(
                    r.clone(),
                    SetTarget {
                        branch: branch.clone(),
                        remote: Some(r.clone()),
                    },
                )
            })
            .collect();
        items.push(MenuItem::unavailable("Define Remote"));
        self.open_menu(items, at, window, cx);
    }

    fn open_tag_modes(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let items = [TagMode::All, TagMode::CurrentBranch]
            .into_iter()
            .map(|m| {
                MenuItem::action(
                    m.title(),
                    SetTagMode {
                        mode: m.id().to_owned(),
                    },
                )
            })
            .collect();
        self.open_menu(items, at, window, cx);
    }

    fn open_push_options(
        &mut self,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prohibited = self.force_prohibited();
        let mut force = MenuItem::action("Force Push", ForcePush).enabled(prohibited.is_none());
        if let Some(text) = prohibited {
            force = force.description(text);
        }
        self.open_menu(vec![force], at, window, cx);
    }

    // ---- state ---------------------------------------------------------------

    /// The repository row as text (`main → origin : main`).
    fn repository_row_text(&self) -> String {
        match &self.target {
            Some(t) => format!("{} \u{2192} {} : {}", self.source, t.remote, t.branch),
            None => format!("{} \u{2192} Define remote", self.source),
        }
    }

    /// The state for `vcs.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        let mut rows = vec![json!({
            "kind": "repository",
            "text": self.repository_row_text(),
            "label": self.target.as_ref().and_then(|t| label_text(t.new, self.upstream_offered() && self.sets_upstream() && !(t.new && self.upstream.is_none()))),
        })];
        rows.extend(self.commits().iter().map(|c| {
            json!({
                "kind": "commit",
                "text": c.subject,
                "oid": c.oid,
                "author": c.author,
            })
        }));
        let (files, files_text) = match &self.files {
            Files::None => (Vec::new(), Some("No commits selected")),
            Files::Loading => (Vec::new(), None),
            Files::Loaded(f) if f.is_empty() => (Vec::new(), Some("No differences")),
            Files::Loaded(f) => (
                f.iter()
                    .map(|f| json!({ "path": f.path, "status": f.status.id(), "old_path": f.old_path }))
                    .collect(),
                None,
            ),
        };
        json!({
            "title": self.title(),
            "source": self.source,
            "target": self.target,
            "remotes": self.remotes,
            "loading": matches!(self.commits, Commits::Loading),
            "error": match &self.commits { Commits::Failed(e) => Some(e.clone()), _ => None },
            "rows": rows,
            "selected": self.selected,
            "files": files,
            "files_text": files_text,
            "editing": self.editing.as_ref().map(|f| f.read(cx).text()),
            "set_upstream": self.upstream_offered().then_some(self.set_upstream),
            "sets_upstream": self.sets_upstream(),
            "push_tags": self.push_tags,
            "tag_mode": self.tag_mode.id(),
            "can_push": self.can_push(),
            "force_push": self.force_prohibited().map_or(json!(true), |d| json!(d)),
            "menu": self.menu.as_ref().map(|(m, _, _)| m.read(cx).state_json()),
        })
    }

    // ---- drawing ---------------------------------------------------------------

    fn render_label(&self, text: &'static str, cx: &App) -> AnyElement {
        let dark = cx.theme().is_dark();
        let pick = |c: (u32, u32)| gpui::rgb(if dark { c.1 } else { c.0 });
        div()
            .flex_none()
            .px(px(3.0))
            .rounded(px(4.0))
            .bg(pick(LABEL_BG))
            .text_color(pick(LABEL_FG))
            .text_size(px(11.0))
            .child(text)
            .into_any_element()
    }

    fn render_repository_row(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.selected == 0;
        let selection = theme.color(UiRole::Selection);
        let mut row = div()
            .id("push-row-0")
            .debug_selector(|| "push-row-0".into())
            .flex()
            .flex_row()
            .items_center()
            .h(px(ROW))
            .px(px(ROW_X))
            .gap(px(0.0))
            .whitespace_nowrap()
            .when(selected, |d| d.bg(selection))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.select_row(0, cx)))
            .child(format!("{} \u{2192} ", self.source));
        let Some(target) = self.target.clone() else {
            return row
                .child(
                    div()
                        .text_color(theme.color(UiRole::TextLink))
                        .child("Define remote"),
                )
                .into_any_element();
        };
        row = row.child(
            ui::link::link("push-remote", target.remote.clone(), cx).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let at = e.position + point(px(0.0), px(14.0));
                    this.open_remotes(at, window, cx);
                }),
            ),
        );
        row = row.child(" : ");
        match &self.editing {
            Some(field) => {
                let focused = field.focus_handle(cx).is_focused(window);
                row = row
                    .child(
                        input_frame(focused, false, cx)
                            .w(px(180.0))
                            .h(px(FIELD_HEIGHT))
                            .px(px(6.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .child(field.clone()),
                            ),
                    )
                    .when(self.upstream_offered(), |d| {
                        d.child(
                            div()
                                .id("push-set-upstream")
                                .debug_selector(|| "push-set-upstream".into())
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .ml(px(10.0))
                                .child(checkbox("push-set-upstream-box", self.set_upstream, cx))
                                .child("Set upstream")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.toggle_upstream(&ToggleUpstream, window, cx)
                                })),
                        )
                    });
            }
            None => {
                row = row.child(
                    ui::link::link("push-target", target.branch.clone(), cx).on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.start_editing(window, cx)
                        },
                    )),
                );
                let new_upstream = self.upstream_offered()
                    && self.set_upstream
                    && !(target.new && self.upstream.is_none());
                if let Some(text) = label_text(target.new, new_upstream) {
                    row = row
                        .child(div().w(px(6.0)))
                        .child(self.render_label(text, cx));
                }
            }
        }
        row.into_any_element()
    }

    fn render_log(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let danger = theme.color(UiRole::Danger);
        let selection = theme.color(UiRole::Selection);
        let mut rows: Vec<AnyElement> = vec![self.render_repository_row(window, cx)];
        match &self.commits {
            Commits::Loading => rows.push(
                div()
                    .h(px(ROW))
                    .pl(px(COMMIT_X))
                    .text_color(muted)
                    .child("Loading\u{2026}")
                    .into_any_element(),
            ),
            Commits::Failed(e) => rows.push(
                div()
                    .min_h(px(ROW))
                    .pl(px(COMMIT_X))
                    .text_color(danger)
                    .child(SharedString::from(e.clone()))
                    .into_any_element(),
            ),
            Commits::Loaded(commits) => {
                for (i, c) in commits.iter().enumerate() {
                    let row = i + 1;
                    let tip = format!("{}  by {}\n\n{}", c.short_oid(), c.author, c.subject);
                    rows.push(
                        div()
                            .id(("push-row", row))
                            .debug_selector(move || format!("push-row-{row}"))
                            .flex()
                            .items_center()
                            .h(px(ROW))
                            .pl(px(COMMIT_X))
                            .pr(px(ROW_X))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .when(self.selected == row, |d| d.bg(selection))
                            .child(SharedString::from(c.subject.clone()))
                            .tooltip(Tooltip::text(tip, None))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.select_row(row, cx)
                            }))
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .id("push-log")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .pt(px(4.0))
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    }

    fn render_files(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let text = theme.color(UiRole::Text);
        let body: Vec<AnyElement> = match &self.files {
            Files::None | Files::Loaded(_) if self.files_message().is_some() => vec![
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child(self.files_message().unwrap_or_default())
                    .into_any_element(),
            ],
            Files::Loaded(files) => files
                .iter()
                .map(|f| {
                    let (dir, name) = match f.path.rsplit_once('/') {
                        Some((d, n)) => (Some(d.to_owned()), n.to_owned()),
                        None => (None, f.path.clone()),
                    };
                    let color = status_role(f.status).map_or(text, |r| theme.color(r));
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .h(px(ROW))
                        .px(px(ROW_X))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(div().text_color(color).child(name))
                        .children(dir.map(|d| div().text_color(muted).child(d)))
                        .into_any_element()
                })
                .collect(),
            _ => Vec::new(),
        };
        div()
            .id("push-files")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .pt(px(4.0))
            .overflow_y_scroll()
            .children(body)
            .into_any_element()
    }

    fn files_message(&self) -> Option<&'static str> {
        match &self.files {
            Files::None => Some("No commits selected"),
            Files::Loaded(f) if f.is_empty() => Some("No differences"),
            _ => None,
        }
    }

    fn render_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let disabled = theme.color(UiRole::TextDisabled);
        let tags = div()
            .id("push-tags")
            .debug_selector(|| "push-tags".into())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(checkbox("push-tags-box", self.push_tags, cx))
            .child("Push tags:")
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_tags(&ToggleTags, window, cx)
            }));
        let combo = ui::form::combo(
            "push-tag-mode",
            vec![div().child(self.tag_mode.title()).into_any_element()],
            cx,
        )
        .debug_selector(|| "push-tag-mode".into())
        .w(px(150.0))
        .when(!self.push_tags, |d| d.text_color(disabled).opacity(0.6))
        .when(self.push_tags, |d| {
            d.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let at = e.position + point(px(-60.0), px(14.0));
                    this.open_tag_modes(at, window, cx);
                }),
            )
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(tags)
            .child(combo)
            .into_any_element()
    }

    /// Push with the arrow of its options (`JBOptionButton`), the default
    /// button.
    fn render_push_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let enabled = self.can_push();
        let accent = theme.color(UiRole::Accent);
        let on_accent = theme.color(UiRole::OnAccent);
        let radius = ui::form::field_radius(cx);
        div()
            .id("push-dialog-push")
            .debug_selector(|| "push-dialog-push".into())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(ui::form::BUTTON_HEIGHT))
            .rounded(radius)
            .bg(accent)
            .text_color(on_accent)
            .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
            .child(
                div()
                    .id("push-dialog-push-main")
                    .flex()
                    .items_center()
                    .h_full()
                    .px(px(14.0))
                    .child("Push")
                    .when(enabled, |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.push(&Push, window, cx)
                        }))
                    }),
            )
            .child(
                div()
                    .w(px(1.0))
                    .h(px(ui::form::BUTTON_HEIGHT - 10.0))
                    .bg(gpui::Hsla {
                        a: 0.4,
                        ..on_accent
                    }),
            )
            .child(
                div()
                    .id("push-dialog-push-more")
                    .debug_selector(|| "push-dialog-push-more".into())
                    .flex()
                    .items_center()
                    .h_full()
                    .px(px(7.0))
                    .child(Icon::new(IconName::CHEVRON_DOWN).tint(on_accent))
                    .when(enabled, |d| {
                        d.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                let at = e.position + point(px(-20.0), px(16.0));
                                this.open_push_options(at, window, cx);
                            }),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for PushDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let surface = theme.color(UiRole::SurfaceRaised);
        let popup_border = theme.color(UiRole::PopupBorder);
        let text_color = theme.color(UiRole::Text);
        let family = theme.ui_family();
        let text_size = theme.ui_size();
        let title = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(TITLE_HEIGHT))
            .font_weight(FontWeight::SEMIBOLD)
            .child(self.title());
        let log = div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(LOG_HEIGHT))
            .border_t_1()
            .border_b_1()
            .border_color(border)
            .child(self.render_log(window, cx))
            .child(div().flex_none().w(px(1.0)).h_full().bg(border))
            .child(self.render_files(cx));
        let bar = BottomBar::new()
            .help(ui::form::help_button("push-dialog-help", None, cx))
            .status(self.render_options(cx))
            .button(BarButton::new(
                "push-dialog-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(CancelPush),
            ))
            .element(self.render_push_button(cx), 100, 0.0)
            .render(cx);
        let menu = self
            .menu
            .as_ref()
            .map(|(menu, at, _)| ui::menu::overlay_at(menu, *at, Corner::TopLeft, window));
        let dialog = div()
            .id("push-dialog")
            .debug_selector(|| "push-dialog".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::push))
            .on_action(cx.listener(Self::force_push))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::select))
            .on_action(cx.listener(Self::edit_target))
            .on_action(cx.listener(Self::confirm_target))
            .on_action(cx.listener(Self::cancel_target))
            .on_action(cx.listener(Self::set_target))
            .on_action(cx.listener(Self::toggle_tags))
            .on_action(cx.listener(Self::set_tag_mode))
            .on_action(cx.listener(Self::toggle_upstream))
            .occlude()
            .flex()
            .flex_col()
            .w(px(WIDTH))
            .rounded(px(10.0))
            .border_1()
            .border_color(popup_border)
            .bg(surface)
            .shadow(popup_shadow())
            .font_family(family)
            .text_size(text_size)
            .text_color(text_color)
            .overflow_hidden()
            .child(title)
            .child(log)
            .child(bar);
        div()
            .id("push-dialog-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .w(size.width)
            .h(size.height)
            .flex()
            .items_center()
            .justify_center()
            .child(dialog)
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_of_the_target() {
        assert_eq!(label_text(true, false), Some("New"));
        assert_eq!(label_text(true, true), Some("New & Upstream"));
        assert_eq!(label_text(false, true), Some("Upstream"));
        assert_eq!(label_text(false, false), None);
    }
}
