//! Wheel and trackpad scrolling, driven by real `ScrollWheelEvent`s at
//! window positions on GPUI's test platform: line deltas as a mouse wheel
//! sends them, pixel deltas with touch phases as a trackpad sends them
//! (momentum included). Each area scrolls only under the pointer.

use std::path::Path;
use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, Focusable as _, Modifiers, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, WindowHandle, point, px,
};
use serde_json::{Value, json};
use themes::Metric;
use ui::theme::ThemeExt as _;
use workbench::{FileType, Product, Workbench};

/// Top-level files in the test project; with the root, `pkg` and
/// `long.py` the tree has this many rows plus three.
const FILES: usize = 80;

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
}

/// Renders the next frame, as the display link would.
fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn invoke(cx: &mut TestAppContext, window: WindowHandle<Workbench>, id: &str, args: Value) {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke(id, args, window, cx)
    })
    .unwrap()
    .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx, window);
}

fn state(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke("workbench.state", json!(null), window, cx)
    })
    .unwrap()
    .unwrap()
}

/// `pkg/` with five files, `long.py` of 300 lines and `FILES` short files.
fn project(root: &Path) {
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    for i in 0..5 {
        std::fs::write(root.join(format!("pkg/p{i}.py")), "x = 1\n").unwrap();
    }
    let long: String = (0..300).map(|i| format!("x{i} = {i}\n")).collect();
    std::fs::write(root.join("long.py"), long).unwrap();
    for i in 0..FILES {
        std::fs::write(root.join(format!("m{i:02}.py")), "x = 1\n").unwrap();
    }
}

fn open_project(cx: &mut TestAppContext) -> (WindowHandle<Workbench>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    (window, dir)
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    part: &'static str,
) -> Bounds<Pixels> {
    VisualTestContext::from_window(window.into(), cx)
        .debug_bounds(part)
        .unwrap_or_else(|| panic!("{part} is not drawn"))
}

fn wheel(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    at: Point<Pixels>,
    delta: ScrollDelta,
    phase: TouchPhase,
) {
    VisualTestContext::from_window(window.into(), cx).simulate_event(ScrollWheelEvent {
        position: at,
        delta,
        modifiers: Modifiers::none(),
        touch_phase: phase,
    });
    draw(cx, window);
}

/// A mouse wheel: whole lines, positive scrolls up.
fn wheel_lines(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    at: Point<Pixels>,
    x: f32,
    y: f32,
) {
    wheel(
        cx,
        window,
        at,
        ScrollDelta::Lines(point(x, y)),
        TouchPhase::Moved,
    );
}

/// A trackpad: pixels, positive scrolls up.
fn swipe(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    at: Point<Pixels>,
    x: f32,
    y: f32,
) {
    wheel(
        cx,
        window,
        at,
        ScrollDelta::Pixels(point(px(x), px(y))),
        TouchPhase::Moved,
    );
}

/// Gives the keyboard focus to the project tree.
fn focus_tree(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    let panel = window.read_with(cx, |w, _| w.panel().clone()).unwrap();
    let handle = cx.update(|cx| panel.focus_handle(cx));
    cx.update_window(window.into(), |_, window, _| window.focus(&handle))
        .unwrap();
    draw(cx, window);
}

