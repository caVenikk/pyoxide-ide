//! The completion popup's list, as the reference's lookup draws an item
//! (`LookupCellRenderer` with `LspCompletionCustomizer.renderLookupElement`):
//! the kind icon, the name in the editor font with the matched chars
//! highlighted (bold for keywords, struck out when deprecated), grey tail
//! text right after it and grey type text at the right edge; under the
//! rows a hint with the keys that insert and replace.

use std::ops::Range;

use code_intel::{CompletionItem, SymbolKind};
use gpui::{
    Action, App, Context, FontWeight, HighlightStyle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, ScrollStrategy, SharedString, StrikethroughStyle, Styled,
    StyledText, UniformListScrollHandle, WeakEntity, Window, div, prelude::FluentBuilder, px,
    uniform_list,
};
use themes::UiRole;
use ui::{Icon, IconName, ThemeExt};

use crate::commands::{Accept, Replace};
use crate::view::CodeIntelView;

use super::{Session, is_bold};

/// Rows visible at once; more scroll.
pub const VISIBLE_ROWS: usize = 11;
/// Room around the rows' content.
const ROW_PAD: f32 = 6.0;
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 5.0;
/// Space kept between the tail text and the type text.
const TYPE_GAP: f32 = 16.0;
const FOOTER_HEIGHT: f32 = 24.0;
/// The popup gets no narrower and no wider (rows longer than that are
/// cut).
const MIN_WIDTH: f32 = 200.0;
const MAX_WIDTH: f32 = 900.0;

/// One row as drawn.
#[derive(Debug, Clone)]
pub struct Row {
    pub icon: Option<IconName>,
    pub label: SharedString,
    /// Matched chars of the label.
    pub matched: Vec<Range<usize>>,
    pub tail: Option<SharedString>,
    pub type_text: Option<SharedString>,
    pub bold: bool,
    pub deprecated: bool,
}

impl Row {
    fn new(item: &CompletionItem, matched: &[Range<usize>]) -> Self {
        Self {
            icon: item.kind.and_then(kind_icon),
            label: item.label.clone().into(),
            matched: matched.to_vec(),
            tail: item.tail_text.clone().map(Into::into),
            type_text: item.type_text.clone().map(Into::into),
            bold: is_bold(item),
            deprecated: item.deprecated,
        }
    }

    /// Chars of the name and tail, and of the type text.
    fn chars(&self) -> (usize, usize) {
        (
            self.label.chars().count() + self.tail.as_ref().map_or(0, |t| t.chars().count()),
            self.type_text.as_ref().map_or(0, |t| t.chars().count()),
        )
    }
}

/// The icon of an item's kind, as the reference's LSP client picks it
/// (`LspCompletionCustomizer.getIcon`): none for modules, keywords,
/// values, units, references, events and operators.
pub fn kind_icon(kind: SymbolKind) -> Option<IconName> {
    use SymbolKind as K;
    Some(match kind {
        K::Text => IconName::NODE_WORD,
        K::Method => IconName::NODE_METHOD,
        K::Function => IconName::NODE_FUNCTION,
        K::Constructor | K::Class => IconName::NODE_CLASS,
        K::Field => IconName::NODE_FIELD,
        K::Variable => IconName::NODE_VARIABLE,
        K::Interface => IconName::NODE_INTERFACE,
        K::Property => IconName::NODE_PROPERTY,
        K::Enum | K::EnumMember => IconName::NODE_ENUM,
        K::Snippet => IconName::NODE_TEMPLATE,
        K::Folder => IconName::FOLDER,
        K::Constant => IconName::NODE_CONSTANT,
        K::Struct => IconName::JSON_OBJECT,
        K::TypeParameter => IconName::NODE_TYPE,
        _ => return None,
    })
}

/// The list of the completion popup.
pub struct CompletionList {
    owner: WeakEntity<CodeIntelView>,
    rows: Vec<Row>,
    selected: usize,
    /// The selection is the person's (else only preselected).
    focused: bool,
    scroll: UniformListScrollHandle,
}

impl CompletionList {
    pub fn new(owner: WeakEntity<CodeIntelView>) -> Self {
        Self {
            owner,
            rows: Vec::new(),
            selected: 0,
            focused: true,
            scroll: UniformListScrollHandle::new(),
        }
    }

