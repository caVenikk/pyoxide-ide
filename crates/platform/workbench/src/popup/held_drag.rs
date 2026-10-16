//! A drag that lasts while the left button is held: a popup's move or
//! resize, a splitter inside a popup. It follows the mouse anywhere in the
//! window, as the reference's `WindowMouseListener`, and every way out ends
//! it: a release anywhere (also outside the popup or the window), a move
//! with no button held (its release went to another window or
//! application), another press (the reference ends the drag too: its
//! release was lost) and the window losing the focus.
//!
//! The owner keeps a [`HeldDrag`], starts it from a mouse-down listener and
//! draws [`track_held_drag`] with every frame, not only once a drag is
//! shown: a quick click (a tap on a trackpad) can bring its release before
//! the next frame, and a drag that waited for listeners drawn with that
//! frame would follow the mouse until the next click.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, DispatchPhase, Entity, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Styled, canvas,
};

/// A drag in progress, `S` what it started from, and whether the window
/// had the focus at the last frame.
#[derive(Debug, Clone, Copy)]
pub struct HeldDrag<S> {
    from: Option<S>,
    window_active: bool,
}

impl<S> Default for HeldDrag<S> {
    fn default() -> Self {
        Self {
            from: None,
            window_active: false,
        }
    }
}

/// The mouse, as the window-wide tracking sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Input {
    Press,
    Move(Point<Pixels>, Option<MouseButton>),
    Release(Point<Pixels>),
    FocusLost,
}

/// What the owner does with it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    /// No drag in progress.
    Idle,
    /// The drag follows the mouse to here.
    Follow(Point<Pixels>),
    /// The drag ends, after following the mouse to here if given.
    End(Option<Point<Pixels>>),
}

impl<S: Copy> HeldDrag<S> {
    pub fn start(&mut self, from: S) {
        self.from = Some(from);
    }

    /// What the drag in progress started from.
    pub fn get(&self) -> Option<S> {
        self.from
    }

    pub fn is_dragging(&self) -> bool {
        self.from.is_some()
    }

    /// Ends the drag in progress (also when its owner closes).
    pub fn end(&mut self) {
        self.from = None;
    }

    fn step(&self, input: Input) -> Step {
        if self.from.is_none() {
            return Step::Idle;
        }
        match input {
            Input::Move(at, Some(MouseButton::Left)) => Step::Follow(at),
            Input::Release(at) => Step::End(Some(at)),
            Input::Move(_, _) | Input::Press | Input::FocusLost => Step::End(None),
        }
    }

    /// Notes whether the window is the active one at this frame; whether a
    /// drag is in progress and the window lost the focus since the last
    /// frame (a window that was not active when the drag started, such as
    /// a background one, does not end it).
    fn window_activation(&mut self, active: bool) -> bool {
        let lost = self.window_active && !active && self.from.is_some();
        self.window_active = active;
        lost
    }
}

/// The window-wide tracking of the owner's held drag, found by `drag`:
/// `moved` follows the mouse, `ended` runs once the drag ended. `cursor`
/// is shown over the whole window (pass it while dragging). The listeners
/// work in the capture phase, before the elements under the mouse, which
/// may stop an event.
pub fn track_held_drag<T, S>(
    owner: Entity<T>,
    drag: impl Fn(&mut T) -> &mut HeldDrag<S> + 'static,
    moved: impl Fn(&mut T, Point<Pixels>, &mut Context<T>) + 'static,
    ended: impl Fn(&mut T, &mut Context<T>) + 'static,
    cursor: Option<CursorStyle>,
) -> AnyElement
where
    T: 'static,
    S: Copy + 'static,
{
    let drag = Rc::new(drag);
    let handle = {
        let drag = drag.clone();
        Rc::new(move |this: &mut T, input: Input, cx: &mut Context<T>| {
            match drag(this).step(input) {
                Step::Idle => return,
                Step::Follow(at) => moved(this, at, cx),
                Step::End(at) => {
                    if let Some(at) = at {
                        moved(this, at, cx);
                    }
                    drag(this).end();
                    ended(this, cx);
                }
            }
            cx.notify();
        })
    };
    canvas(
        |_, _, _| (),
        move |_, _, window, cx| {
            if let Some(cursor) = cursor {
                ui::pointer::request_window(cursor, window);
            }
            let active = window.is_window_active();
            if owner.update(cx, |this, _| drag(this).window_activation(active)) {
                // Not while drawing: the owner is redrawn.
                let (owner, handle) = (owner.clone(), handle.clone());
                cx.defer(move |cx| {
                    owner.update(cx, |this, cx| handle(this, Input::FocusLost, cx));
                });
            }
            let send = move |input: Input, cx: &mut App| {
                owner.update(cx, |this, cx| handle(this, input, cx));
            };
            let send = Rc::new(send);
            let on_press = send.clone();
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    on_press(Input::Press, cx);
                }
            });
            let on_move = send.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    on_move(Input::Move(e.position, e.pressed_button), cx);
                }
            });
            window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    send(Input::Release(e.position), cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use super::*;

    #[test]
    fn a_drag_lasts_while_the_left_button_is_held() {
        let at = point(px(10.0), px(20.0));
        let mut d = HeldDrag::<u8>::default();
        // Without a drag the mouse changes nothing.
        for input in [
            Input::Press,
            Input::Move(at, Some(MouseButton::Left)),
            Input::Release(at),
            Input::FocusLost,
        ] {
            assert_eq!(d.step(input), Step::Idle, "{input:?}");
        }
        d.start(1);
        assert_eq!(d.get(), Some(1));
        // Followed while the left button is held.
        assert_eq!(
            d.step(Input::Move(at, Some(MouseButton::Left))),
            Step::Follow(at)
        );
        // A release anywhere ends it there.
        assert_eq!(d.step(Input::Release(at)), Step::End(Some(at)));
        // A move with no button (or another one) held: the release was
        // lost; the drag ends where it is.
        assert_eq!(d.step(Input::Move(at, None)), Step::End(None));
        assert_eq!(
            d.step(Input::Move(at, Some(MouseButton::Right))),
            Step::End(None)
        );
        // Another press ends it, as the focus going away does.
        assert_eq!(d.step(Input::Press), Step::End(None));
        assert_eq!(d.step(Input::FocusLost), Step::End(None));
        d.end();
        assert!(!d.is_dragging());
    }

    #[test]
    fn the_window_losing_the_focus_ends_a_drag() {
        let mut d = HeldDrag::<u8>::default();
        // A background window: not active, the drag stays.
        d.start(1);
        assert!(!d.window_activation(false));
        // Active, then not: the focus went away.
        assert!(!d.window_activation(true));
        assert!(d.window_activation(false));
        // Without a drag, nothing to end.
        d.end();
        assert!(!d.window_activation(true));
        assert!(!d.window_activation(false));
    }
}
