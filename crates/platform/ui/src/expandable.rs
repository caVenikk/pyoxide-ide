//! Rows of trees and lists whose content does not fit the component, as the
//! reference IDE draws them (`TreeExpandableItemsHandler`,
//! `AbstractExpandableItemsHandler`, `DefaultTreeUI`, `RenderingHelper` of
//! IntelliJ Community, Apache-2.0; only behaviour and numbers are used):
//!
//! - A tree scrolls sideways when its widest row is wider than the view
//!   ([`content_width`], rows at least that wide).
//! - A row's highlight (selection or hover) starts [`SELECTION_INSET`]
//!   from the view's left edge (or at the row's own expand control, when
//!   that is further left) and ends [`SELECTION_INSET`] before its right
//!   edge; a row whose content is wider reaches the edge instead, with a
//!   square right end ([`RowFrame::highlight`]).
//! - Pointing at such a row, or selecting it while the component has the
//!   focus, shows the rest of the row over the component's edge, 10 ms later:
//!   the row's own drawing continuing where the view cuts it, framed by a
//!   1 px border outside the row above, below and at the rounded right end,
//!   none where it joins the row. It stays while the pointer is on it
//!   (clicks on it are clicks on the row), goes when the pointer leaves both
//!   the rows and it, and shows only for a row the view shows whole
//!   ([`ExpandableItems::overlay`]).
//!
//! A component keeps one [`ExpandableItems`] keyed by whatever identifies a
//! row, feeds it the pointer ([`ExpandableItems::attach`]) and the
//! selection ([`ExpandableItems::selection_moved`]), draws each row's
//! highlight from a [`RowFrame`] and adds the overlay on top of itself.

use std::cell::Cell;
use std::hash::Hash;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, Div, EntityId, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, MouseMoveEvent, ParentElement as _, Pixels, Point, SharedString,
    StatefulInteractiveElement, Styled as _, Task, TextRun, UniformListScrollHandle, Window,
    anchored, canvas, div, point, prelude::FluentBuilder as _, px, size,
};

use crate::deferred;
use crate::theme::ThemeExt;

/// Space between the view's edge and a row's highlight
/// (`HORIZONTAL_SELECTION_OFFSET`).
pub const SELECTION_INSET: f32 = 12.0;
/// Space the renderer's cell has after its content (`rendererRight`).
pub const CELL_PAD: f32 = 4.0;
/// Space after the widest row at the end of the scrolled content (the
/// tree's right border inset).
pub const CONTENT_INSET: f32 = 12.0;
/// What is left of the view right of a cut row's highlight.
pub const EDGE_MARGIN: f32 = 1.0;
/// Width of the border of the shown rest of a row.
pub const BORDER: f32 = 1.0;
/// The rest of a row follows the pointer and the selection this much later
/// (the handler's update alarm).
pub const DELAY: Duration = Duration::from_millis(10);
/// How many rows [`content_width`] measures exactly.
const CANDIDATES: usize = 8;

/// Where a row's parts are, as x from the row's left edge (the scrolled
/// content's coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowFrame {
    /// How far the view is scrolled to the right.
    pub scroll_x: Pixels,
    /// Width of the view.
    pub view_width: Pixels,
    /// Left edge of the row's expand control, or of its icon when it has
    /// none.
    pub control_x: Pixels,
    /// Where the row's content (icon, text, details) ends.
    pub content_end: Pixels,
}

/// Where a row is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowPlace {
    /// In the tree, whose scrolled rows are `content` wide.
    Tree { content: Pixels },
    /// As the rest of the row over the component's edge.
    Rest,
}

impl RowPlace {
    /// The row is the rest shown over the edge.
    pub fn is_rest(self) -> bool {
        self == RowPlace::Rest
    }

    /// An invisible child that makes the row as wide as the scrolled rows
    /// when it is measured for the list's width; the row itself is as wide
    /// as the view (plus what is scrolled), so what is drawn and hit is
    /// the visible part.
    pub fn width_spacer(self) -> Option<Div> {
        match self {
            RowPlace::Tree { content } => Some(div().w(content).h(px(0.0)).flex_none()),
            RowPlace::Rest => None,
        }
    }
}

/// A row's highlight: from `left` to `right` (x from the row's left
/// edge), with the corners rounded as flagged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    pub left: Pixels,
    pub right: Pixels,
    pub round_left: bool,
    pub round_right: bool,
}

