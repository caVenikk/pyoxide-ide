//! Navigation, intention actions and formatting in the main window (ADR
//! 0010; tests/scenarios/python-intel/go-to-declaration.md,
//! quick-fixes.md, reformat-optimize-imports.md): Go to Declaration or
//! Usages (one target, several, none, the declaration itself, a library
//! file, Cmd/Ctrl+click and the link under the mouse), Show Usages, the
//! intention actions popup with quick fixes, sub-actions and speed search,
//! the bulb, edits of files that are not open, Reformat Code and Optimize
//! Imports. Driven against the in-memory `FakeCodeIntel`.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use code_intel::fake::{Call, FakeCodeIntel};
use code_intel::{
    CodeAction, CodeActionKind, CodeIntel, Diagnostic, DocumentId, FileChangeKind, FileEdits,
    IntelError, Location, Point, Severity, TextEdit, WorkspaceEdit,
};
use editor::{Editor, EditorEvent};
use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext,
    WindowHandle, point, px,
};
use serde_json::{Value, json};
use text::{Selection, SelectionSet};
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Product, Workbench};

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
    fake: Arc<FakeCodeIntel>,
    _dir: tempfile::TempDir,
}

impl<'a> H<'a> {
    fn new(cx: &'a mut TestAppContext, files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let fake = FakeCodeIntel::new();
        let product = Product {
            name: "Test IDE",
            file_types: vec![FileType {
                language_id: Some("python"),
                ..FileType::plain("Python", &["py", "pyi"], ui::icons::IconName::FILE_TEXT)
            }],
            code_intel: vec![CodeIntelHook::new(Provider(fake.clone()))],
            ..Product::default()
        };
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, Rc::new(product))
        });
        cx.run_until_parked();
        let mut h = Self {
            cx,
            window,
            root,
            fake,
            _dir: dir,
        };
        let path = h.root.display().to_string();
        h.invoke("project.open-folder", json!({ "path": path }));
        h
    }

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
        self.invoke("code.state", json!(null))
    }

    fn list(&mut self) -> Value {
        self.state()["list"].clone()
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(self.window.into(), text);
        self.draw();
    }

    fn wait(&mut self, ms: u64) {
        self.cx.executor().advance_clock(Duration::from_millis(ms));
        self.draw();
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn doc(&self, name: &str) -> DocumentId {
        DocumentId::new(self.path(name))
    }

    /// Opens a file with the caret right after `marker` (the editor has
    /// the focus).
    fn open(&mut self, name: &str, caret_after: &str) {
        let path = self.path(name).display().to_string();
        self.invoke("project.open-file", json!({ "path": path }));
        self.caret_after(caret_after);
        self.focus_editor();
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn offset_after(&mut self, marker: &str) -> usize {
        let editor = self.editor();
        self.cx.update(|cx| {
            let text = editor.read(cx).text();
            let byte = text.find(marker).expect("marker") + marker.len();
            text[..byte].chars().count()
        })
    }

    fn caret_after(&mut self, marker: &str) {
        let offset = self.offset_after(marker);
        let editor = self.editor();
        self.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                e.set_selections(SelectionSet::caret(offset), cx)
            })
        });
        self.draw();
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn editor_state(&mut self) -> Value {
        let editor = self.editor();
        self.cx.update(|cx| editor.read(cx).state_json())
    }

    fn text(&mut self) -> String {
        let editor = self.editor();
        self.cx.update(|cx| editor.read(cx).text())
    }

    /// The active file and the caret in it.
    fn at(&mut self) -> (String, u64, u64) {
        let state = self.editor_state();
        let path = PathBuf::from(state["path"].as_str().unwrap());
        (
            path.file_name().unwrap().to_string_lossy().into_owned(),
            state["caret"]["line"].as_u64().unwrap(),
            state["caret"]["column"].as_u64().unwrap(),
        )
    }

    fn emit(&mut self, event: EditorEvent) {
        let editor = self.editor();
        self.cx
            .update(|cx| editor.update(cx, |_, cx| cx.emit(event)));
        self.draw();
    }

    fn notice(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))["notice"].clone()
    }

    fn click(&mut self, selector: &'static str) {
        let bounds = VisualTestContext::from_window(self.window.into(), self.cx)
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} drawn"));
        let mut cx = VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_click(bounds.center(), Modifiers::default());
        self.draw();
    }

    fn drawn(&mut self, selector: &'static str) -> bool {
        VisualTestContext::from_window(self.window.into(), self.cx)
            .debug_bounds(selector)
            .is_some()
    }
}

