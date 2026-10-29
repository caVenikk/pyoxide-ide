//! A list popup that takes the focus, for the intention actions (quick
//! fixes with their sub-actions), the declaration chooser and Show Usages:
//! a title, rows with an icon, a text (code in the editor font, parts in
//! bold), grey details at the right, separators and sub-actions that open
//! next to their row. It is the reference's list popup as `ui::menu` draws
//! it and answers the same commands (`menu.*`, key context `ContextMenu`),
//! with two differences of the intention list: Enter runs a row that has
//! sub-actions (Right opens them), and the selected row shows a `⋮` button
//! that opens them as well.
//!
//! Typing filters the rows (speed search), Backspace deletes, Escape
//! clears the filter, then closes the sub-actions, then the popup. The
//! popup closes when it loses the focus; it gives the focus back to the
//! element that had it, then reports what was chosen.

use std::ops::Range;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, StyledText, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::menu::{
    Cancel, CloseSubmenu, Confirm, KEY_CONTEXT, OpenSubmenu, SelectNext, SelectPrevious,
};
use ui::pointer::PointerExt as _;
use ui::{Icon, IconName, ThemeExt};

/// Room around the rows and their content (the list popup metrics of
/// `ui::menu`).
const SELECTION_X: f32 = 8.0;
const CONTENT_X: f32 = 8.0;
const PAD_Y: f32 = 8.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const RADIUS: f32 = 8.0;
const SELECTION_RADIUS: f32 = 4.0;
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 6.0;
/// Width of a level of nesting.
const INDENT: f32 = 14.0;
const TITLE_TOP: f32 = 12.0;
const TITLE_BOTTOM: f32 = 10.0;
/// Space between the text and the details of a row.
const DETAIL_GAP: f32 = 24.0;
/// Rows shown at most; more scroll.
const MAX_ROWS: usize = 20;
/// Characters besides letters and digits the speed search takes.
const PUNCTUATION_MARKS: &str = "*_-+\"'/.#$>: ,;?!@%^&";

/// A row of the popup.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListRow {
    pub icon: Option<IconName>,
    pub text: SharedString,
    /// Byte ranges of `text` drawn bold (the symbol in a line of code).
    pub bold: Vec<Range<usize>>,
    /// Grey text at the right (a file and line).
    pub detail: Option<SharedString>,
    /// The text is code: the editor font.
    pub code: bool,
    /// Levels of nesting under the rows before it (a tree shown flat, as
    /// the file structure is).
    pub indent: u8,
    /// Sub-actions, opened next to the row.
    pub children: Vec<ListItem>,
}

impl ListRow {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

/// A row or a separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListItem {
    Row(ListRow),
    Separator,
}

impl ListItem {
    fn row(&self) -> Option<&ListRow> {
        match self {
            ListItem::Row(row) => Some(row),
            ListItem::Separator => None,
        }
    }
}

/// What happened to the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListEvent {
    /// A row was chosen: its index, then the index of the sub-action.
    Chosen(Vec<usize>),
}

/// How a list popup opens.
#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub title: Option<SharedString>,
    /// Grey text after the title (a count).
    pub subtitle: Option<SharedString>,
    /// The selected row shows a `⋮` button.
    pub more_button: bool,
    /// A name for tests (`debug_selector` prefix and `code.state`).
    pub name: &'static str,
    /// The row selected first (the first row that can be chosen without
    /// it).
    pub selected: Option<usize>,
}

