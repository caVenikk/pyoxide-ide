//! Background tasks in the status bar and the Processes popup: the delay
//! before a task shows, the widget's text, counter and tooltip, the popup
//! and its link, cancelling, pausing, the popup closing when its tasks end,
//! moving and resizing it, and Auto Show.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext,
    WindowHandle, point, px,
};
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
    draw(cx, window);
    window
}

/// Lets the window take the tasks' updates (they come at most every
/// 50 ms) and draws it.
fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    for _ in 0..3 {
        cx.executor().advance_clock(progress::UPDATE_INTERVAL);
        cx.run_until_parked();
    }
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

fn tasks(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke("workbench.state", json!(null), window, cx)
    })
    .unwrap()
    .unwrap()["background_tasks"]
        .clone()
}

/// Waits (in real time) until the tasks started so far are shown.
fn wait_shown(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    std::thread::sleep(progress::SHOW_DELAY + Duration::from_millis(50));
    draw(cx, window);
}

/// Waits (in real time) until `done` holds for the tasks, at most 5 s.
fn wait_for(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    done: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..100 {
        let t = tasks(cx, window);
        if done(&t) {
            return t;
        }
        std::thread::sleep(Duration::from_millis(50));
        draw(cx, window);
    }
    panic!("timed out: {}", tasks(cx, window));
}

#[gpui::test]
fn tasks_show_in_the_status_bar_and_the_popup(cx: &mut TestAppContext) {
    let window = open(cx);
    assert_eq!(tasks(cx, window)["widget"], Value::Null);
    invoke(
        cx,
        window,
        "background-tasks.start",
        json!({"title": "Scanning files…", "fraction": 0.5, "seconds": 60}),
    );
    // Not before the delay.
    let t = tasks(cx, window);
    assert_eq!(t["tasks"][0]["shown"], json!(false), "{t}");
    assert_eq!(t["widget"], Value::Null, "{t}");
    wait_shown(cx, window);
    let t = tasks(cx, window);
    assert_eq!(
        t["widget"],
        json!({
            "text": "Scanning files…",
            "counter": "",
            "tooltip": "Scanning files…. Click to see all running processes",
        }),
        "{t}"
    );

    invoke(
        cx,
        window,
        "background-tasks.start",
        json!({"title": "Updating Python interpreter", "text": "Reading packages", "seconds": 60}),
    );
    wait_shown(cx, window);
    let t = tasks(cx, window);
    assert_eq!(t["widget"]["text"], json!("Scanning files…"), "{t}");
    assert_eq!(t["widget"]["counter"], json!("+1"), "{t}");

    invoke(cx, window, "background-tasks.show", json!(null));
    let t = tasks(cx, window);
    assert_eq!(t["popup"], json!(true));
    assert_eq!(t["widget"], json!({"link": "Hide processes (2)"}));

    // Cancelled: stopping until the work ends, then gone.
    let first = t["tasks"][0]["id"].clone();
    invoke(cx, window, "background-tasks.cancel", json!({"id": first}));
    let t = wait_for(cx, window, |t| t["tasks"].as_array().unwrap().len() == 1);
    assert_eq!(t["popup"], json!(true), "{t}");

    // The popup opened for running tasks closes when they end.
    let second = t["tasks"][0]["id"].clone();
    invoke(cx, window, "background-tasks.cancel", json!({"id": second}));
    let t = wait_for(cx, window, |t| t["tasks"].as_array().unwrap().is_empty());
    assert_eq!(t["popup"], json!(false), "{t}");
    assert_eq!(t["widget"], Value::Null, "{t}");
}

#[gpui::test]
fn the_popup_opens_without_tasks_and_toggles(cx: &mut TestAppContext) {
    let window = open(cx);
    invoke(cx, window, "background-tasks.show", json!(null));
    assert_eq!(tasks(cx, window)["popup"], json!(true));
    invoke(cx, window, "background-tasks.show", json!(null));
    assert_eq!(tasks(cx, window)["popup"], json!(false));
    invoke(cx, window, "background-tasks.show", json!(null));
    invoke(cx, window, "background-tasks.hide", json!(null));
    assert_eq!(tasks(cx, window)["popup"], json!(false));
}

