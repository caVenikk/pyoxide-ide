//! The cells of a grid, drawn directly: only the rows and columns in view
//! are shaped and painted, so a page of 500 rows and 50 columns scrolls at
//! the display rate. The row numbers stay at the left while the cells
//! scroll sideways.
//!
//! As in the reference, text longer than its cell is cut at the cell's
//! edge without an ellipsis, and the cell under the pointer shows its whole
//! first line over the cells to its right (the expandable item of the
//! reference's tables, IntelliJ Community `AbstractExpandableItemsHandler`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, Font, Hsla, Pixels, Point, SharedString, TextRun, Window, fill,
    outline, point, px, size,
};

use super::data::{GridPage, Pos, Selection};
use super::format::NULL_TEXT;

/// Text inset from a cell's edges.
pub(super) const CELL_PAD: f32 = 4.0;
/// Space under the last row, so the page bar does not hide it.
pub(super) const BOTTOM_ROOM: f32 = 48.0;
/// Width of the scroll bars' thumbs.
const THUMB: f32 = 6.0;
/// Longest text a cell shapes (the rest is out of any cell anyway).
pub(super) const MAX_SHAPED: usize = 1000;

/// Sizes of the grid's parts for one frame, worked out when the view
/// renders (they need the text system) and kept for hit tests.
#[derive(Debug, Clone, Default)]
pub(super) struct Layout {
    /// The cells' area (below the header), set when it is painted.
    pub bounds: Bounds<Pixels>,
    pub row_height: Pixels,
    /// Width of the row number column.
    pub numbers: Pixels,
    /// Left edge of each column in the scrolled content, and the content's
    /// right edge as the last element.
    pub edges: Vec<Pixels>,
    pub rows: usize,
}

impl Layout {
    pub fn content_width(&self) -> Pixels {
        self.edges.last().copied().unwrap_or_default()
    }

    pub fn width(&self, column: usize) -> Pixels {
        match (self.edges.get(column), self.edges.get(column + 1)) {
            (Some(a), Some(b)) => *b - *a,
            _ => px(0.0),
        }
    }

    /// The farthest the cells scroll.
    pub fn max_scroll(&self) -> Point<Pixels> {
        let view = self.bounds.size;
        let content_h = self.row_height * self.rows as f32 + px(BOTTOM_ROOM);
        point(
            (self.content_width() - (view.width - self.numbers)).max(px(0.0)),
            (content_h - view.height).max(px(0.0)),
        )
    }

    pub fn clamp(&self, scroll: Point<Pixels>) -> Point<Pixels> {
        let max = self.max_scroll();
        point(
            scroll.x.clamp(px(0.0), max.x),
            scroll.y.clamp(px(0.0), max.y),
        )
    }

    /// The column whose span holds content x `x`.
    fn column_at(&self, x: Pixels) -> Option<usize> {
        if self.edges.len() < 2 || x < px(0.0) || x >= self.content_width() {
            return None;
        }
        let ix = self.edges.partition_point(|e| *e <= x);
        Some(ix.saturating_sub(1))
    }

    /// What a window position is over.
    pub fn hit(&self, position: Point<Pixels>, scroll: Point<Pixels>) -> Hit {
        let b = self.bounds;
        if !b.contains(&position) || self.row_height <= px(0.0) {
            return Hit::None;
        }
        let y = position.y - b.top() + scroll.y;
        let row = (f32::from(y) / f32::from(self.row_height)) as usize;
        if row >= self.rows {
            return Hit::None;
        }
        let x = position.x - b.left();
        if x < self.numbers {
            return Hit::RowNumber(row);
        }
        match self.column_at(x - self.numbers + scroll.x) {
            Some(column) => Hit::Cell((row, column)),
            None => Hit::None,
        }
    }

    /// The scroll offset that shows cell `pos` whole (as far as it fits).
    pub fn reveal(&self, scroll: Point<Pixels>, (row, column): Pos) -> Point<Pixels> {
        let view = self.bounds.size;
        let mut s = scroll;
        let top = self.row_height * row as f32;
        let bottom = top + self.row_height;
        if top < s.y {
            s.y = top;
        } else if bottom > s.y + view.height - px(BOTTOM_ROOM) {
            s.y = bottom - (view.height - px(BOTTOM_ROOM)).max(self.row_height);
        }
        if let (Some(left), Some(right)) = (self.edges.get(column), self.edges.get(column + 1)) {
            let width = view.width - self.numbers;
            if *left < s.x {
                s.x = *left;
            } else if *right > s.x + width {
                s.x = (*right - width).min(*left);
            }
        }
        self.clamp(s)
    }
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    None,
    RowNumber(usize),
    Cell(Pos),
}

/// The grid's layout, shared between the view and its painted cells.
#[derive(Default)]
pub(super) struct BodyState {
    layout: Rc<RefCell<Layout>>,
}