fn location(path: PathBuf, line: usize, start: usize, end: usize) -> Location {
    Location {
        path,
        range: Point::new(line, start)..Point::new(line, end),
    }
}

const APP: &str = "from lib import norm\n\n\ndef g():\n    return norm(1)\n\n\nx = g()\n";
const LIB: &str = "def norm(x):\n    return x\n";

#[gpui::test]
fn go_to_declaration_in_the_file_and_in_another(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", APP), ("lib.py", LIB)]);
    let (app, lib) = (h.path("app.py"), h.path("lib.py"));
    h.fake.on_definition(move |_, at| {
        Ok(match at.line {
            4 => vec![location(lib.clone(), 0, 4, 8)],
            7 => vec![location(app.clone(), 3, 4, 5)],
            _ => Vec::new(),
        })
    });
    h.open("app.py", "x = g");
    h.invoke("code.go-to-declaration", json!(null));
    // Asked at the name before the caret; the caret goes to `g` in its
    // `def` line.
    assert!(h.fake.calls().contains(&Call::Definition {
        doc: h.doc("app.py"),
        at: Point::new(7, 4),
    }));
    assert_eq!(h.at(), ("app.py".into(), 3, 4));

    h.caret_after("return no");
    h.invoke("code.go-to-declaration", json!(null));
    assert_eq!(h.at(), ("lib.py".into(), 0, 4));
    // A file of the project opens editable.
    assert_eq!(h.editor_state()["read_only"], false);
}

#[gpui::test]
fn none_several_and_the_declaration_itself(cx: &mut TestAppContext) {
    let mut h = H::new(
        cx,
        &[
            ("app.py", APP),
            ("lib.py", LIB),
            ("other.py", "from lib import norm\nnorm(2)\n"),
        ],
    );
    let (app, lib, other) = (h.path("app.py"), h.path("lib.py"), h.path("other.py"));
    let lib2 = lib.clone();
    h.fake.on_definition(move |doc, at| {
        Ok(match (doc.path().ends_with("lib.py"), at.line) {
            // On `norm` in its own `def` line.
            (true, 0) => vec![location(lib2.clone(), 0, 4, 8)],
            (false, 4) => vec![
                location(lib2.clone(), 0, 4, 8),
                location(app.clone(), 0, 16, 20),
            ],
            _ => Vec::new(),
        })
    });
    h.fake.on_references(move |_, _, include_declaration| {
        assert!(!include_declaration);
        Ok(vec![
            location(other.clone(), 1, 0, 4),
            location(h_app(&other), 4, 11, 15),
        ])
    });

    // No declaration: a hint at the caret.
    h.open("app.py", "x = ");
    h.invoke("code.go-to-declaration", json!(null));
    assert_eq!(h.state()["hint"], "Cannot find declaration to go to");

    // Two: a chooser; the second one is chosen with the keys.
    h.caret_after("return no");
    h.invoke("code.go-to-declaration", json!(null));
    let list = h.list();
    assert_eq!(list["name"], "declarations");
    assert_eq!(list["title"], "Choose Declaration");
    assert_eq!(list["items"][0]["text"], "def norm(x):");
    assert_eq!(list["items"][0]["detail"], "lib.py:1");
    assert_eq!(list["items"][1]["text"], "from lib import norm");
    h.press("down");
    h.press("enter");
    assert!(h.list().is_null());
    assert_eq!(h.at(), ("app.py".into(), 0, 16));

    // On the declaration itself: its usages, in a popup.
    h.open("lib.py", "def no");
    h.invoke("code.go-to-declaration", json!(null));
    let list = h.list();
    assert_eq!(list["name"], "usages");
    assert_eq!(list["title"], "Usages of norm");
    assert_eq!(list["subtitle"], "2 usages");
    // Rows of code (a file that is not open is read from disk), with the
    // file and line; sorted by file and position.
    let rows: Vec<(String, String)> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["text"].as_str().unwrap().to_owned(),
                r["detail"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("return norm(1)".to_owned(), "app.py:5".to_owned()),
            ("norm(2)".to_owned(), "other.py:2".to_owned()),
        ]
    );
    assert!(h.drawn("usages-popup"));
    // Escape closes it; the editor has the focus again.
    h.press("escape");
    assert!(h.list().is_null());
    h.press("right");
    assert_eq!(h.at(), ("lib.py".into(), 0, 7));
    // Show Usages directly; Enter opens the selected usage.
    h.invoke("code.show-usages", json!(null));
    h.press("enter");
    assert_eq!(h.at(), ("app.py".into(), 4, 11));
}

