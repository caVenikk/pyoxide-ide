//! The popup of a change marker (the reference's
//! `LineStatusMarkerPopupPanel`): under the change, aligned with the text,
//! a toolbar row and below it the base text of modified and deleted lines.
//!
//! The toolbar row has the reference's width whatever the text: Previous
//! Change, Next Change, Rollback Lines, Show Diff for Lines, Copy and the
//! Changelists group, then the "Commit this change" field with Amend and
//! Commit (`CommitChunkComponent`). Changelists and the commit of one
//! change are not there yet: the group and the field are drawn disabled,
//! so that the row is as wide as the reference's.
//!
//! The base text is a read-only editor without a gutter (the reference's
//! `EditorTextField` in renderer mode): its text can be selected and
//! copied, its lines keep the base file's colours, and its words are
//! compared with the current lines (`ByWord`): changed and deleted words
//! are highlighted, a bar marks where words were inserted. The same
//! differences go over the current lines in the editor
//! ([`doc_highlights`]). The keyboard focus stays in the editor until the
//! text is clicked; the editor and the popup get the key context
//! [`CONTEXT`], so Show Diff and Escape reach the popup from both.

use std::ops::Range as Bytes;

use editor::{DiffHighlights, Editor, Highlighter, Span};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable as _,
    Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, Rgba,
    SharedString, StatefulInteractiveElement, Styled, TextRun, Window, anchored, div, point, px,
};
use imara_diff::{Algorithm, Diff, InternedInput, Interner};
use text::{AppliedEdit, Buffer};
use themes::UiRole;
use ui::components::{SMALL_BUTTON, Tooltip, input_frame, small_icon_button};
use ui::deferred;
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;
use vcs::{Range, split_lines};

/// The key context added around the editor and the popup while it shows.
pub const CONTEXT: &str = "LineStatusPopup";
/// The popup's left edge is this far left of the text, so that the old
/// text lines up with the editor's (`getEditorTextOffset`: a 1 px border
/// and 2 px inside).
pub const TEXT_OFFSET: f32 = 3.0;
/// Room after the widest line of the old text, as measured on the
/// reference (its text field's preferred width).
pub const TEXT_END_ROOM: f32 = 22.0;
/// The toolbar row inside its border: the New UI's
/// `VersionControl.MarkerPopup.borderInsets` (3, 8, 3, 10 as top, left,
/// bottom, right) with the toolbar's own 2 px at its left; the commit
/// field's 1 px paddings (`DarculaUIUtil.paddings`) above and below it.
const ROW_LEFT: f32 = 10.0;
const ROW_RIGHT: f32 = 10.0;
const ROW_VERTICAL: f32 = 4.0;
/// The toolbar's buttons: 22 px, 26 px apart, 2 px after the last one.
const BUTTON_GAP: f32 = 4.0;
const TOOLBAR_END: f32 = 2.0;
/// The commit panel (`CommitChunkComponent.Spec`): 255 px with a 12 px
/// gap at its left; the field inside 28 px high, its text 8 px from its
/// left edge, its buttons 3 px from its right edge
/// (`CommitInputBorder.COMMIT_BORDER_INSET`), a separator 20 px high with
/// 3 px at both sides between them.
const COMMIT_PANEL: f32 = 255.0;
const COMMIT_GAP: f32 = 12.0;
const COMMIT_PADDING: f32 = 1.0;
const COMMIT_FIELD_HEIGHT: f32 = 28.0;
const COMMIT_TEXT_LEFT: f32 = 7.0;
const COMMIT_INSET: f32 = 3.0;
const COMMIT_SEPARATOR: (f32, f32) = (20.0, 3.0);
/// Interior height of the toolbar row.
const ROW_HEIGHT: f32 = ROW_VERTICAL * 2.0 + COMMIT_FIELD_HEIGHT;
/// Height of the toolbar row with its top border and the border under it.
const TOOLBAR_HEIGHT: f32 = 1.0 + ROW_HEIGHT + 1.0;
/// Room kept between the popup and the window's right and bottom edges.
const WINDOW_GAP: f32 = 10.0;
/// Word differences are not looked for in ranges larger than this (the
/// reference gives up after 200 ms, `BackgroundTaskUtil.tryComputeFast`).
const WORD_DIFF_LIMIT: usize = 64 * 1024;
/// The share of the background in the colour of the changed lines in the
/// editor (`TextDiffTypeFactory.MIDDLE_COLOR_FACTOR`).
const LINE_BACKGROUND_SHARE: f32 = 0.6;