impl BodyState {
    pub fn layout(&self) -> std::cell::Ref<'_, Layout> {
        self.layout.borrow()
    }

    pub fn shared(&self) -> Rc<RefCell<Layout>> {
        self.layout.clone()
    }

    /// Sets what the view works out per frame (the bounds come with the
    /// paint).
    pub fn update(&self, row_height: Pixels, numbers: Pixels, edges: Vec<Pixels>, rows: usize) {
        let mut l = self.layout.borrow_mut();
        l.row_height = row_height;
        l.numbers = numbers;
        l.edges = edges;
        l.rows = rows;
    }

    pub fn viewport(&self) -> Bounds<Pixels> {
        self.layout.borrow().bounds
    }

    pub fn row_height(&self) -> Pixels {
        self.layout.borrow().row_height
    }

    pub fn reveal(&self, scroll: Point<Pixels>, pos: Pos) -> Point<Pixels> {
        self.layout.borrow().reveal(scroll, pos)
    }

    pub fn hit(&self, position: Point<Pixels>, scroll: Point<Pixels>) -> Hit {
        self.layout.borrow().hit(position, scroll)
    }

    pub fn clamp(&self, scroll: Point<Pixels>) -> Point<Pixels> {
        self.layout.borrow().clamp(scroll)
    }
}

/// Colours of the cells.
#[derive(Debug, Clone, Copy)]
pub(super) struct Colors {
    pub background: Hsla,
    pub text: Hsla,
    pub null: Hsla,
    pub number: Hsla,
    pub number_current: Hsla,
    pub lines: Hsla,
    pub selection: Hsla,
    pub current_row: Hsla,
    pub expansion_border: Hsla,
    pub thumb: Hsla,
}

/// What one frame of cells paints.
pub(super) struct Paint {
    pub page: Arc<GridPage>,
    pub layout: Rc<RefCell<Layout>>,
    pub scroll: Point<Pixels>,
    pub selection: Option<Selection>,
    pub hover: Option<Pos>,
    pub font: Font,
    pub font_size: Pixels,
    pub colors: Colors,
}

fn shape(
    window: &mut Window,
    text: &str,
    font: &Font,
    size: Pixels,
    color: Hsla,
) -> gpui::ShapedLine {
    let text: String = text
        .chars()
        .take(MAX_SHAPED)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(SharedString::from(text), size, &[run], None)
}

