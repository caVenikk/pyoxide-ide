//! Find in Files (tests/scenarios/editor/find-in-files.md), driven by real
//! keystrokes through both default keymaps in a headless main window over
//! a project on disk.

use crate::mouse;

use std::rc::Rc;
use std::time::Duration;

use crate::mouse::{drag_to, hover, press, release};
use editor::Editor;
use gpui::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, WindowHandle, point, px,
};
use serde_json::{Value, json};
use text::{Buffer, Selection, SelectionSet};
use workbench::find_in_files::{DEBOUNCE, FindInFiles};
use workbench::keymap::{self, Keymap};
use workbench::{Product, Workbench};

/// The keys of one default keymap.
struct Keys {
    open: &'static str,
    next: &'static str,
    previous: &'static str,
    match_case: &'static str,
    words: &'static str,
    mask: &'static str,
    new_line: &'static str,
    select_all: &'static str,
    replace: &'static str,
    replace_all: &'static str,
    undo: &'static str,
    scope_project: &'static str,
    scope_module: &'static str,
    scope_directory: &'static str,
    scope_named: &'static str,
    recursive: &'static str,
}

const MAC: Keys = Keys {
    open: "cmd-shift-f",
    next: "cmd-g",
    previous: "cmd-shift-g",
    match_case: "alt-c",
    words: "alt-w",
    mask: "alt-k",
    new_line: "cmd-shift-enter",
    select_all: "cmd-a",
    replace: "cmd-shift-r",
    replace_all: "alt-shift-enter",
    undo: "cmd-z",
    scope_project: "alt-p",
    scope_module: "alt-m",
    scope_directory: "alt-d",
    scope_named: "alt-s",
    // The reference binds no Option+Y here.
    recursive: "ctrl-alt-y",
};

