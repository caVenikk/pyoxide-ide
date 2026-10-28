//! Native window helpers (macOS; no-ops elsewhere): drawing a frame now,
//! keeping a window visible during measurements, and `PYOXIDE_SHOT=<path>`,
//! a PNG of the application's own window. To show a state in the image,
//! `PYOXIDE_RUN` runs UI commands first (see `env.rs`), for example
//! `PYOXIDE_RUN='[["find.search", {"text": "def"}]]'` for the find bar.
//!
//! GPUI 0.2.2 cannot read back a rendered frame (the Metal renderer is
//! private, the test platform does not render). On macOS the screenshot
//! asks the window server for the current image of our own window
//! (`CGWindowListCreateImage` with only that window; no Screen Recording
//! permission is needed for the app's own windows), which shows exactly
//! what GPUI presented on screen.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Windows are real platform windows (the application runs, not a test):
/// [`draw_now`] asks them to draw.
static NATIVE: AtomicBool = AtomicBool::new(false);

/// Called by the application start-up: windows are native.
pub fn set_native_windows() {
    NATIVE.store(true, Ordering::Relaxed);
}

/// Whether the windows are real ones (not the test platform's).
pub fn native_windows() -> bool {
    NATIVE.load(Ordering::Relaxed)
}

/// Draws a frame of a native window now rather than at the next display
/// refresh (see [`request_display`]); nothing in tests.
pub fn draw_now(window: &gpui::Window) {
    if NATIVE.load(Ordering::Relaxed) {
        request_display(window);
    }
}

