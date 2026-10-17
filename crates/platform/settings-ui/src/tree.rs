//! The page tree of the Settings window with its search field
//! (`SettingsTreeView`, `SettingsSearch`, `SettingsFilter`): top-level rows
//! bold, modified pages in the link colour, pages with errors in the error
//! colour, the project mark at the right of project pages, the group of
//! the rows under the top edge kept as a header while the tree is
//! scrolled. Typing in the tree goes to the search field; Up and Down in
//! the field move the tree's selection; a search keeps the pages found and
//! their parents, expanded.

use std::collections::HashSet;
use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, Focusable as _, FontWeight, IntoElement, KeyDownEvent, Window,
    canvas, div, prelude::*, px,
};
use themes::UiRole;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;

use crate::commands::{Search, SearchHistory};
use crate::search::{Hits, find};
use crate::window::{PROJECT_MARK, SettingsWindow};

/// How long the search waits for the typing to pause (`SettingsFilter`).
pub(crate) const SEARCH_DELAY: Duration = Duration::from_millis(100);
/// Key contexts of the tree and of the search field.
pub const TREE_CONTEXT: &str = "SettingsTree";
pub const SEARCH_CONTEXT: &str = "SettingsSearch";
/// Tree metrics: row height, indent per level, where the arrow column
/// starts and its width (the text of a top-level row starts at 32).
pub(crate) const ROW: f32 = 25.0;
const INDENT: f32 = 16.0;
const ROW_LEFT: f32 = 12.0;
const ARROW: f32 = 16.0;
const ARROW_GAP: f32 = 4.0;
/// The search panel (`searchPanel` border 7, 5, 6, 5) and its field: one
/// line high, square corners, the history icon at its start and the
/// clear button at its end once there is text.
const SEARCH_INSETS: (f32, f32, f32) = (7.0, 5.0, 6.0);
const SEARCH_HEIGHT: f32 = 24.0;
const SEARCH_RADIUS: f32 = 3.0;

/// How the search text changed (`DocumentEvent.EventType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditKind {
    Insert,
    Remove,
    Change,
}

impl EditKind {
    pub fn between(old: &str, new: &str) -> Self {
        if new.len() > old.len() && new.starts_with(old) {
            EditKind::Insert
        } else if new.len() < old.len() && old.starts_with(new) {
            EditKind::Remove
        } else {
            EditKind::Change
        }
    }
}

/// The search in effect.
#[derive(Debug, Default)]
pub(crate) struct Filter {
    /// Its text, trimmed; empty without a search.
    pub text: String,
    pub hits: Option<Hits>,
    /// The pages the tree shows; `None` without a search.
    pub visible: Option<HashSet<usize>>,
    /// Groups expanded while searching (all of them at first).
    pub expanded: HashSet<&'static str>,
}

impl Filter {
    /// A search that found nothing (the field turns red).
    pub fn no_match(&self) -> bool {
        self.hits.as_ref().is_some_and(Hits::is_empty)
    }
}

/// A row of the tree: a page and its depth.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Row {
    pub page: usize,
    pub depth: usize,
}

impl SettingsWindow {
    /// Whether the group `i` shows its children.
    fn is_expanded(&self, i: usize) -> bool {
        let id = self.pages[i].id;
        if self.filter.visible.is_some() {
            self.filter.expanded.contains(id)
        } else {
            self.expanded.contains(id)
        }
    }

    /// The rows shown: the pages the search keeps whose parents are
    /// expanded.
    pub(crate) fn rows(&self) -> Vec<Row> {
        (0..self.pages.len())
            .filter(|i| self.filter.visible.as_ref().is_none_or(|v| v.contains(i)))
            .filter(|&i| {
                let mut at = self.parent_index(i);
                while let Some(p) = at {
                    if !self.is_expanded(p) {
                        return false;
                    }
                    at = self.parent_index(p);
                }
                true
            })
            .map(|i| Row {
                page: i,
                depth: self.depths[i],
            })
            .collect()
    }