const WIN: Keys = Keys {
    open: "ctrl-shift-f",
    next: "f3",
    previous: "shift-f3",
    match_case: "alt-c",
    words: "alt-w",
    mask: "alt-k",
    new_line: "ctrl-shift-enter",
    select_all: "ctrl-a",
    replace: "ctrl-shift-r",
    replace_all: "alt-shift-enter",
    undo: "ctrl-z",
    scope_project: "alt-p",
    scope_module: "alt-m",
    scope_directory: "alt-d",
    scope_named: "alt-s",
    recursive: "alt-y",
};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    keys: &'static Keys,
    root: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    /// Lets the search start after its delay and finish.
    fn settle(&mut self) {
        self.cx.executor().advance_clock(DEBOUNCE * 2);
        self.draw();
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

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(self.window.into(), text);
        self.draw();
    }

    fn state(&mut self) -> Value {
        self.invoke("find-in-files.state", json!(null))
    }

    fn rows(&mut self) -> Vec<(String, u64)> {
        self.state()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["path"].as_str().unwrap().to_owned(),
                    r["line"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    fn popup(&mut self) -> Entity<FindInFiles> {
        self.window
            .read_with(self.cx, |w, _| w.find_in_files().clone())
            .unwrap()
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn focused(&mut self, field: fn(&FindInFiles) -> &Entity<Editor>) -> bool {
        let popup = self.popup();
        let field = self.cx.update(|cx| field(popup.read(cx)).clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self.editor();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    /// Selects `range` (chars) in the active editor and focuses it.
    fn select(&mut self, range: std::ops::Range<usize>) {
        let editor = self.editor();
        self.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                e.set_selections(
                    SelectionSet::single(Selection::new(range.start, range.end)),
                    cx,
                )
            })
        });
        self.focus_editor();
    }

    fn caret(&mut self) -> (String, usize, usize) {
        let s = self.invoke("editor.state", json!(null));
        let path = s["path"].as_str().unwrap_or_default().to_owned();
        // Both with `/` separators (the state reports them so).
        let root = format!("{}/", self.root.display()).replace('\\', "/");
        let path = path.replace('\\', "/");
        let rel = path.strip_prefix(&root).unwrap_or(&path).to_owned();
        let editor = self.editor();
        let p = self.cx.update(|cx| editor.read(cx).caret_point());
        (rel, p.line, p.column)
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str, keys: &'static Keys) -> H<'a> {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    for (path, text) in [
        (
            "app.py",
            "import os\n\ndef main():\n    print(\"needle\")\n",
        ),
        ("notes.txt", "a needle and a Needle\n"),
        ("pkg/util.py", "x = 1\n\n\n# needle here\nneedle = 2\n"),
        (".git/config", "needle\n"),
        ("pkg/__pycache__/util.cpython-313.pyc", "needle\n"),
        (".idea/workspace.xml", "<needle/>\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        find_file_masks: vec!["*.py", "*.xml"],
        ..Product::default()
    };
    let keymap = Keymap::parse(keymap_text).unwrap();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        // Each run starts a new session.
        cx.set_global(editor::find::LastSearch::default());
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &keymap);
        assert!(errors.is_empty(), "keymap errors: {errors:?}");
        workbench::open_main_window(cx, Rc::new(product))
    });
    // The popup closes when the focus leaves it in an active window.
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    let mut h = H {
        cx,
        window,
        keys,
        root: root.clone(),
        _dir: dir,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    h.invoke("project.open-file", json!({ "path": "app.py" }));
    h
}

fn both(cx: &mut TestAppContext, test: fn(&mut H)) {
    for (text, keys) in [(keymap::MACOS, &MAC), (keymap::DEFAULT, &WIN)] {
        let mut h = setup(cx, text, keys);
        test(&mut h);
    }
}

#[gpui::test]
fn the_selection_is_searched_and_enter_opens_the_match(cx: &mut TestAppContext) {
    both(cx, |h| {
        // `needle` on line 4 of app.py is selected.
        h.select(34..40);
        h.press(h.keys.open);
        let s = h.state();
        assert_eq!(s["open"], json!(true));
        assert_eq!(s["text"], json!("needle"));
        assert!(h.focused(FindInFiles::query_field));
        // First run: the file mask is on with the product's first mask.
        assert_eq!(s["mask"], json!({ "on": true, "text": "*.py" }));
        h.settle();
        assert_eq!(h.state()["info"], json!("3 matches in 2 files"));
        // Off, every file is searched; the focus goes back to the field.
        h.press(h.keys.mask);
        assert!(h.focused(FindInFiles::query_field));
        h.settle();
        // Hidden folders, byte code and the reference's project files are
        // left out; the first file found stays first, the others follow by
        // path; one row per line.
        let rows = h.rows();
        assert_eq!(rows.len(), 4, "{rows:?}");
        let first = rows[0].0.clone();
        let rest: Vec<_> = rows.iter().filter(|r| r.0 != first).cloned().collect();
        let mut sorted = rest.clone();
        sorted.sort();
        assert_eq!(rest, sorted, "by path and line after the first file");
        assert!(rows[..rows.len() - rest.len()].iter().all(|r| r.0 == first));
        for row in [("notes.txt", 1), ("pkg/util.py", 4), ("pkg/util.py", 5)] {
            assert!(rows.contains(&(row.0.to_owned(), row.1)), "{row:?}");
        }
        let s = h.state();
        assert_eq!(s["info"], json!("5 matches in 3 files"));
        assert_eq!(s["selected"], json!(0));
        // Down and Up move through the rows; the focus stays in the field.
        h.press("down");
        h.press("down");
        assert_eq!(h.state()["selected"], json!(2));
        h.press("up");
        assert_eq!(h.state()["selected"], json!(1));
        h.press(h.keys.next);
        h.press(h.keys.next);
        h.press(h.keys.next);
        assert_eq!(h.state()["selected"], json!(3), "not past the last row");
        h.press(h.keys.previous);
        assert_eq!(h.state()["selected"], json!(2));
        assert!(h.focused(FindInFiles::query_field));
        // Enter opens the file at the match and closes the popup.
        let (path, line) = h.rows()[2].clone();
        h.press("enter");
        assert_eq!(h.state()["open"], json!(false));
        let (opened, caret_line, column) = h.caret();
        assert_eq!(opened, path);
        assert_eq!(caret_line as u64 + 1, line);
        let expected_column = match (path.as_str(), line) {
            ("pkg/util.py", 4) => 2,
            ("notes.txt", 1) => 2,
            ("app.py", 4) => 11,
            _ => 0,
        };
        assert_eq!(column, expected_column);
        assert!(h.editor_focused());
        // The search is in the shared history: it fills the field next time.
        h.focus_editor();
        h.press(h.keys.open);
        assert_eq!(h.state()["text"], json!("needle"));
        h.press("escape");
        assert_eq!(h.state()["open"], json!(false));
        assert!(h.editor_focused(), "Escape gives the focus back");
    });
}

#[gpui::test]
fn typing_searches_after_a_pause_and_options_apply(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.focus_editor();
        h.press(h.keys.open);
        let s = h.state();
        assert_eq!(s["text"], json!(""));
        // An empty query: no results, the placeholder, no counter.
        assert_eq!(s["empty_text"], json!("Type search query to find in files"));
        assert_eq!(s["info"], json!(""));
        assert_eq!(s["rows"], json!([]));
        assert_eq!(s["preview"], Value::Null);
        h.press(h.keys.mask);
        h.focus_field(FindInFiles::query_field);
        h.type_text("Needle");
        // Nothing until the pause.
        assert_eq!(h.state()["searching"], json!(true));
        h.settle();
        assert_eq!(h.state()["matches"], json!(5), "case-insensitive");
        h.press(h.keys.match_case);
        assert_eq!(h.state()["case_sensitive"], json!(true));
        h.settle();
        assert_eq!(h.rows(), [("notes.txt".to_owned(), 1)]);
        assert_eq!(h.state()["info"], json!("1 match in 1 file"));
        h.press(h.keys.match_case);
        h.press(h.keys.words);
        h.settle();
        assert_eq!(h.state()["matches"], json!(5));
        h.press(h.keys.words);
        // Over two lines.
        h.press(h.keys.select_all);
        h.type_text("here");
        h.press(h.keys.new_line);
        h.type_text("needle");
        h.settle();
        let s = h.state();
        assert_eq!(s["text"], json!("here\nneedle"));
        assert_eq!(h.rows(), [("pkg/util.py".to_owned(), 4)]);
        // A bad pattern says so.
        h.invoke(
            "find-in-files.search",
            json!({ "text": "(", "regex": true, "mask": "" }),
        );
        let s = h.state();
        assert!(
            s["empty_text"]
                .as_str()
                .unwrap()
                .starts_with("Bad pattern \"(\""),
            "{s}"
        );
        h.invoke(
            "find-in-files.search",
            json!({ "text": "a*", "regex": true }),
        );
        assert_eq!(
            h.state()["empty_text"],
            json!("Regular expression matches empty string")
        );
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn the_file_mask_narrows_the_files_and_is_remembered(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "case_sensitive": false, "regex": false, "whole_words": false }),
        );
        let s = h.state();
        assert_eq!(s["mask"], json!({ "on": true, "text": "*.py" }));
        assert_eq!(s["files"], json!(2));
        // Alt+K: off, every file.
        h.press(h.keys.mask);
        h.settle();
        assert_eq!(h.state()["mask"]["on"], json!(false));
        assert_eq!(h.state()["files"], json!(3));
        // Alt+K checks the box again and puts the focus in the mask.
        h.press(h.keys.mask);
        assert!(h.focused(FindInFiles::mask_field));
        h.settle();
        assert_eq!(h.state()["mask"]["on"], json!(true));
        assert_eq!(h.state()["files"], json!(2));
        h.type_text("*.txt, !app*");
        h.settle();
        assert_eq!(h.rows(), [("notes.txt".to_owned(), 1)]);
        h.press("escape");
        // Remembered: the mask is on with its text next time.
        let settings = h.cx.update(|cx| workbench::find_in_files::settings(cx));
        assert_eq!(settings.file_mask.as_deref(), Some("*.txt, !app*"));
        assert_eq!(settings.recent_masks[0], "*.txt, !app*");
        h.press(h.keys.open);
        let s = h.state();
        assert_eq!(s["mask"], json!({ "on": true, "text": "*.txt, !app*" }));
        h.settle();
        assert_eq!(h.state()["files"], json!(1));
        // Off again; the focus goes back to the search field.
        h.press(h.keys.mask);
        assert!(h.focused(FindInFiles::query_field));
        h.settle();
        assert_eq!(h.state()["files"], json!(3));
        h.press("escape");
        let settings = h.cx.update(|cx| workbench::find_in_files::settings(cx));
        assert_eq!(settings.file_mask, None);
    });
}

