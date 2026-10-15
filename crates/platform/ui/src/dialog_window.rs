//! Dialog windows, as the reference IDE opens its dialogs (`DialogWrapper`
//! in a window of its own): a separate OS window with the system title
//! bar, movable and resizable, centred on its parent window and modal to
//! it.
//!
//! While a dialog is open its parent window takes no input: [`blocker`]
//! covers it (a click there brings the dialog to the front), keystrokes
//! sent to it are dropped, and [`crate::command::invoke`] runs commands in
//! the dialog. On macOS the dialog is a child window of its parent, so it
//! stays above it. When the dialog closes, by [`close`] or by its title
//! bar's close button, the element of the parent that had the focus gets
//! it back.
//!
//! The dialog's view is the root of its window: nothing of the parent
//! window draws into it. On macOS a dialog can draw its own title bar in
//! the theme's colours under the system's window buttons ([`title_bar`]),
//! as the reference does.
//!
//! A popup of a dialog that may reach beyond it (the reference's popups
//! are windows of their own) opens with [`open_popup`]: a borderless window
//! fitted to its content, a child of the dialog in the same way.
//!
//! Dialog and popup windows work on the test platform too.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity,
    FocusHandle, FontWeight, Global, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Point, Render, SharedString, Size, StatefulInteractiveElement as _, Styled,
    Subscription, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, anchored, canvas, deferred, div, point, px,
};
use themes::UiRole;

use crate::theme::ThemeExt;

/// Height of a dialog's own title bar (the system's, under its buttons).
pub const TITLE_BAR: f32 = 28.0;
/// Where the title starts: after the window buttons
/// (`DialogWrapperPeerImpl` on macOS with a custom title bar).
const TITLE_LEFT: f32 = 84.0;

/// How a dialog window opens.
#[derive(Debug, Clone)]
pub struct DialogOptions {
    /// The window's title.
    pub title: SharedString,
    /// The size of its content.
    pub size: Size<Pixels>,
    /// The smallest size it can be resized to.
    pub min_size: Option<Size<Pixels>>,
    /// Activate the application and give the dialog the focus (off for
    /// windows opened in the background).
    pub activate: bool,
    /// The dialog draws its title bar itself ([`title_bar`]); macOS only,
    /// other platforms keep the system's.
    pub themed_title: bool,
}

/// The dialog draws its own title bar: on macOS when asked.
fn own_title(themed: bool) -> bool {
    themed && cfg!(target_os = "macos")
}

/// The height a dialog's own title bar takes from its content: add it to
/// the content's size where the dialog draws it.
pub fn title_height(themed: bool) -> f32 {
    if own_title(themed) { TITLE_BAR } else { 0.0 }
}

/// An open dialog window.
struct Modal {
    dialog: AnyWindowHandle,
    /// Had the focus in the parent window.
    return_focus: Option<FocusHandle>,
    activate: bool,
}

/// The open dialogs by their parent windows.
#[derive(Default)]
struct Modals {
    by_parent: HashMap<AnyWindowHandle, Modal>,
    _subscriptions: Vec<Subscription>,
}

impl Global for Modals {}

/// Installs the hooks that keep dialogs modal; idempotent.
fn ensure_init(cx: &mut App) {
    if cx.has_global::<Modals>() {
        return;
    }
    let subscriptions = vec![
        // Keystrokes to a window with an open dialog go nowhere.
        cx.intercept_keystrokes(|_, window, cx| {
            if let Some(dialog) = modal_of(window.window_handle(), cx) {
                cx.stop_propagation();
                let _ = dialog.update(cx, |_, window, _| window.activate_window());
            }
        }),
        // A dialog closed by its title bar: its parent gets the focus back.
        cx.on_window_closed(forget_closed),
    ];
    cx.set_global(Modals {
        by_parent: HashMap::new(),
        _subscriptions: subscriptions,
    });
}