/// `app.py` next to another file of the project.
fn h_app(other: &Path) -> PathBuf {
    other.with_file_name("app.py")
}

#[gpui::test]
fn library_files_open_read_only(cx: &mut TestAppContext) {
    let library = tempfile::tempdir().unwrap();
    let stub = native_path::canonicalize(library.path())
        .unwrap()
        .join("json/__init__.pyi");
    std::fs::create_dir_all(stub.parent().unwrap()).unwrap();
    std::fs::write(&stub, "def dumps(obj) -> str: ...\n").unwrap();
    let mut h = H::new(cx, &[("app.py", "import json\njson.dumps(1)\n")]);
    let target = stub.clone();
    h.fake
        .on_definition(move |_, _| Ok(vec![location(target.clone(), 0, 4, 9)]));
    h.open("app.py", "json.du");
    h.invoke("code.go-to-declaration", json!(null));
    assert_eq!(h.at(), ("__init__.pyi".into(), 0, 4));
    assert_eq!(h.editor_state()["read_only"], true);
}

#[gpui::test]
fn the_navigation_modifier_click_and_link(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", APP), ("lib.py", LIB)]);
    let lib = h.path("lib.py");
    h.fake.on_definition(move |_, at| {
        Ok(if at.line == 4 {
            vec![location(lib.clone(), 0, 4, 8)]
        } else {
            Vec::new()
        })
    });
    h.open("app.py", "x = ");
    let norm = h.offset_after("return no");
    let modifier = Modifiers::secondary_key();
    // The mouse over `norm` with the modifier: a link over the name.
    h.emit(EditorEvent::Hovered {
        offset: Some(norm),
        position: point(px(0.0), px(0.0)),
        modifiers: modifier,
    });
    let editor = h.editor();
    let link = h.cx.update(|cx| editor.read(cx).link_highlight());
    assert_eq!(link, Some(norm - 2..norm + 2));
    // Without a declaration, or without the modifier: no link.
    let x = h.offset_after("x");
    h.emit(EditorEvent::Hovered {
        offset: Some(x),
        position: point(px(0.0), px(0.0)),
        modifiers: modifier,
    });
    assert_eq!(h.cx.update(|cx| editor.read(cx).link_highlight()), None);
    h.emit(EditorEvent::Hovered {
        offset: Some(norm),
        position: point(px(0.0), px(0.0)),
        modifiers: modifier,
    });
    h.emit(EditorEvent::Hovered {
        offset: Some(norm),
        position: point(px(0.0), px(0.0)),
        modifiers: Modifiers::default(),
    });
    assert_eq!(h.cx.update(|cx| editor.read(cx).link_highlight()), None);
    // A click with the modifier goes to the declaration.
    h.emit(EditorEvent::ModifiedClick {
        offset: norm,
        modifiers: modifier,
    });
    assert_eq!(h.at(), ("lib.py".into(), 0, 4));
}

