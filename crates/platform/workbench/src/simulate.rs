//! Synthetic mouse input for screenshots and agent runs: the main window
//! takes a mouse move or a click at a position as if the pointer were there
//! (hover states, tooltips, clicks), without moving the system pointer and
//! without activating the window. A window run in the background
//! (`PYOXIDE_BACKGROUND=1`) never sees the real pointer, so `PYOXIDE_RUN`
//! reaches hover states this way.
//!
//! On macOS the events go straight to the window's view, as AppKit hands
//! real ones to it (going through the application's event queue would make
//! a click activate the window). Elsewhere the command does nothing.

use gpui::{App, Window};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// A synthetic mouse event in the window.
    SimulateMouse { x: f32, y: f32, action: Option<String> } => "workbench.simulate-mouse", "Simulate Mouse",
        "Moves the window's mouse to a position (logical pixels from the window's top left corner), or clicks there with the left button (`action`: `move`, the default, or `click`), with synthetic events (macOS); the system pointer does not move and the window is not activated.",
        r#"{"x": 400, "y": 300, "action": "click"}"#;
}

pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
}

/// One synthetic event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Move,
    Down,
    Up,
}

/// The events of [`SimulateMouse`], in order.
fn events(action: &SimulateMouse) -> &'static [Event] {
    match action.action.as_deref() {
        Some("click") => &[Event::Move, Event::Down, Event::Up],
        _ => &[Event::Move],
    }
}

/// Handles [`SimulateMouse`]. The view handles the events at once, which
/// runs the window's input handling: it happens in a task, after the
/// command's dispatch let go of the window.
pub(crate) fn simulate_mouse(action: &SimulateMouse, window: &mut Window, cx: &mut App) {
    let Some(view) = native::View::of(window) else {
        return;
    };
    let (x, y) = (action.x, action.y);
    let events = events(action);
    cx.spawn(async move |_| {
        for &event in events {
            view.send(event, x, y);
        }
    })
    .detach();
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
mod native {
    use core_graphics::geometry::CGPoint;
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::Event;

    /// The window's content view and the height of its content.
    pub(super) struct View {
        view: *mut Object,
        height: f64,
    }

    impl View {
        pub(super) fn of(window: &gpui::Window) -> Option<Self> {
            let handle = HasWindowHandle::window_handle(window).ok()?;
            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                return None;
            };
            Some(Self {
                view: appkit.ns_view.as_ptr() as *mut Object,
                height: f64::from(f32::from(window.viewport_size().height)),
            })
        }

        /// Hands a mouse event at a window position (from the top left) to
        /// the view.
        pub(super) fn send(&self, event: Event, x: f32, y: f32) {
            // NSEventType: left mouse down, left mouse up, mouse moved.
            let (kind, clicks, pressure): (usize, isize, f32) = match event {
                Event::Down => (1, 1, 1.0),
                Event::Up => (2, 1, 0.0),
                Event::Move => (5, 0, 0.0),
            };
            // NSPoint is CGPoint on 64-bit macOS; window coordinates start
            // at the bottom left.
            let location = CGPoint::new(f64::from(x), self.height - f64::from(y));
            // SAFETY: AppKit calls on the main thread; the view belongs to
            // the window the command ran in, which is still open.
            unsafe {
                let ns_window: *mut Object = msg_send![self.view, window];
                if ns_window.is_null() {
                    return;
                }
                let number: isize = msg_send![ns_window, windowNumber];
                let native: *mut Object = msg_send![class!(NSEvent),
                    mouseEventWithType: kind
                    location: location
                    modifierFlags: 0usize
                    timestamp: 0.0f64
                    windowNumber: number
                    context: std::ptr::null_mut::<Object>()
                    eventNumber: 0isize
                    clickCount: clicks
                    pressure: pressure];
                if native.is_null() {
                    return;
                }
                match event {
                    Event::Down => {
                        let _: () = msg_send![self.view, mouseDown: native];
                    }
                    Event::Up => {
                        let _: () = msg_send![self.view, mouseUp: native];
                    }
                    Event::Move => {
                        let _: () = msg_send![self.view, mouseMoved: native];
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::Event;

    pub(super) struct View;

    impl View {
        pub(super) fn of(_: &gpui::Window) -> Option<Self> {
            None
        }

        pub(super) fn send(&self, _: Event, _: f32, _: f32) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_is_a_move_a_press_and_a_release() {
        let click = SimulateMouse {
            x: 10.0,
            y: 20.0,
            action: Some("click".into()),
        };
        assert_eq!(events(&click), [Event::Move, Event::Down, Event::Up]);
        let hover = SimulateMouse {
            x: 10.0,
            y: 20.0,
            action: None,
        };
        assert_eq!(events(&hover), [Event::Move]);
    }
}