#[gpui::test]
fn a_pausable_task_pauses_and_resumes(cx: &mut TestAppContext) {
    let window = open(cx);
    invoke(
        cx,
        window,
        "background-tasks.start",
        json!({
            "title": "Scanning files…",
            "text": "Scanning files…",
            "paused_text": "Scanning paused",
            "cancellable": false,
            "seconds": 60,
        }),
    );
    wait_shown(cx, window);
    let t = tasks(cx, window);
    let id = t["tasks"][0]["id"].clone();
    assert_eq!(t["tasks"][0]["pausable"], json!(true), "{t}");
    // The pause button only: the task cannot be cancelled.
    bounds(cx, window, &format!("status-progress-pause-{id}"));
    assert!(try_bounds(cx, window, &format!("status-progress-cancel-{id}")).is_none());

    invoke(cx, window, "background-tasks.pause", json!({"id": id}));
    let t = tasks(cx, window);
    assert_eq!(t["tasks"][0]["paused"], json!(true), "{t}");
    assert_eq!(t["widget"]["text"], json!("Scanning paused"), "{t}");

    // Its button resumes it.
    click(cx, window, &format!("status-progress-pause-{id}"));
    let t = tasks(cx, window);
    assert_eq!(t["tasks"][0]["paused"], json!(false), "{t}");
    assert_eq!(t["widget"]["text"], json!("Scanning files…"), "{t}");
}

#[gpui::test]
fn the_popup_moves_by_its_header_and_resizes_from_its_edges(cx: &mut TestAppContext) {
    let window = open(cx);
    invoke(cx, window, "background-tasks.show", json!(null));
    let popup = bounds(cx, window, "processes-popup");
    let header = bounds(cx, window, "processes-header").center();
    drag(cx, window, header, header - point(px(200.0), px(100.0)));
    let moved = bounds(cx, window, "processes-popup");
    assert_eq!(moved.origin, popup.origin - point(px(200.0), px(100.0)));
    assert_eq!(moved.size, popup.size);
    let t = tasks(cx, window);
    assert_eq!(t["moved"], json!(true), "{t}");
    assert_eq!(t["resized"], Value::Null, "{t}");

    // Wider from the left edge; the right edge stays.
    let left = point(moved.left() + px(1.0), moved.center().y);
    drag(cx, window, left, left - point(px(60.0), px(0.0)));
    let resized = bounds(cx, window, "processes-popup");
    assert_eq!(resized.right(), moved.right());
    assert_eq!(resized.size.width, moved.size.width + px(60.0));
    assert_ne!(tasks(cx, window)["resized"], Value::Null);

    // The place stays when the popup opens again.
    invoke(cx, window, "background-tasks.hide", json!(null));
    invoke(cx, window, "background-tasks.show", json!(null));
    assert_eq!(bounds(cx, window, "processes-popup"), resized);
}

#[gpui::test]
fn auto_show_opens_the_popup_for_a_second_task(cx: &mut TestAppContext) {
    let window = open(cx);
    // Only the active window opens it.
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    assert_eq!(tasks(cx, window)["auto_show"], json!(false));
    let start = |cx: &mut TestAppContext, title: &str| {
        invoke(
            cx,
            window,
            "background-tasks.start",
            json!({"title": title, "seconds": 60}),
        );
        wait_shown(cx, window);
    };
    // Off: no popup.
    start(cx, "First");
    start(cx, "Second");
    assert_eq!(tasks(cx, window)["popup"], json!(false));

    invoke(cx, window, "background-tasks.toggle-auto-show", json!(null));
    assert_eq!(tasks(cx, window)["auto_show"], json!(true));
    start(cx, "Third");
    assert_eq!(tasks(cx, window)["popup"], json!(true));
    invoke(cx, window, "background-tasks.toggle-auto-show", json!(null));
    assert_eq!(tasks(cx, window)["auto_show"], json!(false));
}

fn try_bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Option<gpui::Bounds<Pixels>> {
    draw(cx, window);
    // Debug selectors are looked up by a static name.
    let name: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    VisualTestContext::from_window(window.into(), cx).debug_bounds(name)
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> gpui::Bounds<Pixels> {
    try_bounds(cx, window, selector).unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = bounds(cx, window, selector).center();
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_click(at, Modifiers::none());
    draw(cx, window);
}

/// Presses at `from`, moves the pointer to `to` with the button held and
/// lets it go there.
fn drag(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    from: Point<Pixels>,
    to: Point<Pixels>,
) {
    let none = Modifiers::none();
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(from, None, none);
    vcx.simulate_mouse_down(from, MouseButton::Left, none);
    vcx.simulate_mouse_move(to, MouseButton::Left, none);
    vcx.simulate_mouse_up(to, MouseButton::Left, none);
    draw(cx, window);
}
