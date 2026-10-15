//! The Copy and Move dialogs of the project tool window
//! (`CopyFilesOrDirectoriesDialog`, `MoveFilesOrDirectoriesDialog`): the
//! title, what is copied or moved in bold ("Copy file /p/a.py", "Move
//! specified files"), *New name:* (Copy of one file or folder, its name
//! without the extension selected), *To directory:* (the folder's path;
//! for Move its last part is selected), the check box *Open copy in
//! editor* (on for a file) or *Open in editor* (off), then OK or Refactor
//! and Cancel. Enter confirms, Escape cancels, Tab moves between the
//! fields.

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, Pixels, Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use project::fs_ops::TransferKind;
use text::{Buffer, Selection, SelectionSet};
use themes::UiRole;
use ui::components::{checkbox, input_frame, popup_shadow};
use ui::theme::ThemeExt;

use editor::{Editor, EditorConfig, EditorEvent};

use super::name_popup::{
    Cancel, Confirm, DIALOG_WIDTH, FIELD_HEIGHT, NextField, PreviousField, ToggleOption,
    dialog_button,
};

/// Width of the labels left of the fields.
const LABEL_WIDTH: f32 = 96.0;

/// What the person did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDialogEvent {
    Confirmed {
        /// The new name (Copy of one file or folder).
        name: Option<String>,
        directory: String,
        open_in_editor: bool,
    },
    Cancelled,
}

pub struct TransferDialog {
    kind: TransferKind,
    label: SharedString,
    name: Option<Entity<Editor>>,
    directory: Entity<Editor>,
    open_in_editor: bool,
    message: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TransferDialogEvent> for TransferDialog {}

impl Focusable for TransferDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.name {
            Some(name) => name.focus_handle(cx),
            None => self.directory.focus_handle(cx),
        }
    }
}

fn field(text: &str, window: &mut Window, cx: &mut App) -> Entity<Editor> {
    cx.new(|cx| {
        let mut e = Editor::new(Buffer::new(text), window, cx);
        e.set_single_line(true);
        e.set_ui_font(true);
        e.set_transparent(true);
        e.set_config(EditorConfig {
            brackets: Vec::new(),
            ..EditorConfig::default()
        });
        e
    })
}

fn select(field: &Entity<Editor>, range: std::ops::Range<usize>, cx: &mut App) {
    field.update(cx, |e, cx| {
        e.set_selections(
            SelectionSet::single(Selection::new(range.start, range.end)),
            cx,
        )
    });
}

impl TransferDialog {
    /// A dialog for `kind`; `name` is the new name field's text (Copy of
    /// one item; for a file its name without the extension is selected).
    pub fn new(
        kind: TransferKind,
        label: impl Into<SharedString>,
        name: Option<(String, bool)>,
        directory: &str,
        open_in_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = name.map(|(text, is_file)| {
            let f = field(&text, window, cx);
            let chars = text.chars().count();
            let end = match text.rfind('.') {
                Some(dot) if is_file && dot > 0 => text[..dot].chars().count(),
                _ => chars,
            };
            select(&f, 0..end, cx);
            f
        });
        let dir = field(directory, window, cx);
        // Move selects the last folder of the path.
        let sep = directory.rfind(['/', '\\']);
        let chars = directory.chars().count();
        match sep {
            Some(i) if kind == TransferKind::Move && i + 1 < directory.len() => {
                select(&dir, directory[..i + 1].chars().count()..chars, cx)
            }
            _ => select(&dir, chars..chars, cx),
        }
        let mut subscriptions = Vec::new();
        for f in name.iter().chain([&dir]) {
            subscriptions.push(cx.subscribe(f, |this: &mut Self, _, event, cx| {
                if matches!(event, EditorEvent::Edited(_)) && this.message.take().is_some() {
                    cx.notify();
                }
            }));
        }
        Self {
            kind,
            label: label.into(),
            name,
            directory: dir,
            open_in_editor,
            message: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn kind(&self) -> TransferKind {
        self.kind
    }

    /// The field with the folder's path.
    pub fn directory_field(&self) -> &Entity<Editor> {
        &self.directory
    }

    /// The new name field (Copy of one item).
    pub fn name_field(&self) -> Option<&Entity<Editor>> {
        self.name.as_ref()
    }

    pub fn set_message(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
        self.message = message;
        cx.notify();
    }

    fn title(&self) -> &'static str {
        match self.kind {
            TransferKind::Copy => "Copy",
            TransferKind::Move => "Move",
        }
    }

    fn option_label(&self) -> &'static str {
        match self.kind {
            TransferKind::Copy => "Open copy in editor",
            TransferKind::Move => "Open in editor",
        }
    }

