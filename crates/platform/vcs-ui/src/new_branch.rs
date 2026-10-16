//! The New Branch dialog and its relatives (`GitNewBranchDialog`,
//! `GitRefDialog`): "Create New Branch" (or "Create Branch from {0}")
//! with *Branch Name:*, *Checkout branch* and *Overwrite existing branch*
//! (only with a branch of the name), Cancel and Create; "Rename Branch
//! {0}" with the name and Rename; "Checkout" asking for a reference
//! (Checkout Tag or Revision…).
//!
//! Characters git refuses are dropped while typing and spaces become `-`;
//! a name that clashes with a remote branch, a folder of local branches or
//! (without Overwrite) a local branch is reported under the field as it is
//! typed, an empty name and `HEAD` when Create is pressed.

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use text::{Buffer, Selection, SelectionSet};
use themes::UiRole;
use ui::components::{checkbox, input_frame, popup_shadow};
use ui::form::{BarButton, BottomBar, ButtonRole};
use ui::theme::ThemeExt;
use vcs::Branch;
use vcs::branch_name::{NameError, check_new, clean_up, clean_up_on_typing};

use editor::{Editor, EditorConfig, EditorEvent};

ui::ui_commands! {
    COMMANDS, scope = "BranchDialog";
    /// Create, Rename or OK.
    ConfirmDialog => "branch-dialog.confirm", "Create",
        "Chooses the default button of the New Branch, Rename Branch or Checkout dialog.";
    /// Cancel.
    CancelDialog => "branch-dialog.cancel", "Cancel",
        "Closes the New Branch, Rename Branch or Checkout dialog.";
    /// Checkout branch.
    ToggleCheckout => "branch-dialog.toggle-checkout", "Checkout Branch",
        "Toggles Checkout branch of the New Branch dialog.";
    /// Overwrite existing branch.
    ToggleOverwrite => "branch-dialog.toggle-overwrite", "Overwrite Existing Branch",
        "Toggles Overwrite existing branch of the New Branch dialog (when a branch of the name exists).";
    /// Type a name.
    SetName { text: String } => "branch-dialog.set-name", "Set Name",
        "Replaces the text of the dialog's field (cleaned as typing would).",
        r#"{"text": "feat/x"}"#;
}

/// Key context of the dialog.
pub const KEY_CONTEXT: &str = "BranchDialog";
const WIDTH: f32 = 440.0;
const LABEL_WIDTH: f32 = 96.0;
const FIELD_HEIGHT: f32 = 30.0;

/// What the dialog is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogKind {
    /// New Branch… (from `HEAD`), or New Branch from '…'….
    Create { from: Option<String> },
    /// Rename… of a local branch.
    Rename { from: String },
    /// Checkout Tag or Revision….
    Checkout,
}

impl DialogKind {
    pub fn title(&self) -> String {
        match self {
            Self::Create { from: None } => "Create New Branch".to_owned(),
            Self::Create { from: Some(from) } => format!("Create Branch from {from}"),
            Self::Rename { from } => format!("Rename Branch {from}"),
            Self::Checkout => "Checkout".to_owned(),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Checkout => "Enter reference (branch, tag) name or commit hash:",
            _ => "Branch Name:",
        }
    }

    fn ok(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create",
            Self::Rename { .. } => "Rename",
            Self::Checkout => "OK",
        }
    }

    fn is_name(&self) -> bool {
        !matches!(self, Self::Checkout)
    }
}

/// How the dialog was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchDialogEvent {
    Confirmed {
        /// The name (cleaned) or the reference.
        name: String,
        checkout: bool,
        overwrite: bool,
    },
    Cancelled,
}

