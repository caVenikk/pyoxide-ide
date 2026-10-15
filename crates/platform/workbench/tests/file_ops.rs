//! File operations in the project tool window (#37,
//! tests/scenarios/editor/tree-file-operations.md): New File and New
//! Directory with their popups and the reference IDE's checks, Rename with
//! its dialog, Delete with its question; the tree, the editor tabs, the
//! file index and the language services follow.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::fake::{Call, FakeCodeIntel};
use code_intel::{CodeIntel, FileChange, FileChangeKind};
use gpui::{App, AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{
    CodeIntelHook, CodeIntelProvider, FileType, NewItem, NewItemKind, NewTemplate, Product,
    SyncSettings, Workbench,
};

mod support;

struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
    intel: Arc<FakeCodeIntel>,
    _dir: tempfile::TempDir,
}

/// A product's rows of New, as the Python product has them.
fn test_items() -> Vec<NewItem> {
    let icon = ui::icons::IconName::FILE_TEXT;
    vec![
        NewItem {
            label: "Python Package",
            icon: ui::icons::IconName::PACKAGE,
            title: "New Python Package",
            kinds: vec![NewItemKind {
                label: "Package",
                icon: ui::icons::IconName::PACKAGE,
                template: NewTemplate::Package {
                    marker: "__init__.py",
                    text: "",
                },
            }],
        },
        NewItem {
            label: "Python File",
            icon,
            title: "New Python File",
            kinds: vec![
                NewItemKind {
                    label: "Python file",
                    icon,
                    template: NewTemplate::File {
                        extension: "py",
                        text: "",
                    },
                },
                NewItemKind {
                    label: "Python unit test",
                    icon,
                    template: NewTemplate::File {
                        extension: "py",
                        text: "import unittest\n",
                    },
                },
            ],
        },
    ]
}

fn setup(cx: &mut TestAppContext) -> H<'_> {
    setup_with(cx, SyncSettings::default())
}

