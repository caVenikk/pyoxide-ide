//! A password field (`JBPasswordField`): what is typed shows as dots, is
//! kept apart from any editor buffer (no undo history, no clipboard copy),
//! never reaches a state dump or a log, and is wiped from memory when the
//! field forgets it. Until something is typed the field shows its
//! placeholder (`<hidden>` when a password is kept).

use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, SharedString, Window, div, prelude::*, px,
};
use themes::UiRole;
use ui::components::input_frame;
use ui::form::FIELD_HEIGHT;
use ui::theme::ThemeExt;
use zeroize::Zeroize;

use db_core::Secret;

/// The field was edited.
pub struct PasswordEdited;

pub struct PasswordField {
    text: String,
    /// Something was typed since the field was filled or cleared.
    edited: bool,
    /// Select All was pressed: the next key replaces the text.
    all_selected: bool,
    placeholder: SharedString,
    focus: FocusHandle,
    id: &'static str,
}

impl EventEmitter<PasswordEdited> for PasswordField {}

impl Focusable for PasswordField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for PasswordField {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

impl std::fmt::Debug for PasswordField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordField")
            .field("text", &"<redacted>")
            .field("edited", &self.edited)
            .finish()
    }
}

impl PasswordField {
    pub fn new(id: &'static str, cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            edited: false,
            all_selected: false,
            placeholder: SharedString::default(),
            focus: cx.focus_handle(),
            id,
        }
    }

    /// Empties the field (nothing typed) with `placeholder`.
    pub fn reset(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.text.zeroize();
        self.text.clear();
        self.edited = false;
        self.all_selected = false;
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Sets the text as typed (a command of the dialog).
    pub fn set(&mut self, value: &str, cx: &mut Context<Self>) {
        self.text.zeroize();
        self.text = value.to_owned();
        self.edited = true;
        cx.emit(PasswordEdited);
        cx.notify();
    }

    /// The typed password; `None` while nothing was typed.
    pub fn secret(&self) -> Option<Secret> {
        self.edited.then(|| Secret::new(self.text.clone()))
    }

    /// The field's state without its value, for state dumps.
    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "edited": self.edited,
            "empty": self.text.is_empty(),
            "placeholder": self.placeholder.as_ref(),
        })
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        let platform = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        let changed = match keystroke.key.as_str() {
            "backspace" => {
                if self.all_selected || platform || m.alt {
                    self.text.zeroize();
                    self.text.clear();
                } else {
                    self.text.pop();
                }
                true
            }
            "a" if platform && !m.shift => {
                self.all_selected = !self.text.is_empty();
                cx.stop_propagation();
                cx.notify();
                return;
            }
            "v" if platform && !m.shift => {
                if let Some(pasted) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let line = pasted.lines().next().unwrap_or_default().to_owned();
                    self.insert(&line);
                }
                true
            }
            // Copy and cut never take a password out.
            "c" | "x" if platform => {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
                cx.stop_propagation();
                return;
            }
            _ => match &keystroke.key_char {
                Some(text) if !platform && !m.control && !text.chars().any(char::is_control) => {
                    let text = text.clone();
                    self.insert(&text);
                    true
                }
                _ => false,
            },
        };
        if changed {
            self.edited = true;
            self.all_selected = false;
            cx.stop_propagation();
            cx.emit(PasswordEdited);
            cx.notify();
        }
    }

    fn insert(&mut self, text: &str) {
        if self.all_selected {
            self.text.zeroize();
            self.text.clear();
        }
        self.text.push_str(text);
    }
}

impl Render for PasswordField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focused = self.focus.is_focused(window);
        let shown = self.edited && !self.text.is_empty();
        let dots: String = "\u{2022}".repeat(self.text.chars().count());
        let caret = div().w(px(1.5)).h(px(16.0)).bg(theme.color(UiRole::Caret));
        let id = self.id;
        input_frame(focused, false, cx)
            .id(id)
            .debug_selector(move || id.to_owned())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_click(cx.listener(|this, _, window, _| window.focus(&this.focus)))
            .h(px(FIELD_HEIGHT))
            .pl(px(8.0))
            .pr(px(7.0))
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .map(|d| {
                if shown {
                    d.child(
                        div()
                            .when(self.all_selected, |d| {
                                d.bg(theme.color(UiRole::EditorSelection))
                            })
                            .child(dots),
                    )
                } else if focused {
                    d
                } else {
                    d.text_color(theme.color(UiRole::TextMuted))
                        .child(self.placeholder.clone())
                }
            })
            .when(focused, |d| d.child(caret))
    }
}
