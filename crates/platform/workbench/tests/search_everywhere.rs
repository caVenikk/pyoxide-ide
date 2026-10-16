//! Search Everywhere and Go to File (tests/scenarios/editor/search-everywhere.md),
//! driven by real keystrokes and modifier events through both default
//! keymaps in a headless main window over a project on disk.

mod mouse;

use std::rc::Rc;
use std::time::Duration;

use editor::Editor;
use gpui::{
    AppContext as _, Bounds, Focusable as _, Modifiers, Pixels, PlatformInput, Point,
    TestAppContext, VisualTestContext, WindowHandle, point, px,
};
use mouse::{drag_to, hover, press, release};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use workbench::keymap::{self, Keymap};
use workbench::search_everywhere::DEBOUNCE;

use workbench::{NavigationHooks, Product, Workbench};

struct Keys {
    go_to_file: &'static str,
    go_to_action: &'static str,
    go_to_text: &'static str,
    recent: &'static str,
    match_case: &'static str,
    first: &'static str,
    last: &'static str,
    non_project: &'static str,
    select_all: &'static str,
}

const MAC: Keys = Keys {
    go_to_file: "cmd-shift-o",
    go_to_action: "cmd-shift-a",
    go_to_text: "cmd-alt-shift-e",
    recent: "cmd-e",
    match_case: "alt-c",
    first: "cmd-up",
    last: "cmd-down",
    non_project: "alt-n",
    select_all: "cmd-a",
};

const WIN: Keys = Keys {
    go_to_file: "ctrl-shift-n",
    go_to_action: "ctrl-shift-a",
    go_to_text: "ctrl-alt-shift-e",
    recent: "ctrl-e",
    match_case: "alt-c",
    first: "ctrl-up",
    last: "ctrl-down",
    non_project: "alt-n",
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

    /// Lets a search start after its delay and finish.
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
        self.invoke("search-everywhere.state", json!(null))
    }

    /// The rows as `kind:path-or-id`.
    fn rows(&mut self) -> Vec<String> {
        self.state()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| match r["kind"].as_str().unwrap() {
                "action" => format!("action:{}", r["id"].as_str().unwrap()),
                "more" => "more".to_owned(),
                kind => format!("{kind}:{}", r["path"].as_str().unwrap()),
            })
            .collect()
    }

    fn editor(&mut self) -> gpui::Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn caret(&mut self) -> (String, usize, usize) {
        let s = self.invoke("editor.state", json!(null));
        let path = s["path"].as_str().unwrap_or_default().replace('\\', "/");
        let root = format!("{}/", self.root.display()).replace('\\', "/");
        let rel = path.strip_prefix(&root).unwrap_or(&path).to_owned();
        let editor = self.editor();
        let p = self.cx.update(|cx| editor.read(cx).caret_point());
        (rel, p.line, p.column)
    }

    fn vcx(&mut self) -> VisualTestContext {
        VisualTestContext::from_window(self.window.into(), self.cx)
    }

    /// Shift goes down and up twice, `gap` between the steps (a while
    /// after the last key: a tap right after typing does not count).
    fn double_shift(&mut self, gap: Duration) {
        self.cx.executor().advance_clock(Duration::from_millis(200));
        for _ in 0..2 {
            for modifiers in [Modifiers::shift(), Modifiers::none()] {
                self.vcx().simulate_modifiers_change(modifiers);
                self.cx.executor().advance_clock(gap);
            }
        }
        self.draw();
    }

    /// A click on an element drawn with this debug selector.
    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .vcx()
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        self.vcx()
            .simulate_click(bounds.center(), Modifiers::none());
        self.draw();
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str, keys: &'static Keys) -> H<'a> {
    let dir = tempfile::tempdir().unwrap();
    // A fixed project name: Go to File matches folder paths from it.
    let root = std::fs::canonicalize(dir.path()).unwrap().join("proj");
    for (path, text) in [
        (
            "app.py",
            "import os\n\ndef main():\n    print(\"needle\")\n",
        ),
        ("notes.txt", "a needle\n"),
        ("pkg/__init__.py", ""),
        ("pkg/util.py", "x = 1\n\n\n# needle here\nneedle = 2\n"),
        ("pkg/application.py", "y = 2\n"),
        ("tests/test_app.py", "z = 3\n"),
        ("src/lib/__init__.py", ""),
        ("src/lib/core.py", "c = 1\n"),
        (".git/config", "x\n"),
        (".venv/lib/site.py", "s = 1\n"),
        (".venv/pyvenv.cfg", "home = /usr\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        // `.venv` is excluded as the Python product does.
        auto_excluded: project::AutoExclusions {
            dir_names: &[".venv"],
            root_dir_markers: &[],
        },
        navigation: NavigationHooks {
            source_roots: Some(source_roots),
            file_location: Some(dotted_location),
            package_markers: &["__init__.py"],
        },
        ..Product::default()
    };
    let keymap = Keymap::parse(keymap_text).unwrap();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &keymap);
        assert!(errors.is_empty(), "keymap errors: {errors:?}");
        workbench::open_main_window(cx, Rc::new(product))
    });
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
    // The index is built in the background.
    h.draw();
    let s = h.invoke("workbench.state", json!(null));
    assert!(s["file_index"].as_u64().is_some_and(|n| n > 0), "{s}");
    h
}

