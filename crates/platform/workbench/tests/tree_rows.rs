//! How the project tree draws its rows (tests/scenarios/editor/project-tree-rows.md):
//! indent guides below open folders, and a row whose text is cut by the
//! tool window's edge (its selection reaching the edge, the rest of it
//! shown over the edge while the pointer is at it or the tree selects it,
//! 10 ms later, taking the row's clicks while the pointer is on it).

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Bounds, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext, WindowHandle, point, px,
};
use serde_json::{Value, json};
use workbench::{FileType, Product, Workbench};

/// A root folder name too long for the tool window on every platform.
const LONG_ROOT: &str = "a-project-folder-with-a-name-longer-than-the-tool-window";

fn open(cx: &mut TestAppContext, root: &Path) -> WindowHandle<Workbench> {
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
    // Focus events reach listeners only in the active window.
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    window
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

/// The rest of a row follows the pointer and the selection this much later.
const DELAY: Duration = Duration::from_millis(10);

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.executor().advance_clock(DELAY);
    draw_now(cx, window);
}

/// Draws without letting time pass.
fn draw_now(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    // Twice: rows measure their text against the width drawn the frame
    // before (a new width asks for another frame).
    for _ in 0..2 {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }
}

/// The tree's state now, without letting time pass.
fn tree_now(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke("workbench.state", json!(null), window, cx)
    })
    .unwrap()
    .unwrap()["tree"]
        .clone()
}

fn tree(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    let s = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(s["busy"], json!(false), "background work did not finish");
    s["tree"].clone()
}