#[gpui::test]
fn unsaved_changes_are_searched_and_the_count_stops_at_the_limit(cx: &mut TestAppContext) {
    both(cx, |h| {
        // An unsaved edit of the open file wins over the file on disk.
        let editor = h.editor();
        h.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                *e.buffer_mut() = Buffer::new("one\ntwo haystack\n");
                e.buffer_reloaded(cx);
                e.insert("x", cx);
            })
        });
        h.invoke(
            "find-in-files.search",
            json!({ "text": "haystack", "regex": false, "mask": "" }),
        );
        assert_eq!(h.rows(), [("app.py".to_owned(), 2)]);
        // 150 matches: the search stops at 100 and the counter says so.
        let many: String = (0..150).map(|i| format!("hay {i}\n")).collect();
        std::fs::write(h.root.join("many.txt"), many).unwrap();
        h.invoke("find-in-files.search", json!({ "text": "hay " }));
        let s = h.state();
        assert_eq!(s["matches"], json!(100));
        assert_eq!(s["truncated"], json!(true));
        assert_eq!(s["info"], json!("100+ matches in 1+ file"));
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn the_popup_closes_when_the_focus_leaves_unless_pinned(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        h.focus_editor();
        assert_eq!(h.state()["open"], json!(false));
        // Pinned: it stays open, also after opening a result.
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        h.invoke("find-in-files.toggle-pin", json!(null));
        assert_eq!(h.state()["pinned"], json!(true));
        h.focus_editor();
        assert_eq!(h.state()["open"], json!(true));
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        h.press("enter");
        assert_eq!(h.state()["open"], json!(true));
        assert!(h.editor_focused());
        // Escape closes it all the same.
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        h.press("escape");
        assert_eq!(h.state()["open"], json!(false));
        let settings = h.cx.update(|cx| workbench::find_in_files::settings(cx));
        assert!(settings.pinned, "the pin is remembered");
    });
}

impl H<'_> {
    /// A scroll event over the results (negative scrolls down), then the
    /// frames that follow (loading more starts after the frame that shows
    /// the end).
    fn scroll_results(&mut self, delta: ScrollDelta) {
        let bounds = VisualTestContext::from_window(self.window.into(), self.cx)
            .debug_bounds("find-in-files-rows")
            .expect("the results are drawn");
        VisualTestContext::from_window(self.window.into(), self.cx).simulate_event(
            ScrollWheelEvent {
                position: bounds.center(),
                delta,
                modifiers: Modifiers::none(),
                touch_phase: TouchPhase::Moved,
            },
        );
        for _ in 0..4 {
            self.draw();
        }
    }
}

#[gpui::test]
fn scrolling_to_the_end_loads_more(cx: &mut TestAppContext) {
    both(cx, |h| {
        let many: String = (0..160).map(|i| format!("hay {i}\n")).collect();
        std::fs::write(h.root.join("many.txt"), many).unwrap();
        h.invoke(
            "find-in-files.search",
            json!({ "text": "hay ", "mask": "" }),
        );
        let s = h.state();
        assert_eq!(s["matches"], json!(100));
        assert_eq!(s["info"], json!("100+ matches in 1+ file"));
        let first = s["rows"][0].clone();
        // A trackpad swipe scrolls without moving the selection.
        h.scroll_results(ScrollDelta::Pixels(point(px(0.0), px(-30.0))));
        let s = h.state();
        assert!(
            s["scroll_top"].as_f64().unwrap() > 0.0,
            "{}",
            s["scroll_top"]
        );
        assert_eq!(s["selected"], json!(0));
        assert_eq!(s["matches"], json!(100), "not near the end yet");
        // The wheel to the end: 50 more, the rows on screen stay.
        h.scroll_results(ScrollDelta::Lines(point(0.0, -200.0)));
        let s = h.state();
        assert_eq!(s["matches"], json!(150));
        assert_eq!(s["rows"][0], first);
        assert_eq!(s["info"], json!("150+ matches in 1+ file"));
        // Again: the rest, and the search is complete.
        h.scroll_results(ScrollDelta::Lines(point(0.0, -200.0)));
        let s = h.state();
        assert_eq!(s["matches"], json!(160));
        assert_eq!(s["truncated"], json!(false));
        assert_eq!(s["info"], json!("160 matches in 1 file"));
        h.invoke("find-in-files.close", json!(null));
    });
}

impl H<'_> {
    fn bounds_of(&mut self, selector: &'static str) -> gpui::Bounds<gpui::Pixels> {
        VisualTestContext::from_window(self.window.into(), self.cx)
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
    }

    /// Drags with the left button from `from` to `to`.
    fn drag(&mut self, from: gpui::Point<gpui::Pixels>, to: gpui::Point<gpui::Pixels>) {
        let mut vcx = VisualTestContext::from_window(self.window.into(), self.cx);
        vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear());
        vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        self.draw();
    }

    fn focus_field(&mut self, field: fn(&FindInFiles) -> &Entity<Editor>) {
        let popup = self.popup();
        let handle = self.cx.update(|cx| field(popup.read(cx)).focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn message(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))["message"].clone()
    }

    fn write(&mut self, path: &str, text: &[u8]) {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&mut self, path: &str) -> Vec<u8> {
        std::fs::read(self.root.join(path)).unwrap()
    }
}

