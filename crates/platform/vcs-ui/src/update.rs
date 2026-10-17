//! The Update Project dialog (`UpdateOptionsDialog` with
//! `GitUpdateOptionsPanel`): "Update Project" with the update methods as
//! radio buttons (Merge incoming changes into the current branch, Rebase
//! the current branch on top of incoming changes), Don't show again at the
//! bottom left, Cancel and OK. With Don't show again the next Update
//! Project runs at once with the chosen method (Shift opens the dialog in
//! the reference; here `vcs.update-project` with `dialog: true`).

use gpui::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::{checkbox, popup_shadow};
use ui::form::{BarButton, BottomBar, ButtonRole, radio_row};
use ui::theme::ThemeExt;
use vcs::UpdateMethod;

ui::ui_commands! {
    COMMANDS, scope = "UpdateDialog";
    /// OK.
    ConfirmUpdate => "update-dialog.ok", "OK",
        "Updates the project with the method chosen in the Update Project dialog.";
    /// Cancel.
    CancelUpdate => "update-dialog.cancel", "Cancel", "Closes the Update Project dialog.";
    /// Choose the update method.
    SetMethod { method: String } => "update-dialog.set-method", "Update Method",
        "Chooses the update method of the Update Project dialog: merge or rebase.",
        r#"{"method": "rebase"}"#;
    /// Don't show again.
    ToggleDontShow => "update-dialog.toggle-dont-show", "Don't Show Again",
        "Toggles Don't show again of the Update Project dialog.";
}

/// Key context of the dialog.
pub const KEY_CONTEXT: &str = "UpdateDialog";
const WIDTH: f32 = 420.0;

/// How the dialog was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDialogEvent {
    Confirmed {
        method: UpdateMethod,
        dont_show: bool,
    },
    Cancelled,
}

pub struct UpdateDialog {
    focus: FocusHandle,
    method: UpdateMethod,
    dont_show: bool,
}

impl EventEmitter<UpdateDialogEvent> for UpdateDialog {}

impl Focusable for UpdateDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl UpdateDialog {
    pub fn new(method: UpdateMethod, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            method,
            dont_show: false,
        }
    }

    pub fn title(&self) -> &'static str {
        "Update Project"
    }

    fn confirm(&mut self, _: &ConfirmUpdate, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(UpdateDialogEvent::Confirmed {
            method: self.method,
            dont_show: self.dont_show,
        });
    }

    fn cancel(&mut self, _: &CancelUpdate, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(UpdateDialogEvent::Cancelled);
    }

    fn set_method(&mut self, action: &SetMethod, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(m) = UpdateMethod::from_id(&action.method) {
            self.method = m;
            cx.notify();
        }
    }

    fn toggle_dont_show(&mut self, _: &ToggleDontShow, _: &mut Window, cx: &mut Context<Self>) {
        self.dont_show = !self.dont_show;
        cx.notify();
    }

    /// The state for `vcs.state`.
    pub fn state_json(&self) -> Json {
        let methods: Vec<&str> = [UpdateMethod::Merge, UpdateMethod::Rebase]
            .iter()
            .map(|m| m.description())
            .collect();
        json!({
            "title": self.title(),
            "methods": methods,
            "method": self.method.id(),
            "dont_show": self.dont_show,
        })
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let theme = cx.theme();
        let methods = [UpdateMethod::Merge, UpdateMethod::Rebase]
            .into_iter()
            .map(|m| {
                let id = m.id();
                radio_row(
                    ("update-method", m as usize),
                    ("update-method-radio", m as usize),
                    self.method == m,
                    m.description(),
                    cx,
                )
                .debug_selector(move || format!("update-method-{id}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_method(
                        &SetMethod {
                            method: id.to_owned(),
                        },
                        window,
                        cx,
                    )
                }))
            })
            .collect::<Vec<_>>();
        let dont_show = div()
            .id("update-dont-show")
            .debug_selector(|| "update-dont-show".into())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(checkbox("update-dont-show-box", self.dont_show, cx))
            .child("Don't show again")
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_dont_show(&ToggleDontShow, window, cx)
            }));
        let bar = BottomBar::new()
            .help(ui::form::help_button("update-dialog-help", None, cx))
            .status(dont_show)
            .button(BarButton::new(
                "update-dialog-ok",
                "OK",
                ButtonRole::Ok,
                Box::new(ConfirmUpdate),
            ))
            .button(BarButton::new(
                "update-dialog-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(CancelUpdate),
            ))
            .render(cx);
        let dialog = div()
            .id("update-dialog")
            .debug_selector(|| "update-dialog".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::set_method))
            .on_action(cx.listener(Self::toggle_dont_show))
            .occlude()
            .flex()
            .flex_col()
            .w(px(WIDTH))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(40.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .px(px(20.0))
                    .pt(px(8.0))
                    .pb(px(18.0))
                    .children(methods),
            )
            .child(bar);
        div()
            .id("update-dialog-backdrop")
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