/// `import os` with an unused-import warning; the actions: organize
/// imports, a fix with two sub-actions, fix all.
fn intentions_fixture(h: &mut H<'_>) {
    let doc = h.doc("app.py");
    let mut unused = Diagnostic::new(
        Point::new(0, 7)..Point::new(0, 9),
        Severity::Warning,
        "`os` imported but unused",
    );
    unused.source = Some("Ruff".into());
    h.fake.publish_diagnostics(&doc, 1, vec![unused]);
    let path = doc.path().to_owned();
    h.fake.on_code_actions(move |_, range, context| {
        let edit = |edits| {
            Some(WorkspaceEdit {
                files: vec![FileEdits {
                    path: path.clone(),
                    edits,
                }],
            })
        };
        let mut fix = CodeAction::new(
            "Ruff (unused-import): Remove unused import: `os`",
            CodeActionKind::QuickFix,
        );
        fix.edit = edit(vec![TextEdit::new(Point::new(0, 0)..Point::new(1, 0), "")]);
        fix.diagnostics = context.diagnostics.clone();
        let mut for_file = CodeAction::new("Disable for this file", CodeActionKind::QuickFix);
        for_file.edit = edit(vec![TextEdit::insert(
            Point::new(0, 0),
            "# ruff: noqa: F401\n",
        )]);
        let mut for_line = CodeAction::new("Disable for this line", CodeActionKind::QuickFix);
        for_line.edit = edit(vec![TextEdit::insert(Point::new(0, 9), "  # noqa: F401")]);
        fix.children = vec![for_file, for_line];
        let on_os = range.start.line == 0;
        let mut actions = vec![
            CodeAction::new("Ruff: Organize imports", CodeActionKind::organize_imports()),
            CodeAction::new(
                "Ruff: Fix all auto-fixable problems",
                CodeActionKind::Source("fixAll.ruff".into()),
            ),
        ];
        if on_os {
            actions.insert(1, fix);
        }
        Ok(actions)
    });
}

#[gpui::test]
fn the_intention_actions_popup(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import os\nx = 1\n")]);
    h.open("app.py", "import o");
    intentions_fixture(&mut h);
    h.draw();
    h.invoke("code.show-intentions", json!(null));
    let list = h.list();
    assert_eq!(list["name"], "intentions");
    // The quick fix first with a bulb, a separator, the others by name
    // without an icon; the first row selected.
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(
        items[0]["text"],
        "Ruff (unused-import): Remove unused import: `os`"
    );
    assert_eq!(items[0]["icon"], "codeInsight/intentionBulb");
    assert_eq!(
        items[0]["children"],
        json!(["Disable for this file", "Disable for this line"])
    );
    assert_eq!(items[1]["separator"], true);
    assert_eq!(items[2]["text"], "Ruff: Fix all auto-fixable problems");
    assert!(items[2]["icon"].is_null());
    assert_eq!(items[3]["text"], "Ruff: Organize imports");
    assert_eq!(list["selected"], items[0]["text"]);
    assert!(h.drawn("intentions-popup"));
    assert!(h.drawn("intentions-more"), "the selected row's button");
    // The problem goes with the request.
    let context = h
        .fake
        .calls()
        .into_iter()
        .rev()
        .find_map(|c| match c {
            Call::CodeActions { context, .. } => Some(context),
            _ => None,
        })
        .unwrap();
    assert_eq!(context.diagnostics.len(), 1);

    // Right opens the sub-actions, Left closes them.
    h.press("right");
    assert_eq!(h.list()["sub"]["selected"], "Disable for this file");
    h.press("left");
    assert!(h.list()["sub"].is_null());
    // Enter applies the selected fix as one undo step.
    h.press("enter");
    assert!(h.list().is_null());
    assert_eq!(h.text(), "x = 1\n");
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "import os\nx = 1\n");

    // A sub-action, from the `⋮` button.
    h.caret_after("import o");
    h.invoke("code.show-intentions", json!(null));
    h.click("intentions-more");
    assert_eq!(h.list()["sub"]["selected"], "Disable for this file");
    h.press("down");
    h.press("enter");
    assert_eq!(h.text(), "import os  # noqa: F401\nx = 1\n");

    // Speed search: typing filters, Escape clears, then closes.
    h.invoke("code.show-intentions", json!(null));
    h.type_text("org");
    let list = h.list();
    assert_eq!(list["filter"], "org");
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["selected"], "Ruff: Organize imports");
    h.press("escape");
    assert_eq!(h.list()["filter"], "");
    h.press("escape");
    assert!(h.list().is_null());
    assert_eq!(h.text(), "import os  # noqa: F401\nx = 1\n");

    // On a line without problems: the intentions only.
    h.caret_after("x = 1");
    h.invoke("code.show-intentions", json!(null));
    let items = h.list()["items"].clone();
    assert_eq!(
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["text"].clone())
            .collect::<Vec<_>>(),
        [
            json!("Ruff: Fix all auto-fixable problems"),
            json!("Ruff: Organize imports")
        ]
    );
}