/// What happened to a word: inserted (nothing in the base text), deleted
/// (nothing in the current text) or changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordKind {
    Inserted,
    Deleted,
    Modified,
}

impl WordKind {
    pub fn name(self) -> &'static str {
        match self {
            WordKind::Inserted => "inserted",
            WordKind::Deleted => "deleted",
            WordKind::Modified => "modified",
        }
    }
}

/// A word difference of a range: byte ranges of the base text and of the
/// current text of the range's lines (`DiffFragment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordFragment {
    pub base: Bytes<usize>,
    pub doc: Bytes<usize>,
}

impl WordFragment {
    /// `DiffUtil.getDiffType`.
    pub fn kind(&self) -> WordKind {
        if self.base.is_empty() {
            WordKind::Inserted
        } else if self.doc.is_empty() {
            WordKind::Deleted
        } else {
            WordKind::Modified
        }
    }
}

/// Splits a text into words, whitespace runs, line breaks and single other
/// chars, as byte ranges (the chunks of `ByWord`: words, and a line break
/// always on its own).
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
        } else if c.is_whitespace() && c != '\n' {
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

/// The word differences of `base` against `doc` (the reference's
/// `ByWord.compare` of a modified range); nothing for texts larger than
/// 64 KiB together.
pub fn word_diff(base: &str, doc: &str) -> Vec<WordFragment> {
    if base.len() + doc.len() > WORD_DIFF_LIMIT {
        return Vec::new();
    }
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
    // An empty side is the position where the other side's text goes.
    let at = |tokens: &[Bytes<usize>], range: std::ops::Range<u32>, len: usize| {
        let (s, e) = (range.start as usize, range.end as usize);
        if s < e {
            tokens[s].start..tokens[e - 1].end
        } else {
            let p = tokens.get(s).map_or(len, |t| t.start);
            p..p
        }
    };
    diff.hunks()
        .map(|h| WordFragment {
            base: at(&before, h.before, base.len()),
            doc: at(&after, h.after, doc.len()),
        })
        .collect()
}

/// The old text of a range as the popup shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PopupText {
    /// The range's base lines joined with line breaks (empty lines too).
    pub text: String,
    /// Syntax spans of each line (chars of the line), as the whole base
    /// text colours them.
    pub spans: Vec<Vec<Span>>,
    /// The word differences against the current lines (byte ranges of
    /// `text` and of the current text of the range's lines); only for a
    /// modified range.
    pub words: Vec<WordFragment>,
}

impl PopupText {
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The lines of the text.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n').take(self.spans.len())
    }
}

/// The popup's text of a range: the base lines `vcs_line1..vcs_line2`, the
/// base text's syntax spans (the whole base text is highlighted, so that
/// strings and comments over several lines keep their colour) and, for a
/// modified range, the word differences. `doc` is the current text of the
/// range's lines joined with line breaks.
pub fn popup_text(
    base: &str,
    doc: &str,
    range: &Range,
    highlighter: Option<Box<dyn Highlighter>>,
) -> PopupText {
    let all: Vec<&str> = split_lines(base).collect();
    let v1 = (range.vcs_line1 as usize).min(all.len());
    let v2 = (range.vcs_line2 as usize).min(all.len()).max(v1);
    if v1 == v2 {
        return PopupText::default();
    }
    let text = all[v1..v2].join("\n");
    let mut spans = vec![Vec::new(); v2 - v1];
    if let Some(mut h) = highlighter {
        let buffer = Buffer::new(base);
        h.reset(&buffer);
        h.prepare(&buffer, v1..v2);
        for (i, line) in spans.iter_mut().enumerate() {
            h.line_spans(v1 + i, line);
        }
    }
    let words = if range.line1 < range.line2 {
        word_diff(&text, doc)
    } else {
        Vec::new()
    };
    PopupText { text, spans, words }
}

