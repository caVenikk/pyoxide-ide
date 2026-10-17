//! The highlights of a changed range drawn over an editor's text, as the
//! reference's diff highlighters (`DiffDrawUtil.LineHighlighterBuilder`
//! and `InlineHighlighterBuilder`) are while the popup of a change marker
//! shows: a background over the whole width of the range's lines and the
//! changed words. An empty word (text inserted or deleted on the other
//! side) is a bar 2 px wide over its position, 1 px left of it
//! (`DiffEmptyHighlighterRenderer`).
//!
//! The highlights are positions in the current text: any change of the
//! text drops them.

use std::ops::Range;

use gpui::{Context, Hsla};

use crate::Editor;

/// Width of the bar of an empty word, and how far left of its position it
/// starts.
pub(crate) const EMPTY_WORD_WIDTH: f32 = 2.0;
pub(crate) const EMPTY_WORD_LEFT: f32 = 1.0;

/// What is drawn over an editor's text for a changed range.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffHighlights {
    /// Lines with [`DiffHighlights::line_background`] over the whole width
    /// of the text, over the caret row and under the selection.
    pub lines: Range<usize>,
    pub line_background: Option<Hsla>,
    /// Char ranges of the text and their backgrounds, under the selection;
    /// an empty range is a bar.
    pub words: Vec<(Range<usize>, Hsla)>,
}

impl DiffHighlights {
    /// The words that touch line `line` (chars `line_start..line_end`) as
    /// char columns of the line: non-empty ones clipped to it, empty ones
    /// as a column.
    pub(crate) fn words_of_line(
        &self,
        line_start: usize,
        line_end: usize,
    ) -> impl Iterator<Item = (Range<usize>, Hsla)> + '_ {
        self.words.iter().filter_map(move |(r, color)| {
            if r.is_empty() {
                return (line_start..=line_end)
                    .contains(&r.start)
                    .then(|| (r.start - line_start..r.start - line_start, *color));
            }
            let (a, b) = (r.start.max(line_start), r.end.min(line_end));
            (a < b).then(|| (a - line_start..b - line_start, *color))
        })
    }
}

impl Editor {
    /// Shows `highlights` over the text until they are replaced, removed
    /// or the text changes.
    pub fn set_diff_highlights(
        &mut self,
        highlights: Option<DiffHighlights>,
        cx: &mut Context<Self>,
    ) {
        if self.diff_highlights != highlights {
            self.diff_highlights = highlights;
            cx.notify();
        }
    }

    pub fn diff_highlights(&self) -> Option<&DiffHighlights> {
        self.diff_highlights.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_clipped_to_their_lines() {
        let c = gpui::black();
        let h = DiffHighlights {
            words: vec![(2..8, c), (10..10, c), (12..12, c)],
            ..DiffHighlights::default()
        };
        // Line "abcde" at 0..5, line "fghij" at 6..11.
        let first: Vec<_> = h.words_of_line(0, 5).map(|(r, _)| r).collect();
        assert_eq!(first, vec![2..5]);
        let second: Vec<_> = h.words_of_line(6, 11).map(|(r, _)| r).collect();
        assert_eq!(second, vec![0..2, 4..4]);
    }
}
