//! Files and folders dragged from the file manager onto the main window
//! (tests/scenarios/editor/open-file-tabs.md, "Drop files and folders").
//!
//! The drags go through GPUI's test platform as the platform layer sends
//! them: `FileDropEvent::Entered` with the paths, `Pending` while moving,
//! `Submit` on release and `Exited` at the end.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, ExternalPaths, FileDropEvent, Pixels, Point, TestAppContext,
    VisualTestContext, WindowHandle, point, px,
};
use serde_json::{Value, json};
use smallvec::SmallVec;
use workbench::{FileType, Product, Workbench};

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    draw(cx, window);
    window
}

/// Renders the next frame, as the display link would.
fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn invoke(cx: &mut TestAppContext, window: WindowHandle<Workbench>, id: &str, args: Value) {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke(id, args, window, cx)
    })
    .unwrap()
    .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx, window);
}

fn state(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    let s = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke("workbench.state", json!(null), window, cx)
        })
        .unwrap()
        .unwrap();
    assert_eq!(s["busy"], json!(false), "background work did not finish");
    s
}

fn tabs(s: &Value) -> Vec<String> {
    s["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap().to_owned())
        .collect()
}

/// A project folder: `src/app.py`, `src/util.py`, `README.md`, `data.bin`.
/// The path is canonical, as the workbench keeps the project root.
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let w = |p: &str, text: &[u8]| {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    w("src/app.py", b"def main():\n    pass\n\nmain()\n");
    w("src/util.py", b"X = 1\n");
    w("README.md", b"# Demo\n");
    w("data.bin", b"\0\x01\x02");
    (dir, root)
}

fn open_folder(cx: &mut TestAppContext, window: WindowHandle<Workbench>, root: &Path) {
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    part: &'static str,
) -> Bounds<Pixels> {
    VisualTestContext::from_window(window.into(), cx)
        .debug_bounds(part)
        .unwrap_or_else(|| panic!("{part} is not drawn"))
}

/// The paths of a drag from the file manager, as the platform layer builds
/// them. GPUI 0.2.2 keeps the constructor of `ExternalPaths` (a struct with
/// one private field, `SmallVec<[PathBuf; 2]>`) to itself, so the value is
/// made from that field.
fn external_paths(paths: &[PathBuf]) -> ExternalPaths {
    let list: SmallVec<[PathBuf; 2]> = paths.iter().cloned().collect();
    // SAFETY: relies on gpui 0.2.2 (`gpui = "=0.2.2"` in the workspace
    // manifest), where src/interactive.rs declares
    // `struct ExternalPaths(pub(crate) SmallVec<[PathBuf; 2]>)`; the
    // workspace has one `smallvec` 1.x, so this is the type of that field.
    // `transmute` only compiles when both types have the same size, which
    // leaves the one field at offset 0: the layouts are the same. The paths
    // are read back below. Check this again when GPUI is upgraded.
    let external = unsafe { std::mem::transmute::<SmallVec<[PathBuf; 2]>, ExternalPaths>(list) };
    assert_eq!(external.paths(), paths);
    external
}

fn file_drop(cx: &mut TestAppContext, window: WindowHandle<Workbench>, event: FileDropEvent) {
    VisualTestContext::from_window(window.into(), cx).simulate_event(event);
    draw(cx, window);
}

/// Drags `paths` from the file manager into the window along `route`
/// (window positions) and drops them at its end.
fn drag_and_drop(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    paths: &[PathBuf],
    route: &[Point<Pixels>],
) {
    file_drop(
        cx,
        window,
        FileDropEvent::Entered {
            position: route[0],
            paths: external_paths(paths),
        },
    );
    for &position in &route[1..] {
        file_drop(cx, window, FileDropEvent::Pending { position });
    }
    let position = *route.last().unwrap();
    file_drop(cx, window, FileDropEvent::Submit { position });
    file_drop(cx, window, FileDropEvent::Exited);
    let dragging = cx.update(|cx| cx.has_active_drag());
    assert!(!dragging, "the drag is over");
}

/// Drops `paths` onto the middle of the editor area.
fn drop_on_editor(cx: &mut TestAppContext, window: WindowHandle<Workbench>, paths: &[PathBuf]) {
    let area = bounds(cx, window, "editor-area");
    drag_and_drop(cx, window, paths, &[area.center()]);
}

fn editor_focused(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> bool {
    window
        .update(cx, |workbench, window, cx| {
            workbench
                .active_editor()
                .is_some_and(|e| gpui::Focusable::focus_handle(e.read(cx), cx).is_focused(window))
        })
        .unwrap()
}

fn tree_focused(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> bool {
    window
        .update(cx, |workbench, window, cx| {
            gpui::Focusable::focus_handle(workbench.panel().read(cx), cx)
                .contains_focused(window, cx)
        })
        .unwrap()
}

#[gpui::test]
fn files_dropped_on_the_editor_area_open_in_tabs(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let window = open(cx);
    open_folder(cx, window, &root);

    // Onto the empty editor area: in the order they were dropped, the last
    // one selected.
    drop_on_editor(
        cx,
        window,
        &[root.join("src/util.py"), root.join("README.md")],
    );
    let s = state(cx, window);
    assert_eq!(tabs(&s), ["src/util.py", "README.md"]);
    assert_eq!(s["active_tab"], json!(1));
    assert!(editor_focused(cx, window));

    // Onto the tab strip: a file that is open already has its tab selected.
    let strip = bounds(cx, window, "editor-tabs");
    let on_strip = point(strip.right() - px(20.0), strip.center().y);
    drag_and_drop(
        cx,
        window,
        &[root.join("src/app.py"), root.join("src/util.py")],
        &[on_strip],
    );
    let s = state(cx, window);
    assert_eq!(tabs(&s), ["src/util.py", "README.md", "src/app.py"]);
    assert_eq!(s["active_tab"], json!(0));

    // Onto the editor, dragged in over the tree: a file from outside the
    // project opens too.
    let other = tempfile::tempdir().unwrap();
    let outside = std::fs::canonicalize(other.path())
        .unwrap()
        .join("notes.txt");
    std::fs::write(&outside, "notes\n").unwrap();
    let tree = bounds(cx, window, "project-tool-window");
    let area = bounds(cx, window, "editor-area");
    drag_and_drop(
        cx,
        window,
        std::slice::from_ref(&outside),
        &[tree.center(), area.center()],
    );
    let s = state(cx, window);
    assert_eq!(s["tabs"].as_array().unwrap().len(), 4);
    assert_eq!(s["tabs"][3]["path"], json!(outside.display().to_string()));
    assert_eq!(s["active_tab"], json!(3));
    assert!(editor_focused(cx, window));

    // A file that cannot be opened says so; the others still open.
    invoke(cx, window, "tabs.close", json!(null));
    drop_on_editor(cx, window, &[root.join("data.bin"), outside.clone()]);
    let s = state(cx, window);
    assert_eq!(s["active_tab"], json!(3));
    let message = s["message"].as_str().unwrap();
    assert!(
        message.starts_with("Cannot open") && message.contains("data.bin"),
        "{message}"
    );
}

#[gpui::test]
fn a_drop_on_the_tree_or_the_chrome_does_nothing(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let window = open(cx);
    open_folder(cx, window, &root);
    invoke(cx, window, "project.tree.reveal", json!({"path": "src"}));
    let file = root.join("README.md");
    let folder = root.join("src");

    // Moving and copying files into a folder of the tree is a feature of
    // its own; until then a drop on the tree, on a row or below the rows,
    // does nothing, as a drop on the toolbar, the stripes or the status bar.
    let tree = bounds(cx, window, "project-tool-window");
    for at in [
        point(tree.center().x, tree.top() + px(60.0)),
        tree.center(),
        bounds(cx, window, "toolbar").center(),
        bounds(cx, window, "left-stripe").center(),
        bounds(cx, window, "status-bar").center(),
    ] {
        drag_and_drop(cx, window, &[file.clone(), folder.clone()], &[at]);
        let s = state(cx, window);
        assert_eq!(tabs(&s), Vec::<String>::new(), "dropped at {at:?}");
        assert_eq!(s["tree"]["selected"], json!("src"));
    }

    // The same with a tab open.
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    let row = point(tree.center().x, tree.top() + px(60.0));
    drag_and_drop(cx, window, std::slice::from_ref(&file), &[row]);
    assert_eq!(tabs(&state(cx, window)), ["src/app.py"]);
    // A drag that leaves the window without a drop does nothing either.
    let area = bounds(cx, window, "editor-area");
    file_drop(
        cx,
        window,
        FileDropEvent::Entered {
            position: area.center(),
            paths: external_paths(std::slice::from_ref(&file)),
        },
    );
    file_drop(cx, window, FileDropEvent::Exited);
    assert_eq!(tabs(&state(cx, window)), ["src/app.py"]);
}

#[gpui::test]
fn a_drag_over_an_editor_keeps_its_selection(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let window = open(cx);
    open_folder(cx, window, &root);
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "src/app.py"}),
    );
    let selection = json!({"anchor": {"line": 0, "column": 4}, "head": {"line": 1, "column": 4}});
    invoke(cx, window, "editor.set-selection", selection.clone());
    invoke(cx, window, "workbench.toggle-project", json!(null));
    assert!(tree_focused(cx, window));

    // Files dragged over the text, then out of the window: the editor's
    // selection and the focus stay as they were.
    let (origin, line_height, _) = window
        .update(cx, |workbench, _, cx| {
            workbench.active_editor().unwrap().read(cx).text_geometry()
        })
        .unwrap()
        .unwrap();
    let on_text = point(origin.x + px(30.0), origin.y + line_height * 3.5);
    let tree = bounds(cx, window, "project-tool-window");
    file_drop(
        cx,
        window,
        FileDropEvent::Entered {
            position: tree.center(),
            paths: external_paths(&[root.join("README.md")]),
        },
    );
    file_drop(cx, window, FileDropEvent::Pending { position: on_text });
    file_drop(cx, window, FileDropEvent::Exited);
    let s = state(cx, window);
    assert_eq!(s["editor"]["selections"], json!([selection]));
    assert_eq!(tabs(&s), ["src/app.py"]);
    assert!(tree_focused(cx, window));

    // Dropped there, the file opens; the first editor keeps its selection.
    drag_and_drop(
        cx,
        window,
        &[root.join("README.md")],
        &[tree.center(), on_text],
    );
    assert_eq!(tabs(&state(cx, window)), ["src/app.py", "README.md"]);
    invoke(cx, window, "tabs.select", json!({"index": 0}));
    assert_eq!(
        state(cx, window)["editor"]["selections"],
        json!([selection])
    );
}