/// Char offset of byte `byte` of `text`.
fn chars_to(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

/// `ColorUtil.mix(fg, bg, share)`: `share` of `bg`, the rest of `fg`.
fn mix(fg: Hsla, bg: Hsla, share: f32) -> Hsla {
    let (f, b): (Rgba, Rgba) = (fg.into(), bg.into());
    let m = |x: f32, y: f32| x * (1.0 - share) + y * share;
    Rgba {
        r: m(f.r, b.r),
        g: m(f.g, b.g),
        b: m(f.b, b.b),
        a: m(f.a, b.a),
    }
    .into()
}

/// The highlights of the current lines of a modified range while its
/// popup shows (`installMasterEditorWordHighlighters`): the lines in the
/// changed colour mixed with the editor's background (the "ignored"
/// colour of a modification), changed and inserted words, a bar where
/// words were deleted. `doc` is the current text of the lines, starting at
/// char `doc_start` of the editor's text. `None` without word differences.
pub fn doc_highlights(
    popup: &PopupText,
    doc: &str,
    doc_start: usize,
    range: &Range,
    cx: &App,
) -> Option<DiffHighlights> {
    if popup.words.is_empty() {
        return None;
    }
    let theme = cx.theme();
    let changed = theme.color(UiRole::DiffChanged);
    let words = popup
        .words
        .iter()
        .map(|w| {
            let color = theme.color(match w.kind() {
                WordKind::Inserted => UiRole::DiffAdded,
                WordKind::Deleted => UiRole::DiffRemoved,
                WordKind::Modified => UiRole::DiffChanged,
            });
            let range =
                doc_start + chars_to(doc, w.doc.start)..doc_start + chars_to(doc, w.doc.end);
            (range, color)
        })
        .collect();
    Some(DiffHighlights {
        lines: range.line1 as usize..range.line2 as usize,
        line_background: Some(mix(
            changed,
            theme.color(UiRole::EditorBackground),
            LINE_BACKGROUND_SHARE,
        )),
        words,
    })
}

/// The highlights of the popup's own text: changed and deleted words, a
/// bar where words were inserted (`installPopupEditorWordHighlighters`).
fn base_highlights(popup: &PopupText, cx: &App) -> DiffHighlights {
    let theme = cx.theme();
    let words = popup
        .words
        .iter()
        .map(|w| {
            let color = theme.color(match w.kind() {
                WordKind::Inserted => UiRole::DiffAdded,
                WordKind::Deleted => UiRole::DiffRemoved,
                WordKind::Modified => UiRole::DiffChanged,
            });
            let text = &popup.text;
            (
                chars_to(text, w.base.start)..chars_to(text, w.base.end),
                color,
            )
        })
        .collect();
    DiffHighlights {
        words,
        ..DiffHighlights::default()
    }
}

/// The base text's spans, worked out once (the popup's text never
/// changes).
struct BaseSpans(Vec<Vec<Span>>);

impl Highlighter for BaseSpans {
    fn reset(&mut self, _: &Buffer) {}

    fn edited(&mut self, _: &Buffer, _: &[AppliedEdit]) {}

    fn line_spans(&self, line: usize, out: &mut Vec<Span>) {
        if let Some(spans) = self.0.get(line) {
            out.extend(spans.iter().cloned());
        }
    }
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

/// The texts of the disabled parts of the toolbar row: the changelists
/// group (`ex.changelists`), the commit field's placeholder
/// (`commit.from.gutter.placeholder`) and its buttons (`checkbox.amend`).
pub const CHANGELISTS: &str = "Changelists";
pub const COMMIT_PLACEHOLDER: &str = "Commit this change";
const AMEND: &str = "Amend";

/// Width of the toolbar row with its borders: six buttons, the commit
/// panel and the insets.
pub fn toolbar_width() -> Pixels {
    let buttons = SMALL_BUTTON * 6.0 + BUTTON_GAP * 5.0 + TOOLBAR_END;
    px(1.0 + ROW_LEFT + buttons + COMMIT_PANEL + ROW_RIGHT + 1.0)
}

/// Width of the popup's text area with its borders for old text whose
/// widest line is `text_width` wide: the text with its insets and the room
/// after it, at least as wide as the toolbar row (the text is the panel's
/// centre, the toolbar sits at its top left), at most `max`.
pub fn text_area_width(text_width: Pixels, max: Pixels) -> Pixels {
    (text_width + px(2.0 * TEXT_OFFSET + TEXT_END_ROOM))
        .max(toolbar_width())
        .min(max)
}

/// Height of the text area with its borders for `lines` lines, at most
/// `max` (at least one line).
pub fn text_area_height(lines: usize, line_height: Pixels, max: Option<Pixels>) -> Pixels {
    let full = line_height * lines as f32 + px(2.0 * TEXT_OFFSET);
    match max {
        Some(max) => full.min(max.max(line_height + px(2.0 * TEXT_OFFSET))),
        None => full,
    }
}

/// Width of the widest line of `text` in the editor font, tabs as
/// `tab_size` columns.
fn widest_line(text: &str, tab_size: usize, window: &Window, cx: &App) -> Pixels {
    let editor_font = cx.theme().editor_font();
    let font = editor_font.font();
    let ts = window.text_system();
    text.split('\n')
        .map(|line| {
            let mut shown = String::with_capacity(line.len());
            let mut col = 0;
            for c in line.chars() {
                if c == '\t' {
                    let n = tab_size.max(1) - col % tab_size.max(1);
                    shown.extend(std::iter::repeat_n(' ', n));
                    col += n;
                } else {
                    shown.push(c);
                    col += 1;
                }
            }
            if shown.is_empty() {
                return px(0.0);
            }
            let run = TextRun {
                len: shown.len(),
                font: font.clone(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            ts.shape_line(shown.into(), editor_font.size, &[run], None)
                .width
        })
        .fold(px(0.0), |a, b| a.max(b))
}

/// The popup of one range.
pub struct MarkerPopup {
    editor: Entity<Editor>,
    /// The range as it was when the popup opened (its identity).
    pub range: Range,
    /// The popup goes under this document line.
    pub line: usize,
    text: PopupText,
    /// The old text: a read-only editor, when the range has old lines.
    view: Option<Entity<Editor>>,
    /// Width of the widest old line.
    text_width: Pixels,
    enabled: [bool; 5],
    /// Room from the popup's top to the window's bottom, as last placed:
    /// the old text is cut there (`getPreferredSize`).
    room: std::cell::Cell<Option<Pixels>>,
}

impl EventEmitter<PopupAction> for MarkerPopup {}

impl MarkerPopup {
    /// A popup of `range` under document `line` with the range's old text;
    /// `enabled` says which actions can run (Previous, Next, Rollback,
    /// Show Diff, Copy, in that order).
    pub fn new(
        editor: Entity<Editor>,
        range: Range,
        line: usize,
        text: PopupText,
        enabled: [bool; 5],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tab_size = editor.read(cx).config().tab_size;
        let (view, text_width) = if text.is_empty() {
            (None, px(0.0))
        } else {
            let highlights = base_highlights(&text, cx);
            let spans = BaseSpans(text.spans.clone());
            let view = cx.new(|cx| {
                let mut buffer = Buffer::new(&text.text);
                buffer.set_read_only(true);
                let mut e = Editor::new(buffer, window, cx);
                e.set_single_line(true);
                e.set_transparent(true);
                e.set_viewer(true);
                e.set_config(editor::EditorConfig {
                    tab_size,
                    ..editor::EditorConfig::default()
                });
                e.set_highlighter(Some(Box::new(spans)), cx);
                e.set_diff_highlights(Some(highlights), cx);
                e
            });
            (Some(view), widest_line(&text.text, tab_size, window, cx))
        };
        Self {
            editor,
            range,
            line,
            text,
            view,
            text_width,
            enabled,
            room: std::cell::Cell::new(None),
        }
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    pub fn text(&self) -> &PopupText {
        &self.text
    }

    /// The read-only editor of the old text.
    pub fn view(&self) -> Option<&Entity<Editor>> {
        self.view.as_ref()
    }

    pub fn is_enabled(&self, action: PopupAction) -> bool {
        let i = PopupAction::ALL.iter().position(|a| *a == action);
        i.is_some_and(|i| self.enabled[i])
    }

    /// The popup's state as JSON (`workbench.state`): the old lines and
    /// per line the word differences in them (an inserted word is empty,
    /// at its column).
    pub fn state_json(&self, cx: &App) -> serde_json::Value {
        let text = &self.text.text;
        let mut starts = Vec::new();
        let mut at = 0;
        for l in self.text.lines() {
            starts.push((at, at + l.len()));
            at += l.len() + 1;
        }
        let words: Vec<serde_json::Value> = starts
            .iter()
            .map(|&(s, e)| {
                // As drawn: the parts of words on the line, an empty word
                // as its column.
                let line = self.text.words.iter().filter_map(|w| {
                    let r = &w.base;
                    let (a, b) = (r.start.max(s), r.end.min(e));
                    let drawn = if r.is_empty() {
                        (s..=e).contains(&r.start)
                    } else {
                        a < b
                    };
                    drawn.then(|| {
                        serde_json::json!({
                            "text": &text[a..b.max(a)],
                            "column": text[s..a].chars().count(),
                            "kind": w.kind().name(),
                        })
                    })
                });
                serde_json::Value::Array(line.collect())
            })
            .collect();
        let selected = self.view.as_ref().map(|v| {
            let e = v.read(cx);
            let s = e.selections().primary();
            e.buffer().slice(s.start()..s.end())
        });
        serde_json::json!({
            "lines": [self.range.line1, self.range.line2],
            "vcs_lines": [self.range.vcs_line1, self.range.vcs_line2],
            "under_line": self.line,
            "actions": PopupAction::ALL.iter().zip(self.enabled).map(|(a, on)| serde_json::json!({
                "title": a.title(),
                "enabled": on,
            })).collect::<Vec<_>>(),
            "text": self.text.lines().collect::<Vec<_>>(),
            "words": words,
            "selected": selected,
            "toolbar_width": f32::from(toolbar_width()),
            "commit_field": { "placeholder": COMMIT_PLACEHOLDER, "enabled": false },
        })
    }

    fn render_toolbar(&self, has_text: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let border = theme.color(UiRole::MarkerPopupBorder);
        let focus = self.editor.focus_handle(cx);
        let mut buttons: Vec<AnyElement> = PopupAction::ALL
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
        // The changelists group, a popup group with the corner arrow
        // (`ActionButtonLook.paintDownArrow`: the arrow icon 1 px right of
        // and below the icon).
        buttons.push(
            small_icon_button("marker-popup-changelists", IconName::CHANGELIST, false, cx)
                .debug_selector(|| "marker-popup-changelists".into())
                .relative()
                .tooltip(Tooltip::text(CHANGELISTS, None))
                .child(
                    div()
                        .absolute()
                        .left(px((SMALL_BUTTON - 16.0) / 2.0 + 1.0))
                        .top(px((SMALL_BUTTON - 16.0) / 2.0 + 1.0))
                        .child(Icon::new(IconName::DROPDOWN_CORNER).size(px(16.0))),
                )
                .into_any_element(),
        );
        let commit = div()
            .flex()
            .flex_row()
            .flex_none()
            .w(px(COMMIT_PANEL))
            .pl(px(COMMIT_GAP + COMMIT_PADDING))
            .pr(px(COMMIT_PADDING))
            .child(self.render_commit_field(cx));
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(ROW_HEIGHT + if has_text { 1.0 } else { 2.0 }))
            .pl(px(ROW_LEFT))
            .pr(px(ROW_RIGHT))
            .bg(theme.color(UiRole::MarkerPopupToolbar))
            .border_color(border)
            .border_t_1()
            .border_l_1()
            .border_r_1()
            .when(!has_text, |d| d.border_b_1())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .gap(px(BUTTON_GAP))
                    .pr(px(TOOLBAR_END))
                    .children(buttons),
            )
            .child(commit)
    }

    /// The "Commit this change" field, disabled: committing one change
    /// comes with partial commits.
    fn render_commit_field(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (sep_height, sep_gap) = COMMIT_SEPARATOR;
        input_frame(false, false, cx)
            .debug_selector(|| "marker-popup-commit-field".into())
            .flex_1()
            .h(px(COMMIT_FIELD_HEIGHT))
            .bg(theme.color(UiRole::EditorBackground))
            .pl(px(COMMIT_TEXT_LEFT))
            .pr(px(COMMIT_INSET - 1.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .font_family(theme.mono_family())
                    .text_size(theme.editor_size())
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(COMMIT_PLACEHOLDER),
            )
            .child(
                small_icon_button("marker-popup-amend", IconName::AMEND_INLINE, false, cx)
                    .tooltip(Tooltip::text(AMEND, None)),
            )
            .child(
                div()
                    .flex_none()
                    .mx(px(sep_gap))
                    .w(px(1.0))
                    .h(px(sep_height))
                    .bg(theme.color(UiRole::Divider)),
            )
            .child(
                small_icon_button("marker-popup-commit", IconName::COMMIT_INLINE, false, cx)
                    .tooltip(Tooltip::text(COMMIT_PLACEHOLDER, None)),
            )
            .into_any_element()
    }
}

impl Render for MarkerPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_height = self
            .editor
            .read(cx)
            .text_geometry()
            .map(|(_, lh, _)| lh)
            .unwrap_or(px(20.0));
        let toolbar = self.render_toolbar(self.view.is_some(), window, cx);
        let theme = cx.theme();
        let max_width = window.viewport_size().width - px(WINDOW_GAP);
        let max_height = self.room.get().map(|room| room - px(TOOLBAR_HEIGHT));
        let text = self.view.as_ref().map(|view| {
            let lines = self.text.spans.len();
            div()
                .id("marker-popup-text")
                .debug_selector(|| "marker-popup-text".into())
                .flex_none()
                .w(text_area_width(self.text_width, max_width))
                .h(text_area_height(lines, line_height, max_height))
                .p(px(TEXT_OFFSET - 1.0))
                .border_1()
                .border_color(theme.color(UiRole::MarkerPopupBorder))
                .bg(theme.color(UiRole::MarkerPopupText))
                .overflow_hidden()
                .child(view.clone())
        });
        div()
            .id("marker-popup")
            .debug_selector(|| "marker-popup".into())
            .key_context(CONTEXT)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The fragments as (base text, current text, kind).
    fn diff(base: &str, doc: &str) -> Vec<(String, String, WordKind)> {
        word_diff(base, doc)
            .into_iter()
            .map(|w| {
                let kind = w.kind();
                (base[w.base].to_owned(), doc[w.doc].to_owned(), kind)
            })
            .collect()
    }

    #[test]
    fn word_differences_of_a_range() {
        assert_eq!(
            diff("value = compute(1)", "value = compute(2)"),
            vec![("1".into(), "2".into(), WordKind::Modified)]
        );
        assert_eq!(
            diff("a = b + c", "a = b"),
            vec![(" + c".into(), String::new(), WordKind::Deleted)]
        );
        assert_eq!(
            diff("x", "x y"),
            vec![(String::new(), " y".into(), WordKind::Inserted)]
        );
        // A word grown by letters of another alphabet is a changed word;
        // text typed after a bracket is an insertion at the bracket's end.
        assert_eq!(
            diff("IPS = s.IPS\nrun(", "IPS = s.IPSавава\nrun(вава"),
            vec![
                ("IPS".into(), "IPSавава".into(), WordKind::Modified),
                (String::new(), "вава".into(), WordKind::Inserted),
            ]
        );
        let inserted = word_diff("run(\n", "run(x\n");
        assert_eq!(inserted[0].base, 4..4, "the bar goes after the bracket");
        // Line breaks are words of their own: an emptied line is not glued
        // to the indentation of the next one.
        assert_eq!(
            diff("a\n\n    b", "a\n    b"),
            vec![("\n".into(), String::new(), WordKind::Deleted)]
        );
    }

    #[test]
    fn large_ranges_have_no_word_differences() {
        let big = "x ".repeat(WORD_DIFF_LIMIT);
        assert!(word_diff(&big, "y").is_empty());
    }

    #[test]
    fn popup_text_keeps_empty_lines() {
        let base = "a\nfirst = 1\n\n\nlast = 2\nz\n";
        // Lines 1..5 of the base became one line.
        let range = Range::new(1, 2, 1, 5);
        let p = popup_text(base, "first = 1", &range, None);
        assert_eq!(p.text, "first = 1\n\n\nlast = 2");
        assert_eq!(
            p.lines().collect::<Vec<_>>(),
            vec!["first = 1", "", "", "last = 2"]
        );
        assert_eq!(p.spans.len(), 4);
        let kinds: Vec<WordKind> = p.words.iter().map(WordFragment::kind).collect();
        assert_eq!(kinds, vec![WordKind::Deleted]);
        assert_eq!(&p.text[p.words[0].base.clone()], "\n\n\nlast = 2");
        // Added lines have no old text, deleted ones no word differences.
        assert!(popup_text(base, "x", &Range::new(1, 2, 1, 1), None).is_empty());
        let deleted = popup_text(base, "", &Range::new(1, 1, 1, 2), None);
        assert_eq!(deleted.text, "first = 1");
        assert!(deleted.words.is_empty());
    }

    #[gpui::test]
    fn the_editor_gets_char_ranges_of_the_current_lines(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            ui::theme::init(cx, themes::ThemeKind::Dark);
            let base = "x = 1\nuvicorn.run(\n";
            let doc = "x = 1\nuvicorn.run(вава";
            let range = Range::new(0, 2, 0, 2);
            let p = popup_text(base, doc, &range, None);
            // The current lines start at char 10 of the editor's text.
            let h = doc_highlights(&p, doc, 10, &range, cx).expect("word differences");
            assert_eq!(h.lines, 0..2);
            let theme = cx.theme();
            assert_eq!(
                h.words,
                vec![(28..32, theme.color(UiRole::DiffAdded))],
                "chars, not bytes"
            );
            // The lines: the changed colour, 60 % of it the background's.
            let line = h.line_background.unwrap();
            let expected = mix(
                theme.color(UiRole::DiffChanged),
                theme.color(UiRole::EditorBackground),
                0.6,
            );
            assert_eq!(line, expected);
            // The popup's own text: a bar at the end of the bracket.
            let own = base_highlights(&p, cx);
            assert_eq!(own.words.len(), 1);
            assert_eq!(own.words[0].0, 18..18);
        });
    }

    #[gpui::test]
    fn the_old_text_is_a_read_only_editor_with_its_empty_lines(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| ui::theme::init(cx, themes::ThemeKind::Dark));
        let base = "first = 1\n\nthird = 3\nkeep\n";
        // The three first lines became one.
        let range = Range::new(0, 1, 0, 3);
        let window = cx.add_window(|window, cx| {
            let main = editor::new_editor("first = 10\nkeep\n", window, cx);
            let text = popup_text(base, "first = 10", &range, None);
            MarkerPopup::new(main, range, 0, text, [true; 5], window, cx)
        });
        let state = window
            .update(cx, |popup, _, cx| {
                let view = popup.view().expect("old text").clone();
                view.update(cx, |e, cx| {
                    assert!(e.is_viewer());
                    assert!(e.buffer().is_read_only());
                    assert_eq!(e.text(), "first = 1\n\nthird = 3");
                    // A selection over the empty line (as the mouse makes it).
                    let selection = text::Selection {
                        anchor: 4,
                        head: 16,
                        goal: None,
                    };
                    e.set_selections(text::SelectionSet::single(selection), cx);
                });
                popup.state_json(cx)
            })
            .unwrap();
        assert_eq!(
            state["text"],
            serde_json::json!(["first = 1", "", "third = 3"])
        );
        assert_eq!(state["selected"], "t = 1\n\nthird");
        // One changed stretch over the three lines, nothing drawn on the
        // empty one.
        assert_eq!(
            state["words"],
            serde_json::json!([
                [{"text": "1", "column": 8, "kind": "modified"}],
                [],
                [{"text": "third = 3", "column": 0, "kind": "modified"}],
            ])
        );
    }

    #[test]
    fn mixing_takes_the_share_of_the_background() {
        let fg: Hsla = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
        .into();
        let bg: Hsla = Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }
        .into();
        let m: Rgba = mix(fg, bg, 0.6).into();
        assert!(
            (m.r - 0.4).abs() < 1e-3 && (m.b - 0.6).abs() < 1e-3,
            "{m:?}"
        );
    }

    #[test]
    fn the_toolbar_row_has_the_reference_width() {
        // Measured on the reference: 429 px inside its 1 px borders.
        assert_eq!(toolbar_width(), px(431.0));
        // Short old text: the text area is as wide as the toolbar row.
        assert_eq!(text_area_width(px(80.0), px(2000.0)), px(431.0));
        // Long old text: the text, its insets and the room after it.
        assert_eq!(
            text_area_width(px(901.0), px(2000.0)),
            px(901.0 + 6.0 + 22.0)
        );
        // Never wider than the window allows.
        assert_eq!(text_area_width(px(3000.0), px(1200.0)), px(1200.0));
    }

    #[test]
    fn the_text_area_is_cut_at_the_window() {
        let lh = px(24.0);
        assert_eq!(text_area_height(6, lh, None), px(150.0));
        assert_eq!(text_area_height(6, lh, Some(px(100.0))), px(100.0));
        // At least one line.
        assert_eq!(text_area_height(6, lh, Some(px(5.0))), px(30.0));
    }
}
