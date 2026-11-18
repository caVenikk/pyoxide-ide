//! A password field (`JBPasswordField`): what is typed shows as dots, is
//! kept apart from any editor buffer (no undo history, no clipboard copy),
//! never reaches a state dump or a log, and is wiped from memory when the
//! field forgets it. Until something is typed the field shows its
//! placeholder (`<hidden>` when a password is kept).

use std::ops::Range;

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
    /// UTF-16 range currently composed by the platform input method.
    marked: Option<Range<usize>>,
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
            marked: None,
            edited: false,
            all_selected: false,
            placeholder: SharedString::default(),
            focus: cx.focus_handle().tab_stop(true),
            id,
        }
    }

    /// Empties the field (nothing typed) with `placeholder`.
    pub fn reset(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.text.zeroize();
        self.text.clear();
        self.marked = None;
        self.edited = false;
        self.all_selected = false;
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Sets the text as typed (a command of the dialog).
    pub fn set(&mut self, value: &str, cx: &mut Context<Self>) {
        self.text.zeroize();
        self.text = value.to_owned();
        self.all_selected = false;
        self.marked = None;
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
            // Printable input (including dead keys and IME) arrives through
            // the platform text handler, just as it does in the editor.
            _ => false,
        };
        if changed {
            self.marked = None;
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

    fn selection(&self) -> Range<usize> {
        let end = self.text.encode_utf16().count();
        if self.all_selected { 0..end } else { end..end }
    }

    fn replace_input(
        &mut self,
        range: Option<Range<usize>>,
        input: &str,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let range = range
            .or_else(|| self.marked.take())
            .unwrap_or_else(|| self.selection());
        let byte_offset = |offset: usize| {
            let mut units = 0;
            for (byte, ch) in self.text.char_indices() {
                if units >= offset {
                    return byte;
                }
                units += ch.len_utf16();
            }
            self.text.len()
        };
        let start = byte_offset(range.start);
        let end = byte_offset(range.end).max(start);
        let start16 = self.text[..start].encode_utf16().count();
        let line = input.lines().next().unwrap_or_default();
        self.text.replace_range(start..end, line);
        self.all_selected = false;
        self.edited = true;
        self.marked = None;
        cx.emit(PasswordEdited);
        cx.notify();
        start16..start16 + line.encode_utf16().count()
    }
}

impl gpui::EntityInputHandler for PasswordField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let end = self.text.encode_utf16().count();
        let range = range.start.min(end)..range.end.min(end);
        let masked = "•".repeat(range.len());
        *actual = Some(range);
        Some(masked)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection {
            range: self.selection(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone()
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_input(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let marked = self.replace_input(range, text, cx);
        self.marked = (!marked.is_empty()).then_some(marked);
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<gpui::Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
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
        let field = cx.entity();
        let focus = self.focus.clone();
        input_frame(focused, false, cx)
            .id(id)
            .debug_selector(move || id.to_owned())
            .track_focus(&self.focus)
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, field),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler as _, TestAppContext};

    #[gpui::test]
    fn platform_input_composes_and_returns_only_masked_text(cx: &mut TestAppContext) {
        cx.update(|cx| ui::init(cx, themes::ThemeKind::Dark));
        let (field, cx) = cx.add_window_view(|window, cx| {
            let field = PasswordField::new("password", cx);
            window.focus(&field.focus);
            field
        });
        cx.update(|window, cx| window.draw(cx).clear());
        cx.simulate_input("a");
        field.update_in(cx, |field, window, cx| {
            assert_eq!(field.text, "a");
            field.replace_and_mark_text_in_range(None, "e", None, window, cx);
            field.replace_text_in_range(None, "é", window, cx);
            field.replace_text_in_range(None, "🔑", window, cx);
            field.replace_text_in_range(Some(1..2), "x", window, cx);
            assert_eq!(field.text, "ax🔑");
            assert_eq!(field.marked_text_range(window, cx), None);
            let mut actual = None;
            let queried = field.text_for_range(0..10, &mut actual, window, cx);
            assert_eq!(queried.as_deref(), Some("••••"));
            assert_eq!(actual, Some(0..4));
            assert!(!field.state_json().to_string().contains(&field.text));
        });
    }
}
