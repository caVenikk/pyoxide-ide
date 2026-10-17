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
        auto_excluded: workbench::AutoExclusions {
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
        json!([{"path": "src/app.py", "modified": false, "deleted": false}])
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
        json!([{"path": "src/app.py", "modified": false, "deleted": false}])
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

fn excluded(s: &Value) -> Vec<(String, bool)> {
    s["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["path"].as_str().unwrap().to_owned(),
                r["excluded"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[gpui::test]
fn reference_exclusions_are_imported_once_and_gitignore_is_content(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let w = |p: &str, text: &str| {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    w(
        ".idea/modules.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="ProjectModuleManager">
    <modules>
      <module fileurl="file://$PROJECT_DIR$/.idea/x.iml" filepath="$PROJECT_DIR$/.idea/x.iml" />
    </modules>
  </component>
</project>"#,
    );
    w(
        ".idea/x.iml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<module type="PYTHON_MODULE" version="4">
  <component name="NewModuleRootManager">
    <content url="file://$MODULE_DIR$">
      <excludeFolder url="file://$MODULE_DIR$/venv" />
    </content>
  </component>
</module>"#,
    );
    w("venv/lib/site.py", "");
    w("build/out.txt", "");
    w("src/app.py", "");
    w(".gitignore", "build/\nvenv/\n");
    let window = open(cx);
    let open_folder = |cx: &mut TestAppContext| {
        invoke(
            cx,
            window,
            "project.open-folder",
            json!({"path": root.display().to_string()}),
        )
    };
    open_folder(cx);
    let s = state(cx, window);
    // `.idea` is shown; `venv` is excluded as in the reference; the folder
    // named in `.gitignore` is ordinary content.
    let want = |venv: bool, build: bool| {
        vec![
            ("".to_owned(), false),
            (".idea".to_owned(), false),
            ("build".to_owned(), build),
            ("src".to_owned(), false),
            ("venv".to_owned(), venv),
            (".gitignore".to_owned(), false),
        ]
    };
    assert_eq!(excluded(&s), want(true, false));
    // The import wrote Pyoxide's own settings and left `.idea` alone.
    let settings = root.join(".oxide/project.toml");
    let text = std::fs::read_to_string(&settings).unwrap();
    assert!(text.contains("paths = [\n    \"venv\",\n]"), "{text}");

    // From now on only `.oxide` counts: `.idea` is not read again.
    std::fs::write(
        &settings,
        "version = 1\n\n[exclusions]\npaths = [\"build\"]\n",
    )
    .unwrap();
    open_folder(cx);
    let s = state(cx, window);
    assert_eq!(excluded(&s), want(false, true));
}

#[gpui::test]
fn opening_a_project_creates_no_settings_folder(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    // A reference project without exclusions has nothing to import.
    std::fs::create_dir_all(dir.path().join(".idea")).unwrap();
    std::fs::write(dir.path().join(".idea/misc.xml"), "<project/>").unwrap();
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    let s = state(cx, window);
    assert_eq!(rows(&s)[..3], ["", ".idea", ".venv"]);
    assert!(!dir.path().join(".oxide").exists());
}

#[gpui::test]
fn toolbar_shows_the_active_file_path(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    assert_eq!(state(cx, window)["title_path"], json!(null));
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    let full = root.join("src").join("app.py");
    let relative = Path::new("src").join("app.py");
    let expected = format!("{} [{}]", relative.display(), full.display());
    assert_eq!(state(cx, window)["title_path"], json!(expected));
    // It follows the active tab.
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    let expected = format!("README.md [{}]", root.join("README.md").display());
    assert_eq!(state(cx, window)["title_path"], json!(expected));

    // Centred in the toolbar right of the space for the window buttons
    // while there is room on both sides (tests/toolbar_layout.rs: with
    // less room).
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.simulate_resize(gpui::size(gpui::px(2400.0), gpui::px(800.0)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let toolbar = cx.debug_bounds("toolbar").unwrap();
    let widget = cx.debug_bounds("file-path-widget").unwrap();
    let inset = if cfg!(target_os = "macos") {
        workbench::chrome::TRAFFIC_LIGHTS_INSET
    } else {
        8.0
    };
    let expected_centre = (f32::from(toolbar.left()) + inset + f32::from(toolbar.right())) / 2.0;
    let centre = f32::from(widget.center().x);
    assert!(
        (centre - expected_centre).abs() < 1.0,
        "centre {centre}, expected {expected_centre}"
    );
}

#[gpui::test]
fn dragging_the_splitter_resizes_the_project_tool_window(cx: &mut TestAppContext) {
    let window = open(cx);
    let width = |cx: &mut TestAppContext| {
        state(cx, window)["tool_windows"]["project"]["width"]
            .as_f64()
            .unwrap() as f32
    };
    let before = width(cx);
    let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let splitter = vcx.debug_bounds("project-splitter").unwrap();
    let from = splitter.center();
    let to = gpui::point(from.x + gpui::px(50.0), from.y);
    vcx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    vcx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(width(cx), before + 50.0);
    // It does not get narrower than its minimum.
    let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let from = gpui::point(from.x + gpui::px(50.0), from.y);
    let to = gpui::point(gpui::px(0.0), from.y);
    vcx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    vcx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.run_until_parked();
    // The main splitter's minimum (`crate::layout`).
    let min = invoke(cx, window, "workbench.layout", json!(null))["min"].clone();
    assert_eq!(json!(width(cx)), min);
}

#[gpui::test]
fn go_to_line_field_closes_when_it_loses_the_focus(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    // Focus events reach listeners only in the active window (test windows
    // start inactive).
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    invoke(cx, window, "workbench.go-to-line", json!(null));
    assert_eq!(state(cx, window)["go_to_line"], json!("1:1"));
    // Text that is not a line number keeps the field open.
    cx.simulate_input(window.into(), "x");
    invoke(cx, window, "go-to-line.confirm", json!(null));
    assert_eq!(state(cx, window)["go_to_line"], json!("x"));
    invoke(cx, window, "workbench.focus-editor", json!(null));
    assert_eq!(state(cx, window)["go_to_line"], json!(null));
}

#[gpui::test]
fn pointer_shapes_over_text_gutter_and_splitter(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    let editor = window
        .read_with(cx, |w, _| w.active_editor().cloned())
        .unwrap()
        .unwrap();
    let (origin, lh, cw) = editor.read_with(cx, |e, _| e.text_geometry().unwrap());
    let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let splitter = vcx.debug_bounds("project-splitter").unwrap().center();
    let y = origin.y + lh / 2.0;
    for (position, expected) in [
        (
            gpui::point(origin.x + cw * 2.0, y),
            gpui::CursorStyle::IBeam,
        ),
        (
            gpui::point(origin.x - gpui::px(24.0), y),
            gpui::CursorStyle::Arrow,
        ),
        (splitter, gpui::CursorStyle::ResizeLeftRight),
        (
            gpui::point(gpui::px(300.0), gpui::px(20.0)),
            gpui::CursorStyle::Arrow,
        ),
    ] {
        vcx.simulate_mouse_move(position, None, gpui::Modifiers::none());
        vcx.run_until_parked();
        let shape = vcx.update(|window, _| ui::pointer::resolve(window));
        assert_eq!(shape, expected, "at {position:?}");
    }
}

fn open_project_with_files(
    cx: &mut TestAppContext,
) -> (WindowHandle<Workbench>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let window = open(cx);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    (window, dir)
}

fn advance(cx: &mut TestAppContext, ms: u64) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(ms));
    cx.run_until_parked();
}

#[gpui::test]
fn files_are_saved_after_idle_time(cx: &mut TestAppContext) {
    let (window, dir) = open_project_with_files(cx);
    let file = dir.path().join("src/app.py");
    let original = std::fs::read(&file).unwrap();
    invoke(cx, window, "editor.type", json!({"text": "x"}));
    advance(cx, 1500);
    assert_eq!(std::fs::read(&file).unwrap(), original, "saved too early");
    // More input restarts the idle time.
    invoke(cx, window, "editor.type", json!({"text": "y"}));
    advance(cx, 1500);
    assert_eq!(std::fs::read(&file).unwrap(), original, "saved too early");
    advance(cx, 600);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "xydef main():\r\n    pass\r\n"
    );
    assert_eq!(state(cx, window)["tabs"][0]["modified"], json!(false));

    // Keyboard input counts as activity too.
    invoke(cx, window, "editor.type", json!({"text": "z"}));
    advance(cx, 1500);
    cx.simulate_keystrokes(window.into(), "escape");
    advance(cx, 1500);
    assert!(std::fs::read_to_string(&file).unwrap().starts_with("xyd"));
    advance(cx, 600);
    assert!(std::fs::read_to_string(&file).unwrap().starts_with("xyz"));

    // Off: no save after the idle time.
    window
        .update(cx, |w, _, _| {
            w.set_save_settings(workbench::SaveSettings {
                save_on_idle: false,
                ..workbench::SaveSettings::default()
            })
        })
        .unwrap();
    invoke(cx, window, "editor.type", json!({"text": "!"}));
    advance(cx, 10_000);
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.starts_with("xyzd"), "{disk:?}");
}

#[gpui::test]
fn leaving_a_tab_saves_its_file(cx: &mut TestAppContext) {
    let (window, dir) = open_project_with_files(cx);
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    invoke(cx, window, "editor.type", json!({"text": "Hi "}));
    assert_eq!(state(cx, window)["tabs"][1]["modified"], json!(true));
    invoke(cx, window, "tabs.select", json!({"index": 0}));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "Hi # Demo\n"
    );
    assert_eq!(state(cx, window)["tabs"][1]["modified"], json!(false));
}

#[gpui::test]
fn deactivating_the_window_saves_files(cx: &mut TestAppContext) {
    let (window, dir) = open_project_with_files(cx);
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    invoke(cx, window, "editor.type", json!({"text": "x"}));
    let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    vcx.deactivate_window();
    vcx.run_until_parked();
    assert!(
        std::fs::read_to_string(dir.path().join("src/app.py"))
            .unwrap()
            .starts_with("xdef")
    );
}

#[gpui::test]
fn a_modified_file_shows_a_dot_in_its_tab(cx: &mut TestAppContext) {
    let (window, _dir) = open_project_with_files(cx);
    let marker = |cx: &mut TestAppContext| {
        gpui::VisualTestContext::from_window(window.into(), cx).debug_bounds("tab-modified-0")
    };
    assert!(marker(cx).is_none());
    invoke(cx, window, "editor.type", json!({"text": "x"}));
    let dot = marker(cx).expect("the modified dot is drawn");
    // In the place of the close button, which is 16 px wide.
    assert_eq!(dot.size.width, gpui::px(16.0));
    // The title has no asterisk.
    assert_eq!(state(cx, window)["tabs"][0]["modified"], json!(true));
}
