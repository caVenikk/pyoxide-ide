//! A popup over the main window that can be moved by its header and resized
//! from its edges and corners, like the reference IDE's Find in Files and
//! Search Everywhere popups. The owner keeps a [`PopupFrame`], draws its
//! content into [`PopupFrame::render`] and starts a move from its header
//! with [`PopupFrame::start_move`].

use gpui::{
    AnyElement, Bounds, Context, CursorStyle, Div, Entity, HitboxBehavior, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Size, Styled, Window, canvas, div, point, px, size,
};
use serde::{Deserialize, Serialize};
use themes::{Metric, UiRole};
use ui::theme::ThemeExt;

/// Room kept between a popup and the window's edges.
const MARGIN: f32 = 16.0;
/// Width of the band along the edges that resizes the popup.
const GRIP: f32 = 5.0;

/// Where a popup was left and how large it was, kept across sessions
/// (offset from its default place, so it stays sensible in another window
/// size).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PopupPlace {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Which edges a resize moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
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

/// Position and size of a popup.
#[derive(Debug, Clone)]
pub struct PopupFrame {
    /// Default size, and the smallest one.
    default_size: Size<Pixels>,
    min_size: Size<Pixels>,
    /// Distance of the default place from the top of the window.
    top: Pixels,
    /// Moved by the user: offset from the default place.
    offset: Point<Pixels>,
    /// Resized by the user.
    size: Option<Size<Pixels>>,
    drag: Option<Drag>,
    /// The bounds of the last frame.
    last: Option<Bounds<Pixels>>,
}

impl PopupFrame {
    pub fn new(default_size: Size<Pixels>, min_size: Size<Pixels>, top: Pixels) -> Self {
        Self {
            default_size,
            min_size,
            top,
            offset: point(px(0.0), px(0.0)),
            size: None,
            drag: None,
            last: None,
        }
    }

    /// Restores a place kept from an earlier session.
    pub fn restore(&mut self, place: PopupPlace) {
        self.offset = point(px(place.x), px(place.y));
        self.size = Some(size(px(place.width), px(place.height)));
    }