#[gpui::test]
fn the_bulb_shows_that_actions_are_there(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import os\nx = 1\n")]);
    h.open("app.py", "x = 1");
    intentions_fixture(&mut h);
    h.caret_after("import o");
    assert!(h.state()["bulb"].is_null());
    h.wait(300);
    let bulb = h.state()["bulb"].clone();
    assert_eq!(bulb["line"], 0);
    assert_eq!(bulb["icon"], "codeInsight/intentionBulb");
    assert!(h.drawn("intention-bulb"));
    // Another line: the bulb goes, then comes back when its actions are
    // known.
    h.press("down");
    assert!(h.state()["bulb"].is_null());
    h.wait(300);
    assert_eq!(h.state()["bulb"]["line"], 1);
    // The bulb is drawn while the editor has the focus (GPUI keeps the
    // debug bounds of earlier frames, so only its return is checked).
    h.invoke("workbench.toggle-project", json!(null));
    h.invoke("workbench.focus-editor", json!(null));
    h.state();
    assert!(h.drawn("intention-bulb"));
    // A click on it opens the list.
    h.click("intention-bulb");
    assert_eq!(h.list()["name"], "intentions");
}

#[gpui::test]
fn edits_of_files_that_are_not_documents(cx: &mut TestAppContext) {
    let outside = tempfile::tempdir().unwrap();
    let outside = native_path::canonicalize(outside.path())
        .unwrap()
        .join("pyproject.toml");
    let mut h = H::new(
        cx,
        &[
            ("app.py", "import os\n"),
            ("pyproject.toml", "[tool.ruff.lint]\nignore = []\n"),
            ("notes.txt", "notes\n"),
        ],
    );
    // A tab that is no document.
    h.invoke(
        "project.open-file",
        json!({ "path": h.path("notes.txt").display().to_string() }),
    );
    h.open("app.py", "import os");
    let (pyproject, ruff, notes, app) = (
        h.path("pyproject.toml"),
        h.path("sub/ruff.toml"),
        h.path("notes.txt"),
        h.path("app.py"),
    );
    let outside2 = outside.clone();
    h.fake.on_code_actions(move |_, _, _| {
        let mut action = CodeAction::new("Disable for this project", CodeActionKind::QuickFix);
        action.edit = Some(WorkspaceEdit {
            files: vec![
                FileEdits {
                    path: pyproject.clone(),
                    edits: vec![TextEdit::new(
                        Point::new(1, 9)..Point::new(1, 11),
                        "[\"F401\"]",
                    )],
                },
                FileEdits {
                    path: ruff.clone(),
                    edits: vec![TextEdit::insert(Point::new(0, 0), "[lint]\n")],
                },
                FileEdits {
                    path: notes.clone(),
                    edits: vec![TextEdit::insert(Point::new(0, 0), "more ")],
                },
                FileEdits {
                    path: app.clone(),
                    edits: vec![TextEdit::insert(Point::new(0, 9), "  # fixed")],
                },
                FileEdits {
                    path: outside2.clone(),
                    edits: vec![TextEdit::insert(Point::new(0, 0), "x")],
                },
            ],
        });
        Ok(vec![action])
    });
    h.invoke("code.show-intentions", json!(null));
    h.press("enter");
    // The document and the open tab change in their editors.
    assert_eq!(h.text(), "import os  # fixed\n");
    // Files that are not open change on disk, a new one is created; the
    // services hear of both.
    assert_eq!(
        std::fs::read_to_string(h.path("pyproject.toml")).unwrap(),
        "[tool.ruff.lint]\nignore = [\"F401\"]\n"
    );
    assert_eq!(
        std::fs::read_to_string(h.path("sub/ruff.toml")).unwrap(),
        "[lint]\n"
    );
    let changed: Vec<(PathBuf, FileChangeKind)> = h
        .fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::FilesChanged(changes) => Some(changes),
            _ => None,
        })
        .flatten()
        .map(|c| (c.path, c.kind))
        .collect();
    assert!(changed.contains(&(h.path("pyproject.toml"), FileChangeKind::Changed)));
    assert!(changed.contains(&(h.path("sub/ruff.toml"), FileChangeKind::Created)));
    // Nothing outside the project.
    assert!(!outside.exists());
    assert!(
        h.notice().as_str().unwrap().contains("outside the project"),
        "{}",
        h.notice()
    );
    h.invoke(
        "project.open-file",
        json!({ "path": h.path("notes.txt").display().to_string() }),
    );
    assert_eq!(h.text(), "more notes\n");
}