    /// The pages a search keeps (`SettingsFilter.shouldBeShowing` in a
    /// filtering tree): those found, the pages under a page found by its
    /// name, and the parents of all of them.
    fn visible_pages(&self, hits: &Hits) -> HashSet<usize> {
        let found = hits.all();
        let mut visible = HashSet::new();
        for i in 0..self.pages.len() {
            let mut shown = found.contains(&i);
            let mut at = Some(i);
            while !shown && let Some(p) = at {
                shown = hits.name.contains(&p);
                at = self.parent_index(p);
            }
            if shown {
                let mut at = Some(i);
                while let Some(p) = at {
                    if !visible.insert(p) {
                        break;
                    }
                    at = self.parent_index(p);
                }
            }
        }
        visible
    }

    /// Searches for `text` (`SettingsFilter.update`): the tree keeps what
    /// it finds; with `adjust` the selection moves to the first page found
    /// unless the selected one is among them (deleting keeps a selected
    /// page that is still shown).
    pub(crate) fn update_filter(
        &mut self,
        text: &str,
        kind: EditKind,
        adjust: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.trim();
        if text.is_empty() {
            self.filter = Filter::default();
        } else {
            let hits = find(&self.pages, text);
            let visible = self.visible_pages(&hits);
            let expanded = visible
                .iter()
                .filter(|&&i| self.children(i).any(|c| visible.contains(&c)))
                .map(|&i| self.pages[i].id)
                .collect();
            self.filter = Filter {
                text: text.to_string(),
                hits: Some(hits),
                visible: Some(visible),
                expanded,
            };
        }
        let current = self.selected;
        let all = self.filter.hits.as_ref().map(Hits::all);
        let mut should_move = self
            .filter
            .hits
            .as_ref()
            .is_none_or(|h| !h.name.contains(&current) && !h.content.contains(&current));
        if should_move
            && kind != EditKind::Insert
            && all.as_ref().is_none_or(|a| a.contains(&current))
        {
            should_move = false;
        }
        let mut candidate = adjust.then_some(current);
        if should_move && let Some(hits) = &self.filter.hits {
            if let Some(&first) = hits.name.first() {
                candidate = Some(first);
            } else if let Some(&first) = hits
                .content
                .iter()
                .find(|&&i| self.pages[i].create.is_some())
                .or(hits.content.first())
            {
                candidate = Some(first);
            }
        }
        if all.as_ref().is_none_or(|a| !a.is_empty())
            && candidate.is_none()
            && let Some(last) = self.last_selected.take()
        {
            candidate = Some(last);
        }
        if candidate.is_none() {
            self.last_selected = Some(current);
        }
        let shown = |i: usize| self.filter.visible.as_ref().is_none_or(|v| v.contains(&i));
        match candidate {
            Some(c) if adjust && c != current && shown(c) => self.select(c, window, cx),
            _ => {
                self.reveal_selected();
                self.hand_over_search(window, cx);
            }
        }
        cx.notify();
    }

    /// Up and Down: the previous or next row (the first one when the
    /// selected page is not shown).
    pub(crate) fn move_selection(
        &mut self,
        by: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let next = match rows.iter().position(|r| r.page == self.selected) {
            Some(at) => (at as isize + by).clamp(0, rows.len() as isize - 1) as usize,
            None => 0,
        };
        if rows[next].page != self.selected {
            self.select(rows[next].page, window, cx);
        }
    }

    /// Right: expands the selected group, or selects its first page.
    pub(crate) fn expand(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.selected;
        if !self.has_children(i) {
            return;
        }
        if !self.is_expanded(i) {
            self.toggle(i, cx);
        } else if let Some(first) = self
            .rows()
            .iter()
            .find(|r| self.pages[r.page].parent == Some(self.pages[i].id))
        {
            self.select(first.page, window, cx);
        }
    }

