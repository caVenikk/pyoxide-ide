//! The name field of the project tool window's file operations: the New
//! File and New Directory popups (the reference IDE's `NewItemPopupUtil`
//! with `NewItemSimplePopupPanel`: a title and a field with the
//! placeholder "Name", the error or warning in a balloon above the field,
//! centred in the window, closed by Escape or a click outside) and the
//! Rename dialog (`RenameDialog`: "Rename file 'a.py' to:", the name with
//! its extension left out of the selection, the error below the field,
//! Refactor and Cancel).

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, MouseButton, Pixels, Render, SharedString, Subscription, Window, anchored, div,
    point, prelude::*, px,
};
use text::{Buffer, Selection, SelectionSet};
use themes::{Metric, UiRole};
use ui::components::{input_frame, popup_shadow};
use ui::deferred;
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use editor::{Editor, EditorConfig, EditorEvent};

ui::ui_commands! {
    COMMANDS, scope = "NameInput";
    Confirm => "name-input.confirm", "Confirm",
        "Creates or renames with the name in the open New File, New Directory or Rename popup (Enter).";
    Cancel => "name-input.cancel", "Cancel",
        "Closes the open New File, New Directory or Rename popup without a change (Escape).";
    NextKind => "name-input.next-kind", "Next Kind",
        "Selects the next kind in the list of a New popup (Python file, Python unit test, ...).";
    PreviousKind => "name-input.previous-kind", "Previous Kind",
        "Selects the previous kind in the list of a New popup.";
    NextField => "name-input.next-field", "Next Field",
        "Moves to the next field of the open Copy or Move dialog (Tab).";
    PreviousField => "name-input.previous-field", "Previous Field",
        "Moves to the previous field of the open Copy or Move dialog (Shift+Tab).";
    ToggleOption => "name-input.toggle-option", "Toggle Option",
        "Toggles the check box of the open Copy or Move dialog (Open copy in editor, Open in editor).";
}

/// Width of the New File and New Directory popups (30 columns of the
/// reference's field).
const POPUP_WIDTH: f32 = 340.0;
/// Height of their field in the New UI (`NewItemSimplePopupPanel`).
pub(super) const FIELD_HEIGHT: f32 = 32.0;
/// Width of the Rename dialog.
pub(super) const DIALOG_WIDTH: f32 = 460.0;

/// Which popup it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamePopupKind {
    /// New File, New Directory: a light popup.
    NewItem,
    /// Rename: a modal dialog.
    Rename,
}

/// What the person did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamePopupEvent {
    /// The text changed.
    Changed(String),
    /// Enter, or the dialog's Refactor button.
    Confirmed(String),
    /// Escape, a click outside the popup, or Cancel.
    Cancelled,
}

/// An error or warning about the typed name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: SharedString,
    pub warning: bool,
}

pub struct NamePopup {
    kind: NamePopupKind,
    title: SharedString,
    /// The dialog's question ("Rename file 'a.py' to:").
    label: Option<SharedString>,
    field: Entity<Editor>,
    message: Option<Message>,
    /// The dialog's Refactor button can be used (a valid name).
    can_confirm: bool,
    /// The kinds listed under the field of a New popup, and the selected
    /// one.
    kinds: Vec<(SharedString, IconName)>,
    selected_kind: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NamePopupEvent> for NamePopup {}

impl Focusable for NamePopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl NamePopup {
    /// The New File or New Directory popup titled `title`, empty.
    pub fn new_item(
        title: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = Self::make_field("", Some("Name".into()), window, cx);
        Self::build(NamePopupKind::NewItem, title.into(), None, field, cx)
    }

    /// A New popup with a list of kinds under the field
    /// (`CreateFileFromTemplateDialog`), the first one selected; Up and
    /// Down change it.
    pub fn new_item_with_kinds(
        title: impl Into<SharedString>,
        kinds: Vec<(SharedString, IconName)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut popup = Self::new_item(title, window, cx);
        popup.kinds = kinds;
        popup
    }

    /// The selected kind of a New popup with a list.
    pub fn selected_kind(&self) -> usize {
        self.selected_kind
    }

    fn next_kind(&mut self, _: &NextKind, _: &mut Window, cx: &mut Context<Self>) {
        if !self.kinds.is_empty() {
            self.selected_kind = (self.selected_kind + 1) % self.kinds.len();
            cx.notify();
        }
    }

    fn previous_kind(&mut self, _: &PreviousKind, _: &mut Window, cx: &mut Context<Self>) {
        if !self.kinds.is_empty() {
            self.selected_kind = (self.selected_kind + self.kinds.len() - 1) % self.kinds.len();
            cx.notify();
        }
    }