fn row(tree: &Value, path: &str) -> Value {
    tree["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == json!(path))
        .unwrap_or_else(|| panic!("no row {path}"))
        .clone()
}

/// Where an element was last drawn (the test window keeps the bounds of
/// elements drawn in earlier frames, so this does not tell whether it is
/// still there).
fn drawn(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Bounds<Pixels> {
    draw(cx, window);
    // Debug selectors are looked up by a static name.
    let name: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    VisualTestContext::from_window(window.into(), cx)
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn mouse_to(cx: &mut TestAppContext, window: WindowHandle<Workbench>, at: Point<Pixels>) {
    VisualTestContext::from_window(window.into(), cx).simulate_mouse_move(
        at,
        None,
        Modifiers::none(),
    );
    draw(cx, window);
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = drawn(cx, window, selector).center();
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_click(at, Modifiers::none());
    draw(cx, window);
}

/// Left button clicks at `at`: one, or two for a double click.
fn click_at(cx: &mut TestAppContext, window: WindowHandle<Workbench>, at: Point<Pixels>, n: usize) {
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    for click_count in 1..=n {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    draw(cx, window);
}

fn right_click_at(cx: &mut TestAppContext, window: WindowHandle<Workbench>, at: Point<Pixels>) {
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    vcx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    draw(cx, window);
}

fn press(cx: &mut TestAppContext, window: WindowHandle<Workbench>, keys: &str) {
    cx.simulate_keystrokes(window.into(), keys);
    draw(cx, window);
}

fn layout(root: &Path) {
    for (path, text) in [
        ("src/pkg/mod.py", "x = 1\n"),
        ("src/app.py", "y = 2\n"),
        ("README.md", "# Readme\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

/// The left edges of a row's indent guides, from the row's left edge.
fn guides(tree: &Value, path: &str) -> Vec<f64> {
    row(tree, path)["guides"]
        .as_array()
        .unwrap_or_else(|| panic!("{path} has no guides"))
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[gpui::test]
fn indent_guides_run_below_open_folders(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    layout(&root);
    let window = open(cx, &root);
    invoke(
        cx,
        window,
        "project.tree.reveal",
        json!({"path": "src/pkg/mod.py"}),
    );
    let t = tree(cx, window);
    // A line at the centre of each open ancestor's chevron (12 px in, 16
    // px wide, 19 px per level), through the rows below it.
    assert_eq!(guides(&t, ""), [19.0], "the open root: below its chevron");
    assert_eq!(guides(&t, "src"), [19.0, 38.0]);
    assert_eq!(guides(&t, "src/pkg"), [19.0, 38.0, 57.0]);
    assert_eq!(guides(&t, "src/pkg/mod.py"), [19.0, 38.0, 57.0]);
    assert_eq!(guides(&t, "src/app.py"), [19.0, 38.0]);
    assert_eq!(guides(&t, "README.md"), [19.0]);

    // Off: no lines.
    window
        .update(cx, |wb, _, cx| {
            let panel = wb.panel().clone();
            panel.update(cx, |p, cx| p.set_indent_guides(false, cx));
        })
        .unwrap();
    let t = tree(cx, window);
    assert_eq!(row(&t, "src")["guides"], json!(null));
}

#[gpui::test]
fn a_cut_row_reaches_the_edge_and_shows_its_rest_on_hover(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join(LONG_ROOT);
    layout(&root);
    let window = open(cx, &root);
    let t = tree(cx, window);
    assert_eq!(row(&t, "")["overflows"], json!(true));
    assert_eq!(row(&t, "src")["overflows"], json!(false));
    assert_eq!(
        t["expanded_row"],
        json!(null),
        "nothing before the pointer comes"
    );

    // The selected root's selection runs to the edge but for a pixel; a
    // short row's keeps its inset of 12 px on both sides.
    let area = drawn(cx, window, "project-rows");
    let selection = drawn(cx, window, "tree-selection");
    assert_eq!(selection.left(), area.left() + px(12.0));
    assert_eq!(selection.right(), area.right() - px(1.0));
    click(cx, window, "tree-row:src");
    let selection = drawn(cx, window, "tree-selection");
    assert_eq!(selection.left(), area.left() + px(12.0));
    assert_eq!(selection.right(), area.right() - px(12.0));

    // The pointer at the root: its rest shows over the edge, continuing
    // the row without a seam: it starts where the row's (hover) background
    // ends, level with the row and as high; its 1 px border runs outside
    // the row above, below and at the right end.
    let root_row = drawn(cx, window, "tree-row:");
    mouse_to(cx, window, root_row.center());
    let t = tree(cx, window);
    assert_eq!(t["expanded_row"], json!(""));
    let rest = drawn(cx, window, "tree-row-expansion");
    let part = drawn(cx, window, "tree-row-expansion-row");
    let hovered = drawn(cx, window, "tree-hovered");
    assert_eq!(hovered.right(), area.right() - px(1.0));
    assert_eq!(part.left(), hovered.right());
    assert_eq!(part.top(), root_row.top());
    assert_eq!(part.size.height, root_row.size.height);
    assert_eq!(rest.left(), part.left(), "no border where it joins the row");
    assert_eq!(rest.top(), part.top() - px(1.0));
    assert_eq!(rest.bottom(), part.bottom() + px(1.0));
    assert_eq!(rest.right(), part.right() + px(1.0));
    assert!(part.size.width > px(0.0));
    let window_width = cx
        .update_window(window.into(), |_, window, _| window.viewport_size().width)
        .unwrap();
    assert!(rest.right() <= window_width, "cut at the window's edge");

    // Not for a row that fits; gone when the pointer leaves the rows.
    let src = drawn(cx, window, "tree-row:src").center();
    mouse_to(cx, window, src);
    assert_eq!(tree(cx, window)["expanded_row"], json!(null));
    mouse_to(cx, window, root_row.center());
    assert_eq!(tree(cx, window)["expanded_row"], json!(""));
    mouse_to(
        cx,
        window,
        point(area.right() + px(200.0), area.top() + px(200.0)),
    );
    assert_eq!(tree(cx, window)["expanded_row"], json!(null));
}

#[gpui::test]
fn selecting_a_cut_row_with_the_keyboard_shows_its_rest(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join(LONG_ROOT);
    layout(&root);
    let window = open(cx, &root);
    // Focus the tree on a short row, then go up to the root.
    click(cx, window, "tree-row:src");
    assert_eq!(tree(cx, window)["expanded_row"], json!(null));
    press(cx, window, "up");
    let t = tree(cx, window);
    assert_eq!(t["selected"], json!(""));
    assert_eq!(t["expanded_row"], json!(""));
    // Down again: the short row shows nothing.
    press(cx, window, "down");
    assert_eq!(tree(cx, window)["expanded_row"], json!(null));
}

#[gpui::test]
fn the_rest_of_a_row_shows_10_ms_after_the_pointer_comes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join(LONG_ROOT);
    layout(&root);
    let window = open(cx, &root);
    let at = drawn(cx, window, "tree-row:").center();
    VisualTestContext::from_window(window.into(), cx).simulate_mouse_move(
        at,
        None,
        Modifiers::none(),
    );
    draw_now(cx, window);
    assert_eq!(tree_now(cx, window)["expanded_row"], json!(null));
    cx.executor().advance_clock(DELAY);
    draw_now(cx, window);
    assert_eq!(tree_now(cx, window)["expanded_row"], json!(""));
}

#[gpui::test]
fn the_rest_of_a_row_stays_under_the_pointer_and_takes_the_rows_clicks(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join(LONG_ROOT);
    layout(&root);
    let window = open(cx, &root);
    click(cx, window, "tree-row:src");
    let root_row = drawn(cx, window, "tree-row:");
    mouse_to(cx, window, root_row.center());
    let rest = drawn(cx, window, "tree-row-expansion");

    // The pointer moves from the row onto its rest: it stays.
    mouse_to(cx, window, rest.center());
    let t = tree(cx, window);
    assert_eq!(t["expanded_row"], json!(""));
    assert_eq!(t["selected"], json!("src"));

    // A click on it selects the row; a double click opens (here: closes)
    // the folder, another one opens it again.
    click_at(cx, window, rest.center(), 1);
    let t = tree(cx, window);
    assert_eq!(t["selected"], json!(""));
    assert_eq!(t["expanded_row"], json!(""));
    click_at(cx, window, rest.center(), 2);
    assert_eq!(row(&tree(cx, window), "")["expanded"], json!(false));
    click_at(cx, window, rest.center(), 2);
    assert_eq!(row(&tree(cx, window), "")["expanded"], json!(true));

    // Away from both the row and its rest: it goes.
    mouse_to(
        cx,
        window,
        point(rest.right() + px(40.0), rest.bottom() + px(200.0)),
    );
    assert_eq!(tree(cx, window)["expanded_row"], json!(null));

    // Back to the row and onto its rest: a right click opens the row's
    // context menu.
    click(cx, window, "tree-row:src");
    mouse_to(cx, window, root_row.center());
    mouse_to(cx, window, rest.center());
    right_click_at(cx, window, rest.center());
    let t = tree(cx, window);
    assert_eq!(t["selected"], json!(""));
    assert!(t["context_menu"].is_object(), "no context menu: {t}");
}