impl Paint {
    pub fn paint(self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        self.layout.borrow_mut().bounds = bounds;
        let l = self.layout.borrow().clone();
        let c = self.colors;
        let row_h = l.row_height;
        if row_h <= px(0.0) {
            return;
        }
        window.paint_quad(fill(bounds, c.background));
        let rows = self.page.row_count();
        let first = (f32::from(self.scroll.y) / f32::from(row_h)) as usize;
        let shown = (f32::from(bounds.size.height) / f32::from(row_h)) as usize + 2;
        let last = (first + shown).min(rows);
        let cells_left = bounds.left() + l.numbers;
        let cells = Bounds::from_corners(point(cells_left, bounds.top()), bounds.bottom_right());
        // Columns in view.
        let view_left = self.scroll.x;
        let view_right = self.scroll.x + cells.size.width;
        let columns: Vec<usize> = (0..l.edges.len().saturating_sub(1))
            .filter(|&i| l.edges[i + 1] > l.edges[i])
            .filter(|&i| l.edges[i + 1] > view_left && l.edges[i] < view_right)
            .collect();
        let x_of = |i: usize| cells_left + l.edges[i] - self.scroll.x;
        let y_of = |r: usize| bounds.top() + row_h * r as f32 - self.scroll.y;
        let current_row = self.selection.map(|s| s.lead.0);
        let mut expansion = None;

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for r in first..last {
                let y = y_of(r);
                // The current row's band and the selection.
                if current_row == Some(r) {
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.left(), y), size(bounds.size.width, row_h)),
                        c.current_row,
                    ));
                }
                window.with_content_mask(Some(ContentMask { bounds: cells }), |window| {
                    for &i in &columns {
                        let x = x_of(i);
                        let w = l.width(i);
                        let cell = Bounds::new(point(x, y), size(w, row_h));
                        if self.selection.is_some_and(|s| s.contains((r, i))) {
                            window.paint_quad(fill(cell, c.selection));
                        }
                        let Some(text) = self.page.cell(r, i) else {
                            continue;
                        };
                        let (shown, color) = match text {
                            Some(t) => (t.as_ref(), c.text),
                            None => (NULL_TEXT, c.null),
                        };
                        if shown.is_empty() {
                            continue;
                        }
                        let line = shape(window, shown, &self.font, self.font_size, color);
                        let inner = w - px(2.0 * CELL_PAD);
                        let right = self.page.columns[i].right;
                        let tx = if right && line.width <= inner {
                            x + w - px(CELL_PAD) - line.width
                        } else {
                            x + px(CELL_PAD)
                        };
                        if line.width > inner && self.hover == Some((r, i)) {
                            expansion = Some((cell, line.width, shown.to_owned(), color));
                        }
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds: Bounds::new(
                                    point(x + px(1.0), y),
                                    size(w - px(1.0), row_h),
                                ),
                            }),
                            |window| {
                                let _ = line.paint(point(tx, y), row_h, window, cx);
                            },
                        );
                    }
                });
                // The row number.
                let number = (self.page.first_row + r as u64 + 1).to_string();
                let color = if current_row == Some(r) {
                    c.number_current
                } else {
                    c.number
                };
                let line = shape(window, &number, &self.font, self.font_size, color);
                let _ = line.paint(
                    point(bounds.left() + px(CELL_PAD * 2.0), y),
                    row_h,
                    window,
                    cx,
                );
                // The line under the row.
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), y + row_h - px(1.0)),
                        size(bounds.size.width, px(1.0)),
                    ),
                    c.lines,
                ));
            }
            // Column lines down the whole height, and the row numbers' edge.
            window.paint_quad(fill(
                Bounds::new(
                    point(cells_left - px(1.0), bounds.top()),
                    size(px(1.0), bounds.size.height),
                ),
                c.lines,
            ));
            window.with_content_mask(Some(ContentMask { bounds: cells }), |window| {
                for &i in &columns {
                    let x = x_of(i) + l.width(i) - px(1.0);
                    window.paint_quad(fill(
                        Bounds::new(point(x, bounds.top()), size(px(1.0), bounds.size.height)),
                        c.lines,
                    ));
                }
            });
            // The whole text of the cell under the pointer, over its
            // neighbours.
            if let Some((cell, width, text, color)) = expansion {
                let w = (width + px(2.0 * CELL_PAD)).min(bounds.right() - cell.left());
                let area = Bounds::new(cell.origin, size(w, cell.size.height));
                let back = if self.selection.is_some_and(|s| {
                    let hover = self.hover.unwrap_or_default();
                    s.contains(hover)
                }) {
                    c.selection
                } else if current_row == self.hover.map(|h| h.0) {
                    c.current_row
                } else {
                    c.background
                };
                window.paint_quad(fill(area, c.background));
                window.paint_quad(fill(area, back));
                window.paint_quad(outline(area, c.expansion_border, gpui::BorderStyle::Solid));
                let line = shape(window, &text, &self.font, self.font_size, color);
                let _ = line.paint(
                    point(cell.left() + px(CELL_PAD), cell.top()),
                    cell.size.height,
                    window,
                    cx,
                );
            }
            // Scroll bar thumbs.
            let max = l.max_scroll();
            let content_h = row_h * rows as f32 + px(BOTTOM_ROOM);
            if max.y > px(0.0) {
                let view = bounds.size.height;
                let len = (view * (view / content_h)).max(px(24.0));
                let top = bounds.top() + (view - len) * (self.scroll.y / max.y);
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(bounds.right() - px(THUMB + 2.0), top),
                            size(px(THUMB), len),
                        ),
                        c.thumb,
                    )
                    .corner_radii(px(THUMB / 2.0)),
                );
            }
            if max.x > px(0.0) {
                let view = cells.size.width;
                let content = l.content_width();
                let len = (view * (view / content)).max(px(24.0));
                let left = cells.left() + (view - len) * (self.scroll.x / max.x);
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(left, bounds.bottom() - px(THUMB + 2.0)),
                            size(len, px(THUMB)),
                        ),
                        c.thumb,
                    )
                    .corner_radii(px(THUMB / 2.0)),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout {
            bounds: Bounds::new(point(px(100.0), px(50.0)), size(px(300.0), px(200.0))),
            row_height: px(20.0),
            numbers: px(40.0),
            edges: vec![px(0.0), px(100.0), px(250.0), px(400.0)],
            rows: 100,
        }
    }

    #[test]
    fn hits_find_cells_and_row_numbers() {
        let l = layout();
        let s = point(px(0.0), px(0.0));
        assert_eq!(l.hit(point(px(110.0), px(55.0)), s), Hit::RowNumber(0));
        assert_eq!(l.hit(point(px(150.0), px(75.0)), s), Hit::Cell((1, 0)));
        assert_eq!(l.hit(point(px(250.0), px(75.0)), s), Hit::Cell((1, 1)));
        // Scrolled by a column and two rows.
        let s = point(px(100.0), px(40.0));
        assert_eq!(l.hit(point(px(150.0), px(55.0)), s), Hit::Cell((2, 1)));
        assert_eq!(l.hit(point(px(50.0), px(55.0)), s), Hit::None);
    }

    #[test]
    fn reveal_scrolls_just_enough() {
        let l = layout();
        let s = point(px(0.0), px(0.0));
        assert_eq!(l.reveal(s, (0, 0)), s);
        let down = l.reveal(s, (20, 0));
        assert_eq!(down.y, px(20.0 * 21.0) - (px(200.0) - px(BOTTOM_ROOM)));
        let right = l.reveal(s, (0, 2));
        assert_eq!(right.x, px(400.0 - 260.0));
        assert_eq!(l.max_scroll().x, px(140.0));
        assert_eq!(l.clamp(point(px(-5.0), px(1e6))).x, px(0.0));
    }
}
