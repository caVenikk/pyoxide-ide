//! Notification balloons: the stack at the bottom right, the time they
//! stay, hover, the close button and action links (`workbench.notify`).

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle, point, px,
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

fn balloons(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    invoke(cx, window, "workbench.state", json!(null))["notifications"].clone()
}

fn titles(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Vec<String> {
    balloons(cx, window)["balloons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["title"].as_str().unwrap().to_owned())
        .collect()
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    VisualTestContext::from_window(window.into(), cx).debug_bounds(selector)
}

fn move_to(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    at: gpui::Point<gpui::Pixels>,
) {
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    draw(cx, window);
}

fn click(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    at: gpui::Point<gpui::Pixels>,
    modifiers: Modifiers,
) {
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, modifiers);
    vcx.simulate_click(at, modifiers);
    draw(cx, window);
}

fn wait(cx: &mut TestAppContext, window: WindowHandle<Workbench>, secs: u64) {
    cx.executor().advance_clock(Duration::from_secs(secs));
    draw(cx, window);
}

fn notify(cx: &mut TestAppContext, window: WindowHandle<Workbench>, args: Value) -> u64 {
    invoke(cx, window, "workbench.notify", args);
    balloons(cx, window)["balloons"][0]["id"].as_u64().unwrap()
}

/// A point of the editor area, away from the balloons.
fn elsewhere() -> gpui::Point<gpui::Pixels> {
    point(px(400.0), px(200.0))
}

#[gpui::test]
fn balloons_stack_upwards_and_older_ones_collapse(cx: &mut TestAppContext) {
    let window = open(cx);
    let a = notify(cx, window, json!({ "title": "a", "content": "first" }));
    let b = notify(cx, window, json!({ "title": "b", "kind": "warning" }));
    let state = balloons(cx, window);
    assert_eq!(state["balloons"][0]["kind"], json!("warning"));
    assert_eq!(state["collapsed"], json!(0));
    // At the bottom right, the newest on top, 10 apart, 370 wide.
    let (ba, bb) = (
        bounds(cx, window, &format!("balloon-{a}")).unwrap(),
        bounds(cx, window, &format!("balloon-{b}")).unwrap(),
    );
    assert_eq!(ba.size.width, px(370.0));
    assert_eq!(bb.right(), ba.right());
    assert_eq!(bb.bottom() + px(10.0), ba.top());
    let viewport = cx
        .update_window(window.into(), |_, window, _| window.viewport_size())
        .unwrap();
    let stripe = bounds(cx, window, "right-stripe").unwrap();
    assert_eq!(ba.right(), stripe.left() - px(10.0));
    assert!(ba.bottom() < viewport.height);

    notify(cx, window, json!({ "title": "c" }));
    notify(cx, window, json!({ "title": "d" }));
    assert_eq!(titles(cx, window), ["d", "c", "b"]);
    assert_eq!(balloons(cx, window)["collapsed"], json!(1));
    notify(cx, window, json!({ "title": "e" }));
    assert_eq!(titles(cx, window), ["e", "d", "c"]);
    assert_eq!(balloons(cx, window)["collapsed"], json!(2));
    // The strip lies below the oldest balloon; a click closes all.
    let more = bounds(cx, window, "balloon-more").unwrap();
    let oldest = bounds(cx, window, &format!("balloon-{}", b + 1)).unwrap();
    assert!(more.top() < oldest.bottom() && more.bottom() > oldest.bottom());
    click(
        cx,
        window,
        point(more.center().x, more.bottom() - px(6.0)),
        Modifiers::none(),
    );
    assert!(titles(cx, window).is_empty());
}

#[gpui::test]
fn a_balloon_goes_after_ten_seconds_of_activity_and_an_error_stays(cx: &mut TestAppContext) {
    let window = open(cx);
    notify(
        cx,
        window,
        json!({ "title": "Saved", "content": "All files were saved." }),
    );
    notify(
        cx,
        window,
        json!({ "kind": "error", "title": "Push failed", "content": "rejected" }),
    );
    let state = balloons(cx, window);
    assert_eq!(state["balloons"][0]["sticky"], json!(true));
    assert_eq!(state["balloons"][0]["running"], json!(false));
    // Without input the time does not run.
    wait(cx, window, 60);
    assert_eq!(titles(cx, window), ["Push failed", "Saved"]);
    move_to(cx, window, elsewhere());
    assert_eq!(balloons(cx, window)["balloons"][1]["running"], json!(true));
    wait(cx, window, 11);
    assert_eq!(titles(cx, window), ["Push failed"]);
    wait(cx, window, 300);
    assert!(titles(cx, window).is_empty());
}

#[gpui::test]
fn hover_keeps_a_balloon_and_shows_its_close_button(cx: &mut TestAppContext) {
    let window = open(cx);
    let id = notify(
        cx,
        window,
        json!({ "title": "Indexing", "content": "Done" }),
    );
    let other = notify(cx, window, json!({ "title": "Other" }));
    move_to(cx, window, elsewhere());
    assert!(bounds(cx, window, &format!("balloon-close-{id}")).is_none());
    let balloon = bounds(cx, window, &format!("balloon-{id}")).unwrap();
    move_to(cx, window, balloon.center());
    assert_eq!(balloons(cx, window)["balloons"][1]["hovered"], json!(true));
    wait(cx, window, 30);
    // The hovered one stays, the other went.
    assert_eq!(titles(cx, window), ["Indexing"]);
    let _ = other;
    // Its close button shows while hovered and closes it.
    let close = bounds(cx, window, &format!("balloon-close-{id}")).unwrap();
    click(cx, window, close.center(), Modifiers::none());
    assert!(titles(cx, window).is_empty());

    // Alt-click closes all.
    let a = notify(cx, window, json!({ "title": "a" }));
    notify(cx, window, json!({ "title": "b" }));
    let balloon = bounds(cx, window, &format!("balloon-{a}")).unwrap();
    move_to(cx, window, balloon.center());
    let close = bounds(cx, window, &format!("balloon-close-{a}")).unwrap();
    click(cx, window, close.center(), Modifiers::alt());
    assert!(titles(cx, window).is_empty());
}

#[gpui::test]
fn an_action_link_runs_its_command_and_closes_the_balloon(cx: &mut TestAppContext) {
    let window = open(cx);
    let theme = |cx: &mut TestAppContext| {
        invoke(cx, window, "workbench.state", json!(null))["theme"].clone()
    };
    assert_ne!(theme(cx), json!("islands-light"));
    let id = notify(
        cx,
        window,
        json!({
            "title": "Theme",
            "content": "Switch to the light theme?",
            "actions": [
                { "text": "Switch", "command": "workbench.set-theme", "args": { "id": "islands-light" } },
                { "text": "Keep", "command": "workbench.state" }
            ]
        }),
    );
    let state = balloons(cx, window);
    assert_eq!(state["balloons"][0]["actions"], json!(["Switch", "Keep"]));
    let switch = bounds(cx, window, &format!("balloon-action-{id}-0")).unwrap();
    let keep = bounds(cx, window, &format!("balloon-action-{id}-1")).unwrap();
    assert_eq!(keep.left() - switch.right(), px(16.0));
    click(cx, window, switch.center(), Modifiers::none());
    assert!(titles(cx, window).is_empty());
    assert_eq!(theme(cx), json!("islands-light"));
}

#[gpui::test]
fn the_action_links_show_the_hand_and_the_close_button_the_arrow(cx: &mut TestAppContext) {
    let window = open(cx);
    let shape = |cx: &mut TestAppContext, selector: String| {
        let at = bounds(cx, window, &selector).unwrap().center();
        move_to(cx, window, at);
        VisualTestContext::from_window(window.into(), cx)
            .update(|window, _| ui::pointer::resolve(window))
    };
    let id = notify(
        cx,
        window,
        json!({
            "title": "Theme",
            "content": "Switch to the light theme?",
            "actions": [{ "text": "Keep", "command": "workbench.state" }]
        }),
    );
    assert_eq!(
        shape(cx, format!("balloon-action-{id}-0")),
        gpui::CursorStyle::PointingHand
    );
    assert_eq!(
        shape(cx, format!("balloon-close-{id}")),
        gpui::CursorStyle::Arrow
    );
    assert_eq!(
        shape(cx, format!("balloon-content-{id}")),
        gpui::CursorStyle::Arrow
    );
}

#[gpui::test]
fn the_command_takes_kinds_and_rejects_unknown_fields(cx: &mut TestAppContext) {
    let window = open(cx);
    notify(
        cx,
        window,
        json!({ "kind": "info", "content": "only content", "sticky": true }),
    );
    let b = &balloons(cx, window)["balloons"][0];
    assert_eq!(b["kind"], json!("information"));
    assert_eq!(b["sticky"], json!(true));
    assert_eq!(b["title"], json!(""));
    let err = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke("workbench.notify", json!({ "colour": "red" }), window, cx)
        })
        .unwrap();
    assert!(err.is_err());
}