/// The Python product's source roots, as a product hook: `src`.
fn source_roots(root: &Path) -> Vec<PathBuf> {
    let src = root.join("src");
    if src.is_dir() { vec![src] } else { Vec::new() }
}

/// The Python product's locations: a module's package.
fn dotted_location(name: &str, folder: &str) -> Option<String> {
    name.ends_with(".py").then(|| folder.replace('/', "."))
}

fn both(cx: &mut TestAppContext, test: fn(&mut H)) {
    for (text, keys) in [(keymap::MACOS, &MAC), (keymap::DEFAULT, &WIN)] {
        let mut h = setup(cx, text, keys);
        test(&mut h);
    }
}

#[gpui::test]
fn go_to_file_finds_a_file_and_opens_it_at_a_line(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        let s = h.state();
        assert_eq!(s["open"], json!(true));
        assert_eq!(s["tab"], json!("files"));
        assert_eq!(s["text"], json!(""));
        assert!(h.rows().is_empty(), "nothing for an empty field");
        h.type_text("util");
        assert_eq!(h.state()["searching"], json!(true), "after a pause");
        h.settle();
        assert_eq!(h.rows(), ["file:pkg/util.py"]);
        // A line and column after the name.
        h.type_text(":4:3");
        h.settle();
        assert_eq!(h.rows(), ["file:pkg/util.py"]);
        h.press("enter");
        assert_eq!(h.state()["open"], json!(false));
        h.draw();
        assert_eq!(h.caret(), ("pkg/util.py".into(), 3, 2));
        // Reopened: the last search of the tab, selected.
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["text"], json!("util:4:3"));
        h.type_text("app");
        h.settle();
        assert_eq!(
            h.rows(),
            [
                "file:app.py",
                "file:pkg/application.py",
                "file:tests/test_app.py"
            ]
        );
        // Up and Down go round; Ctrl/Cmd+Up and Down to the ends.
        h.press("up");
        assert_eq!(h.state()["selected"], json!(2));
        h.press("down");
        assert_eq!(h.state()["selected"], json!(0));
        h.press(h.keys.last);
        assert_eq!(h.state()["selected"], json!(2));
        h.press(h.keys.first);
        assert_eq!(h.state()["selected"], json!(0));
        h.press("escape");
        assert_eq!(h.state()["open"], json!(false));
    });
}

#[gpui::test]
fn folders_in_the_pattern(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        h.type_text("pkg/app");
        h.settle();
        assert_eq!(h.rows(), ["file:pkg/application.py"]);
        h.press("escape");
        // A folder: shown in the project tool window.
        h.press(h.keys.go_to_file);
        h.press(h.keys.select_all);
        h.type_text("tests/");
        h.settle();
        assert_eq!(h.rows()[0], "folder:tests");
        h.press("enter");
        let s = h.invoke("workbench.state", json!(null));
        assert_eq!(s["search_everywhere"], json!(false));
    });
}

#[gpui::test]
fn locations_package_folders_and_the_footer(cx: &mut TestAppContext) {
    both(cx, |h| {
        let row = |h: &mut H, i: usize, key: &str| h.state()["rows"][i][key].clone();
        h.press(h.keys.go_to_file);
        h.type_text("core");
        h.settle();
        assert_eq!(h.rows(), ["file:src/lib/core.py"]);
        // A module's package from its source root; the footer's path too.
        assert_eq!(row(h, 0, "location"), json!("lib"));
        assert_eq!(h.state()["footer"], json!("lib/core.py"));
        h.press(h.keys.select_all);
        h.type_text("lib");
        h.settle();
        assert_eq!(h.rows()[0], "folder:src/lib");
        // A folder shows its own path; one with `__init__.py` is a package.
        assert_eq!(row(h, 0, "location"), json!("src/lib"));
        assert_eq!(row(h, 0, "package"), json!(true));
        h.press(h.keys.select_all);
        h.type_text("tests");
        h.settle();
        assert_eq!(h.rows()[0], "folder:tests");
        assert_eq!(row(h, 0, "package"), json!(false));
        h.press(h.keys.select_all);
        h.type_text("notes");
        h.settle();
        // Not Python: the folder; at the project folder, nothing.
        assert_eq!(row(h, 0, "location"), json!(""));
        let footer = h.state()["footer"].as_str().unwrap().replace('\\', "/");
        assert!(footer.ends_with("proj/notes.txt"), "{footer}");
        assert!(
            h.vcx()
                .debug_bounds("search-everywhere-right-split")
                .is_some()
        );
        h.press("escape");
    });
}

