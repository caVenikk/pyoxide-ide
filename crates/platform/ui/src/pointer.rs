//! Pointer (mouse cursor) shapes.
//!
//! Elements ask for a shape over a hitbox while they paint, through
//! [`request`] (or [`PointerExt`] for an element's whole area); that also
//! hands the request to GPUI. GPUI 0.2.2 applies a shape only when the set
//! of hovered hitboxes changes or after a frame, and on macOS skips it when
//! `NSCursor.currentCursor` already reports that shape. The system,
//! however, changes the shown pointer on its own while the real pointer
//! moves (window edges and corners, the title bar area, entering the
//! window), so a shape applied once is lost while the pointer moves inside
//! the same element, and GPUI does not apply it again. As the reference IDE
//! does, the shape is therefore applied on every mouse move: [`apply`]
//! resolves it from this frame's requests and sets it.
//!
//! That works only for shapes asked for through this module: GPUI's own
//! `cursor_pointer()` and the like (and `InteractiveText`, which sets the
//! hand itself) are invisible to it and get overwritten with the arrow. Use
//! [`PointerExt::pointing_hand`], [`crate::link`] and
//! [`crate::link::linked_text`] instead; a test keeps the others out.
//!
//! Every window that uses shapes draws a [`frame`] once (the main window,
//! dialog windows, popup windows); requests in a window without one reach
//! GPUI only.
//!
//! What is drawn over an element that asks for a shape (an editor's text
//! cursor) must take the mouse from it, as GPUI's hover rules say: an
//! overlay blocks the mouse for what is under it (`occlude`,
//! `block_mouse_except_scroll`), then nothing under it is hovered and the
//! arrow shows, or the overlay's own shape.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, CursorStyle, DispatchPhase, ElementId, Hitbox, HitboxBehavior, HitboxId,
    IntoElement, MouseExitEvent, MouseMoveEvent, ParentElement, Pixels, Point, Styled, Window,
    WindowId, canvas,
};

/// Whether the mouse is over a part of the element that asks for the shape.
type Only = Rc<dyn Fn(Point<Pixels>) -> bool>;

struct Request {
    /// `None`: the whole window.
    hitbox: Option<HitboxId>,
    style: CursorStyle,
    /// The shape applies only where this holds (the link ranges of a text).
    only: Option<Only>,
    /// The element is a link: it is underlined while the mouse is over it.
    link: Option<ElementId>,
}

thread_local! {
    /// Whether [`apply`] sets the native pointer (off on the test platform,
    /// which has no native windows).
    static NATIVE: Cell<bool> = const { Cell::new(false) };
    /// The requests of the frame on screen of each window that draws a
    /// [`frame`], in paint order.
    static FRAMES: RefCell<HashMap<WindowId, Vec<Request>>> = RefCell::new(HashMap::new());
    /// The link under the mouse, set on mouse moves.
    static HOVERED_LINK: RefCell<Option<ElementId>> = const { RefCell::new(None) };
}

/// Lets [`apply`] set the native pointer; called by the application when
/// it runs on a real platform.
pub fn enable_native() {
    NATIVE.with(|n| n.set(true));
}

/// Windows are native ones (not the test platform's).
pub fn native() -> bool {
    NATIVE.with(Cell::get)
}

/// Forgets the requests of the previous frame of `window` (and makes the
/// window take requests); called when the window starts to draw a frame,
/// by [`frame`].
pub fn begin_frame(window: &Window) {
    let id = window.window_handle().window_id();
    FRAMES.with(|f| f.borrow_mut().entry(id).or_default().clear());
}

fn push(window: &Window, request: Request) {
    let id = window.window_handle().window_id();
    FRAMES.with(|f| {
        if let Some(requests) = f.borrow_mut().get_mut(&id) {
            requests.push(request);
        }
    });
}

/// Asks for `style` while the mouse is over `hitbox` (call while painting).
/// Later requests win over earlier ones, as elements painted later are on
/// top.
pub fn request(style: CursorStyle, hitbox: &Hitbox, window: &mut Window) {
    window.set_cursor_style(style, hitbox);
    push(
        window,
        Request {
            hitbox: Some(hitbox.id),
            style,
            only: None,
            link: None,
        },
    );
}

/// Like [`request`], but only where `only` holds for the mouse position
/// (the clickable ranges of a text).
pub fn request_where(
    style: CursorStyle,
    hitbox: &Hitbox,
    window: &mut Window,
    only: impl Fn(Point<Pixels>) -> bool + 'static,
) {
    if only(window.mouse_position()) {
        window.set_cursor_style(style, hitbox);
    }
    push(
        window,
        Request {
            hitbox: Some(hitbox.id),
            style,
            only: Some(Rc::new(only)),
            link: None,
        },
    );
}

/// Asks for `style` everywhere in the window, over any element (call while
/// painting; for example while dragging a splitter).
pub fn request_window(style: CursorStyle, window: &mut Window) {
    window.set_window_cursor_style(style);
    push(
        window,
        Request {
            hitbox: None,
            style,
            only: None,
            link: None,
        },
    );
}

/// The shape for the current mouse position: a window-wide request if there
/// is one, otherwise the last request whose hitbox is hovered, otherwise the
/// arrow.
pub fn resolve(window: &Window) -> CursorStyle {
    let id = window.window_handle().window_id();
    FRAMES.with(|f| {
        let f = f.borrow();
        let Some(requests) = f.get(&id) else {
            return CursorStyle::Arrow;
        };
        if let Some(r) = requests.iter().rev().find(|r| r.hitbox.is_none()) {
            return r.style;
        }
        hovered(requests, window).map_or(CursorStyle::Arrow, |r| r.style)
    })
}

