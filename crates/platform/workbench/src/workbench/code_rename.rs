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

/// The open dialog and where the focus goes when it closes.
pub(super) struct CodeRename {
    popup: Entity<NamePopup>,
    name: String,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
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
            popup,
            name,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
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
                open.popup.update(cx, |p, cx| {
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
        if let Some(open) = &self.code_rename {
            open.popup.update(cx, |p, cx| {
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

    pub(super) fn render_code_rename(&self) -> Option<AnyElement> {
        self.code_rename
            .as_ref()
            .map(|open| name_popup::overlay(&open.popup))
    }

    /// The dialog's state for `workbench.state`.
    pub(super) fn code_rename_json(&self, cx: &gpui::App) -> serde_json::Value {
        self.code_rename
            .as_ref()
            .map(|open| open.popup.read(cx).state_json(cx))
            .unwrap_or_default()
    }
}