/// Opens `build`'s view as a dialog window of `parent`, modal to it.
pub fn open<V: 'static + Render>(
    parent: &mut Window,
    options: DialogOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<WindowHandle<V>> {
    ensure_init(cx);
    let parent_handle = parent.window_handle();
    let return_focus = parent.focused(cx);
    let parent_bounds = parent.bounds();
    let origin =
        parent_bounds.center() - point(options.size.width / 2.0, options.size.height / 2.0);
    let bounds = Bounds::new(point(origin.x.floor(), origin.y.floor()), options.size);
    let window_options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(options.title.clone()),
            appears_transparent: own_title(options.themed_title),
            traffic_light_position: None,
        }),
        focus: options.activate,
        show: true,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: false,
        display_id: parent.display(cx).map(|d| d.id()),
        window_min_size: options.min_size,
        ..WindowOptions::default()
    };
    let handle = cx.open_window(window_options, build).ok()?;
    let dialog: AnyWindowHandle = handle.into();
    cx.global_mut::<Modals>().by_parent.insert(
        parent_handle,
        Modal {
            dialog,
            return_focus,
            activate: options.activate,
        },
    );
    let _ = dialog.update(cx, |_, window, _| {
        native::attach(parent, window);
        if options.activate {
            window.activate_window();
        }
    });
    parent.refresh();
    Some(handle)
}

/// The dialog open over `parent`, if any.
pub fn modal_of(parent: AnyWindowHandle, cx: &App) -> Option<AnyWindowHandle> {
    let modal = cx.try_global::<Modals>()?.by_parent.get(&parent)?;
    cx.windows().contains(&modal.dialog).then_some(modal.dialog)
}

/// The window that takes the input of `window`: the innermost dialog or
/// popup open over it, else itself.
pub fn innermost(window: AnyWindowHandle, cx: &App) -> AnyWindowHandle {
    let mut current = window;
    while let Some(child) = modal_of(current, cx) {
        current = child;
    }
    current
}

/// A dialog's title bar in the theme's colours, under the system's window
/// buttons; it moves the window like the system's (macOS drags it) and
/// zooms it on a double click. `None` where the system draws it.
pub fn title_bar(title: impl Into<SharedString>, themed: bool, cx: &App) -> Option<AnyElement> {
    if !own_title(themed) {
        return None;
    }
    let theme = cx.theme();
    Some(
        div()
            .id("dialog-title-bar")
            .debug_selector(|| "dialog-title-bar".into())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .h(px(TITLE_BAR))
            .pl(px(TITLE_LEFT))
            .pr(px(12.0))
            .bg(theme.color(UiRole::Surface))
            .text_color(theme.color(UiRole::Text))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .overflow_hidden()
            .child(title.into())
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .into_any_element(),
    )
}

/// The content of a popup window: drawn at its top left corner and
/// measured, so that the window fits it.
pub struct PopupHost {
    content: AnyView,
    fitted: Rc<Cell<Option<Size<Pixels>>>>,
}

impl Render for PopupHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let fitted = self.fitted.clone();
        div().size_full().child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(self.content.clone())
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            let size = bounds.size;
                            if fitted.get() != Some(size) && size.width > px(0.0) {
                                fitted.set(Some(size));
                                window.defer(cx, move |window, _| native::fit(window, size));
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
        )
    }
}

/// Opens `build`'s view as a popup window of `parent`: borderless, with
/// its top left corner at `at` (in `parent`'s coordinates), as large as
/// `max` at first and then fitted to the view. It takes `parent`'s input
/// while it is open, like a dialog, and closes with [`close`].
pub fn open_popup(
    parent: &mut Window,
    at: Point<Pixels>,
    max: Size<Pixels>,
    activate: bool,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> AnyView,
) -> Option<WindowHandle<PopupHost>> {
    ensure_init(cx);
    let parent_handle = parent.window_handle();
    let return_focus = parent.focused(cx);
    // The parent's content in screen coordinates (below a system title
    // bar, if it has one).
    let frame = parent.bounds();
    let viewport = parent.viewport_size();
    let content = point(
        frame.left() + (frame.size.width - viewport.width) / 2.0,
        frame.bottom() - viewport.height,
    );
    let mut origin = content + at;
    let mut size = max;
    if let Some(display) = parent.display(cx).map(|d| d.bounds()) {
        size.height = size.height.min(display.bottom() - origin.y - px(8.0));
        if origin.x + size.width > display.right() {
            origin.x = (display.right() - size.width).max(display.left());
        }
    }
    let window_options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(origin.x.floor(), origin.y.floor()),
            size,
        ))),
        titlebar: None,
        focus: activate,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: parent.display(cx).map(|d| d.id()),
        window_background: WindowBackgroundAppearance::Transparent,
        ..WindowOptions::default()
    };
    let handle = cx
        .open_window(window_options, |window, cx| {
            let content = build(window, cx);
            cx.new(|_| PopupHost {
                content,
                fitted: Rc::default(),
            })
        })
        .ok()?;
    let popup: AnyWindowHandle = handle.into();
    cx.global_mut::<Modals>().by_parent.insert(
        parent_handle,
        Modal {
            dialog: popup,
            return_focus,
            activate,
        },
    );
    let _ = popup.update(cx, |_, window, _| {
        native::attach(parent, window);
        if activate {
            window.activate_window();
        }
    });
    Some(handle)
}

