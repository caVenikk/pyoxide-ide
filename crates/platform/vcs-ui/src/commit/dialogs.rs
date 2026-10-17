//! The dialogs of the Commit tool window, over the main window:
//!
//! - "Rollback Changes" (`RollbackChangesDialog`): the changes with check
//!   boxes (the ones chosen ticked), the legend, "Delete local copies of
//!   added files" (enabled when an added file is ticked), Rollback and
//!   Close.
//! - "Git User Name Is Not Defined" (`GitUserNameNotDefinedDialog`):
//!   Name, E-mail, "Set properties globally", Cancel and "Set and Commit".

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use editor::Editor;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, Render, SharedString, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::{checkbox, popup_shadow};
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;
use vcs::{Change, FileStatus};

use super::legend::Legend;
use super::tree::{origin_text, parent_text};
use crate::status_colors::status_role;

ui::ui_commands! {
    ROLLBACK_COMMANDS, scope = "RollbackDialog";
    /// Rollback.
    ConfirmRollback => "rollback-dialog.confirm", "Rollback",
        "Rolls back the ticked changes of the Rollback Changes dialog.";
    /// Close.
    CancelRollback => "rollback-dialog.cancel", "Close",
        "Closes the Rollback Changes dialog without a rollback.";
    /// Delete local copies of added files.
    ToggleDeleteAdded => "rollback-dialog.toggle-delete", "Delete Local Copies of Added Files",
        "Toggles \"Delete local copies of added files\" of the Rollback Changes dialog (when an added file is ticked).";
    /// A file's check box.
    ToggleRollbackFile { path: String } => "rollback-dialog.toggle", "Include File",
        "Ticks or unticks a change of the Rollback Changes dialog (relative to the repository root).",
        r#"{"path": "main.py"}"#;
}

ui::ui_commands! {
    USER_NAME_COMMANDS, scope = "UserNameDialog";
    /// Set and Commit.
    ConfirmUserName => "user-name-dialog.confirm", "Set and Commit",
        "Sets the user name and e-mail of the Git User Name Is Not Defined dialog and commits.";
    /// Cancel.
    CancelUserName => "user-name-dialog.cancel", "Cancel",
        "Closes the Git User Name Is Not Defined dialog; nothing is committed.";
    /// Tab.
    NextUserNameField => "user-name-dialog.next-field", "Next Field",
        "Moves the focus between Name and E-mail of the Git User Name Is Not Defined dialog.";
    /// Fill in the dialog.
    SetUserName { name: Option<String>, email: Option<String>, global: Option<bool> } => "user-name-dialog.set", "Set Name and E-mail",
        "Fills in Name, E-mail and Set properties globally of the Git User Name Is Not Defined dialog.",
        r#"{"name": "Name", "email": "name@example.com", "global": false}"#;
}

/// Key contexts.
pub const ROLLBACK_KEY_CONTEXT: &str = "RollbackDialog";
pub const USER_NAME_KEY_CONTEXT: &str = "UserNameDialog";

/// `changes.action.rollback.custom.title` with the operation's name.
pub const ROLLBACK_TITLE: &str = "Rollback Changes";
/// `changes.checkbox.delete.locally.added.files`.
pub const DELETE_ADDED: &str = "Delete local copies of added files";
/// `title.user.name.email.not.specified`.
pub const USER_NAME_TITLE: &str = "Git User Name Is Not Defined";
/// `validation.warning.set.name.email.for.git`,
/// `validation.error.email.no.at`.
pub const NAME_EMAIL_MISSING: &str = "You need to specify username and email for Git";
pub const EMAIL_NO_AT: &str = "The email address must contain the '@' symbol";

const ROLLBACK_WIDTH: f32 = 520.0;
const USER_NAME_WIDTH: f32 = 440.0;
const LIST_HEIGHT: f32 = 220.0;
const LABEL_WIDTH: f32 = 64.0;

/// The frame of a dialog over the main window: centred, with a backdrop
/// that takes the mouse.
fn frame(id: &'static str, dialog: impl IntoElement, window: &Window) -> AnyElement {
    let size = window.viewport_size();
    div()
        .id(id)
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
        .into_any_element()
}

