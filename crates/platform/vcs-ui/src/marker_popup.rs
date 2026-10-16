//! The popup of a change marker (the reference's
//! `LineStatusMarkerPopupPanel`): under the change, aligned with the text,
//! a toolbar with Previous Change, Next Change, Rollback Lines, Show Diff
//! for Lines and Copy, and below it the base text of modified and deleted
//! lines with the changed words highlighted. The keyboard focus stays in
//! the editor; while the popup shows, the editor gets the key context
//! [`CONTEXT`], so Show Diff and Escape reach the popup.

use std::ops::Range as Bytes;

use editor::{Editor, Highlighter};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, Context, Entity, EventEmitter, Focusable as _, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement, Styled, StyledText, Window, anchored, deferred, div,
    point, px,
};
use imara_diff::{Algorithm, Diff, InternedInput, Interner};
use text::Buffer;
use themes::{TextRole, UiRole};
use ui::components::{Tooltip, small_icon_button};
use ui::icons::IconName;
use ui::theme::{ThemeExt, color};
use vcs::{Range, split_lines};

/// The key context added around the editor while its popup shows.
pub const CONTEXT: &str = "LineStatusPopup";
/// The popup's left edge is this far left of the text, so that the old
/// text lines up with the editor's (`getEditorTextOffset`: a 1 px border
/// and 2 px inside).
pub const TEXT_OFFSET: f32 = 3.0;
/// The toolbar's insets (top, right, bottom, left): the New UI's
/// `VersionControl.MarkerPopup.borderInsets` (3, 8, 3, 10 as top, left,
/// bottom, right), the toolbar's own 3 above and below and 2 at the
/// sides, as measured on the reference: buttons of 22 px, 26 px apart.
const TOOLBAR_INSETS: (f32, f32, f32, f32) = (6.0, 12.0, 6.0, 10.0);
const BUTTON_GAP: f32 = 4.0;
/// Height of the toolbar with its borders.
const TOOLBAR_HEIGHT: f32 = 1.0 + 6.0 + 22.0 + 6.0 + 1.0;
/// Room kept between the popup and the window's right and bottom edges.
const WINDOW_GAP: f32 = 10.0;

/// What a word of the base text became.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordKind {
    Deleted,
    Modified,
}

/// Splits a text into words, whitespace runs and single other chars, as
/// byte ranges.
fn tokens(text: &str) -> Vec<Bytes<usize>> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out: Vec<Bytes<usize>> = Vec::new();
    let mut last: Option<Class> = None;
    for (i, c) in text.char_indices() {
        let k = class(c);
        let joins = k != Class::Other && last.as_ref() == Some(&k);
        match out.last_mut() {
            Some(t) if joins => t.end = i + c.len_utf8(),
            _ => out.push(i..i + c.len_utf8()),
        }
        last = Some(k);
    }
    out
}

/// The changed words of `base` against `doc` (byte ranges of `base`),
/// the reference's word differences of a modified range.
pub fn base_word_highlights(base: &str, doc: &str) -> Vec<(Bytes<usize>, WordKind)> {
    let before = tokens(base);
    let after = tokens(doc);
    let mut input = InternedInput {
        before: Vec::with_capacity(before.len()),
        after: Vec::with_capacity(after.len()),
        interner: Interner::new(before.len() + after.len()),
    };
    input.update_before(before.iter().map(|r| &base[r.clone()]));
    input.update_after(after.iter().map(|r| &doc[r.clone()]));
    let diff = Diff::compute(Algorithm::Myers, &input);
    diff.hunks()
        .filter(|h| !h.before.is_empty())
        .map(|h| {
            let start = before[h.before.start as usize].start;
            let end = before[h.before.end as usize - 1].end;
            let kind = if h.after.is_empty() {
                WordKind::Deleted
            } else {
                WordKind::Modified
            };
            (start..end, kind)
        })
        .collect()
}

/// One line of base text in the popup: syntax spans and changed words as
/// byte ranges.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PopupLine {
    pub text: String,
    pub spans: Vec<(Bytes<usize>, TextRole)>,
    pub words: Vec<(Bytes<usize>, WordKind)>,
}