#[gpui::test]
fn a_dropped_folder_of_the_project_is_selected_in_the_tree(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let window = open(cx);
    open_folder(cx, window, &root);
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    invoke(cx, window, "workbench.hide-tool-window", json!(null));

    // The project tool window opens, selects the folder and takes the
    // focus; the tabs stay as they are.
    drop_on_editor(cx, window, &[root.join("src")]);
    let s = state(cx, window);
    assert_eq!(s["tool_windows"]["project"]["visible"], json!(true));
    assert_eq!(s["tree"]["selected"], json!("src"));
    assert_eq!(tabs(&s), ["README.md"]);
    assert!(tree_focused(cx, window));

    // Files and a folder: the files open, and the folder, dropped last, is
    // selected.
    drop_on_editor(cx, window, &[root.join("src/app.py"), root.clone()]);
    let s = state(cx, window);
    assert_eq!(tabs(&s), ["README.md", "src/app.py"]);
    assert_eq!(s["active_tab"], json!(1));
    assert_eq!(s["tree"]["selected"], json!(""));
    assert!(tree_focused(cx, window));

    // A folder outside the project is left alone.
    let other = tempfile::tempdir().unwrap();
    invoke(cx, window, "workbench.focus-editor", json!(null));
    drop_on_editor(cx, window, &[other.path().to_owned()]);
    let s = state(cx, window);
    assert_eq!(s["project"]["root"], json!(root.display().to_string()));
    assert_eq!(s["tree"]["selected"], json!(""));
    assert_eq!(tabs(&s), ["README.md", "src/app.py"]);
}

