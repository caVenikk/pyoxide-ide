//! The Rename dialog of the code intelligence (Shift+F6 in an editor): the
//! reference's `RenameDialog` as the project tool window's Rename shows it
//! ([`NamePopup`]), over the symbol's name with "Rename 'a' and its usages
//! to:". Refactor asks the services for the edits and applies them; a name
//! they refuse shows below the field.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _, Subscription, Window,
};

use super::Workbench;
use crate::project_panel::name_popup::{self, Message, NamePopup, NamePopupEvent};
use editor::{Editor, EditorEvent};
use gpui::{anchored, div, prelude::*, px};
use std::ops::Range;
use ui::ThemeExt as _;

/// The open dialog and where the focus goes when it closes.
pub(super) struct CodeRename {
    popup: Option<Entity<NamePopup>>,
    inline: Option<InlineRename>,
    name: String,
    return_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

struct InlineRename {
    source: Entity<Editor>,
    field: Entity<Editor>,
    active: Range<usize>,
    error: Option<String>,
}

impl Workbench {
    /// Opens the dialog for the symbol `name` of the active editor.
    pub(super) fn open_rename_dialog(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_rename_dialog(window, cx);
        let label = format!("Rename '{name}' and its usages to:");
        let chars = name.chars().count();
        let popup = cx.new(|cx| NamePopup::rename(label, &name, 0..chars, window, cx));
        let return_focus = window.focused(cx);
        let subscription = cx.subscribe_in(&popup, window, |this, _, event, window, cx| {
            this.rename_dialog_event(event, window, cx)
        });
        window.focus(&popup.focus_handle(cx));
        popup.update(cx, |p, cx| p.set_can_confirm(false, cx));
        self.code_rename = Some(CodeRename {
            popup: Some(popup),
            inline: None,
            name,
            return_focus,
            _subscriptions: vec![subscription],
        });
        cx.notify();
    }