fn tree_top(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> f32 {
    state(cx, window)["tree"]["scroll_top"].as_f64().unwrap() as f32
}

fn first_visible_row(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> usize {
    state(cx, window)["tree"]["first_visible_row"]
        .as_u64()
        .unwrap() as usize
}

fn tree_row_count(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> usize {
    state(cx, window)["tree"]["rows"].as_array().unwrap().len()
}

fn selected_row(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> usize {
    let s = state(cx, window);
    let selected = s["tree"]["selected"].clone();
    s["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .position(|r| r["path"] == selected)
        .unwrap()
}

fn row_height(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| f32::from(cx.theme().metric(Metric::RowHeight)))
}

/// Largest scroll offset of the tree: the last row at the bottom.
fn tree_max(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> f32 {
    let rows = tree_row_count(cx, window) as f32;
    let height = f32::from(bounds(cx, window, "project-rows").size.height);
    rows * row_height(cx) - height
}

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{what}: {actual}, expected {expected}"
    );
}

fn editor_top(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> f32 {
    let editor = window
        .read_with(cx, |w, _| w.active_editor().cloned())
        .unwrap()
        .expect("an editor is open");
    f32::from(editor.read_with(cx, |e, _| e.scroll_top()))
}

#[gpui::test]
fn trackpad_and_mouse_wheel_scroll_the_project_tree(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    assert_eq!(tree_row_count(cx, window), FILES + 3);
    let rh = row_height(cx);
    let list = bounds(cx, window, "project-rows");
    assert!(
        rh * (FILES as f32) > f32::from(list.size.height),
        "the tree must be taller than the tool window"
    );
    let at = list.center();
    assert_eq!(tree_top(cx, window), 0.0);

    // A trackpad gesture: pixel deltas, then momentum after the fingers
    // are lifted (macOS sends those with no phase, GPUI as `Moved`).
    let gesture = [
        (TouchPhase::Started, 0.0),
        (TouchPhase::Moved, -20.0),
        (TouchPhase::Moved, -25.0),
        (TouchPhase::Ended, 0.0),
        (TouchPhase::Moved, -30.0),
        (TouchPhase::Moved, -5.0),
    ];
    for (phase, dy) in gesture {
        wheel(
            cx,
            window,
            at,
            ScrollDelta::Pixels(point(px(0.0), px(dy))),
            phase,
        );
    }
    assert_eq!(tree_top(cx, window), 80.0);
    assert_eq!(first_visible_row(cx, window), (80.0 / rh).floor() as usize);

    // A mouse wheel scrolls by lines.
    wheel_lines(cx, window, at, 0.0, -3.0);
    let after_wheel = tree_top(cx, window);
    assert!(
        after_wheel > 80.0,
        "the wheel did not scroll: {after_wheel}"
    );

    // Scrolling stops at the last row and at the first.
    swipe(cx, window, at, 0.0, -100_000.0);
    assert_near(tree_top(cx, window), tree_max(cx, window), "bottom");
    let visible = (f32::from(list.size.height) / rh).ceil() as usize;
    assert!(first_visible_row(cx, window) + visible >= FILES + 3);
    wheel_lines(cx, window, at, 0.0, 10_000.0);
    assert_eq!(tree_top(cx, window), 0.0);
    assert_eq!(first_visible_row(cx, window), 0);
}

#[gpui::test]
fn a_sideways_swipe_does_not_scroll_the_tree(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    let at = bounds(cx, window, "project-rows").center();
    swipe(cx, window, at, -80.0, 0.0);
    wheel_lines(cx, window, at, -3.0, 0.0);
    assert_eq!(tree_top(cx, window), 0.0);
    // Mostly downwards with a little sideways drift still scrolls.
    swipe(cx, window, at, -2.0, -40.0);
    assert_eq!(tree_top(cx, window), 40.0);
}

#[gpui::test]
fn the_wheel_scrolls_only_what_is_under_the_pointer(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    invoke(cx, window, "project.open-file", json!({"path": "long.py"}));
    let tree = bounds(cx, window, "project-rows").center();
    let editor = bounds(cx, window, "editor-area").center();

    // Over the editor: the editor scrolls, the tree stays.
    wheel_lines(cx, window, editor, 0.0, -5.0);
    let editor_after = editor_top(cx, window);
    assert!(editor_after > 0.0, "the editor did not scroll");
    assert_eq!(tree_top(cx, window), 0.0);
    swipe(cx, window, editor, 0.0, -30.0);
    assert_eq!(editor_top(cx, window), editor_after + 30.0);
    assert_eq!(tree_top(cx, window), 0.0);

    // Over the tree: the tree scrolls, the editor stays.
    swipe(cx, window, tree, 0.0, -50.0);
    wheel_lines(cx, window, tree, 0.0, -2.0);
    assert!(tree_top(cx, window) > 50.0);
    assert_eq!(editor_top(cx, window), editor_after + 30.0);
}

#[gpui::test]
fn expanding_and_collapsing_keeps_the_tree_scroll_position(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    let rh = row_height(cx);
    let at = bounds(cx, window, "project-rows").center();
    // `pkg` (row 1) at the top of the view; select it with the keyboard,
    // which does not scroll a visible row.
    swipe(cx, window, at, 0.0, -rh);
    assert_eq!(first_visible_row(cx, window), 1);
    focus_tree(cx, window);
    cx.simulate_keystrokes(window.into(), "down");
    draw(cx, window);
    assert_eq!(state(cx, window)["tree"]["selected"], json!("pkg"));
    assert_eq!(tree_top(cx, window), rh);

    // Its rows open below it; the view does not move.
    cx.simulate_keystrokes(window.into(), "right");
    draw(cx, window);
    assert_eq!(tree_row_count(cx, window), FILES + 8);
    assert_eq!(tree_top(cx, window), rh);
    cx.simulate_keystrokes(window.into(), "left");
    draw(cx, window);
    assert_eq!(tree_row_count(cx, window), FILES + 3);
    assert_eq!(tree_top(cx, window), rh);

    // At the bottom, collapsing rows keeps the last row at the bottom
    // instead of leaving empty space.
    cx.simulate_keystrokes(window.into(), "right");
    draw(cx, window);
    swipe(cx, window, at, 0.0, -100_000.0);
    let expanded_max = tree_max(cx, window);
    assert_near(tree_top(cx, window), expanded_max, "bottom, expanded");
    invoke(cx, window, "project.tree.collapse-all", json!(null));
    assert_eq!(tree_row_count(cx, window), FILES + 3);
    let collapsed_max = tree_max(cx, window);
    assert_near(collapsed_max, expanded_max - 5.0 * rh, "max after collapse");
    assert_near(tree_top(cx, window), collapsed_max, "bottom, collapsed");
}

#[gpui::test]
fn keyboard_moves_scroll_the_tree_as_little_as_needed(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    let rh = row_height(cx);
    let list = bounds(cx, window, "project-rows");
    let height = f32::from(list.size.height);
    // Rows that fit whole.
    let whole = (height / rh).floor() as usize;
    focus_tree(cx, window);

    // Down to the last row that fits: nothing scrolls.
    for _ in 0..whole - 1 {
        cx.simulate_keystrokes(window.into(), "down");
    }
    draw(cx, window);
    assert_eq!(selected_row(cx, window), whole - 1);
    assert_eq!(tree_top(cx, window), 0.0);
    // One more: the view moves just enough to show the row at the bottom.
    cx.simulate_keystrokes(window.into(), "down");
    draw(cx, window);
    assert_eq!(selected_row(cx, window), whole);
    assert_near(
        tree_top(cx, window),
        (whole + 1) as f32 * rh - height,
        "after moving below the view",
    );
    // Further down, every move scrolls by one row.
    for _ in 0..whole {
        cx.simulate_keystrokes(window.into(), "down");
    }
    draw(cx, window);
    let selected = selected_row(cx, window);
    assert_eq!(selected, 2 * whole);
    let top = tree_top(cx, window);
    assert_near(top, (selected + 1) as f32 * rh - height, "one row a move");

    // Up through the rows that are whole in view: nothing scrolls.
    let first_whole = (top / rh).ceil() as usize;
    for _ in 0..selected - first_whole {
        cx.simulate_keystrokes(window.into(), "up");
    }
    draw(cx, window);
    assert_eq!(selected_row(cx, window), first_whole);
    assert_eq!(tree_top(cx, window), top);
    // One more: the row above comes in at the top.
    cx.simulate_keystrokes(window.into(), "up");
    draw(cx, window);
    assert_eq!(selected_row(cx, window), first_whole - 1);
    assert_near(
        tree_top(cx, window),
        (first_whole - 1) as f32 * rh,
        "after moving above the view",
    );

    // Scrolled away with the wheel, the next move brings the selection
    // back into view.
    swipe(cx, window, list.center(), 0.0, -100_000.0);
    cx.simulate_keystrokes(window.into(), "down");
    draw(cx, window);
    let row = selected_row(cx, window) as f32;
    let top = tree_top(cx, window);
    assert!(
        row * rh >= top && (row + 1.0) * rh <= top + height,
        "row {row} not visible at {top}"
    );
}

#[gpui::test]
fn the_tab_strip_scrolls_sideways(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    open_many_tabs(cx, window, 30);
    let strip = bounds(cx, window, "editor-tabs");
    // The last tab is the active one: the strip is scrolled to it.
    assert!(
        bounds(cx, window, "tab-0").right() < strip.left(),
        "the tabs must be wider than the strip"
    );
    let first_left = |cx: &mut TestAppContext| f32::from(bounds(cx, window, "tab-0").left());
    let start = first_left(cx);
    // A mouse wheel turned up scrolls the tabs to the right.
    wheel_lines(cx, window, strip.center(), 0.0, 3.0);
    let after_wheel = first_left(cx);
    assert!(after_wheel > start, "the wheel did not scroll the tabs");
    // A sideways swipe scrolls them too.
    swipe(cx, window, strip.center(), -40.0, 0.0);
    assert_eq!(first_left(cx), after_wheel - 40.0);
    // Neither moved the editor or the tree.
    assert_eq!(editor_top(cx, window), 0.0);
    assert_eq!(tree_top(cx, window), 0.0);
}

/// Opens `count` files `m00.py`... so the tabs are wider than the strip.
fn open_many_tabs(cx: &mut TestAppContext, window: WindowHandle<Workbench>, count: usize) {
    for i in 0..count {
        invoke(
            cx,
            window,
            "project.open-file",
            json!({"path": format!("m{i:02}.py")}),
        );
    }
}

/// Whether the whole tab `part` is inside the tab strip.
fn tab_is_shown(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    part: &'static str,
) -> bool {
    let strip = bounds(cx, window, "editor-tabs");
    let tab = bounds(cx, window, part);
    tab.left() >= strip.left() && tab.right() <= strip.right()
}

#[gpui::test]
fn the_active_tab_is_scrolled_into_view(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    open_many_tabs(cx, window, 30);
    // Opening a tab beyond the strip shows it.
    assert!(tab_is_shown(cx, window, "tab-29"));
    // Activating one far to the left brings it back.
    invoke(cx, window, "project.open-file", json!({"path": "m00.py"}));
    assert!(tab_is_shown(cx, window, "tab-0"));
    // And one far to the right again.
    invoke(cx, window, "project.open-file", json!({"path": "m20.py"}));
    assert!(tab_is_shown(cx, window, "tab-20"));
    // A tab already inside the strip does not move the strip.
    let left = bounds(cx, window, "tab-20").left();
    invoke(cx, window, "project.open-file", json!({"path": "m19.py"}));
    assert!(tab_is_shown(cx, window, "tab-19"));
    assert_eq!(bounds(cx, window, "tab-20").left(), left);
    // Closing the last tab shows the one that becomes active.
    invoke(cx, window, "project.open-file", json!({"path": "m29.py"}));
    invoke(cx, window, "tabs.close", json!({"index": 29}));
    assert!(tab_is_shown(cx, window, "tab-28"));
}

#[gpui::test]
fn the_go_to_line_field_does_not_scroll(cx: &mut TestAppContext) {
    let (window, _dir) = open_project(cx);
    invoke(cx, window, "project.open-file", json!({"path": "long.py"}));
    // Focus events reach listeners only in the active window.
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    invoke(cx, window, "workbench.go-to-line", json!(null));
    cx.simulate_input(window.into(), "120");
    draw(cx, window);
    let field = window
        .read_with(cx, |w, _| w.go_to_line_field().cloned())
        .unwrap()
        .expect("the Go to Line field is open");
    let geometry = |cx: &mut TestAppContext| field.read_with(cx, |e, _| e.text_geometry().unwrap());
    let (origin, lh, cw) = geometry(cx);
    let at = point(origin.x + cw, origin.y + lh / 2.0);
    let editor_before = editor_top(cx, window);

    swipe(cx, window, at, -60.0, 0.0);
    swipe(cx, window, at, 0.0, -60.0);
    wheel_lines(cx, window, at, 0.0, -3.0);
    // The text stays in place and the editor under the popup stays too.
    assert_eq!(geometry(cx).0, origin);
    assert_eq!(editor_top(cx, window), editor_before);
}
