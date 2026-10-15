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
//! window draws into it. Dialog windows work on the test platform too.

use std::collections::HashMap;

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, Entity, FocusHandle, Global, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString, Size, Styled,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    anchored, deferred, div, point, px,
};

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
            appears_transparent: false,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AppContext as _, Context, FocusHandle, Focusable, TestAppContext, VisualTestContext, size,
    };

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
}
