//! Drawing above everything else: the one way to use GPUI's deferred draws.
//!
//! GPUI 0.2.2 draws a [`gpui::deferred`] element after the rest of the
//! window, but it aborts the process ("cannot call defer_draw during deferred
//! drawing") when a deferred element is drawn inside another one: a context
//! menu opened from a popup, an editor balloon in a field of a popup, a
//! dropdown inside a dialog. Whether two components nest is decided far from
//! either of them, so every caller uses [`deferred`] from this module, and
//! the direct one is refused by clippy (`clippy.toml`).
//!
//! Outside any deferred element [`deferred`] is GPUI's deferred draw. Inside
//! one, the child is drawn by the outermost deferred element once its own
//! child is drawn, so it still covers what it would have covered and is not
//! cut off by its parents:
//! - an [`Anchored`] child (what popups and menus use) is laid out on its own
//!   at the position it has in the tree, as GPUI does for tooltips;
//! - any other child keeps its place in the layout and is drawn in place,
//!   which is the only way to keep a layout that depends on its parents.

use std::any::TypeId;
use std::cell::{Cell, RefCell};

use gpui::{
    Anchored, AnyElement, App, AvailableSpace, Bounds, Display, Element, ElementId,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Position, Size,
    Style, TextStyle, TextStyleRefinement, Window,
};

thread_local! {
    /// More than zero while the child of a deferred element is laid out.
    static LAYOUT_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// One list per deferred element being prepainted: the anchored children
    /// found inside it, drawn after it.
    static NESTED: RefCell<Vec<Vec<Nested>>> = const { RefCell::new(Vec::new()) };
}

/// Draws `child` after the rest of the window, above it. Safe to nest: see
/// the module documentation.
pub fn deferred(child: impl IntoElement) -> Deferred {
    let child = child.into_element();
    let anchored = is_anchored(&child);
    Deferred {
        child: Some(child.into_any_element()),
        anchored,
        priority: 0,
        mode: Mode::Unknown,
    }
}

fn is_anchored<E: 'static>(_: &E) -> bool {
    TypeId::of::<E>() == TypeId::of::<Anchored>()
}

/// The element [`deferred`] builds.
pub struct Deferred {
    child: Option<AnyElement>,
    /// The child positions itself in the window (see the module docs).
    anchored: bool,
    priority: usize,
    mode: Mode,
}

/// How a [`Deferred`] draws, decided when it is laid out.
enum Mode {
    Unknown,
    /// Outside any deferred element: GPUI's deferred draw of the child,
    /// wrapped in a [`Scope`].
    Top,
    /// Inside a deferred element, anchored: a placeholder in the tree; the
    /// child goes to the enclosing [`Scope`] when prepainted.
    Lifted,
    /// Inside a deferred element, not anchored: drawn in place.
    Inline,
    /// [`Mode::Lifted`] without an enclosing [`Scope`] (not expected): the
    /// child laid out on its own and drawn in place.
    LiftedHere(Box<Nested>),
}

impl Deferred {
    /// Higher priorities are drawn above lower ones (GPUI's deferred
    /// priority); a deferred element inside another one is drawn just above
    /// that one.
    pub fn with_priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }

    /// The same as [`Deferred::with_priority`].
    pub fn priority(self, priority: usize) -> Self {
        self.with_priority(priority)
    }
}

/// An anchored child lifted out of a deferred element: drawn by the
/// enclosing [`Scope`] after its own child.
pub struct Nested {
    element: AnyElement,
    origin: Point<Pixels>,
    text_style: TextStyleRefinement,
}

impl Nested {
    fn layout_and_prepaint(&mut self, index: usize, window: &mut Window, cx: &mut App) {
        let origin = self.origin;
        let style = self.text_style.clone();
        let element = &mut self.element;
        window.with_id(nested_id(index), |window| {
            window.with_text_style(Some(style), |window| {
                LAYOUT_DEPTH.set(LAYOUT_DEPTH.get() + 1);
                element.layout_as_root(
                    Size {
                        width: AvailableSpace::MaxContent,
                        height: AvailableSpace::MaxContent,
                    },
                    window,
                    cx,
                );
                LAYOUT_DEPTH.set(LAYOUT_DEPTH.get() - 1);
                element.prepaint_at(origin, window, cx);
            })
        });
    }