#[gpui::test]
fn a_path_outside_the_project_typed_in_full(cx: &mut TestAppContext) {
    both(cx, |h| {
        let outside = tempfile::tempdir().unwrap();
        let dir = if cfg!(windows) {
            outside.path().to_path_buf()
        } else {
            std::fs::canonicalize(outside.path()).unwrap()
        };
        let file = dir.join("outside.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        h.press(h.keys.go_to_file);
        h.type_text(&format!("{}:2", file.display()));
        h.settle();
        let shown = file.display().to_string().replace('\\', "/");
        assert_eq!(h.rows(), [format!("file:{shown}")]);
        h.press("enter");
        h.draw();
        assert_eq!(h.caret(), (shown, 1, 0));
    });
}

#[gpui::test]
fn double_shift_opens_search_everywhere(cx: &mut TestAppContext) {
    both(cx, |h| {
        // Too slow: nothing.
        h.double_shift(Duration::from_millis(400));
        assert_eq!(h.state()["open"], json!(false));
        h.double_shift(Duration::from_millis(60));
        let s = h.state();
        assert_eq!(s["open"], json!(true));
        assert_eq!(s["tab"], json!("all"));
        // Again on the All tab: non-project items.
        h.double_shift(Duration::from_millis(60));
        assert_eq!(h.state()["everywhere"], json!(true));
        h.press("escape");
        assert_eq!(h.state()["open"], json!(false));
    });
}

#[gpui::test]
fn recent_files_with_an_empty_field(cx: &mut TestAppContext) {
    both(cx, |h| {
        for file in ["notes.txt", "pkg/util.py", "app.py"] {
            h.invoke("project.open-file", json!({ "path": file }));
        }
        h.double_shift(Duration::from_millis(60));
        // The file of the active editor is left out; newest first.
        assert_eq!(h.rows(), ["file:pkg/util.py", "file:notes.txt"]);
        assert_eq!(h.state()["rows"][0]["recent"], json!(true));
        // Typed: recent files on top, then the others by weight, a file
        // once.
        h.type_text("ut");
        h.settle();
        let rows = h.rows();
        assert_eq!(rows[0], "file:pkg/util.py", "{rows:?}");
        assert_eq!(h.state()["rows"][0]["recent"], json!(true));
        let files: Vec<_> = rows.iter().filter(|r| r.starts_with("file:")).collect();
        assert_eq!(files, ["file:pkg/util.py"], "once");
        h.press("escape");
        // A recent file deleted since is left out.
        std::fs::remove_file(h.root.join("notes.txt")).unwrap();
        h.double_shift(Duration::from_millis(60));
        // The last search comes back selected: clear it.
        h.press("backspace");
        h.settle();
        assert_eq!(h.rows(), ["file:pkg/util.py"]);
        h.press("escape");
    });
}

#[gpui::test]
fn tabs_switch_and_the_shortcut_again_includes_non_project_items(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        h.type_text("site");
        h.settle();
        // Nothing in the project: non-project items are looked at.
        let s = h.state();
        assert_eq!(s["everywhere"], json!(true));
        assert_eq!(h.rows(), ["file:.venv/lib/site.py"]);
        assert_eq!(h.state()["rows"][0]["non_project"], json!(true));
        // A text that still contains it keeps them.
        h.type_text(".py");
        h.settle();
        assert_eq!(h.state()["everywhere"], json!(true));
        // Another text: project items again.
        h.press(h.keys.select_all);
        h.type_text("app");
        h.settle();
        assert_eq!(h.state()["everywhere"], json!(false));
        // Go to File again: non-project items on; Alt+N off.
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["everywhere"], json!(true));
        h.press(h.keys.non_project);
        assert_eq!(h.state()["everywhere"], json!(false));
        // Tab and Shift+Tab switch tabs, round; the text stays.
        h.press("tab");
        assert_eq!(h.state()["tab"], json!("actions"));
        h.press("tab");
        assert_eq!(h.state()["tab"], json!("text"));
        h.press("tab");
        assert_eq!(h.state()["tab"], json!("all"));
        h.press("shift-tab");
        assert_eq!(h.state()["tab"], json!("text"));
        assert_eq!(h.state()["text"], json!("app"));
        // Go to File from another tab: the Files tab.
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["tab"], json!("files"));
        h.press("escape");
    });
}

