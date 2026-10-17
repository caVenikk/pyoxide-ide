//! What the editors of a side-by-side diff need (the reference's
//! `TwosideTextDiffViewer`): the mirrored layout of the left side, its line
//! numbers at the right next to the divider and its scrollbar at the left
//! (`EditorEx.setVerticalScrollbarOrientation(LEFT)`); backgrounds of char
//! ranges (the changed words, `DiffDrawUtil.createInlineHighlighter`); and
//! scrolling set by the owner, which keeps the other side in step
//! (`SyncScrollSupport`).

use std::ops::Range;

use gpui::{Context, Pixels, Point, px};
use text::AppliedEdit;
use themes::UiRole;

use crate::editor::Editor;

/// A background of a char range, drawn over the full-line backgrounds and
/// under the selection. An empty range is drawn as a thin bar (the
/// reference's `DiffEmptyHighlighterRenderer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeBackground {
    pub range: Range<usize>,
    pub color: UiRole,
}

impl RangeBackground {
    pub fn new(range: Range<usize>, color: UiRole) -> Self {
        Self { range, color }
    }
}

/// Moves a position through an applied edit: after the edit (text
/// inserted at the position counts as before it) it shifts, before it it
/// stays, inside a replaced range it goes to the edit's start (`start`
/// side) or the end of the new text.
fn map_position(pos: usize, e: &AppliedEdit, start: bool) -> usize {
    if pos >= e.old_end {
        pos - e.old_end + e.new_end
    } else if pos <= e.start {
        pos
    } else if start {
        e.start
    } else {
        e.new_end
    }
}

impl Editor {
    /// Lays the editor out mirrored: the line numbers at the right edge,
    /// the scrollbar and the error stripe at the left.
    pub fn set_mirrored(&mut self, mirrored: bool, cx: &mut Context<Self>) {
        if self.mirrored != mirrored {
            self.mirrored = mirrored;
            cx.notify();
        }
    }

    pub fn is_mirrored(&self) -> bool {
        self.mirrored
    }

    /// Replaces the char range backgrounds (sorted by start or not).
    pub fn set_range_backgrounds(&mut self, ranges: Vec<RangeBackground>, cx: &mut Context<Self>) {
        let len = self.buffer.len_chars();
        self.range_backgrounds = ranges
            .into_iter()
            .map(|mut r| {
                r.range.end = r.range.end.min(len);
                r.range.start = r.range.start.min(r.range.end);
                r
            })
            .collect();
        cx.notify();
    }

    pub fn range_backgrounds(&self) -> &[RangeBackground] {
        &self.range_backgrounds
    }

    /// Moves the range backgrounds through applied edits (in the order
    /// they were applied): text typed inside a range grows it.
    pub(crate) fn range_backgrounds_edited(&mut self, applied: &[AppliedEdit]) {
        if self.range_backgrounds.is_empty() {
            return;
        }
        for e in applied {
            for r in &mut self.range_backgrounds {
                let start = map_position(r.range.start, e, true);
                let end = map_position(r.range.end, e, false).max(start);
                r.range = start..end;
            }
        }
    }

    /// Scrolls the view to `position` (clamped to the text) without
    /// reporting [`crate::EditorEvent::Scrolled`]: the owner moves this view
    /// with another one.
    pub fn set_scroll_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let max_y = self.max_scroll_y();
        let y = position.y.clamp(px(0.0), max_y);
        let x = if self.soft_wraps() {
            px(0.0)
        } else {
            position.x.max(px(0.0))
        };
        if self.scroll.y != y || self.scroll.x != x {
            self.scroll.y = y;
            self.scroll.x = x;
            cx.notify();
        }
    }

    /// The scroll offset of the view.
    pub fn scroll_position(&self) -> Point<Pixels> {
        self.scroll
    }

    /// The height of a row and of the view in the last frame.
    pub fn view_metrics(&self) -> Option<(Pixels, Pixels)> {
        let l = self.layout.as_ref()?;
        Some((l.line_height, l.bounds.size.height))
    }

    /// How far the view scrolls down at most (the last row at the top).
    pub fn max_scroll_top(&self) -> Pixels {
        self.max_scroll_y()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(start: usize, old_end: usize, new_end: usize) -> AppliedEdit {
        AppliedEdit {
            start,
            old_end,
            new_end,
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start_position: (0, start),
            old_end_position: (0, old_end),
            new_end_position: (0, new_end),
            start_utf16_col: start,
            old_end_utf16_col: old_end,
            text: "x".repeat(new_end - start).into(),
        }
    }

    #[test]
    fn ranges_move_with_edits_around_them() {
        let r = |range: Range<usize>, e: &AppliedEdit| {
            let s = map_position(range.start, e, true);
            s..map_position(range.end, e, false).max(s)
        };
        // Typed before: shifted.
        assert_eq!(r(4..6, &edit(1, 1, 3)), 6..8);
        // Typed after: kept.
        assert_eq!(r(4..6, &edit(8, 8, 9)), 4..6);
        // Typed at the start: outside; at the end: inside.
        assert_eq!(r(4..6, &edit(4, 4, 5)), 5..7);
        assert_eq!(r(4..6, &edit(6, 6, 7)), 4..7);
        // Deleted around it: empty at the deletion.
        assert_eq!(r(4..6, &edit(3, 7, 3)), 3..3);
    }
}
