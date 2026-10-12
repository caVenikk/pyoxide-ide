//! Excluded folders (ADR 0008, tests/scenarios/editor/excluded-folders.md)
//! driven with real mouse and key events: a right click on a row opens its
//! context menu, *Mark Directory As > Excluded* marks the folder, *Cancel
//! Exclusion* unmarks it, and the project settings follow.

use std::path::Path;
use std::rc::Rc;

use gpui::{
    AppContext as _, Modifiers, MouseButton, Point, TestAppContext, VisualTestContext, WindowHandle,
};
use serde_json::{Value, json};
use workbench::{AutoExclusions, FileType, Product, Workbench};

fn open(cx: &mut TestAppContext, root: &Path) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        auto_excluded: AutoExclusions {
            dir_names: &[".venv"],
            root_dir_markers: &["pyvenv.cfg"],
        },
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    // Focus events reach listeners only in the active window.
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
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
    draw(cx, window);
    out
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn tree(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    let s = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(s["busy"], json!(false), "background work did not finish");
    s["tree"].clone()
}

fn row(tree: &Value, path: &str) -> Value {
    tree["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == json!(path))
        .unwrap_or_else(|| panic!("no row {path}"))
        .clone()
}

/// Labels of the rows of each open menu level.
fn menu(tree: &Value) -> Vec<Vec<String>> {
    tree["context_menu"]["levels"]
        .as_array()
        .map(|levels| {
            levels
                .iter()
                .map(|l| {
                    l["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| i["label"].as_str().unwrap_or("-").to_owned())
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn centre(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Point<gpui::Pixels> {
    draw(cx, window);
    // Debug selectors are looked up by a static name.
    let name: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    VisualTestContext::from_window(window.into(), cx)
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn right_click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    vcx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    draw(cx, window);
}

fn hover(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    VisualTestContext::from_window(window.into(), cx).simulate_mouse_move(
        at,
        None,
        Modifiers::none(),
    );
    draw(cx, window);
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_click(at, Modifiers::none());
    draw(cx, window);
}

fn press(cx: &mut TestAppContext, window: WindowHandle<Workbench>, keys: &str) {
    cx.simulate_keystrokes(window.into(), keys);
    draw(cx, window);
}

fn layout(root: &Path) {
    for (path, text) in [
        ("docs/index.md", "# Docs\n"),
        ("docs/api/ref.md", "ref\n"),
        ("src/app.py", "x = 1\n"),
        (".venv/pyvenv.cfg", "home = /usr\n"),
        ("notes.txt", "n\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn settings(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join(".oxide/project.toml")).ok()
}

#[gpui::test]
fn mark_directory_as_excluded_and_cancel_with_mouse_and_keys(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    layout(&root);
    let window = open(cx, &root);
    let t = tree(cx, window);
    assert_eq!(row(&t, "docs")["excluded"], json!(false));
    assert_eq!(row(&t, "docs")["icon"], json!("nodes/folder"));
    assert_eq!(row(&t, "docs")["background"], json!(null));
    assert!(settings(&root).is_none(), "opening creates no settings");

    // A right click selects the row and opens its context menu.
    right_click(cx, window, "tree-row:docs");
    let t = tree(cx, window);
    assert_eq!(t["selected"], json!("docs"));
    assert_eq!(
        menu(&t),
        [[
            "Find in Files…",
            "Replace in Files…",
            "-",
            "Mark Directory As"
        ]]
    );

    // Hovering the submenu row opens it; it offers Excluded.
    hover(cx, window, "menu-item:Mark Directory As");
    let t = tree(cx, window);
    assert_eq!(menu(&t)[1], ["Excluded"]);
    assert_eq!(
        t["context_menu"]["levels"][1]["items"][0]["command"],
        json!("project.tree.mark-excluded")
    );

    click(cx, window, "menu-item:Excluded");
    let t = tree(cx, window);
    assert_eq!(t["context_menu"], json!(null), "the menu closed");
    let docs = row(&t, "docs");
    assert_eq!(docs["excluded"], json!(true));
    assert_eq!(docs["icon"], json!("nodes/excludeRoot"));
    assert_eq!(docs["background"], json!("non-project-file"));
    assert_eq!(docs["expanded"], json!(false), "not expanded on its own");
    assert_eq!(t["exclusions"], json!(["docs"]));
    let text = settings(&root).expect("the settings were written");
    assert!(text.contains("paths = [\n    \"docs\",\n]"), "{text}");
    // The tree has the focus again.
    let panel = window.read_with(cx, |w, _| w.panel().clone()).unwrap();
    let focused = cx
        .update_window(window.into(), |_, window, cx| {
            gpui::Focusable::focus_handle(panel.read(cx), cx).contains_focused(window, cx)
        })
        .unwrap();
    assert!(focused, "the tree has the focus back");

    // Expand All from the root leaves it closed; it expands by hand, and
    // what is inside is excluded too (subfolders with the excluded icon).
    invoke(cx, window, "project.tree.collapse-all", json!(null));
    invoke(cx, window, "project.tree.expand-all", json!(null));
    assert_eq!(row(&tree(cx, window), "docs")["expanded"], json!(false));
    invoke(cx, window, "project.tree.reveal", json!({"path": "docs"}));
    invoke(cx, window, "project.tree.expand", json!(null));
    let t = tree(cx, window);
    assert_eq!(row(&t, "docs/api")["excluded"], json!(true));
    assert_eq!(row(&t, "docs/api")["icon"], json!("nodes/excludeRoot"));
    assert_eq!(
        row(&t, "docs/index.md")["background"],
        json!("non-project-file")
    );
    // Rows inside an excluded folder have nothing to mark.
    right_click(cx, window, "tree-row:docs/api");
    assert_eq!(
        menu(&tree(cx, window)),
        [vec!["Find in Files…", "Replace in Files…"]]
    );
    press(cx, window, "escape");
    invoke(cx, window, "project.tree.collapse-all", json!(null));

    // The menu again, now with the keyboard: Up selects its last row,
    // Right opens the submenu and selects its first row, which is Cancel
    // Exclusion; Enter runs it.
    right_click(cx, window, "tree-row:docs");
    press(cx, window, "up");
    assert_eq!(
        tree(cx, window)["context_menu"]["levels"][0]["selected"],
        json!(3)
    );
    press(cx, window, "right");
    let t = tree(cx, window);
    assert_eq!(menu(&t)[1], ["Cancel Exclusion"]);
    assert_eq!(t["context_menu"]["levels"][1]["selected"], json!(0));
    // Left closes the submenu, Escape the menu.
    press(cx, window, "left");
    assert_eq!(menu(&tree(cx, window)).len(), 1);
    press(cx, window, "escape");
    assert_eq!(tree(cx, window)["context_menu"], json!(null));
    assert_eq!(row(&tree(cx, window), "docs")["excluded"], json!(true));

    right_click(cx, window, "tree-row:docs");
    press(cx, window, "up right enter");
    let t = tree(cx, window);
    assert_eq!(t["context_menu"], json!(null));
    assert_eq!(row(&t, "docs")["excluded"], json!(false));
    assert_eq!(row(&t, "docs")["icon"], json!("nodes/folder"));
    assert_eq!(row(&t, "docs")["background"], json!(null));
    let text = settings(&root).unwrap();
    assert!(text.contains("paths = []"), "{text}");

    // A click outside the menu closes it without doing anything.
    right_click(cx, window, "tree-row:notes.txt");
    assert_eq!(
        menu(&tree(cx, window)),
        [["Find in Files…", "Replace in Files…", "-", "Mark File As"]]
    );
    click(cx, window, "tree-row:src");
    let t = tree(cx, window);
    assert_eq!(t["context_menu"], json!(null));
    assert_eq!(row(&t, "notes.txt")["excluded"], json!(false));

    // A folder a product rule excludes offers no Cancel Exclusion, and the
    // project root has nothing to mark.
    right_click(cx, window, "tree-row:.venv");
    assert_eq!(
        menu(&tree(cx, window)),
        [vec!["Find in Files…", "Replace in Files…"]]
    );
    press(cx, window, "escape");
    right_click(cx, window, "tree-row:");
    assert_eq!(
        menu(&tree(cx, window)),
        [vec!["Find in Files…", "Replace in Files…"]]
    );
    press(cx, window, "escape");
}

#[gpui::test]
fn marks_persist_and_files_can_be_excluded_by_command(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    layout(&root);
    let window = open(cx, &root);
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": "notes.txt"}),
    );
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": "docs"}),
    );
    // Marking what is excluded already, the root, or a path outside the
    // project changes nothing.
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": "docs/api"}),
    );
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": ".venv"}),
    );
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": root.display().to_string()}),
    );
    invoke(
        cx,
        window,
        "project.tree.mark-excluded",
        json!({"path": "/elsewhere"}),
    );
    // Cancelling an automatic exclusion does nothing either.
    invoke(
        cx,
        window,
        "project.tree.cancel-exclusion",
        json!({"path": ".venv"}),
    );
    let t = tree(cx, window);
    assert_eq!(t["exclusions"], json!(["docs", "notes.txt"]));
    assert_eq!(row(&t, "notes.txt")["excluded"], json!(true));
    assert_eq!(
        row(&t, "notes.txt")["background"],
        json!("non-project-file")
    );
    assert_eq!(row(&t, ".venv")["excluded"], json!(true));
    // The file's menu says Mark File As.
    right_click(cx, window, "tree-row:notes.txt");
    hover(cx, window, "menu-item:Mark File As");
    assert_eq!(menu(&tree(cx, window))[1], ["Cancel Exclusion"]);
    press(cx, window, "escape escape");

    // Reopening the project keeps the marks.
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    let t = tree(cx, window);
    assert_eq!(t["exclusions"], json!(["docs", "notes.txt"]));
    assert_eq!(row(&t, "docs")["excluded"], json!(true));

    // The panel's rules follow the marks at once (Find in Files reads them).
    invoke(
        cx,
        window,
        "project.tree.cancel-exclusion",
        json!({"path": "docs"}),
    );
    let panel = window.read_with(cx, |w, _| w.panel().clone()).unwrap();
    let excluded = panel.read_with(cx, |p, _| {
        p.filter()
            .unwrap()
            .is_excluded(&root.join("docs/index.md"), false)
    });
    assert!(!excluded);
    assert!(settings(&root).unwrap().contains("\"notes.txt\""));
}

#[gpui::test]
fn context_menu_from_the_keyboard(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    layout(&root);
    let window = open(cx, &root);
    invoke(cx, window, "project.tree.reveal", json!({"path": "src"}));
    invoke(cx, window, "project.tree.context-menu", json!(null));
    let t = tree(cx, window);
    assert_eq!(
        menu(&t),
        [[
            "Find in Files…",
            "Replace in Files…",
            "-",
            "Mark Directory As"
        ]]
    );
    // It opens below the selected row.
    let row_bounds = {
        draw(cx, window);
        VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("tree-row:src")
            .unwrap()
    };
    let menu_bounds = VisualTestContext::from_window(window.into(), cx)
        .debug_bounds("menu-level-0")
        .unwrap();
    assert!(
        (menu_bounds.top() - row_bounds.bottom()).abs() < gpui::px(1.0),
        "menu at {menu_bounds:?}, row at {row_bounds:?}"
    );
    press(cx, window, "up enter");
    // Enter on a submenu row opens it and selects its first row.
    let t = tree(cx, window);
    assert_eq!(menu(&t)[1], ["Excluded"]);
    press(cx, window, "enter");
    assert_eq!(row(&tree(cx, window), "src")["excluded"], json!(true));
}