    /// The state reported by `workbench.state`.
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        serde_json::json!({
            "title": self.title(),
            "label": self.label.to_string(),
            "name": self.name.as_ref().map(|n| n.read(cx).text()),
            "directory": self.directory.read(cx).text(),
            "option": self.option_label(),
            "open_in_editor": self.open_in_editor,
            "message": self.message.as_ref().map(|m| m.to_string()),
        })
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TransferDialogEvent::Confirmed {
            name: self.name.as_ref().map(|n| n.read(cx).text()),
            directory: self.directory.read(cx).text(),
            open_in_editor: self.open_in_editor,
        });
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TransferDialogEvent::Cancelled);
    }

    fn fields(&self) -> Vec<Entity<Editor>> {
        self.name
            .iter()
            .cloned()
            .chain([self.directory.clone()])
            .collect()
    }

    fn move_focus(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let fields = self.fields();
        let at = fields
            .iter()
            .position(|f| f.focus_handle(cx).is_focused(window))
            .unwrap_or(0) as isize;
        let next = (at + step).rem_euclid(fields.len() as isize) as usize;
        window.focus(&fields[next].focus_handle(cx));
        cx.notify();
    }

    fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, window, cx);
    }

    fn previous_field(&mut self, _: &PreviousField, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(-1, window, cx);
    }

    fn toggle_option(&mut self, _: &ToggleOption, _: &mut Window, cx: &mut Context<Self>) {
        self.open_in_editor = !self.open_in_editor;
        cx.notify();
    }

    fn row(
        &self,
        label: &'static str,
        field: &Entity<Editor>,
        line: Pixels,
        window: &Window,
        cx: &App,
    ) -> gpui::Div {
        let focused = field.focus_handle(cx).is_focused(window);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(div().w(px(LABEL_WIDTH)).flex_none().child(label))
            .child(
                input_frame(focused, false, cx)
                    .flex_1()
                    .h(px(FIELD_HEIGHT - 4.0))
                    .px(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h(line)
                            .overflow_hidden()
                            .child(field.clone()),
                    ),
            )
    }
}

impl Render for TransferDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let line = editor::field_line_height(window, cx);
        let theme = cx.theme();
        let danger = theme.color(UiRole::Danger);
        let (ok_id, ok_label) = match self.kind {
            TransferKind::Copy => ("ok", "OK"),
            TransferKind::Move => ("refactor", "Refactor"),
        };
        let name_row = self
            .name
            .as_ref()
            .map(|n| self.row("New name:", n, line, window, cx));
        let dir_row = self.row("To directory:", &self.directory, line, window, cx);
        let option = div()
            .id("transfer-option")
            .debug_selector(|| "transfer-option".into())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(checkbox("transfer-option-box", self.open_in_editor, cx))
            .child(self.option_label())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_option(&ToggleOption, window, cx)
            }));
        let dialog = div()
            .id("transfer-dialog")
            .debug_selector(|| "transfer-dialog".into())
            .key_context("NameInput")
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::previous_field))
            .on_action(cx.listener(Self::toggle_option))
            .occlude()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .w(px(DIALOG_WIDTH + 80.0))
            .p(px(20.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(self.title()))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(self.label.clone()),
            )
            .children(name_row)
            .child(dir_row)
            .child(option)
            .children(self.message.clone().map(|m| {
                div()
                    .debug_selector(|| "transfer-message".into())
                    .text_color(danger)
                    .child(m)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .pt(px(6.0))
                    .child(dialog_button("cancel", "Cancel", false, true, cx).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.cancel(&Cancel, window, cx)
                        }),
                    ))
                    .child(
                        dialog_button(ok_id, ok_label, true, true, cx).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.confirm(&Confirm, window, cx),
                        )),
                    ),
            );
        div()
            .id("transfer-backdrop")
            .occlude()
            .w(size.width)
            .h(size.height)
            .flex()
            .items_center()
            .justify_center()
            .child(dialog)
    }
}
