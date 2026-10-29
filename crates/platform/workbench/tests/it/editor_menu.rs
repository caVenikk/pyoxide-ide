//! The context menu of the text editor: a right click on the text opens it
//! with the rows of the reference's editor popup the product has, the
//! command opens it below the caret, and a file type adds its own rows.

use std::sync::Arc;

use gpui::{Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
use gpui::{TestAppContext, point, px};
use serde_json::{Value, json};
use ui::menu::MenuItem;
use workbench::test_support::keymap::Harness;
use workbench::{FileType, Product};

use crate::mouse;

const MACOS: bool = cfg!(target_os = "macos");

fn product() -> Product {
    let mut code = FileType::plain("Code", &["code"], ui::icons::IconName::FILE_TEXT);
    code.language_id = Some("code");
    code.context_menu = Some(Arc::new(|path, _| {
        // Only the files this type names itself.
        if path.file_name().is_some_and(|n| n == "plain.code") {
            return Vec::new();
        }
        vec![
            MenuItem::submenu(
                "Extra",
                vec![MenuItem::action("Inner", editor::commands::SelectAll)],
            ),
            MenuItem::action("Select Everything", editor::commands::SelectAll),
        ]
    }));
    Product {
        name: "Test IDE",
        file_types: vec![code],
        ..Product::default()
    }
}

fn open<'a>(cx: &'a mut TestAppContext, root: &std::path::Path) -> Harness<'a> {
    std::fs::write(root.join("a.code"), "alpha beta\nsecond line\n").unwrap();
    std::fs::write(root.join("plain.code"), "one\n").unwrap();
    std::fs::write(root.join("b.txt"), "text\n").unwrap();
    let mut h = Harness::new(cx, product(), MACOS, Some(root));
    h.invoke("project.open-file", json!({ "path": "a.code" }));
    h
}

fn editor(h: &mut Harness) -> Entity<editor::Editor> {
    h.window
        .read_with(h.cx, |w, _| w.active_editor().cloned())
        .unwrap()
        .expect("an editor")
}

/// A right click on the text of line `line`, column `col`.
fn right_click(h: &mut Harness, line: usize, col: usize) {
    let editor = editor(h);
    let (origin, lh, cw) = editor.read_with(h.cx, |e, _| e.text_geometry().unwrap());
    let at = point(
        origin.x + cw * col as f32 + px(1.0),
        origin.y + lh * line as f32 + lh / 2.0,
    );
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

fn rows(h: &mut Harness) -> Vec<Value> {
    h.state()["editor_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn labels(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| r["label"].as_str().unwrap_or("---").to_owned())
        .collect()
}

#[gpui::test]
fn a_right_click_opens_the_menu_and_moves_the_caret(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    assert!(h.state()["editor_menu"].is_null());
    right_click(&mut h, 1, 3);
    // The caret went to the click; without a selection there is no Cut or
    // Copy.
    let caret = editor(&mut h).read_with(h.cx, |e, _| e.selections().primary().head);
    assert_eq!(caret, "alpha beta\nsec".len());
    let rows = rows(&mut h);
    assert_eq!(
        labels(&rows),
        [
            "Show Context Actions",
            "---",
            "Paste",
            "---",
            "Find Usages",
            "Go To",
            "---",
            "Duplicate",
            "Reformat Code",
            "---",
            "Extra",
            "Select Everything",
            "---",
            "Open In",
        ]
    );
    let shortcut =
        |label: &str| rows.iter().find(|r| r["label"] == label).unwrap()["shortcut"].clone();
    if MACOS {
        assert_eq!(shortcut("Paste"), json!("\u{2318}V"));
    } else {
        assert_eq!(shortcut("Paste"), json!("Ctrl+V"));
    }
    let submenu =
        |label: &str| rows.iter().find(|r| r["label"] == label).unwrap()["submenu"].clone();
    assert_eq!(submenu("Go To"), json!(true));
    assert_eq!(submenu("Extra"), json!(true));
    assert_eq!(submenu("Open In"), json!(true));

    h.invoke("menu.cancel", Value::Null);
    assert!(h.state()["editor_menu"].is_null());
}

#[gpui::test]
fn the_selection_gives_cut_and_copy_and_survives_a_click_inside_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("editor.select-all", Value::Null);
    right_click(&mut h, 0, 2);
    let rows = rows(&mut h);
    assert_eq!(
        &labels(&rows)[..5],
        ["Show Context Actions", "---", "Cut", "Copy", "Paste"]
    );
    let selected = editor(&mut h).read_with(h.cx, |e, _| e.selections().primary().is_empty());
    assert!(!selected);
}

#[gpui::test]
fn a_row_runs_its_command_on_the_editor(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    right_click(&mut h, 0, 2);
    // Duplicate by its mnemonic key.
    h.press("d");
    assert_eq!(
        editor(&mut h).read_with(h.cx, |e, _| e.text()),
        "alpha beta\nalpha beta\nsecond line\n"
    );
    assert!(h.state()["editor_menu"].is_null());
}

#[gpui::test]
fn the_command_opens_the_menu_and_a_type_without_rows_adds_none(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = open(cx, dir.path());
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    assert!(labels(&rows(&mut h)).contains(&"Extra".to_owned()));
    h.invoke("menu.cancel", Value::Null);

    // The type's hook may leave a file out.
    h.invoke("project.open-file", json!({ "path": "plain.code" }));
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    let labels = labels(&rows(&mut h));
    assert!(!labels.contains(&"Extra".to_owned()), "{labels:?}");
    h.invoke("menu.cancel", Value::Null);

    // A file of a type without language services has the plain rows.
    h.invoke("project.open-file", json!({ "path": "b.txt" }));
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    assert_eq!(
        self::labels(&rows(&mut h)),
        ["Paste", "---", "Duplicate", "---", "Open In"]
    );
}
