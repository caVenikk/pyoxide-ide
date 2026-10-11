//! Headless tests of the main window (GPUI's test platform: no window
//! server, no GPU), driven through command ids.

use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, VisualTestContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, Workbench};

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
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
    cx.run_until_parked();
    // The test platform draws only when asked: render the next frame as
    // the display link would.
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    out
}

#[gpui::test]
fn the_chrome_is_drawn(cx: &mut TestAppContext) {
    let window = open(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    for part in [
        "toolbar",
        "project-widget",
        "left-stripe",
        "right-stripe",
        "project-tool-window",
        "editor-area",
        "status-bar",
    ] {
        assert!(cx.debug_bounds(part).is_some(), "{part} is not drawn");
    }
    let toolbar = cx.debug_bounds("toolbar").unwrap();
    let status = cx.debug_bounds("status-bar").unwrap();
    assert_eq!(toolbar.size.height, gpui::px(40.0));
    assert_eq!(status.size.height, gpui::px(30.0));
    assert!(toolbar.origin.y < status.origin.y);
}

#[gpui::test]
fn toggles_the_project_tool_window_by_command(cx: &mut TestAppContext) {
    let window = open(cx);
    let editor_width = |cx: &mut TestAppContext| {
        VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("editor-area")
            .unwrap()
            .size
            .width
    };
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(true));
    assert_eq!(state["product"], json!("Test IDE"));
    let narrow = editor_width(cx);

    invoke(cx, window, "workbench.toggle-project", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(false));
    // The editor island takes the space of the hidden tool window.
    assert!(editor_width(cx) > narrow);

    invoke(cx, window, "workbench.toggle-project", json!(null));
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["tool_windows"]["project"]["visible"], json!(true));
}

#[gpui::test]
fn switches_the_theme_by_command(cx: &mut TestAppContext) {
    let window = open(cx);
    invoke(
        cx,
        window,
        "workbench.set-theme",
        json!({"id": "pyoxide-light"}),
    );
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(state["theme"], json!("pyoxide-light"));
}

#[gpui::test]
fn lists_its_commands(cx: &mut TestAppContext) {
    let _window = open(cx);
    let ids: Vec<&str> = cx.update(|cx| {
        ui::command::list(cx)
            .into_iter()
            .map(|c| c.meta.id)
            .collect()
    });
    for id in [
        "app.quit",
        "workbench.set-theme",
        "workbench.state",
        "workbench.toggle-project",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
}
