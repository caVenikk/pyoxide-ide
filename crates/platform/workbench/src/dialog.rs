//! A modal message dialog of the main window, as the reference IDE's
//! message dialogs (`Messages`, drawn by `AlertMessagesManager`'s
//! `AlertDialog` on macOS): a window of its own without a title bar,
//! centred on the window and a quarter of its height from its top, with an
//! icon, a title, a message, an optional "Don't ask again" check box and
//! buttons, the default one highlighted. Enter chooses the default button,
//! Escape cancels; the window behind takes no input while the dialog is
//! open ([`ui::dialog_window`]).
//!
//! The owner keeps the [`Dialog`] entity while it is open, focuses it and
//! draws it in its window as before: drawn there, it opens its window
//! (`ui::dialog_window::open_popup`) and leaves nothing in the owner's.
//! When a button closes it the window closes first, then
//! [`DialogEvent::Closed`] reaches the owner, which gives the focus back
//! where it was; a dialog dropped while open closes its window too.

use gpui::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, IntoElement, Render, SharedString, Subscription, WeakEntity, Window,
    div, point, prelude::*, px, size,
};
use themes::{Metric, UiRole};
use ui::components::checkbox;
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

/// Size of the dialog's icon and its distance from the top of the texts,
/// as the reference's 32 × 32 `warningDialog` draws (measured).
const ICON: f32 = 30.0;
const ICON_TOP: f32 = 3.0;
/// Width of a message dialog (`AlertDialog.calculateInitialSize`): 370,
/// or 440 for a message longer than 130 characters or buttons wider than
/// 278.
const WIDTH: f32 = 370.0;
const WIDE: f32 = 440.0;
const LONG_MESSAGE: usize = 130;
const WIDE_BUTTONS: f32 = 278.0;
/// Insets of the dialog (`AlertDialog.createContentPaneBorder`: 20).
const PADDING: f32 = 20.0;
/// Between the icon and the texts (`BorderLayout(12, 0)`).
const ICON_GAP: f32 = 12.0;
/// Between the title and the message (`BorderLayout(0, 8)`).
const TEXT_GAP: f32 = 8.0;
/// Above the check box: 12 from the message to the check box's bounds,
/// whose insets draw nothing.
const OPTION_GAP: f32 = 16.0;
/// Above the buttons: 22 from the message, 22 from the check box's bounds.
const BUTTONS_GAP: f32 = 22.0;
const BUTTONS_GAP_OPTION: f32 = 27.0;
/// Between the buttons, as they are drawn (12 between their bounds, whose
/// insets draw nothing).
const BUTTON_GAP: f32 = 18.0;
/// A button: at least 72 wide, its text 20 from its sides, 28 high.
const BUTTON_MIN: f32 = 72.0;
const BUTTON_PADDING: f32 = 20.0;
const BUTTON_HEIGHT: f32 = 28.0;
/// Line height of the title and the message.
const LINE: f32 = 17.0;
/// Radius of the dialog's corners.
const RADIUS: f32 = 10.0;
/// The window is first this high, then fitted to the dialog.
const MAX_HEIGHT: f32 = 600.0;

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