    fn paint(&mut self, index: usize, window: &mut Window, cx: &mut App) {
        let style = self.text_style.clone();
        let element = &mut self.element;
        window.with_id(nested_id(index), |window| {
            window.with_text_style(Some(style), |window| element.paint(window, cx))
        });
    }
}

/// Keeps the element state of two lifted children of one scope apart.
fn nested_id(index: usize) -> ElementId {
    ElementId::NamedInteger("ui-deferred-nested".into(), index as u64)
}

/// The whole of a text style as a refinement, so a lifted child keeps the
/// text style it has in the tree.
fn refinement(style: TextStyle) -> TextStyleRefinement {
    TextStyleRefinement {
        color: Some(style.color),
        font_family: Some(style.font_family),
        font_features: Some(style.font_features),
        font_fallbacks: style.font_fallbacks,
        font_size: Some(style.font_size),
        line_height: Some(style.line_height),
        font_weight: Some(style.font_weight),
        font_style: Some(style.font_style),
        background_color: style.background_color,
        underline: style.underline,
        strikethrough: style.strikethrough,
        white_space: Some(style.white_space),
        text_overflow: style.text_overflow,
        text_align: Some(style.text_align),
        line_clamp: style.line_clamp,
    }
}

impl Element for Deferred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let nested = LAYOUT_DEPTH.get() > 0;
        let layout_id = if !nested {
            self.mode = Mode::Top;
            let child = self.child.take().expect("laid out once");
            let mut scope = Scope {
                child,
                nested: Vec::new(),
            }
            .into_any_element();
            let layout_id = scope.request_layout(window, cx);
            self.child = Some(scope);
            layout_id
        } else if self.anchored {
            self.mode = Mode::Lifted;
            // The box `anchored()` itself has in the tree: the lifted child
            // is placed where this one ends up.
            let style = Style {
                position: Position::Absolute,
                display: Display::Flex,
                ..Style::default()
            };
            window.request_layout(style, [], cx)
        } else {
            self.mode = Mode::Inline;
            self.child
                .as_mut()
                .expect("laid out once")
                .request_layout(window, cx)
        };
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let in_scope = NESTED.with_borrow(|stack| !stack.is_empty());
        match self.mode {
            Mode::Top if !in_scope => {
                let child = self.child.take().expect("prepainted once");
                let offset = window.element_offset();
                // The one deferred draw; a nested one would abort.
                #[allow(clippy::disallowed_methods)]
                window.defer_draw(child, offset, self.priority);
            }
            // Laid out outside any deferred element and prepainted inside
            // one (not expected): the scope is drawn in place.
            Mode::Top | Mode::Inline => {
                if let Some(child) = self.child.as_mut() {
                    child.prepaint(window, cx);
                }
            }
            Mode::Lifted => {
                let nested = Nested {
                    element: self.child.take().expect("prepainted once"),
                    origin: bounds.origin,
                    text_style: refinement(window.text_style()),
                };
                if in_scope {
                    NESTED.with_borrow_mut(|stack| {
                        if let Some(list) = stack.last_mut() {
                            list.push(nested);
                        }
                    });
                } else {
                    let mut nested = Box::new(nested);
                    nested.layout_and_prepaint(0, window, cx);
                    self.mode = Mode::LiftedHere(nested);
                }
            }
            Mode::LiftedHere(_) | Mode::Unknown => {}
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        match &mut self.mode {
            Mode::Top | Mode::Inline => {
                if let Some(child) = self.child.as_mut() {
                    child.paint(window, cx);
                }
            }
            Mode::LiftedHere(nested) => nested.paint(0, window, cx),
            Mode::Lifted | Mode::Unknown => {}
        }
    }
}

impl IntoElement for Deferred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The child of a top-level [`Deferred`], as GPUI draws it after the
/// window: marks everything inside it as nested and draws the anchored
/// children lifted out of it after it.
struct Scope {
    child: AnyElement,
    nested: Vec<Nested>,
}