    /// The place to keep, once the popup was moved or resized.
    pub fn place(&self) -> Option<PopupPlace> {
        let size = self.size?;
        Some(PopupPlace {
            x: f32::from(self.offset.x),
            y: f32::from(self.offset.y),
            width: f32::from(size.width),
            height: f32::from(size.height),
        })
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Stops a move or resize (the popup closed).
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    /// The popup's bounds in a window of this size: horizontally centred,
    /// `top` below the top, moved and resized as the user left it, and kept
    /// inside the window.
    pub fn bounds(&self, viewport: Size<Pixels>) -> Bounds<Pixels> {
        let wanted = self.size.unwrap_or(self.default_size);
        let width = wanted
            .width
            .max(self.min_size.width)
            .min(viewport.width - px(2.0 * MARGIN))
            .max(px(1.0));
        let height = wanted
            .height
            .max(self.min_size.height)
            .min(viewport.height - self.top.min(viewport.height / 4.0) - px(MARGIN))
            .max(px(1.0));
        let x = ((viewport.width - width) / 2.0 + self.offset.x)
            .clamp(px(0.0), (viewport.width - width).max(px(0.0)));
        let y = (self.top + self.offset.y).clamp(px(0.0), (viewport.height - height).max(px(0.0)));
        Bounds::new(point(x, y), size(width, height))
    }

    /// Starts moving the popup with the mouse (from its header).
    pub fn start_move(&mut self, mouse: Point<Pixels>) {
        self.drag = Some(Drag::Move {
            mouse,
            offset: self.offset,
        });
    }

    fn start_resize(&mut self, edges: Edges, mouse: Point<Pixels>) {
        if let Some(bounds) = self.last {
            self.drag = Some(Drag::Resize {
                edges,
                mouse,
                bounds,
            });
        }
    }

    /// The mouse moved while dragging.
    fn drag_to(&mut self, mouse: Point<Pixels>, viewport: Size<Pixels>) {
        match self.drag {
            Some(Drag::Move {
                mouse: start,
                offset,
            }) => self.offset = offset + (mouse - start),
            Some(Drag::Resize {
                edges,
                mouse: start,
                bounds,
            }) => {
                let d = mouse - start;
                let mut left = bounds.left();
                let mut right = bounds.right();
                let mut top = bounds.top();
                let mut bottom = bounds.bottom();
                if edges.left {
                    left = (left + d.x).min(right - self.min_size.width);
                }
                if edges.right {
                    right = (right + d.x).max(left + self.min_size.width);
                }
                if edges.top {
                    top = (top + d.y).min(bottom - self.min_size.height);
                }
                if edges.bottom {
                    bottom = (bottom + d.y).max(top + self.min_size.height);
                }
                let width = right - left;
                let height = bottom - top;
                self.size = Some(size(width, height));
                // The default place of this size, and the offset from it.
                let x0 = (viewport.width - width) / 2.0;
                self.offset = point(left - x0, top - self.top);
            }
            None => {}
        }
    }

    /// Draws the popup: `content` in a raised, rounded frame at its bounds,
    /// with the resize grips along the edges and, while dragging, the
    /// window-wide mouse tracking. `frame` finds the frame in the owner.
    pub fn render<T: 'static>(
        &mut self,
        id: &'static str,
        content: impl IntoElement,
        frame: fn(&mut T) -> &mut PopupFrame,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Div {
        let viewport = window.viewport_size();
        let bounds = self.bounds(viewport);
        self.last = Some(bounds);
        let theme = cx.theme();
        let owner = cx.entity();
        let tracking = self
            .drag
            .is_some()
            .then(|| drag_tracking(owner.clone(), frame, viewport));
        let grips = grips(owner, frame);
        div()
            .absolute()
            .occlude()
            .left(bounds.origin.x)
            .top(bounds.origin.y)
            .w(bounds.size.width)
            .h(bounds.size.height)
            .debug_selector(move || id.into())
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(theme.metric(Metric::CornerRadius) * 2.0)
                    .border_1()
                    .border_color(theme.color(UiRole::PopupBorder))
                    .bg(theme.color(UiRole::SurfaceRaised))
                    .shadow(ui::components::popup_shadow())
                    .child(content),
            )
            .children(grips)
            .children(tracking)
    }
}

/// While dragging: follows the mouse anywhere in the window.
fn drag_tracking<T: 'static>(
    owner: Entity<T>,
    frame: fn(&mut T) -> &mut PopupFrame,
    viewport: Size<Pixels>,
) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, cx| {
            if let Some(Drag::Resize { edges, .. }) = frame_of(&owner, frame, cx) {
                ui::pointer::request_window(edges.cursor(), window);
            }
            let moved = owner.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, _, _, cx| {
                moved.update(cx, |this, cx| {
                    frame(this).drag_to(e.position, viewport);
                    cx.notify();
                })
            });
            let released = owner.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                released.update(cx, |this, cx| {
                    frame(this).drag = None;
                    cx.notify();
                })
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

/// The drag in progress of the owner's frame.
fn frame_of<T: 'static>(
    owner: &Entity<T>,
    frame: fn(&mut T) -> &mut PopupFrame,
    cx: &mut gpui::App,
) -> Option<Drag> {
    owner.update(cx, |this, _| frame(this).drag)
}

impl Edges {
    fn cursor(self) -> CursorStyle {
        match (self.left || self.right, self.top || self.bottom) {
            (true, false) => CursorStyle::ResizeLeftRight,
            (false, true) => CursorStyle::ResizeUpDown,
            _ if self.left == self.top => CursorStyle::ResizeUpLeftDownRight,
            _ => CursorStyle::ResizeUpRightDownLeft,
        }
    }
}

