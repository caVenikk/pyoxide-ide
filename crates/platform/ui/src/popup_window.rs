//! Independent native surfaces for transient menus. Model and command ownership
//! stay in the opening window; each visible level has its own window identity.

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, EntityId, FocusHandle,
    Global, IntoElement, ParentElement, Pixels, Point, Render, Size, Styled, Subscription, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, canvas, div, point, px,
};
use std::{collections::HashMap, rc::Rc};

type Dismiss = Rc<dyn Fn(&mut App)>;
type Draw = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key(pub EntityId, pub usize);
struct Surface {
    window: AnyWindowHandle,
    owner: AnyWindowHandle,
    order: usize,
}
#[derive(Default)]
struct Surfaces(HashMap<Key, Surface>);
impl Global for Surfaces {}

pub fn enabled() -> bool {
    cfg!(target_os = "macos") && crate::pointer::native()
}

pub(crate) fn contains(id: EntityId, cx: &App) -> bool {
    cx.try_global::<Surfaces>()
        .is_some_and(|s| s.0.keys().any(|k| k.0 == id))
}

pub(crate) fn owner(id: EntityId, cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Surfaces>()?
        .0
        .iter()
        .find(|(k, _)| k.0 == id)
        .map(|(_, s)| s.owner)
}

pub(crate) fn close_from(id: EntityId, level: usize, cx: &mut App) {
    if !cx.has_global::<Surfaces>() {
        return;
    }
    let surfaces = cx.global_mut::<Surfaces>();
    let keys: Vec<_> = surfaces
        .0
        .keys()
        .filter(|k| k.0 == id && k.1 >= level)
        .copied()
        .collect();
    let windows: Vec<_> = keys
        .into_iter()
        .filter_map(|k| surfaces.0.remove(&k))
        .collect();
    cx.defer(move |cx| {
        for surface in windows {
            let _ = surface
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
    });
}

pub fn screen_origin(window: &Window) -> Point<Pixels> {
    let frame = window.bounds();
    let viewport = window.viewport_size();
    point(
        frame.left() + (frame.size.width - viewport.width) / 2.0,
        frame.bottom() - viewport.height,
    )
}

struct Host {
    draw: Draw,
    _subscriptions: Vec<Subscription>,
}
impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = (self.draw)(window, cx);
        div()
            .size_full()
            .child(
                div().absolute().top_0().left_0().child(content).child(
                    canvas(
                        |bounds, window, cx| {
                            let size = bounds.size;
                            if size.width > px(0.0)
                                && size.height > px(0.0)
                                && size != window.viewport_size()
                            {
                                window.defer(cx, move |window, _| {
                                    crate::dialog_window::native::fit(window, size)
                                });
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                ),
            )
            .child(crate::pointer::frame())
    }
}

pub(crate) fn show<T: 'static>(
    model: &gpui::Entity<T>,
    key: Key,
    at: Point<Pixels>,
    size: Size<Pixels>,
    focus: FocusHandle,
    draw: Draw,
    dismiss: Dismiss,
    persistent: bool,
    parent: &mut Window,
    cx: &mut App,
) {
    if !cx.has_global::<Surfaces>() {
        cx.set_global(Surfaces::default());
        cx.on_window_closed(|cx| {
            let open = cx.windows();
            let stale: Vec<_> = cx
                .global::<Surfaces>()
                .0
                .iter()
                .filter(|(_, surface)| {
                    !open.contains(&surface.owner) || !open.contains(&surface.window)
                })
                .map(|(key, _)| *key)
                .collect();
            for key in stale {
                close_from(key.0, key.1, cx);
            }
        })
        .detach();
    }
    let owner = owner(key.0, cx).unwrap_or(parent.window_handle());
    let mut origin = screen_origin(parent) + at;
    if let Some(display) = parent.display(cx) {
        let bounds = display.bounds();
        origin.x = origin.x.min(bounds.right() - size.width).max(bounds.left());
        origin.y = origin
            .y
            .min(bounds.bottom() - size.height)
            .max(bounds.top());
    }
    if let Some(existing) = cx.global::<Surfaces>().0.get(&key).map(|s| s.window) {
        let _ = existing.update(cx, |_, window, _| {
            crate::dialog_window::native::place(window, origin, size, Some(parent))
        });
        return;
    }
    let above = crate::dialog_window::native::window_number(parent);
    let activate = parent.is_window_active();
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size))),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: parent.display(cx).map(|d| d.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        |window, cx| {
            window.focus(&focus);
            cx.new(|cx| Host {
                draw,
                _subscriptions: vec![
                    cx.observe(model, |_, _, cx| cx.notify()),
                    cx.observe_release(model, move |_, _, cx| close_from(key.0, 0, cx)),
                    cx.observe_window_activation(window, move |_, window, cx| {
                        if persistent || window.is_window_active() {
                            return;
                        }
                        let dismiss = dismiss.clone();
                        cx.spawn(async move |_, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(30))
                                .await;
                            let _ = cx.update(|cx| {
                                if !family_active(key.0, cx)
                                    && !crate::dialog_window::native::capturing()
                                {
                                    dismiss(cx);
                                    close_from(key.0, 0, cx);
                                }
                            });
                        })
                        .detach();
                    }),
                ],
            })
        },
    );
    let Ok(handle) = handle else { return };
    let order = cx
        .global::<Surfaces>()
        .0
        .values()
        .map(|s| s.order)
        .max()
        .unwrap_or(0)
        + 1;
    cx.global_mut::<Surfaces>().0.insert(
        key,
        Surface {
            window: handle.into(),
            owner,
            order,
        },
    );
    let _ = handle.update(cx, |_, window, _| {
        crate::dialog_window::native::popup_behavior(window, activate && !persistent);
        crate::dialog_window::native::order_above(window, above);
        if activate {
            window.activate_window();
        }
    });
}

