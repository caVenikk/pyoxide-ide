//! Background tasks in the status bar and the Processes popup: the delay
//! before a task shows, the widget's text, counter and tooltip, the popup
//! and its link, cancelling, and the popup closing when its tasks end.

use std::rc::Rc;
use std::time::Duration;

use gpui::{TestAppContext, WindowHandle};
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
