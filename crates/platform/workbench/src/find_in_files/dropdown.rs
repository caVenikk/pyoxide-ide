//! Lists below the popup's fields: the search and replace history
//! (Option+Down / Alt+Down, or the history icon), the recent file masks and
//! folders (the arrow of their fields) and the named scopes.

use gpui::{
    AnyElement, ClickEvent, Context, Focusable as _, InteractiveElement, IntoElement,
    ParentElement, Window, deferred, div, prelude::*, px,
};
use serde_json::json;
use themes::{Metric, UiRole};
use ui::theme::ThemeExt;

use super::{FindInFiles, NamedScope, ScopeKind, settings};

/// Rows a history list shows before it scrolls, and a combo's list.
const HISTORY_ROWS: usize = 15;
const COMBO_ROWS: usize = 8;
const ROW: f32 = 24.0;

/// What a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DropKind {
    SearchHistory,
    ReplaceHistory,
    Masks,
    Directories,
    Scopes,
    Modules,
}

impl DropKind {
    fn name(self) -> &'static str {
        match self {
            DropKind::SearchHistory => "search-history",
            DropKind::ReplaceHistory => "replace-history",
            DropKind::Masks => "masks",
            DropKind::Directories => "directories",
            DropKind::Scopes => "scopes",
            DropKind::Modules => "modules",
        }
    }

    fn rows(self) -> usize {
        match self {
            DropKind::SearchHistory | DropKind::ReplaceHistory => HISTORY_ROWS,
            _ => COMBO_ROWS,
        }
    }
}

/// An open list.
#[derive(Debug, Clone)]
pub(super) struct Dropdown {
    pub kind: DropKind,
    pub items: Vec<String>,
    pub selected: usize,
}

impl Dropdown {
    pub fn state_json(&self) -> serde_json::Value {
        json!({
            "kind": self.kind.name(),
            "items": self.items,
            "selected": self.selected,
        })
    }
}

impl FindInFiles {
    /// The items of a list, newest first, and the one to select (the
    /// field's current text when it is there).
    fn dropdown_items(&self, kind: DropKind, cx: &gpui::App) -> (Vec<String>, String) {
        let last = editor::find::last_search(cx);
        let settings = settings(cx);
        match kind {
            DropKind::SearchHistory => (last.history, self.query.read(cx).text()),
            DropKind::ReplaceHistory => (last.replace_history, self.replace.read(cx).text()),
            DropKind::Masks => (settings.recent_masks, self.mask.read(cx).text()),
            DropKind::Directories => {
                let current = self.directory.read(cx).text();
                let mut items = settings.recent_directories;
                if !current.is_empty() && !items.contains(&current) {
                    items.insert(0, current.clone());
                }
                (items, current)
            }
            DropKind::Modules => (vec![self.module_name()], self.module_name()),
            DropKind::Scopes => (
                self.named_scopes()
                    .iter()
                    .map(|s| s.name().to_owned())
                    .collect(),
                self.named.name().to_owned(),
            ),
        }
    }

    /// Opens a list below its field (or closes it when it is open).
    pub(super) fn show_dropdown(
        &mut self,
        kind: DropKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dropdown.as_ref().is_some_and(|d| d.kind == kind) {
            self.close_dropdown(window, cx);
            return;
        }
        let (items, current) = self.dropdown_items(kind, cx);
        if items.is_empty() {
            return;
        }
        let selected = items.iter().position(|i| *i == current).unwrap_or(0);
        self.dropdown = Some(Dropdown {
            kind,
            items,
            selected,
        });
        window.focus(&self.dropdown_focus);
        cx.notify();
    }

    fn anchor(&self, kind: DropKind) -> &gpui::Entity<editor::Editor> {
        match kind {
            DropKind::SearchHistory => &self.query,
            DropKind::ReplaceHistory => &self.replace,
            DropKind::Masks => &self.mask,
            DropKind::Directories => &self.directory,
            DropKind::Scopes | DropKind::Modules => &self.query,
        }
    }