/// Where the dialog's window is.
#[derive(Clone, Copy, PartialEq)]
enum Shown {
    /// Not drawn by its owner yet.
    No,
    /// Being opened.
    Opening,
    Open(AnyWindowHandle),
    Closed,
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
    /// What the owner focuses: the dialog's place in the owner's window
    /// and the dialog in its own window.
    focus: FocusHandle,
    shown: Shown,
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
        // A dialog dropped while its window is open closes the window.
        cx.on_release(|this, cx| {
            if let Shown::Open(window) = this.shown {
                ui::dialog_window::close(window, cx);
            }
        })
        .detach();
        Self {
            title: title.into(),
            message: message.into(),
            option: None,
            default: buttons.len().saturating_sub(1),
            buttons,
            icon: None,
            focus: cx.focus_handle(),
            shown: Shown::No,
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
            "focused": self.focused_button().and_then(|i| self.buttons.get(i)).map(|b| b.label.replace('&', "")),
            "option": self.option.as_ref().map(|(label, on)| serde_json::json!({
                "label": label.to_string(),
                "checked": on,
            })),
        })
    }

    /// The dialog's width: wider for a long message or wide buttons.
    fn width(&self) -> f32 {
        // The buttons as drawn, with an average character width of the UI
        // font.
        let buttons: f32 = self
            .buttons
            .iter()
            .map(|b| {
                let text = b.label.replace('&', "").chars().count() as f32 * 7.5;
                (text + 2.0 * BUTTON_PADDING).max(BUTTON_MIN)
            })
            .sum::<f32>()
            + BUTTON_GAP * self.buttons.len().saturating_sub(1) as f32;
        if self.message.chars().count() <= LONG_MESSAGE && buttons <= WIDE_BUTTONS {
            WIDTH
        } else {
            WIDE
        }
    }

    /// The button with the focus ring (`AlertDialog.getPreferredFocusedComponent`):
    /// on macOS the first button from the right that is not the default
    /// one (with three or more buttons not Cancel either); elsewhere none.
    fn focused_button(&self) -> Option<usize> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let n = self.buttons.len();
        (0..n).rev().find(|&i| {
            i != self.default && (n < 3 || self.buttons[i].label.replace('&', "") != "Cancel")
        })
    }

    fn close(&mut self, button: Option<&'static str>, cx: &mut Context<Self>) {
        if self.shown == Shown::Closed {
            return;
        }
        let checked = self.option.as_ref().is_some_and(|(_, on)| *on);
        let window = match self.shown {
            Shown::Open(window) => Some(window),
            _ => None,
        };
        self.shown = Shown::Closed;
        // The window closes after the handler that closes it (it may run in
        // that window), and gives the focus back to its parent; the owner
        // hears of it after that, so that the focus it gives back wins.
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Some(window) = window {
                ui::dialog_window::close(window, cx);
            }
            cx.defer(move |cx| {
                let _ = this.update(cx, |_, cx| cx.emit(DialogEvent::Closed { button, checked }));
            });
        });
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

    /// Opens the dialog's window over `parent`: centred, a quarter of the
    /// window's height from its top (`AlertDialog.initialLocation`).
    fn open_window(this: Entity<Self>, parent: &mut Window, cx: &mut App) {
        if this.read(cx).shown != Shown::Opening {
            return;
        }
        let width = this.read(cx).width();
        let viewport = parent.viewport_size();
        let at = point(
            ((viewport.width - px(width)) / 2.0).max(px(0.0)).floor(),
            (viewport.height * 0.25).floor(),
        );
        let focus = this.read(cx).focus.clone();
        let content = this.clone();
        let window = ui::dialog_window::open_popup(
            parent,
            at,
            size(px(width), px(MAX_HEIGHT)),
            !crate::env::background(),
            cx,
            move |window, cx| {
                window.focus(&focus);
                cx.new(|cx| MessageWindow::new(&content, cx)).into()
            },
        );
        this.update(cx, |dialog, cx| {
            dialog.shown = match window {
                Some(window) => Shown::Open(window.into()),
                None => Shown::Closed,
            };
            cx.notify();
        });
    }

    /// The dialog itself, in its window.
    fn render_dialog(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let accent = theme.color(UiRole::Accent);
        let on_accent = theme.color(UiRole::OnAccent);
        let border = theme.color(UiRole::ButtonBorder);
        let default_border = theme.color(UiRole::ButtonDefaultBorder);
        let background = theme.color(UiRole::ButtonBackground);
        let focused_border = theme.color(UiRole::ButtonFocusedBorder);
        let default_focused_border = theme.color(UiRole::ButtonDefaultFocusedBorder);
        let hover = theme.color(UiRole::Hover);
        let ring = theme.color(UiRole::Focus);
        let ring_width = theme.metric(Metric::FocusWidth);
        let radius = theme.metric(Metric::CornerRadius);
        let focused = self.focused_button();
        let buttons_gap = if self.option.is_some() {
            BUTTONS_GAP_OPTION
        } else {
            BUTTONS_GAP
        };
        let buttons = self.buttons.iter().enumerate().map(|(i, b)| {
            let default = i == self.default;
            let id = b.id;
            div()
                .id(SharedString::from(format!("dialog-button-{id}")))
                .debug_selector(move || format!("dialog-button-{id}"))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(BUTTON_HEIGHT))
                .min_w(px(BUTTON_MIN))
                .px(px(BUTTON_PADDING))
                .rounded(radius)
                .whitespace_nowrap()
                .border_1()
                .map(|d| {
                    let focused = focused == Some(i);
                    if default {
                        d.bg(accent).text_color(on_accent).border_color(if focused {
                            default_focused_border
                        } else {
                            default_border
                        })
                    } else {
                        d.bg(background)
                            .border_color(if focused { focused_border } else { border })
                            .hover(move |s| s.bg(hover))
                    }
                })
                .child(SharedString::from(b.label.replace('&', "")))
                // The focused button's ring, outside it.
                .when(focused == Some(i), |d| {
                    // Over the button's border and half outside it.
                    let outside = ring_width / 2.0;
                    d.child(
                        div()
                            .absolute()
                            .top(-outside)
                            .left(-outside)
                            .right(-outside)
                            .bottom(-outside)
                            .rounded(radius + outside)
                            .border(ring_width)
                            .border_color(ring),
                    )
                })
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
                .mt(px(OPTION_GAP - TEXT_GAP))
                .child(checkbox("dialog-option-box", on, cx))
                // A long label wraps, the box centred beside it (Swing's
                // check box with an HTML text).
                .child(div().flex_1().min_w(px(0.0)).child(label))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_option(&ToggleOption, window, cx)
                }))
        });
        div()
            .id("dialog")
            .debug_selector(|| "dialog".into())
            .key_context("Dialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::choose))
            .on_action(cx.listener(Self::toggle_option))
            // Space presses the button with the focus ring.
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                let m = &e.keystroke.modifiers;
                if e.keystroke.key != "space" || m.control || m.alt || m.platform || m.shift {
                    return;
                }
                if let Some(button) = this.focused_button().and_then(|i| this.buttons.get(i)) {
                    let id = button.id;
                    cx.stop_propagation();
                    this.close(Some(id), cx);
                }
            }))
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .flex()
            .flex_row()
            .gap(px(ICON_GAP))
            .w(px(self.width()))
            .p(px(PADDING))
            .rounded(px(RADIUS))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .children(self.icon.map(|icon| {
                div()
                    .flex_none()
                    .pt(px(ICON_TOP))
                    .child(Icon::new(icon).size(px(ICON)))
            }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(TEXT_GAP))
                    .line_height(px(LINE))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(self.title.clone()),
                    )
                    .children(
                        self.message
                            .split('\n')
                            .map(|line| div().child(SharedString::from(line.to_owned())))
                            .collect::<Vec<_>>(),
                    )
                    .children(option)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(BUTTON_GAP))
                            .mt(px(buttons_gap - TEXT_GAP))
                            .children(buttons),
                    ),
            )
    }
}

impl Focusable for Dialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Dialog {
    /// In its owner's window: nothing to see but the place of the focus
    /// (keys there go to the dialog's window); the first time, the dialog's
    /// window opens.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.shown == Shown::No {
            self.shown = Shown::Opening;
            let this = cx.entity();
            window.defer(cx, move |window, cx| Dialog::open_window(this, window, cx));
        }
        div()
            .id("dialog-host")
            .debug_selector(|| "dialog-host".into())
            .key_context("Dialog")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::choose))
            .on_action(cx.listener(Self::toggle_option))
            .absolute()
            .size_0()
    }
}

/// The dialog's window: draws the dialog while it is there.
struct MessageWindow {
    dialog: WeakEntity<Dialog>,
    _observe: Subscription,
}

impl MessageWindow {
    fn new(dialog: &Entity<Dialog>, cx: &mut Context<Self>) -> Self {
        Self {
            dialog: dialog.downgrade(),
            _observe: cx.observe(dialog, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for MessageWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().children(
            self.dialog
                .upgrade()
                .map(|dialog| dialog.update(cx, |d, cx| d.render_dialog(cx))),
        )
    }
}
