//! Pointer (mouse cursor) shapes.
//!
//! Elements ask for a shape over a hitbox while they paint, through
//! [`request`]; that also hands the request to GPUI. GPUI 0.2.2 applies a
//! shape only when the set of hovered hitboxes changes or after a frame,
//! and on macOS skips it when `NSCursor.currentCursor` already reports that
//! shape. The system, however, changes the shown pointer on its own while
//! the real pointer moves (window edges and corners, the title bar area,
//! entering the window), so a shape applied once is lost while the pointer
//! moves inside the same element, and GPUI does not apply it again. As the
//! reference IDE does, the shape is therefore applied on every mouse move:
//! [`apply`] resolves it from this frame's requests and sets it.

use std::cell::{Cell, RefCell};

use gpui::{CursorStyle, Hitbox, HitboxId, Window};

thread_local! {
    /// Whether [`apply`] sets the native pointer (off on the test platform,
    /// which has no native windows).
    static NATIVE: Cell<bool> = const { Cell::new(false) };
    /// Requests of the frame on screen, in paint order; `None` is a
    /// request for the whole window.
    static REQUESTS: RefCell<Vec<(Option<HitboxId>, CursorStyle)>> = const { RefCell::new(Vec::new()) };
}

/// Lets [`apply`] set the native pointer; called by the application when
/// it runs on a real platform.
pub fn enable_native() {
    NATIVE.with(|n| n.set(true));
}

/// Forgets the requests of the previous frame; called when the main
/// window starts to draw a frame.
pub fn begin_frame() {
    REQUESTS.with(|r| r.borrow_mut().clear());
}

/// Asks for `style` while the mouse is over `hitbox` (call while painting).
/// Later requests win over earlier ones, as elements painted later are on
/// top.
pub fn request(style: CursorStyle, hitbox: &Hitbox, window: &mut Window) {
    window.set_cursor_style(style, hitbox);
    REQUESTS.with(|r| r.borrow_mut().push((Some(hitbox.id), style)));
}

/// Asks for `style` everywhere in the window, over any element (call while
/// painting; for example while dragging a splitter).
pub fn request_window(style: CursorStyle, window: &mut Window) {
    window.set_window_cursor_style(style);
    REQUESTS.with(|r| r.borrow_mut().push((None, style)));
}

/// The shape for the current mouse position: a window-wide request if there
/// is one, otherwise the last request whose hitbox is hovered, otherwise the
/// arrow.
pub fn resolve(window: &Window) -> CursorStyle {
    REQUESTS.with(|r| {
        let r = r.borrow();
        if let Some((_, style)) = r.iter().rev().find(|(id, _)| id.is_none()) {
            return *style;
        }
        r.iter()
            .rev()
            .find(|(id, _)| id.is_some_and(|id| id.is_hovered(window)))
            .map_or(CursorStyle::Arrow, |(_, style)| *style)
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