#[gpui::test]
fn actions_run_from_the_actions_tab(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.invoke("project.open-file", json!({ "path": "app.py" }));
        h.invoke("editor.type", json!({ "text": "x" }));
        let tab =
            |h: &mut H| h.invoke("workbench.state", json!(null))["tabs"][0]["modified"].clone();
        assert_eq!(tab(h), json!(true));
        h.press(h.keys.go_to_action);
        let s = h.state();
        assert_eq!(s["tab"], json!("actions"));
        h.type_text("save all");
        h.settle();
        let rows = h.rows();
        assert_eq!(rows[0], "action:workbench.save-all", "{rows:?}");
        let shortcut = h.state()["rows"][0]["shortcut"].clone();
        assert!(shortcut.is_string(), "{shortcut}");
        h.press("enter");
        assert_eq!(h.state()["open"], json!(false));
        h.draw();
        assert_eq!(tab(h), json!(false), "saved");
    });
}

#[gpui::test]
fn the_more_row_shows_the_next_page(cx: &mut TestAppContext) {
    both(cx, |h| {
        for i in 0..120 {
            std::fs::write(h.root.join(format!("pkg/mod_{i:03}.py")), "").unwrap();
        }
        // The index follows a new build of the project's rules.
        h.invoke(
            "project.open-folder",
            json!({ "path": h.root.display().to_string() }),
        );
        h.draw();
        h.press(h.keys.go_to_file);
        h.type_text("mod_");
        h.settle();
        let rows = h.rows();
        assert_eq!(rows.len(), 16, "a page and the more row");
        assert_eq!(rows[15], "more");
        let first = rows[0].clone();
        // Page Down goes a page on; Up on the first row goes round.
        h.press(h.keys.last);
        let page = h.state()["selected"].as_u64().unwrap();
        assert!(page > 1 && page < 16, "{page}");
        h.press(h.keys.first);
        h.press("up");
        assert_eq!(h.state()["selected"], json!(15), "the more row");
        // A click on it: the next page below the rows shown, its first row
        // selected.
        h.click("search-everywhere-more");
        h.settle();
        let rows = h.rows();
        assert_eq!(rows.len(), 31);
        assert_eq!(rows[0], first, "the rows shown stay");
        assert_eq!(h.state()["selected"], json!(15));
        // Enter on it, up to the last page: all of them, no more row.
        for _ in 0..10 {
            if h.rows().last().map(String::as_str) != Some("more") {
                break;
            }
            h.press(h.keys.first);
            h.press("up");
            h.press("enter");
            h.settle();
        }
        let rows = h.rows();
        assert_eq!(rows.len(), 120, "all, no more row");
        assert!(!rows.contains(&"more".to_owned()));
        h.press("escape");
    });
}

#[gpui::test]
fn the_text_tab_finds_lines(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_text);
        assert_eq!(h.state()["tab"], json!("text"));
        h.type_text("needle");
        h.settle();
        let rows = h.state()["rows"].clone();
        let lines: Vec<String> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| format!("{}:{}", r["path"].as_str().unwrap(), r["line"]))
            .collect();
        // In the order the files are found (each file's lines in order).
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            ["app.py:4", "notes.txt:1", "pkg/util.py:4", "pkg/util.py:5"],
            "{rows}"
        );
        // Match Case: the options are shared with Find in Files.
        h.press(h.keys.match_case);
        h.settle();
        assert_eq!(h.state()["options"]["case_sensitive"], json!(true));
        // Enter opens the line at its match.
        let first = h.state()["rows"][0].clone();
        let column = |text: &str| text.find("needle").unwrap();
        let path = first["path"].as_str().unwrap().to_owned();
        let line = first["line"].as_u64().unwrap() as usize - 1;
        let text = std::fs::read_to_string(h.root.join(&path)).unwrap();
        let col = column(text.lines().nth(line).unwrap());
        h.press("enter");
        h.draw();
        assert_eq!(h.caret(), (path, line, col));
        h.press(h.keys.go_to_text);
        assert_eq!(h.state()["options"]["case_sensitive"], json!(true));
        h.press(h.keys.match_case);
        h.press("escape");
        // On All, lines come after the files and actions.
        h.double_shift(Duration::from_millis(60));
        h.type_text("needle");
        h.settle();
        let rows = h.rows();
        assert!(rows.last().unwrap().starts_with("text:"), "{rows:?}");
        h.press("escape");
    });
}

