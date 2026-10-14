//! Problems shown in an editor: the editor's own marks, so that it knows
//! no language service. Whoever has diagnostics (the code intelligence
//! controller) turns them into [`DiagnosticMark`]s.
//!
//! The marks behave as the reference IDE's highlighters of problems: an
//! empty range at a line end is drawn after the end of the line, any other
//! empty range covers the next char; the marks move with the text as its
//! range markers do; each is drawn with the editor colour scheme's
//! attributes of its kind, the most severe on top.

use std::ops::Range;

use gpui::{Hsla, SharedString};
use text::{AppliedEdit, Buffer};
use themes::{TextRole, TextStyle, UnderlineStyle as ThemeUnderline};
use ui::theme::{Theme, color};

/// How a mark is drawn, from the most severe (the reference's highlight
/// levels: error, warning, weak warning, information).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarkSeverity {
    /// A wave in the error colour; counted in the inspection widget.
    Error,
    /// A wave in the warning colour; counted.
    Warning,
    /// A wave in the weak warning colour; counted.
    WeakWarning,
    /// Not counted; drawn as the theme's information (hint) attributes,
    /// which the reference's default colour schemes leave empty, so
    /// usually not drawn at all. Its messages show in the tooltip.
    Information,
}

impl MarkSeverity {
    /// Name in `editor.state`.
    pub fn name(self) -> &'static str {
        match self {
            MarkSeverity::Error => "error",
            MarkSeverity::Warning => "warning",
            MarkSeverity::WeakWarning => "weak_warning",
            MarkSeverity::Information => "information",
        }
    }

    /// Paint order: marks of a higher layer are drawn over lower ones (the
    /// reference's highlighter layers ERROR > WARNING > WEAK_WARNING >
    /// ADDITIONAL_SYNTAX).
    fn layer(self) -> u8 {
        match self {
            MarkSeverity::Error => 3,
            MarkSeverity::Warning => 2,
            MarkSeverity::WeakWarning => 1,
            MarkSeverity::Information => 0,
        }
    }
}

/// One message of a mark (marks of several services at one range carry
/// all their messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkMessage {
    pub text: SharedString,
    /// Who reports it (`Ruff`), shown grey after the text.
    pub source: Option<SharedString>,
    /// The rule (`unused-import`), shown in brackets after the source.
    pub code: Option<SharedString>,
    /// Documentation of the rule: the code is a link to it.
    pub href: Option<SharedString>,
}

/// A problem in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticMark {
    /// Char range in the text. An empty range at a line end is drawn after
    /// the end of the line; any other empty range becomes the char after it
    /// when the marks are set.
    pub range: Range<usize>,
    pub severity: MarkSeverity,
    /// Greyed like unused code, without the severity's wave.
    pub unnecessary: bool,
    /// Struck out, without the severity's wave.
    pub deprecated: bool,
    pub messages: Vec<MarkMessage>,
}

impl DiagnosticMark {
    /// The theme role the mark is drawn with. Unnecessary and deprecated
    /// code looks the same at every severity (the reference's highlight
    /// types "like unused symbol", checked first, and "like deprecated");
    /// the others look like their severity.
    pub(crate) fn role(&self) -> TextRole {
        if self.unnecessary {
            TextRole::DiagnosticUnused
        } else if self.deprecated {
            TextRole::DiagnosticDeprecated
        } else {
            match self.severity {
                MarkSeverity::Error => TextRole::DiagnosticError,
                MarkSeverity::Warning => TextRole::DiagnosticWarning,
                MarkSeverity::WeakWarning => TextRole::DiagnosticInfo,
                MarkSeverity::Information => TextRole::DiagnosticHint,
            }
        }
    }
}

/// Moving more marks through more edits than this (marks times edits)
/// drops the marks instead: the services report them again for the new
/// text, and a big batch (reformatting a large file) stays cheap.
const MAX_MOVES: usize = 1 << 22;

/// The marks of an editor, sorted by range and moved through edits.
#[derive(Debug, Default)]
pub(crate) struct Marks {
    list: Vec<DiagnosticMark>,
    /// Length of the longest mark, to find the marks over a range without
    /// looking at all of them.
    longest: usize,
}

impl Marks {
    pub fn as_slice(&self) -> &[DiagnosticMark] {
        &self.list
    }