#[gpui::test]
fn the_directory_scope_with_and_without_subfolders(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.write("pkg/sub/deep.py", b"needle\n");
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        // Option+D / Alt+D: the Directory tab, the focus in its field.
        h.press(h.keys.scope_directory);
        let s = h.state();
        assert_eq!(s["scope"], json!("directory"));
        assert!(h.focused(FindInFiles::directory_field));
        let pkg = h.root.join("pkg").display().to_string();
        h.type_text(&pkg);
        h.settle();
        let files: Vec<String> = h.rows().into_iter().map(|r| r.0).collect();
        assert!(files.contains(&"pkg/sub/deep.py".to_owned()), "{files:?}");
        assert!(files.iter().all(|f| f.starts_with("pkg/")), "{files:?}");
        // Option+Y / Alt+Y: not into subfolders.
        h.press(h.keys.recursive);
        h.settle();
        let s = h.state();
        assert_eq!(s["recursive"], json!(false));
        let files: Vec<String> = h.rows().into_iter().map(|r| r.0).collect();
        assert!(files.iter().all(|f| f == "pkg/util.py"), "{files:?}");
        h.press(h.keys.recursive);
        // A folder that is not there.
        h.type_text("/missing");
        h.settle();
        assert_eq!(h.state()["empty_text"], json!("Directory is not found"));
        // Back to the whole project: Option+P / Alt+P.
        h.press(h.keys.scope_project);
        h.settle();
        let s = h.state();
        assert_eq!(s["scope"], json!("project"));
        assert_eq!(s["files"], json!(4));
        h.press("escape");
        // A folder selected in the project tool window becomes the scope.
        h.invoke("project.tree.reveal", json!({ "path": "pkg" }));
        let panel = h.window.read_with(h.cx, |w, _| w.panel().clone()).unwrap();
        let handle = h.cx.update(|cx| panel.focus_handle(cx));
        h.cx.update_window(h.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        h.draw();
        h.press(h.keys.open);
        let s = h.state();
        assert_eq!(s["scope"], json!("directory"));
        assert_eq!(s["directory"], json!(pkg));
        h.press("escape");
    });
}

#[gpui::test]
fn named_scopes_open_and_recent_files(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke("project.open-file", json!({ "path": "notes.txt" }));
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        let s = h.state();
        assert_eq!(
            s["scopes"],
            json!([
                "Project Files",
                "Recently Viewed Files",
                "Open Files",
                "Current File"
            ])
        );
        // Option+M / Alt+M: the Module tab, the project's one module.
        h.press(h.keys.scope_module);
        h.settle();
        let s = h.state();
        assert_eq!(s["scope"], json!("module"));
        let name = h.root.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(s["module"], json!(name));
        assert_eq!(s["files"], json!(3));
        // Option+S / Alt+S: the Scope tab with the scope chosen last.
        h.press(h.keys.scope_named);
        h.settle();
        assert_eq!(h.state()["scope"], json!("Project Files"));
        assert_eq!(h.state()["files"], json!(3));
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "scope": "Current File" }),
        );
        assert_eq!(h.rows(), [("notes.txt".to_owned(), 1)]);
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "scope": "Open Files" }),
        );
        let mut files: Vec<String> = h.rows().into_iter().map(|r| r.0).collect();
        files.dedup();
        files.sort();
        assert_eq!(files, ["app.py", "notes.txt"]);
        // An edit puts the file into Recently Changed Files.
        let editor = h.editor();
        h.cx.update(|cx| editor.update(cx, |e, cx| e.insert("needle ", cx)));
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "scope": "Recently Changed Files" }),
        );
        assert_eq!(h.rows(), [("notes.txt".to_owned(), 1)]);
        assert_eq!(h.state()["matches"], json!(3), "the edited text");
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn the_preview_shows_the_selected_line(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        let rows = h.rows();
        h.cx.executor().advance_clock(DEBOUNCE);
        h.draw();
        let p = h.state()["preview"].clone();
        assert_eq!(p["path"], json!(rows[0].0));
        assert_eq!(p["caret"][0], json!(rows[0].1 - 1));
        assert_eq!(p["read_only"], json!(false), "the preview edits the file");
        assert!(p["highlights"].as_u64().unwrap() >= 1);
        // Another row: after a short wait.
        h.press("down");
        h.press("down");
        h.cx.executor().advance_clock(DEBOUNCE);
        h.draw();
        let p = h.state()["preview"].clone();
        assert_eq!(p["path"], json!(rows[2].0));
        assert_eq!(p["caret"][0], json!(rows[2].1 - 1));
        // The splitter moves the boundary between the list and the preview.
        let split = h.state()["split"].as_f64().unwrap();
        let bar = h.bounds_of("find-in-files-splitter");
        h.drag(bar.center(), bar.center() + point(px(0.0), px(60.0)));
        let moved = h.state()["split"].as_f64().unwrap();
        assert!(moved > split + 0.05, "{split} -> {moved}");
        h.invoke("find-in-files.close", json!(null));
        let settings = h.cx.update(|cx| workbench::find_in_files::settings(cx));
        assert!((settings.splitter.unwrap() as f64 - moved).abs() < 1e-6);
    });
}

#[gpui::test]
fn the_popup_is_resized_from_its_edges_and_keeps_its_size(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        let before = h.bounds_of("find-in-files");
        let grip = h.bounds_of("popup-grip-right");
        h.drag(grip.center(), grip.center() + point(px(-60.0), px(0.0)));
        let after = h.bounds_of("find-in-files");
        assert!(
            (f32::from(after.size.width) - f32::from(before.size.width) + 60.0).abs() < 1.0,
            "{before:?} -> {after:?}"
        );
        assert_eq!(after.left(), before.left());
        // Moved by its header.
        let header = h.bounds_of("find-in-files-header");
        let at = header.origin + point(px(200.0), px(10.0));
        h.drag(at, at + point(px(30.0), px(40.0)));
        let moved = h.bounds_of("find-in-files");
        assert_eq!(moved.origin, after.origin + point(px(30.0), px(40.0)));
        h.press("escape");
        h.press(h.keys.open);
        assert_eq!(h.bounds_of("find-in-files"), moved, "kept");
        let settings = h.cx.update(|cx| workbench::find_in_files::settings(cx));
        assert!(settings.popup.is_some());
        h.press("escape");
    });
}

