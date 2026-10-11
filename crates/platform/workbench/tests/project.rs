//! The shell driven only through command ids, as an agent or the control
//! channel would: open a folder, browse the tree, open files in tabs, type,
//! undo, save, close. Files are real (a temporary directory); reads and
//! writes run on GPUI's background executor.

use std::path::Path;
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{FileType, Product, Workbench};

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            indent_after: &[':'],
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
}

fn try_invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Result<Value, ui::command::CommandError> {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    out
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    try_invoke(cx, window, id, args).unwrap_or_else(|e| panic!("{id}: {e:?}"))
}

fn state(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    let s = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(s["busy"], json!(false), "background work did not finish");
    s
}

fn project(root: &Path) {
    let w = |p: &str, text: &[u8]| {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    w("src/app.py", b"def main():\r\n    pass\r\n");
    w("README.md", b"# Demo\n");
    w(".git/HEAD", b"ref: refs/heads/main\n");
    w("src/__pycache__/app.cpython-313.pyc", b"\0\0");
    w(".venv/pyvenv.cfg", b"home = /usr\n");
    w("data.bin", b"\0\x01\x02");
}

fn rows(s: &Value) -> Vec<String> {
    s["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_owned())
        .collect()
}

#[gpui::test]
fn open_edit_undo_save_close(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    project(root);
    let window = open(cx);

    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    let s = state(cx, window);
    assert_eq!(
        s["project"]["name"],
        json!(root.file_name().unwrap().to_string_lossy())
    );
    // Version control folders and byte code are hidden; the virtual
    // environment is shown as excluded.
    assert_eq!(rows(&s), ["", ".venv", "src", "data.bin", "README.md"]);
    assert_eq!(s["tree"]["rows"][1]["excluded"], json!(true));

    // Reveal a file: its folders are listed and expanded on the way.
    invoke(
        cx,
        window,
        "project.tree.reveal",
        json!({"path": "src/app.py"}),
    );
    let s = state(cx, window);
    assert_eq!(s["tree"]["selected"], json!("src/app.py"));
    assert!(rows(&s).contains(&"src/app.py".to_owned()));

    // Enter on the selected row opens the file in a tab.
    invoke(cx, window, "project.tree.open", json!(null));
    let s = state(cx, window);
    assert_eq!(
        s["tabs"],
        json!([{"path": "src/app.py", "modified": false}])
    );
    assert_eq!(s["editor"]["line_ending"], json!("CRLF"));

    // Type after the colon: Enter adds an indentation level.
    invoke(
        cx,
        window,
        "editor.set-selection",
        json!({"anchor": {"line": 0, "column": 11}, "head": {"line": 0, "column": 11}}),
    );
    invoke(cx, window, "editor.newline", json!(null));
    invoke(cx, window, "editor.type", json!({"text": "x = 1"}));
    let text = invoke(cx, window, "editor.text", json!(null));
    assert_eq!(text, json!("def main():\n    x = 1\n    pass\n"));
    let s = state(cx, window);
    assert_eq!(s["tabs"][0]["modified"], json!(true));
    assert_eq!(s["editor"]["caret"], json!({"line": 1, "column": 9}));

    // Save writes the file with its own line separator.
    invoke(cx, window, "editor.save", json!(null));
    let s = state(cx, window);
    assert_eq!(s["tabs"][0]["modified"], json!(false));
    assert_eq!(
        std::fs::read(root.join("src/app.py")).unwrap(),
        b"def main():\r\n    x = 1\r\n    pass\r\n"
    );

    // Undo makes the file modified again; Save All writes it back.
    invoke(cx, window, "editor.undo", json!(null));
    assert_eq!(state(cx, window)["tabs"][0]["modified"], json!(true));
    invoke(cx, window, "workbench.save-all", json!(null));
    assert_eq!(state(cx, window)["tabs"][0]["modified"], json!(false));
    assert_eq!(
        std::fs::read_to_string(root.join("src/app.py")).unwrap(),
        // Undo removed the typing; the new line stays (its own group).
        "def main():\r\n    \r\n    pass\r\n"
    );

    // A second file; opening the first again selects its tab.
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    let s = state(cx, window);
    assert_eq!(s["tabs"].as_array().unwrap().len(), 2);
    assert_eq!(s["active_tab"], json!(0));

    // Closing a modified tab saves it first.
    invoke(cx, window, "tabs.select", json!({"index": 1}));
    invoke(cx, window, "editor.type", json!({"text": "Hi "}));
    invoke(cx, window, "tabs.close", json!(null));
    let s = state(cx, window);
    assert_eq!(
        s["tabs"],
        json!([{"path": "src/app.py", "modified": false}])
    );
    assert_eq!(s["active_tab"], json!(0));
    assert_eq!(
        std::fs::read_to_string(root.join("README.md")).unwrap(),
        "Hi # Demo\n"
    );

    invoke(cx, window, "tabs.close", json!(null));
    let s = state(cx, window);
    assert_eq!(s["tabs"], json!([]));
    assert_eq!(s["editor"], json!(null));
    assert_eq!(
        try_invoke(cx, window, "editor.text", json!(null))
            .unwrap_err()
            .code,
        "not_available"
    );
}

#[gpui::test]
fn files_that_cannot_be_opened_are_reported(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    invoke(cx, window, "project.open-file", json!({"path": "data.bin"}));
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "missing.py"}),
    );
    let s = state(cx, window);
    assert_eq!(s["tabs"], json!([]));
    assert!(
        s["message"].as_str().unwrap().contains("Cannot open"),
        "{}",
        s["message"]
    );
}

#[gpui::test]
fn tree_keyboard_commands(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    // The root is selected; down twice reaches "src".
    invoke(cx, window, "project.tree.down", json!(null));
    invoke(cx, window, "project.tree.down", json!(null));
    assert_eq!(state(cx, window)["tree"]["selected"], json!("src"));
    invoke(cx, window, "project.tree.expand", json!(null));
    let s = state(cx, window);
    assert!(rows(&s).contains(&"src/app.py".to_owned()));
    // Expand on an open folder selects its first child.
    invoke(cx, window, "project.tree.expand", json!(null));
    assert_eq!(state(cx, window)["tree"]["selected"], json!("src/app.py"));
    // Collapse on a file selects the parent, then collapses it.
    invoke(cx, window, "project.tree.collapse", json!(null));
    invoke(cx, window, "project.tree.collapse", json!(null));
    let s = state(cx, window);
    assert_eq!(s["tree"]["selected"], json!("src"));
    assert!(!rows(&s).contains(&"src/app.py".to_owned()));
    // The project tool window hides and comes back.
    invoke(cx, window, "workbench.toggle-project", json!(null));
    assert_eq!(
        state(cx, window)["tool_windows"]["project"]["visible"],
        json!(false)
    );
    invoke(cx, window, "workbench.toggle-project", json!(null));
    assert_eq!(
        state(cx, window)["tool_windows"]["project"]["visible"],
        json!(true)
    );
}