fn char_to_byte(text: &str, ch: usize) -> usize {
    text.char_indices()
        .nth(ch)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// The base lines of a range for the popup: the base text's syntax spans
/// (the whole base text is highlighted, so that strings and comments over
/// several lines keep their colour) and, for a modified range, the words
/// that changed. `doc` is the document's text of the range's lines.
pub fn popup_lines(
    base: &str,
    doc: &str,
    range: &Range,
    highlighter: Option<Box<dyn Highlighter>>,
) -> Vec<PopupLine> {
    let all: Vec<&str> = split_lines(base).collect();
    let v1 = (range.vcs_line1 as usize).min(all.len());
    let v2 = (range.vcs_line2 as usize).min(all.len()).max(v1);
    let mut lines: Vec<PopupLine> = all[v1..v2]
        .iter()
        .map(|l| PopupLine {
            text: (*l).to_owned(),
            ..PopupLine::default()
        })
        .collect();
    if lines.is_empty() {
        return lines;
    }
    if let Some(mut h) = highlighter {
        let buffer = Buffer::new(base);
        h.reset(&buffer);
        h.prepare(&buffer, v1..v2);
        let mut spans = Vec::new();
        for (i, line) in lines.iter_mut().enumerate() {
            spans.clear();
            h.line_spans(v1 + i, &mut spans);
            line.spans = spans
                .iter()
                .map(|(r, role)| {
                    (
                        char_to_byte(&line.text, r.start)..char_to_byte(&line.text, r.end),
                        *role,
                    )
                })
                .filter(|(r, _)| r.start < r.end)
                .collect();
        }
    }
    if range.line1 < range.line2 {
        let text = all[v1..v2].join("\n");
        let mut starts = Vec::with_capacity(lines.len());
        let mut at = 0;
        for l in &lines {
            starts.push(at);
            at += l.text.len() + 1;
        }
        for (r, kind) in base_word_highlights(&text, doc) {
            for (i, line) in lines.iter_mut().enumerate() {
                let (s, e) = (starts[i], starts[i] + line.text.len());
                let (a, b) = (r.start.max(s), r.end.min(e));
                if a < b {
                    line.words.push((a - s..b - s, kind));
                }
            }
        }
    }
    lines
}

/// What the popup's buttons ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    Previous,
    Next,
    Rollback,
    ShowDiff,
    Copy,
}

impl PopupAction {
    /// The reference's action texts.
    pub fn title(self) -> &'static str {
        match self {
            PopupAction::Previous => "Previous Change",
            PopupAction::Next => "Next Change",
            PopupAction::Rollback => "Rollback Lines",
            PopupAction::ShowDiff => "Show Diff for Lines",
            PopupAction::Copy => "Copy",
        }
    }

    fn icon(self) -> IconName {
        match self {
            PopupAction::Previous => IconName::ARROW_UP,
            PopupAction::Next => IconName::ARROW_DOWN,
            PopupAction::Rollback => IconName::ROLLBACK,
            PopupAction::ShowDiff => IconName::DIFF,
            PopupAction::Copy => IconName::COPY,
        }
    }

    /// The command the action runs from the keyboard (its shortcut is
    /// shown in the tooltip).
    fn command(self) -> Box<dyn Action> {
        match self {
            PopupAction::Previous => Box::new(crate::PreviousChange),
            PopupAction::Next => Box::new(crate::NextChange),
            PopupAction::Rollback => Box::new(crate::RollbackLines),
            PopupAction::ShowDiff => Box::new(crate::ShowDiffLines),
            PopupAction::Copy => Box::new(crate::CopyChangeLines),
        }
    }

    fn id(self) -> &'static str {
        match self {
            PopupAction::Previous => "marker-popup-previous",
            PopupAction::Next => "marker-popup-next",
            PopupAction::Rollback => "marker-popup-rollback",
            PopupAction::ShowDiff => "marker-popup-show-diff",
            PopupAction::Copy => "marker-popup-copy",
        }
    }

    pub const ALL: [PopupAction; 5] = [
        PopupAction::Previous,
        PopupAction::Next,
        PopupAction::Rollback,
        PopupAction::ShowDiff,
        PopupAction::Copy,
    ];
}

/// The popup of one range.
pub struct MarkerPopup {
    editor: Entity<Editor>,
    /// The range as it was when the popup opened (its identity).
    pub range: Range,
    /// The popup goes under this document line.
    pub line: usize,
    lines: Vec<PopupLine>,
    enabled: [bool; 5],
    /// Room from the popup's top to the window's bottom, as last placed:
    /// the old text is cut there (`getPreferredSize`).
    room: std::cell::Cell<Option<Pixels>>,
}

impl EventEmitter<PopupAction> for MarkerPopup {}

