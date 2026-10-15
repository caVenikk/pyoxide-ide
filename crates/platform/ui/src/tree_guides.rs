//! Indent guides of trees, as the reference IDE draws them with "Show
//! indent guides" (`ClassicPainter` of IntelliJ Community, Apache-2.0): a
//! 1 px line in the theme role `tree-indent-guide` (`Tree.hash`) at the
//! centre of each ancestor's expand control, through the whole row, and
//! for an open folder the part of its own line below its control. Rows
//! draw their own part, so the lines of a folder run down its visible
//! content; there is no emphasis of the selected row's path.
//!
//! A tree describes where its expand controls are with [`TreeGuides`] and
//! adds [`TreeGuides::row`] to each row (an absolutely placed layer that
//! takes no input).

use gpui::{Div, Hsla, ParentElement, Pixels, Styled, div, px};

/// Where a tree's expand controls (the chevrons) are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeGuides {
    /// Left edge of the control of a top-level row (depth 0), from the
    /// row's left edge.
    pub first: Pixels,
    /// Indentation of one level.
    pub indent: Pixels,
    /// Size of the control (square).
    pub control: Pixels,
}

/// Width of a guide line.
const LINE: f32 = 1.0;

impl TreeGuides {
    /// Left edge of the line of the control at `level`: the control's
    /// centre, the line drawn just left of it (`LinePainter2D` centred on
    /// the half pixel before the centre).
    pub fn line_x(&self, level: usize) -> Pixels {
        self.first + self.indent * level as f32 + self.control / 2.0 - px(LINE)
    }

    /// The lines of a row at `depth`, `row_height` high: one through the
    /// row for each level above it (`0..depth`), and, when the row is an
    /// expanded folder, the part of the line of its own level below its
    /// control. `None` when there is nothing to draw (a closed top-level
    /// row).
    pub fn row(
        &self,
        depth: usize,
        expanded_folder: bool,
        row_height: Pixels,
        color: Hsla,
    ) -> Option<Div> {
        if depth == 0 && !expanded_folder {
            return None;
        }
        let line = |left: Pixels, top: Pixels, height: Pixels| {
            div()
                .absolute()
                .left(left)
                .top(top)
                .w(px(LINE))
                .h(height)
                .bg(color)
        };
        let mut layer = div().absolute().top_0().left_0().size_full();
        for level in 0..depth {
            layer = layer.child(line(self.line_x(level), px(0.0), row_height));
        }
        if expanded_folder {
            // Below the control, which is centred in the row.
            let offset = (row_height - self.control) / 2.0;
            if offset > px(0.0) {
                layer = layer.child(line(self.line_x(depth), row_height - offset, offset));
            }
        }
        Some(layer)
    }

    /// The left edges of the lines a row at `depth` draws (for tests and
    /// state reports).
    pub fn row_lines(&self, depth: usize, expanded_folder: bool) -> Vec<Pixels> {
        let mut out: Vec<Pixels> = (0..depth).map(|l| self.line_x(l)).collect();
        if expanded_folder {
            out.push(self.line_x(depth));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_sit_at_the_ancestors_controls() {
        let g = TreeGuides {
            first: px(12.0),
            indent: px(20.0),
            control: px(16.0),
        };
        assert_eq!(g.line_x(0), px(19.0));
        assert_eq!(g.line_x(2), px(59.0));
        // A closed top-level row draws nothing; an open one the part below
        // its control.
        assert!(g.row(0, false, px(26.0), Hsla::default()).is_none());
        assert_eq!(g.row_lines(0, true), [px(19.0)]);
        // A file two levels down: the lines of its two ancestors.
        assert_eq!(g.row_lines(2, false), [px(19.0), px(39.0)]);
        assert_eq!(g.row_lines(1, true), [px(19.0), px(39.0)]);
    }
}