    /// Replaces the marks with `marks` in the current text of `buffer`:
    /// ranges are clamped to the text, and an empty range that is not at a
    /// line end covers the char after it, as the reference makes "something
    /// visible" of an empty problem. Returns whether anything changed.
    pub fn set(&mut self, marks: Vec<DiagnosticMark>, buffer: &Buffer) -> bool {
        let len = buffer.len_chars();
        let mut marks = marks;
        for m in &mut marks {
            let end = m.range.end.min(len);
            let start = m.range.start.min(end);
            let end = if start == end && buffer.char_at(start).is_some_and(|c| c != '\n') {
                end + 1
            } else {
                end
            };
            m.range = start..end;
        }
        marks.sort_by_key(|m| (m.range.start, m.range.end));
        if marks == self.list {
            return false;
        }
        self.list = marks;
        self.update_longest();
        true
    }

    /// Removes every mark; returns whether there were any.
    pub fn clear(&mut self) -> bool {
        let had = !self.list.is_empty();
        self.list.clear();
        self.longest = 0;
        had
    }

    fn update_longest(&mut self) {
        self.longest = self.list.iter().map(|m| m.range.len()).max().unwrap_or(0);
    }

    /// Moves the marks through applied edits (in the order they were
    /// applied). A mark whose whole text is replaced by a larger edit is
    /// dropped; see [`move_range`].
    pub fn edited(&mut self, edits: &[AppliedEdit]) {
        if self.list.is_empty() || edits.is_empty() {
            return;
        }
        if edits.len().saturating_mul(self.list.len()) > MAX_MOVES {
            self.clear();
            return;
        }
        for e in edits {
            self.list
                .retain_mut(|m| match move_range(m.range.clone(), e) {
                    Some(r) => {
                        m.range = r;
                        true
                    }
                    None => false,
                });
        }
        // A replaced prefix can move a start past the next mark's.
        if !self.list.is_sorted_by_key(|m| (m.range.start, m.range.end)) {
            self.list.sort_by_key(|m| (m.range.start, m.range.end));
        }
        self.update_longest();
    }

    /// Indices of the marks that may touch the chars `range` (a mark ending
    /// at its start or starting at its end included; callers filter).
    pub fn near(&self, range: Range<usize>) -> Range<usize> {
        let from = range.start.saturating_sub(self.longest);
        let first = self.list.partition_point(|m| m.range.start < from);
        let last = self.list.partition_point(|m| m.range.start <= range.end);
        first..last.max(first)
    }

    pub fn get(&self, index: usize) -> &DiagnosticMark {
        &self.list[index]
    }
}

/// Where a mark's char range goes through one applied edit, as the
/// reference moves a range marker that grows neither to the left nor to the
/// right: text inserted at either end stays outside, text inserted inside
/// widens it, a replaced prefix or suffix is cut off, and a mark whose
/// whole text is replaced by a larger edit is gone (`None`). An empty mark
/// stays where it is when text is inserted there and is gone when the text
/// around it is deleted.
pub(crate) fn move_range(range: Range<usize>, e: &AppliedEdit) -> Option<Range<usize>> {
    let (start, end) = (range.start, range.end);
    let (offset, old_end) = (e.start, e.old_end);
    let shift = |p: usize| p + e.new_end - old_end;
    if start == end {
        if offset < start && start < old_end {
            return None;
        }
        if start > old_end || (start == old_end && old_end > offset) {
            return Some(shift(start)..shift(start));
        }
        return Some(range);
    }
    // After the end, or an insertion at the end.
    if offset >= end {
        return Some(range);
    }
    // Before the start, or ending at the start.
    if start >= old_end {
        return Some(shift(start)..shift(end));
    }
    // Inside: widens or narrows.
    if start <= offset && old_end <= end {
        return Some(start..shift(end));
    }
    // A replaced prefix.
    if start >= offset && end > old_end {
        return Some(e.new_end..shift(end));
    }
    // A replaced suffix.
    if start < offset && end <= old_end {
        return Some(start..offset);
    }
    None
}

/// How decorations draw a stretch of text: the merged attributes of the
/// marks over it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Look {
    pub fg: Option<Hsla>,
    pub bg: Option<Hsla>,
    pub bold: bool,
    pub italic: bool,
    /// An underline, or `Some` of [`ThemeUnderline::None`] to remove one.
    pub underline: Option<Underline>,
    pub strikethrough: bool,
}

