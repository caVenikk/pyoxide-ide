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

fn menu_labels(h: &mut Harness, tab: usize) -> Vec<String> {
    h.invoke("tabs.context-menu", json!({ "index": tab }));
    let rows = h.state()["tab_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap_or("---").to_owned())
        .collect::<Vec<_>>();
    h.invoke("menu.cancel", Value::Null);
    rows
}

#[gpui::test]
fn close_all_tabs_closes_one_group_and_is_named_so_in_a_split(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    let single = menu_labels(&mut h, 0);
    assert!(single.contains(&"Close All Tabs".to_owned()), "{single:?}");
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    let rows = menu_labels(&mut h, 0);
    assert!(
        rows.contains(&"Close All Tabs In Group".to_owned()),
        "{rows:?}"
    );
    assert!(!rows.contains(&"Close All Tabs".to_owned()), "{rows:?}");
    h.invoke("tabs.close-all", json!({ "index": 0 }));
    // The group of a.py is gone, b.py's stays.
    assert_eq!(groups(&mut h), [["b.py"]]);
}

#[gpui::test]
fn the_tab_menu_of_a_split_has_the_opposite_group_rows(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    let rows = menu_labels(&mut h, 0);
    let at = |label: &str| {
        rows.iter()
            .position(|r| r == label)
            .unwrap_or_else(|| panic!("{label} in {rows:?}"))
    };
    // Split Right, Split and Move Right, Split Down, Split and Move Down,
    // Move to Opposite Group, Open in Opposite Group, Change Splitter
    // Orientation, Unsplit.
    let first = at("Split Right");
    for (k, label) in [
        "Split Right",
        "Split and Move Right",
        "Split Down",
        "Split and Move Down",
        "Move to Opposite Group",
        "Open in Opposite Group",
        "Change Splitter Orientation",
        "Unsplit",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(at(label), first + k, "{label} in {rows:?}");
    }
    // Two groups: no Unsplit All.
    assert!(!rows.contains(&"Unsplit All".to_owned()), "{rows:?}");
}

#[gpui::test]
fn move_to_opposite_group_takes_the_tab_and_open_in_opposite_group_copies_it(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    // Open in Opposite Group: a.py shows in the right group too.
    h.invoke("tabs.open-in-opposite-group", json!({ "index": 0 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py", "a.py"]]);
    assert_eq!(active_path(&mut h), "a.py");
    // Again: the other group shows it already, nothing is added.
    h.invoke("tabs.open-in-opposite-group", json!({ "index": 0 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py", "a.py"]]);
    // Move to Opposite Group: c.py goes to the right.
    // (The tabs are a, b, c and the second view of a: c is the third.)
    h.invoke("tabs.move-to-opposite-group", json!({ "index": 2 }));
    assert_eq!(groups(&mut h), [vec!["a.py"], vec!["b.py", "a.py", "c.py"]]);
    assert_eq!(active_path(&mut h), "c.py");
    // Moving a.py: the right group has it, so the left tab only closes and
    // that group goes.
    h.invoke("tabs.move-to-opposite-group", json!({ "index": 0 }));
    assert_eq!(groups(&mut h), [["b.py", "a.py", "c.py"]]);
    assert_eq!(active_path(&mut h), "a.py");
}

#[gpui::test]
fn the_opposite_group_needs_exactly_one_group_beside(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    // Not split: nothing to do.
    h.invoke("tabs.move-to-opposite-group", json!({ "index": 0 }));
    assert_eq!(groups(&mut h), [["a.py", "b.py", "c.py"]]);
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    h.invoke("tabs.split-move-down", json!({ "index": 0 }));
    // Three groups: b.py's group has two beside it, the rows are not
    // offered and the commands do nothing.
    let before = groups(&mut h);
    let rows = menu_labels(&mut h, 1);
    assert!(
        !rows.contains(&"Move to Opposite Group".to_owned()),
        "{rows:?}"
    );
    h.invoke("tabs.move-to-opposite-group", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), before);
}

#[gpui::test]
fn unsplit_all_shows_each_file_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    // c.py in three groups.
    h.invoke("tabs.split-right", json!(null));
    h.invoke("tabs.split-right", json!(null));
    assert_eq!(
        groups(&mut h),
        [vec!["a.py", "b.py", "c.py"], vec!["c.py"], vec!["c.py"]]
    );
    h.invoke("tabs.unsplit-all", json!(null));
    let mut files = groups(&mut h);
    assert_eq!(files.len(), 1, "{files:?}");
    let mut tabs = files.remove(0);
    tabs.sort();
    assert_eq!(tabs, ["a.py", "b.py", "c.py"]);
    assert_eq!(active_path(&mut h), "c.py");
    // Different files in each group, one of them in two.
    let mut h = open(h.cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    h.invoke("tabs.split-right", json!({ "index": 0 }));
    h.invoke("tabs.unsplit-all", json!(null));
    let state = h.state();
    let mut paths: Vec<String> = state["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap().to_owned())
        .collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), state["tabs"].as_array().unwrap().len());
}

#[gpui::test]
fn opening_a_file_that_another_group_shows_focuses_that_group(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    assert_eq!(focused(&mut h), 1);
    // a.py is open in the left group: its tab is selected there, no second
    // copy opens.
    h.invoke("project.open-file", json!({ "path": "a.py" }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    assert_eq!(focused(&mut h), 0);
    assert_eq!(active_path(&mut h), "a.py");
}

#[gpui::test]
fn open_in_right_split_splits_once_and_then_reuses_the_right_group(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("d.py"), "d = 1\n").unwrap();
    let mut h = open(cx, dir.path());
    // An open file: a second view in a new group at the right.
    h.invoke("project.open-in-right-split", json!({ "path": "a.py" }));
    assert_eq!(groups(&mut h), [vec!["a.py", "b.py", "c.py"], vec!["a.py"]]);
    assert_eq!(focused(&mut h), 1);
    // From the left group again: the right group is reused.
    h.invoke("tabs.next-splitter", json!(null));
    assert_eq!(focused(&mut h), 0);
    h.invoke("project.open-in-right-split", json!({ "path": "d.py" }));
    h.cx.run_until_parked();
    h.draw();
    assert_eq!(
        groups(&mut h),
        [vec!["a.py", "b.py", "c.py"], vec!["a.py", "d.py"]]
    );
    assert_eq!(focused(&mut h), 1);
    assert_eq!(active_path(&mut h), "d.py");
    // A file the right group shows already: its tab is selected.
    h.invoke("tabs.next-splitter", json!(null));
    h.invoke("project.open-in-right-split", json!({ "path": "a.py" }));
    assert_eq!(
        groups(&mut h),
        [vec!["a.py", "b.py", "c.py"], vec!["a.py", "d.py"]]
    );
    assert_eq!(active_path(&mut h), "a.py");
    assert_eq!(focused(&mut h), 1);
}

// ---- dragging tabs ---------------------------------------------------------------

/// Presses on the tab drawn at `selector` and moves to `to`: the drag is on
/// while the button stays down.
fn start_drag(h: &mut Harness, tab: &str, to: gpui::Point<Pixels>) {
    let from = bounds(h, tab).center();
    mouse::send(
        h.cx,
        h.window.into(),
        vec![
            mouse::press(from),
            // Past the drag threshold, then to the target.
            mouse::drag_to(point(from.x + px(8.0), from.y + px(1.0))),
            mouse::drag_to(to),
        ],
    );
}

fn drop_at(h: &mut Harness, at: gpui::Point<Pixels>, modifiers: gpui::Modifiers) {
    mouse::send(
        h.cx,
        h.window.into(),
        vec![mouse::drag_to(at), mouse::release_with(at, modifiers)],
    );
}

/// The editor area of the group whose strip is `strip`: from the strip's
/// top to the bottom of the editor area.
fn group_area(h: &mut Harness, strip: &str) -> Bounds<Pixels> {
    let strip = bounds(h, strip);
    let area = bounds(h, "editor-area");
    Bounds::from_corners(strip.origin, point(strip.right(), area.bottom()))
}

/// Where the dragged tab would go (`splits.drag.drop.place`), `null` when
/// nothing is dragged.
fn drag_place(h: &mut Harness) -> Value {
    let drag = h.state()["splits"]["drag"].clone();
    if drag.is_null() {
        return Value::Null;
    }
    drag["drop"]["place"].clone()
}

fn drawn(h: &mut Harness, part: &str) -> bool {
    let name: &'static str = Box::leak(part.to_owned().into_boxed_str());
    VisualTestContext::from_window(h.window.into(), h.cx)
        .debug_bounds(name)
        .is_some()
}

#[gpui::test]
fn dragging_a_tab_along_its_strip_changes_its_place(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    let last = bounds(&mut h, "tab-2");
    let strip = bounds(&mut h, "editor-tabs");
    // While dragged, the tab leaves the strip and a slot opens where it
    // would go: after the last tab.
    let over = point(last.right() - px(4.0), strip.center().y);
    start_drag(&mut h, "tab-0", over);
    assert!(drawn(&mut h, "tab-drop-slot"));
    assert!(drawn(&mut h, "tab-drag-image"));
    assert_eq!(drag_place(&mut h), json!("strip 2"));
    drop_at(&mut h, over, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [["b.py", "c.py", "a.py"]]);
    assert_eq!(active_path(&mut h), "a.py");
    assert_eq!(drag_place(&mut h), Value::Null);
    // Back to the start: before the first tab.
    let first = bounds(&mut h, "tab-0");
    let to = point(first.left() + px(4.0), strip.center().y);
    start_drag(&mut h, "tab-2", to);
    drop_at(&mut h, to, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [["a.py", "b.py", "c.py"]]);
}

#[gpui::test]
fn dropping_a_tab_in_the_middle_of_its_own_editor_changes_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    let area = group_area(&mut h, "editor-tabs");
    start_drag(&mut h, "tab-0", area.center());
    // The whole editor is the drop area.
    let shown = bounds(&mut h, "tab-drop-area");
    assert!((shown.size.width - area.size.width).abs() < px(2.0));
    drop_at(&mut h, area.center(), gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [["a.py", "b.py", "c.py"]]);
}

#[gpui::test]
fn a_tab_dropped_in_another_groups_strip_moves_there(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    // c.py (tab 2) over the left half of b.py's tab (tab 1) in the second
    // group.
    let target = bounds(&mut h, "tab-1");
    let strip = bounds(&mut h, "editor-tabs");
    let over = point(target.left() + px(4.0), strip.center().y);
    start_drag(&mut h, "tab-2", over);
    // The slot opens before b.py's tab.
    let slot = bounds(&mut h, "tab-drop-slot");
    assert!(
        slot.left() < target.left() + px(40.0),
        "{slot:?} {target:?}"
    );
    drop_at(&mut h, over, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["a.py"], vec!["c.py", "b.py"]]);
    assert_eq!(focused(&mut h), 1);
    assert_eq!(active_path(&mut h), "c.py");
}

#[gpui::test]
fn a_tab_dropped_on_another_editor_goes_to_its_end(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    // a.py (tab 0) onto the middle of the second group's editor.
    let area = group_area(&mut h, "editor-tabs");
    start_drag(&mut h, "tab-0", area.center());
    let shown = bounds(&mut h, "tab-drop-area");
    assert!((shown.size.width - area.size.width).abs() < px(2.0));
    assert!((shown.size.height - (area.size.height - px(30.0))).abs() < px(20.0));
    drop_at(&mut h, area.center(), gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["c.py"], vec!["b.py", "a.py"]]);
    assert_eq!(active_path(&mut h), "a.py");
}

#[gpui::test]
fn a_tab_dropped_on_an_edge_makes_a_split_on_that_side(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    let area = group_area(&mut h, "editor-tabs");
    // The right edge: the right half is the drop area.
    let right = point(area.right() - px(10.0), area.center().y);
    start_drag(&mut h, "tab-0", right);
    let shown = bounds(&mut h, "tab-drop-area");
    assert!(
        (shown.size.width - area.size.width / 2.0).abs() < px(2.0),
        "{shown:?}"
    );
    assert!(
        (shown.left() - area.center().x).abs() < px(2.0),
        "{shown:?}"
    );
    drop_at(&mut h, right, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["b.py", "c.py"], vec!["a.py"]]);
    assert_eq!(focused(&mut h), 1);
    assert_eq!(
        h.state()["splits"]["tree"]["split"]["vertical"],
        json!(false)
    );
    // The left edge of the first group: the new group comes first.
    let area = group_area(&mut h, "editor-tabs-0");
    let left = point(area.left() + px(10.0), area.center().y);
    start_drag(&mut h, "tab-1", left);
    let shown = bounds(&mut h, "tab-drop-area");
    assert!((shown.left() - area.left()).abs() < px(2.0), "{shown:?}");
    drop_at(&mut h, left, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["b.py"], vec!["c.py"], vec!["a.py"]]);
}

#[gpui::test]
fn a_tab_dropped_on_the_bottom_edge_splits_downwards(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    let area = group_area(&mut h, "editor-tabs");
    let bottom = point(area.center().x, area.bottom() - px(10.0));
    start_drag(&mut h, "tab-1", bottom);
    let shown = bounds(&mut h, "tab-drop-area");
    assert!(
        (shown.bottom() - area.bottom()).abs() < px(2.0),
        "{shown:?}"
    );
    assert!(shown.size.width > area.size.width - px(2.0));
    drop_at(&mut h, bottom, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    assert_eq!(
        h.state()["splits"]["tree"]["split"]["vertical"],
        json!(true)
    );
}

#[gpui::test]
fn the_last_tab_dragged_out_of_a_group_removes_the_group(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    // b.py, the only tab of the second group, onto the first group.
    let area = group_area(&mut h, "editor-tabs-0");
    start_drag(&mut h, "tab-1", area.center());
    drop_at(&mut h, area.center(), gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [["a.py", "c.py", "b.py"]]);
    assert!(h.state()["splits"]["tree"].get("leaf").is_some());
    assert_eq!(active_path(&mut h), "b.py");
}

#[gpui::test]
fn control_copies_the_file_to_the_other_group(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    let area = group_area(&mut h, "editor-tabs");
    // a.py onto the second group's editor with Control held.
    start_drag(&mut h, "tab-0", area.center());
    let control = gpui::Modifiers::control();
    drop_at(&mut h, area.center(), control);
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py", "a.py"]]);
    let state = h.state();
    let mirrors = state["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["mirror"] == json!(true))
        .count();
    assert_eq!(mirrors, 1);
    // The copy dragged back to the first group, which shows a.py: that
    // tab is selected, nothing is opened twice.
    let area = group_area(&mut h, "editor-tabs-0");
    start_drag(&mut h, "tab-2", area.center());
    drop_at(&mut h, area.center(), gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
    assert_eq!(active_path(&mut h), "a.py");
}

#[gpui::test]
fn a_drag_released_outside_the_editor_changes_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    start_drag(&mut h, "tab-1", point(px(2.0), px(2.0)));
    // Dragged, but over no place.
    assert!(h.state()["splits"]["drag"].is_object());
    assert_eq!(drag_place(&mut h), Value::Null);
    drop_at(&mut h, point(px(2.0), px(2.0)), gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [["a.py", "b.py", "c.py"]]);
    // The drag is over and the tab is back in its strip.
    assert!(h.state()["splits"]["drag"].is_null());
    bounds(&mut h, "tab-1");
}

#[gpui::test]
fn a_group_with_one_tab_dropped_on_its_own_edge_changes_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.split-move-right", json!({ "index": 1 }));
    let area = group_area(&mut h, "editor-tabs");
    let edge = point(area.right() - px(10.0), area.center().y);
    start_drag(&mut h, "tab-1", edge);
    drop_at(&mut h, edge, gpui::Modifiers::none());
    assert_eq!(groups(&mut h), [vec!["a.py", "c.py"], vec!["b.py"]]);
}
