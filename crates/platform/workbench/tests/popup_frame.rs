//! The movable popup frame (Search Everywhere, Find in Files, Recent Files)
//! driven by synthetic mouse sequences: a move lasts while the left button
//! is held on the header and every way out ends it, also a release that
//! comes before the next frame and one that is lost.

mod mouse;

use gpui::{
    AppContext as _, Bounds, ClickEvent, Context, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, PlatformInput, Point, Render,
    StatefulInteractiveElement as _, Styled, TestAppContext, VisualTestContext, Window,
    WindowHandle, div, point, px, size,
};
use mouse::{drag_to, hover, press, release};
use workbench::popup::{PopupFrame, no_move};

/// A popup with a header that moves it and a tab in the header.
struct Owner {
    frame: PopupFrame,
    tab_clicks: usize,
}

impl Render for Owner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A press on the header asks for no redraw (it has no id, so GPUI
        // keeps no pressed state for it, and the listener does not notify):
        // the release of a quick click finds the frame from before the
        // press, as it can on a real platform.
        let header = div()
            .debug_selector(|| "header".into())
            .flex()
            .flex_row()
            .h(px(40.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, _| this.frame.start_move(e.position)),
            )
            .child(
                div()
                    .id("tab")
                    .debug_selector(|| "tab".into())
                    .w(px(80.0))
                    .h_full()
                    .on_mouse_down(MouseButton::Left, no_move)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.tab_clicks += 1)),
            );
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().id("body").debug_selector(|| "body".into()).flex_1());
        div().size_full().child(self.frame.render(
            "popup",
            content,
            |this| &mut this.frame,
            window,
            cx,
        ))
    }
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Owner>,
}

impl H<'_> {
    fn new(cx: &mut TestAppContext) -> H<'_> {
        cx.update(|cx| ui::theme::init(cx, themes::ThemeKind::Dark));
        let window = cx.add_window(|_, _| Owner {
            frame: PopupFrame::new(
                size(px(600.0), px(400.0)),
                size(px(300.0), px(200.0)),
                px(100.0),
            ),
            tab_clicks: 0,
        });
        mouse::draw(cx, window.into());
        H { cx, window }
    }

    fn bounds(&mut self, selector: &'static str) -> Bounds<Pixels> {
        VisualTestContext::from_window(self.window.into(), self.cx)
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
    }

    fn popup(&mut self) -> Point<Pixels> {
        self.bounds("popup").origin
    }

    /// A point on the header's free space.
    fn on_header(&mut self) -> Point<Pixels> {
        self.bounds("header").origin + point(px(300.0), px(20.0))
    }

    fn dragging(&mut self) -> bool {
        self.window
            .read_with(self.cx, |o, _| o.frame.is_dragging())
            .unwrap()
    }

    fn tab_clicks(&mut self) -> usize {
        self.window.read_with(self.cx, |o, _| o.tab_clicks).unwrap()
    }

    /// Each event in its own frame.
    fn send(&mut self, events: Vec<PlatformInput>) {
        mouse::send(self.cx, self.window.into(), events);
    }

    /// The events with no frame between them that they did not ask for.
    fn quick(&mut self, events: Vec<PlatformInput>) {
        mouse::quick(self.cx, self.window.into(), events);
    }
}