impl MarkerPopup {
    /// A popup of `range` under document `line` with the range's base
    /// lines; `enabled` says which actions can run (Previous, Next,
    /// Rollback, Show Diff, Copy, in that order).
    pub fn new(
        editor: Entity<Editor>,
        range: Range,
        line: usize,
        lines: Vec<PopupLine>,
        enabled: [bool; 5],
    ) -> Self {
        Self {
            editor,
            range,
            line,
            lines,
            enabled,
            room: std::cell::Cell::new(None),
        }
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    pub fn lines(&self) -> &[PopupLine] {
        &self.lines
    }

    pub fn is_enabled(&self, action: PopupAction) -> bool {
        let i = PopupAction::ALL.iter().position(|a| *a == action);
        i.is_some_and(|i| self.enabled[i])
    }

    /// The popup's state as JSON (`workbench.state`).
    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "lines": [self.range.line1, self.range.line2],
            "vcs_lines": [self.range.vcs_line1, self.range.vcs_line2],
            "under_line": self.line,
            "actions": PopupAction::ALL.iter().zip(self.enabled).map(|(a, on)| serde_json::json!({
                "title": a.title(),
                "enabled": on,
            })).collect::<Vec<_>>(),
            "text": self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            "words": self.lines.iter().map(|l| l.words.iter().map(|(r, k)| serde_json::json!({
                "text": &l.text[r.clone()],
                "kind": match k { WordKind::Deleted => "deleted", WordKind::Modified => "modified" },
            })).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })
    }

    fn render_line(&self, line: &PopupLine, cx: &App) -> AnyElement {
        let theme = cx.theme();
        // Boundaries of the syntax spans and the changed words.
        let mut cuts: Vec<usize> = vec![0, line.text.len()];
        for (r, _) in &line.spans {
            cuts.push(r.start);
            cuts.push(r.end);
        }
        for (r, _) in &line.words {
            cuts.push(r.start);
            cuts.push(r.end);
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut highlights = Vec::new();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b {
                continue;
            }
            let role = line
                .spans
                .iter()
                .find(|(r, _)| r.start <= a && b <= r.end)
                .map(|(_, role)| *role);
            let word = line
                .words
                .iter()
                .find(|(r, _)| r.start <= a && b <= r.end)
                .map(|(_, k)| *k);
            if role.is_none() && word.is_none() {
                continue;
            }
            let style = role.map(|r| theme.text_style(r));
            highlights.push((
                a..b,
                HighlightStyle {
                    color: style.and_then(|s| s.fg).map(color),
                    font_weight: style.filter(|s| s.is_bold()).map(|_| FontWeight::BOLD),
                    font_style: style
                        .filter(|s| s.is_italic())
                        .map(|_| gpui::FontStyle::Italic),
                    background_color: word.map(|k| {
                        theme.color(match k {
                            WordKind::Deleted => UiRole::DiffRemoved,
                            WordKind::Modified => UiRole::DiffChanged,
                        })
                    }),
                    ..HighlightStyle::default()
                },
            ));
        }
        div()
            .whitespace_nowrap()
            .child(StyledText::new(line.text.clone()).with_highlights(highlights))
            .into_any_element()
    }
}

impl Render for MarkerPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.color(UiRole::MarkerPopupBorder);
        let (line_height, _) = self
            .editor
            .read(cx)
            .text_geometry()
            .map(|(_, lh, cw)| (lh, cw))
            .unwrap_or((px(20.0), px(8.0)));
        let has_text = !self.lines.is_empty();
        let focus = self.editor.focus_handle(cx);
        let buttons: Vec<AnyElement> = PopupAction::ALL
            .iter()
            .enumerate()
            .map(|(i, &action)| {
                let enabled = self.enabled[i];
                let shortcut = ui::keys::shortcut_in(action.command().as_ref(), &focus, window)
                    .map(SharedString::from);
                let button = small_icon_button(action.id(), action.icon(), enabled, cx)
                    .debug_selector(move || action.id().to_owned())
                    .tooltip(Tooltip::text(action.title(), shortcut));
                if enabled {
                    button
                        .on_click(cx.listener(move |_, _: &gpui::ClickEvent, _, cx| {
                            cx.emit(action);
                        }))
                        .into_any_element()
                } else {
                    button.into_any_element()
                }
            })
            .collect();
        let (top, right, bottom, left) = TOOLBAR_INSETS;
        let toolbar = div()
            .flex()
            .flex_row()
            .flex_none()
            .gap(px(BUTTON_GAP))
            .pt(px(top))
            .pr(px(right))
            .pb(px(bottom))
            .pl(px(left))
            .bg(theme.color(UiRole::MarkerPopupToolbar))
            .border_color(border)
            .border_t_1()
            .border_l_1()
            .border_r_1()
            .when(!has_text, |d| d.border_b_1())
            .children(buttons);
        let max_width = window.viewport_size().width - px(WINDOW_GAP);
        let max_height = self
            .room
            .get()
            .map(|room| (room - px(TOOLBAR_HEIGHT)).max(line_height + px(2.0 * TEXT_OFFSET)));
        let text = has_text.then(|| {
            let lines: Vec<AnyElement> =
                self.lines.iter().map(|l| self.render_line(l, cx)).collect();
            // At least as wide as the toolbar (the text is the panel's
            // centre, the toolbar sits at its top left).
            div()
                .flex()
                .flex_col()
                .min_w(toolbar_width())
                .max_w(max_width)
                .when_some(max_height, |d, h| d.max_h(h))
                .overflow_hidden()
                .p(px(TEXT_OFFSET - 1.0))
                .border_1()
                .border_color(border)
                .bg(theme.color(UiRole::MarkerPopupText))
                .font_family(theme.mono_family())
                .text_size(theme.editor_size())
                .line_height(line_height)
                .text_color(theme.color(UiRole::EditorText))
                .children(lines)
        });
        div()
            .id("marker-popup")
            .debug_selector(|| "marker-popup".into())
            .flex()
            .flex_col()
            .items_start()
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .child(toolbar)
            .children(text)
    }
}

