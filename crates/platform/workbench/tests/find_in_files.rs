//! Find in Files (tests/scenarios/editor/find-in-files.md), driven by real
//! keystrokes through both default keymaps in a headless main window over
//! a project on disk.

use std::rc::Rc;

use editor::Editor;
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, WindowHandle};
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
        let rel = path
            .strip_prefix(&format!("{}/", self.root.display()))
            .unwrap_or(&path)
            .replace('\\', "/");
        let editor = self.editor();
        let p = self.cx.update(|cx| editor.read(cx).caret_point());
        (rel, p.line, p.column)
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str, keys: &'static Keys) -> H<'a> {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
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
        assert_eq!(s["empty_text"], json!("Type search query to find in files"));
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
        assert_eq!(s["mask"], json!({ "on": false, "text": "*.py" }));
        assert_eq!(s["files"], json!(3));
        // Alt+K checks the box and puts the focus in the mask.
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