fn offset(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

#[gpui::test]
fn a_move_lasts_while_the_header_is_held(cx: &mut TestAppContext) {
    let mut h = H::new(cx);
    let start = h.popup();
    let at = h.on_header();
    h.send(vec![press(at)]);
    assert!(h.dragging());
    h.send(vec![drag_to(at + offset(30.0, 20.0))]);
    assert_eq!(h.popup(), start + offset(30.0, 20.0));
    h.send(vec![release(at + offset(30.0, 20.0))]);
    assert!(!h.dragging());
    // Afterwards the popup does not follow the mouse.
    h.send(vec![hover(at + offset(120.0, 90.0))]);
    assert_eq!(h.popup(), start + offset(30.0, 20.0));
}

#[gpui::test]
fn a_quick_click_on_the_header_does_not_leave_a_move(cx: &mut TestAppContext) {
    let mut h = H::new(cx);
    let start = h.popup();
    let at = h.on_header();
    // The release comes before the frame that follows the press.
    h.quick(vec![press(at), release(at)]);
    assert!(!h.dragging(), "the release ends the move");
    h.send(vec![hover(at + offset(80.0, 60.0))]);
    assert_eq!(h.popup(), start);
    // Pressed, moved and released before a frame: moved, and ended.
    h.quick(vec![
        press(at),
        drag_to(at + offset(10.0, 5.0)),
        release(at + offset(10.0, 5.0)),
    ]);
    assert!(!h.dragging());
    assert_eq!(h.popup(), start + offset(10.0, 5.0));
}

#[gpui::test]
fn a_release_outside_the_popup_and_the_window_ends_a_move(cx: &mut TestAppContext) {
    let mut h = H::new(cx);
    let start = h.popup();
    let at = h.on_header();
    // Released below the popup, over the rest of the window.
    let popup = h.bounds("popup");
    let below = point(popup.center().x, popup.bottom() + px(40.0));
    h.send(vec![
        press(at),
        drag_to(at + offset(30.0, 20.0)),
        release(below),
    ]);
    assert!(!h.dragging());
    let moved = h.popup();
    assert_ne!(moved, start);
    h.send(vec![hover(at)]);
    assert_eq!(h.popup(), moved);
    // Released outside the window.
    let at = h.on_header();
    h.send(vec![
        press(at),
        drag_to(at + offset(10.0, 10.0)),
        release(offset(-40.0, -30.0)),
    ]);
    assert!(!h.dragging());
    let moved = h.popup();
    h.send(vec![hover(at + offset(60.0, 60.0))]);
    assert_eq!(h.popup(), moved);
}

#[gpui::test]
fn a_lost_release_ends_the_move_on_the_next_move_or_press(cx: &mut TestAppContext) {
    let mut h = H::new(cx);
    let start = h.popup();
    let at = h.on_header();
    // The release went to another window: the next move has no button.
    h.send(vec![press(at), drag_to(at + offset(30.0, 20.0))]);
    h.send(vec![hover(at + offset(200.0, 150.0))]);
    assert!(!h.dragging());
    assert_eq!(h.popup(), start + offset(30.0, 20.0));
    // Or the next press comes first: it ends the move (on the body it
    // starts no other one).
    let at = h.on_header();
    h.send(vec![press(at)]);
    let body = h.bounds("body").center();
    h.send(vec![press(body)]);
    assert!(!h.dragging());
    h.send(vec![drag_to(body + offset(40.0, 40.0)), release(body)]);
    assert_eq!(h.popup(), start + offset(30.0, 20.0));
}

#[gpui::test]
fn a_click_on_a_tab_selects_it_and_does_not_move_the_popup(cx: &mut TestAppContext) {
    let mut h = H::new(cx);
    let start = h.popup();
    let tab = h.bounds("tab").center();
    h.quick(vec![press(tab), release(tab)]);
    assert_eq!(h.tab_clicks(), 1);
    assert!(!h.dragging());
    h.send(vec![hover(tab + offset(80.0, 60.0))]);
    assert_eq!(h.popup(), start);
    // Pressed and dragged on the tab: the popup stays.
    h.send(vec![press(tab)]);
    assert!(!h.dragging());
    h.send(vec![
        drag_to(tab + offset(30.0, 20.0)),
        release(tab + offset(30.0, 20.0)),
    ]);
    assert_eq!(h.popup(), start);
}

#[gpui::test]
fn the_window_losing_the_focus_ends_a_move(cx: &mut TestAppContext) {
    let mut h = H::new(cx);
    h.cx.update_window(h.window.into(), |_, window, _| window.activate_window())
        .unwrap();
    mouse::draw(h.cx, h.window.into());
    let start = h.popup();
    let at = h.on_header();
    h.send(vec![press(at), drag_to(at + offset(30.0, 20.0))]);
    assert!(h.dragging());
    VisualTestContext::from_window(h.window.into(), h.cx).deactivate_window();
    mouse::draw(h.cx, h.window.into());
    assert!(!h.dragging());
    // The button is still held (released in the other application).
    h.send(vec![drag_to(at + offset(90.0, 70.0))]);
    assert_eq!(h.popup(), start + offset(30.0, 20.0));
}