/// An underline of a [`Look`]; its colour is the text colour when unset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Underline {
    pub kind: ThemeUnderline,
    pub color: Option<Hsla>,
}

impl Look {
    /// Draws `style` over this look: what it sets wins.
    fn overlay(&mut self, style: &TextStyle) {
        if let Some(fg) = style.fg {
            self.fg = Some(color(fg));
        }
        if let Some(bg) = style.bg {
            self.bg = Some(color(bg));
        }
        if let Some(bold) = style.bold {
            self.bold = bold;
        }
        if let Some(italic) = style.italic {
            self.italic = italic;
        }
        if let Some(kind) = style.underline {
            self.underline = Some(Underline {
                kind,
                color: style.underline_colour().map(color),
            });
        }
        if let Some(strike) = style.strikethrough {
            self.strikethrough = strike;
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Look::default()
    }

    /// The colour of a dotted underline of this look over text of colour
    /// `text`.
    pub fn dotted(&self, text: Hsla) -> Option<Hsla> {
        let u = self.underline?;
        (u.kind == ThemeUnderline::Dotted).then(|| u.color.or(self.fg).unwrap_or(text))
    }
}

/// The decorations of one line from the marks over it: char column ranges
/// of the line, each with the look of the marks covering it (the most
/// severe drawn last, so on top), and the look of the empty marks on the
/// line, drawn after its end.
pub(crate) struct LineLooks {
    pub ranges: Vec<(Range<usize>, Look)>,
    pub after_end: Option<Look>,
}

/// Builds the decorations of the line `line_start..line_start + line_len`
/// (chars) from marks that may touch it.
pub(crate) fn line_looks<'a>(
    marks: impl IntoIterator<Item = &'a DiagnosticMark>,
    line_start: usize,
    line_len: usize,
    theme: &Theme,
) -> LineLooks {
    let line_end = line_start + line_len;
    // (column range, layer, role) of the marks over the line's chars.
    let mut parts: Vec<(Range<usize>, u8, TextRole)> = Vec::new();
    let mut after_end: Vec<(u8, TextRole)> = Vec::new();
    for m in marks {
        let (start, end) = (m.range.start, m.range.end);
        if start == end {
            if (line_start..=line_end).contains(&start) {
                after_end.push((m.severity.layer(), m.role()));
            }
            continue;
        }
        if end <= line_start || start >= line_end {
            continue;
        }
        let cols = start.max(line_start) - line_start..end.min(line_end) - line_start;
        parts.push((cols, m.severity.layer(), m.role()));
    }
    let merged = |mut layers: Vec<(u8, TextRole)>| -> Look {
        // Stable: equal layers keep the marks' order.
        layers.sort_by_key(|(layer, _)| *layer);
        let mut look = Look::default();
        for (_, role) in layers {
            look.overlay(theme.text_style(role));
        }
        look
    };
    let mut ranges: Vec<(Range<usize>, Look)> = Vec::new();
    if !parts.is_empty() {
        let mut bounds: Vec<usize> = parts
            .iter()
            .flat_map(|(r, _, _)| [r.start, r.end])
            .collect();
        bounds.sort_unstable();
        bounds.dedup();
        for w in bounds.windows(2) {
            let (a, b) = (w[0], w[1]);
            let covering: Vec<(u8, TextRole)> = parts
                .iter()
                .filter(|(r, _, _)| r.start <= a && b <= r.end)
                .map(|(_, layer, role)| (*layer, *role))
                .collect();
            if covering.is_empty() {
                continue;
            }
            let look = merged(covering);
            if look.is_empty() {
                continue;
            }
            match ranges.last_mut() {
                Some((r, l)) if r.end == a && *l == look => r.end = b,
                _ => ranges.push((a..b, look)),
            }
        }
    }
    let after_end = (!after_end.is_empty())
        .then(|| merged(after_end))
        .filter(|l| !l.is_empty());
    LineLooks { ranges, after_end }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(range: Range<usize>, severity: MarkSeverity) -> DiagnosticMark {
        DiagnosticMark {
            range,
            severity,
            unnecessary: false,
            deprecated: false,
            messages: Vec::new(),
        }
    }

    /// The edit that replaces `start..old_end` with `new_len` chars.
    fn edit(start: usize, old_end: usize, new_len: usize) -> AppliedEdit {
        AppliedEdit {
            start,
            old_end,
            new_end: start + new_len,
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: start + new_len,
            start_position: (0, start),
            old_end_position: (0, old_end),
            new_end_position: (0, start + new_len),
            start_utf16_col: start,
            old_end_utf16_col: old_end,
            text: "x".repeat(new_len).into(),
        }
    }

    #[test]
    fn ranges_move_as_the_reference_moves_range_markers() {
        let m = |e: AppliedEdit| move_range(10..20, &e);
        // After the end, and an insertion at the end: unchanged.
        assert_eq!(m(edit(25, 26, 0)), Some(10..20));
        assert_eq!(m(edit(20, 20, 3)), Some(10..20));
        // Before the start, and an insertion at the start: shifted.
        assert_eq!(m(edit(2, 4, 7)), Some(15..25));
        assert_eq!(m(edit(10, 10, 2)), Some(12..22));
        assert_eq!(m(edit(5, 10, 0)), Some(5..15));
        // Inside: typing widens it, deleting narrows it.
        assert_eq!(m(edit(15, 15, 1)), Some(10..21));
        assert_eq!(m(edit(12, 18, 0)), Some(10..14));
        // Exactly its text replaced: kept over the new text.
        assert_eq!(m(edit(10, 20, 3)), Some(10..13));
        // A replaced prefix or suffix is cut off.
        assert_eq!(m(edit(5, 12, 1)), Some(6..14));
        assert_eq!(m(edit(18, 25, 1)), Some(10..18));
        // All of it inside a larger replacement: gone.
        assert_eq!(m(edit(5, 25, 3)), None);
        assert_eq!(m(edit(10, 21, 0)), None);
    }

    #[test]
    fn empty_ranges_move_as_points() {
        let m = |e: AppliedEdit| move_range(10..10, &e);
        assert_eq!(m(edit(10, 10, 4)), Some(10..10));
        assert_eq!(m(edit(2, 3, 0)), Some(9..9));
        assert_eq!(m(edit(8, 10, 0)), Some(8..8));
        assert_eq!(m(edit(10, 12, 0)), Some(10..10));
        assert_eq!(m(edit(8, 12, 1)), None);
        assert_eq!(m(edit(11, 12, 1)), Some(10..10));
    }

    #[test]
    fn setting_marks_clamps_sorts_and_widens_empty_ones() {
        let buffer = Buffer::new("import os\nx\n");
        let mut marks = Marks::default();
        assert!(marks.set(
            vec![
                mark(10..10, MarkSeverity::Error),
                mark(9..9, MarkSeverity::Warning),
                mark(0..6, MarkSeverity::Warning),
                mark(11..40, MarkSeverity::Error),
                mark(50..60, MarkSeverity::Error),
            ],
            &buffer,
        ));
        let ranges: Vec<_> = marks.as_slice().iter().map(|m| m.range.clone()).collect();
        // `9..9` is at the end of line 0 and stays empty (drawn after the
        // line); `10..10` covers `x`; the rest is clamped to the text.
        assert_eq!(ranges, [0..6, 9..9, 10..11, 11..12, 12..12]);
        assert!(!marks.set(marks.as_slice().to_vec(), &buffer));
        assert!(marks.clear());
        assert!(!marks.clear());
    }

    #[test]
    fn marks_move_through_edits_and_stay_sorted() {
        let buffer = Buffer::new("aaaa bbbb cccc\n");
        let mut marks = Marks::default();
        marks.set(
            vec![
                mark(0..4, MarkSeverity::Error),
                mark(5..9, MarkSeverity::Warning),
                mark(10..14, MarkSeverity::WeakWarning),
            ],
            &buffer,
        );
        // Replace "aaaa bb" with one char: the first mark is gone, the
        // second loses its prefix, the third moves.
        marks.edited(&[edit(0, 7, 1)]);
        let ranges: Vec<_> = marks.as_slice().iter().map(|m| m.range.clone()).collect();
        assert_eq!(ranges, [1..3, 4..8]);
        assert_eq!(marks.near(0..0), 0..0);
        assert_eq!(marks.near(2..2), 0..1);
        assert_eq!(marks.near(0..100), 0..2);
    }

    fn theme() -> Theme {
        Theme::new(themes::builtin::default_theme(themes::ThemeKind::Dark), &[])
    }

    fn style_color(theme: &Theme, role: TextRole) -> Hsla {
        let s = theme.text_style(role);
        color(s.underline_colour().expect("the theme colours the role"))
    }

    #[test]
    fn severities_are_drawn_with_their_theme_attributes() {
        let theme = theme();
        let marks = [
            mark(0..2, MarkSeverity::Error),
            mark(3..5, MarkSeverity::Warning),
            mark(6..8, MarkSeverity::WeakWarning),
            mark(9..11, MarkSeverity::Information),
        ];
        let looks = line_looks(&marks, 0, 12, &theme);
        let wave = |role| {
            Some(Underline {
                kind: ThemeUnderline::Wavy,
                color: Some(style_color(&theme, role)),
            })
        };
        // The default dark theme draws waves for the three counted levels
        // and nothing for information (the reference's empty attributes).
        assert_eq!(
            looks
                .ranges
                .iter()
                .map(|(r, l)| (r.clone(), l.underline))
                .collect::<Vec<_>>(),
            [
                (0..2, wave(TextRole::DiagnosticError)),
                (3..5, wave(TextRole::DiagnosticWarning)),
                (6..8, wave(TextRole::DiagnosticInfo)),
            ]
        );
        assert!(looks.after_end.is_none());
    }

    #[test]
    fn unused_code_is_greyed_and_deprecated_code_struck_out_without_a_wave() {
        let theme = theme();
        let unused = DiagnosticMark {
            unnecessary: true,
            deprecated: true,
            ..mark(0..9, MarkSeverity::Warning)
        };
        let deprecated = DiagnosticMark {
            deprecated: true,
            ..mark(10..16, MarkSeverity::Error)
        };
        let looks = line_looks([&unused, &deprecated], 0, 20, &theme);
        let grey = theme.text_style(TextRole::DiagnosticUnused).fg.map(color);
        assert!(grey.is_some());
        assert_eq!(
            looks.ranges,
            [
                (
                    0..9,
                    Look {
                        fg: grey,
                        ..Look::default()
                    }
                ),
                (
                    10..16,
                    Look {
                        strikethrough: true,
                        ..Look::default()
                    }
                ),
            ]
        );
    }

    #[test]
    fn the_most_severe_mark_is_on_top_and_marks_split_at_their_ends() {
        let theme = theme();
        // A warning over the whole line and an error in its middle, given
        // error first: the error's wave wins where both are.
        let marks = [
            mark(4..6, MarkSeverity::Error),
            mark(0..10, MarkSeverity::Warning),
        ];
        let looks = line_looks(&marks, 0, 10, &theme);
        let colors: Vec<_> = looks
            .ranges
            .iter()
            .map(|(r, l)| (r.clone(), l.underline.and_then(|u| u.color)))
            .collect();
        let warning = Some(style_color(&theme, TextRole::DiagnosticWarning));
        let error = Some(style_color(&theme, TextRole::DiagnosticError));
        assert_eq!(colors, [(0..4, warning), (4..6, error), (6..10, warning)]);
    }

    #[test]
    fn marks_over_several_lines_and_after_the_line_end() {
        let theme = theme();
        // Line 1 is chars 10..15 of the text.
        let marks = [
            mark(8..12, MarkSeverity::Error),
            mark(15..15, MarkSeverity::Error),
            mark(14..30, MarkSeverity::Warning),
            mark(0..10, MarkSeverity::Warning),
        ];
        let looks = line_looks(&marks, 10, 5, &theme);
        let ranges: Vec<_> = looks.ranges.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, [0..2, 4..5]);
        let after = looks.after_end.expect("the empty error at the line end");
        assert_eq!(after.underline.map(|u| u.kind), Some(ThemeUnderline::Wavy));
    }

    #[test]
    fn dotted_underlines_take_their_colour() {
        let look = Look {
            fg: Some(gpui::red()),
            underline: Some(Underline {
                kind: ThemeUnderline::Dotted,
                color: None,
            }),
            ..Look::default()
        };
        assert_eq!(look.dotted(gpui::black()), Some(gpui::red()));
        assert_eq!(Look::default().dotted(gpui::black()), None);
    }
}