#[gpui::test]
fn histories_masks_and_tab_between_fields(cx: &mut TestAppContext) {
    both(cx, |h| {
        for text in ["alpha", "beta"] {
            h.invoke("find-in-files.search", json!({ "text": text }));
            h.invoke("find-in-files.close", json!(null));
        }
        h.press(h.keys.open);
        // Option+Down / Alt+Down: the search history, newest first.
        h.press("alt-down");
        let d = h.state()["dropdown"].clone();
        assert_eq!(d["kind"], json!("search-history"));
        assert_eq!(d["items"][0], json!("beta"));
        assert_eq!(d["items"][1], json!("alpha"));
        h.press("down");
        h.press("enter");
        let s = h.state();
        assert_eq!(s["dropdown"], Value::Null);
        assert_eq!(s["text"], json!("alpha"));
        assert!(h.focused(FindInFiles::query_field));
        // The masks of the mask field (on from the first run).
        h.focus_field(FindInFiles::mask_field);
        h.press("alt-down");
        let d = h.state()["dropdown"].clone();
        assert_eq!(d["kind"], json!("masks"));
        assert_eq!(d["items"], json!(["*.py", "*.xml"]));
        h.press("down");
        h.press("enter");
        assert_eq!(h.state()["mask"]["text"], json!("*.xml"));
        // Escape closes a list and leaves the popup open.
        h.press("alt-down");
        h.press("escape");
        let s = h.state();
        assert_eq!(s["dropdown"], Value::Null);
        assert_eq!(s["open"], json!(true));
        // Tab goes through the fields and back.
        h.focus_field(FindInFiles::query_field);
        h.press("tab");
        assert!(h.focused(FindInFiles::mask_field));
        h.press("tab");
        assert!(h.focused(FindInFiles::query_field));
        h.press("shift-tab");
        assert!(h.focused(FindInFiles::mask_field));
        // Without a query nothing is listed, also with the mask on.
        h.invoke(
            "find-in-files.search",
            json!({ "text": "", "mask": "*.txt" }),
        );
        let s = h.state();
        assert_eq!(s["rows"], json!([]));
        assert_eq!(s["info"], json!(""));
        assert_eq!(s["empty_text"], json!("Type search query to find in files"));
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn replace_one_match_and_all_of_them(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.write("crlf.txt", b"needle one\r\nneedle two\r\n");
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "caf\u{e9} needle\n".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        h.write("utf16.txt", &utf16);
        // Cmd+Shift+R / Ctrl+Shift+R: Replace in Files, a row per match.
        h.focus_editor();
        h.press(h.keys.replace);
        let s = h.state();
        assert_eq!(s["replace_mode"], json!(true));
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "replace": "pin", "mask": "" }),
        );
        let s = h.state();
        assert_eq!(s["matches"], json!(8));
        assert_eq!(s["rows"].as_array().unwrap().len(), 8, "a row per match");
        // Enter replaces the selected match: notes.txt is not open, so the
        // file changes on disk.
        let notes = h.rows().iter().position(|r| r.0 == "notes.txt").unwrap();
        for _ in 0..notes {
            h.press("down");
        }
        h.press("enter");
        assert_eq!(h.read("notes.txt"), b"a pin and a Needle\n");
        let s = h.state();
        assert_eq!(s["rows"].as_array().unwrap().len(), 7);
        assert_eq!(s["selected"], json!(notes), "the next row in its place");
        assert_eq!(s["open"], json!(true), "the popup stays open");
        // In the open file the buffer changes, one undo step.
        let app = h.rows().iter().position(|r| r.0 == "app.py").unwrap();
        h.invoke("find-in-files.page-up", json!(null));
        for _ in 0..app {
            h.invoke("find-in-files.next-result", json!(null));
        }
        h.invoke("find-in-files.replace-selected", json!(null));
        let text = h.invoke("editor.text", json!(null));
        assert!(text.as_str().unwrap().contains("print(\"pin\")"), "{text}");
        assert!(
            h.read("app.py").windows(6).any(|w| w == b"needle"),
            "not saved"
        );
        // A read-only file: nothing is replaced, the status bar says why.
        h.write("ro.txt", b"needle\n");
        set_read_only(&h.root.join("ro.txt"));
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "replace": "pin" }),
        );
        h.invoke("find-in-files.replace-all", json!(null));
        assert!(h.cx.has_pending_prompt(), "several files: a confirmation");
        let (title, detail) = h.cx.pending_prompt().unwrap();
        assert_eq!(title, "Replace All");
        assert!(
            detail.starts_with("Replace 7 occurrences of 'needle' across 5 files with 'pin'"),
            "{detail}"
        );
        h.cx.simulate_prompt_answer("Replace");
        h.draw();
        let message = h.message();
        assert!(
            message
                .as_str()
                .unwrap_or_default()
                .contains("ro.txt is read-only"),
            "{message}"
        );
        assert_eq!(h.read("crlf.txt"), b"needle one\r\nneedle two\r\n");
        // Without it: every match is replaced, the popup closes.
        set_writable(&h.root.join("ro.txt"));
        std::fs::remove_file(h.root.join("ro.txt")).unwrap();
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "replace": "pin" }),
        );
        h.press(h.keys.replace_all);
        h.cx.simulate_prompt_answer("Replace");
        h.draw();
        assert_eq!(h.state()["open"], json!(false));
        assert_eq!(h.message(), json!("6 occurrences replaced"));
        assert_eq!(h.read("crlf.txt"), b"pin one\r\npin two\r\n");
        let back = text::decode(&h.read("utf16.txt")).unwrap();
        assert_eq!(back.text, "caf\u{e9} pin\n");
        assert_eq!(back.encoding, text::Encoding::Utf16Le);
        assert_eq!(h.read("pkg/util.py"), b"x = 1\n\n\n# pin here\npin = 2\n");
        // The open file: one undo step brings its match back.
        h.focus_editor();
        h.press(h.keys.undo);
        let text = h.invoke("editor.text", json!(null));
        assert!(
            text.as_str().unwrap().contains("print(\"needle\")"),
            "{text}"
        );
    });
}

#[cfg(unix)]
fn set_read_only(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
}

#[cfg(unix)]
fn set_writable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[cfg(not(unix))]
fn set_read_only(path: &std::path::Path) {
    let mut p = std::fs::metadata(path).unwrap().permissions();
    p.set_readonly(true);
    std::fs::set_permissions(path, p).unwrap();
}

#[cfg(not(unix))]
#[allow(clippy::permissions_set_readonly_false)]
fn set_writable(path: &std::path::Path) {
    let mut p = std::fs::metadata(path).unwrap().permissions();
    p.set_readonly(false);
    std::fs::set_permissions(path, p).unwrap();
}