/// The project opened with these settings of the file watcher.
fn setup_with(cx: &mut TestAppContext, sync: SyncSettings) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for (path, text) in [
        ("app.py", "a = 1\n"),
        ("notes.txt", "n\n"),
        ("src/m.py", "m = 3\n"),
        ("src/pkg/n.py", "n = 4\n"),
        ("docs/index.md", "# docs\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let intel = FakeCodeIntel::new();
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        code_intel: Some(CodeIntelHook::new(Provider(intel.clone()))),
        new_items: test_items(),
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
        .update(cx, |w, _, _| w.set_sync_settings(sync))
        .unwrap();
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    let mut h = H {
        cx,
        window,
        root: root.clone(),
        intel,
        _dir: dir,
    };
    h.invoke(
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    // Tests never fill the system trash.
    let panel = h.window.read_with(h.cx, |w, _| w.panel().clone()).unwrap();
    h.cx.update(|cx| panel.update(cx, |p, _| p.set_delete_to_trash(false)));
    h
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
        out
    }

    fn state(&mut self) -> Value {
        let s = self.invoke("workbench.state", json!(null));
        assert_eq!(s["busy"], json!(false), "background work did not finish");
        s
    }

    fn tree(&mut self) -> Value {
        self.state()["tree"].clone()
    }

    fn rows(&mut self) -> Vec<String> {
        self.tree()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect()
    }

    fn popup(&mut self) -> Value {
        self.tree()["name_popup"].clone()
    }

    fn select(&mut self, path: &str) {
        self.invoke("project.tree.reveal", json!({ "path": path }));
        let panel = self
            .window
            .read_with(self.cx, |w, _| w.panel().clone())
            .unwrap();
        let focus = self
            .cx
            .update(|cx| gpui::Focusable::focus_handle(panel.read(cx), cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&focus))
            .unwrap();
        self.draw();
        assert_eq!(self.tree()["selected"], json!(path));
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    /// Replaces the text of the popup's field.
    fn set_field(&mut self, text: &str) {
        self.invoke("editor.select-all", json!(null));
        if text.is_empty() {
            self.invoke("editor.backspace", json!(null));
        } else {
            self.invoke("editor.type", json!({ "text": text }));
        }
    }

    fn tabs(&mut self) -> Vec<(String, bool)> {
        self.state()["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["path"].as_str().unwrap().to_owned(),
                    t["modified"].as_bool().unwrap(),
                )
            })
            .collect()
    }

    fn file_changes(&mut self) -> Vec<FileChange> {
        self.intel
            .take_calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::FilesChanged(changes) => Some(changes),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// Paths Go to File finds for `text`.
    fn go_to_file(&mut self, text: &str) -> Vec<String> {
        self.invoke(
            "search-everywhere.search",
            json!({ "text": text, "tab": "files" }),
        );
        let s = self.invoke("search-everywhere.state", json!(null));
        self.invoke("search-everywhere.close", json!(null));
        s["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["path"].as_str())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn transfer(&mut self) -> Value {
        self.tree()["transfer"].clone()
    }

    fn clipboard_text(&mut self) -> Option<String> {
        self.cx
            .update(|cx| cx.read_from_clipboard())
            .and_then(|c| c.text())
    }

    /// Presses on the row of `from` and moves the pointer over the row of
    /// `to` without letting go.
    fn drag_over(&mut self, from: &str, to: &str) -> gpui::Point<gpui::Pixels> {
        let centre = |h: &mut H, path: &str| {
            h.draw();
            let name: &'static str = Box::leak(format!("tree-row:{path}").into_boxed_str());
            gpui::VisualTestContext::from_window(h.window.into(), h.cx)
                .debug_bounds(name)
                .unwrap_or_else(|| panic!("{path} is not drawn"))
                .center()
        };
        let start = centre(self, from);
        let end = centre(self, to);
        let none = gpui::Modifiers::none();
        let vcx = &mut gpui::VisualTestContext::from_window(self.window.into(), self.cx);
        vcx.simulate_mouse_move(start, None, none);
        vcx.simulate_mouse_down(start, gpui::MouseButton::Left, none);
        let mid = gpui::point(start.x, (start.y + end.y) / 2.0);
        vcx.simulate_mouse_move(mid, gpui::MouseButton::Left, none);
        vcx.simulate_mouse_move(end, gpui::MouseButton::Left, none);
        self.draw();
        end
    }

    /// Drags the row of `from` onto the row of `to` with the mouse.
    fn drag(&mut self, from: &str, to: &str, modifiers: gpui::Modifiers) {
        let centre = |h: &mut H, path: &str| {
            h.draw();
            let name: &'static str = Box::leak(format!("tree-row:{path}").into_boxed_str());
            gpui::VisualTestContext::from_window(h.window.into(), h.cx)
                .debug_bounds(name)
                .unwrap_or_else(|| panic!("{path} is not drawn"))
                .center()
        };
        let start = centre(self, from);
        let end = centre(self, to);
        let vcx = &mut gpui::VisualTestContext::from_window(self.window.into(), self.cx);
        vcx.simulate_mouse_move(start, None, modifiers);
        vcx.simulate_mouse_down(start, gpui::MouseButton::Left, modifiers);
        let mid = gpui::point(start.x, (start.y + end.y) / 2.0);
        vcx.simulate_mouse_move(mid, gpui::MouseButton::Left, modifiers);
        vcx.simulate_mouse_move(end, gpui::MouseButton::Left, modifiers);
        vcx.simulate_mouse_up(end, gpui::MouseButton::Left, modifiers);
        self.draw();
    }

    /// Waits for real events (the file watcher) until `f` holds.
    fn wait(&mut self, what: &str, f: impl Fn(&mut Self) -> bool) {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(30) {
            support::tick(self.cx);
            if f(self) {
                return;
            }
        }
        panic!("timed out waiting for {what}");
    }

    /// The watcher has reported what happened before `name` was written
    /// outside the application (its row shows), and the time a missing
    /// file is given has passed.
    fn settle(&mut self, name: &str) {
        std::fs::write(self.root.join(name), "").unwrap();
        self.wait(name, |h| h.rows().contains(&name.to_owned()));
        for _ in 0..40 {
            support::tick(self.cx);
        }
        self.draw();
    }

    /// The active editor has the focus.
    fn editor_focused(&mut self) -> bool {
        let editor = self
            .window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor");
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                gpui::Focusable::focus_handle(editor.read(cx), cx).is_focused(window)
            })
            .unwrap()
    }
}

#[gpui::test]
fn new_file_from_the_popup_opens_it(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("src/m.py");
    // Cmd+N / Alt+Insert: the New popup of the file's folder.
    let keys = if cfg!(target_os = "macos") {
        "cmd-n"
    } else {
        "alt-insert"
    };
    h.press(keys);
    let menu = h.tree()["context_menu"].clone();
    assert_eq!(menu["title"], json!("New"));
    let labels: Vec<&str> = menu["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        ["File", "Directory", "Python Package", "Python File"]
    );
    h.invoke("menu.confirm", json!(null));
    let popup = h.popup();
    assert_eq!(popup["kind"], json!("new-item"));
    assert_eq!(popup["title"], json!("New File"));
    assert_eq!(popup["text"], json!(""));
    // An existing name is an error; it stays until the name changes.
    h.set_field("m.py");
    h.press("enter");
    assert_eq!(
        h.popup()["message"],
        json!("A file with the name 'm.py' already exists")
    );
    h.set_field("pkg/sub/new.py");
    assert_eq!(h.popup()["message"], Value::Null);
    h.press("enter");
    assert!(h.root.join("src/pkg/sub/new.py").is_file());
    let s = h.state();
    assert_eq!(s["tree"]["name_popup"], Value::Null);
    assert_eq!(s["tree"]["selected"], json!("src/pkg/sub/new.py"));
    let active = s["active_tab"].as_u64().unwrap() as usize;
    assert_eq!(s["tabs"][active]["path"], json!("src/pkg/sub/new.py"));
    assert!(h.editor_focused(), "the new file's editor has the focus");
    let rows = h.rows();
    assert!(rows.contains(&"src/pkg/sub".to_owned()), "{rows:?}");
    // The index and the language services know the new files.
    assert!(
        h.go_to_file("new.py").iter().any(|p| p.contains("new.py")),
        "Go to File finds the new file"
    );
    let changes = h.file_changes();
    assert!(changes.contains(&FileChange::new(
        h.root.join("src/pkg/sub/new.py"),
        FileChangeKind::Created
    )));
}