/// The last request whose area the mouse is over.
fn hovered<'a>(requests: &'a [Request], window: &Window) -> Option<&'a Request> {
    let mouse = window.mouse_position();
    requests.iter().rev().find(|r| {
        r.hitbox.is_some_and(|id| id.is_hovered(window))
            && r.only.as_ref().is_none_or(|only| only(mouse))
    })
}

/// Shows the resolved shape now, if the window is the key window of the
/// active application (macOS; on other platforms GPUI's own handling
/// stays in place).
pub fn apply(window: &Window) {
    if NATIVE.with(Cell::get) {
        native::set(window, resolve(window));
    }
}

/// The mouse moved: the link under it changes, and the shape shows.
fn moved(window: &mut Window) {
    let id = window.window_handle().window_id();
    let link = FRAMES.with(|f| {
        let f = f.borrow();
        let requests = f.get(&id)?;
        requests
            .iter()
            .rev()
            .find(|r| {
                r.link.is_some()
                    && r.hitbox.is_some_and(|id| id.is_hovered(window))
                    && r.only
                        .as_ref()
                        .is_none_or(|only| only(window.mouse_position()))
            })
            .and_then(|r| r.link.clone())
    });
    set_link(link, window);
    apply(window);
}

fn set_link(link: Option<ElementId>, window: &mut Window) {
    let changed = HOVERED_LINK.with(|h| {
        let mut h = h.borrow_mut();
        let changed = *h != link;
        *h = link;
        changed
    });
    // Links draw their underline from this state, so they draw again.
    if changed {
        window.refresh();
    }
}

/// Whether the link `id` ([`crate::link::link`]) is under the mouse (it
/// draws its underline then).
pub fn link_hovered(id: &ElementId) -> bool {
    HOVERED_LINK.with(|h| h.borrow().as_ref() == Some(id))
}

/// What a window draws once to use pointer shapes: it starts each frame's
/// requests and applies the shape on every mouse move. Add it to the
/// window's root.
pub fn frame() -> AnyElement {
    canvas(
        |_, window, _| begin_frame(window),
        |_, _, window, _| {
            window.on_mouse_event(|_: &MouseMoveEvent, phase, window, _| {
                if phase == DispatchPhase::Capture {
                    moved(window);
                }
            });
            window.on_mouse_event(|_: &MouseExitEvent, phase, window, _| {
                if phase == DispatchPhase::Capture {
                    set_link(None, window);
                }
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

/// An area over the parent that asks for `style`: an absolutely placed
/// child with a hitbox of its own, which takes no room and blocks nothing.
pub fn area(style: CursorStyle) -> AnyElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| request(style, &hitbox, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// Like [`area`], but only where `only` holds for the mouse position (the
/// clickable ranges of a text).
pub fn area_where(
    style: CursorStyle,
    only: impl Fn(Point<Pixels>) -> bool + 'static,
) -> AnyElement {
    let only: Only = Rc::new(only);
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            let only = only.clone();
            request_where(style, &hitbox, window, move |p| only(p));
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// Like [`area`], for the link `id`: it also marks the link as under the
/// mouse.
pub(crate) fn link_area(id: ElementId) -> AnyElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
            push(
                window,
                Request {
                    hitbox: Some(hitbox.id),
                    style: CursorStyle::PointingHand,
                    only: None,
                    link: Some(id.clone()),
                },
            );
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// Pointer shapes for an element's whole area, by [`area`].
pub trait PointerExt: ParentElement + Sized {
    /// `style` over the element.
    fn pointer(self, style: CursorStyle) -> Self {
        self.child(area(style))
    }

    /// The pointing hand over the element: for links and for what the
    /// reference gives it (popup lists, icons inside a text field). Buttons
    /// keep the arrow.
    fn pointing_hand(self) -> Self {
        self.pointer(CursorStyle::PointingHand)
    }
}

impl<T: ParentElement> PointerExt for T {}

#[cfg(target_os = "macos")]
mod native {
    use gpui::{CursorStyle, Window};
    use objc::runtime::{BOOL, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
    pub fn set(window: &Window, style: CursorStyle) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        let view = appkit.ns_view.as_ptr() as *mut Object;
        // SAFETY: AppKit calls on the main thread with a live window.
        unsafe {
            let ns_window: *mut Object = msg_send![view, window];
            if ns_window.is_null() {
                return;
            }
            let key: BOOL = msg_send![ns_window, isKeyWindow];
            if key == NO {
                return;
            }
            let cursor: *mut Object = match style {
                CursorStyle::IBeam => msg_send![class!(NSCursor), IBeamCursor],
                CursorStyle::PointingHand => msg_send![class!(NSCursor), pointingHandCursor],
                CursorStyle::ResizeLeftRight | CursorStyle::ResizeColumn => {
                    msg_send![class!(NSCursor), resizeLeftRightCursor]
                }
                CursorStyle::ResizeUpDown | CursorStyle::ResizeRow => {
                    msg_send![class!(NSCursor), resizeUpDownCursor]
                }
                CursorStyle::Crosshair => msg_send![class!(NSCursor), crosshairCursor],
                CursorStyle::ClosedHand => msg_send![class!(NSCursor), closedHandCursor],
                CursorStyle::OpenHand => msg_send![class!(NSCursor), openHandCursor],
                // Shapes the workbench does not use stay with GPUI.
                CursorStyle::Arrow => msg_send![class!(NSCursor), arrowCursor],
                _ => return,
            };
            let _: () = msg_send![cursor, set];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn set(_: &gpui::Window, _: gpui::CursorStyle) {}
}