/// The box of a dialog with its title.
fn panel(id: &'static str, title: &str, width: f32, cx: &App) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .occlude()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .w(px(width))
        .pt(px(16.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.color(UiRole::PopupBorder))
        .bg(theme.color(UiRole::SurfaceRaised))
        .shadow(popup_shadow())
        .font_family(theme.ui_family())
        .text_size(theme.ui_size())
        .text_color(theme.color(UiRole::Text))
        .child(
            div()
                .px(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(SharedString::from(title.to_owned())),
        )
}

// ---- Rollback Changes ----------------------------------------------------------

/// How the Rollback Changes dialog was closed.
#[derive(Debug, Clone, PartialEq)]
pub enum RollbackEvent {
    Confirmed {
        changes: Vec<Change>,
        delete_added: bool,
    },
    Cancelled,
}

pub struct RollbackDialog {
    focus: FocusHandle,
    root: PathBuf,
    changes: Vec<Change>,
    included: HashSet<PathBuf>,
    delete_added: bool,
    file_icon: Rc<dyn Fn(&Path) -> IconName>,
}

impl EventEmitter<RollbackEvent> for RollbackDialog {}

impl Focusable for RollbackDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RollbackDialog {
    /// The dialog for the repository's `changes`, `included` ticked;
    /// `delete_added` is the check box's last state.
    pub fn new(
        root: &Path,
        changes: Vec<Change>,
        included: &[PathBuf],
        delete_added: bool,
        file_icon: Rc<dyn Fn(&Path) -> IconName>,
        cx: &mut Context<Self>,
    ) -> Self {
        let included = included
            .iter()
            .filter(|p| changes.iter().any(|c| &c.path == *p))
            .cloned()
            .collect();
        Self {
            focus: cx.focus_handle(),
            root: root.to_owned(),
            changes,
            included,
            delete_added,
            file_icon,
        }
    }

    fn included_changes(&self) -> Vec<Change> {
        self.changes
            .iter()
            .filter(|c| self.included.contains(&c.path))
            .cloned()
            .collect()
    }

    /// An added file is ticked: the check box can be used.
    fn has_added(&self) -> bool {
        self.included_changes()
            .iter()
            .any(|c| c.status == FileStatus::Added)
    }

    pub fn delete_added(&self) -> bool {
        self.delete_added
    }

    fn confirm(&mut self, _: &ConfirmRollback, _: &mut Window, cx: &mut Context<Self>) {
        let changes = self.included_changes();
        if changes.is_empty() {
            return;
        }
        let delete_added = self.delete_added && self.has_added();
        cx.emit(RollbackEvent::Confirmed {
            changes,
            delete_added,
        });
    }

    fn cancel(&mut self, _: &CancelRollback, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(RollbackEvent::Cancelled);
    }

    fn toggle_delete(&mut self, _: &ToggleDeleteAdded, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_added() {
            self.delete_added = !self.delete_added;
            cx.notify();
        }
    }

    fn toggle_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.included.remove(path) && self.changes.iter().any(|c| c.path == path) {
            self.included.insert(path.to_owned());
        }
        cx.notify();
    }

    fn toggle_file(&mut self, action: &ToggleRollbackFile, _: &mut Window, cx: &mut Context<Self>) {
        let mut path = self.root.clone();
        for part in action.path.split('/').filter(|p| !p.is_empty()) {
            path.push(part);
        }
        self.toggle_path(&path, cx);
    }

    pub fn state_json(&self) -> Json {
        let rel = |p: &Path| {
            p.strip_prefix(&self.root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| p.display().to_string())
        };
        let legend = Legend::of(&self.included_changes(), 0);
        json!({
            "kind": "rollback",
            "title": ROLLBACK_TITLE,
            "files": self.changes.iter().map(|c| json!({
                "path": rel(&c.path),
                "status": c.status.id(),
                "included": self.included.contains(&c.path),
            })).collect::<Vec<_>>(),
            "legend": legend.text(false),
            "delete_added": self.delete_added,
            "delete_added_enabled": self.has_added(),
            "ok": "Rollback",
            "ok_enabled": !self.included.is_empty(),
        })
    }
}

impl Render for RollbackDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let rows: Vec<AnyElement> = self
            .changes
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let included = self.included.contains(&c.path);
                let name = c
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let color =
                    status_role(c.status).map_or(theme.color(UiRole::Text), |r| theme.color(r));
                let path = c.path.clone();
                div()
                    .id(("rollback-file", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(24.0))
                    .px(px(8.0))
                    .child(checkbox(("rollback-check", i), included, cx))
                    .child(Icon::new((self.file_icon)(&c.path)))
                    .child(div().text_color(color).child(name))
                    .children(origin_text(c, &self.root).map(|o| div().child(o)))
                    .child(
                        div()
                            .text_color(muted)
                            .child(parent_text(&c.path, &self.root)),
                    )
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_path(&path, cx)),
                    )
                    .into_any_element()
            })
            .collect();
        let legend = Legend::of(&self.included_changes(), 0);
        let legend_el =
            div()
                .flex()
                .flex_row()
                .children(
                    legend
                        .chunks(false)
                        .into_iter()
                        .enumerate()
                        .map(|(i, chunk)| {
                            let color = status_role(chunk.status)
                                .map_or(theme.color(UiRole::Text), |r| theme.color(r));
                            div().text_color(color).child(if i > 0 {
                                format!("{}{}", super::legend::SEPARATOR, chunk.text)
                            } else {
                                chunk.text
                            })
                        }),
                );
        let has_added = self.has_added();
        let delete = ui::form::check_row(
            "rollback-delete-added",
            "rollback-delete-added-check",
            self.delete_added,
            DELETE_ADDED,
            cx,
        )
        .debug_selector(|| "rollback-delete-added".into())
        .when(!has_added, |d| d.opacity(ui::components::DISABLED_ALPHA))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_delete(&ToggleDeleteAdded, window, cx)
        }));
        let buttons = BottomBar::new()
            .button(
                BarButton::new(
                    "rollback-dialog-ok",
                    "Rollback",
                    ButtonRole::Ok,
                    Box::new(ConfirmRollback),
                )
                .enabled(!self.included.is_empty()),
            )
            .button(BarButton::new(
                "rollback-dialog-cancel",
                "Close",
                ButtonRole::Cancel,
                Box::new(CancelRollback),
            ))
            .render(cx);
        let dialog = panel("rollback-dialog", ROLLBACK_TITLE, ROLLBACK_WIDTH, cx)
            .key_context(ROLLBACK_KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::toggle_delete))
            .on_action(cx.listener(Self::toggle_file))
            .child(
                div()
                    .id("rollback-files")
                    .mx(px(20.0))
                    .h(px(LIST_HEIGHT))
                    .overflow_y_scroll()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(theme.color(UiRole::Border))
                    .py(px(4.0))
                    .children(rows),
            )
            .child(div().px(px(20.0)).child(legend_el))
            .child(div().px(px(20.0)).child(delete))
            .child(buttons);
        frame("rollback-dialog-backdrop", dialog, window)
    }
}