impl RowFrame {
    /// Right edge of the visible part of the content.
    pub fn view_right(&self) -> Pixels {
        self.scroll_x + self.view_width
    }

    /// Right edge of the row's cell.
    pub fn cell_right(&self) -> Pixels {
        self.content_end + px(CELL_PAD)
    }

    /// The view cuts the row's content (false before the view has a
    /// width).
    pub fn is_cut(&self) -> bool {
        self.view_width > px(0.0) && self.cell_right() > self.view_right()
    }

    /// The highlight of the row. `expanded` is the row drawn in the rest
    /// shown over the edge: it runs to the end of the cell with a rounded
    /// end. Otherwise a cut row's reaches the view's edge but for a pixel.
    pub fn highlight(&self, expanded: bool) -> Highlight {
        let left = (self.scroll_x + px(SELECTION_INSET)).min(self.control_x);
        let full = (self.view_right() - px(SELECTION_INSET)).max(self.cell_right());
        let edge = self.view_right() - px(EDGE_MARGIN);
        if expanded || self.view_width <= px(0.0) || full <= edge {
            Highlight {
                left,
                right: full,
                round_left: true,
                round_right: true,
            }
        } else {
            Highlight {
                left,
                right: edge,
                round_left: true,
                round_right: false,
            }
        }
    }
}

impl Highlight {
    /// The highlight as an absolutely placed layer, `radius` at its
    /// rounded corners; the caller sets its colour.
    pub fn layer(&self, radius: Pixels) -> Div {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(self.left)
            .w((self.right - self.left).max(px(0.0)))
            .when(self.round_left, |d| d.rounded_tl(radius).rounded_bl(radius))
            .when(self.round_right, |d| {
                d.rounded_tr(radius).rounded_br(radius)
            })
    }
}

/// Width the scrolled content has: its widest row's content end, the cell's
/// space after it and the tree's right inset. The widest row is looked for
/// by `estimate` (any unit, proportional to the row's width: cheap, for
/// every row) and then exactly by `measure` (the content end of a row, for
/// the few rows the estimate ranks first).
pub fn content_width(
    count: usize,
    estimate: impl Fn(usize) -> f32,
    mut measure: impl FnMut(usize) -> Pixels,
) -> Pixels {
    let mut best: Vec<(f32, usize)> = Vec::with_capacity(CANDIDATES);
    let mut floor = f32::NEG_INFINITY;
    for ix in 0..count {
        let e = estimate(ix);
        if best.len() < CANDIDATES {
            best.push((e, ix));
            if best.len() == CANDIDATES {
                floor = best.iter().map(|b| b.0).fold(f32::INFINITY, f32::min);
            }
        } else if e > floor {
            let (at, _) = best
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
                .expect("candidates are not empty");
            best[at] = (e, ix);
            floor = best.iter().map(|b| b.0).fold(f32::INFINITY, f32::min);
        }
    }
    let widest = best
        .into_iter()
        .map(|(_, ix)| measure(ix))
        .fold(px(0.0), Pixels::max);
    widest + px(CELL_PAD + CONTENT_INSET)
}