impl Element for Scope {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        LAYOUT_DEPTH.set(LAYOUT_DEPTH.get() + 1);
        let layout_id = self.child.request_layout(window, cx);
        LAYOUT_DEPTH.set(LAYOUT_DEPTH.get() - 1);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        NESTED.with_borrow_mut(|stack| stack.push(Vec::new()));
        self.child.prepaint(window, cx);
        // Children lifted out of a lifted child join the list and are drawn
        // after it, above it.
        let mut next = 0;
        while let Some(mut nested) = NESTED.with_borrow_mut(|stack| {
            let list = stack.last_mut()?;
            (!list.is_empty()).then(|| list.remove(0))
        }) {
            nested.layout_and_prepaint(next, window, cx);
            self.nested.push(nested);
            next += 1;
        }
        NESTED.with_borrow_mut(|stack| stack.pop());
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        for (index, nested) in self.nested.iter_mut().enumerate() {
            nested.paint(index, window, cx);
        }
    }
}

impl IntoElement for Scope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui::{
        Context, DismissEvent, Entity, InteractiveElement, Modifiers, MouseButton, ParentElement,
        Render, StatefulInteractiveElement, Styled, TestAppContext, VisualTestContext, anchored,
        div, point, px,
    };

    use super::*;
    use crate::components::Tooltip;
    use crate::menu::{ContextMenu, MenuItem};

    gpui::actions!(deferred_test, [Noop]);

    /// What the popup holds.
    #[derive(Clone, Copy, PartialEq)]
    enum Inside {
        /// Deferred anchored elements (popups of the popup): one over the
        /// popup's last child, one outside the popup's bounds.
        Anchored,
        /// A deferred dropdown positioned under its field, as in the Find
        /// in Files popup.
        Dropdown,
        /// A context menu drawn with `menu::overlay`.
        Menu,
        /// A button with a tooltip.
        Tooltip,
        /// A deferred element inside a deferred element inside the popup.
        ThreeLevels,
    }

    struct Host {
        inside: Inside,
        menu: Option<Entity<ContextMenu>>,
        /// Clicks on the nested elements and on the popup's last child.
        nested_clicks: Rc<Cell<usize>>,
        popup_clicks: Rc<Cell<usize>>,
    }

    fn square(id: &'static str, clicks: &Rc<Cell<usize>>) -> impl IntoElement {
        let clicks = clicks.clone();
        div()
            .id(id)
            .occlude()
            .w(px(40.0))
            .h(px(40.0))
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                clicks.set(clicks.get() + 1)
            })
    }

    fn nested_at(x: f32, y: f32, id: &'static str, clicks: &Rc<Cell<usize>>) -> Deferred {
        deferred(
            anchored()
                .position(point(px(x), px(y)))
                .child(square(id, clicks)),
        )
    }

    impl Render for Host {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let inner: AnyElement = match self.inside {
                Inside::Anchored => div()
                    .child(nested_at(25.0, 25.0, "nested-over", &self.nested_clicks).priority(1))
                    .child(nested_at(300.0, 300.0, "nested-far", &self.nested_clicks))
                    .into_any_element(),
                Inside::Dropdown => div()
                    .relative()
                    .h(px(20.0))
                    .child(
                        deferred(
                            div()
                                .id("dropdown")
                                .absolute()
                                .top_full()
                                .left_0()
                                .right_0()
                                .h(px(60.0))
                                .child("history"),
                        )
                        .with_priority(1),
                    )
                    .into_any_element(),
                Inside::Menu => {
                    div()
                        .children(self.menu.as_ref().map(|menu| {
                            crate::menu::overlay(menu, point(px(60.0), px(60.0)), window)
                        }))
                        .into_any_element()
                }
                Inside::Tooltip => div()
                    .id("with-tooltip")
                    .w(px(40.0))
                    .h(px(20.0))
                    .tooltip(Tooltip::text("Tip", None))
                    .into_any_element(),
                Inside::ThreeLevels => {
                    deferred(anchored().position(point(px(100.0), px(100.0))).child(
                        div().w(px(50.0)).h(px(50.0)).child(nested_at(
                            300.0,
                            300.0,
                            "nested-far",
                            &self.nested_clicks,
                        )),
                    ))
                    .into_any_element()
                }
            };
            // The popup at 20,20: clipped, with a last child at its top
            // left corner that a nested element drawn in place would be
            // under.
            div().size_full().child(deferred(
                anchored().position(point(px(20.0), px(20.0))).child(
                    div()
                        .id("popup")
                        .relative()
                        .w(px(200.0))
                        .h(px(150.0))
                        .overflow_hidden()
                        .child(inner)
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .child(square("popup-cover", &self.popup_clicks)),
                        ),
                ),
            ))
        }
    }

    fn open(inside: Inside, cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
        cx.update(|cx| crate::theme::init(cx, themes::ThemeKind::Dark));
        let (host, cx) = cx.add_window_view(|_, _| Host {
            inside,
            menu: None,
            nested_clicks: Rc::default(),
            popup_clicks: Rc::default(),
        });
        if inside == Inside::Menu {
            let menu = cx.update(|window, cx| {
                ContextMenu::build(vec![MenuItem::action("Noop", Noop)], window, cx)
            });
            host.update(cx, |h, cx| {
                h.menu = Some(menu);
                cx.notify();
            });
        }
        redraw(cx);
        (host, cx)
    }

    /// Draws a few frames: the first lays out, later ones reuse state.
    fn redraw(cx: &mut VisualTestContext) {
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear()
            });
        }
    }

    fn clicks(host: &Entity<Host>, cx: &mut VisualTestContext) -> (usize, usize) {
        host.read_with(cx, |h, _| (h.nested_clicks.get(), h.popup_clicks.get()))
    }

    #[gpui::test]
    fn an_anchored_element_inside_a_popup_is_drawn_above_it_and_not_cut_off(
        cx: &mut TestAppContext,
    ) {
        let (host, cx) = open(Inside::Anchored, cx);
        // Over the popup's last child: drawn above it.
        cx.simulate_click(point(px(35.0), px(35.0)), Modifiers::none());
        redraw(cx);
        assert_eq!(clicks(&host, cx), (1, 0));
        // Outside the popup's bounds: not cut off by its overflow.
        cx.simulate_click(point(px(320.0), px(320.0)), Modifiers::none());
        redraw(cx);
        assert_eq!(clicks(&host, cx), (2, 0));
    }

    #[gpui::test]
    fn a_deferred_element_three_levels_deep_draws(cx: &mut TestAppContext) {
        let (host, cx) = open(Inside::ThreeLevels, cx);
        cx.simulate_click(point(px(320.0), px(320.0)), Modifiers::none());
        redraw(cx);
        assert_eq!(clicks(&host, cx), (1, 0));
    }

    #[gpui::test]
    fn a_dropdown_inside_a_popup_draws_in_place(cx: &mut TestAppContext) {
        let (_, cx) = open(Inside::Dropdown, cx);
        redraw(cx);
    }

    #[gpui::test]
    fn a_context_menu_inside_a_popup_covers_the_window(cx: &mut TestAppContext) {
        let (host, cx) = open(Inside::Menu, cx);
        let dismissed = Rc::new(Cell::new(false));
        let flag = dismissed.clone();
        let menu = host.read_with(cx, |h, _| h.menu.clone().expect("a menu"));
        cx.update(|_, cx| {
            cx.subscribe(&menu, move |_, _: &DismissEvent, _| flag.set(true))
                .detach()
        });
        cx.simulate_mouse_move(point(px(70.0), px(70.0)), None, Modifiers::none());
        redraw(cx);
        // The menu's backdrop covers the window, not only the popup: a
        // click outside the popup closes the menu.
        cx.simulate_click(point(px(400.0), px(400.0)), Modifiers::none());
        redraw(cx);
        assert!(dismissed.get());
    }

    #[gpui::test]
    fn a_tooltip_inside_a_popup_draws(cx: &mut TestAppContext) {
        let (_, cx) = open(Inside::Tooltip, cx);
        cx.simulate_mouse_move(point(px(30.0), px(30.0)), None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_secs(2));
        redraw(cx);
        cx.simulate_mouse_move(point(px(32.0), px(31.0)), None, Modifiers::none());
        redraw(cx);
    }
}
