//! Editor splits (the reference's `EditorWindow.split`, `SplitterAction`):
//! Split Right / Down and Split and Move, a tab strip per group, focus and
//! moving between the groups, a file shown in two groups, closing the last
//! tab of a group, the splitter's drag.

use std::path::Path;

use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, point, px};
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;
use workbench::{FileType, Product};

use crate::mouse;

const MACOS: bool = cfg!(target_os = "macos");

fn product() -> Product {
    Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        ..Product::default()
    }
}

/// `a.py` to `c.py` (made when missing) open in tabs, `c.py` active.
fn open<'a>(cx: &'a mut TestAppContext, root: &Path) -> Harness<'a> {
    for name in ["a", "b", "c"] {
        let file = root.join(format!("{name}.py"));
        if !file.exists() {
            std::fs::write(file, format!("{name} = 1\n")).unwrap();
        }
    }
    let mut h = Harness::new(cx, product(), MACOS, Some(root));
    for name in ["a", "b", "c"] {
        h.invoke("project.open-file", json!({ "path": format!("{name}.py") }));
    }
    h
}

fn bounds(h: &mut Harness, part: &str) -> Bounds<Pixels> {
    // A selector is looked up by a `'static` name.
    let name: &'static str = Box::leak(part.to_owned().into_boxed_str());
    VisualTestContext::from_window(h.window.into(), h.cx)
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{part} is not drawn"))
}

fn click_at(h: &mut Harness, at: gpui::Point<Pixels>) {
    mouse::send(
        h.cx,
        h.window.into(),
        vec![mouse::press(at), mouse::release(at)],
    );
    h.draw();
}

/// The tabs of each group in tree order, as file names.
fn groups(h: &mut Harness) -> Vec<Vec<String>> {
    let state = h.state();
    let tabs = state["tabs"].as_array().unwrap().clone();
    state["splits"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            g["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| {
                    let t = &tabs[i.as_u64().unwrap() as usize];
                    t["path"].as_str().unwrap_or("?").to_owned()
                })
                .collect()
        })
        .collect()
}

fn focused(h: &mut Harness) -> u64 {
    h.state()["splits"]["focused"].as_u64().unwrap()
}

fn active_path(h: &mut Harness) -> String {
    let state = h.state();
    let i = state["active_tab"].as_u64().unwrap() as usize;
    state["tabs"][i]["path"].as_str().unwrap().to_owned()
}

fn editor_text(h: &mut Harness) -> String {
    h.invoke("editor.text", Value::Null)
        .as_str()
        .unwrap()
        .to_owned()
}

#[gpui::test]
fn split_right_shows_the_file_in_a_new_group_that_has_the_focus(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    assert_eq!(groups(&mut h), [["a.py", "b.py", "c.py"]]);
    h.invoke("tabs.split-right", json!(null));
    assert_eq!(groups(&mut h), [vec!["a.py", "b.py", "c.py"], vec!["c.py"]]);
    let state = h.state();
    assert_eq!(state["splits"]["focused"], json!(1));
    // The second view of c.py is the active tab.
    assert_eq!(state["active_tab"], json!(3));
    assert_eq!(state["tabs"][3]["mirror"], json!(true));
    assert!(state["tabs"][2].get("mirror").is_none());
    let tree = &state["splits"]["tree"]["split"];
    assert_eq!(tree["vertical"], json!(false));
    assert_eq!(tree["proportion"], json!(0.5));
    // Each group draws its own tab strip.
    bounds(&mut h, "editor-tabs");
    bounds(&mut h, "editor-tabs-0");
    let (left, right) = (
        bounds(&mut h, "editor-tabs-0"),
        bounds(&mut h, "editor-tabs"),
    );
    assert!(left.right() <= right.left() + px(1.0), "{left:?} {right:?}");
}

#[gpui::test]
fn split_down_stacks_the_groups(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-down", json!({ "index": 0 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "b.py", "c.py"], vec!["a.py"]]);
    let state = h.state();
    assert_eq!(state["splits"]["tree"]["split"]["vertical"], json!(true));
    let (above, below) = (
        bounds(&mut h, "editor-tabs-0"),
        bounds(&mut h, "editor-tabs"),
    );
    assert!(
        above.bottom() <= below.top() + px(1.0),
        "{above:?} {below:?}"
    );
}

#[gpui::test]
fn split_and_move_takes_the_tab_along_and_needs_another_tab(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    assert_eq!(focused(&mut h), 1);
    assert_eq!(active_path(&mut h), "b.py");
    // The first group shows its last tab now.
    h.invoke("tabs.next-splitter", json!(null));
    assert_eq!(active_path(&mut h), "c.py");
    // A group's only tab cannot be moved: nothing changes.
    h.invoke("tabs.next-splitter", json!(null));
    h.invoke("tabs.split-move-down", json!(null));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
}

