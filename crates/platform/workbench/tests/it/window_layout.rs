//! The main window's areas while the window is resized
//! (tests/scenarios/editor/window-layout.md): the editor gives way first,
//! then the tool window areas in proportion to their sizes, down to the
//! main splitter's minimum; nothing overlaps, every size changes
//! continuously, and growing the window back restores the sizes.

use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    ParentElement, Pixels, Render, Styled, TestAppContext, VisualTestContext, Window, WindowHandle,
    div, point, px, size,
};
use serde_json::{Value, json};
use workbench::tool_windows::{Area, ToolWindowContent, ToolWindowSpec, ToolWindowView};
use workbench::workbench::ToolWindow;
use workbench::{Product, Workbench};

const EPS: f32 = 0.6;

fn product() -> Product {
    Product {
        name: "Test IDE",
        ..Product::default()
    }
}

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = product();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    draw(cx, window);
    window
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx, window);
    out
}

struct TestToolWindow {
    focus: gpui::FocusHandle,
}

impl Render for TestToolWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full().child("test")
    }
}

impl gpui::Focusable for TestToolWindow {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

impl ToolWindowView for TestToolWindow {}

/// A tool window of the right area, shown.
fn show_right(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    window
        .update(cx, |w, _, cx| {
            let view = cx.new(|cx| TestToolWindow {
                focus: cx.focus_handle(),
            });
            w.register_tool_window(
                ToolWindowSpec {
                    id: ToolWindow("test-right"),
                    area: Area::Right,
                    order: 5,
                    icon: ui::icons::IconName::TOOL_NOTIFICATIONS,
                    title: "Test",
                    command: None,
                    registered: true,
                },
                Some(ToolWindowContent::new(view)),
                cx,
            );
        })
        .unwrap();
    draw(cx, window);
    invoke(
        cx,
        window,
        "workbench.activate-tool-window",
        json!({"id": "test-right"}),
    );
}

fn resize(cx: &mut TestAppContext, window: WindowHandle<Workbench>, width: f32, height: f32) {
    VisualTestContext::from_window(window.into(), cx).simulate_resize(size(px(width), px(height)));
    draw(cx, window);
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &'static str,
) -> Bounds<Pixels> {
    VisualTestContext::from_window(window.into(), cx)
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

/// Drags a splitter by `by` pixels (across for a side area, down for the
/// bottom area).
fn drag(cx: &mut TestAppContext, window: WindowHandle<Workbench>, splitter: &'static str, by: f32) {
    let b = bounds(cx, window, splitter);
    let from = b.center();
    let to = if b.size.width < b.size.height {
        point(from.x + px(by), from.y)
    } else {
        point(from.x, from.y + px(by))
    };
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(from, None, Modifiers::none());
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    draw(cx, window);
}

fn x(p: Pixels) -> f32 {
    f32::from(p)
}

fn stored(cx: &mut TestAppContext, window: WindowHandle<Workbench>, area: &str) -> f32 {
    let state = invoke(cx, window, "workbench.state", json!(null));
    let key = if area == "bottom" { "height" } else { "width" };
    state["tool_window_areas"][area][key].as_f64().unwrap() as f32
}

#[derive(Debug, Clone, Copy)]
struct Across {
    width: f32,
    left: f32,
    editor: f32,
    right: f32,
}

fn across(cx: &mut TestAppContext, window: WindowHandle<Workbench>, width: f32) -> Across {
    resize(cx, window, width, 800.0);
    let stripe_l = bounds(cx, window, "left-stripe");
    let left = bounds(cx, window, "project-tool-window");
    let editor = bounds(cx, window, "editor-area");
    let right = bounds(cx, window, "test-right-tool-window");
    let stripe_r = bounds(cx, window, "right-stripe");
    let layout = invoke(cx, window, "workbench.layout", json!(null));
    let min = layout["min"].as_f64().unwrap() as f32;
    // Nothing overlaps: stripe, left area, editor, right area, stripe.
    assert!(x(stripe_l.right()) <= x(left.left()) + EPS, "{width}");
    assert!(
        x(left.right()) < x(editor.left()),
        "{width}: {left:?} {editor:?}"
    );
    assert!(
        x(editor.right()) < x(right.left()),
        "{width}: {editor:?} {right:?}"
    );
    // The right area ends at the right stripe until a side area reaches its
    // minimum (then the rest runs under the stripe, cut there).
    let at_minimum = |b: Bounds<Pixels>| x(b.size.width) < min + EPS;
    if !at_minimum(left) && !at_minimum(right) {
        assert!(
            (x(right.right()) - x(stripe_r.left())).abs() < EPS,
            "{width}: {right:?} {stripe_r:?}"
        );
    }
    // The drawn sizes are the layout's.
    let drawn = Across {
        width,
        left: x(left.size.width),
        editor: x(editor.size.width),
        right: x(right.size.width),
    };
    assert!((drawn.left - layout["left"].as_f64().unwrap() as f32).abs() < EPS);
    assert!((drawn.right - layout["right"].as_f64().unwrap() as f32).abs() < EPS);
    if drawn.left > min + EPS || drawn.right > min + EPS {
        assert!(
            (drawn.editor - layout["editor"].as_f64().unwrap() as f32).abs() < EPS,
            "{width}: {drawn:?} {layout}"
        );
    }
    drawn
}

/// Narrowing the window shrinks the editor first; at the editor's minimum
/// both side areas shrink in proportion to their sizes, down to the same
/// minimum. Growing the window back gives every width the same layout and
/// the user's sizes again.
#[gpui::test]
fn the_editor_gives_way_first_then_both_side_areas(cx: &mut TestAppContext) {
    let window = open(cx);
    show_right(cx, window);
    // Different widths for the two sides: 233 on the left, 333 on the right.
    drag(cx, window, "project-splitter", -100.0);
    let (left0, right0) = (stored(cx, window, "left"), stored(cx, window, "right"));
    assert_eq!((left0, right0), (233.0, 333.0));
    let min = invoke(cx, window, "workbench.layout", json!(null))["min"]
        .as_f64()
        .unwrap() as f32;
    assert_eq!(min, 25.0);

    let widths: Vec<f32> = (150..=1600).rev().step_by(7).map(|w| w as f32).collect();
    let down: Vec<Across> = widths.iter().map(|w| across(cx, window, *w)).collect();
    let mut editor_reached_min = false;
    for (i, f) in down.iter().enumerate() {
        assert!(f.editor > min - EPS, "{f:?}");
        assert!(f.left > min - EPS && f.right > min - EPS, "{f:?}");
        if f.editor > min + EPS {
            // The editor gives way first: the sides keep their sizes.
            assert!(!editor_reached_min, "the editor grew again: {f:?}");
            assert!((f.left - left0).abs() < EPS, "{f:?}");
            assert!((f.right - right0).abs() < EPS, "{f:?}");
        } else {
            editor_reached_min = true;
            if f.left > min + EPS && f.right > min + EPS {
                // Both sides lose in proportion to their sizes.
                let ratio = f.left / f.right;
                assert!((ratio - left0 / right0).abs() < 0.01, "{f:?}");
            }
        }
        // Continuous: no width changes by more than the window did.
        if let Some(prev) = i.checked_sub(1).map(|p| down[p]) {
            let step = prev.width - f.width + EPS;
            assert!((prev.left - f.left).abs() <= step, "{prev:?} -> {f:?}");
            assert!((prev.right - f.right).abs() <= step, "{prev:?} -> {f:?}");
            assert!((prev.editor - f.editor).abs() <= step, "{prev:?} -> {f:?}");
        }
    }
    assert!(editor_reached_min);
    let narrowest = down.last().unwrap();
    assert!((narrowest.left - min).abs() < EPS && (narrowest.right - min).abs() < EPS);
    // The user's sizes are not changed by the layout.
    assert_eq!(stored(cx, window, "left"), left0);
    assert_eq!(stored(cx, window, "right"), right0);

    // Growing back: the same layout for every width, and the sizes return.
    for f in down.iter().rev() {
        let up = across(cx, window, f.width);
        assert!((up.left - f.left).abs() < EPS, "{up:?} {f:?}");
        assert!((up.right - f.right).abs() < EPS, "{up:?} {f:?}");
        assert!((up.editor - f.editor).abs() < EPS, "{up:?} {f:?}");
    }
    let widest = across(cx, window, 1600.0);
    assert!((widest.left - left0).abs() < EPS && (widest.right - right0).abs() < EPS);
}

/// With only the left area shown it alone gives way once the editor is at
/// its minimum.
#[gpui::test]
fn one_side_area_gives_way_alone(cx: &mut TestAppContext) {
    let window = open(cx);
    let min = invoke(cx, window, "workbench.layout", json!(null))["min"]
        .as_f64()
        .unwrap() as f32;
    for width in (150..=900).rev().step_by(11) {
        resize(cx, window, width as f32, 800.0);
        let project = bounds(cx, window, "project-tool-window");
        let editor = bounds(cx, window, "editor-area");
        let stripe = bounds(cx, window, "right-stripe");
        assert!(x(project.right()) < x(editor.left()));
        assert!(x(editor.size.width) > min - EPS, "{width}: {editor:?}");
        assert!(x(project.size.width) > min - EPS, "{width}: {project:?}");
        assert!(x(project.size.width) < 333.0 + EPS);
        if x(project.size.width) > min + EPS {
            assert!(
                (x(editor.right()) - x(stripe.left())).abs() < EPS,
                "{width}"
            );
            if x(project.size.width) < 333.0 - EPS {
                assert!((x(editor.size.width) - min).abs() < EPS, "{width}");
            }
        }
    }
    resize(cx, window, 1280.0, 800.0);
    assert_eq!(
        x(bounds(cx, window, "project-tool-window").size.width),
        333.0
    );
}

/// Lowering the window shrinks the editor row first while the bottom area
/// keeps its height; at the row's minimum the bottom area shrinks down to
/// the same minimum. The toolbar and the status bar keep their heights.
#[gpui::test]
fn the_editor_row_gives_way_first_then_the_bottom_area(cx: &mut TestAppContext) {
    let window = open(cx);
    invoke(cx, window, "workbench.toggle-run", json!(null));
    let bottom0 = stored(cx, window, "bottom");
    let toolbar0 = bounds(cx, window, "toolbar").size.height;
    let status0 = bounds(cx, window, "status-bar").size.height;
    let layout = invoke(cx, window, "workbench.layout", json!(null));
    let (min, gap) = (
        layout["min"].as_f64().unwrap() as f32,
        layout["gap"].as_f64().unwrap() as f32,
    );
    let mut frames: Vec<(f32, f32, f32)> = Vec::new();
    // Down to a pane lower than both minimums (below the window's minimum).
    for height in (90..=1100).rev().step_by(5) {
        let height = height as f32;
        resize(cx, window, 1280.0, height);
        let toolbar = bounds(cx, window, "toolbar");
        let status = bounds(cx, window, "status-bar");
        let editor = bounds(cx, window, "editor-area");
        let project = bounds(cx, window, "project-tool-window");
        let run = bounds(cx, window, "run-tool-window");
        assert_eq!(toolbar.size.height, toolbar0, "{height}");
        assert_eq!(status.size.height, status0, "{height}");
        assert_eq!(editor.size.height, project.size.height, "{height}");
        assert!(x(toolbar.bottom()) <= x(editor.top()) + EPS, "{height}");
        assert!(
            (x(run.top()) - x(editor.bottom()) - gap).abs() < EPS,
            "{height}: {editor:?} {run:?}"
        );
        let (row, bottom) = (x(editor.size.height), x(run.size.height));
        assert!(
            row > min - EPS && bottom > min - EPS,
            "{height}: {row} {bottom}"
        );
        if row > min + EPS {
            assert!((bottom - bottom0).abs() < EPS, "{height}: {bottom}");
            assert!((x(run.bottom()) - x(status.top())).abs() < EPS, "{height}");
        } else if bottom > min + EPS {
            assert!((x(run.bottom()) - x(status.top())).abs() < EPS, "{height}");
        }
        if let Some((h, r, b)) = frames.last().copied() {
            let step: f32 = h - height + EPS;
            assert!(
                (r - row).abs() <= step && (b - bottom).abs() <= step,
                "{height}"
            );
        }
        frames.push((height, row, bottom));
    }
    let (_, row, bottom) = *frames.last().unwrap();
    assert!((row - min).abs() < EPS && (bottom - min).abs() < EPS);
    assert_eq!(stored(cx, window, "bottom"), bottom0);
    // Growing back restores the bottom area's height.
    resize(cx, window, 1280.0, 1100.0);
    assert!((x(bounds(cx, window, "run-tool-window").size.height) - bottom0).abs() < EPS);
}

/// A drag that starts while the window squeezes an area starts from the
/// area's size on screen, not from the size the user gave it.
#[gpui::test]
fn a_drag_starts_from_the_size_on_screen(cx: &mut TestAppContext) {
    let window = open(cx);
    resize(cx, window, 300.0, 800.0);
    let squeezed = x(bounds(cx, window, "project-tool-window").size.width);
    assert!(squeezed < 333.0 - 1.0, "{squeezed}");
    drag(cx, window, "project-splitter", -30.0);
    let after = x(bounds(cx, window, "project-tool-window").size.width);
    assert!(
        (after - (squeezed - 30.0)).abs() < EPS,
        "{squeezed} -> {after}"
    );
    assert!((stored(cx, window, "left") - after).abs() < EPS);
    // Growing the window keeps the new width.
    resize(cx, window, 1280.0, 800.0);
    assert!((x(bounds(cx, window, "project-tool-window").size.width) - after).abs() < EPS);
}

/// In a low window the stripe's groups keep their places at the top and at
/// the bottom; the top group covers the bottom one pixel by pixel (the
/// bottom group is cut where the top one ends, and is gone once the top
/// group fills the stripe).
#[gpui::test]
fn the_top_stripe_group_covers_the_bottom_group(cx: &mut TestAppContext) {
    let window = open(cx);
    resize(cx, window, 1280.0, 900.0);
    let top0 = bounds(cx, window, "left-stripe-top");
    let bottom0 = bounds(cx, window, "left-stripe-bottom");
    let stripe0 = bounds(cx, window, "left-stripe");
    let below = stripe0.bottom() - bottom0.bottom();
    assert!(
        x(top0.bottom()) < x(bottom0.top()),
        "a gap between the groups"
    );
    let mut covered_before = 0.0;
    for height in (120..=900).rev().step_by(3) {
        resize(cx, window, 1280.0, height as f32);
        let stripe = bounds(cx, window, "left-stripe");
        let top = bounds(cx, window, "left-stripe-top");
        let bottom = bounds(cx, window, "left-stripe-bottom");
        assert_eq!(top.top(), stripe.top(), "{height}");
        assert_eq!(top.size, top0.size, "{height}");
        assert_eq!(bottom.size, bottom0.size, "{height}");
        if top.bottom() < stripe.bottom() {
            assert!((x(stripe.bottom() - bottom.bottom()) - x(below)).abs() < EPS);
        }
        // The covered part grows with every pixel the window loses.
        let covered = (x(top.bottom()) - x(bottom.top())).max(0.0);
        assert!(covered >= covered_before - EPS, "{height}");
        assert!(covered - covered_before <= 3.0 + EPS, "{height}");
        covered_before = covered;
    }
    assert!(covered_before > 0.0, "the groups met");
}

/// The window can be made as narrow as the reference's frame (340) and as
/// low as the toolbar, the status bar and both areas at their minimum.
#[gpui::test]
fn the_window_minimum(cx: &mut TestAppContext) {
    let window = open(cx);
    let product = product();
    let min = cx.update(|cx| workbench::app::window_options(cx, &product).window_min_size);
    let min = min.unwrap();
    assert_eq!(min.width, px(340.0));
    let toolbar = bounds(cx, window, "toolbar").size.height;
    let status = bounds(cx, window, "status-bar").size.height;
    let layout = invoke(cx, window, "workbench.layout", json!(null));
    let island = layout["min"].as_f64().unwrap() as f32;
    let gap = layout["gap"].as_f64().unwrap() as f32;
    assert_eq!(min.height, toolbar + status + px(2.0 * island + gap));
}
