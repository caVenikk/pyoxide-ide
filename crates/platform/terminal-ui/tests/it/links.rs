//! Links in the output of a session: URLs that a plain click opens, paths
//! that open with Cmd (Ctrl) held and show the hint "Open" after a plain
//! click, and the filters a product adds. A shell command prints the text
//! and hands over to `cat`, so the screen holds exactly that.
#![cfg(unix)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use console::Link;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use serde_json::Value;
use terminal_ui::{LinkFilter, LinkTarget, SessionView, SessionViewEvent};

use crate::view::{at, cell, click, frame, open, wait};

/// The links the view reports to its owner.
type Opened = Rc<RefCell<Vec<LinkTarget>>>;

fn opened(cx: &mut VisualTestContext, view: &Entity<SessionView>) -> (Opened, gpui::Subscription) {
    let list = Opened::default();
    let sink = list.clone();
    let subscription = cx.update(|_, cx| {
        cx.subscribe(view, move |_, event: &SessionViewEvent, _| {
            if let SessionViewEvent::OpenLink(target) = event {
                sink.borrow_mut().push(target.clone());
            }
        })
    });
    (list, subscription)
}

/// Cmd on macOS, Ctrl elsewhere.
fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    }
}

/// Prints `text` (printf format, `\r\n` for line breaks) and keeps the
/// session open.
fn shell_printing(text: &str) -> Vec<String> {
    vec!["-c".into(), format!("printf '{text}'; exec cat")]
}

fn open_printing<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<SessionView>, &'a mut VisualTestContext) {
    let args = shell_printing(text);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    open(cx, "/bin/sh", &args)
}

/// A folder with a Python file in it, for the paths to point at.
fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pyoxide-links-{}-{name}", std::process::id()));
    std::fs::create_dir_all(dir.join("pkg")).unwrap();
    std::fs::write(dir.join("pkg").join("app.py"), "print(1)\n").unwrap();
    dir.canonicalize().unwrap()
}

/// Draws frames for about `ms` of the real clock (the hint's delay is
/// measured on it) and the executor's.
fn pass(cx: &mut VisualTestContext, ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
    for _ in 0..5 {
        frame(cx);
    }
}

fn link_of_kind(s: &Value, kind: &str) -> Option<Value> {
    s["links"]
        .as_array()?
        .iter()
        .find(|l| l["kind"] == kind)
        .cloned()
}

#[gpui::test]
fn a_url_is_underlined_and_a_plain_click_opens_it(cx: &mut TestAppContext) {
    let (view, cx) = open_printing(cx, "see https://example.com/a?b=1 now\\r\\n");
    let s = wait(cx, &view, "the URL link", |s| {
        link_of_kind(s, "url").is_some()
    });
    let link = link_of_kind(&s, "url").unwrap();
    assert_eq!(link["target"], "https://example.com/a?b=1");
    assert_eq!(link["visible"], true, "a URL is underlined without a key");
    assert_eq!(link["columns"], serde_json::json!([4, 29]));
    let (list, _subscription) = opened(cx, &view);
    // A click beside the link opens nothing, one on it opens it.
    let p = cell(cx, &view, 0, 1);
    click(cx, p, 1, Modifiers::none());
    pass(cx, 0);
    assert!(list.borrow().is_empty());
    let p = cell(cx, &view, 0, 10);
    click(cx, p, 1, Modifiers::none());
    pass(cx, 0);
    assert_eq!(
        *list.borrow(),
        vec![LinkTarget::Url("https://example.com/a?b=1".into())]
    );
}

#[gpui::test]
fn an_osc_8_hyperlink_opens_its_target(cx: &mut TestAppContext) {
    let (view, cx) = open_printing(
        cx,
        "\\033]8;;https://example.org/docs\\033\\\\the docs\\033]8;;\\033\\\\ end\\r\\n",
    );
    let s = wait(cx, &view, "the hyperlink", |s| {
        link_of_kind(s, "url").is_some()
    });
    assert_eq!(
        link_of_kind(&s, "url").unwrap()["target"],
        "https://example.org/docs"
    );
    let (list, _subscription) = opened(cx, &view);
    let p = cell(cx, &view, 0, 3);
    click(cx, p, 1, Modifiers::none());
    pass(cx, 0);
    assert_eq!(
        *list.borrow(),
        vec![LinkTarget::Url("https://example.org/docs".into())]
    );
}