/// The window a dialog was opened over.
pub fn parent_of(dialog: AnyWindowHandle, cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Modals>()?
        .by_parent
        .iter()
        .find(|(_, m)| m.dialog == dialog)
        .map(|(parent, _)| *parent)
}

/// Closes a dialog window; its parent takes input again and the element
/// that had the focus gets it back. Nothing for a dialog already closed.
pub fn close(dialog: AnyWindowHandle, cx: &mut App) {
    // A popup open over it goes first.
    if let Some(child) = modal_of(dialog, cx) {
        close(child, cx);
    }
    let modal = parent_of(dialog, cx).and_then(|parent| {
        cx.global_mut::<Modals>()
            .by_parent
            .remove(&parent)
            .map(|m| (parent, m))
    });
    // While the dialog itself is being updated (its title bar's close
    // button) it is not reachable and closes on its own.
    let _ = dialog.update(cx, |_, window, _| window.remove_window());
    // After the current update: the parent may be the window being
    // updated now (a dialog closed from one of its handlers).
    if let Some((parent, modal)) = modal {
        cx.defer(move |cx| give_back(parent, modal, cx));
    }
}

/// Forgets dialogs whose windows are gone.
fn forget_closed(cx: &mut App) {
    let open = cx.windows();
    if !cx.has_global::<Modals>() {
        return;
    }
    let modals = cx.global_mut::<Modals>();
    let closed: Vec<AnyWindowHandle> = modals
        .by_parent
        .iter()
        .filter(|(_, m)| !open.contains(&m.dialog))
        .map(|(p, _)| *p)
        .collect();
    let closed: Vec<(AnyWindowHandle, Modal)> = closed
        .into_iter()
        .filter_map(|p| modals.by_parent.remove(&p).map(|m| (p, m)))
        .collect();
    for (parent, modal) in closed {
        give_back(parent, modal, cx);
    }
}

fn give_back(parent: AnyWindowHandle, modal: Modal, cx: &mut App) {
    let _ = parent.update(cx, |_, window, _| {
        if let Some(focus) = &modal.return_focus {
            window.focus(focus);
        }
        if modal.activate {
            window.activate_window();
        }
        window.refresh();
    });
}

/// Covers a window that has a dialog open: the mouse reaches nothing in
/// it, and a click brings the dialog to the front. `None` without a
/// dialog.
pub fn blocker(window: &Window, cx: &App) -> Option<AnyElement> {
    let dialog = modal_of(window.window_handle(), cx)?;
    let size = window.viewport_size();
    let bring_front = move |_: &gpui::MouseDownEvent, _: &mut Window, cx: &mut App| {
        let _ = dialog.update(cx, |_, window, _| window.activate_window());
    };
    Some(
        deferred(
            anchored().position(point(px(0.0), px(0.0))).child(
                div()
                    .id("dialog-window-blocker")
                    .debug_selector(|| "dialog-window-blocker".into())
                    .occlude()
                    .w(size.width)
                    .h(size.height)
                    .on_mouse_down(MouseButton::Left, bring_front)
                    .on_mouse_down(MouseButton::Right, bring_front),
            ),
        )
        .with_priority(10)
        .into_any_element(),
    )
}

#[cfg(target_os = "macos")]
mod native {
    use gpui::Window;
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// `NSWindowAbove`.
    const ABOVE: isize = 1;

