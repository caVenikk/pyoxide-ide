//! The context menu of an editor tab (the reference's `EditorTabPopupMenu`):
//! its rows in the reference's order with the shortcuts of the macOS or the
//! default keymap, a right click that opens it for the clicked tab and
//! leaves the active tab, and the Close group and Pin Tab chosen with the
//! mouse or a mnemonic key.

use std::path::Path;
use std::time::{Duration, Instant};

use gpui::{
    Bounds, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, VisualTestContext,
};
use gpui::{PlatformInput, TestAppContext};
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;
use workbench::{FileType, Product};

use crate::{mouse, support};

const MACOS: bool = cfg!(target_os = "macos");

fn product() -> Product {
    Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        run: Some(support::run_hook()),
        debug: Some(support::debug_hook()),
        ..Product::default()
    }
}

/// `a.py` to `d.py` (made when missing) open in tabs, `d.py` active.
fn open<'a>(cx: &'a mut TestAppContext, root: &Path) -> Harness<'a> {
    for name in ["a", "b", "c", "d"] {
        let file = root.join(format!("{name}.py"));
        if !file.exists() {
            std::fs::write(file, format!("{name} = 1\n")).unwrap();
        }
    }
    let mut h = Harness::new(cx, product(), MACOS, Some(root));
    for name in ["a", "b", "c", "d"] {
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

/// A right click in the middle of tab `index`.
fn right_click(h: &mut Harness, index: usize) {
    let at = bounds(h, &format!("tab-{index}")).center();
    let events = vec![
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Right,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        }),
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Right,
            position: at,
            modifiers: Modifiers::none(),
            click_count: 1,
        }),
    ];
    mouse::send(h.cx, h.window.into(), events);
    h.draw();
}

/// A left click in the middle of a drawn part.
fn click(h: &mut Harness, part: &str) {
    let at = bounds(h, part).center();
    mouse::send(
        h.cx,
        h.window.into(),
        vec![mouse::press(at), mouse::release(at)],
    );
    h.draw();
}

/// The rows of the open tab menu's first level.
fn menu_rows(h: &mut Harness) -> Vec<Value> {
    h.state()["tab_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The labels of the rows, "---" for a separator.
fn labels(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| match r["label"].as_str() {
            Some(l) => l.to_owned(),
            None => "---".to_owned(),
        })
        .collect()
}

fn row<'a>(rows: &'a [Value], label: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["label"] == label)
        .unwrap_or_else(|| panic!("no row {label}: {rows:?}"))
}

