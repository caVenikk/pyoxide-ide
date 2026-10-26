//! Speed search of the object tree (ref DBCTX83): typing in the tree opens
//! a search field over the left of the toolbar (a search icon, the text, a
//! filter button), selects the first visible row whose name matches from
//! the selected one on, and highlights the matched letters in every row;
//! Up and Down go to the previous and next match, Backspace deletes,
//! Escape closes the search. The filter button keeps only the matching
//! rows and the rows above them. Names match as the reference's tree
//! speed search matches them (words, camel humps, a `*` before the
//! pattern).

use std::ops::Range;

use gpui::{
    App, ClickEvent, Context, FontWeight, HighlightStyle, IntoElement, KeyDownEvent, StyledText,
    div, prelude::*, px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::Explorer;
use crate::tree::Row;

ui::ui_commands! {
    SEARCH_COMMANDS, scope = "Database";
    /// Search the tree.
    Search { text: String } => "db.explorer.search", "Speed Search",
        "Types `text` into the speed search of the Database tool window's tree (empty closes it): the first matching row is selected and the matches are highlighted.",
        r#"{"text": "ord"}"#;
    /// Toggle the filter of the speed search.
    ToggleSearchFilter => "db.explorer.search-filter", "Filter Matching Rows",
        "Turns the filter of the Database tool window's speed search on or off: only the matching rows and the rows above them are shown.";
}

/// The search field's width (logical px, read off the reference: it
/// covers the toolbar's first group).
const FIELD_WIDTH: f32 = 116.0;

#[derive(Debug, Default, Clone)]
pub(crate) struct SpeedSearch {
    pub text: String,
    pub filter: bool,
    matcher: Option<Matcher>,
}

impl SpeedSearch {
    pub fn active(&self) -> bool {
        !self.text.is_empty()
    }

    fn set(&mut self, text: String) {
        self.matcher =
            (!text.is_empty()).then(|| Matcher::new(&format!("*{text}"), MatchingMode::IgnoreCase));
        self.text = text;
        if self.text.is_empty() {
            self.filter = false;
        }
    }

    /// The matched char ranges of `name`; `None` when it does not match
    /// or nothing is searched.
    pub fn find(&self, name: &str) -> Option<Vec<Range<usize>>> {
        self.matcher.as_ref()?.find(name)
    }

    /// The highlights of `name` (byte ranges) for a [`StyledText`].
    pub fn highlights(&self, name: &str, cx: &App) -> Vec<(Range<usize>, HighlightStyle)> {
        let Some(fragments) = self.find(name) else {
            return Vec::new();
        };
        let mut offsets: Vec<usize> = name.char_indices().map(|(i, _)| i).collect();
        offsets.push(name.len());
        let style = HighlightStyle {
            background_color: Some(cx.theme().color(UiRole::SearchMatch)),
            ..HighlightStyle::default()
        };
        fragments
            .iter()
            .filter_map(|r| {
                let a = r.start.min(offsets.len() - 1);
                let b = r.end.min(offsets.len() - 1);
                (b > a).then(|| (offsets[a]..offsets[b], style))
            })
            .collect()
    }
}

/// The rows a filter keeps: the matching ones (by `matches`) and the rows
/// they are under.
pub(crate) fn filter_rows(rows: Vec<Row>, matches: impl Fn(&Row) -> bool) -> Vec<Row> {
    let mut keep = vec![false; rows.len()];
    // The depth of each open ancestor of the current row, by index.
    let mut stack: Vec<usize> = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        while stack.last().is_some_and(|&a| rows[a].depth >= row.depth) {
            stack.pop();
        }
        if matches(row) {
            keep[ix] = true;
            for &a in &stack {
                keep[a] = true;
            }
        }
        stack.push(ix);
    }
    rows.into_iter()
        .zip(keep)
        .filter_map(|(r, k)| k.then_some(r))
        .collect()
}

pub(super) fn init(cx: &mut App) {
    ui::command::register(cx, SEARCH_COMMANDS);
    cx.on_action(|a: &Search, cx| {
        let text = a.text.clone();
        super::with_target(cx, move |e, _, cx| e.set_search(text, cx))
    });
    cx.on_action(|_: &ToggleSearchFilter, cx| {
        super::with_target(cx, |e, _, cx| e.toggle_search_filter(cx))
    });
}

impl Explorer {
    /// The row's name matches the search.
    fn row_matches(&self, row: &Row) -> bool {
        self.tree
            .search
            .find(&self.row_text(row).view.name)
            .is_some()
    }

    /// Sets the searched text and selects the first match from the
    /// selected row on.
    pub(super) fn set_search(&mut self, text: String, cx: &mut Context<Self>) {
        self.tree.search.set(text);
        self.rebuild_rows();
        if self.tree.search.active() {
            let start = self.selected_ix().unwrap_or(0);
            let n = self.tree.rows.len();
            let found = (0..n)
                .map(|i| (start + i) % n.max(1))
                .find(|&i| self.row_matches(&self.tree.rows[i].clone()));
            if let Some(ix) = found {
                self.select_ix(ix, cx);
            }
        }
        cx.notify();
    }