#[gpui::test]
fn new_file_names_follow_the_reference_rules(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("src");
    h.invoke("project.tree.new-file", json!({}));
    for (name, message) in [
        ("pkg", "A directory with the name 'pkg' already exists"),
        ("m.py/x.py", "A file with the name 'm.py' already exists"),
        ("..", "'..' is an invalid file name"),
        (
            "x.pyc",
            "Trying to create a directory with an ignored name ('x.pyc'); the result will not be visible",
        ),
        ("../../x.py", "'../../x.py' is outside the project"),
    ] {
        h.set_field(name);
        h.invoke("name-input.confirm", json!(null));
        assert_eq!(h.popup()["message"], json!(message), "{name}");
    }
    // Escape closes the popup; the focus goes back to the tree.
    h.press("escape");
    let s = h.state();
    assert_eq!(s["tree"]["name_popup"], Value::Null);
    assert!(!h.root.join("x.py").exists());
    // With a name, the command creates at once.
    h.invoke(
        "project.tree.new-file",
        json!({ "name": "a/b.py", "directory": "docs" }),
    );
    assert!(h.root.join("docs/a/b.py").is_file());
    assert_eq!(h.tree()["selected"], json!("docs/a/b.py"));
}

#[gpui::test]
fn new_directory_checks_as_you_type(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("app.py");
    h.invoke("project.tree.new-directory", json!(null));
    assert_eq!(h.popup()["title"], json!("New Directory"));
    h.set_field("src");
    assert_eq!(
        h.popup()["message"],
        json!("A directory with the name 'src' already exists")
    );
    assert_eq!(h.popup()["warning"], json!(false));
    h.set_field("a.b");
    assert_eq!(
        h.popup()["message"],
        json!(
            "Note: \".\" in the name is treated as a regular character. Use \"/\" instead if you mean to create nested directories"
        )
    );
    assert_eq!(h.popup()["warning"], json!(true));
    // A warning does not stop it.
    h.press("enter");
    assert!(h.root.join("a.b").is_dir());
    let s = h.state();
    assert_eq!(s["tree"]["selected"], json!("a.b"));
    assert_eq!(s["tree"]["name_popup"], Value::Null);
    // Folders come before files.
    let rows = h.rows();
    let at = |p: &str| rows.iter().position(|r| r == p).unwrap();
    assert!(
        at("a.b") < at("docs") && at("src") < at("app.py"),
        "{rows:?}"
    );
}

#[gpui::test]
fn rename_keeps_the_tab_and_its_changes(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.invoke("project.open-file", json!({ "path": "app.py" }));
    h.invoke("editor.type", json!({ "text": "x" }));
    assert_eq!(h.tabs(), [("app.py".to_owned(), true)]);
    h.intel.take_calls();
    h.select("app.py");
    h.press("shift-f6");
    let popup = h.popup();
    assert_eq!(popup["kind"], json!("rename"));
    assert_eq!(popup["title"], json!("Rename"));
    assert_eq!(popup["label"], json!("Rename file 'app.py' to:"));
    assert_eq!(popup["text"], json!("app.py"));
    // The name without its extension is selected: typing replaces it.
    h.invoke("editor.type", json!({ "text": "main" }));
    assert_eq!(h.popup()["text"], json!("main.py"));
    // An invalid name disables Refactor.
    h.set_field("a/b");
    let p = h.popup();
    assert_eq!(p["can_confirm"], json!(false));
    assert_eq!(p["message"], json!("'a/b' is not a valid identifier"));
    h.press("enter");
    assert!(h.root.join("app.py").exists());
    // Onto an existing file: an error, nothing renamed.
    h.set_field("notes.txt");
    h.press("enter");
    assert_eq!(
        h.popup()["message"],
        json!("File 'notes.txt' already exists.")
    );
    h.set_field("main.py");
    h.press("enter");
    assert!(!h.root.join("app.py").exists());
    assert!(h.root.join("main.py").is_file());
    let s = h.state();
    assert_eq!(s["tree"]["name_popup"], Value::Null);
    assert_eq!(s["tree"]["selected"], json!("main.py"));
    assert_eq!(h.tabs(), [("main.py".to_owned(), true)]);
    assert!(
        h.state()["title_path"]
            .as_str()
            .unwrap()
            .starts_with("main.py")
    );
    // The unsaved change goes to the new file.
    h.invoke("workbench.save-all", json!(null));
    assert_eq!(
        std::fs::read_to_string(h.root.join("main.py")).unwrap(),
        "xa = 1\n"
    );
    assert!(!h.root.join("app.py").exists());
    // The services see the old file go and the new one come.
    let calls = h.intel.take_calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::DidClose { doc } if doc.path() == h.root.join("app.py"))),
        "{calls:?}"
    );
    assert!(
        calls.iter().any(
            |c| matches!(c, Call::DidOpen { doc, .. } if doc.path() == h.root.join("main.py"))
        ),
        "{calls:?}"
    );
    assert!(calls.iter().any(|c| matches!(c, Call::FilesChanged(ch) if ch.contains(&FileChange::new(h.root.join("main.py"), FileChangeKind::Created)))));
}