    /// Left: collapses the selected group, or selects the parent.
    pub(crate) fn collapse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let i = self.selected;
        if self.has_children(i) && self.is_expanded(i) {
            self.toggle(i, cx);
        } else if let Some(parent) = self.parent_index(i) {
            self.select(parent, window, cx);
        }
    }

    pub(crate) fn toggle(&mut self, i: usize, cx: &mut Context<Self>) {
        let id = self.pages[i].id;
        let set = if self.filter.visible.is_some() {
            &mut self.filter.expanded
        } else {
            &mut self.expanded
        };
        if !set.remove(id) {
            set.insert(id);
        }
        cx.notify();
    }

    /// The top-level group of a page.
    fn group_of(&self, i: usize) -> usize {
        let mut at = i;
        while let Some(p) = self.parent_index(at) {
            at = p;
        }
        at
    }

    /// The group shown over the scrolled tree: the group of the row at the
    /// top edge, when the row just below the header belongs to it too
    /// (`GradientViewport.getHeader`, `findGroupNameAt`).
    pub(crate) fn sticky_header(&self, rows: &[Row]) -> Option<usize> {
        let scrolled = -f32::from(self.tree_scroll.offset().y);
        if scrolled <= 0.0 || rows.is_empty() {
            return None;
        }
        let row_at = |y: f32| {
            let n = ((y + scrolled) / ROW).floor().max(0.0) as usize;
            rows.get(n.min(rows.len() - 1)).map(|r| r.page)
        };
        let top = self.group_of(row_at(0.0)?);
        let below = self.group_of(row_at(ROW + 3.0)?);
        (top == below).then_some(top)
    }

    /// Printable keys typed in the tree go to the search field
    /// (`SettingsSearch.keyTyped`).
    fn tree_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = e.keystroke.modifiers;
        if m.control || m.platform || m.alt || m.function {
            return;
        }
        let Some(text) = e
            .keystroke
            .key_char
            .clone()
            .filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))
        else {
            return;
        };
        cx.stop_propagation();
        let focus = self.search.focus_handle(cx);
        window.focus(&focus);
        self.search
            .update(cx, |editor, cx| editor.insert(&text, cx));
    }

    fn render_row(&self, row: Row, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selection = theme.color(if focused {
            UiRole::Selection
        } else {
            UiRole::SelectionInactive
        });
        let text = theme.color(UiRole::TreeText);
        let selected_text = theme.color(UiRole::SelectionText);
        let modified = theme.color(UiRole::TextLink);
        let error = theme.color(UiRole::Danger);
        let muted = theme.color(UiRole::TextMuted);
        let i = row.page;
        let page = &self.pages[i];
        let selected = i == self.selected;
        let parent = self.has_children(i);
        let expanded = self.is_expanded(i);
        // The modified colour shows on a selected row too, as the
        // reference draws it.
        let color = if self.has_error(i) {
            error
        } else if self.is_modified(i, cx) {
            modified
        } else if selected && focused {
            selected_text
        } else {
            text
        };
        let selector = format!("settings-row:{}", page.id);
        div()
            .id(("settings-row", i))
            .debug_selector(move || selector.clone())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(ROW))
            .pl(px(ROW_LEFT + INDENT * row.depth as f32))
            .pr(px(10.0))
            .when(selected, |d| d.bg(selection))
            .child(
                div()
                    .id(("settings-arrow", i))
                    .flex_none()
                    .w(px(ARROW))
                    .when(parent, |d| {
                        d.child(ui::components::tree_arrow(expanded, cx))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.toggle(i, cx);
                            }))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .ml(px(ARROW_GAP))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(color)
                    .when(row.depth == 0, |d| d.font_weight(FontWeight::BOLD))
                    .child(page.title),
            )
            .when(page.scope.is_project(), |d| {
                d.child(
                    div()
                        .id(("settings-project", i))
                        .flex_none()
                        .text_color(muted)
                        .child(Icon::new(IconName::PROJECT_CONFIGURABLE))
                        .tooltip(Tooltip::text(PROJECT_MARK, None)),
                )
            })
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                window.focus(&this.tree_focus);
                this.select(i, window, cx);
                if e.click_count() == 2 && this.has_children(i) {
                    this.toggle(i, cx);
                }
            }))
            .into_any_element()
    }

    /// The search field: a flat field with the history icon, the text and,
    /// once there is text, the clear button; red when nothing is found.
    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let focused = self.search.focus_handle(cx).is_focused(window);
        let error = self.filter.no_match();
        let has_text = !self.search.read(cx).text().is_empty();
        let anchor = self.search_anchor.clone();
        let line = editor::field_line_height_for(true, window, cx);
        let field = ui::components::input_frame(focused, false, cx)
            .rounded(px(SEARCH_RADIUS))
            .h(px(SEARCH_HEIGHT))
            .pl(px(4.0))
            .pr(px(4.0))
            .gap(px(4.0))
            .bg(theme.color(if error {
                UiRole::DangerBg
            } else {
                UiRole::SearchField
            }))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(
                div()
                    .id("settings-search-history")
                    .debug_selector(|| "settings-search-history".into())
                    .flex_none()
                    .flex()
                    .items_center()
                    .pointing_hand()
                    .child(Icon::new(IconName::SEARCH_HISTORY))
                    .tooltip(Tooltip::text("Search History", None))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(SearchHistory), cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(line)
                    .overflow_hidden()
                    .child(self.search.clone()),
            )
            .when(has_text, |d| {
                d.child(
                    div()
                        .id("settings-search-clear")
                        .debug_selector(|| "settings-search-clear".into())
                        .flex_none()
                        .flex()
                        .items_center()
                        .pointing_hand()
                        .child(Icon::new(IconName::CLOSE_SMALL))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.search.focus_handle(cx));
                            window.dispatch_action(
                                Box::new(Search {
                                    text: String::new(),
                                }),
                                cx,
                            );
                        })),
                )
            });
        div()
            .id("settings-search")
            .debug_selector(|| "settings-search".into())
            .key_context(SEARCH_CONTEXT)
            .flex_none()
            .px(px(SEARCH_INSETS.1))
            .pt(px(SEARCH_INSETS.0))
            .pb(px(SEARCH_INSETS.2))
            .child(field)
            .into_any_element()
    }

    pub(crate) fn render_tree(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.tree_focus.is_focused(window);
        let rows = self.rows();
        let header = self.sticky_header(&rows).map(|g| {
            let theme = cx.theme();
            div()
                .debug_selector(|| "settings-sticky-header".into())
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(ROW))
                .flex()
                .items_center()
                .pl(px(ROW_LEFT + ARROW + ARROW_GAP))
                .bg(theme.color(UiRole::SurfaceAlt))
                .text_color(theme.color(UiRole::TreeText))
                .font_weight(FontWeight::BOLD)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(self.pages[g].title)
        });
        let elements: Vec<AnyElement> = rows
            .iter()
            .map(|&row| self.render_row(row, focused, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_search(window, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .id("settings-tree")
                            .debug_selector(|| "settings-tree".into())
                            .key_context(TREE_CONTEXT)
                            .track_focus(&self.tree_focus)
                            .track_scroll(&self.tree_scroll)
                            .on_key_down(cx.listener(Self::tree_key_down))
                            .size_full()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(elements),
                    )
                    .children(header),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_deleting_are_told_apart() {
        assert_eq!(EditKind::between("so", "sof"), EditKind::Insert);
        assert_eq!(EditKind::between("sof", "so"), EditKind::Remove);
        assert_eq!(EditKind::between("soft", "wrap"), EditKind::Change);
        assert_eq!(EditKind::between("", ""), EditKind::Change);
    }
}
