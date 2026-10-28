//! The find bar of a session: the matches are counted over the whole
//! output (history included) on a background thread, the options change
//! them, Next and Previous wrap around, new output is counted, and closing
//! the bar gives the focus back. A shell command prints the text and hands
//! over to `cat`.
#![cfg(unix)]

use gpui::{Entity, TestAppContext, VisualTestContext};
use serde_json::Value;
use terminal::SearchDirection;
use terminal_ui::SessionView;
use terminal_ui::commands::{FindToggleCase, FindToggleRegex};

use crate::view::{open, open_with, options, wait};

fn open_printing<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<SessionView>, &'a mut VisualTestContext) {
    open(
        cx,
        "/bin/sh",
        &["-c", &format!("printf '{text}'; exec cat")],
    )
}

fn find_text(cx: &mut VisualTestContext, view: &Entity<SessionView>, text: &str) {
    cx.update(|window, cx| view.update(cx, |v, cx| v.open_find(Some(text), window, cx)));
}

fn step(cx: &mut VisualTestContext, view: &Entity<SessionView>, direction: SearchDirection) {
    cx.update(|window, cx| view.update(cx, |v, cx| v.find_step(direction, window, cx)));
}

fn find(s: &Value) -> &Value {
    &s["find"]
}

#[gpui::test]
fn the_matches_are_counted_and_next_and_previous_wrap_around(cx: &mut TestAppContext) {
    let (view, cx) = open_printing(cx, "alpha beta\\r\\nbeta gamma\\r\\nBETA\\r\\n");
    wait(cx, &view, "the output", |s| s["lines"][2] == "BETA");
    assert_eq!(find(&view.read_with(cx, |v, _| v.state()))["open"], false);
    find_text(cx, &view, "beta");
    let s = wait(cx, &view, "three matches", |s| {
        find(s)["matches"] == 3 && !find(s)["current"].is_null()
    });
    assert_eq!(find(&s)["query"], "beta");
    let first = find(&s)["current"].as_u64().unwrap();
    // Next goes through the three matches and back to the first.
    let mut seen = vec![first];
    for _ in 0..3 {
        let last = *seen.last().unwrap();
        step(cx, &view, SearchDirection::Next);
        let s = wait(cx, &view, "the next match", |s| {
            find(s)["current"].as_u64().is_some_and(|c| c != last)
        });
        seen.push(find(&s)["current"].as_u64().unwrap());
    }
    assert_eq!(seen[0], seen[3], "wraps around: {seen:?}");
    let mut sorted = seen[..3].to_vec();
    sorted.sort();
    assert_eq!(sorted, vec![1, 2, 3]);
    // Previous goes back.
    step(cx, &view, SearchDirection::Previous);
    let s = wait(cx, &view, "the previous match", |s| {
        find(s)["current"].as_u64() == Some(seen[2])
    });
    assert_eq!(find(&s)["matches"], 3);
}

#[gpui::test]
fn match_case_and_regex_change_the_matches(cx: &mut TestAppContext) {
    let (view, cx) = open_printing(cx, "alpha beta\\r\\nbeta gamma\\r\\nBETA\\r\\n");
    wait(cx, &view, "the output", |s| s["lines"][2] == "BETA");
    find_text(cx, &view, "beta");
    wait(cx, &view, "three matches", |s| find(s)["matches"] == 3);
    // Match Case leaves the upper-case one out.
    cx.dispatch_action(FindToggleCase);
    let s = wait(cx, &view, "two matches", |s| find(s)["matches"] == 2);
    assert_eq!(find(&s)["match_case"], true);
    cx.dispatch_action(FindToggleCase);
    wait(cx, &view, "three matches again", |s| {
        find(s)["matches"] == 3
    });
    // As a regular expression `b.ta` matches too, and `(` is not valid.
    find_text(cx, &view, "b.ta");
    wait(cx, &view, "no match for the dot", |s| {
        find(s)["matches"] == 0
    });
    cx.dispatch_action(FindToggleRegex);
    let s = wait(cx, &view, "three regex matches", |s| {
        find(s)["matches"] == 3
    });
    assert_eq!(find(&s)["regex"], true);
    find_text(cx, &view, "(");
    let s = wait(cx, &view, "the error", |s| !find(s)["error"].is_null());
    assert_eq!(find(&s)["matches"], 0);
}

#[gpui::test]
fn output_that_arrives_while_the_bar_is_open_is_counted(cx: &mut TestAppContext) {
    let (view, cx) = open_printing(cx, "one two\\r\\n");
    wait(cx, &view, "the output", |s| s["lines"][0] == "one two");
    find_text(cx, &view, "two");
    wait(cx, &view, "one match", |s| find(s)["matches"] == 1);
    // `cat` echoes what is typed and prints it back: two more.
    view.update(cx, |v, cx| v.type_text("two\r", cx));
    wait(cx, &view, "three matches", |s| find(s)["matches"] == 3);
}

#[gpui::test]
fn the_history_is_searched_and_closing_gives_the_focus_back(cx: &mut TestAppContext) {
    // Far more lines than the screen holds: the match is in the history.
    let mut opts = options(
        "/bin/sh",
        &[
            "-c",
            "echo needle; i=0; while [ $i -lt 200 ]; do echo line$i; i=$((i+1)); done; exec cat",
        ],
    );
    opts.scrollback_lines = 1000;
    let (view, cx) = open_with(cx, opts);
    wait(cx, &view, "the history", |s| {
        s["history"].as_u64().is_some_and(|h| h > 60) && s["lines"].as_array().is_some()
    });
    find_text(cx, &view, "needle");
    let s = wait(cx, &view, "the match in the history", |s| {
        find(s)["matches"] == 1
    });
    assert_eq!(find(&s)["current"], 1);
    // The match is scrolled into view.
    let s = wait(cx, &view, "the view scrolled to it", |s| {
        s["display_offset"].as_u64().is_some_and(|o| o > 0)
    });
    assert!(find(&s)["visible_matches"].as_u64().unwrap() >= 1, "{s:#}");
    // Closing the bar clears the marks and focuses the session.
    cx.update(|window, cx| view.update(cx, |v, cx| v.close_find(window, cx)));
    let s = wait(cx, &view, "the bar closed", |s| find(s)["open"] == false);
    assert_eq!(find(&s)["visible_matches"], Value::Null);
    let focused =
        cx.update(|window, cx| gpui::Focusable::focus_handle(view.read(cx), cx).is_focused(window));
    assert!(focused);
}
