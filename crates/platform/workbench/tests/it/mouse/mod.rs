//! Synthetic mouse input for the tests of drags (popups, splitters).
//!
//! On a real platform a frame is drawn on the display's schedule, so a
//! quick click (a tap on a trackpad) can bring its release before the
//! frame that follows the press. The test platform draws a window that
//! asked for a redraw after every event; [`quick`] draws nothing else
//! between the events, so a press that does not ask for a redraw keeps the
//! frame from before it for the release.

#![allow(dead_code)]

use gpui::{
    AnyWindowHandle, AppContext as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, TestAppContext, VisualTestContext,
};

/// The left button goes down at `at`.
pub fn press(at: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    })
}

/// The left button goes up at `at`.
pub fn release(at: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
    })
}

/// The left button goes up at `at` with the modifiers held.
pub fn release_with(at: Point<Pixels>, modifiers: Modifiers) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers,
        click_count: 1,
    })
}

/// The mouse moves to `at` with the left button held.
pub fn drag_to(at: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position: at,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::none(),
    })
}

/// The mouse moves to `at` with no button held.
pub fn hover(at: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position: at,
        pressed_button: None,
        modifiers: Modifiers::none(),
    })
}

/// Sends the events with a frame drawn after each.
pub fn send(cx: &mut TestAppContext, window: AnyWindowHandle, events: Vec<PlatformInput>) {
    for event in events {
        simulate(cx, window, event);
        draw(cx, window);
    }
}

/// Sends the events one after another, drawing no frame between them
/// that an event did not ask for; then draws.
pub fn quick(cx: &mut TestAppContext, window: AnyWindowHandle, events: Vec<PlatformInput>) {
    for event in events {
        simulate(cx, window, event);
    }
    draw(cx, window);
}

fn simulate(cx: &mut TestAppContext, window: AnyWindowHandle, event: PlatformInput) {
    let mut vcx = VisualTestContext::from_window(window, cx);
    match event {
        PlatformInput::MouseDown(e) => vcx.simulate_event(e),
        PlatformInput::MouseUp(e) => vcx.simulate_event(e),
        PlatformInput::MouseMove(e) => vcx.simulate_event(e),
        other => panic!("not a mouse event: {other:?}"),
    }
}

pub fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.draw(cx).clear())
        .unwrap();
}