#[gpui::test]
fn renaming_a_folder_moves_its_tabs_and_exclusion(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.invoke("project.open-file", json!({ "path": "src/pkg/n.py" }));
    h.invoke("project.tree.mark-excluded", json!({ "path": "docs" }));
    h.select("src/pkg");
    h.invoke("project.tree.expand", json!(null));
    h.invoke("project.tree.rename", json!(null));
    let popup = h.popup();
    assert_eq!(popup["label"], json!("Rename directory 'pkg' to:"));
    h.set_field("lib");
    h.invoke("name-input.confirm", json!(null));
    assert!(h.root.join("src/lib/n.py").is_file());
    assert_eq!(h.tabs(), [("src/lib/n.py".to_owned(), false)]);
    let tree = h.tree();
    assert_eq!(tree["selected"], json!("src/lib"));
    let row = tree["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == json!("src/lib"))
        .unwrap()
        .clone();
    assert_eq!(row["expanded"], json!(true), "it stays expanded");
    // A stored exclusion follows its folder.
    h.invoke(
        "project.tree.rename",
        json!({ "path": "docs", "name": "manual" }),
    );
    assert_eq!(h.tree()["exclusions"], json!(["manual"]));
    assert!(h.go_to_file("n.py").iter().any(|p| p.contains("lib")));
}

#[gpui::test]
fn delete_asks_and_closes_tabs(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.invoke("project.open-file", json!({ "path": "src/m.py" }));
    h.invoke("project.open-file", json!({ "path": "notes.txt" }));
    h.select("src");
    let rows = h.rows();
    let src_row = rows.iter().position(|r| r == "src").unwrap();
    h.press("delete");
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Delete"));
    assert_eq!(
        dialog["message"],
        json!(
            "Delete directory \"src\"?\nAll files and subdirectories in \"src\" will be deleted.\nYou might not be able to fully undo this operation!"
        )
    );
    assert_eq!(dialog["buttons"], json!(["Cancel", "Delete"]));
    assert_eq!(dialog["default"], json!("Delete"));
    // Cancel keeps everything.
    h.press("escape");
    assert!(h.root.join("src").exists());
    assert_eq!(h.state()["dialog"], Value::Null);
    assert_eq!(h.tree()["selected"], json!("src"));
    // Enter deletes; the tabs of its files close, the row below is
    // selected.
    h.invoke("project.tree.delete", json!(null));
    h.press("enter");
    assert!(!h.root.join("src").exists());
    let s = h.state();
    assert_eq!(s["dialog"], Value::Null);
    let rows = h.rows();
    assert!(!rows.iter().any(|r| r.starts_with("src")), "{rows:?}");
    assert_eq!(s["tree"]["selected"], json!(rows[src_row]));
    assert_eq!(h.tabs(), [("notes.txt".to_owned(), false)]);
    let found = h.go_to_file("m.py");
    assert!(!found.iter().any(|p| p.ends_with("m.py")), "{found:?}");
    let changes = h.file_changes();
    assert!(changes.contains(&FileChange::new(
        h.root.join("src"),
        FileChangeKind::Deleted
    )));
    // A file, by its path.
    h.invoke("project.tree.delete", json!({ "paths": ["notes.txt"] }));
    assert_eq!(
        h.state()["dialog"]["message"],
        json!("Delete file \"notes.txt\"?")
    );
    h.invoke("dialog.choose", json!({ "button": "Delete" }));
    assert!(!h.root.join("notes.txt").exists());
    assert!(h.tabs().is_empty());
    // The project root is not deleted.
    h.select("");
    h.invoke("project.tree.delete", json!(null));
    assert_eq!(h.state()["dialog"], Value::Null);
}

#[gpui::test]
fn the_context_menu_has_the_file_operations(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("app.py");
    h.invoke("project.tree.context-menu", json!(null));
    let items = h.tree()["context_menu"]["levels"][0]["items"].clone();
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap_or("-"))
        .collect();
    assert_eq!(
        labels,
        [
            "New",
            "-",
            "Cut",
            "Copy",
            "Copy Path/Reference…",
            "Paste",
            "-",
            "Find in Files…",
            "Replace in Files…",
            "-",
            "Rename…",
            "Refactor",
            "-",
            "Delete…",
            "-",
            "Reload from Disk",
            "-",
            "Mark File As"
        ]
    );
    // New > File creates in the file's folder (the project root).
    h.invoke("menu.down", json!(null));
    h.invoke("menu.open-submenu", json!(null));
    h.invoke("menu.confirm", json!(null));
    assert_eq!(h.popup()["title"], json!("New File"));
    h.set_field("b.py");
    h.press("enter");
    assert!(h.root.join("b.py").is_file());
    // The root row has no Rename or Delete to choose.
    h.select("");
    h.invoke("project.tree.context-menu", json!(null));
    let items = h.tree()["context_menu"]["levels"][0]["items"].clone();
    let rename = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == json!("Rename…"))
        .unwrap()
        .clone();
    assert_eq!(rename["enabled"], json!(false), "{rename}");
    h.invoke("menu.cancel", json!(null));
}