/// A persistent custom popup whose owner controls closing and focus restoration.
pub trait Popup: Render + gpui::Focusable + Sized {
    fn dismiss_popup(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

type CloseView = Rc<dyn Fn(&mut Window, &mut App)>;
type Watch = Rc<dyn Fn(&mut App)>;

/// Presents a custom chooser in an independent window on macOS.
pub fn view<T: Render + gpui::Focusable + gpui::EventEmitter<gpui::DismissEvent>>(
    model: &gpui::Entity<T>,
) -> AnyElement {
    if !enabled() {
        return model.clone().into_any_element();
    }
    let weak = model.downgrade();
    let dismiss: CloseView = Rc::new(move |_, cx| {
        let _ = weak.update(cx, |_, cx| cx.emit(gpui::DismissEvent));
    });
    let weak = model.downgrade();
    let watch: Watch = Rc::new(move |cx| {
        if let Some(model) = weak.upgrade() {
            let id = model.entity_id();
            cx.subscribe(&model, move |_, _: &gpui::DismissEvent, cx| {
                close_from(id, 0, cx)
            })
            .detach();
        }
    });
    Portal {
        model: model.clone(),
        extent: None,
        dismiss,
        watch: Some(watch),
        persistent: false,
        draw: None,
        focus: None,
        level: 0,
    }
    .into_any_element()
}

/// A fixed-size portal, avoiding recursive measurement of a self-presenting frame.
pub fn frame<T: Popup>(model: &gpui::Entity<T>, extent: Size<Pixels>) -> AnyElement {
    let weak = model.downgrade();
    let dismiss: CloseView = Rc::new(move |window, cx| {
        let _ = weak.update(cx, |model, cx| model.dismiss_popup(window, cx));
    });
    Portal {
        model: model.clone(),
        extent: Some(extent),
        dismiss,
        watch: None,
        persistent: true,
        draw: None,
        focus: None,
        level: 0,
    }
    .into_any_element()
}

/// Hosts one renderer of a model, such as a combo list or a nested options
/// panel. The returned element occupies `extent` at its caller's anchor.
/// The owner keeps its backdrop and explicitly calls [`close`] when hiding
/// a persistent model's surface. Rendering and dismissal use the same model.
pub fn surface<T: Render + gpui::Focusable>(
    model: &gpui::Entity<T>,
    level: usize,
    extent: Size<Pixels>,
    focus: FocusHandle,
    render: fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement,
    dismiss: fn(&mut T, &mut Window, &mut Context<T>),
    persistent: bool,
) -> AnyElement {
    let weak = model.downgrade();
    let draw: Draw = Rc::new(move |window, cx| {
        weak.update(cx, |model, cx| render(model, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    });
    let weak = model.downgrade();
    let dismiss: CloseView = Rc::new(move |window, cx| {
        let _ = weak.update(cx, |model, cx| dismiss(model, window, cx));
    });
    Portal {
        model: model.clone(),
        extent: Some(extent),
        dismiss,
        watch: None,
        persistent,
        draw: Some(draw),
        focus: Some(focus),
        level,
    }
    .into_any_element()
}

/// Closes every native surface owned by the model.
pub fn close(id: EntityId, cx: &mut App) {
    close_from(id, 0, cx);
}

/// Closes a nested custom surface and any levels above it, keeping its root.
pub fn close_level(id: EntityId, level: usize, cx: &mut App) {
    close_from(id, level, cx);
}

struct Portal<T: Render + gpui::Focusable> {
    model: gpui::Entity<T>,
    extent: Option<Size<Pixels>>,
    dismiss: CloseView,
    watch: Option<Watch>,
    persistent: bool,
    draw: Option<Draw>,
    focus: Option<FocusHandle>,
    level: usize,
}
impl<T: Render + gpui::Focusable> IntoElement for Portal<T> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<T: Render + gpui::Focusable> gpui::Element for Portal<T> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, AnyElement) {
        let mut child = self.model.clone().into_any_element();
        let size = self.extent.unwrap_or_else(|| {
            child.layout_as_root(
                gpui::size(
                    gpui::AvailableSpace::MaxContent,
                    gpui::AvailableSpace::MaxContent,
                ),
                window,
                cx,
            )
        });
        let style = gpui::Style {
            size: gpui::size(size.width.into(), size.height.into()),
            ..Default::default()
        };
        (window.request_layout(style, None, cx), child)
    }
    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        let model = self.model.clone();
        let owner = window.window_handle();
        let dismiss = self.dismiss.clone();
        let watch = self.watch.clone();
        let persistent = self.persistent;
        let level = self.level;
        let custom_draw = self.draw.clone();
        let custom_focus = self.focus.clone();
        window.defer(cx, move |window, cx| {
            let weak = model.downgrade();
            let draw: Draw = custom_draw.unwrap_or_else(|| {
                Rc::new(move |_: &mut Window, _: &mut App| {
                    weak.upgrade()
                        .map(|model| model.into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                })
            });
            let focus = custom_focus
                .or_else(|| window.focused(cx))
                .unwrap_or_else(|| model.read(cx).focus_handle(cx));
            let key = Key(model.entity_id(), level);
            let first = !contains(key.0, cx);
            let dismiss: Dismiss = Rc::new(move |cx| {
                let _ = owner.update(cx, |_, window, cx| dismiss(window, cx));
            });
            show(
                &model,
                key,
                bounds.origin,
                bounds.size,
                focus,
                draw,
                dismiss,
                persistent,
                window,
                cx,
            );
            if first && let Some(watch) = watch {
                watch(cx);
            }
        });
    }
    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut AnyElement,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}

/// Whether `window` is displaying `model` as a native popup surface.
pub fn displays(id: EntityId, window: &Window, cx: &App) -> bool {
    cx.try_global::<Surfaces>().is_some_and(|s| {
        s.0.iter()
            .any(|(key, s)| key.0 == id && s.window == window.window_handle())
    })
}

fn root_owner(mut window: AnyWindowHandle, cx: &App) -> AnyWindowHandle {
    let Some(surfaces) = cx.try_global::<Surfaces>() else {
        return window;
    };
    while let Some(surface) = surfaces.0.values().find(|s| s.window == window) {
        window = surface.owner;
    }
    window
}

fn family_active(id: EntityId, cx: &mut App) -> bool {
    let Some(owner) = owner(id, cx) else {
        return false;
    };
    let root = root_owner(owner, cx);
    let mut windows = windows_of(root, cx);
    windows.push(root);
    windows.into_iter().any(|handle| {
        handle
            .update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false)
    })
}

/// The foremost native popup reachable from the window, for registry commands.
pub fn innermost(mut window: AnyWindowHandle, cx: &App) -> AnyWindowHandle {
    let Some(surfaces) = cx.try_global::<Surfaces>() else {
        return window;
    };
    loop {
        let next = surfaces
            .0
            .values()
            .filter(|s| s.owner == window)
            .max_by_key(|s| s.order)
            .map(|s| s.window);
        match next {
            Some(next) if next != window => window = next,
            _ => return window,
        }
    }
}

/// Native popup surfaces belonging to an opening window, in stacking order.
pub fn windows_of(window: AnyWindowHandle, cx: &App) -> Vec<AnyWindowHandle> {
    let Some(surfaces) = cx.try_global::<Surfaces>() else {
        return Vec::new();
    };
    let root = root_owner(window, cx);
    let mut matching: Vec<_> = surfaces
        .0
        .values()
        .filter(|s| root_owner(s.owner, cx) == root)
        .collect();
    matching.sort_by_key(|s| s.order);
    matching.into_iter().map(|s| s.window).collect()
}

/// Moves and fits a custom popup on its current display without activation.
pub fn place(window: &mut Window, bounds: Bounds<Pixels>) {
    crate::dialog_window::native::place(window, bounds.origin, bounds.size, None);
}
