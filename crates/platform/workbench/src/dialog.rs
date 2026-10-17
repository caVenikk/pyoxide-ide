//! A modal message dialog of the main window, as the reference IDE's
//! message dialogs (`Messages`): a title, a message, an optional "Do not
//! show this dialog in the future" check box and buttons, the default one
//! highlighted. Enter chooses the default button, Escape cancels; the rest
//! of the window ignores the mouse while the dialog is open.

use gpui::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, SharedString, Window, div, prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{checkbox, popup_shadow};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

ui::ui_commands! {
    COMMANDS, scope = "Dialog";
    /// Choose the default button.
    Confirm => "dialog.confirm", "Choose Default Button",
        "Chooses the open dialog's default button.";
    /// Close the dialog.
    Cancel => "dialog.cancel", "Cancel", "Closes the open dialog without a choice (its Cancel button).";
    /// Choose a button.
    Choose { button: String } => "dialog.choose", "Choose Button",
        "Chooses a button of the open dialog by its label without mnemonics.",
        r#"{"button": "Terminate"}"#;
    /// Toggle the check box.
    ToggleOption => "dialog.toggle-option", "Toggle Option",
        "Toggles the open dialog's check box.";
}

/// Size of the dialog's icon (`questionDialog`, `warningDialog`).
const ICON: f32 = 28.0;
/// Width of a message dialog.
const WIDTH: f32 = 420.0;

/// One button of a dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct DialogButton {
    /// What the caller gets back.
    pub id: &'static str,
    pub label: SharedString,
}

/// How a dialog was closed.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogEvent {
    /// A button was chosen (`None`: cancelled), with the check box's state.
    Closed {
        button: Option<&'static str>,
        checked: bool,
    },
}

/// A modal message dialog.
pub struct Dialog {
    title: SharedString,
    message: SharedString,
    /// The check box's label and state.
    option: Option<(SharedString, bool)>,
    /// Buttons from left to right; the last one is the default.
    buttons: Vec<DialogButton>,
    /// Index of the default button.
    default: usize,
    icon: Option<IconName>,
    focus: FocusHandle,
}

impl EventEmitter<DialogEvent> for Dialog {}

impl Dialog {
    /// A dialog with `buttons` from left to right; the rightmost is the
    /// default, as in the reference IDE's message dialogs.
    pub fn new(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        buttons: Vec<DialogButton>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            option: None,
            default: buttons.len().saturating_sub(1),
            buttons,
            icon: None,
            focus: cx.focus_handle(),
        }
    }

    /// The icon left of the title (question, warning).
    pub fn with_icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Makes the button `id` the default one instead of the rightmost.
    pub fn with_default(mut self, id: &str) -> Self {
        if let Some(i) = self.buttons.iter().position(|b| b.id == id) {
            self.default = i;
        }
        self
    }

    /// Adds a check box, off.
    pub fn with_option(mut self, label: impl Into<SharedString>) -> Self {
        self.option = Some((label.into(), false));
        self
    }

    /// The state reported by `workbench.state`.
    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "title": self.title.to_string(),
            "message": self.message.to_string(),
            "buttons": self.buttons.iter().map(|b| b.label.replace('&', "")).collect::<Vec<_>>(),
            "default": self.buttons.get(self.default).map(|b| b.label.replace('&', "")),
            "option": self.option.as_ref().map(|(label, on)| serde_json::json!({
                "label": label.to_string(),
                "checked": on,
            })),
        })
    }

    fn close(&mut self, button: Option<&'static str>, cx: &mut Context<Self>) {
        let checked = self.option.as_ref().is_some_and(|(_, on)| *on);
        cx.emit(DialogEvent::Closed { button, checked });
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let button = self.buttons.get(self.default).map(|b| b.id);
        self.close(button, cx);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.close(None, cx);
    }

    fn choose(&mut self, action: &Choose, _: &mut Window, cx: &mut Context<Self>) {
        let wanted = action.button.replace('&', "");
        if let Some(b) = self
            .buttons
            .iter()
            .find(|b| b.label.replace('&', "") == wanted)
        {
            let id = b.id;
            self.close(Some(id), cx);
        }
    }

    fn toggle_option(&mut self, _: &ToggleOption, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, on)) = &mut self.option {
            *on = !*on;
            cx.notify();
        }
    }
}

impl Focusable for Dialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = theme.color(UiRole::Accent);
        let on_accent = theme.color(UiRole::OnAccent);
        let border = theme.color(UiRole::BorderInput);
        let hover = theme.color(UiRole::Hover);
        let buttons = self.buttons.iter().enumerate().map(|(i, b)| {
            let default = i == self.default;
            let id = b.id;
            div()
                .id(SharedString::from(format!("dialog-button-{id}")))
                .debug_selector(move || format!("dialog-button-{id}"))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(28.0))
                .min_w(px(72.0))
                .px(px(14.0))
                .rounded(theme.metric(Metric::CornerRadius))
                .whitespace_nowrap()
                .map(|d| {
                    if default {
                        d.bg(accent).text_color(on_accent)
                    } else {
                        d.border_1()
                            .border_color(border)
                            .hover(move |s| s.bg(hover))
                    }
                })
                .child(SharedString::from(b.label.replace('&', "")))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.close(Some(id), cx)))
        });
        let option = self.option.clone().map(|(label, on)| {
            div()
                .id("dialog-option")
                .debug_selector(|| "dialog-option".into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(checkbox("dialog-option-box", on, cx))
                // A long label wraps, the box centred beside it (Swing's
                // check box with an HTML text).
                .child(div().flex_1().min_w(px(0.0)).child(label))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_option(&ToggleOption, window, cx)
                }))
        });
        let dialog = div()
            .id("dialog")
            .debug_selector(|| "dialog".into())
            .key_context("Dialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::choose))
            .on_action(cx.listener(Self::toggle_option))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .w(px(WIDTH))
            .p(px(20.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(16.0))
                    .children(
                        self.icon
                            .map(|icon| div().flex_none().child(Icon::new(icon).size(px(ICON)))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.title.clone()),
                            )
                            .children(
                                self.message
                                    .split('\n')
                                    .map(|line| div().child(SharedString::from(line.to_owned())))
                                    .collect::<Vec<_>>(),
                            )
                            .children(option.map(|o| o.mt(px(4.0)))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .pt(px(4.0))
                    .children(buttons),
            );
        // The window behind ignores the mouse while the dialog is open.
        div()
            .id("dialog-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .child(dialog)
    }
}
