//! Notification balloons: the stack at the bottom right, the time they
//! stay, hover, the close button, action links, long content
//! (`workbench.notify`), and the Notifications tool window that lists
//! them.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle, point, px,
};
use serde_json::{Value, json};
use ui::theme::ThemeExt as _;
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
    // And the Notifications tool window opens with all five.
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(
        state["tool_windows"]["notifications"]["visible"],
        json!(true)
    );
    let timeline = state["notifications"]["timeline"].as_array().unwrap().len();
    assert_eq!(timeline, 5);
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

fn tool_window_visible(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> bool {
    invoke(cx, window, "workbench.state", json!(null))["tool_windows"]["notifications"]["visible"]
        == json!(true)
}

fn timeline(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Vec<Value> {
    balloons(cx, window)["timeline"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The bell of the right stripe opens the Notifications tool window
/// (#201): the empty text, then the notifications, the newest first,
/// until "Clear all"; a second click hides it.
#[gpui::test]
fn the_bell_opens_the_notifications_tool_window(cx: &mut TestAppContext) {
    let window = open(cx);
    let bell = bounds(cx, window, "stripe-notifications").unwrap();
    click(cx, window, bell.center(), Modifiers::none());
    assert!(tool_window_visible(cx, window));
    let tool_window = bounds(cx, window, "notifications-tool-window").unwrap();
    let stripe = bounds(cx, window, "right-stripe").unwrap();
    assert!(tool_window.right() <= stripe.left());
    let empty = bounds(cx, window, "notifications-empty").unwrap();
    assert!(tool_window.contains(&empty.center()));

    let first = notify(
        cx,
        window,
        json!({ "title": "3 files committed", "content": "test commit",
                "actions": [{ "text": "Edit Commit Message", "command": "workbench.state" }] }),
    );
    let second = notify(
        cx,
        window,
        json!({ "kind": "warning", "title": "Low memory" }),
    );
    let entries = timeline(cx, window);
    let listed: Vec<&str> = entries
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ["Low memory", "3 files committed"]);
    assert_eq!(entries[1]["actions"], json!(["Edit Commit Message"]));
    assert!(!entries[0]["time"].as_str().unwrap().is_empty());
    let newer = bounds(cx, window, &format!("notification-entry-{second}")).unwrap();
    let older = bounds(cx, window, &format!("notification-entry-{first}")).unwrap();
    assert!(newer.bottom() <= older.top(), "the newest on top");
    assert!(tool_window.contains(&older.center()));

    // "Clear all" empties the timeline and closes the balloons.
    let clear = bounds(cx, window, "notifications-clear-all").unwrap();
    click(cx, window, clear.center(), Modifiers::none());
    assert!(timeline(cx, window).is_empty());
    assert!(titles(cx, window).is_empty());
    assert!(tool_window_visible(cx, window));

    // The bell again: hidden.
    click(cx, window, bell.center(), Modifiers::none());
    assert!(!tool_window_visible(cx, window));
}

/// Long content wraps inside the balloon (#201): at 240 px once it does
/// not fit on a line of 320, three lines under a title, the chevron at
/// the right of the last one; a click on it (or on the text) shows the
/// whole text, the chevron below collapses it again.
#[gpui::test]
fn long_content_wraps_inside_the_balloon_and_expands(cx: &mut TestAppContext) {
    let window = open(cx);
    let content = "Imported 4 themes and 1 editor color scheme. Dusk is now the theme. \
        The keys that could not be mapped are listed in the log of the IDE, \
        group by group, with their counts.";
    let id = notify(
        cx,
        window,
        json!({ "title": "Themes imported", "content": content }),
    );
    let chevron = bounds(cx, window, &format!("balloon-expand-{id}"))
        .expect("the expand chevron of long content");
    let state = &balloons(cx, window)["balloons"][0];
    let lines: Vec<String> = state["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_owned())
        .collect();
    assert!(lines.len() > 3, "{lines:?}");
    assert_eq!(lines.join(" "), content);
    assert_eq!(state["shown_lines"], json!(3));
    assert_eq!(state["expandable"], json!(true));
    let width = state["content_width"].as_f64().unwrap();
    assert!(width > 200.0 && width <= 240.0, "{width}");

    let balloon = bounds(cx, window, &format!("balloon-{id}")).unwrap();
    let text = bounds(cx, window, &format!("balloon-content-{id}")).unwrap();
    assert_eq!(text.size.height, px(48.0), "three lines of 16");
    assert!(text.right() <= balloon.right());
    // Every line fits in 240 px (the test font: 0.6 em per character).
    let ui_size = cx.update(|cx| cx.theme().ui_size());
    for line in &lines {
        let width = ui_size * 0.6 * line.chars().count() as f32;
        assert!(width <= px(240.0), "{line:?} is {width:?} wide");
    }
    let third = bounds(cx, window, &format!("balloon-line-{id}-2")).unwrap();
    assert_eq!(chevron.top(), third.top());
    assert!(chevron.right() <= balloon.right() - px(9.0));
    assert!(chevron.left() > text.left() + px(240.0));

    click(cx, window, chevron.center(), Modifiers::none());
    let state = &balloons(cx, window)["balloons"][0];
    assert_eq!(state["expanded"], json!(true));
    assert_eq!(state["shown_lines"], json!(lines.len()));
    let text = bounds(cx, window, &format!("balloon-content-{id}")).unwrap();
    assert_eq!(text.size.height, px(16.0) * lines.len() as f32);
    let collapse = bounds(cx, window, &format!("balloon-expand-{id}")).unwrap();
    assert!(collapse.top() >= text.bottom(), "below the text");
    click(cx, window, collapse.center(), Modifiers::none());
    assert_eq!(
        balloons(cx, window)["balloons"][0]["expanded"],
        json!(false)
    );

    // A click on the collapsed text expands it too.
    let text = bounds(cx, window, &format!("balloon-content-{id}")).unwrap();
    click(
        cx,
        window,
        point(text.left() + px(20.0), text.center().y),
        Modifiers::none(),
    );
    assert_eq!(balloons(cx, window)["balloons"][0]["expanded"], json!(true));

    // Content that fits on a line of 320 px stays on one line.
    let short = notify(
        cx,
        window,
        json!({ "title": "Saved", "content": "All files were saved." }),
    );
    let state = &balloons(cx, window)["balloons"][0];
    assert_eq!(state["id"], json!(short));
    assert_eq!(state["lines"], json!(["All files were saved."]));
    assert_eq!(state["expandable"], json!(false));
}