#[gpui::test]
fn the_tab_menu_offers_the_split_rows_that_apply(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.context-menu", json!({ "index": 1 }));
    let rows = h.state()["tab_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap_or("---").to_owned())
        .collect::<Vec<_>>();
    let at = rows.iter().position(|l| l == "Split Right").unwrap();
    assert_eq!(
        rows[at - 1..at + 6],
        [
            "---",
            "Split Right",
            "Split and Move Right",
            "Split Down",
            "Split and Move Down",
            "---",
            "Pin Tab"
        ]
    );
    // Choosing Split Right splits the clicked tab's file.
    h.invoke("menu.cancel", Value::Null);
    h.invoke("tabs.context-menu", json!({ "index": 0 }));
    let rows = h.state()["tab_menu"]["menu"]["levels"][0]["items"].clone();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|r| r["label"] == "Split Right")
    );
    h.invoke("tabs.split-right", json!({ "index": 0 }));
    assert_eq!(active_path(&mut h), "a.py");
    // A group with one tab offers only the splits that keep the tab.
    h.invoke("tabs.context-menu", json!({ "index": 3 }));
    let rows = h.state()["tab_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap_or("---").to_owned())
        .collect::<Vec<_>>();
    assert!(rows.contains(&"Split Right".to_owned()), "{rows:?}");
    assert!(
        !rows.contains(&"Split and Move Right".to_owned()),
        "{rows:?}"
    );
}

#[gpui::test]
fn editing_one_view_changes_the_other_and_the_carets_stay_apart(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    // Typed at the start of the second view's text.
    h.press("cmd-home");
    h.press("x y");
    let typed = editor_text(&mut h);
    assert!(typed.starts_with("xyc = 1"), "{typed:?}");
    h.invoke("tabs.previous-splitter", json!(null));
    assert_eq!(focused(&mut h), 0);
    assert_eq!(editor_text(&mut h), typed, "the first view has the text");
    // Its own caret stayed where it was (the end of c.py's text).
    let caret = h.state()["editor"]["selections"][0]["head"].clone();
    assert_ne!(caret, json!(0), "{caret:?}");
    // Typing here reaches the second view.
    h.press("z");
    let both = editor_text(&mut h);
    h.invoke("tabs.next-splitter", json!(null));
    assert_eq!(editor_text(&mut h), both);
    // Switching groups saved the file (the tab switch setting): neither view
    // is modified.
    h.cx.run_until_parked();
    let state = h.state();
    assert_eq!(state["tabs"][2]["modified"], json!(false), "{state:?}");
    assert_eq!(state["tabs"][3]["modified"], json!(false));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("c.py")).unwrap(),
        both
    );
}

#[gpui::test]
fn closing_the_last_tab_of_a_group_removes_the_group(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    h.invoke("tabs.split-down", json!(null));
    assert_eq!(groups(&mut h).len(), 3);
    // 0 | (1 over 2): closing group 2's tab leaves 0 | 1.
    h.invoke("tabs.close", Value::Null);
    assert_eq!(groups(&mut h).len(), 2);
    assert_eq!(focused(&mut h), 1);
    assert_eq!(active_path(&mut h), "c.py");
    h.invoke("tabs.close", Value::Null);
    assert_eq!(groups(&mut h), [["a.py", "b.py", "c.py"]]);
    assert_eq!(focused(&mut h), 0);
    let state = h.state();
    assert!(state["splits"]["tree"].get("leaf").is_some());
    // The remaining view is the file's first view.
    assert!(state["tabs"][2].get("mirror").is_none());
}

#[gpui::test]
fn the_first_view_closes_and_the_second_takes_over(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    h.invoke("tabs.close", json!({ "index": 2 }));
    let state = h.state();
    assert_eq!(groups(&mut h), [vec!["a.py", "b.py"], vec!["c.py"]]);
    assert!(state["tabs"][2].get("mirror").is_none(), "{state:?}");
    // Editing still reaches the file.
    h.press("q");
    h.invoke("editor.save", Value::Null);
    h.cx.run_until_parked();
    let text = std::fs::read_to_string(dir.path().join("c.py")).unwrap();
    assert!(text.contains('q'), "{text:?}");
}

#[gpui::test]
fn close_and_next_tab_act_within_the_group(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 2 }));
    // a.py, b.py | c.py
    h.invoke("tabs.next-splitter", json!(null));
    assert_eq!(active_path(&mut h), "b.py");
    h.invoke("tabs.next", json!(null));
    assert_eq!(active_path(&mut h), "a.py");
    h.invoke("tabs.previous", json!(null));
    assert_eq!(active_path(&mut h), "b.py");
    h.invoke("tabs.close-others", json!(null));
    assert_eq!(groups(&mut h), [vec!["b.py"], vec!["c.py"]]);
    // Opening a file opens it in the focused group.
    h.invoke("project.open-file", json!({ "path": "a.py" }));
    assert_eq!(groups(&mut h), [vec!["b.py", "a.py"], vec!["c.py"]]);
    // A file open in another group is selected there.
    h.invoke("project.open-file", json!({ "path": "c.py" }));
    assert_eq!(focused(&mut h), 1);
    assert_eq!(active_path(&mut h), "c.py");
}