pub struct BranchDialog {
    kind: DialogKind,
    field: Entity<Editor>,
    branches: Vec<Branch>,
    checkout: bool,
    overwrite: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<BranchDialogEvent> for BranchDialog {}

impl Focusable for BranchDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl BranchDialog {
    /// A dialog of `kind` with `initial` in the field (selected), checked
    /// against the repository's `branches`.
    pub fn new(
        kind: DialogKind,
        initial: &str,
        branches: Vec<Branch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.set_single_line(true);
            e.set_ui_font(true);
            e.set_transparent(true);
            e.set_config(EditorConfig {
                brackets: Vec::new(),
                ..EditorConfig::default()
            });
            e.set_text_selected(initial, cx);
            e
        });
        let subscription = cx.subscribe(&field, |this: &mut Self, _, event, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                this.edited(cx);
            }
        });
        Self {
            kind,
            field,
            branches,
            checkout: true,
            overwrite: false,
            error: None,
            _subscriptions: vec![subscription],
        }
    }

    pub fn kind(&self) -> &DialogKind {
        &self.kind
    }

    pub fn field(&self) -> &Entity<Editor> {
        &self.field
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// A local branch has the name in the field.
    fn name_exists(&self, cx: &App) -> bool {
        let name = clean_up(&self.field.read(cx).text());
        self.branches
            .iter()
            .any(|b| b.kind == vcs::BranchKind::Local && b.name == name)
    }

    /// The text changed: cleaned as typed, checked again.
    fn edited(&mut self, cx: &mut Context<Self>) {
        if !self.kind.is_name() {
            self.error = None;
            cx.notify();
            return;
        }
        let text = self.field.read(cx).text();
        let cleaned = clean_up_on_typing(&text);
        if cleaned != text {
            let caret = self
                .field
                .read(cx)
                .selections()
                .primary()
                .head
                .min(cleaned.chars().count());
            self.field.update(cx, |e, cx| {
                e.set_text_selected(&cleaned, cx);
                e.set_selections(SelectionSet::single(Selection::caret(caret)), cx);
            });
        }
        if !self.name_exists(cx) {
            self.overwrite = false;
        }
        self.error = self.check(false, cx).err().map(|e| self.message(&e));
        cx.notify();
    }

    fn check(&self, complete: bool, cx: &App) -> Result<String, NameError> {
        let name = clean_up(&self.field.read(cx).text());
        check_new(&name, &self.branches, self.overwrite, complete)?;
        Ok(name)
    }

    /// The text of an error; with Overwrite offered, a clash with a local
    /// branch says how to go on.
    fn message(&self, e: &NameError) -> String {
        match e {
            NameError::Exists(_) if self.offers_overwrite() => {
                format!("{e}.\nChange the name or overwrite existing branch")
            }
            _ => e.to_string(),
        }
    }

    fn offers_overwrite(&self) -> bool {
        matches!(self.kind, DialogKind::Create { .. })
    }

    fn confirm(&mut self, _: &ConfirmDialog, _: &mut Window, cx: &mut Context<Self>) {
        if !self.kind.is_name() {
            let name = self.field.read(cx).text().trim().to_owned();
            if !name.is_empty() {
                cx.emit(BranchDialogEvent::Confirmed {
                    name,
                    checkout: true,
                    overwrite: false,
                });
            }
            return;
        }
        match self.check(true, cx) {
            Ok(name) => cx.emit(BranchDialogEvent::Confirmed {
                name,
                checkout: self.checkout || !self.offers_overwrite(),
                overwrite: self.overwrite,
            }),
            Err(e) => {
                self.error = Some(self.message(&e));
                cx.notify();
            }
        }
    }

    fn cancel(&mut self, _: &CancelDialog, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(BranchDialogEvent::Cancelled);
    }

    fn toggle_checkout(&mut self, _: &ToggleCheckout, _: &mut Window, cx: &mut Context<Self>) {
        self.checkout = !self.checkout;
        cx.notify();
    }

    fn toggle_overwrite(&mut self, _: &ToggleOverwrite, _: &mut Window, cx: &mut Context<Self>) {
        if self.name_exists(cx) {
            self.overwrite = !self.overwrite;
            self.error = self.check(false, cx).err().map(|e| self.message(&e));
            cx.notify();
        }
    }

    fn set_name(&mut self, action: &SetName, _: &mut Window, cx: &mut Context<Self>) {
        self.field
            .update(cx, |e, cx| e.set_text_selected(&action.text, cx));
        self.edited(cx);
    }

    /// The state for `workbench.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "title": self.kind.title(),
            "label": self.kind.label(),
            "text": self.field.read(cx).text(),
            "checkout": self.offers_overwrite().then_some(self.checkout),
            "overwrite": (self.offers_overwrite() && self.name_exists(cx)).then_some(self.overwrite),
            "error": self.error,
            "ok": self.kind.ok(),
        })
    }
}

impl Render for BranchDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let line = editor::field_line_height(window, cx);
        let theme = cx.theme();
        let danger = theme.color(UiRole::Danger);
        let focused = self.field.focus_handle(cx).is_focused(window);
        let error = self.error.is_some();
        let label = self.kind.label();
        let wide_label = !self.kind.is_name();
        let field = input_frame(focused, error, cx)
            .flex_1()
            .h(px(FIELD_HEIGHT))
            .px(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(line)
                    .overflow_hidden()
                    .child(self.field.clone()),
            );
        let field_row = if wide_label {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(label)
                .child(field)
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(div().w(px(LABEL_WIDTH)).flex_none().child(label))
                .child(field)
        };
        let exists = self.offers_overwrite() && self.name_exists(cx);
        let option = |id: &'static str, label: &'static str, on: bool, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .when(!enabled, |d| d.opacity(0.5))
                .child(checkbox((id, 0usize), on, cx))
                .child(label)
        };
        let options = self.offers_overwrite().then(|| {
            div()
                .flex()
                .flex_row()
                .gap(px(16.0))
                .pl(px(LABEL_WIDTH + 8.0))
                .child(
                    option(
                        "branch-dialog-checkout",
                        "Checkout branch",
                        self.checkout,
                        true,
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.toggle_checkout(&ToggleCheckout, window, cx)
                        },
                    )),
                )
                .child(
                    option(
                        "branch-dialog-overwrite",
                        "Overwrite existing branch",
                        self.overwrite,
                        exists,
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.toggle_overwrite(&ToggleOverwrite, window, cx)
                        },
                    )),
                )
        });
        let message = self.error.clone().map(|m| {
            div()
                .debug_selector(|| "branch-dialog-error".into())
                .pl(px(if wide_label { 0.0 } else { LABEL_WIDTH + 8.0 }))
                .text_color(danger)
                .children(
                    m.lines()
                        .map(|l| div().child(SharedString::from(l.to_owned())))
                        .collect::<Vec<_>>(),
                )
        });
        let buttons = BottomBar::new()
            .button(BarButton::new(
                "branch-dialog-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(CancelDialog),
            ))
            .button(BarButton::new(
                "branch-dialog-ok",
                self.kind.ok(),
                ButtonRole::Ok,
                Box::new(ConfirmDialog),
            ))
            .render(cx);
        let dialog = div()
            .id("branch-dialog")
            .debug_selector(|| "branch-dialog".into())
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::toggle_checkout))
            .on_action(cx.listener(Self::toggle_overwrite))
            .on_action(cx.listener(Self::set_name))
            .occlude()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .w(px(WIDTH))
            .p(px(20.0))
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
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.kind.title()),
            )
            .child(field_row)
            .children(message)
            .children(options)
            .child(buttons);
        div()
            .id("branch-dialog-backdrop")
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
    }
}