#[gpui::test]
fn a_folder_marked_excluded_is_left_out_at_once(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        assert_eq!(h.state()["files"], json!(3));
        h.invoke("find-in-files.close", json!(null));
        // Mark Directory As > Excluded in the tree.
        h.invoke("project.tree.mark-excluded", json!({ "path": "pkg" }));
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        let files: Vec<String> = h.rows().into_iter().map(|r| r.0).collect();
        assert!(files.iter().all(|f| !f.starts_with("pkg/")), "{files:?}");
        assert_eq!(h.state()["files"], json!(2));
        h.invoke("find-in-files.close", json!(null));
        // Cancel Exclusion: it is searched again.
        h.invoke("project.tree.cancel-exclusion", json!({ "path": "pkg" }));
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        assert_eq!(h.state()["files"], json!(3));
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn find_in_files_from_the_tree_context_menu(cx: &mut TestAppContext) {
    both(cx, |h| {
        let pkg = h.root.join("pkg").display().to_string();
        // On a folder: Find in Files… searches that folder.
        h.invoke("project.tree.reveal", json!({ "path": "pkg" }));
        h.invoke("project.tree.context-menu", json!(null));
        // New, Cut, Copy, Copy Path/Reference… (Paste is off), then Find
        // in Files….
        h.press("down down down down down enter");
        let s = h.state();
        assert_eq!(s["open"], json!(true));
        assert_eq!(s["replace_mode"], json!(false));
        assert_eq!(s["scope"], json!("directory"));
        assert_eq!(s["directory"], json!(pkg));
        h.press("escape");
        // On a file: Replace in Files… in the file's folder.
        h.invoke("project.tree.reveal", json!({ "path": "pkg/util.py" }));
        h.invoke("project.tree.context-menu", json!(null));
        h.press("down down down down down down enter");
        let s = h.state();
        assert_eq!(s["replace_mode"], json!(true));
        assert_eq!(s["scope"], json!("directory"));
        assert_eq!(s["directory"], json!(pkg));
        h.press("escape");
    });
}

impl H<'_> {
    /// Selects the first row of `path` and waits for the preview.
    fn select_row_of(&mut self, path: &str) {
        let index = self
            .rows()
            .iter()
            .position(|r| r.0 == path)
            .unwrap_or_else(|| panic!("no row of {path}"));
        self.invoke("find-in-files.page-up", json!(null));
        for _ in 0..index {
            self.invoke("find-in-files.next-result", json!(null));
        }
        self.cx.executor().advance_clock(DEBOUNCE);
        self.draw();
        assert_eq!(self.state()["preview"]["path"], json!(path));
    }

    fn tab_count(&mut self) -> usize {
        self.invoke("workbench.state", json!(null))["tabs"]
            .as_array()
            .unwrap()
            .len()
    }
}

#[gpui::test]
fn the_preview_edits_the_open_file_with_its_tab(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        h.select_row_of("app.py");
        assert_eq!(h.state()["preview"]["linked"], json!(true));
        // Typing in the preview: the tab shows it at once.
        h.focus_field(FindInFiles::preview_editor);
        h.type_text("XYZ");
        let preview = h.popup();
        let text =
            h.cx.update(|cx| preview.read(cx).preview_editor().read(cx).text());
        assert!(text.contains("print(\"XYZneedle\")"), "{text}");
        let tab = h.invoke("editor.text", json!(null));
        assert!(
            tab.as_str().unwrap().contains("print(\"XYZneedle\")"),
            "{tab}"
        );
        // The match marked in the preview moved with the text.
        let marked = h.cx.update(|cx| {
            let e = preview.read(cx).preview_editor().read(cx);
            let m = e.search_highlights().unwrap().matches[0].clone();
            e.buffer().slice(m)
        });
        assert_eq!(marked, "needle");
        // Undo in the preview: both views go back.
        h.press(h.keys.undo);
        let tab = h.invoke("editor.text", json!(null));
        assert!(tab.as_str().unwrap().contains("print(\"needle\")"), "{tab}");
        let text =
            h.cx.update(|cx| preview.read(cx).preview_editor().read(cx).text());
        assert!(text.contains("print(\"needle\")"), "{text}");
        // An edit in the tab shows in the preview.
        let editor = h.editor();
        h.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                e.set_selections(text::SelectionSet::caret(0), cx);
                e.insert("# top\n", cx);
            })
        });
        h.draw();
        let text =
            h.cx.update(|cx| preview.read(cx).preview_editor().read(cx).text());
        assert!(text.starts_with("# top\nimport os"), "{text}");
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn the_preview_edits_and_saves_a_file_that_is_not_open(cx: &mut TestAppContext) {
    both(cx, |h| {
        let tabs = h.tab_count();
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        h.select_row_of("notes.txt");
        assert_eq!(h.state()["preview"]["linked"], json!(false));
        h.focus_field(FindInFiles::preview_editor);
        h.type_text("QQ");
        assert_eq!(h.state()["preview"]["modified"], json!(true));
        assert_eq!(h.read("notes.txt"), b"a needle and a Needle\n", "not yet");
        // Saved after the idle time, without a tab.
        h.cx.executor().advance_clock(Duration::from_secs(3));
        h.draw();
        assert_eq!(h.read("notes.txt"), b"a QQneedle and a Needle\n");
        assert_eq!(h.state()["preview"]["modified"], json!(false));
        assert_eq!(h.tab_count(), tabs, "no tab opened");
        // Undo in the preview, saved again when the popup closes.
        h.press(h.keys.undo);
        h.press("escape");
        h.draw();
        assert_eq!(h.read("notes.txt"), b"a needle and a Needle\n");
        assert_eq!(h.tab_count(), tabs);
    });
}

#[gpui::test]
fn keys_in_the_preview_go_to_its_editor(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        h.select_row_of("notes.txt");
        let rows = h.state()["rows"].clone();
        h.focus_field(FindInFiles::preview_editor);
        let preview = h.popup();
        let text = |h: &mut H| {
            h.cx.update(|cx| preview.read(cx).preview_editor().read(cx).text())
        };
        // Enter breaks the line (it does not open the result), Tab
        // indents.
        h.press("enter");
        assert_eq!(h.state()["open"], json!(true));
        assert_eq!(text(h), "a \nneedle and a Needle\n");
        h.press("tab");
        assert_eq!(text(h), "a \n    needle and a Needle\n");
        // The rows stay as they were found (the reference does not search
        // again for an edit in the preview).
        assert_eq!(h.state()["rows"], rows);
        // Escape closes the popup; the edits are saved.
        h.press("escape");
        assert_eq!(h.state()["open"], json!(false));
        h.draw();
        assert_eq!(h.read("notes.txt"), b"a \n    needle and a Needle\n");
    });
}