#[gpui::test]
fn a_click_in_a_group_gives_it_the_focus(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    assert_eq!(focused(&mut h), 1);
    let strip = bounds(&mut h, "editor-tabs-0");
    // A click on a tab of the other group.
    let tab = bounds(&mut h, "tab-1");
    click_at(&mut h, tab.center());
    assert_eq!(focused(&mut h), 0);
    assert_eq!(active_path(&mut h), "b.py");
    assert!(strip.size.width > px(100.0));
    // And in the other group's text.
    let right = bounds(&mut h, "editor-tabs-1");
    click_at(&mut h, point(right.center().x, right.bottom() + px(100.0)));
    assert_eq!(focused(&mut h), 1);
}

#[gpui::test]
fn dragging_the_splitter_moves_it_within_its_limits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    let area = bounds(&mut h, "editor-area");
    let grab = bounds(&mut h, "splitter-");
    let from = grab.center();
    let target = point(area.left() + area.size.width * 0.25, from.y);
    mouse::send(
        h.cx,
        h.window.into(),
        vec![
            mouse::press(from),
            mouse::drag_to(target),
            mouse::release(target),
        ],
    );
    let share = h.state()["splits"]["tree"]["split"]["proportion"]
        .as_f64()
        .unwrap();
    assert!((share - 0.25).abs() < 0.03, "{share}");
    // Past the limit it stops at 0.1.
    let from = bounds(&mut h, "splitter-").center();
    let far = point(area.left() - px(50.0), from.y);
    mouse::send(
        h.cx,
        h.window.into(),
        vec![mouse::press(from), mouse::drag_to(far), mouse::release(far)],
    );
    let share = h.state()["splits"]["tree"]["split"]["proportion"]
        .as_f64()
        .unwrap();
    assert!((share - 0.1).abs() < 1e-6, "{share}");
}

#[gpui::test]
fn splits_start_over_with_another_project(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    h.invoke(
        "project.open-folder",
        json!({ "path": other.path().display().to_string() }),
    );
    assert_eq!(groups(&mut h), [Vec::<String>::new()]);
    assert_eq!(focused(&mut h), 0);
}

#[gpui::test]
fn unsplit_merges_the_neighbours_tabs_after_its_own(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    // b.py's group takes the first group's tabs; it shows b.py.
    h.invoke("tabs.unsplit", json!(null));
    assert_eq!(groups(&mut h), [["b.py", "a.py", "c.py"]]);
    assert_eq!(active_path(&mut h), "b.py");
    assert!(h.state()["splits"]["tree"].get("leaf").is_some());
}

#[gpui::test]
fn unsplit_closes_the_second_view_of_a_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-right", json!(null));
    // The second view's group takes the first group's tabs but c.py,
    // which it shows; the first view of c.py closes and the second takes
    // over.
    h.invoke("tabs.unsplit", json!(null));
    assert_eq!(groups(&mut h), [["c.py", "a.py", "b.py"]]);
    let state = h.state();
    assert!(state["tabs"][0].get("mirror").is_none(), "{state:?}");
    assert_eq!(active_path(&mut h), "c.py");
    // The file still saves from the remaining view.
    h.press("w");
    h.invoke("editor.save", Value::Null);
    h.cx.run_until_parked();
    assert!(
        std::fs::read_to_string(dir.path().join("c.py"))
            .unwrap()
            .contains('w')
    );
}

#[gpui::test]
fn unsplit_all_leaves_one_group_and_the_orientation_turns(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    h.invoke("tabs.split-move-down", json!({ "index": 0 }));
    assert_eq!(groups(&mut h).len(), 3);
    // The innermost splitter is the focused group's: it turns.
    let tree = h.state()["splits"]["tree"].clone();
    assert_eq!(tree["split"]["vertical"], json!(false));
    h.invoke("tabs.change-splitter-orientation", json!(null));
    let turned = h.state()["splits"]["tree"].clone();
    assert_ne!(turned, tree);
    // Its rows are in the tab menu of a split editor area.
    h.invoke("tabs.context-menu", json!({ "index": 0 }));
    let rows = h.state()["tab_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap_or("---").to_owned())
        .collect::<Vec<_>>();
    for label in ["Change Splitter Orientation", "Unsplit", "Unsplit All"] {
        assert!(rows.contains(&label.to_owned()), "{label} in {rows:?}");
    }
    h.invoke("menu.cancel", Value::Null);
    h.invoke("tabs.unsplit-all", json!(null));
    assert_eq!(groups(&mut h).len(), 1);
    assert_eq!(groups(&mut h)[0].len(), 3);
    // Nothing to turn or merge any more.
    h.invoke("tabs.change-splitter-orientation", json!(null));
    h.invoke("tabs.unsplit", json!(null));
    assert_eq!(groups(&mut h).len(), 1);
}