/// Width of `text` in the UI font (`size`, `weight`).
pub fn text_width(
    text: &str,
    size: Pixels,
    weight: FontWeight,
    window: &Window,
    cx: &App,
) -> Pixels {
    if text.is_empty() {
        return px(0.0);
    }
    let theme = cx.theme();
    let mut font = gpui::font(theme.ui_family());
    font.weight = weight;
    let run = TextRun {
        len: text.len(),
        font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(SharedString::from(text.to_owned()), size, &[run], None)
        .width
}

/// How far a list is scrolled: `x` to the right, `y` up (both zero or
/// positive).
pub fn scrolled(handle: &UniformListScrollHandle) -> Point<Pixels> {
    let offset = handle.0.borrow().base_handle.offset();
    point(-offset.x, -offset.y)
}

/// The row of a list of equal rows the pointer is at.
pub fn row_at(
    position: Point<Pixels>,
    view: Bounds<Pixels>,
    scroll_top: Pixels,
    row_height: Pixels,
    count: usize,
) -> Option<usize> {
    if !view.contains(&position) || row_height <= px(0.0) {
        return None;
    }
    let row = ((position.y - view.origin.y + scroll_top) / row_height).floor();
    (row >= 0.0 && (row as usize) < count).then_some(row as usize)
}

/// How an [`ExpandableItems`] is found in the view that owns it (for the
/// timer and the listeners).
pub type Access<V, K> = fn(&mut V) -> &mut ExpandableItems<K>;

/// Which row the pointer is at and which row shows its rest.
pub struct ExpandableItems<K> {
    /// The row the pointer is over, now (for hover highlights).
    pointer: Option<K>,
    /// The row that shows its rest, or the row last selected: follows the
    /// pointer and the selection [`DELAY`] later.
    key: Option<K>,
    /// `key` came from the selection: it shows while the component has the
    /// focus and the row stays selected.
    by_selection: bool,
    /// Where the view of the rows was drawn last (window coordinates).
    view: Rc<Cell<Bounds<Pixels>>>,
    /// Where the rest of a row was drawn last (window coordinates).
    hint: Cell<Option<Bounds<Pixels>>>,
    /// The change of `key` waiting for [`DELAY`].
    pending: Option<Task<()>>,
}

impl<K> Default for ExpandableItems<K> {
    fn default() -> Self {
        Self {
            pointer: None,
            key: None,
            by_selection: false,
            view: Rc::default(),
            hint: Cell::new(None),
            pending: None,
        }
    }
}

/// What [`ExpandableItems::overlay`] draws.
pub struct Overlay {
    /// Top of the row in the view.
    pub row_top: Pixels,
    pub row_height: Pixels,
    pub frame: RowFrame,
    /// The component's background (under the row).
    pub background: Hsla,
    /// The border colour.
    pub border: Hsla,
    /// Radius of the highlight's corners.
    pub radius: Pixels,
    /// The row as the component draws it, with the frame's `expanded`
    /// highlight, `w_full` and `row_height` high.
    pub row: AnyElement,
}

impl<K: Clone + PartialEq + Eq + Hash + 'static> ExpandableItems<K> {
    /// Forgets everything (the rows changed).
    pub fn clear(&mut self) {
        self.pointer = None;
        self.key = None;
        self.by_selection = false;
        self.hint.set(None);
        self.pending = None;
    }

    /// Where the view of the rows was drawn last, zero before the first
    /// frame.
    pub fn view(&self) -> Bounds<Pixels> {
        self.view.get()
    }

    /// Width of the view.
    pub fn view_width(&self) -> Pixels {
        self.view.get().size.width
    }

    /// The row the pointer is over (also while it is on the rest of the
    /// row): the row to draw the hover highlight on.
    pub fn hovered(&self) -> Option<&K> {
        self.pointer.as_ref()
    }

    /// A layer that records where the view is, and draws the owner again
    /// when its width changes (rows measure their text against it). Add it
    /// to the element that holds the rows.
    pub fn track_view<V: 'static>(&self, cx: &mut Context<V>) -> AnyElement {
        let bounds = self.view.clone();
        let owner: EntityId = cx.entity_id();
        canvas(
            move |b, _, cx| {
                if bounds.replace(b).size.width != b.size.width {
                    cx.notify(owner);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    /// Lets the pointer's row be found: `row_at` tells the row at a
    /// position (window coordinates) of the owner.
    pub fn attach<V: 'static, E: StatefulInteractiveElement>(
        el: E,
        row_at: fn(&V, Point<Pixels>, &App) -> Option<K>,
        access: Access<V, K>,
        cx: &mut Context<V>,
    ) -> E {
        el.on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            let key = row_at(this, e.position, cx);
            Self::pointer_moved(key, access, this, cx);
        }))
        .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
            if !hovered {
                Self::pointer_left(access, this, window, cx);
            }
        }))
    }

    /// Shows the rest of `key` (or of nothing) after [`DELAY`]; a change
    /// back to the current row cancels a waiting one.
    fn expand_later<V: 'static>(
        key: Option<K>,
        by_selection: bool,
        access: Access<V, K>,
        this: &mut V,
        cx: &mut Context<V>,
    ) {
        let me = access(this);
        if key == me.key && by_selection == me.by_selection {
            me.pending = None;
            return;
        }
        let timer = cx.background_executor().timer(DELAY);
        me.pending = Some(cx.spawn(async move |view, cx| {
            timer.await;
            let _ = view.update(cx, |this, cx| {
                let me = access(this);
                me.key = key;
                me.by_selection = by_selection;
                me.pending = None;
                cx.notify();
            });
        }));
    }

    /// The pointer moved over the rows, to `key` (none: below the last
    /// row).
    pub fn pointer_moved<V: 'static>(
        key: Option<K>,
        access: Access<V, K>,
        this: &mut V,
        cx: &mut Context<V>,
    ) {
        let me = access(this);
        if me.pointer != key {
            me.pointer = key.clone();
            cx.notify();
        }
        Self::expand_later(key, false, access, this, cx);
    }

    /// The selection moved to `key`.
    pub fn selection_moved<V: 'static>(
        key: K,
        access: Access<V, K>,
        this: &mut V,
        cx: &mut Context<V>,
    ) {
        Self::expand_later(Some(key), true, access, this, cx);
    }

    /// The pointer left the rows or the rest of a row: they go unless the
    /// pointer is on the other one.
    pub fn pointer_left<V: 'static>(
        access: Access<V, K>,
        this: &mut V,
        window: &Window,
        cx: &mut Context<V>,
    ) {
        let me = access(this);
        let position = window.mouse_position();
        if me.view.get().contains(&position) || me.hint.get().is_some_and(|b| b.contains(&position))
        {
            return;
        }
        me.pending = None;
        let mut changed = me.pointer.take().is_some();
        if !me.by_selection {
            changed |= me.key.take().is_some();
        }
        if changed {
            cx.notify();
        }
    }

    /// The row whose rest should show now, if it fits: pointed at, or
    /// selected while the component has the focus.
    pub fn shown(&self, focused: bool, selected: Option<&K>) -> Option<&K> {
        let key = self.key.as_ref()?;
        if self.by_selection && (!focused || selected != Some(key)) {
            return None;
        }
        Some(key)
    }

    /// Nothing is shown over the edge (until [`Self::overlay`] shows
    /// something): call it before returning without one.
    pub fn reset_hint(&self) {
        self.hint.set(None);
    }

    /// The rest of a cut row over the component's edge, cut at the
    /// window's edge; `None` when the row is not cut or not whole in view.
    /// Draw it in the owner's element tree.
    pub fn overlay<V: 'static>(
        &self,
        o: Overlay,
        window: &Window,
        access: Access<V, K>,
        cx: &mut Context<V>,
    ) -> Option<AnyElement> {
        self.hint.set(None);
        let view = self.view.get();
        let frame = o.frame;
        if !frame.is_cut() || o.row_top < px(0.0) || o.row_top + o.row_height > view.size.height {
            return None;
        }
        let border = px(BORDER);
        // It starts where the row's highlight ends, so the two meet, and
        // its border runs outside the row's height: top, bottom and right,
        // none on the side that joins the row.
        let start = frame.view_right() - px(EDGE_MARGIN);
        let origin = point(
            view.left() + frame.view_width - px(EDGE_MARGIN),
            view.top() + o.row_top - border,
        );
        let width =
            (frame.cell_right() - start + border).min(window.viewport_size().width - origin.x);
        if width <= border {
            return None;
        }
        let height = o.row_height + border * 2.0;
        self.hint
            .set(Some(Bounds::new(origin, size(width, height))));
        let hint = div()
            .id("row-expansion")
            .debug_selector(|| "row-expansion".into())
            // What is below gets no pointer while the pointer is on it.
            .occlude()
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                if !hovered {
                    Self::pointer_left(access, this, window, cx);
                }
            }));
        Some(
            deferred(
                anchored().position(origin).child(
                    hint.relative()
                        .w(width)
                        .h(height)
                        .bg(o.border)
                        .rounded_tr(o.radius + border)
                        .rounded_br(o.radius + border)
                        .child(
                            // The row's part beyond the edge, exactly as
                            // high as the row and level with it.
                            div()
                                .debug_selector(|| "row-expansion-row".into())
                                .absolute()
                                .top(border)
                                .left_0()
                                .w(width - border)
                                .h(o.row_height)
                                .overflow_hidden()
                                .rounded_tr(o.radius)
                                .rounded_br(o.radius)
                                .bg(o.background)
                                .child(
                                    // The row itself, drawn once more and
                                    // moved left so that its content
                                    // continues where the view cuts it.
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(-start)
                                        .w(frame.cell_right())
                                        .h(o.row_height)
                                        .child(o.row),
                                ),
                        ),
                ),
            )
            .into_any_element(),
        )
    }

    /// The row whose rest is drawn over the edge now (for state reports).
    pub fn shown_now(&self) -> Option<&K> {
        self.hint.get().and(self.key.as_ref())
    }

    /// The pointer's row has its rest shown (for state reports).
    pub fn is_showing(&self) -> bool {
        self.hint.get().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(scroll_x: f32, content_end: f32) -> RowFrame {
        RowFrame {
            scroll_x: px(scroll_x),
            view_width: px(200.0),
            control_x: px(12.0),
            content_end: px(content_end),
        }
    }

    #[test]
    fn a_row_that_fits_keeps_its_inset_on_both_sides() {
        let h = frame(0.0, 100.0).highlight(false);
        assert_eq!((h.left, h.right), (px(12.0), px(188.0)));
        assert!(h.round_left && h.round_right);
        assert!(!frame(0.0, 100.0).is_cut());
    }

    #[test]
    fn a_cut_row_reaches_the_edge_but_for_a_pixel_with_a_square_end() {
        let f = frame(0.0, 300.0);
        assert!(f.is_cut());
        let h = f.highlight(false);
        assert_eq!(h.right, px(199.0));
        assert!(h.round_left && !h.round_right);
        // The rest over the edge ends at the cell, rounded.
        let h = f.highlight(true);
        assert_eq!(h.right, px(304.0));
        assert!(h.round_right);
    }

    #[test]
    fn a_row_a_little_too_wide_for_the_inset_ends_inside_the_view() {
        // Content to 190: the cell ends at 194, past the inset (188) but
        // inside the view.
        let f = frame(0.0, 190.0);
        assert!(!f.is_cut());
        let h = f.highlight(false);
        assert_eq!(h.right, px(194.0));
        assert!(h.round_right);
    }

    #[test]
    fn scrolled_to_the_end_the_highlight_ends_inside_the_view() {
        // The content is 300 + 4 + 12 wide: scrolled all the way, the view
        // spans 116..316.
        let f = frame(116.0, 300.0);
        assert!(!f.is_cut());
        let h = f.highlight(false);
        assert_eq!(h.right, px(304.0));
        assert!(h.round_right);
        // Its left end follows the view.
        assert_eq!(h.left, px(12.0));
        let deep = RowFrame {
            control_x: px(60.0),
            ..frame(116.0, 300.0)
        };
        assert_eq!(deep.highlight(false).left, px(60.0));
        let deep = RowFrame {
            control_x: px(60.0),
            ..frame(10.0, 300.0)
        };
        assert_eq!(deep.highlight(false).left, px(22.0));
    }

    #[test]
    fn the_widest_row_is_found_among_the_ones_estimated_widest() {
        // Estimates rank by length; the exact widths differ a little.
        let widths = [
            10.0f32, 80.0, 30.0, 82.0, 5.0, 79.0, 1.0, 2.0, 3.0, 4.0, 6.0, 7.0,
        ];
        let measured = std::cell::RefCell::new(Vec::new());
        let w = content_width(
            widths.len(),
            |i| widths[i],
            |i| {
                measured.borrow_mut().push(i);
                px(widths[i])
            },
        );
        assert_eq!(w, px(82.0 + CELL_PAD + CONTENT_INSET));
        assert!(measured.borrow().len() <= CANDIDATES);
        assert_eq!(
            content_width(0, |_| 0.0, |_| px(1.0)),
            px(CELL_PAD + CONTENT_INSET)
        );
    }

    #[test]
    fn rows_are_found_by_position() {
        let view = Bounds::new(point(px(10.0), px(100.0)), size(px(200.0), px(260.0)));
        let at = |y: f32, top: f32| row_at(point(px(50.0), px(y)), view, px(top), px(26.0), 20);
        assert_eq!(at(100.0, 0.0), Some(0));
        assert_eq!(at(126.0, 0.0), Some(1));
        assert_eq!(at(110.0, 52.0), Some(2));
        assert_eq!(at(99.0, 0.0), None);
        assert_eq!(
            row_at(point(px(5.0), px(110.0)), view, px(0.0), px(26.0), 20),
            None
        );
        // Below the last row.
        assert_eq!(
            row_at(point(px(50.0), px(300.0)), view, px(0.0), px(26.0), 3),
            None
        );
    }
}