    pub(super) fn open_inline_rename(
        &mut self,
        name: &str,
        active: Range<usize>,
        occurrences: Vec<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_rename_dialog(window, cx);
        let Some(source) = self.active_editor().cloned() else {
            return;
        };
        let return_focus = window.focused(cx);
        let field = editor::field::new(name, None, false, window, cx);
        field.update(cx, |e, cx| e.set_text_selected(name, cx));
        let focus = field.focus_handle(cx);
        let preview = source.clone();
        let occurrences: std::sync::Arc<[Range<usize>]> = occurrences.into();
        let changed = cx.subscribe_in(&field, window, move |this, field, event, window, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                if let Some(inline) = this.code_rename.as_mut().and_then(|r| r.inline.as_mut()) {
                    inline.error = None;
                }
                let text = field.read(cx).text();
                preview.update(cx, |e, cx| {
                    e.set_linked_preview(Some((occurrences.clone(), text)), cx)
                });
                // The source editor lays out the preview in this frame. Re-anchor
                // the input once its updated glyph positions are available.
                let workbench = cx.weak_entity();
                window.on_next_frame(move |_, cx| {
                    let _ = workbench.update(cx, |_, cx| cx.notify());
                });
                cx.notify();
            }
        });
        let blur = cx.on_blur(&focus, window, |this, window, cx| {
            this.cancel_inline(window, cx)
        });
        let deactivated = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.cancel_inline(window, cx);
            }
        });
        window.focus(&focus);
        self.code_rename = Some(CodeRename {
            popup: None,
            inline: Some(InlineRename {
                source,
                field,
                active,
                error: None,
            }),
            name: name.into(),
            return_focus,
            _subscriptions: vec![changed, blur, deactivated],
        });
        cx.notify();
    }

    fn cancel_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .code_rename
            .as_ref()
            .is_some_and(|r| r.inline.is_some())
        {
            self.close_rename_dialog(window, cx);
            if let Some(view) = &self.code_intel {
                view.update(cx, |v, cx| v.rename_cancelled(cx));
            }
        }
    }

    fn rename_dialog_event(
        &mut self,
        event: &NamePopupEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = &self.code_rename else {
            return;
        };
        let Some(popup) = &open.popup else { return };
        match event {
            NamePopupEvent::Cancelled => {
                self.close_rename_dialog(window, cx);
                if let Some(view) = self.code_intel.clone() {
                    view.update(cx, |v, cx| v.rename_cancelled(cx));
                }
            }
            NamePopupEvent::Changed(text) => {
                // Refactor needs a name that is not the old one.
                let text = text.trim();
                let can = !text.is_empty() && text != open.name;
                popup.update(cx, |p, cx| {
                    p.set_can_confirm(can, cx);
                    p.set_message(None, cx);
                });
            }
            NamePopupEvent::Confirmed(text) => {
                if let Some(view) = self.code_intel.clone() {
                    let text = text.clone();
                    view.update(cx, |v, cx| v.rename_confirmed(text, cx));
                }
            }
        }
    }

    /// The services refused the name.
    pub(super) fn rename_dialog_rejected(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(inline) = self.code_rename.as_mut().and_then(|r| r.inline.as_mut()) {
            inline.error = Some(message);
            cx.notify();
            return;
        }
        if let Some(open) = &self.code_rename
            && let Some(popup) = &open.popup
        {
            popup.update(cx, |p, cx| {
                p.set_message(
                    Some(Message {
                        text: message.into(),
                        warning: false,
                    }),
                    cx,
                )
            });
        }
    }

    /// Closes the dialog and gives the focus back to where it was.
    pub(super) fn close_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.code_rename.take() {
            if let Some(inline) = &open.inline {
                inline
                    .source
                    .update(cx, |e, cx| e.set_linked_preview(None, cx));
                if !inline.field.focus_handle(cx).is_focused(window) {
                    cx.notify();
                    return;
                }
            }
            match &open.return_focus {
                Some(focus) => window.focus(focus),
                None => {
                    if let Some(editor) = self.active_editor().cloned() {
                        window.focus(&editor.focus_handle(cx));
                    }
                }
            }
            cx.notify();
        }
    }

    pub(super) fn render_code_rename(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.code_rename.as_ref()?;
        if let Some(popup) = &open.popup {
            return Some(name_popup::overlay(popup));
        }
        let inline = open.inline.as_ref()?;
        let bounds = inline
            .source
            .read(cx)
            .bounds_for_range(inline.active.clone())?;
        let content = div()
            .id("inline-rename")
            .key_context("NameInput")
            .on_action(cx.listener(|this, _: &name_popup::Confirm, _, cx| {
                if let Some(r) = this.code_rename.as_ref().and_then(|r| r.inline.as_ref()) {
                    let text = r.field.read(cx).text();
                    if text.trim().is_empty() {
                        this.rename_dialog_rejected("The name is empty".into(), cx);
                    } else if let Some(view) = &this.code_intel {
                        view.update(cx, |v, cx| v.rename_confirmed(text, cx));
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &name_popup::Cancel, window, cx| {
                this.cancel_inline(window, cx)
            }))
            .occlude()
            .bg(cx.theme().color(themes::UiRole::EditorBackground))
            .w(bounds.size.width.max(px(2.0)) + px(2.0))
            .h(bounds.size.height)
            .child(inline.field.clone());
        Some(
            ui::deferred(
                anchored().position(bounds.origin).child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .child(content)
                        .children(inline.error.clone().map(|error| {
                            div()
                                .bg(cx.theme().color(themes::UiRole::EditorBackground))
                                .text_color(cx.theme().color(themes::UiRole::Danger))
                                .child(error)
                        })),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The dialog's state for `workbench.state`.
    pub(super) fn code_rename_json(&self, cx: &gpui::App) -> serde_json::Value {
        self.code_rename
            .as_ref()
            .and_then(|open| {
                if let Some(inline) = &open.inline {
                    Some(serde_json::json!({"kind":"inline", "text":inline.field.read(cx).text(), "error":inline.error}))
                } else {
                    open.popup.as_ref().map(|p| p.read(cx).state_json(cx))
                }
            })
            .unwrap_or_default()
    }
}