#[gpui::test]
fn opening_a_file_edited_in_the_preview_keeps_the_edits(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        h.select_row_of("notes.txt");
        h.focus_field(FindInFiles::preview_editor);
        h.type_text("ZZ");
        // Enter in the search field opens the row: the tab continues with
        // the edited text.
        h.focus_field(FindInFiles::query_field);
        h.press("enter");
        assert_eq!(h.state()["open"], json!(false));
        let s = h.invoke("workbench.state", json!(null));
        let active = s["active_tab"].as_u64().unwrap() as usize;
        assert_eq!(s["tabs"][active]["path"], json!("notes.txt"));
        assert_eq!(s["tabs"][active]["modified"], json!(true));
        let text = h.invoke("editor.text", json!(null));
        assert_eq!(text, json!("a ZZneedle and a Needle\n"));
        // The tab saves it.
        h.invoke("workbench.save-all", json!(null));
        assert_eq!(h.read("notes.txt"), b"a ZZneedle and a Needle\n");
    });
}

impl H<'_> {
    /// The placeholder a popup field shows now.
    fn placeholder(&mut self, field: fn(&FindInFiles) -> &Entity<Editor>) -> Option<String> {
        let popup = self.popup();
        let field = self.cx.update(|cx| field(popup.read(cx)).clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                let focused = field.focus_handle(cx).is_focused(window);
                field
                    .read(cx)
                    .shown_placeholder(focused)
                    .map(|p| p.to_string())
            })
            .unwrap()
    }

    /// The preview's selection, as text.
    fn preview_selection(&mut self) -> (usize, String) {
        let popup = self.popup();
        self.cx.update(|cx| {
            let e = popup.read(cx).preview_editor().read(cx);
            let r = e.selections().primary().range();
            (r.start, e.buffer().slice(r))
        })
    }

    /// The text of the matches marked in the preview.
    fn preview_marks(&mut self) -> Vec<String> {
        let popup = self.popup();
        self.cx.update(|cx| {
            let e = popup.read(cx).preview_editor().read(cx);
            e.search_highlights()
                .map(|h| {
                    h.matches
                        .iter()
                        .map(|m| e.buffer().slice(m.clone()))
                        .collect()
                })
                .unwrap_or_default()
        })
    }
}

#[gpui::test]
fn the_empty_fields_show_their_placeholder_only_without_the_focus(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.focus_editor();
        h.press(h.keys.replace);
        h.press(h.keys.select_all);
        h.press("backspace");
        assert!(h.focused(FindInFiles::query_field));
        // The reference shows the caret only in the empty field with the
        // focus, and "Search" / "Replace" in an empty one without it.
        assert_eq!(h.placeholder(FindInFiles::query_field), None);
        h.press("tab");
        assert!(h.focused(FindInFiles::replace_field));
        assert_eq!(
            h.placeholder(FindInFiles::query_field),
            Some("Search".into())
        );
        h.press(h.keys.select_all);
        h.press("backspace");
        assert_eq!(h.placeholder(FindInFiles::replace_field), None);
        h.press("shift-tab");
        assert_eq!(
            h.placeholder(FindInFiles::replace_field),
            Some("Replace".into())
        );
        h.press("escape");
    });
}

#[gpui::test]
fn find_next_in_the_preview_is_the_editors(cx: &mut TestAppContext) {
    both(cx, |h| {
        // The last search in a file (Find Next repeats it).
        h.cx.update(|cx| {
            cx.set_global(editor::find::LastSearch {
                text: "needle".into(),
                performed: true,
                ..Default::default()
            })
        });
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        h.select_row_of("notes.txt");
        let selected = h.state()["selected"].clone();
        h.focus_field(FindInFiles::preview_editor);
        // "a needle and a Needle": the caret is on the first match.
        h.press(h.keys.next);
        assert_eq!(h.preview_selection(), (2, "needle".into()));
        h.press(h.keys.next);
        assert_eq!(h.preview_selection(), (15, "Needle".into()));
        h.press(h.keys.previous);
        assert_eq!(h.preview_selection(), (2, "needle".into()));
        // The rows did not move.
        assert_eq!(h.state()["selected"], selected);
        // Past the last match: the editor's hint, gone with the next key.
        h.press(h.keys.next);
        h.press(h.keys.next);
        let hint = h.state()["preview"]["hint"].clone();
        assert!(
            hint.as_str()
                .is_some_and(|t| t.contains("not found, press")),
            "{hint}"
        );
        h.press("right");
        assert_eq!(h.state()["preview"]["hint"], json!(null));
        // In the search field the same keys move through the rows (the
        // order of the files depends on which is found first).
        h.focus_field(FindInFiles::query_field);
        let last = h.rows().len() as u64 - 1;
        let key = if selected == json!(last) {
            h.keys.previous
        } else {
            h.keys.next
        };
        h.press(key);
        assert_ne!(h.state()["selected"], selected);
        h.press("escape");
    });
}

#[gpui::test]
fn edit_source_opens_the_file_at_the_previews_caret(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        h.select_row_of("notes.txt");
        h.focus_field(FindInFiles::preview_editor);
        h.press("end");
        h.press("f4");
        assert_eq!(h.state()["open"], json!(false));
        assert_eq!(h.caret(), ("notes.txt".into(), 0, 21));
        assert!(h.editor_focused());
        // From the search field F4 opens the selected row at its match.
        h.focus_editor();
        h.press(h.keys.open);
        h.settle();
        let rows = h.rows();
        let util = rows.iter().position(|r| r.0 == "pkg/util.py").unwrap();
        h.invoke("find-in-files.page-up", json!(null));
        for _ in 0..util {
            h.invoke("find-in-files.next-result", json!(null));
        }
        h.press("f4");
        assert_eq!(h.state()["open"], json!(false));
        assert_eq!(h.caret(), ("pkg/util.py".into(), 3, 2));
    });
}