/// The bands along the edges and the corners that resize the popup.
fn grips<T: 'static>(owner: Entity<T>, frame: fn(&mut T) -> &mut PopupFrame) -> Vec<AnyElement> {
    let edges = |left, right, top, bottom| Edges {
        left,
        right,
        top,
        bottom,
    };
    let grip = px(GRIP);
    let corner = px(GRIP * 2.0);
    let all = [
        ("popup-grip-left", edges(true, false, false, false)),
        ("popup-grip-right", edges(false, true, false, false)),
        ("popup-grip-top", edges(false, false, true, false)),
        ("popup-grip-bottom", edges(false, false, false, true)),
        ("popup-grip-top-left", edges(true, false, true, false)),
        ("popup-grip-top-right", edges(false, true, true, false)),
        ("popup-grip-bottom-left", edges(true, false, false, true)),
        ("popup-grip-bottom-right", edges(false, true, false, true)),
    ];
    all.into_iter()
        .map(|(id, e)| {
            let owner = owner.clone();
            let cursor = e.cursor();
            let el = div()
                .id(id)
                .debug_selector(move || id.into())
                .absolute()
                .child(
                    canvas(
                        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                        move |_, hitbox, window, _| ui::pointer::request(cursor, &hitbox, window),
                    )
                    .size_full(),
                )
                .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    owner.update(cx, |this, cx| {
                        frame(this).start_resize(e, ev.position);
                        cx.notify();
                    });
                });
            let el = match (e.left || e.right, e.top || e.bottom) {
                // A corner.
                (true, true) => el.w(corner).h(corner),
                // A side.
                (true, false) => el.w(grip).top(corner).bottom(corner),
                _ => el.h(grip).left(corner).right(corner),
            };
            let el = if e.left { el.left(-grip / 2.0) } else { el };
            let el = if e.right { el.right(-grip / 2.0) } else { el };
            let el = if e.top { el.top(-grip / 2.0) } else { el };
            let el = if e.bottom { el.bottom(-grip / 2.0) } else { el };
            el.into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> PopupFrame {
        PopupFrame::new(
            size(px(800.0), px(600.0)),
            size(px(400.0), px(300.0)),
            px(115.0),
        )
    }

    #[test]
    fn centred_below_the_top_and_kept_inside_the_window() {
        let f = frame();
        let b = f.bounds(size(px(1280.0), px(800.0)));
        assert_eq!(b.origin, point(px(240.0), px(115.0)));
        assert_eq!(b.size, size(px(800.0), px(600.0)));
        // A small window: the popup shrinks to fit.
        let b = f.bounds(size(px(700.0), px(500.0)));
        assert!(b.right() <= px(700.0) && b.bottom() <= px(500.0), "{b:?}");
    }

    #[test]
    fn moving_and_resizing_are_kept_as_a_place() {
        let viewport = size(px(1280.0), px(800.0));
        let mut f = frame();
        f.start_move(point(px(500.0), px(130.0)));
        f.drag_to(point(px(520.0), px(150.0)), viewport);
        f.drag = None;
        let b = f.bounds(viewport);
        assert_eq!(b.origin, point(px(260.0), px(135.0)));
        // Wider from the right edge; the left edge stays.
        f.last = Some(b);
        f.start_resize(
            Edges {
                left: false,
                right: true,
                top: false,
                bottom: false,
            },
            point(px(1060.0), px(300.0)),
        );
        f.drag_to(point(px(1110.0), px(300.0)), viewport);
        let b = f.bounds(viewport);
        assert_eq!(b.size.width, px(850.0));
        assert_eq!(b.left(), px(260.0));
        // Not smaller than the minimum.
        f.drag_to(point(px(0.0), px(300.0)), viewport);
        assert_eq!(f.bounds(viewport).size.width, px(400.0));
        let place = f.place().unwrap();
        let mut g = frame();
        g.restore(place);
        assert_eq!(g.bounds(viewport), f.bounds(viewport));
    }
}