    /// Shows the rows of a session, with its selected row in view.
    pub fn set(&mut self, session: &Session, cx: &mut Context<Self>) {
        self.rows = session
            .shown
            .iter()
            .map(|s| Row::new(&session.items[s.item], &s.matched))
            .collect();
        self.selected = session.selected;
        self.focused = session.focused;
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The height of the popup with `rows` rows.
    pub fn height(rows: usize, cx: &App) -> Pixels {
        row_height(cx) * rows.min(VISIBLE_ROWS) as f32 + px(FOOTER_HEIGHT) + px(2.0)
    }

    /// The width of the widest row (the reference's lookup is as wide as
    /// its widest item), in the editor's monospace font, within limits.
    fn width(&self, window: &Window, cx: &App) -> Pixels {
        let theme = cx.theme();
        let font = editor_font(cx);
        let ts = window.text_system();
        let id = ts.resolve_font(&font);
        let size = theme.editor_size();
        let char_width = ts
            .advance(id, size, '0')
            .map(|s| s.width)
            .unwrap_or(size * 0.6);
        let (name, type_text) = self.rows.iter().fold((0, 0), |(n, t), row| {
            let (rn, rt) = row.chars();
            (n.max(rn), t.max(rt))
        });
        let types = if type_text > 0 {
            px(TYPE_GAP) + char_width * type_text as f32
        } else {
            px(0.0)
        };
        (px(2.0 + 2.0 * ROW_PAD + ICON + ICON_GAP) + char_width * name as f32 + types)
            .ceil()
            .clamp(px(MIN_WIDTH), px(MAX_WIDTH))
    }

    /// How far the name of a row is right of the popup's left edge: the
    /// popup is moved left by this much so that the names line up with the
    /// typed prefix.
    pub fn text_inset() -> Pixels {
        px(1.0 + ROW_PAD + ICON + ICON_GAP)
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let muted = theme.color(UiRole::TextMuted);
        let selected = index == self.selected;
        let mut name_style = HighlightStyle {
            font_weight: row.bold.then_some(FontWeight::BOLD),
            strikethrough: row.deprecated.then(|| StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
            ..HighlightStyle::default()
        };
        let label_len = row.label.len();
        let mut highlights = Vec::new();
        if name_style != HighlightStyle::default() {
            highlights.push((0..label_len, name_style));
        }
        name_style.color = Some(theme.color(UiRole::CompletionMatch));
        for range in byte_ranges(&row.label, &row.matched) {
            highlights.push((range, name_style));
        }
        let highlights = merge_highlights(label_len, highlights);
        let owner = self.owner.clone();
        div()
            .id(("completion-row", index))
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(row_height(cx))
            .line_height(row_height(cx))
            .px(px(ROW_PAD))
            .whitespace_nowrap()
            .overflow_hidden()
            .when(selected, |d| {
                // A preselected item of an automatic popup has the inactive
                // selection (`CompletionPopup.selectionInactiveBackground`).
                d.bg(theme.color(if self.focused {
                    UiRole::CompletionSelection
                } else {
                    UiRole::SelectionInactive
                }))
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                if let Some(owner) = owner.upgrade() {
                    owner.update(cx, |v, cx| v.click_completion_row(index, window, cx));
                }
            })
            .child(
                div()
                    .flex_none()
                    .w(px(ICON))
                    .mr(px(ICON_GAP))
                    .children(row.icon.map(|icon| Icon::new(icon).size(px(ICON)))),
            )
            .child(
                div()
                    .flex_none()
                    .child(StyledText::new(row.label.clone()).with_highlights(highlights)),
            )
            .children(
                row.tail
                    .clone()
                    .map(|tail| div().flex_none().text_color(muted).child(tail)),
            )
            .child(div().flex_1().min_w(px(TYPE_GAP)))
            .children(
                row.type_text
                    .clone()
                    .map(|t| div().flex_none().text_color(muted).child(t)),
            )
            .into_any_element()
    }
}

/// The hint under the rows: the keys that insert and replace (the
/// reference's lookup advertisement).
pub fn hint(cx: &App) -> String {
    match (shortcut(&Accept, cx), shortcut(&Replace, cx)) {
        (Some(insert), Some(replace)) => format!("Press {insert} to insert, {replace} to replace"),
        (Some(insert), None) => format!("Press {insert} to insert"),
        _ => String::new(),
    }
}

/// The key of a popup command: its last binding, the one that wins (the
/// popup's commands are bound in its own key context only, which is not on
/// the focus path yet in the frame that opens the popup).
fn shortcut(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings_for_action(action)
        .next_back()
        .map(ui::keys::binding_text)
}

/// The editor's font, ligatures as the editor has them (names and types
/// are code).
fn editor_font(cx: &App) -> gpui::Font {
    let theme = cx.theme();
    let mut font = gpui::font(theme.mono_family());
    if !theme.raw().fonts.editor_ligatures {
        font.features = gpui::FontFeatures::disable_ligatures();
    }
    font
}

/// Rows as high as the editor's lines (about 15 pt with the default font,
/// as the reference's lookup measures).
fn row_height(cx: &App) -> Pixels {
    let theme = cx.theme();
    (theme.editor_size() * 1.2).round().max(px(14.0))
}

/// Char ranges of a text as byte ranges.
fn byte_ranges(text: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let last = offsets.len() - 1;
    ranges
        .iter()
        .filter_map(|r| {
            let (a, b) = (offsets[r.start.min(last)], offsets[r.end.min(last)]);
            (b > a).then_some(a..b)
        })
        .collect()
}

/// Highlights that may overlap (the whole name, then its matched parts) as
/// the sorted, disjoint list `StyledText` takes; a later one wins where
/// they overlap.
fn merge_highlights(
    len: usize,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut bounds: Vec<usize> = highlights
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .chain([0, len])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    bounds
        .windows(2)
        .filter_map(|w| {
            let style = highlights
                .iter()
                .rev()
                .find(|(r, _)| r.start <= w[0] && w[1] <= r.end)?
                .1;
            Some((w[0]..w[1], style))
        })
        .collect()
}

impl Render for CompletionList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let width = self.width(window, cx);
        let visible = self.rows.len().min(VISIBLE_ROWS);
        let list = uniform_list(
            "completion-rows",
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "completion-rows".into())
        .track_scroll(self.scroll.clone())
        .h(row_height(cx) * visible as f32);
        let hint = hint(cx);
        div()
            .debug_selector(|| "completion-popup".into())
            .flex()
            .flex_col()
            .w(width)
            .rounded(px(4.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::CompletionPopup))
            .shadow(ui::components::popup_shadow())
            .overflow_hidden()
            .font(editor_font(cx))
            .text_size(theme.editor_size())
            .text_color(theme.color(UiRole::Text))
            .child(list)
            .child(
                div()
                    .debug_selector(|| "completion-hint".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(FOOTER_HEIGHT))
                    .px(px(ROW_PAD))
                    .border_t_1()
                    .border_color(theme.color(UiRole::PopupBorder))
                    .font_family(theme.ui_family())
                    .text_size(px(12.0))
                    .text_color(muted)
                    .whitespace_nowrap()
                    .child(hint),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_icons() {
        assert_eq!(kind_icon(SymbolKind::Method), Some(IconName::NODE_METHOD));
        assert_eq!(
            kind_icon(SymbolKind::Constructor),
            Some(IconName::NODE_CLASS)
        );
        assert_eq!(kind_icon(SymbolKind::Keyword), None);
        assert_eq!(kind_icon(SymbolKind::Module), None);
    }

    #[test]
    fn highlights_of_a_name() {
        assert_eq!(byte_ranges("añb", &[1..3]), vec![1..4]);
        let bold = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..HighlightStyle::default()
        };
        let matched = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            color: Some(gpui::red()),
            ..HighlightStyle::default()
        };
        let merged = merge_highlights(4, vec![(0..4, bold), (1..2, matched)]);
        assert_eq!(merged, vec![(0..1, bold), (1..2, matched), (2..4, bold)]);
        // Only matched parts: the gaps have no style.
        let merged = merge_highlights(4, vec![(1..2, matched)]);
        assert_eq!(merged, vec![(1..2, matched)]);
    }
}