#[gpui::test]
fn the_files_scope_and_filter(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["scope"], json!("Project Files"));
        // Go to File again: All Places, kept for the next time.
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["scope"], json!("All Places"));
        h.press("escape");
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["scope"], json!("All Places"));
        h.press(h.keys.go_to_file);
        assert_eq!(h.state()["scope"], json!("Project Files"));
        // The filter leaves out folders.
        h.type_text("pkg");
        h.settle();
        assert!(h.rows().contains(&"folder:pkg".to_owned()));
        h.click("search-everywhere-filter");
        assert_eq!(h.state()["dropdown"], json!("filetypes"));
        h.click("search-everywhere-filter-0");
        h.settle();
        assert_eq!(h.state()["hidden_file_types"], json!(["Directory"]));
        assert!(!h.rows().contains(&"folder:pkg".to_owned()));
        h.click("search-everywhere-filter-all");
        h.settle();
        assert!(h.rows().contains(&"folder:pkg".to_owned()));
        h.press("escape");
        h.press("escape");
    });
}

#[gpui::test]
fn recent_files(cx: &mut TestAppContext) {
    both(cx, |h| {
        for file in ["notes.txt", "pkg/util.py", "app.py"] {
            h.invoke("project.open-file", json!({ "path": file }));
        }
        h.press(h.keys.recent);
        let s = h.invoke("recent-files.state", json!(null));
        assert_eq!(s["open"], json!(true));
        // The current file first, the one before it selected.
        assert_eq!(s["files"], json!(["app.py", "pkg/util.py", "notes.txt"]));
        assert_eq!(s["selected"], json!(1));
        assert_eq!(s["tool_windows"], json!(["Project", "Notifications"]));
        // Typing filters the lists.
        h.press("n o");
        let s = h.invoke("recent-files.state", json!(null));
        assert_eq!(s["files"], json!(["notes.txt"]));
        h.press("backspace backspace");
        assert_eq!(
            h.invoke("recent-files.state", json!(null))["filter"],
            json!("")
        );
        // Pressing it again: the edited files only.
        h.press(h.keys.recent);
        let s = h.invoke("recent-files.state", json!(null));
        assert_eq!(s["edited_only"], json!(true));
        assert_eq!(s["files"], json!([]));
        h.press(h.keys.recent);
        // Delete takes a file off the list and closes its tab.
        h.press("down");
        h.press("delete");
        let s = h.invoke("recent-files.state", json!(null));
        assert_eq!(s["files"], json!(["app.py", "pkg/util.py"]));
        // Enter opens the selected file.
        assert_eq!(
            h.invoke("recent-files.state", json!(null))["selected"],
            json!(1)
        );
        h.press("enter");
        let s = h.invoke("workbench.state", json!(null));
        assert_eq!(s["recent_files"], json!(false));
        let active = s["active_tab"].as_u64().unwrap() as usize;
        assert_eq!(s["tabs"][active]["path"], json!("pkg/util.py"));
        assert_eq!(s["tabs"].as_array().unwrap().len(), 2);
    });
}

impl H<'_> {
    /// Lets the preview follow the selection and read its file.
    fn preview_settle(&mut self) {
        self.cx
            .executor()
            .advance_clock(workbench::find_in_files::PREVIEW_DELAY * 2);
        self.draw();
    }

    fn preview(&mut self) -> Value {
        self.state()["preview"].clone()
    }

    fn mouse_move(&mut self, selector: &'static str) {
        let bounds = self
            .vcx()
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        self.vcx()
            .simulate_mouse_move(bounds.center(), None, Modifiers::none());
        self.draw();
    }

    fn preview_editor(&mut self) -> gpui::Entity<Editor> {
        self.window
            .read_with(self.cx, |w, cx| {
                w.search_everywhere().read(cx).preview_editor(cx)
            })
            .unwrap()
    }
}