    fn toggle_search_filter(&mut self, cx: &mut Context<Self>) {
        if !self.tree.search.active() {
            return;
        }
        self.tree.search.filter = !self.tree.search.filter;
        self.rebuild_rows();
        cx.notify();
    }

    /// Up and Down while searching: the previous or next match. `false`
    /// when no search is open.
    pub(super) fn search_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if !self.tree.search.active() {
            return false;
        }
        let n = self.tree.rows.len();
        if n == 0 {
            return true;
        }
        let start = self.selected_ix().unwrap_or(0);
        let found = (1..=n)
            .map(|i| (start as isize + delta * i as isize).rem_euclid(n as isize) as usize)
            .find(|&i| self.row_matches(&self.tree.rows[i].clone()));
        if let Some(ix) = found {
            self.select_ix(ix, cx);
        }
        true
    }

    /// A key in the tree: typing searches, Backspace deletes, Escape
    /// closes the search. `true` when the key was taken.
    pub(super) fn search_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let m = &e.keystroke.modifiers;
        if m.control || m.platform || m.function {
            return false;
        }
        match e.keystroke.key.as_str() {
            "backspace" if self.tree.search.active() => {
                let mut text = self.tree.search.text.clone();
                text.pop();
                self.set_search(text, cx);
                true
            }
            "escape" if self.tree.search.active() => {
                self.set_search(String::new(), cx);
                true
            }
            _ => match e.keystroke.key_char.as_deref() {
                Some(c)
                    if !c.is_empty()
                        && c.chars().all(|c| !c.is_control())
                        && (self.tree.search.active() || !c.starts_with(' ')) =>
                {
                    let text = format!("{}{c}", self.tree.search.text);
                    self.set_search(text, cx);
                    true
                }
                _ => false,
            },
        }
    }

    /// The name of a row with the matched letters highlighted.
    pub(super) fn row_label(&self, name: &str, cx: &App) -> StyledText {
        let highlights = if self.tree.search.active() {
            self.tree.search.highlights(name, cx)
        } else {
            Vec::new()
        };
        StyledText::new(name.to_owned()).with_highlights(highlights)
    }

    /// The search field over the left of the toolbar.
    pub(super) fn render_search_field(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.tree.search.active() {
            return None;
        }
        let theme = cx.theme();
        let no_match = !self
            .tree
            .rows
            .iter()
            .any(|r| self.tree.search.find(&self.row_text(r).view.name).is_some());
        let filter = self.tree.search.filter;
        Some(
            div()
                .id("database-search")
                .debug_selector(|| "database-search".into())
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(px(FIELD_WIDTH))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .bg(theme.color(UiRole::Surface))
                .border_1()
                .border_color(theme.color(UiRole::Border))
                .child(Icon::new(IconName::SEARCH_SMALL).size(px(16.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .when(no_match, |d| d.text_color(theme.color(UiRole::Danger)))
                        .child(self.tree.search.text.clone()),
                )
                .child(
                    div()
                        .id("database-search-filter")
                        .debug_selector(|| "database-search-filter".into())
                        .flex_none()
                        .rounded(px(3.0))
                        .when(filter, |d| d.bg(theme.color(UiRole::Hover)))
                        .tooltip(Tooltip::text("Filter", None))
                        .child(Icon::new(IconName::FILTER).size(px(16.0)))
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toggle_search_filter(cx)
                            }),
                        ),
                )
                .font_weight(FontWeight::NORMAL)
                .into_any_element(),
        )
    }

    /// The search for `db.state`.
    pub(super) fn search_json(&self) -> Json {
        json!({
            "text": self.tree.search.text,
            "filter": self.tree.search.filter,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::tree::{NodeKey, RowKey};

    fn row(depth: usize, n: u64) -> Row {
        let _ = n;
        Row {
            key: RowKey {
                source: Arc::from(format!("s{depth}-{n}").as_str()),
                node: NodeKey::Source,
            },
            depth,
            expandable: true,
            expanded: true,
        }
    }

    #[test]
    fn a_filter_keeps_matches_and_their_parents() {
        let rows = vec![
            row(0, 0),
            row(1, 1),
            row(2, 2),
            row(2, 3),
            row(1, 4),
            row(2, 5),
        ];
        // Matches: the rows 3 and 5.
        let kept = filter_rows(rows, |r| matches!(&*r.key.source, "s2-3" | "s2-5"));
        let names: Vec<_> = kept.iter().map(|r| r.key.source.to_string()).collect();
        assert_eq!(names, ["s0-0", "s1-1", "s2-3", "s1-4", "s2-5"]);
    }

    #[test]
    fn names_match_anywhere_by_words() {
        let mut s = SpeedSearch::default();
        s.set("ord".into());
        assert_eq!(s.find("order_items"), Some(vec![0..3]));
        assert!(s.find("customers").is_none());
        assert!(s.find("v_order_totals").is_some());
        s.set(String::new());
        assert!(!s.active());
    }
}