    /// The Rename dialog with `name` in the field and `selected` (char
    /// offsets) selected.
    pub fn rename(
        label: impl Into<SharedString>,
        name: &str,
        selected: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = Self::make_field(name, None, window, cx);
        field.update(cx, |e, cx| {
            e.set_selections(
                SelectionSet::single(Selection::new(selected.start, selected.end)),
                cx,
            )
        });
        Self::build(
            NamePopupKind::Rename,
            "Rename".into(),
            Some(label.into()),
            field,
            cx,
        )
    }

    fn make_field(
        text: &str,
        placeholder: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(text), window, cx);
            e.set_single_line(true);
            e.set_ui_font(true);
            e.set_transparent(true);
            e.set_placeholder(placeholder);
            e.set_config(EditorConfig {
                brackets: Vec::new(),
                ..EditorConfig::default()
            });
            e
        })
    }

    fn build(
        kind: NamePopupKind,
        title: SharedString,
        label: Option<SharedString>,
        field: Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> Self {
        let edited = cx.subscribe(&field, |this, field, event, cx| {
            if matches!(event, EditorEvent::Edited(_)) {
                let text = field.read(cx).text();
                cx.emit(NamePopupEvent::Changed(text));
                this.edited(cx);
            }
        });
        Self {
            kind,
            title,
            label,
            field,
            message: None,
            can_confirm: true,
            kinds: Vec::new(),
            selected_kind: 0,
            _subscriptions: vec![edited],
        }
    }

    /// The New File popup forgets its error when the name changes
    /// (`NewItemSimplePopupPanel` without live validation).
    fn edited(&mut self, cx: &mut Context<Self>) {
        if self.message.as_ref().is_some_and(|m| !m.warning) && self.kind == NamePopupKind::NewItem
        {
            self.message = None;
        }
        cx.notify();
    }

    pub fn kind(&self) -> NamePopupKind {
        self.kind
    }

    pub fn text(&self, cx: &App) -> String {
        self.field.read(cx).text()
    }

    pub fn field(&self) -> &Entity<Editor> {
        &self.field
    }

    /// Shows an error (or a warning), or nothing.
    pub fn set_message(&mut self, message: Option<Message>, cx: &mut Context<Self>) {
        if self.message != message {
            self.message = message;
            cx.notify();
        }
    }

    pub fn set_can_confirm(&mut self, can: bool, cx: &mut Context<Self>) {
        if self.can_confirm != can {
            self.can_confirm = can;
            cx.notify();
        }
    }

    /// The state reported by `workbench.state`.
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        serde_json::json!({
            "kind": match self.kind {
                NamePopupKind::NewItem => "new-item",
                NamePopupKind::Rename => "rename",
            },
            "title": self.title.to_string(),
            "label": self.label.as_ref().map(|l| l.to_string()),
            "text": self.text(cx),
            "message": self.message.as_ref().map(|m| m.text.to_string()),
            "warning": self.message.as_ref().is_some_and(|m| m.warning),
            "can_confirm": self.can_confirm,
            "kinds": self.kinds.iter().map(|(label, _)| label.to_string()).collect::<Vec<_>>(),
            "selected_kind": self.kinds.get(self.selected_kind).map(|(label, _)| label.to_string()),
        })
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if self.kind == NamePopupKind::Rename && !self.can_confirm {
            return;
        }
        let text = self.text(cx);
        cx.emit(NamePopupEvent::Confirmed(text));
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(NamePopupEvent::Cancelled);
    }

    fn message_box(&self, cx: &App) -> Option<gpui::Div> {
        let message = self.message.as_ref()?;
        let theme = cx.theme();
        let (fg, bg) = if message.warning {
            (UiRole::Warning, UiRole::WarningBg)
        } else {
            (UiRole::Danger, UiRole::DangerBg)
        };
        Some(
            div()
                .px(px(8.0))
                .py(px(4.0))
                .rounded(theme.metric(Metric::CornerRadius))
                .border_1()
                .border_color(theme.color(fg))
                .bg(theme.color(bg))
                .text_color(theme.color(UiRole::Text))
                .child(message.text.clone()),
        )
    }

    /// The field in a box one line high.
    fn field_box(&self, line: Pixels) -> gpui::Div {
        div()
            .flex_1()
            .min_w(px(0.0))
            .h(line)
            .overflow_hidden()
            .child(self.field.clone())
    }

    fn render_new_item(&self, line: Pixels, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let theme = cx.theme();
        let error = self.message.as_ref().is_some_and(|m| !m.warning);
        // The balloon sits above the field, over the popup's header.
        let balloon = self.message_box(cx).map(|b| {
            div()
                .absolute()
                .left(px(8.0))
                .right(px(8.0))
                .bottom(px(FIELD_HEIGHT + 14.0))
                .debug_selector(|| "name-popup-message".into())
                .child(b)
        });
        let row_height = theme.metric(Metric::RowHeight);
        let selection = theme.color(UiRole::Selection);
        let selection_text = theme.color(UiRole::SelectionText);
        let hover = theme.color(UiRole::Hover);
        let radius = theme.metric(Metric::CornerRadius);
        let kinds = (!self.kinds.is_empty()).then(|| {
            div().flex().flex_col().px(px(8.0)).pb(px(8.0)).children(
                self.kinds.iter().enumerate().map(|(ix, (label, icon))| {
                    let selected = ix == self.selected_kind;
                    div()
                        .id(("name-popup-kind", ix))
                        .debug_selector(move || format!("name-popup-kind-{ix}"))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .h(row_height)
                        .px(px(6.0))
                        .rounded(radius)
                        .when(selected, |d| d.bg(selection).text_color(selection_text))
                        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                        .child(Icon::new(*icon))
                        .child(label.clone())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_kind = ix;
                            cx.notify();
                        }))
                }),
            )
        });
        div()
            .id("name-popup")
            .debug_selector(|| "name-popup".into())
            .key_context("NameInput")
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::next_kind))
            .on_action(cx.listener(Self::previous_kind))
            .occlude()
            // A click inside does not reach the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .relative()
            .flex()
            .flex_col()
            .w(px(POPUP_WIDTH))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .child(
                // The title and the field; the balloon sits over the title.
                div()
                    .relative()
                    .child(
                        div()
                            .px(px(12.0))
                            .pt(px(8.0))
                            .pb(px(6.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title.clone()),
                    )
                    .child(
                        div().px(px(8.0)).pb(px(8.0)).child(
                            input_frame(true, error, cx)
                                .h(px(FIELD_HEIGHT))
                                .pl(px(12.0))
                                .pr(px(8.0))
                                .child(self.field_box(line)),
                        ),
                    )
                    .children(balloon),
            )
            .children(kinds)
    }

    fn render_rename(&self, line: Pixels, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let theme = cx.theme();
        let error = self.message.as_ref().map(|m| {
            div()
                .debug_selector(|| "name-popup-message".into())
                .text_color(theme.color(if m.warning {
                    UiRole::Warning
                } else {
                    UiRole::Danger
                }))
                .child(m.text.clone())
        });
        let button = |id, label, default, enabled| dialog_button(id, label, default, enabled, cx);
        div()
            .id("name-popup")
            .debug_selector(|| "name-popup".into())
            .key_context("NameInput")
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .occlude()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .w(px(DIALOG_WIDTH))
            .p(px(20.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .children(self.label.clone().map(|l| div().child(l)))
            .child(
                input_frame(true, self.message.as_ref().is_some_and(|m| !m.warning), cx)
                    .h(px(FIELD_HEIGHT - 4.0))
                    .px(px(8.0))
                    .child(self.field_box(line)),
            )
            .children(error)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .pt(px(6.0))
                    .child(
                        button("cancel", "Cancel", false, true).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.cancel(&Cancel, window, cx),
                        )),
                    )
                    .child(
                        button("refactor", "Refactor", true, self.can_confirm).on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm(&Confirm, window, cx)
                            }),
                        ),
                    ),
            )
    }
}

