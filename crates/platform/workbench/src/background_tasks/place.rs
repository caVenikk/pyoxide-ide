//! Where the Processes popup is: at the bottom right of the window, as
//! large as its content, until the user moves it by its header or resizes
//! it from its edges (the reference's popup is movable and resizable and
//! keeps its place, `setDimensionServiceKey`). A move keeps the size
//! following the content; a resize fixes it.

use gpui::{Bounds, Pixels, Point, Size, point, px, size};

use crate::popup::Edges;

/// What a drag started from.
#[derive(Debug, Clone, Copy)]
pub(super) enum Drag {
    Move {
        mouse: Point<Pixels>,
        offset: Point<Pixels>,
    },
    Resize {
        edges: Edges,
        mouse: Point<Pixels>,
        bounds: Bounds<Pixels>,
    },
}

/// The popup's place relative to its default one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Place {
    /// Moved by the user: right and down from the default place.
    pub offset: Point<Pixels>,
    /// Resized by the user.
    pub size: Option<Size<Pixels>>,
}

/// The room the default place leaves: from the window's right edge and
/// from its bottom edge to the popup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Margins {
    pub right: Pixels,
    pub bottom: Pixels,
}

impl Place {
    /// The distances of the popup from the window's right and bottom edges.
    pub fn insets(&self, margins: Margins) -> (Pixels, Pixels) {
        (
            margins.right - self.offset.x,
            margins.bottom - self.offset.y,
        )
    }

    /// Follows the mouse during `drag`: `laid_out` is where the popup was
    /// drawn last, `min` its smallest size.
    pub fn drag_to(
        &mut self,
        drag: Drag,
        mouse: Point<Pixels>,
        laid_out: Bounds<Pixels>,
        viewport: Size<Pixels>,
        margins: Margins,
        min: Size<Pixels>,
    ) {
        match drag {
            Drag::Move {
                mouse: start,
                offset,
            } => {
                let wanted = offset + (mouse - start);
                // Kept inside the window.
                let w = laid_out.size.width;
                let h = laid_out.size.height;
                let x = wanted
                    .x
                    .min(margins.right)
                    .max((margins.right - viewport.width + w).min(margins.right));
                let y = wanted
                    .y
                    .min(margins.bottom)
                    .max((margins.bottom - viewport.height + h).min(margins.bottom));
                self.offset = point(x, y);
            }
            Drag::Resize {
                edges,
                mouse: start,
                bounds,
            } => {
                let d = mouse - start;
                let mut left = bounds.left();
                let mut right = bounds.right();
                let mut top = bounds.top();
                let mut bottom = bounds.bottom();
                if edges.left {
                    left = (left + d.x).max(px(0.0)).min(right - min.width);
                }
                if edges.right {
                    right = (right + d.x).min(viewport.width).max(left + min.width);
                }
                if edges.top {
                    top = (top + d.y).max(px(0.0)).min(bottom - min.height);
                }
                if edges.bottom {
                    bottom = (bottom + d.y).min(viewport.height).max(top + min.height);
                }
                self.size = Some(size(right - left, bottom - top));
                self.offset = point(
                    margins.right - (viewport.width - right),
                    margins.bottom - (viewport.height - bottom),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Size<Pixels> = Size {
        width: px(1200.0),
        height: px(800.0),
    };
    const MARGINS: Margins = Margins {
        right: px(20.0),
        bottom: px(34.0),
    };
    const MIN: Size<Pixels> = Size {
        width: px(300.0),
        height: px(138.0),
    };

    fn drawn(place: &Place, w: f32, h: f32) -> Bounds<Pixels> {
        let (right, bottom) = place.insets(MARGINS);
        let w = place.size.map_or(px(w), |s| s.width);
        let h = place.size.map_or(px(h), |s| s.height);
        Bounds::new(
            point(VIEWPORT.width - right - w, VIEWPORT.height - bottom - h),
            size(w, h),
        )
    }

    #[test]
    fn a_move_keeps_the_size_and_stays_in_the_window() {
        let mut place = Place::default();
        let b = drawn(&place, 400.0, 200.0);
        assert_eq!(b.origin, point(px(780.0), px(566.0)));
        let drag = Drag::Move {
            mouse: point(px(900.0), px(580.0)),
            offset: place.offset,
        };
        place.drag_to(drag, point(px(700.0), px(380.0)), b, VIEWPORT, MARGINS, MIN);
        assert_eq!(place.size, None);
        assert_eq!(
            drawn(&place, 400.0, 200.0).origin,
            point(px(580.0), px(366.0))
        );
        // Past the top left corner: it stops at the edges.
        place.drag_to(
            drag,
            point(px(-900.0), px(-900.0)),
            b,
            VIEWPORT,
            MARGINS,
            MIN,
        );
        assert_eq!(drawn(&place, 400.0, 200.0).origin, point(px(0.0), px(0.0)));
        // Past the bottom right corner: it stops at the edges too.
        place.drag_to(
            drag,
            point(px(3000.0), px(3000.0)),
            b,
            VIEWPORT,
            MARGINS,
            MIN,
        );
        let b = drawn(&place, 400.0, 200.0);
        assert_eq!(b.right(), VIEWPORT.width);
        assert_eq!(b.bottom(), VIEWPORT.height);
    }

    #[test]
    fn a_resize_fixes_the_size_not_below_the_minimum() {
        let mut place = Place::default();
        let b = drawn(&place, 400.0, 200.0);
        let edges = Edges {
            left: true,
            right: false,
            top: true,
            bottom: false,
        };
        let drag = Drag::Resize {
            edges,
            mouse: b.origin,
            bounds: b,
        };
        place.drag_to(
            drag,
            b.origin - point(px(100.0), px(50.0)),
            b,
            VIEWPORT,
            MARGINS,
            MIN,
        );
        let r = drawn(&place, 0.0, 0.0);
        assert_eq!(r.size, size(px(500.0), px(250.0)));
        // The right and bottom edges stay.
        assert_eq!((r.right(), r.bottom()), (b.right(), b.bottom()));
        place.drag_to(
            drag,
            b.origin + point(px(390.0), px(190.0)),
            b,
            VIEWPORT,
            MARGINS,
            MIN,
        );
        assert_eq!(place.size, Some(MIN));
    }
}