/// Asks Core Animation to redraw the window's layer: GPUI draws and presents
/// a frame at the end of the current run loop pass, without waiting for the
/// display link. Used after keystrokes (typed text is ready for the next
/// refresh) and before screenshots (GPUI stops its display link while a
/// window is fully covered, so a screenshot would show a stale frame).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn request_display(window: &gpui::Window) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;
    // SAFETY: `view` is the live NSView of this window, on the main thread.
    unsafe {
        let layer: *mut Object = msg_send![view, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setNeedsDisplay];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_display(_: &gpui::Window) {}

/// Measurements: keeps the window above normal windows on every Space,
/// without activating the app or taking the focus, so that it is not
/// covered and its display link keeps producing frames.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn keep_visible(window: &gpui::Window) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// NSFloatingWindowLevel.
    const FLOATING: isize = 3;
    /// NSWindowCollectionBehaviorCanJoinAllSpaces | FullScreenAuxiliary.
    const ALL_SPACES: usize = 1 | 256;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;
    // SAFETY: `view` is the live NSView of this window, on the main thread.
    unsafe {
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return;
        }
        let _: () = msg_send![ns_window, setLevel: FLOATING];
        let _: () = msg_send![ns_window, setCollectionBehavior: ALL_SPACES];
        let _: () = msg_send![ns_window, orderFrontRegardless];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn keep_visible(_: &gpui::Window) {}

/// Diagnostics: the system cursor the app shows now, by name.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn current_cursor() -> &'static str {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: class methods of NSCursor, on the main thread.
    unsafe {
        let current: *mut Object = msg_send![class!(NSCursor), currentCursor];
        let named: [(&str, *mut Object); 4] = [
            ("arrow", msg_send![class!(NSCursor), arrowCursor]),
            ("ibeam", msg_send![class!(NSCursor), IBeamCursor]),
            (
                "resize-left-right",
                msg_send![class!(NSCursor), resizeLeftRightCursor],
            ),
            (
                "pointing-hand",
                msg_send![class!(NSCursor), pointingHandCursor],
            ),
        ];
        named
            .iter()
            .find(|(_, c)| *c == current)
            .map_or("other", |(n, _)| n)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn current_cursor() -> &'static str {
    "unknown"
}

/// Diagnostics: shows the arrow as the system does on its own (window
/// edges, title bar), without GPUI knowing.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn system_sets_arrow() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: NSCursor class methods on the main thread.
    unsafe {
        let arrow: *mut Object = msg_send![class!(NSCursor), arrowCursor];
        let _: () = msg_send![arrow, set];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn system_sets_arrow() {}

/// Diagnostics: the cursor the system displays now (in any app), as the
/// hot spot of its image, and the real pointer position in window
/// coordinates (top-left origin).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn system_cursor(window: &gpui::Window) -> serde_json::Value {
    use core_graphics::geometry::{CGPoint, CGRect};
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // SAFETY: AppKit class methods and properties on the main thread.
    unsafe {
        let hot = |c: *mut Object| -> (f64, f64) {
            if c.is_null() {
                return (-1.0, -1.0);
            }
            let p: CGPoint = msg_send![c, hotSpot];
            (p.x, p.y)
        };
        let system: *mut Object = msg_send![class!(NSCursor), currentSystemCursor];
        let ibeam: *mut Object = msg_send![class!(NSCursor), IBeamCursor];
        let arrow: *mut Object = msg_send![class!(NSCursor), arrowCursor];
        let resize: *mut Object = msg_send![class!(NSCursor), resizeLeftRightCursor];
        let mouse: CGPoint = msg_send![class!(NSEvent), mouseLocation];
        let mut in_window = None;
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
        {
            let view = appkit.ns_view.as_ptr() as *mut Object;
            let ns_window: *mut Object = msg_send![view, window];
            let frame: CGRect = msg_send![ns_window, frame];
            in_window = Some((
                mouse.x - frame.origin.x,
                frame.size.height - (mouse.y - frame.origin.y),
            ));
        }
        serde_json::json!({
            "system_hot_spot": hot(system),
            "ibeam_hot_spot": hot(ibeam),
            "arrow_hot_spot": hot(arrow),
            "resize_hot_spot": hot(resize),
            "pointer_in_window": in_window,
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn system_cursor(_: &gpui::Window) -> serde_json::Value {
    serde_json::Value::Null
}

/// Diagnostics: the views of the window (from the frame view down) with
/// their tracking areas (options bit mask and rectangle) and whether they
/// have cursor rectangles.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn tracking_areas(window: &gpui::Window) -> serde_json::Value {
    use core_graphics::geometry::CGRect;
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    unsafe fn walk(view: *mut Object, depth: usize, out: &mut Vec<serde_json::Value>) {
        // SAFETY: (caller) `view` is a live NSView on the main thread.
        unsafe {
            let class: *const objc::runtime::Class = msg_send![view, class];
            let name = (*class).name().to_owned();
            let areas: *mut Object = msg_send![view, trackingAreas];
            let count: usize = msg_send![areas, count];
            let mut list = Vec::new();
            for i in 0..count {
                let area: *mut Object = msg_send![areas, objectAtIndex: i];
                let options: usize = msg_send![area, options];
                let rect: CGRect = msg_send![area, rect];
                let owner: *mut Object = msg_send![area, owner];
                let owner_class: *const objc::runtime::Class = msg_send![owner, class];
                list.push(serde_json::json!({
                    "options": format!("{options:#x}"),
                    "cursor_update": options & 0x4 != 0,
                    "rect": [rect.origin.x, rect.origin.y, rect.size.width, rect.size.height],
                    "owner": (*owner_class).name(),
                }));
            }
            out.push(serde_json::json!({ "depth": depth, "view": name, "tracking_areas": list }));
            let subviews: *mut Object = msg_send![view, subviews];
            let n: usize = msg_send![subviews, count];
            for i in 0..n {
                let sub: *mut Object = msg_send![subviews, objectAtIndex: i];
                walk(sub, depth + 1, out);
            }
        }
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return serde_json::Value::Null;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return serde_json::Value::Null;
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;
    let mut out = Vec::new();
    // SAFETY: AppKit views of this window, on the main thread.
    unsafe {
        let ns_window: *mut Object = msg_send![view, window];
        let content: *mut Object = msg_send![ns_window, contentView];
        let frame_view: *mut Object = msg_send![content, superview];
        walk(
            if frame_view.is_null() {
                content
            } else {
                frame_view
            },
            0,
            &mut out,
        );
    }
    serde_json::Value::Array(out)
}

#[cfg(not(target_os = "macos"))]
pub fn tracking_areas(_: &gpui::Window) -> serde_json::Value {
    serde_json::Value::Null
}

/// Diagnostics: posts a mouse-moved event at a window position into the
/// app's own event queue, so it goes through AppKit's event dispatch like a
/// real mouse move (the real pointer does not move).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn post_mouse_move(window: &gpui::Window, position: gpui::Point<gpui::Pixels>) {
    use core_graphics::geometry::CGPoint;
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// NSEventTypeMouseMoved.
    const MOUSE_MOVED: usize = 5;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;
    let height = f64::from(f32::from(window.viewport_size().height));
    // NSPoint is CGPoint on 64-bit macOS.
    let location = CGPoint::new(
        f64::from(f32::from(position.x)),
        height - f64::from(f32::from(position.y)),
    );
    // SAFETY: AppKit calls on the main thread with a live window.
    unsafe {
        let ns_window: *mut Object = msg_send![view, window];
        let number: isize = msg_send![ns_window, windowNumber];
        let event: *mut Object = msg_send![class!(NSEvent),
            mouseEventWithType: MOUSE_MOVED
            location: location
            modifierFlags: 0usize
            timestamp: 0.0f64
            windowNumber: number
            context: std::ptr::null_mut::<Object>()
            eventNumber: 0isize
            clickCount: 0isize
            pressure: 0.0f32];
        if event.is_null() {
            return;
        }
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, postEvent: event atStart: false];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn post_mouse_move(_: &gpui::Window, _: gpui::Point<gpui::Pixels>) {}

/// Diagnostics: hands a mouse-moved event to the window's frame view, as
/// AppKit does for the frame view's own tracking area when the real pointer
/// moves over the window (posted events do not reach tracking areas).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn frame_view_mouse_moved(window: &gpui::Window, position: gpui::Point<gpui::Pixels>) {
    use core_graphics::geometry::CGPoint;
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const MOUSE_MOVED: usize = 5;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;
    let height = f64::from(f32::from(window.viewport_size().height));
    let location = CGPoint::new(
        f64::from(f32::from(position.x)),
        height - f64::from(f32::from(position.y)),
    );
    // SAFETY: AppKit calls on the main thread with a live window.
    unsafe {
        let ns_window: *mut Object = msg_send![view, window];
        let number: isize = msg_send![ns_window, windowNumber];
        let content: *mut Object = msg_send![ns_window, contentView];
        let frame_view: *mut Object = msg_send![content, superview];
        if frame_view.is_null() {
            return;
        }
        let event: *mut Object = msg_send![class!(NSEvent),
            mouseEventWithType: MOUSE_MOVED
            location: location
            modifierFlags: 0usize
            timestamp: 0.0f64
            windowNumber: number
            context: std::ptr::null_mut::<Object>()
            eventNumber: 0isize
            clickCount: 0isize
            pressure: 0.0f32];
        if !event.is_null() {
            let _: () = msg_send![frame_view, mouseMoved: event];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn frame_view_mouse_moved(_: &gpui::Window, _: gpui::Point<gpui::Pixels>) {}

/// Save the window's on-screen image; returns its size in pixels.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
pub fn save(window: &gpui::Window, path: &Path) -> Result<(u32, u32), String> {
    use core_graphics::display::CGRectNull;
    use core_graphics::window::{
        create_image, kCGWindowImageBestResolution, kCGWindowImageBoundsIgnoreFraming,
        kCGWindowListOptionIncludingWindow,
    };
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return Err("not an AppKit window".into());
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;
    // SAFETY: `view` is the live NSView of this window, on the main thread.
    let number: isize = unsafe {
        let ns_window: *mut Object = msg_send![view, window];
        msg_send![ns_window, windowNumber]
    };
    let image = create_image(
        // SAFETY: a constant exported by CoreGraphics.
        unsafe { CGRectNull },
        kCGWindowListOptionIncludingWindow,
        number as u32,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution,
    )
    .ok_or("CGWindowListCreateImage returned no image")?;
    if image.bits_per_pixel() != 32 {
        return Err(format!(
            "unexpected {} bits per pixel",
            image.bits_per_pixel()
        ));
    }
    let (w, h, stride) = (image.width(), image.height(), image.bytes_per_row());
    let data = image.data();
    // 32-bit little-endian BGRA, premultiplied, in the display's colour
    // space: Display P3 on Apple displays (GPUI's sRGB colours are colour
    // matched by the compositor). Convert back to sRGB with the exact
    // matrix, so neutral greys keep their values.
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in data.bytes().chunks(stride).take(h) {
        for &[b, g, r, a] in row[..w * 4].as_chunks::<4>().0 {
            let [r, g, b] = p3_to_srgb(unpremultiply([r, g, b], a));
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    let (w, h) = (w as u32, h as u32);
    image::RgbaImage::from_raw(w, h, rgba)
        .ok_or("pixel buffer size")?
        .save(path)
        .map_err(|e| e.to_string())?;
    Ok((w, h))
}

#[cfg(not(target_os = "macos"))]
pub fn save(_: &gpui::Window, _: &Path) -> Result<(u32, u32), String> {
    Err(format!(
        "{} is implemented on macOS only",
        app_info::env_name("SHOT")
    ))
}

/// Straight alpha for PNG (only the rounded window corners are translucent).
#[cfg(target_os = "macos")]
fn unpremultiply(c: [u8; 3], a: u8) -> [u8; 3] {
    if a == 0 || a == 255 {
        return c;
    }
    c.map(|v| (u32::from(v) * 255 / u32::from(a)).min(255) as u8)
}

/// Display P3 to sRGB (same white point and transfer curve).
#[cfg(target_os = "macos")]
fn p3_to_srgb(c: [u8; 3]) -> [u8; 3] {
    const M: [[f32; 3]; 3] = [
        [1.224_94, -0.224_94, 0.0],
        [-0.042_057, 1.042_057, 0.0],
        [-0.019_638, -0.078_636, 1.098_274],
    ];
    let lin = c.map(|v| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let mut out = [0u8; 3];
    for (o, m) in out.iter_mut().zip(M) {
        let l = (m[0] * lin[0] + m[1] * lin[1] + m[2] * lin[2]).clamp(0.0, 1.0);
        let v = if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        };
        *o = (v * 255.0).round() as u8;
    }
    out
}