#[gpui::test]
fn the_popups_work_with_the_tool_window_hidden(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.invoke("project.open-file", json!({ "path": "app.py" }));
    h.invoke("workbench.hide-tool-window", json!(null));
    assert_eq!(
        h.state()["tool_windows"]["project"]["visible"],
        json!(false)
    );
    h.invoke("project.tree.rename", json!({ "path": "app.py" }));
    assert_eq!(h.popup()["label"], json!("Rename file 'app.py' to:"));
    h.set_field("main.py");
    h.press("enter");
    assert!(h.root.join("main.py").is_file());
    assert_eq!(h.tabs(), [("main.py".to_owned(), false)]);
}

#[gpui::test]
fn the_navigation_history_follows_renames_and_deletes(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    for file in ["src/pkg/n.py", "src/m.py", "app.py"] {
        h.invoke("project.open-file", json!({ "path": file }));
    }
    let back = |h: &mut H| -> Vec<String> {
        h.state()["navigation"]["back"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["path"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(back(&mut h), ["src/pkg/n.py", "src/m.py"]);
    h.invoke(
        "project.tree.rename",
        json!({ "path": "src/pkg", "name": "lib" }),
    );
    assert_eq!(back(&mut h), ["src/lib/n.py", "src/m.py"]);
    h.invoke("project.tree.delete", json!({ "paths": ["src/m.py"] }));
    h.invoke("dialog.confirm", json!(null));
    assert_eq!(back(&mut h), ["src/lib/n.py"]);
    // Back goes to the renamed file.
    h.invoke("workbench.back", json!(null));
    let s = h.state();
    let active = s["active_tab"].as_u64().unwrap() as usize;
    assert_eq!(s["tabs"][active]["path"], json!("src/lib/n.py"));
}

#[gpui::test]
fn product_items_create_files_from_templates_and_packages(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("src");
    h.invoke("project.tree.context-menu", json!(null));
    h.invoke("menu.down", json!(null));
    h.invoke("menu.open-submenu", json!(null));
    let new: Vec<String> = h.tree()["context_menu"]["levels"][1]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(new, ["File", "Directory", "Python Package", "Python File"]);
    h.invoke("menu.cancel", json!(null));
    // Python File: the popup lists the kinds; Down picks the unit test.
    h.invoke("project.tree.new-item", json!({ "item": "Python File" }));
    let popup = h.popup();
    assert_eq!(popup["title"], json!("New Python File"));
    assert_eq!(popup["kinds"], json!(["Python file", "Python unit test"]));
    assert_eq!(popup["selected_kind"], json!("Python file"));
    h.press("down");
    assert_eq!(h.popup()["selected_kind"], json!("Python unit test"));
    h.set_field("test_m");
    h.press("enter");
    assert_eq!(
        std::fs::read_to_string(h.root.join("src/test_m.py")).unwrap(),
        "import unittest\n"
    );
    let s = h.state();
    let active = s["active_tab"].as_u64().unwrap() as usize;
    assert_eq!(s["tabs"][active]["path"], json!("src/test_m.py"));
    // Python Package: dotted names, an __init__.py in each new folder.
    h.select("src");
    h.invoke("project.tree.new-item", json!({ "item": "Python Package" }));
    assert_eq!(h.popup()["title"], json!("New Python Package"));
    h.set_field("pkg");
    assert_eq!(
        h.popup()["message"],
        json!("A directory with the name 'pkg' already exists")
    );
    h.set_field("app.core");
    h.press("enter");
    assert!(h.root.join("src/app/__init__.py").is_file());
    assert!(h.root.join("src/app/core/__init__.py").is_file());
    assert!(!h.root.join("src/__init__.py").exists());
    assert_eq!(h.tree()["selected"], json!("src/app/core"));
    // By command, with a kind.
    h.invoke(
        "project.tree.new-item",
        json!({ "item": "Python File", "kind": "Python file", "name": "tool", "directory": "docs" }),
    );
    assert!(h.root.join("docs/tool.py").is_file());
}

#[gpui::test]
fn copy_and_paste_asks_in_the_copy_dialog(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("app.py");
    let copy = if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    };
    let paste = if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    };
    h.press(copy);
    assert_eq!(
        h.tree()["clipboard"],
        json!({ "paths": ["app.py"], "cut": false })
    );
    h.select("src");
    h.press(paste);
    let t = h.transfer();
    assert_eq!(t["title"], json!("Copy"));
    let label = t["label"].as_str().unwrap();
    assert!(
        label.starts_with("Copy file ") && label.ends_with("app.py"),
        "{t}"
    );
    assert_eq!(t["name"], json!("app.py"));
    assert_eq!(
        t["directory"],
        json!(h.root.join("src").display().to_string())
    );
    assert_eq!(t["option"], json!("Open copy in editor"));
    assert_eq!(t["open_in_editor"], json!(true));
    // The name without the extension is selected: typing replaces it.
    h.invoke("editor.type", json!({ "text": "copy" }));
    assert_eq!(h.transfer()["name"], json!("copy.py"));
    h.press("enter");
    assert_eq!(
        std::fs::read_to_string(h.root.join("src/copy.py")).unwrap(),
        "a = 1\n"
    );
    assert!(h.root.join("app.py").exists(), "a copy keeps the original");
    let s = h.state();
    assert_eq!(s["tree"]["transfer"], Value::Null);
    assert_eq!(s["tree"]["selected"], json!("src/copy.py"));
    let active = s["active_tab"].as_u64().unwrap() as usize;
    assert_eq!(s["tabs"][active]["path"], json!("src/copy.py"));
    // The clipboard keeps the copied file: paste again elsewhere.
    h.select("docs");
    h.invoke("project.tree.paste", json!(null));
    h.invoke("name-input.toggle-option", json!(null));
    h.invoke("name-input.confirm", json!(null));
    assert!(h.root.join("docs/app.py").is_file());
}

#[gpui::test]
fn cut_and_paste_moves_and_tabs_follow(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.invoke("project.open-file", json!({ "path": "src/m.py" }));
    h.select("src/m.py");
    h.invoke("project.tree.cut", json!(null));
    assert_eq!(
        h.tree()["clipboard"],
        json!({ "paths": ["src/m.py"], "cut": true })
    );
    h.select("docs");
    h.invoke("project.tree.paste", json!(null));
    let t = h.transfer();
    assert_eq!(t["title"], json!("Move"));
    let label = t["label"].as_str().unwrap();
    assert!(
        label.starts_with("Move file ") && label.ends_with("m.py"),
        "{t}"
    );
    assert_eq!(t["name"], Value::Null);
    assert_eq!(
        t["directory"],
        json!(h.root.join("docs").display().to_string())
    );
    assert_eq!(t["option"], json!("Open in editor"));
    assert_eq!(t["open_in_editor"], json!(false));
    h.press("enter");
    assert!(h.root.join("docs/m.py").is_file());
    assert!(!h.root.join("src/m.py").exists());
    let s = h.state();
    assert_eq!(s["tree"]["clipboard"], Value::Null, "the cut ends");
    assert_eq!(s["tree"]["selected"], json!("docs/m.py"));
    assert_eq!(h.tabs(), [("docs/m.py".to_owned(), false)]);
    // F6 on a folder: into itself is refused in the dialog.
    h.select("src");
    h.press("f6");
    h.set_field(&h.root.join("src/pkg").display().to_string());
    h.press("enter");
    assert_eq!(
        h.transfer()["message"],
        json!("Cannot place directory into its subdirectory.")
    );
    h.press("escape");
    assert_eq!(h.transfer(), Value::Null);
    // By command, without the dialog.
    h.invoke(
        "project.tree.move",
        json!({ "paths": ["src/pkg"], "to": "docs" }),
    );
    assert!(h.root.join("docs/pkg/n.py").is_file());
}

#[gpui::test]
fn a_file_of_the_same_name_asks_overwrite_or_skip(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    std::fs::write(h.root.join("docs/notes.txt"), "old\n").unwrap();
    h.invoke(
        "project.tree.copy-element",
        json!({ "paths": ["notes.txt"], "to": "docs" }),
    );
    let d = h.state()["dialog"].clone();
    assert_eq!(d["title"], json!("Copy Files/Directories"));
    assert_eq!(
        d["message"],
        json!(format!(
            "File 'notes.txt' already exists in directory '{}'",
            h.root.join("docs").display()
        ))
    );
    assert_eq!(d["buttons"], json!(["Skip", "Overwrite"]));
    h.invoke("dialog.choose", json!({ "button": "Skip" }));
    assert_eq!(
        std::fs::read_to_string(h.root.join("docs/notes.txt")).unwrap(),
        "old\n"
    );
    h.invoke(
        "project.tree.copy-element",
        json!({ "paths": ["notes.txt"], "to": "docs" }),
    );
    h.invoke("dialog.confirm", json!(null));
    assert_eq!(
        std::fs::read_to_string(h.root.join("docs/notes.txt")).unwrap(),
        "n\n"
    );
}

#[gpui::test]
fn copy_paths_and_references(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("src/pkg/n.py");
    let keys = if cfg!(target_os = "macos") {
        "cmd-shift-c"
    } else {
        "ctrl-shift-c"
    };
    h.press(keys);
    assert_eq!(
        h.clipboard_text(),
        Some(
            h.root
                .join("src")
                .join("pkg")
                .join("n.py")
                .display()
                .to_string()
        )
    );
    h.invoke("project.tree.copy-path", json!({ "kind": "content-root" }));
    assert_eq!(h.clipboard_text().as_deref(), Some("src/pkg/n.py"));
    h.invoke("project.tree.copy-path", json!({ "kind": "file-name" }));
    assert_eq!(h.clipboard_text().as_deref(), Some("n.py"));
    // In the context menu.
    h.invoke("project.tree.context-menu", json!(null));
    let items = h.tree()["context_menu"]["levels"][0]["items"].clone();
    let copy_path = items
        .as_array()
        .unwrap()
        .iter()
        .position(|i| i["label"] == json!("Copy Path/Reference…"))
        .unwrap();
    while h.tree()["context_menu"]["levels"][0]["selected"] != json!(copy_path) {
        h.invoke("menu.down", json!(null));
    }
    h.invoke("menu.open-submenu", json!(null));
    let sub: Vec<String> = h.tree()["context_menu"]["levels"][1]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        sub,
        ["Absolute Path", "File Name", "Path from Content Root"]
    );
    h.invoke("menu.cancel", json!(null));
}