/// An open list popup.
pub struct ListPopup {
    focus: FocusHandle,
    origin: Option<FocusHandle>,
    items: Vec<ListItem>,
    options: ListOptions,
    /// The selected row of the list, and of the open sub-actions.
    selected: Option<usize>,
    sub: Option<(usize, usize)>,
    filter: String,
    matcher: Option<Matcher>,
    dismissed: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ListEvent> for ListPopup {}
impl EventEmitter<DismissEvent> for ListPopup {}

impl Focusable for ListPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ListPopup {
    /// A popup of `items` with the first row that can be chosen selected;
    /// it takes the focus.
    pub fn new(
        items: Vec<ListItem>,
        options: ListOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_blur(&focus, window, |this, window, cx| this.dismiss(window, cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.dismiss(window, cx);
                }
            }),
        ];
        let origin = window.focused(cx);
        window.focus(&focus);
        let mut popup = Self {
            focus,
            origin,
            items,
            options,
            selected: None,
            sub: None,
            filter: String::new(),
            matcher: None,
            dismissed: false,
            _subscriptions: subscriptions,
        };
        popup.selected = popup
            .options
            .selected
            .filter(|&i| popup.items.get(i).is_some_and(|item| item.row().is_some()))
            .or_else(|| popup.first_shown());
        popup
    }

    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// The height of the popup: its title and rows (at most the rows shown
    /// at once).
    pub fn height(&self, cx: &App) -> gpui::Pixels {
        let row = Self::row_height(cx);
        let rows: f32 = self
            .items
            .iter()
            .take(MAX_ROWS)
            .map(|i| {
                if i.row().is_some() {
                    row
                } else {
                    SEPARATOR_HEIGHT
                }
            })
            .sum();
        let title = if self.options.title.is_some() {
            TITLE_TOP + row + TITLE_BOTTOM
        } else {
            PAD_Y
        };
        px(title + rows + PAD_Y + 2.0)
    }

    /// Closes the popup and gives the focus back.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismissed {
            return;
        }
        self.dismissed = true;
        if self.focus.contains_focused(window, cx)
            && let Some(origin) = &self.origin
        {
            window.focus(origin);
        }
        cx.emit(DismissEvent);
        cx.notify();
    }

    fn matches(&self, item: &ListItem) -> bool {
        match (&self.matcher, item.row()) {
            (None, _) => true,
            (Some(m), Some(row)) => m.matches(&row.text),
            (Some(_), None) => false,
        }
    }

    /// The rows shown: all without a filter; with one the matching rows
    /// and no separators.
    fn shown(&self) -> Vec<bool> {
        self.items
            .iter()
            .map(|item| match self.matcher {
                None => true,
                Some(_) => item.row().is_some() && self.matches(item),
            })
            .collect()
    }

    fn first_shown(&self) -> Option<usize> {
        let shown = self.shown();
        self.items
            .iter()
            .enumerate()
            .position(|(i, item)| shown[i] && item.row().is_some())
    }

    fn set_filter(&mut self, filter: String, cx: &mut Context<Self>) {
        if filter == self.filter {
            return;
        }
        self.filter = filter;
        self.matcher = (!self.filter.is_empty())
            .then(|| Matcher::new(&format!("*{}", self.filter), MatchingMode::IgnoreCase));
        self.sub = None;
        self.selected = self.first_shown();
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.sub.is_some() {
            return;
        }
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.platform || k.modifiers.alt || k.modifiers.function {
            return;
        }
        if k.key == "backspace" {
            if !self.filter.is_empty() {
                let mut filter = self.filter.clone();
                filter.pop();
                self.set_filter(filter, cx);
                cx.stop_propagation();
            }
            return;
        }
        let Some(text) = k.key_char.as_deref() else {
            return;
        };
        let mut filter = self.filter.clone();
        for c in text.chars() {
            let leading_space = filter.is_empty() && c.is_whitespace();
            if c.is_alphanumeric() || (!leading_space && PUNCTUATION_MARKS.contains(c)) {
                filter.push(c);
            }
        }
        if filter != self.filter {
            self.set_filter(filter, cx);
            cx.stop_propagation();
        }
    }

    fn children(&self, row: usize) -> &[ListItem] {
        self.items
            .get(row)
            .and_then(ListItem::row)
            .map_or(&[], |r| r.children.as_slice())
    }

    /// Moves the selection of the innermost open level, going round.
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (items, shown, current): (&[ListItem], Vec<bool>, Option<usize>) = match self.sub {
            Some((row, child)) => {
                let items = self.children(row);
                (items, vec![true; items.len()], Some(child))
            }
            None => (&self.items, self.shown(), self.selected),
        };
        let n = items.len() as isize;
        if n == 0 {
            return;
        }
        let start = current.map_or(if delta > 0 { -1 } else { n }, |i| i as isize);
        let mut i = start;
        for _ in 0..n {
            i = (i + delta).rem_euclid(n);
            if shown[i as usize] && items[i as usize].row().is_some() {
                match &mut self.sub {
                    Some((_, child)) => *child = i as usize,
                    None => self.selected = Some(i as usize),
                }
                cx.notify();
                return;
            }
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    /// Opens the sub-actions of the selected row, the first one selected.
    fn open_sub(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(row) = self.selected.filter(|_| self.sub.is_none()) else {
            return false;
        };
        let Some(first) = self.children(row).iter().position(|i| i.row().is_some()) else {
            return false;
        };
        self.sub = Some((row, first));
        cx.notify();
        true
    }

    fn open_submenu(&mut self, _: &OpenSubmenu, _: &mut Window, cx: &mut Context<Self>) {
        self.open_sub(cx);
    }

    fn close_submenu(&mut self, _: &CloseSubmenu, _: &mut Window, cx: &mut Context<Self>) {
        if self.sub.take().is_some() {
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filter.is_empty() {
            self.set_filter(String::new(), cx);
        } else if self.sub.take().is_some() {
            cx.notify();
        } else {
            self.dismiss(window, cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let path = match (self.sub, self.selected) {
            (Some((row, child)), _) => vec![row, child],
            (None, Some(row)) => vec![row],
            (None, None) => return,
        };
        self.choose(path, window, cx);
    }

    /// Closes the popup, gives the focus back, then reports the choice.
    fn choose(&mut self, path: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
        cx.emit(ListEvent::Chosen(path));
    }

    /// The popup's state for tests and agents.
    pub fn state_json(&self) -> Json {
        let shown = self.shown();
        let row_json = |row: &ListRow| {
            json!({
                "text": row.text.as_ref(),
                "icon": row.icon.map(|i| i.0),
                "detail": row.detail.as_ref().map(|d| d.as_ref()),
                "indent": row.indent,
                "children": row.children.iter().map(|c| match c {
                    ListItem::Row(r) => json!(r.text.as_ref()),
                    ListItem::Separator => json!("-"),
                }).collect::<Vec<_>>(),
            })
        };
        json!({
            "name": self.options.name,
            "title": self.options.title.as_ref().map(|t| t.as_ref()),
            "subtitle": self.options.subtitle.as_ref().map(|t| t.as_ref()),
            "filter": self.filter,
            "items": self.items.iter().enumerate().filter(|(i, _)| shown[*i]).map(|(_, item)| match item {
                ListItem::Row(row) => row_json(row),
                ListItem::Separator => json!({ "separator": true }),
            }).collect::<Vec<_>>(),
            "selected": self.selected.and_then(|s| self.items[s].row()).map(|r| r.text.as_ref()),
            "sub": self.sub.map(|(row, child)| json!({
                "row": self.items[row].row().map(|r| r.text.as_ref()),
                "selected": self.children(row).get(child).and_then(ListItem::row).map(|r| r.text.as_ref()),
            })),
        })
    }

    // ---- drawing ------------------------------------------------------------

    fn row_height(cx: &App) -> f32 {
        f32::from(cx.theme().metric(Metric::RowHeight))
    }

    fn render_rows(
        &self,
        items: &[ListItem],
        shown: &[bool],
        selected: Option<usize>,
        level: usize,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let theme = cx.theme();
        let height = Self::row_height(cx);
        let with_icons = items
            .iter()
            .any(|i| i.row().is_some_and(|r| r.icon.is_some()));
        let selection = theme.color(UiRole::Selection);
        let muted = theme.color(UiRole::TextMuted);
        let line = theme.color(UiRole::PopupBorder);
        let mono = theme.mono_family();
        let name = self.options.name;
        items
            .iter()
            .enumerate()
            .filter(|(i, _)| shown[*i])
            .map(|(ix, item)| {
                let Some(row) = item.row() else {
                    return div()
                        .flex()
                        .flex_col()
                        .h(px(SEPARATOR_HEIGHT))
                        .pt(px(4.0))
                        .px(px(SELECTION_X))
                        .child(div().h(px(1.0)).w_full().bg(line))
                        .into_any_element();
                };
                let is_selected = selected == Some(ix);
                let bold = HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..HighlightStyle::default()
                };
                let text = StyledText::new(row.text.clone())
                    .with_highlights(row.bold.iter().map(|r| (r.clone(), bold)));
                let more = (level == 0 && is_selected && self.options.more_button).then(|| {
                    div()
                        .id("list-more")
                        .debug_selector(move || format!("{name}-more"))
                        .flex_none()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_sub(cx);
                            }),
                        )
                        .child(Icon::new(IconName::MORE_VERTICAL))
                });
                // Sub-actions: an arrow, unless the row shows its button.
                let arrow = (!row.children.is_empty() && more.is_none())
                    .then(|| Icon::new(IconName::CHEVRON_RIGHT));
                let id = SharedString::from(format!("{name}-{level}-{ix}"));
                let selector = format!("{name}-row-{level}-{ix}");
                div()
                    .id(id)
                    .debug_selector(move || selector.clone())
                    .h(px(height))
                    .px(px(SELECTION_X))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .pl(px(CONTENT_X + INDENT * f32::from(row.indent)))
                            .pr(px(CONTENT_X))
                            .rounded(px(SELECTION_RADIUS))
                            .when(is_selected, |d| d.bg(selection))
                            .when(with_icons, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .w(px(ICON + ICON_GAP))
                                        .children(row.icon.map(Icon::new)),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .when(row.code, |d| d.font_family(mono.clone()))
                                    .child(text),
                            )
                            .children(row.detail.clone().map(|detail| {
                                div()
                                    .flex_none()
                                    .pl(px(DETAIL_GAP))
                                    .text_color(muted)
                                    .whitespace_nowrap()
                                    .child(detail)
                            }))
                            .children(arrow.map(|a| div().flex_none().pl(px(8.0)).child(a)))
                            .children(more.map(|m| m.ml(px(8.0)))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let path = match level {
                            0 => vec![ix],
                            _ => match this.sub {
                                Some((row, _)) => vec![row, ix],
                                None => return,
                            },
                        };
                        this.choose(path, window, cx);
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for ListPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = Self::row_height(cx);
        let shown = self.shown();
        let items = self.items.clone();
        let rows = self.render_rows(&items, &shown, self.selected, 0, cx);
        // The open sub-actions, next to their row.
        let sub_rows = self.sub.map(|(row, child)| {
            let children = self.children(row).to_vec();
            let all = vec![true; children.len()];
            let shown_before = shown[..row].iter().filter(|s| **s).count();
            (
                self.render_rows(&children, &all, Some(child), 1, cx),
                shown_before,
            )
        });
        let theme = cx.theme();
        let text = theme.color(UiRole::Text);
        let muted = theme.color(UiRole::TextMuted);
        let line = theme.color(UiRole::PopupBorder);
        let name = self.options.name;
        let frame = || {
            div()
                .flex()
                .flex_col()
                .rounded(px(RADIUS))
                .border_1()
                .border_color(theme.color(UiRole::PopupBorder))
                .bg(theme.color(UiRole::SurfaceRaised))
                .shadow(ui::components::popup_shadow())
        };
        let title = self.options.title.clone().map(|title| {
            div()
                .flex()
                .flex_row()
                .justify_center()
                .gap(px(8.0))
                .pt(px(TITLE_TOP))
                .pb(px(TITLE_BOTTOM))
                .px(px(10.0))
                .whitespace_nowrap()
                .child(div().font_weight(FontWeight::BOLD).child(title))
                .children(
                    self.options
                        .subtitle
                        .clone()
                        .map(|s| div().text_color(muted).child(s)),
                )
        });
        let search = (!self.filter.is_empty()).then(|| {
            div()
                .debug_selector(move || format!("{name}-search"))
                .mx(px(12.0))
                .my(px(4.0))
                .pb(px(8.0))
                .pt(px(4.0))
                .border_b_1()
                .border_color(line)
                .flex()
                .flex_row()
                .gap(px(4.0))
                .child(Icon::new(IconName::SEARCH))
                .child(div().whitespace_nowrap().child(self.filter.clone()))
        });
        let visible = shown.iter().filter(|s| **s).count().min(MAX_ROWS);
        let main = frame()
            .debug_selector(move || format!("{name}-popup"))
            .children(title)
            .children(search)
            .child(
                // The hand over the list (`ListPopupImpl`), on an element
                // that does not scroll.
                div()
                    .child(
                        div()
                            .id("list-rows")
                            .flex()
                            .flex_col()
                            .when(self.options.title.is_none(), |d| d.pt(px(PAD_Y)))
                            .pb(px(PAD_Y))
                            .max_h(px(height * visible as f32 + 2.0 * PAD_Y))
                            .overflow_y_scroll()
                            .children(rows),
                    )
                    .pointing_hand(),
            );
        let header = if self.options.title.is_some() {
            TITLE_TOP + height + TITLE_BOTTOM - PAD_Y
        } else {
            0.0
        };
        let sub = sub_rows.map(|(rows, before)| {
            div().mt(px(header + height * before as f32)).child(
                frame()
                    .debug_selector(move || format!("{name}-sub"))
                    .py(px(PAD_Y))
                    .children(rows)
                    .pointing_hand(),
            )
        });
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::open_submenu))
            .on_action(cx.listener(Self::close_submenu))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_key_down(cx.listener(Self::key_down))
            .flex()
            .flex_row()
            .items_start()
            .font_family(cx.theme().ui_family())
            .text_size(cx.theme().ui_size())
            .text_color(text)
            .child(main)
            .children(sub)
    }
}