#[gpui::test]
fn the_preview_follows_the_selection(cx: &mut TestAppContext) {
    both(cx, |h| {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("search-everywhere.toml");
        h.cx.update(|cx| workbench::search_everywhere::load_settings(cx, settings.clone()));
        h.press(h.keys.go_to_text);
        h.type_text("needle");
        h.settle();
        // Off by default: no preview.
        let p = h.preview();
        assert_eq!(p["on"], json!(false), "{p}");
        assert_eq!(p["shown"], json!(false));
        assert!(
            h.vcx()
                .debug_bounds("search-everywhere-preview-pane")
                .is_none()
        );
        // The header's button turns it on (remembered).
        h.click("search-everywhere-preview");
        h.preview_settle();
        let p = h.preview();
        assert_eq!(p["on"], json!(true), "{p}");
        assert_eq!(p["shown"], json!(true));
        assert!(
            h.vcx()
                .debug_bounds("search-everywhere-preview-pane")
                .is_some()
        );
        let saved = std::fs::read_to_string(&settings).unwrap();
        assert!(saved.contains("preview = true"), "{saved}");
        // The selected line's file at its line, every occurrence marked.
        let rows = h.state()["rows"].clone();
        let row_at = |i: usize| {
            (
                rows[i]["path"].as_str().unwrap().to_owned(),
                rows[i]["line"].as_u64().unwrap() - 1,
            )
        };
        let (path, line) = row_at(0);
        let view = h.preview()["view"].clone();
        assert_eq!(view["path"], json!(path), "{view}");
        assert_eq!(view["caret"][0], json!(line), "{view}");
        let occurrences = std::fs::read_to_string(h.root.join(&path))
            .unwrap()
            .matches("needle")
            .count();
        assert_eq!(view["highlights"], json!(occurrences), "{view}");
        assert_eq!(view["viewer"], json!(true), "{view}");
        // It follows the selection.
        h.press("down");
        h.preview_settle();
        let (path, line) = row_at(1);
        let view = h.preview()["view"].clone();
        assert_eq!(view["path"], json!(path), "{view}");
        assert_eq!(view["caret"][0], json!(line), "{view}");
        // Still on after the popup is opened again; the footer stays.
        h.press("escape");
        h.press(h.keys.go_to_text);
        h.settle();
        h.preview_settle();
        let s = h.state();
        assert_eq!(s["preview"]["shown"], json!(true), "{s}");
        assert!(!s["footer"].as_str().unwrap().is_empty(), "{s}");
        // Off again.
        h.invoke("search-everywhere.toggle-preview", json!(null));
        assert_eq!(h.preview()["on"], json!(false));
        h.press("escape");
    });
}

#[gpui::test]
fn the_preview_is_read_only_and_shows_no_preview_for_other_items(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.type_text("app.py");
        h.settle();
        h.preview_settle();
        let view = h.preview()["view"].clone();
        assert_eq!(view["path"], json!("app.py"), "{view}");
        assert_eq!(view["caret"], json!([0, 0]), "{view}");
        // A click puts the focus and the caret in the preview; typing
        // changes nothing and shows the read-only hint.
        h.click("search-everywhere-preview-area");
        let editor = h.preview_editor();
        let focused =
            h.cx.update_window(h.window.into(), |_, window, cx| {
                editor.read(cx).focus_handle(cx).is_focused(window)
            })
            .unwrap();
        assert!(focused, "the preview has the focus");
        // A key press (the test input sends an empty keystroke after the
        // text, which would hide the hint at once).
        h.press("x");
        let text = std::fs::read_to_string(h.root.join("app.py")).unwrap();
        assert_eq!(h.cx.update(|cx| editor.read(cx).text()), text);
        assert_eq!(
            h.preview()["view"]["read_only_hint"],
            json!(editor::READ_ONLY_HINT)
        );
        assert!(h.vcx().debug_bounds("editor-read-only-hint").is_some());
        assert_eq!(h.state()["open"], json!(true));
        // The next key hides it.
        h.press("right");
        assert_eq!(h.preview()["view"]["read_only_hint"], json!(null));
        // A folder is not a place in a file.
        h.press("escape");
        h.press(h.keys.go_to_file);
        h.press(h.keys.select_all);
        h.type_text("tests/");
        h.settle();
        h.preview_settle();
        assert_eq!(h.rows()[0], "folder:tests");
        let p = h.preview();
        assert_eq!(p["shown"], json!(true), "{p}");
        assert_eq!(
            p["view"]["empty"],
            json!("There is no preview for the selected item")
        );
        assert!(
            h.vcx()
                .debug_bounds("search-everywhere-preview-empty")
                .is_some()
        );
        // Off again (the next keymap's run shares the settings).
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.press("escape");
    });
}