// ---- Git User Name Is Not Defined ------------------------------------------------

/// How the user name dialog was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserNameEvent {
    Confirmed {
        name: String,
        email: String,
        global: bool,
    },
    Cancelled,
}

pub struct UserNameDialog {
    name: Entity<Editor>,
    email: Entity<Editor>,
    global: bool,
    error: Option<&'static str>,
    /// The commit's name ("Commit", "Amend Commit") for "Set and …".
    action: &'static str,
}

impl EventEmitter<UserNameEvent> for UserNameDialog {}

impl Focusable for UserNameDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.focus_handle(cx)
    }
}

impl UserNameDialog {
    /// The dialog with the proposed name and e-mail.
    pub fn new(
        name: &str,
        email: &str,
        global: bool,
        action: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = editor::field::new(name, None, true, window, cx);
        let email = editor::field::new(email, None, true, window, cx);
        Self {
            name,
            email,
            global,
            error: None,
            action,
        }
    }

    fn ok_label(&self) -> String {
        format!("Set and {}", self.action)
    }

    fn confirm(&mut self, _: &ConfirmUserName, _: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_owned();
        let email = self.email.read(cx).text().trim().to_owned();
        self.error = if name.is_empty() || email.is_empty() {
            Some(NAME_EMAIL_MISSING)
        } else if !email.contains('@') {
            Some(EMAIL_NO_AT)
        } else {
            None
        };
        if self.error.is_none() {
            cx.emit(UserNameEvent::Confirmed {
                name,
                email,
                global: self.global,
            });
        }
        cx.notify();
    }

    fn cancel(&mut self, _: &CancelUserName, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(UserNameEvent::Cancelled);
    }

    fn next_field(&mut self, _: &NextUserNameField, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.name.focus_handle(cx).is_focused(window) {
            &self.email
        } else {
            &self.name
        };
        window.focus(&target.focus_handle(cx));
    }

    fn set(&mut self, action: &SetUserName, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = &action.name {
            self.name.update(cx, |e, cx| e.set_text_selected(name, cx));
        }
        if let Some(email) = &action.email {
            self.email
                .update(cx, |e, cx| e.set_text_selected(email, cx));
        }
        if let Some(global) = action.global {
            self.global = global;
        }
        self.error = None;
        cx.notify();
    }

    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "kind": "user_name",
            "title": USER_NAME_TITLE,
            "name": self.name.read(cx).text(),
            "email": self.email.read(cx).text(),
            "global": self.global,
            "error": self.error,
            "ok": self.ok_label(),
        })
    }
}

impl Render for UserNameDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let field = |label: &'static str, editor: &Entity<Editor>, window: &Window, cx: &App| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(20.0))
                .child(div().w(px(LABEL_WIDTH)).flex_none().child(label))
                .child(editor::field::frame(editor, true, false, window, cx).flex_1())
        };
        let global = ui::form::check_row(
            "user-name-global",
            "user-name-global-check",
            self.global,
            "Set properties globally",
            cx,
        )
        .debug_selector(|| "user-name-global".into())
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.global = !this.global;
            cx.notify();
        }));
        let error = self.error.map(|e| {
            div()
                .debug_selector(|| "user-name-error".into())
                .px(px(20.0))
                .text_color(theme.color(UiRole::Danger))
                .child(e)
        });
        let buttons = BottomBar::new()
            .button(BarButton::new(
                "user-name-dialog-ok",
                self.ok_label(),
                ButtonRole::Ok,
                Box::new(ConfirmUserName),
            ))
            .button(BarButton::new(
                "user-name-dialog-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(CancelUserName),
            ))
            .render(cx);
        let dialog = panel("user-name-dialog", USER_NAME_TITLE, USER_NAME_WIDTH, cx)
            .key_context(USER_NAME_KEY_CONTEXT)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::set))
            .child(field("Name:", &self.name, window, cx))
            .child(field("E-mail:", &self.email, window, cx))
            .child(div().px(px(20.0 + LABEL_WIDTH + 8.0)).child(global))
            .children(error)
            .child(buttons);
        frame("user-name-dialog-backdrop", dialog, window)
    }
}