/// The open tabs' file names.
fn tabs(h: &mut Harness) -> Vec<String> {
    h.state()["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[gpui::test]
fn a_right_click_opens_the_reference_menu_for_the_clicked_tab(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    right_click(&mut h, 1);
    let state = h.state();
    // The clicked tab is the menu's; the active tab stays.
    assert_eq!(state["active_tab"], json!(3));
    assert_eq!(state["tab_menu"]["tab"], json!(1));
    let rows = menu_rows(&mut h);
    assert_eq!(
        labels(&rows),
        [
            "Close",
            "Close Other Tabs",
            "Close All Tabs",
            "Close Tabs to the Left",
            "Close Tabs to the Right",
            "---",
            "Copy Path/Reference\u{2026}",
            "---",
            "Pin Tab",
            "---",
            "Run 'b'",
            "Debug 'b'",
            "---",
            "Open In",
            "---",
            "Rename File\u{2026}",
        ]
    );
    let shortcut = |label: &str| row(&rows, label)["shortcut"].clone();
    let mnemonic = |label: &str| row(&rows, label)["mnemonic"].clone();
    if MACOS {
        assert_eq!(shortcut("Close"), json!("\u{2318}W"));
        assert_eq!(shortcut("Run 'b'"), json!("\u{2303}\u{21e7}R"));
        assert_eq!(shortcut("Debug 'b'"), json!("\u{2303}\u{21e7}D"));
    } else {
        assert_eq!(shortcut("Close"), json!("Ctrl+F4"));
        assert_eq!(shortcut("Run 'b'"), json!("Ctrl+Shift+F10"));
    }
    assert_eq!(mnemonic("Close"), json!("c"));
    assert_eq!(mnemonic("Close Other Tabs"), json!("o"));
    assert_eq!(mnemonic("Close All Tabs"), json!("a"));
    assert_eq!(mnemonic("Pin Tab"), json!("i"));
    assert_eq!(mnemonic("Run 'b'"), json!("u"));
    assert_eq!(mnemonic("Debug 'b'"), json!("d"));
    assert_eq!(
        row(&rows, "Copy Path/Reference\u{2026}")["submenu"],
        json!(true)
    );
    assert_eq!(row(&rows, "Open In")["submenu"], json!(true));

    // The first tab has nothing to its left; a single tab has no others.
    h.invoke("menu.cancel", Value::Null);
    right_click(&mut h, 0);
    let first = labels(&menu_rows(&mut h));
    assert!(
        !first.iter().any(|l| l == "Close Tabs to the Left"),
        "{first:?}"
    );
    h.invoke("menu.cancel", Value::Null);
    for _ in 0..3 {
        h.invoke("tabs.close", json!({ "index": 1 }));
    }
    right_click(&mut h, 0);
    let rows = menu_rows(&mut h);
    assert_eq!(row(&rows, "Close Other Tabs")["enabled"], json!(false));
}

#[gpui::test]
fn close_other_tabs_by_its_mnemonic_keeps_the_clicked_and_pinned_tabs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("tabs.toggle-pin", json!({ "index": 0 }));
    right_click(&mut h, 2);
    h.press("o");
    assert!(h.state()["tab_menu"].is_null(), "the menu closes");
    assert_eq!(tabs(&mut h), ["a.py", "c.py"]);
}

#[gpui::test]
fn close_tabs_to_the_right_with_the_mouse(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    right_click(&mut h, 1);
    click(&mut h, "menu-item:Close Tabs to the Right");
    assert_eq!(tabs(&mut h), ["a.py", "b.py"]);
    // The active tab was closed: its left neighbour is active.
    assert_eq!(h.state()["active_tab"], json!(1));
}

#[gpui::test]
fn a_pinned_tab_moves_left_and_stays_on_close_all_but_pinned(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    right_click(&mut h, 2);
    click(&mut h, "menu-item:Pin Tab");
    // `groupPinnedFirst`: the pinned c.py trades places with a.py; d.py
    // stays active.
    assert_eq!(tabs(&mut h), ["c.py", "b.py", "a.py", "d.py"]);
    let state = h.state();
    assert_eq!(state["tabs"][0]["pinned"], json!(true));
    assert_eq!(state["active_tab"], json!(3));
    // The pinned tab's menu offers Unpin Tab and Close All but Pinned.
    right_click(&mut h, 0);
    let rows = menu_rows(&mut h);
    assert_eq!(row(&rows, "Unpin Tab")["mnemonic"], json!("i"));
    click(&mut h, "menu-item:Close All but Pinned");
    assert_eq!(tabs(&mut h), ["c.py"]);
    // The pin on the tab unpins it.
    h.invoke("project.open-file", json!({ "path": "a.py" }));
    click(&mut h, "tab-pin-0");
    assert_eq!(h.state()["tabs"][0]["pinned"], json!(false));
    // Close All Tabs closes pinned tabs too (`CloseAllEditorsAction`).
    h.invoke("tabs.toggle-pin", json!({ "index": 0 }));
    right_click(&mut h, 1);
    click(&mut h, "menu-item:Close All Tabs");
    assert!(tabs(&mut h).is_empty());
}

fn git(dir: &Path, args: &[&str]) {
    let exe = git::GitExe::find(None).expect("git is installed");
    let out = std::process::Command::new(exe.path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// With version control, Close Unmodified Tabs closes the tabs of files
/// without changes (unversioned ones too) and keeps changed and pinned
/// ones; Git offers the file's actions.
#[gpui::test]
fn close_unmodified_tabs_keeps_changed_files(cx: &mut TestAppContext) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    git::testing::init(&root);
    for name in ["a", "b", "c", "d"] {
        std::fs::write(root.join(format!("{name}.py")), format!("{name} = 1\n")).unwrap();
    }
    git(&root, &["add", "a.py", "b.py", "c.py"]);
    git(&root, &["commit", "-q", "-m", "first"]);
    // Another size: a change the status sees without reading the file.
    std::fs::write(root.join("b.py"), "b = 1\nb += 1\n").unwrap();
    let mut h = open(cx, &root);
    h.invoke("tabs.toggle-pin", json!({ "index": 2 }));
    let start = Instant::now();
    while h.state()["vcs"]["status"]["changes"]
        .as_array()
        .is_none_or(|c| c.is_empty())
    {
        assert!(start.elapsed() < Duration::from_secs(30), "no status");
        support::tick(h.cx);
    }
    right_click(&mut h, 0);
    let rows = menu_rows(&mut h);
    assert_eq!(row(&rows, "Close Unmodified Tabs")["mnemonic"], json!("u"));
    assert_eq!(row(&rows, "Git")["submenu"], json!(true));
    h.press("u");
    // a.py is clean and d.py unversioned: closed; b.py is modified and the
    // pinned c.py (now first) stays.
    assert_eq!(tabs(&mut h), ["c.py", "b.py"]);
}