#[gpui::test]
fn without_a_project_a_dropped_folder_opens_as_the_project(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let window = open(cx);

    // Files open in tabs without a project.
    drop_on_editor(cx, window, &[root.join("README.md")]);
    let s = state(cx, window);
    assert_eq!(s["project"], json!(null));
    assert_eq!(tabs(&s), [root.join("README.md").display().to_string()]);

    // A folder opens as the project; the rest of the drop is left alone.
    drop_on_editor(cx, window, &[root.join("src/app.py"), root.clone()]);
    let s = state(cx, window);
    assert_eq!(s["project"]["root"], json!(root.display().to_string()));
    assert_eq!(tabs(&s), Vec::<String>::new());
}

const EMPTY_TEXT_LINES: [&str; 3] = ["empty-text-0", "empty-text-1", "empty-text-2"];

/// The lines of the empty editor area, with a shortcut after its action.
fn empty_text(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Vec<String> {
    let lines = window
        .update(cx, |workbench, window, cx| workbench.empty_text(window, cx))
        .unwrap();
    // They are drawn one below the other (GPUI keeps the bounds of earlier
    // frames, so lines that are gone cannot be told apart here).
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    let top = vcx.debug_bounds(EMPTY_TEXT_LINES[0]).map(|b| b.top());
    for (i, name) in EMPTY_TEXT_LINES.iter().take(lines.len()).enumerate() {
        let line = vcx
            .debug_bounds(name)
            .unwrap_or_else(|| panic!("{name} is not drawn"));
        assert_eq!(Some(line.top()), top.map(|t| t + px(36.0) * i as f32));
    }
    lines
        .into_iter()
        .map(|(text, keys)| match keys {
            Some(keys) => format!("{text} {keys}"),
            None => text.to_owned(),
        })
        .collect()
}

#[gpui::test]
fn the_empty_editor_area_shows_the_hints_of_the_reference(cx: &mut TestAppContext) {
    let project_view = if cfg!(target_os = "macos") {
        "Project View ⌘1"
    } else {
        "Project View Alt+1"
    };
    let window = open(cx);
    // No project: how to start, then the hints.
    assert_eq!(
        empty_text(cx, window),
        ["Open a folder to start", "Drop files here to open them"]
    );
    invoke(cx, window, "workbench.hide-tool-window", json!(null));
    assert_eq!(
        empty_text(cx, window),
        [
            "Open a folder to start",
            project_view,
            "Drop files here to open them"
        ]
    );

    // A project: the Project View hint only while the tool window is
    // hidden.
    let (_dir, root) = project();
    open_folder(cx, window, &root);
    assert_eq!(empty_text(cx, window), ["Drop files here to open them"]);
    invoke(cx, window, "workbench.hide-tool-window", json!(null));
    assert_eq!(
        empty_text(cx, window),
        [project_view, "Drop files here to open them"]
    );

    // The lines are left-aligned in a block centred across the area, the
    // first line at 3/8 of the area's height.
    let area = bounds(cx, window, "editor-area");
    let first = bounds(cx, window, "empty-text-0");
    let second = bounds(cx, window, "empty-text-1");
    let baseline = area.top() + area.size.height * 0.375;
    assert!(first.top() < baseline && baseline < first.bottom());
    assert_eq!(second.top() - first.top(), px(36.0));
    assert_eq!(first.left(), second.left());
    let block_right = first.right().max(second.right());
    let (left_room, right_room) = (first.left() - area.left(), area.right() - block_right);
    assert!((left_room - right_room).abs() < px(1.0));

    // No hints while a file is open.
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    assert_eq!(empty_text(cx, window), Vec::<String>::new());
}
