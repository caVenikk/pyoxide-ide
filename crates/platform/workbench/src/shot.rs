//! `PYOXIDE_SHOT=<path>`: a PNG of the application's own window.
//!
//! GPUI 0.2.2 cannot read back a rendered frame (the Metal renderer is
//! private, the test platform does not render). On macOS this asks the
//! window server for the current image of our own window
//! (`CGWindowListCreateImage` with only that window; no Screen Recording
//! permission is needed for the app's own windows), which shows exactly
//! what GPUI presented on screen.

use std::path::Path;

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
    Err("PYOXIDE_SHOT is implemented on macOS only".into())
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