    #[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
    fn ns_window(window: &Window) -> Option<*mut Object> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };
        let view = appkit.ns_view.as_ptr() as *mut Object;
        // SAFETY: `view` is the live NSView of this window, on the main
        // thread.
        let window: *mut Object = unsafe { msg_send![view, window] };
        (!window.is_null()).then_some(window)
    }

    /// Sizes a popup window to `size`, its top left corner where it is.
    #[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
    pub fn fit(window: &mut Window, size: gpui::Size<gpui::Pixels>) {
        use cocoa_types::{NSPoint, NSRect, NSSize};
        if !crate::pointer::native() {
            window.resize(size);
            return;
        }
        let Some(ns) = ns_window(window) else {
            return;
        };
        // SAFETY: a live NSWindow, on the main thread.
        unsafe {
            let frame: NSRect = msg_send![ns, frame];
            let top_left = NSPoint {
                x: frame.origin.x,
                y: frame.origin.y + frame.size.height,
            };
            let content = NSSize {
                width: f64::from(f32::from(size.width)),
                height: f64::from(f32::from(size.height)),
            };
            let _: () = msg_send![ns, setContentSize: content];
            let _: () = msg_send![ns, setFrameTopLeftPoint: top_left];
            let _: () = msg_send![ns, invalidateShadow];
        }
    }

    /// Plain Cocoa geometry for `msg_send!` (`NSPoint`, `NSSize`,
    /// `NSRect`), laid out as AppKit's.
    mod cocoa_types {
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NSPoint {
            pub x: f64,
            pub y: f64,
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NSSize {
            pub width: f64,
            pub height: f64,
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NSRect {
            pub origin: NSPoint,
            pub size: NSSize,
        }

        unsafe impl objc::Encode for NSPoint {
            fn encode() -> objc::Encoding {
                // SAFETY: the type encoding of CGPoint on 64-bit.
                unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
            }
        }

        unsafe impl objc::Encode for NSSize {
            fn encode() -> objc::Encoding {
                // SAFETY: the type encoding of CGSize on 64-bit.
                unsafe { objc::Encoding::from_str("{CGSize=dd}") }
            }
        }

        unsafe impl objc::Encode for NSRect {
            fn encode() -> objc::Encoding {
                // SAFETY: the type encoding of CGRect on 64-bit.
                unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
            }
        }
    }

    /// Makes `dialog` a child window of `parent`: it stays above it.
    #[allow(unexpected_cfgs)] // objc's msg_send! checks a cfg of its own
    pub fn attach(parent: &Window, dialog: &Window) {
        // The test platform has no native windows.
        if !crate::pointer::native() {
            return;
        }
        let (Some(parent), Some(dialog)) = (ns_window(parent), ns_window(dialog)) else {
            return;
        };
        // SAFETY: both are live NSWindows, on the main thread.
        unsafe {
            let _: () = msg_send![parent, addChildWindow: dialog ordered: ABOVE];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn attach(_: &gpui::Window, _: &gpui::Window) {}

    /// Sizes a popup window to `size`; the window keeps its top left
    /// corner.
    pub fn fit(window: &mut gpui::Window, size: gpui::Size<gpui::Pixels>) {
        window.resize(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, FocusHandle, Focusable, TestAppContext, VisualTestContext, size};

    gpui::actions!(dialog_window_test, [Ping]);

    /// A window that records the actions and clicks it gets.
    struct Recorder {
        focus: FocusHandle,
        got: Vec<&'static str>,
    }

    impl Focusable for Recorder {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Render for Recorder {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("recorder")
                .key_context("Recorder")
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, _: &Ping, _, _| this.got.push("ping")))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.got.push("click")),
                )
                .size_full()
                .children(blocker(window, cx))
        }
    }

    fn recorder(cx: &mut Context<Recorder>) -> Recorder {
        Recorder {
            focus: cx.focus_handle(),
            got: Vec::new(),
        }
    }

    fn draw(window: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    #[gpui::test]
    fn a_dialog_window_is_modal_to_its_parent(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::theme::init(cx, themes::ThemeKind::Dark);
            cx.bind_keys([gpui::KeyBinding::new("cmd-p", Ping, Some("Recorder"))]);
        });
        let parent = cx.add_window(|_, cx| recorder(cx));
        let parent_any: AnyWindowHandle = parent.into();
        let parent_focus = parent
            .update(cx, |r, window, _| {
                window.focus(&r.focus);
                r.focus.clone()
            })
            .unwrap();
        draw(parent_any, cx);
        let dialog = parent
            .update(cx, |_, window, cx| {
                let options = DialogOptions {
                    title: "Dialog".into(),
                    size: size(px(400.0), px(300.0)),
                    min_size: None,
                    activate: true,
                    themed_title: false,
                };
                open(window, options, cx, |window, cx| {
                    cx.new(|cx| {
                        let r = recorder(cx);
                        window.focus(&r.focus);
                        r
                    })
                })
                .unwrap()
            })
            .unwrap();
        let dialog_any: AnyWindowHandle = dialog.into();
        draw(parent_any, cx);
        draw(dialog_any, cx);
        assert!(cx.update(|cx| modal_of(parent_any, cx)) == Some(dialog_any));
        assert!(cx.update(|cx| parent_of(dialog_any, cx)) == Some(parent_any));
        // The dialog's title and size.
        let bounds = dialog
            .update(cx, |_, window, _| window.viewport_size())
            .unwrap();
        assert_eq!(bounds, size(px(400.0), px(300.0)));
        // Keys and clicks do not reach the parent; the dialog gets keys.
        cx.simulate_keystrokes(parent_any, "cmd-p");
        let mut vcx = VisualTestContext::from_window(parent_any, cx);
        vcx.simulate_click(point(px(20.0), px(20.0)), gpui::Modifiers::none());
        assert!(parent.read_with(cx, |r, _| r.got.is_empty()).unwrap());
        cx.simulate_keystrokes(dialog_any, "cmd-p");
        assert_eq!(
            dialog.read_with(cx, |r, _| r.got.clone()).unwrap(),
            ["ping"]
        );
        // Closing it gives the parent its input and focus back.
        cx.update(|cx| close(dialog_any, cx));
        draw(parent_any, cx);
        assert!(cx.update(|cx| modal_of(parent_any, cx)).is_none());
        assert!(!cx.update(|cx| cx.windows().contains(&dialog_any)));
        let focused = parent
            .update(cx, |_, window, _| parent_focus.is_focused(window))
            .unwrap();
        assert!(focused, "the focus went back");
        cx.simulate_keystrokes(parent_any, "cmd-p");
        assert_eq!(
            parent.read_with(cx, |r, _| r.got.clone()).unwrap(),
            ["ping"]
        );
    }

    /// A popup window of a window: input goes to it while it is open, it
    /// fits its content, and closing it gives the focus back.
    #[gpui::test]
    fn a_popup_window_fits_its_content(cx: &mut TestAppContext) {
        cx.update(|cx| crate::theme::init(cx, themes::ThemeKind::Dark));
        let parent = cx.add_window(|_, cx| recorder(cx));
        let parent_any: AnyWindowHandle = parent.into();
        let parent_focus = parent
            .update(cx, |r, window, _| {
                window.focus(&r.focus);
                r.focus.clone()
            })
            .unwrap();
        draw(parent_any, cx);
        let popup = parent
            .update(cx, |_, window, cx| {
                open_popup(
                    window,
                    point(px(20.0), px(30.0)),
                    size(px(400.0), px(300.0)),
                    true,
                    cx,
                    |_, cx| cx.new(|_| KnownSize).into(),
                )
                .unwrap()
            })
            .unwrap();
        let popup_any: AnyWindowHandle = popup.into();
        draw(popup_any, cx);
        draw(popup_any, cx);
        assert!(cx.update(|cx| innermost(parent_any, cx)) == popup_any);
        let fitted = popup
            .update(cx, |_, window, _| window.viewport_size())
            .unwrap();
        assert_eq!(fitted, size(px(120.0), px(40.0)), "fitted to the content");
        cx.update(|cx| close(popup_any, cx));
        draw(parent_any, cx);
        assert!(cx.update(|cx| innermost(parent_any, cx)) == parent_any);
        let focused = parent
            .update(cx, |_, window, _| parent_focus.is_focused(window))
            .unwrap();
        assert!(focused, "the focus went back");
    }

    /// Content of a known size.
    struct KnownSize;

    impl Render for KnownSize {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(120.0)).h(px(40.0))
        }
    }
}