#[gpui::test]
fn rows_open_where_edits_in_the_preview_moved_their_matches(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "mask": "" }),
        );
        let rows = h.state()["rows"].clone();
        // pkg/util.py: "# needle here" (line 4) and "needle = 2" (line 5).
        h.select_row_of("pkg/util.py");
        h.focus_field(FindInFiles::preview_editor);
        // A line break before the first match moves both down a line.
        h.press("enter");
        assert_eq!(h.preview_marks(), vec!["needle".to_owned()]);
        // The rows keep their text; the next one is marked where its
        // match is now.
        assert_eq!(h.state()["rows"], rows);
        h.focus_field(FindInFiles::query_field);
        h.press(h.keys.next);
        h.cx.executor().advance_clock(DEBOUNCE);
        h.draw();
        assert_eq!(h.preview_marks(), vec!["needle".to_owned()]);
        assert_eq!(h.state()["preview"]["caret"], json!([5, 0]));
        // Text typed before it in the preview: the row opens at the match.
        h.focus_field(FindInFiles::preview_editor);
        h.type_text("new_");
        assert_eq!(h.preview_marks(), vec!["needle".to_owned()]);
        h.focus_field(FindInFiles::query_field);
        h.press("enter");
        assert_eq!(h.caret(), ("pkg/util.py".into(), 5, 4));
        let text = h.invoke("editor.text", json!(null));
        assert_eq!(text, json!("x = 1\n\n\n# \nneedle here\nnew_needle = 2\n"));
    });
}

#[gpui::test]
fn replace_changes_the_match_where_preview_edits_moved_it(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke(
            "find-in-files.search",
            json!({ "text": "needle", "replace": "pin", "mask": "" }),
        );
        h.select_row_of("notes.txt");
        h.focus_field(FindInFiles::preview_editor);
        h.type_text("big ");
        // Replace (from the replace field): the match it was found at, in
        // the document the preview edits; the preview saves it.
        h.focus_field(FindInFiles::replace_field);
        h.press("enter");
        let popup = h.popup();
        let text =
            h.cx.update(|cx| popup.read(cx).preview_editor().read(cx).text());
        assert_eq!(text, "a big pin and a Needle\n");
        h.press("escape");
        h.draw();
        assert_eq!(h.read("notes.txt"), b"a big pin and a Needle\n");
    });
}

// ---- moving the popup and the splitter --------------------------------------

impl H<'_> {
    fn dragging(&mut self) -> Value {
        self.state()["dragging"].clone()
    }

    fn split(&mut self) -> f64 {
        self.state()["split"].as_f64().unwrap()
    }

    /// Each event in its own frame.
    fn send(&mut self, events: Vec<gpui::PlatformInput>) {
        mouse::send(self.cx, self.window.into(), events);
    }

    /// The events with no frame between them that they did not ask for.
    fn quick(&mut self, events: Vec<gpui::PlatformInput>) {
        mouse::quick(self.cx, self.window.into(), events);
    }
}

#[gpui::test]
fn a_quick_click_on_the_header_leaves_no_move(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        let start = h.bounds_of("find-in-files").origin;
        let header = h.bounds_of("find-in-files-header");
        let at = header.origin + point(px(200.0), px(10.0));
        h.quick(vec![press(at), release(at)]);
        assert_eq!(h.dragging(), json!({ "popup": false, "split": false }));
        h.send(vec![hover(at + point(px(80.0), px(60.0)))]);
        assert_eq!(h.bounds_of("find-in-files").origin, start);
        // A click on the pin button selects it; pressed and dragged, the
        // button does not move the popup.
        let pin = h.bounds_of("find-in-files-pin").center();
        let pinned = h.state()["pinned"].clone();
        h.quick(vec![press(pin), release(pin)]);
        assert_ne!(h.state()["pinned"], pinned);
        let away = pin + point(px(-80.0), px(60.0));
        h.send(vec![press(pin), drag_to(away), release(away)]);
        assert_eq!(h.dragging(), json!({ "popup": false, "split": false }));
        assert_eq!(h.bounds_of("find-in-files").origin, start);
        assert_ne!(h.state()["pinned"], pinned, "released elsewhere");
        h.quick(vec![press(pin), release(pin)]);
        assert_eq!(h.state()["pinned"], pinned);
        h.invoke("find-in-files.close", json!(null));
    });
}

#[gpui::test]
fn the_splitter_moves_only_while_held(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke("find-in-files.search", json!({ "text": "needle" }));
        let split = json!({ "popup": false, "split": true });
        let idle = json!({ "popup": false, "split": false });
        let start = h.split();
        // A quick click on the splitter: no drag is left.
        let bar = h.bounds_of("find-in-files-splitter").center();
        h.quick(vec![press(bar), release(bar)]);
        assert_eq!(h.dragging(), idle);
        h.send(vec![hover(bar + point(px(0.0), px(60.0)))]);
        assert_eq!(h.split(), start);
        // Held: the split follows; a lost release ends the drag.
        h.send(vec![press(bar), drag_to(bar + point(px(0.0), px(40.0)))]);
        assert_eq!(h.dragging(), split);
        let moved = h.split();
        assert!(moved > start + 0.02, "{start} -> {moved}");
        h.send(vec![hover(bar + point(px(0.0), px(120.0)))]);
        assert_eq!(h.dragging(), idle);
        assert_eq!(h.split(), moved);
        // A release outside the window ends it there.
        let bar = h.bounds_of("find-in-files-splitter").center();
        h.send(vec![press(bar), drag_to(bar - point(px(0.0), px(30.0)))]);
        h.send(vec![release(point(px(-20.0), bar.y - px(40.0)))]);
        assert_eq!(h.dragging(), idle);
        let kept = h.split();
        assert!(kept < moved, "{moved} -> {kept}");
        // The window losing the focus ends it.
        let bar = h.bounds_of("find-in-files-splitter").center();
        h.send(vec![press(bar), drag_to(bar + point(px(0.0), px(10.0)))]);
        assert_eq!(h.dragging(), split);
        VisualTestContext::from_window(h.window.into(), h.cx).deactivate_window();
        h.draw();
        assert_eq!(h.dragging(), idle);
        h.send(vec![release(bar)]);
        h.cx.update_window(h.window.into(), |_, window, _| window.activate_window())
            .unwrap();
        h.draw();
        h.invoke("find-in-files.close", json!(null));
    });
}
