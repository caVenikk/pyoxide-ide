//! Independent native surfaces for transient menus. Model and command ownership
//! stay in the opening window; each visible level has its own window identity.

use crate::theme::ThemeExt as _;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, EntityId, FocusHandle,
    Global, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, Size, Styled,
    Subscription, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    canvas, div, point, px,
};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

type Dismiss = Rc<dyn Fn(&mut App)>;
type Draw = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key(pub EntityId, pub usize);
struct Surface {
    window: AnyWindowHandle,
    owner: AnyWindowHandle,
    parent: AnyWindowHandle,
    order: usize,
    keyboard: bool,
    focus: FocusHandle,
}
#[derive(Default)]
struct Surfaces(HashMap<Key, Surface>, HashSet<EntityId>);
impl Global for Surfaces {}

pub fn enabled() -> bool {
    cfg!(target_os = "macos") && crate::pointer::native()
}

pub(crate) fn contains(id: EntityId, cx: &App) -> bool {
    cx.try_global::<Surfaces>()
        .is_some_and(|s| s.0.keys().any(|k| k.0 == id))
}

/// The opening window of a model's native surface, for action and focus routing.
pub fn owner(id: EntityId, cx: &App) -> Option<AnyWindowHandle> {
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

pub(crate) fn fit_bounds(
    parent: &Window,
    at: Point<Pixels>,
    size: Size<Pixels>,
    cx: &App,
) -> Bounds<Pixels> {
    let mut origin = screen_origin(parent) + at;
    if let Some(display) = parent.display(cx) {
        let bounds = display.bounds();
        origin.x = origin.x.min(bounds.right() - size.width).max(bounds.left());
        origin.y = origin
            .y
            .min(bounds.bottom() - size.height)
            .max(bounds.top());
    }
    Bounds::new(origin, size)
}

struct Host {
    owner: AnyWindowHandle,
    draw: Draw,
    _subscriptions: Vec<Subscription>,
}
impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = (self.draw)(window, cx);
        let theme = cx.theme();
        let owner = self.owner;
        div()
            .on_key_down(move |event, _, cx| {
                let function_key = event
                    .keystroke
                    .key
                    .strip_prefix('f')
                    .and_then(|key| key.parse::<u8>().ok())
                    .is_some_and(|key| (1..=24).contains(&key));
                if event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.control
                    || function_key
                {
                    let key = event.keystroke.clone();
                    cx.defer(move |cx| {
                        let _ =
                            owner.update(cx, |_, window, cx| window.dispatch_keystroke(key, cx));
                    });
                    cx.stop_propagation();
                }
            })
            .size_full()
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(themes::UiRole::Text))
            .child(
                div().absolute().top_0().left_0().child(content).child(
                    canvas(
                        |bounds, window, cx| {
                            let size = bounds.size;
                            if size.width > px(0.0)
                                && size.height > px(0.0)
                                && size != window.viewport_size()
                            {
                                window.defer(cx, move |window, cx| {
                                    crate::dialog_window::native::fit(window, size, cx)
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

pub(crate) struct SurfaceOptions {
    pub key: Key,
    pub at: Point<Pixels>,
    pub size: Size<Pixels>,
    pub focus: FocusHandle,
    pub persistent: bool,
    pub keyboard: bool,
}

pub(crate) fn show<T: 'static>(
    model: &gpui::Entity<T>,
    options: SurfaceOptions,
    draw: Draw,
    dismiss: Dismiss,
    parent: &mut Window,
    cx: &mut App,
) {
    let SurfaceOptions {
        key,
        at,
        size,
        focus,
        persistent,
        keyboard,
    } = options;
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
    let origin = fit_bounds(parent, at, size, cx).origin;
    if let Some(existing) = cx.global::<Surfaces>().0.get(&key).map(|s| s.window) {
        if let Some(existing) = existing.downcast::<Host>() {
            let _ = existing.update(cx, |host, window, cx| {
                host.draw = draw;
                crate::dialog_window::native::place(window, origin, size, Some(parent), cx);
                cx.notify();
            });
        }
        return;
    }
    let above = crate::dialog_window::native::window_number(parent);
    let activate = keyboard && parent.is_window_active();
    cx.global_mut::<Surfaces>().1.insert(key.0);
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
                owner,
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
    cx.global_mut::<Surfaces>().1.remove(&key.0);
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
            parent: parent.window_handle(),
            order,
            keyboard,
            focus,
        },
    );
    let _ = handle.update(cx, |_, window, _| {
        crate::dialog_window::native::popup_behavior(window, activate && !persistent);
        crate::dialog_window::native::order_above(window, above);
        if !keyboard {
            crate::dialog_window::native::keep_owner_focus(window);
        }
        if activate {
            window.activate_window();
        }
    });
}

/// A persistent custom popup whose owner controls closing and focus restoration.
pub trait Popup: Render + gpui::Focusable + Sized {
    fn dismiss_popup(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn dismiss_outside(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

type CloseView = Rc<dyn Fn(&mut Window, &mut App)>;
type Watch = Rc<dyn Fn(&mut App)>;

/// Presents a custom chooser in an independent window on macOS.
pub fn view<T: Render + gpui::Focusable + gpui::EventEmitter<gpui::DismissEvent>>(
    model: &gpui::Entity<T>,
) -> AnyElement {
    view_with_dismiss(model, |_, _, cx| cx.emit(gpui::DismissEvent))
}

/// Like [`view`], with the model's own close/focus lifecycle on deactivation.
pub fn view_with_dismiss<T: Render + gpui::Focusable + gpui::EventEmitter<gpui::DismissEvent>>(
    model: &gpui::Entity<T>,
    close: fn(&mut T, &mut Window, &mut Context<T>),
) -> AnyElement {
    if !enabled() {
        return model.clone().into_any_element();
    }
    let weak = model.downgrade();
    let dismiss: CloseView = Rc::new(move |window, cx| {
        let _ = weak.update(cx, |model, cx| close(model, window, cx));
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
        keyboard: true,
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
        keyboard: true,
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
    surface_behavior(
        model,
        level,
        extent,
        focus,
        render,
        dismiss,
        (persistent, true),
    )
}

/// A mouse-driven popup that initially keeps keyboard input in its owner.
pub fn passive_surface<T: Render + gpui::Focusable>(
    model: &gpui::Entity<T>,
    level: usize,
    extent: Size<Pixels>,
    focus: FocusHandle,
    render: fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement,
    dismiss: fn(&mut T, &mut Window, &mut Context<T>),
    persistent: bool,
) -> AnyElement {
    surface_behavior(
        model,
        level,
        extent,
        focus,
        render,
        dismiss,
        (persistent, false),
    )
}

fn surface_behavior<T: Render + gpui::Focusable>(
    model: &gpui::Entity<T>,
    level: usize,
    extent: Size<Pixels>,
    focus: FocusHandle,
    render: fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement,
    dismiss: fn(&mut T, &mut Window, &mut Context<T>),
    behavior: (bool, bool),
) -> AnyElement {
    let (persistent, keyboard) = behavior;
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
        keyboard,
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
    keyboard: bool,
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
            size: gpui::size(
                if size.width == px(0.0) {
                    gpui::relative(1.0).into()
                } else {
                    size.width.into()
                },
                size.height.into(),
            ),
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
        // Keep the popup's former focus node in the owner's dispatch ancestry.
        let focus = self
            .focus
            .clone()
            .or_else(|| {
                cx.try_global::<Surfaces>()
                    .and_then(|surfaces| surfaces.0.get(&Key(self.model.entity_id(), self.level)))
                    .map(|surface| surface.focus.clone())
            })
            .or_else(|| window.focused(cx))
            .unwrap_or_else(|| self.model.read(cx).focus_handle(cx));
        window.set_focus_handle(&focus, cx);
        let model = self.model.clone();
        let owner = window.window_handle();
        let dismiss = self.dismiss.clone();
        let watch = self.watch.clone();
        let persistent = self.persistent;
        let level = self.level;
        let keyboard = self.keyboard;
        let custom_draw = self.draw.clone();
        let custom_focus = Some(focus);
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
                SurfaceOptions {
                    key,
                    at: bounds.origin,
                    size: bounds.size,
                    focus,
                    persistent,
                    keyboard,
                },
                draw,
                dismiss,
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
        s.1.contains(&id)
            || s.0
                .iter()
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
            .filter(|s| s.owner == window && s.keyboard)
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
pub fn place(window: &mut Window, bounds: Bounds<Pixels>, cx: &mut App) {
    crate::dialog_window::native::place(window, bounds.origin, bounds.size, None, cx);
}

/// The immediate native parent, including a parent popup for a nested surface.
pub fn parent_of(window: AnyWindowHandle, cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Surfaces>()?
        .0
        .values()
        .find(|s| s.window == window)
        .map(|s| s.parent)
}

/// Whether a specific level is rendered in this window.
pub fn displays_level(id: EntityId, level: usize, window: &Window, cx: &App) -> bool {
    cx.try_global::<Surfaces>().is_some_and(|s| {
        s.0.get(&Key(id, level))
            .is_some_and(|surface| surface.window == window.window_handle())
    })
}

/// An editor-attached chooser that keeps keyboard input in its owner. Its
/// controller closes it with `close(content.entity_id(), cx)` when hidden.
pub fn attached_view<T: Render + gpui::Focusable>(
    owner: &gpui::Entity<T>,
    content: gpui::AnyView,
    max_height: Pixels,
) -> AnyElement {
    Attached {
        owner: owner.clone(),
        content,
        max_height,
    }
    .into_any_element()
}

struct Attached<T: Render + gpui::Focusable> {
    owner: gpui::Entity<T>,
    content: gpui::AnyView,
    max_height: Pixels,
}
impl<T: Render + gpui::Focusable> IntoElement for Attached<T> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<T: Render + gpui::Focusable> gpui::Element for Attached<T> {
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
        let mut child = self.content.clone().into_any_element();
        let mut size = child.layout_as_root(
            gpui::size(
                gpui::AvailableSpace::MaxContent,
                gpui::AvailableSpace::MaxContent,
            ),
            window,
            cx,
        );
        size.height = size.height.min(self.max_height);
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
        let owner = self.owner.clone();
        let content = self.content.downgrade();
        let key = Key(self.content.entity_id(), 0);
        let height = self.max_height;
        window.defer(cx, move |window, cx| {
            let draw: Draw = Rc::new(move |_, _| {
                div()
                    .max_h(height)
                    .overflow_hidden()
                    .children(content.upgrade())
                    .into_any_element()
            });
            let focus = window
                .focused(cx)
                .unwrap_or_else(|| owner.read(cx).focus_handle(cx));
            show(
                &owner,
                SurfaceOptions {
                    key,
                    at: bounds.origin,
                    size: bounds.size,
                    focus,
                    persistent: true,
                    keyboard: false,
                },
                draw,
                Rc::new(|_| {}),
                window,
                cx,
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Focusable, TestAppContext, size};

    crate::ui_commands! {
        COMMANDS, scope = "NativeOwner";
        Ping => "native-test.owner-command", "Owner command", "Tests native popup routing.";
    }

    struct Model(FocusHandle);

    struct Owner {
        focus: FocusHandle,
        model: gpui::Entity<Model>,
        commands: usize,
    }
    impl Render for Owner {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context("NativeOwner")
                .on_action(cx.listener(|this, _: &Ping, _, _| this.commands += 1))
                .size_full()
                .child(
                    div()
                        .key_context("NewTarget")
                        .track_focus(&self.focus)
                        .h(px(20.0)),
                )
                .child(Portal {
                    model: self.model.clone(),
                    extent: Some(size(px(120.0), px(60.0))),
                    dismiss: Rc::new(|_, _| {}),
                    watch: None,
                    persistent: true,
                    draw: None,
                    focus: None,
                    level: 0,
                    keyboard: true,
                })
        }
    }
    impl Focusable for Model {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.0.clone()
        }
    }
    impl Render for Model {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.0).w(px(120.0)).h(px(60.0))
        }
    }

    /// Call the presenter directly: ordinary headless rendering deliberately
    /// uses in-window popups, so it cannot exercise native-window ownership.
    #[gpui::test]
    fn presenter_tracks_levels_and_keeps_passive_input_in_owner(cx: &mut TestAppContext) {
        cx.update(|cx| crate::theme::init(cx, themes::ThemeKind::Dark));
        let parent = cx.add_window(|_, cx| Model(cx.focus_handle()));
        let model = cx.update(|cx| cx.new(|cx| Model(cx.focus_handle())));
        let id = model.entity_id();
        let open = |level, keyboard, cx: &mut TestAppContext| {
            parent
                .update(cx, |_, window, cx| {
                    show(
                        &model,
                        SurfaceOptions {
                            key: Key(id, level),
                            at: point(px(20.0), px(30.0)),
                            size: size(px(120.0), px(60.0)),
                            focus: model.read(cx).focus_handle(cx),
                            persistent: true,
                            keyboard,
                        },
                        Rc::new(|_, _| div().w(px(120.0)).h(px(60.0)).into_any_element()),
                        Rc::new(|_| {}),
                        window,
                        cx,
                    );
                })
                .unwrap();
            cx.run_until_parked();
        };
        open(0, false, cx);
        assert_eq!(cx.update(|cx| windows_of(parent.into(), cx).len()), 1);
        assert!(cx.update(|cx| innermost(parent.into(), cx)) == parent.into());
        open(1, true, cx);
        let child = cx.update(|cx| innermost(parent.into(), cx));
        assert!(child != parent.into());
        assert!(cx.update(|cx| owner(id, cx)) == Some(parent.into()));
        assert_eq!(cx.update(|cx| windows_of(parent.into(), cx).len()), 2);
        // Re-presenting a level reuses its identity rather than leaking windows.
        open(1, true, cx);
        assert!(cx.update(|cx| innermost(parent.into(), cx)) == child);
        cx.update(|cx| close_level(id, 1, cx));
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| windows_of(parent.into(), cx).len()), 1);
        assert!(cx.update(|cx| innermost(parent.into(), cx)) == parent.into());
        cx.update(|cx| close(id, cx));
        cx.run_until_parked();
        assert!(cx.update(|cx| windows_of(parent.into(), cx).is_empty()));
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    }
    #[gpui::test]
    fn unhandled_popup_commands_and_shortcuts_reach_owner(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::theme::init(cx, themes::ThemeKind::Dark);
            crate::command::register(cx, COMMANDS);
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-p", Ping, Some("NativeOwner")),
                gpui::KeyBinding::new("f9", Ping, Some("NativeOwner")),
                gpui::KeyBinding::new("cmd-t", Ping, Some("NewTarget")),
            ]);
        });
        let model = cx.update(|cx| cx.new(|cx| Model(cx.focus_handle())));
        let parent = cx.add_window(|window, cx| {
            let focus = cx.focus_handle();
            // Constructors focus the popup while it still belongs to the owner.
            // Its portal must keep that focused node on the owner's action path.
            window.focus(&model.read(cx).0);
            Owner {
                focus,
                model: model.clone(),
                commands: 0,
            }
        });
        cx.run_until_parked();
        cx.update_window(parent.into(), |_, window, cx| {
            window.draw(cx).clear();
            crate::command::invoke(
                "native-test.owner-command",
                serde_json::Value::Null,
                window,
                cx,
            )
            .unwrap();
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(parent.read_with(cx, |owner, _| owner.commands).unwrap(), 1);
        let popup = cx.update(|cx| innermost(parent.into(), cx));
        popup
            .update(cx, |_, window, cx| {
                window.draw(cx).clear();
                window.dispatch_keystroke(gpui::Keystroke::parse("cmd-p").unwrap(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(parent.read_with(cx, |owner, _| owner.commands).unwrap(), 2);
        popup
            .update(cx, |_, window, cx| {
                window.dispatch_keystroke(gpui::Keystroke::parse("f9").unwrap(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(parent.read_with(cx, |owner, _| owner.commands).unwrap(), 3);
        // A pinned popup remains while another owner field takes focus. Its
        // portal must not steal that new field's dispatch node on the next frame.
        parent
            .update(cx, |owner, window, _| window.focus(&owner.focus))
            .unwrap();
        cx.update_window(parent.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.dispatch_keystroke(gpui::Keystroke::parse("cmd-t").unwrap(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(parent.read_with(cx, |owner, _| owner.commands).unwrap(), 4);
        cx.update_window(parent.into(), |_, window, cx| {
            crate::dialog_window::open_popup(
                window,
                point(px(5.0), px(5.0)),
                size(px(120.0), px(60.0)),
                false,
                cx,
                |_, cx| cx.new(|cx| Model(cx.focus_handle())).into(),
            )
            .unwrap();
        })
        .unwrap();
        cx.run_until_parked();
        let blocked = cx
            .update_window(parent.into(), |_, window, cx| {
                crate::command::invoke(
                    "native-test.owner-command",
                    serde_json::Value::Null,
                    window,
                    cx,
                )
            })
            .unwrap();
        assert!(
            blocked.is_err(),
            "a real modal must still block owner commands"
        );
        cx.run_until_parked();
        assert_eq!(parent.read_with(cx, |owner, _| owner.commands).unwrap(), 4);
    }
}