#[gpui::test]
fn a_path_opens_with_the_command_key_and_shows_a_hint_on_a_plain_click(cx: &mut TestAppContext) {
    let dir = project("path");
    let file = dir.join("pkg").join("app.py");
    let text = format!(
        "at {}:12:5 failed\\r\\nand {}:3 is missing\\r\\n",
        file.display(),
        dir.join("nothing.py").display()
    );
    let (view, cx) = open_printing(cx, &text);
    wait(cx, &view, "the output", |s| {
        s["lines"][1]
            .as_str()
            .is_some_and(|l| l.contains("missing"))
    });
    let s = view.read_with(cx, |v, _| v.state());
    assert!(
        link_of_kind(&s, "file").is_none_or(|l| l["visible"] == false),
        "a path in the text is not underlined by itself: {s:#}"
    );
    let (list, _subscription) = opened(cx, &view);
    let p = cell(cx, &view, 0, 8);

    // Cmd+click opens the file at its line and column.
    click(cx, p, 1, command());
    wait(cx, &view, "the opened file", |_| !list.borrow().is_empty());
    assert_eq!(
        *list.borrow(),
        vec![LinkTarget::File {
            path: file.clone(),
            line: Some(12),
            column: Some(5),
        }]
    );
    list.borrow_mut().clear();

    // A plain click opens nothing; after the delay the hint offers it.
    click(cx, p, 1, Modifiers::none());
    pass(cx, 220);
    let s = wait(cx, &view, "the hint", |s| !s["link_hint"].is_null());
    assert!(list.borrow().is_empty());
    assert_eq!(s["link_hint"]["target"], format!("{}:12:5", file.display()));
    // The next press hides it.
    let elsewhere = cell(cx, &view, 3, 0);
    click(cx, elsewhere, 1, Modifiers::none());
    wait(cx, &view, "the hint gone", |s| s["link_hint"].is_null());

    // A path to a file that does not exist is not a link.
    let missing = cell(cx, &view, 1, 8);
    click(cx, missing, 1, command());
    pass(cx, 50);
    click(cx, missing, 1, Modifiers::none());
    pass(cx, 220);
    let s = view.read_with(cx, |v, _| v.state());
    assert!(list.borrow().is_empty());
    assert!(s["link_hint"].is_null(), "{s:#}");
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn the_open_link_command_opens_the_link_at_a_cell(cx: &mut TestAppContext) {
    let dir = project("command");
    let file = dir.join("pkg").join("app.py");
    let text = format!("{}\\r\\n", file.display());
    let (view, cx) = open_printing(cx, &text);
    wait(cx, &view, "the output", |s| s["lines"][0] != "");
    let (list, _subscription) = opened(cx, &view);
    view.update(cx, |v, cx| v.open_link_at(terminal::Point::new(0, 2), cx));
    wait(cx, &view, "the opened file", |_| !list.borrow().is_empty());
    assert_eq!(
        *list.borrow(),
        vec![LinkTarget::File {
            path: file,
            line: None,
            column: None,
        }]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn a_relative_path_is_taken_from_the_working_directory_of_the_shell(cx: &mut TestAppContext) {
    let dir = project("relative");
    // The shell reports its folder (OSC 7), then names a file in it.
    let text = format!(
        "\\033]7;file://localhost{}\\033\\\\pkg/app.py:7\\r\\n",
        dir.display()
    );
    let (view, cx) = open_printing(cx, &text);
    wait(cx, &view, "the folder", |s| {
        s["working_directory"]
            .as_str()
            .is_some_and(|d| d.ends_with("relative"))
    });
    let (list, _subscription) = opened(cx, &view);
    let p = cell(cx, &view, 0, 4);
    click(cx, p, 1, command());
    wait(cx, &view, "the opened file", |_| !list.borrow().is_empty());
    assert_eq!(
        *list.borrow(),
        vec![LinkTarget::File {
            path: dir.join("pkg").join("app.py"),
            line: Some(7),
            column: None,
        }]
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Links every `#<digits>` as an issue.
struct Issues;

impl LinkFilter for Issues {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        for (start, _) in line.match_indices('#') {
            let digits = line[start + 1..]
                .chars()
                .take_while(char::is_ascii_digit)
                .count();
            if digits > 0 {
                out.push(Link {
                    range: start..start + 1 + digits,
                    target: LinkTarget::Url(format!(
                        "issue:{}",
                        &line[start + 1..start + 1 + digits]
                    )),
                });
            }
        }
    }
}

#[gpui::test]
fn the_filters_of_a_product_add_links(cx: &mut TestAppContext) {
    let (view, cx) = open_printing(cx, "fixes #160 and #7 (see a.py)\\r\\n");
    view.update(cx, |v, cx| {
        v.set_link_filters(Some(Arc::new(|_| vec![Arc::new(Issues)])), cx)
    });
    let s = wait(cx, &view, "the filtered links", |s| {
        s["links"].as_array().is_some_and(|l| l.len() == 2)
    });
    let links = s["links"].as_array().unwrap();
    assert_eq!(links[0]["target"], "issue:160");
    assert_eq!(links[0]["columns"], serde_json::json!([6, 10]));
    assert_eq!(links[1]["target"], "issue:7");
    assert!(links.iter().all(|l| l["visible"] == true));
    let (list, _subscription) = opened(cx, &view);
    let p = at(cx, &view, 0, 8.5);
    click(cx, p, 1, Modifiers::none());
    pass(cx, 0);
    assert_eq!(*list.borrow(), vec![LinkTarget::Url("issue:160".into())]);
}