#[gpui::test]
fn the_preview_marks_every_occurrence_until_the_caret_moves(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_text);
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.type_text("needle");
        h.settle();
        // The row of the comment in pkg/util.py (two occurrences there).
        let rows = h.state()["rows"].clone();
        let index = rows
            .as_array()
            .unwrap()
            .iter()
            .position(|r| r["path"] == json!("pkg/util.py") && r["line"] == json!(4))
            .expect("the comment's row");
        for _ in 0..index {
            h.press("down");
        }
        h.preview_settle();
        let view = h.preview()["view"].clone();
        assert_eq!(view["path"], json!("pkg/util.py"), "{view}");
        // "x = 1\n\n\n# needle here\nneedle = 2\n": the row's match at 10..16,
        // the other one on the next line at 22..28; the caret after the
        // row's match.
        assert_eq!(
            view["highlight_ranges"],
            json!([[10, 16], [22, 28]]),
            "{view}"
        );
        assert_eq!(view["highlight_background"], json!("occurrence"), "{view}");
        assert_eq!(view["caret"], json!([3, 8]), "{view}");
        // A click moves the caret in the preview: only the row's match
        // stays, as a found usage.
        h.click("search-everywhere-preview-area");
        let view = h.preview()["view"].clone();
        assert_ne!(view["caret"], json!([3, 8]), "{view}");
        assert_eq!(view["highlight_ranges"], json!([[10, 16]]), "{view}");
        assert_eq!(view["highlight_background"], json!("active"), "{view}");
        // A row shown again marks the occurrences again.
        h.invoke("search-everywhere.close", json!(null));
        h.press(h.keys.go_to_text);
        h.settle();
        h.preview_settle();
        let view = h.preview()["view"].clone();
        assert_eq!(view["highlight_background"], json!("occurrence"), "{view}");
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.press("escape");
    });
}

#[gpui::test]
fn the_soft_wrap_button_of_the_previews(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.type_text("app.py");
        h.settle();
        h.preview_settle();
        // Shown while the mouse moves over the preview, hidden a while
        // after it stopped.
        assert_eq!(h.preview()["view"]["soft_wrap_button"], json!(false));
        h.mouse_move("search-everywhere-preview-area");
        assert_eq!(h.preview()["view"]["soft_wrap_button"], json!(true));
        h.cx.executor().advance_clock(
            workbench::file_preview::SHOWING + workbench::file_preview::RETENTION * 2,
        );
        h.draw();
        assert_eq!(h.preview()["view"]["soft_wrap_button"], json!(false));
        // A click turns soft wraps on in the previews (remembered).
        h.mouse_move("search-everywhere-preview-area");
        h.click("search-everywhere-preview-soft-wrap");
        assert_eq!(h.preview()["view"]["soft_wraps"], json!(true));
        assert_eq!(h.state()["open"], json!(true));
        let fif = h.invoke("find-in-files.state", json!(null));
        let on =
            h.cx.update(|cx| workbench::file_preview::settings(cx).soft_wraps);
        assert!(on, "{fif}");
        h.invoke("preview.toggle-soft-wraps", json!(null));
        assert_eq!(h.preview()["view"]["soft_wraps"], json!(false));
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.press("escape");
    });
}

// ---- moving the popup and the splitter --------------------------------------

impl H<'_> {
    fn bounds(&mut self, selector: &'static str) -> Bounds<Pixels> {
        self.vcx()
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
    }

    fn popup_at(&mut self) -> Point<Pixels> {
        self.bounds("search-everywhere").origin
    }

    /// A point on the header's free space, between the tabs and the
    /// toolbar.
    fn on_header(&mut self) -> Point<Pixels> {
        let tab = self.bounds("search-everywhere-tab-text");
        let toolbar = ["search-everywhere-everywhere", "search-everywhere-scope"]
            .into_iter()
            .find_map(|s| self.vcx().debug_bounds(s))
            .expect("the toolbar is drawn");
        assert!(
            toolbar.left() - tab.right() > px(20.0),
            "{tab:?} {toolbar:?}"
        );
        point((tab.right() + toolbar.left()) / 2.0, tab.center().y)
    }

    fn dragging(&mut self) -> Value {
        self.state()["dragging"].clone()
    }

    fn split(&mut self) -> f64 {
        self.preview()["split"].as_f64().unwrap()
    }

    /// Each event in its own frame.
    fn send(&mut self, events: Vec<PlatformInput>) {
        mouse::send(self.cx, self.window.into(), events);
    }

    /// The events with no frame between them that they did not ask for.
    fn quick(&mut self, events: Vec<PlatformInput>) {
        mouse::quick(self.cx, self.window.into(), events);
    }

    /// The window loses the focus to another application, and gets it back
    /// once `check` ran.
    fn away(&mut self, check: impl FnOnce(&mut Self)) {
        self.vcx().deactivate_window();
        self.draw();
        check(self);
        self.cx
            .update_window(self.window.into(), |_, window, _| window.activate_window())
            .unwrap();
        self.draw();
    }
}