#[gpui::test]
fn dragging_rows_onto_a_folder_moves_them(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("src");
    h.invoke("project.tree.expand", json!(null));
    h.drag("notes.txt", "src/pkg", gpui::Modifiers::none());
    let t = h.transfer();
    assert_eq!(t["title"], json!("Move"));
    assert_eq!(
        t["directory"],
        json!(h.root.join("src/pkg").display().to_string())
    );
    h.press("enter");
    assert!(h.root.join("src/pkg/notes.txt").is_file());
    // With the copy modifier, onto a file: a copy into its folder.
    let modifiers = if cfg!(target_os = "macos") {
        gpui::Modifiers::alt()
    } else {
        gpui::Modifiers::control()
    };
    h.drag("app.py", "src/m.py", modifiers);
    let t = h.transfer();
    assert_eq!(t["title"], json!("Copy"));
    assert_eq!(
        t["directory"],
        json!(h.root.join("src").display().to_string())
    );
    h.press("escape");
    // Onto its own folder: nothing.
    h.drag("app.py", "", gpui::Modifiers::none());
    assert_eq!(h.transfer(), Value::Null);
}

#[gpui::test]
fn the_file_watcher_does_not_take_tree_operations_for_external_changes(cx: &mut TestAppContext) {
    support::allow_processes(cx);
    let mut h = setup_with(
        cx,
        SyncSettings {
            watch: true,
            ..SyncSettings::default()
        },
    );
    let window = h.window;
    h.wait("the watcher to start", |h| {
        window.read_with(h.cx, |w, _| w.is_watching()).unwrap()
    });
    // A renamed file with unsaved changes keeps its tab and its changes:
    // no tab closes, nothing reloads, no dialog asks.
    h.invoke("project.open-file", json!({ "path": "app.py" }));
    h.invoke("editor.type", json!({ "text": "x" }));
    h.invoke(
        "project.tree.rename",
        json!({ "path": "app.py", "name": "main.py" }),
    );
    h.settle("marker1.txt");
    let s = h.state();
    assert_eq!(s["dialog"], Value::Null, "{s:#}");
    assert_eq!(s["tabs"][0]["path"], json!("main.py"));
    assert_eq!(s["tabs"][0]["modified"], json!(true));
    assert_eq!(s["tabs"][0]["deleted"], json!(false));
    let text = |h: &mut H| {
        let editor = h
            .window
            .read_with(h.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .unwrap();
        h.cx.update(|cx| editor.read(cx).text())
    };
    assert_eq!(text(&mut h), "xa = 1\n");
    // Moved: the same.
    h.invoke(
        "project.tree.move",
        json!({ "paths": ["main.py"], "to": "src" }),
    );
    h.settle("marker2.txt");
    let s = h.state();
    assert_eq!(s["dialog"], Value::Null, "{s:#}");
    assert_eq!(h.tabs(), [("src/main.py".to_owned(), true)]);
    assert_eq!(text(&mut h), "xa = 1\n");
    // Saved under the new path, and the watcher stays quiet about it.
    h.invoke("workbench.save-all", json!(null));
    h.settle("marker3.txt");
    assert_eq!(
        std::fs::read_to_string(h.root.join("src/main.py")).unwrap(),
        "xa = 1\n"
    );
    assert_eq!(h.state()["dialog"], Value::Null);
    assert_eq!(h.tabs(), [("src/main.py".to_owned(), false)]);
    // The renamed file is still watched under its new path: a change made
    // outside reloads it.
    std::fs::write(h.root.join("src/main.py"), "y = 2\n").unwrap();
    h.wait("the renamed file to reload", |h| text(h) == "y = 2\n");
    // A deleted folder with an open file: its tab closes, no dialog.
    h.invoke("project.open-file", json!({ "path": "docs/index.md" }));
    h.invoke("project.tree.delete", json!({ "paths": ["docs"] }));
    h.invoke("dialog.confirm", json!(null));
    h.settle("marker4.txt");
    assert_eq!(h.state()["dialog"], Value::Null);
    assert_eq!(h.tabs(), [("src/main.py".to_owned(), false)]);
}

#[gpui::test]
fn reload_from_disk_looks_at_the_selection_again(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.invoke("project.open-file", json!({ "path": "src/m.py" }));
    h.select("src");
    h.invoke("project.tree.expand", json!(null));
    // Changed and created outside, with nothing watching.
    std::fs::write(h.root.join("src/m.py"), "m = 4\n").unwrap();
    std::fs::write(h.root.join("src/outside.py"), "").unwrap();
    assert!(!h.rows().contains(&"src/outside.py".to_owned()));
    // In the context menu after Delete…, before Mark Directory As.
    h.invoke("project.tree.context-menu", json!(null));
    let labels: Vec<String> = h.tree()["context_menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap_or("-").to_owned())
        .collect();
    let at = labels.iter().position(|l| l == "Reload from Disk").unwrap();
    assert_eq!(labels[at - 2], "Delete…");
    assert_eq!(labels[at + 2], "Mark Directory As");
    h.invoke("menu.cancel", json!(null));
    h.invoke("project.tree.reload-from-disk", json!(null));
    let s = h.state();
    assert!(
        s["tree"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["path"] == json!("src/outside.py")),
        "the folder is listed again"
    );
    assert_eq!(
        s["notice"],
        json!("File synchronization completed successfully.")
    );
    let editor = h
        .window
        .read_with(h.cx, |w, _| w.active_editor().cloned())
        .unwrap()
        .unwrap();
    assert_eq!(h.cx.update(|cx| editor.read(cx).text()), "m = 4\n");
    // A file elsewhere is not looked at.
    std::fs::write(h.root.join("app.py"), "a = 2\n").unwrap();
    h.invoke("project.open-file", json!({ "path": "app.py" }));
    std::fs::write(h.root.join("app.py"), "a = 3\n").unwrap();
    h.invoke("project.tree.reload-from-disk", json!({ "paths": ["src"] }));
    let editor = h
        .window
        .read_with(h.cx, |w, _| w.active_editor().cloned())
        .unwrap()
        .unwrap();
    assert_eq!(h.cx.update(|cx| editor.read(cx).text()), "a = 2\n");
    h.invoke(
        "project.tree.reload-from-disk",
        json!({ "paths": ["app.py"] }),
    );
    assert_eq!(h.cx.update(|cx| editor.read(cx).text()), "a = 3\n");
}

#[gpui::test]
fn the_drop_target_is_the_row_under_the_pointer(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.select("src");
    h.invoke("project.tree.expand", json!(null));
    let end = h.drag_over("notes.txt", "src/pkg");
    assert_eq!(h.tree()["drop_target"], json!("src/pkg"));
    // The dragged row's image follows the pointer.
    h.draw();
    assert!(
        gpui::VisualTestContext::from_window(h.window.into(), h.cx)
            .debug_bounds("tree-drag-image")
            .is_some()
    );
    // Onto a file: that row (its folder takes the files).
    let over_file = {
        let name = "tree-row:src/m.py";
        h.draw();
        gpui::VisualTestContext::from_window(h.window.into(), h.cx)
            .debug_bounds(name)
            .unwrap()
            .center()
    };
    gpui::VisualTestContext::from_window(h.window.into(), h.cx).simulate_mouse_move(
        over_file,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    h.draw();
    assert_eq!(h.tree()["drop_target"], json!("src/m.py"));
    gpui::VisualTestContext::from_window(h.window.into(), h.cx).simulate_mouse_up(
        end,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    h.draw();
    assert_eq!(h.tree()["drop_target"], Value::Null, "no drag: no target");
    h.press("escape");
    // A folder over itself or inside itself: no target.
    h.drag_over("src", "src/pkg");
    assert_eq!(h.tree()["drop_target"], Value::Null);
}