#[gpui::test]
fn reformat_code_and_optimize_imports(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import sys\nimport os\nx=1\ny=2\n")]);
    h.open("app.py", "x=1");
    // Without a formatter: a notice.
    h.invoke("code.reformat", json!(null));
    assert_eq!(h.notice(), "No formatter is available for this file");
    h.fake.on_format(|_, range, options| {
        assert_eq!(options.tab_size, 4);
        Ok(match range {
            None => vec![
                TextEdit::new(Point::new(2, 1)..Point::new(2, 2), " = "),
                TextEdit::new(Point::new(3, 1)..Point::new(3, 2), " = "),
            ],
            Some(r) => vec![TextEdit::new(
                Point::new(r.start.line, 1)..Point::new(r.start.line, 2),
                " = ",
            )],
        })
    });
    // The whole file, one undo step.
    h.invoke("code.reformat", json!(null));
    assert_eq!(h.text(), "import sys\nimport os\nx = 1\ny = 2\n");
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "import sys\nimport os\nx=1\ny=2\n");
    // A selection: range formatting.
    let start = h.offset_after("x=1\n");
    let editor = h.editor();
    h.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            e.set_selections(SelectionSet::single(Selection::new(start, start + 3)), cx)
        })
    });
    h.draw();
    h.invoke("code.reformat", json!(null));
    assert_eq!(h.text(), "import sys\nimport os\nx=1\ny = 2\n");
    let ranges: Vec<Option<std::ops::Range<Point>>> = h
        .fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Format { range, .. } => Some(range),
            _ => None,
        })
        .collect();
    assert_eq!(
        ranges.last().unwrap(),
        &Some(Point::new(3, 0)..Point::new(3, 3))
    );

    // Optimize Imports: the organize imports action, one undo step.
    let path = h.path("app.py");
    h.fake.on_code_actions(move |_, _, context| {
        if !context.wants(&CodeActionKind::organize_imports()) || context.only.is_none() {
            return Err(IntelError::Unsupported);
        }
        let mut organize = CodeAction::new(
            "Organize imports",
            CodeActionKind::parse("source.organizeImports.ruff"),
        );
        organize.edit = Some(WorkspaceEdit {
            files: vec![FileEdits {
                path: path.clone(),
                edits: vec![TextEdit::new(
                    Point::new(0, 0)..Point::new(2, 0),
                    "import os\nimport sys\n",
                )],
            }],
        });
        Ok(vec![organize])
    });
    h.invoke("code.optimize-imports", json!(null));
    assert_eq!(h.text(), "import os\nimport sys\nx=1\ny = 2\n");
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "import sys\nimport os\nx=1\ny = 2\n");
}