fn offset(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

#[gpui::test]
fn the_popup_moves_only_while_its_header_is_held(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_file);
        let start = h.popup_at();
        let idle = json!({ "popup": false, "split": false });
        // A quick click on the header leaves no move (the release before
        // the next frame: tests/popup_frame.rs).
        let at = h.on_header();
        h.quick(vec![press(at), release(at)]);
        assert_eq!(h.dragging(), idle);
        h.send(vec![hover(at + offset(80.0, 60.0))]);
        assert_eq!(h.popup_at(), start);
        // A click on a tab selects it and does not move the popup.
        let tab = h.bounds("search-everywhere-tab-text").center();
        h.quick(vec![press(tab), release(tab)]);
        assert_eq!(h.state()["tab"], json!("text"));
        assert_eq!(h.dragging(), idle);
        h.send(vec![press(tab), drag_to(tab + offset(30.0, 20.0))]);
        assert_eq!(h.popup_at(), start, "not moved from a tab");
        h.send(vec![release(tab + offset(30.0, 20.0))]);
        // A button in the header neither.
        let button = h.bounds("search-everywhere-preview").center();
        h.send(vec![press(button), drag_to(button + offset(-30.0, 20.0))]);
        assert_eq!(h.dragging(), idle);
        assert_eq!(h.popup_at(), start, "not moved from a button");
        h.send(vec![release(button + offset(-30.0, 20.0))]);
        // Held on the header: the popup follows the mouse; a release
        // outside the window ends the move.
        let at = h.on_header();
        h.send(vec![press(at), drag_to(at + offset(30.0, 20.0))]);
        assert_eq!(h.dragging()["popup"], json!(true));
        assert_eq!(h.popup_at(), start + offset(30.0, 20.0));
        h.send(vec![release(at + offset(30.0, 20.0) - offset(0.0, 2000.0))]);
        assert_eq!(h.dragging(), idle);
        let moved = h.popup_at();
        h.send(vec![hover(at + offset(120.0, 90.0))]);
        assert_eq!(h.popup_at(), moved);
        // A lost release: the next move has no button and ends the move.
        let at = h.on_header();
        h.send(vec![press(at), drag_to(at + offset(20.0, 30.0))]);
        h.send(vec![hover(at + offset(200.0, 100.0))]);
        assert_eq!(h.dragging(), idle);
        let moved = moved + offset(20.0, 30.0);
        assert_eq!(h.popup_at(), moved);
        // The window losing the focus ends a move (the popup stays open).
        let at = h.on_header();
        h.send(vec![press(at), drag_to(at + offset(10.0, 10.0))]);
        h.away(|h| {
            assert_eq!(h.dragging(), json!({ "popup": false, "split": false }));
            assert_eq!(h.state()["open"], json!(true));
            h.send(vec![drag_to(at + offset(90.0, 90.0)), release(at)]);
        });
        assert_eq!(h.popup_at(), moved + offset(10.0, 10.0));
        h.press("escape");
    });
}

#[gpui::test]
fn the_preview_splitter_moves_only_while_held(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.press(h.keys.go_to_text);
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.type_text("needle");
        h.settle();
        h.preview_settle();
        let split = json!({ "popup": false, "split": true });
        let idle = json!({ "popup": false, "split": false });
        let start = h.split();
        // A quick click on the splitter: no drag is left.
        let bar = h.bounds("search-everywhere-splitter").center();
        h.quick(vec![press(bar), release(bar)]);
        assert_eq!(h.dragging(), idle);
        h.send(vec![hover(bar + offset(0.0, 60.0))]);
        assert_eq!(h.split(), start);
        // Held: the split follows; a lost release ends the drag.
        h.send(vec![press(bar), drag_to(bar + offset(0.0, 40.0))]);
        assert_eq!(h.dragging(), split);
        let moved = h.split();
        assert!(moved > start + 0.02, "{start} -> {moved}");
        h.send(vec![hover(bar + offset(0.0, 120.0))]);
        assert_eq!(h.dragging(), idle);
        assert_eq!(h.split(), moved);
        // A release outside the window ends it there, and keeps it.
        let bar = h.bounds("search-everywhere-splitter").center();
        h.send(vec![press(bar), drag_to(bar + offset(0.0, -30.0))]);
        h.send(vec![release(point(px(-20.0), bar.y - px(40.0)))]);
        assert_eq!(h.dragging(), idle);
        let kept = h.split();
        assert!(kept < moved, "{moved} -> {kept}");
        let saved =
            h.cx.update(|cx| workbench::search_everywhere::settings(cx).preview_split);
        assert!((f64::from(saved.unwrap()) - kept).abs() < 1e-6, "{saved:?}");
        // The window losing the focus ends it.
        let bar = h.bounds("search-everywhere-splitter").center();
        h.send(vec![press(bar), drag_to(bar + offset(0.0, 10.0))]);
        assert_eq!(h.dragging(), split);
        h.away(|h| {
            assert_eq!(h.dragging(), json!({ "popup": false, "split": false }));
            h.send(vec![release(bar)]);
        });
        h.invoke("search-everywhere.toggle-preview", json!(null));
        h.press("escape");
    });
}
