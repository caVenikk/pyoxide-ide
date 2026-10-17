//! The divider between the sides of a diff (`DiffDividerDrawUtil`,
//! `DiffDrawUtil.drawCurveTrapezium`): the gutter's background and, for
//! every change in view, a band from its lines on the left to its lines
//! on the right, its edges cubic curves.

use gpui::{
    App, Bounds, Entity, Hsla, IntoElement, PathBuilder, Pixels, Styled as _, Window, canvas,
    point, px,
};
use themes::UiRole;
use ui::theme::ThemeExt as _;

use editor::Editor;

use super::model::{ChangeKind, DiffChange};

/// The divider's width (`diff.divider.width`).
pub const WIDTH: f32 = 24.0;
/// Where the curves' control points are, as a part of the width
/// (`DiffDrawUtil.CTRL_PROXIMITY_X`).
const CTRL_PROXIMITY: f32 = 0.3;

/// The colour of a change's lines.
pub fn change_role(kind: ChangeKind) -> UiRole {
    match kind {
        ChangeKind::Inserted => UiRole::DiffAdded,
        ChangeKind::Deleted => UiRole::DiffRemoved,
        ChangeKind::Modified => UiRole::DiffChanged,
    }
}

/// A band between the sides: its top and bottom on the left and on the
/// right, in pixels from the top of the views.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub start1: f32,
    pub end1: f32,
    pub start2: f32,
    pub end2: f32,
}

/// Where a change's lines are painted in a view (`getGutterMarkerPaintRange`):
/// from the top of its first line to the bottom of its last; an empty
/// range is the boundary above its line (one pixel down at the top of the
/// text).
fn marker_range(lines: &std::ops::Range<u32>, line_height: f32, scroll: f32) -> (f32, f32) {
    let y = |line: u32| line as f32 * line_height - scroll;
    if lines.start == lines.end {
        let y1 = if lines.start == 0 {
            y(0) + 1.0
        } else {
            y(lines.start)
        };
        (y1, y1)
    } else {
        (y(lines.start), y(lines.end))
    }
}

/// The band of a change (`DividerPolygon.paint`): a side of fewer than two
/// pixels is a line one pixel high at its place.
pub fn band(change: &DiffChange, line_height: f32, scroll1: f32, scroll2: f32) -> Band {
    let (s1, e1) = marker_range(&change.left, line_height, scroll1);
    let (s2, e2) = marker_range(&change.right, line_height, scroll2);
    let side = |s: f32, e: f32| {
        if e - s < 2.0 {
            (s - 1.0, s)
        } else {
            (s, e - 1.0)
        }
    };
    let (start1, end1) = side(s1, e1);
    let (start2, end2) = side(s2, e2);
    Band {
        start1,
        end1,
        start2,
        end2,
    }
}

/// The divider element of the views `left` and `right`.
pub fn render(
    left: &Entity<Editor>,
    right: &Entity<Editor>,
    changes: std::rc::Rc<Vec<DiffChange>>,
) -> impl IntoElement {
    let (left, right) = (left.clone(), right.clone());
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| paint(bounds, &left, &right, &changes, window, cx),
    )
    .flex_none()
    .w(px(WIDTH))
    .h_full()
}

fn paint(
    bounds: Bounds<Pixels>,
    left: &Entity<Editor>,
    right: &Entity<Editor>,
    changes: &[DiffChange],
    window: &mut Window,
    cx: &mut App,
) {
    let theme = cx.theme();
    window.paint_quad(gpui::fill(bounds, theme.color(UiRole::Gutter)));
    let (l, r) = (left.read(cx), right.read(cx));
    let Some((line_height, _)) = l.view_metrics().or(r.view_metrics()) else {
        return;
    };
    let lh = f32::from(line_height);
    let (scroll1, scroll2) = (
        f32::from(l.scroll_position().y),
        f32::from(r.scroll_position().y),
    );
    let height = f32::from(bounds.size.height);
    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
        for change in changes {
            let b = band(change, lh, scroll1, scroll2);
            if (b.end1 < 0.0 && b.end2 < 0.0) || (b.start1 > height && b.start2 > height) {
                continue;
            }
            let color = theme.color(change_role(change.kind()));
            paint_band(window, bounds, b, color);
        }
    });
}

/// `DiffDrawUtil.drawCurveTrapezium`: the area between two cubic curves,
/// and the middle curve one pixel thick so that a steep band never gets
/// thinner than that.
fn paint_band(window: &mut Window, bounds: Bounds<Pixels>, b: Band, color: Hsla) {
    let x1 = bounds.left();
    let width = f32::from(bounds.size.width);
    let x2 = x1 + px(width);
    let top = bounds.top();
    let p = |x: Pixels, y: f32| point(x, top + px(y));
    let c1 = x1 + px(width * CTRL_PROXIMITY);
    let c2 = x1 + px(width * (1.0 - CTRL_PROXIMITY));
    let mut path = PathBuilder::fill();
    path.move_to(p(x1, b.start1));
    path.cubic_bezier_to(p(x2, b.start2), p(c1, b.start1), p(c2, b.start2));
    path.line_to(p(x2, b.end2 + 1.0));
    path.cubic_bezier_to(
        p(x1, b.end1 + 1.0),
        p(c2, b.end2 + 1.0),
        p(c1, b.end1 + 1.0),
    );
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
    let (m1, m2) = ((b.start1 + b.end1) / 2.0, (b.start2 + b.end2) / 2.0);
    let mut middle = PathBuilder::stroke(px(1.0));
    middle.move_to(p(x1, m1));
    middle.cubic_bezier_to(p(x2, m2), p(c1, m1), p(c2, m2));
    if let Ok(path) = middle.build() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_side_is_a_line_at_its_boundary() {
        // One line inserted on the right after left line 21 (the shot of
        // the reference: `# changed`).
        let change = DiffChange {
            left: 21..21,
            right: 21..22,
            words: None,
        };
        let b = band(&change, 20.0, 0.0, 0.0);
        assert_eq!(
            b,
            Band {
                start1: 419.0,
                end1: 420.0,
                start2: 420.0,
                end2: 439.0
            }
        );
        // At the top of the text: one pixel down.
        let change = DiffChange {
            left: 0..0,
            right: 0..2,
            words: None,
        };
        assert_eq!(band(&change, 20.0, 0.0, 0.0).start1, 0.0);
    }
}