/// A button of the dialogs (Rename, Copy, Move): the default one in the
/// accent colour, greyed out while it cannot be used.
pub(super) fn dialog_button(
    id: &'static str,
    label: &'static str,
    default: bool,
    enabled: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    let accent = theme.color(UiRole::Accent);
    let on_accent = theme.color(UiRole::OnAccent);
    let border = theme.color(UiRole::BorderInput);
    let hover = theme.color(UiRole::Hover);
    let disabled = theme.color(UiRole::TextDisabled);
    div()
        .id(id)
        .debug_selector(move || format!("name-popup-{id}"))
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
            if default && enabled {
                d.bg(accent).text_color(on_accent)
            } else if default {
                d.border_1().border_color(border).text_color(disabled)
            } else {
                d.border_1()
                    .border_color(border)
                    .hover(move |s| s.bg(hover))
            }
        })
        .child(label)
}

impl Render for NamePopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let line = editor::field_line_height(window, cx);
        let content = match self.kind {
            NamePopupKind::NewItem => self.render_new_item(line, cx),
            NamePopupKind::Rename => self.render_rename(line, cx),
        };
        let backdrop = div()
            .id("name-popup-backdrop")
            .occlude()
            .w(size.width)
            .h(size.height)
            .flex()
            .items_center()
            .justify_center()
            .child(content);
        // A click outside closes the light popup (the reference's
        // `setCancelOnClickOutside`); the dialog is modal.
        match self.kind {
            NamePopupKind::NewItem => backdrop.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
            ),
            NamePopupKind::Rename => backdrop,
        }
    }
}

/// Draws `popup` over the whole window (from any view's render).
pub fn overlay(popup: &Entity<NamePopup>) -> AnyElement {
    deferred(
        anchored()
            .position(point(px(0.0), px(0.0)))
            .child(popup.clone()),
    )
    .into_any_element()
}