/// The popup at its place (returned too): `line`'s bottom in the editor's
/// last frame, [`TEXT_OFFSET`] left of the text's column 0; nothing while
/// the line is not on the screen. Clicks stay in the popup; a press
/// outside it calls `close`.
pub fn render_popup(
    popup: &Entity<MarkerPopup>,
    close: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
    cx: &App,
) -> Option<(AnyElement, Point<Pixels>)> {
    let p = popup.read(cx);
    let editor = p.editor.read(cx);
    let line = p.line.min(editor.buffer().len_lines().saturating_sub(1));
    if !editor.visible_lines()?.contains(&line) {
        return None;
    }
    let end = editor.buffer().line_end(line);
    let bottom = editor.bounds_for_range(end..end)?.bottom();
    let (origin, _, _) = editor.text_geometry()?;
    let area = editor.text_area_bounds()?;
    if bottom > area.bottom() {
        return None;
    }
    let position = point(origin.x - px(TEXT_OFFSET), bottom);
    p.room.set(Some(
        window.viewport_size().height - bottom - px(WINDOW_GAP),
    ));
    Some((
        deferred(
            anchored()
                .position(position)
                .snap_to_window_with_margin(px(WINDOW_GAP / 2.0))
                .child(
                    div()
                        .id("marker-popup-host")
                        .occlude()
                        .on_mouse_down_out(move |_, window, cx| close(window, cx))
                        .child(popup.clone()),
                ),
        )
        .with_priority(1)
        .into_any_element(),
        position,
    ))
}

/// Width of the popup's toolbar (five buttons), for tests and layout.
pub fn toolbar_width() -> Pixels {
    let (_, right, _, left) = TOOLBAR_INSETS;
    px(left + right + 2.0) + px(ui::components::SMALL_BUTTON) * 5.0 + px(BUTTON_GAP) * 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_words_of_the_base_text() {
        let words = |base: &str, doc: &str| -> Vec<(String, WordKind)> {
            base_word_highlights(base, doc)
                .into_iter()
                .map(|(r, k)| (base[r].to_owned(), k))
                .collect()
        };
        assert_eq!(
            words("value = compute(1)", "value = compute(2)"),
            vec![("1".to_owned(), WordKind::Modified)]
        );
        assert_eq!(
            words("a = b + c", "a = b"),
            vec![(" + c".to_owned(), WordKind::Deleted)]
        );
        assert!(words("x", "x y").is_empty());
    }

    #[test]
    fn popup_lines_split_words_by_line() {
        let base = "a\nold_name = 1\nkeep\nz\n";
        let range = Range::new(1, 3, 1, 3);
        let lines = popup_lines(base, "new_name = 1\nkeep", &range, None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "old_name = 1");
        assert_eq!(lines[0].words, vec![(0..8, WordKind::Modified)]);
        assert!(lines[1].words.is_empty());
        // Added lines have no base text.
        let added = Range::new(1, 2, 1, 1);
        assert!(popup_lines(base, "x", &added, None).is_empty());
        // Deleted lines show without word highlights.
        let deleted = Range::new(1, 1, 1, 2);
        let lines = popup_lines(base, "", &deleted, None);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].words.is_empty());
    }
}