    fn close_dropdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = self.dropdown.take() {
            window.focus(&self.anchor(d.kind).focus_handle(cx));
            cx.notify();
        }
    }

    fn pick(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.dropdown.take() else {
            return;
        };
        let Some(item) = d.items.get(index.unwrap_or(d.selected)).cloned() else {
            return;
        };
        match d.kind {
            DropKind::Modules => {
                self.scope = ScopeKind::Module;
                window.focus(&self.query.focus_handle(cx));
            }
            DropKind::Scopes => {
                if let Some(named) = NamedScope::from_name(&item) {
                    self.named = named;
                    self.scope = ScopeKind::Named;
                    super::update_settings(cx, |s| s.search_scope = Some(item.clone()));
                }
                window.focus(&self.query.focus_handle(cx));
                self.schedule(cx);
            }
            kind => {
                let field = self.anchor(kind).clone();
                // The field's own subscription searches again.
                field.update(cx, |f, cx| f.set_text_selected(&item, cx));
                window.focus(&field.focus_handle(cx));
                if kind != DropKind::ReplaceHistory {
                    self.schedule(cx);
                }
            }
        }
        cx.notify();
    }

    fn move_in_dropdown(&mut self, down: bool, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.dropdown {
            let len = d.items.len();
            d.selected = if down {
                (d.selected + 1) % len
            } else {
                (d.selected + len - 1) % len
            };
            cx.notify();
        }
    }

    /// The list for the field with the focus: its history, or the recent
    /// masks or folders.
    pub(super) fn show_dropdown_action(
        &mut self,
        _: &editor::find::ShowHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_preview(window, cx) {
            cx.propagate();
            return;
        }
        let kind = match self.focused_field_kind(window, cx) {
            super::Field::Query => DropKind::SearchHistory,
            super::Field::Replace => DropKind::ReplaceHistory,
            super::Field::Mask => DropKind::Masks,
            super::Field::Directory => DropKind::Directories,
        };
        self.show_dropdown(kind, window, cx);
    }

    pub(super) fn dropdown_up_action(
        &mut self,
        _: &editor::find::HistoryUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_in_dropdown(false, cx);
    }

    pub(super) fn dropdown_down_action(
        &mut self,
        _: &editor::find::HistoryDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_in_dropdown(true, cx);
    }

    pub(super) fn dropdown_pick_action(
        &mut self,
        _: &editor::find::HistoryPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick(None, window, cx);
    }

    pub(super) fn dropdown_close_action(
        &mut self,
        _: &editor::find::HistoryClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_dropdown(window, cx);
    }

    /// The open list, when it belongs below this field: drawn over the rest
    /// of the popup.
    pub(super) fn render_dropdown(
        &self,
        kinds: &[DropKind],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let d = self.dropdown.as_ref().filter(|d| kinds.contains(&d.kind))?;
        let theme = cx.theme();
        let selected_bg = theme.color(UiRole::Selection);
        let selected_text = theme.color(UiRole::SelectionText);
        let hover = theme.color(UiRole::Hover);
        let rows: Vec<_> = d
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let selected = i == d.selected;
                let text: String = item.replace('\n', " ⏎ ");
                div()
                    .id(("find-in-files-dropdown-item", i))
                    .flex_none()
                    .h(px(ROW))
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .rounded(px(4.0))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(selected, |d| d.bg(selected_bg).text_color(selected_text))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(text)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.pick(Some(i), window, cx)
                    }))
            })
            .collect();
        let height = px(ROW) * d.items.len().min(d.kind.rows()) as f32 + px(10.0);
        Some(
            deferred(
                div()
                    .id("find-in-files-dropdown")
                    .debug_selector(|| "find-in-files-dropdown".into())
                    .key_context("FindHistory")
                    .track_focus(&self.dropdown_focus)
                    .absolute()
                    .top_full()
                    .left_0()
                    .right_0()
                    .mt(px(2.0))
                    .h(height)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .p(px(4.0))
                    .rounded(theme.metric(Metric::CornerRadius) * 2.0)
                    .border_1()
                    .border_color(theme.color(UiRole::PopupBorder))
                    .bg(theme.color(UiRole::SurfaceRaised))
                    .shadow(ui::components::popup_shadow())
                    .children(rows),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
