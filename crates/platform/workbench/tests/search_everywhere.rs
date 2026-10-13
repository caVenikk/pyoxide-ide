//! Search Everywhere and Go to File (tests/scenarios/editor/search-everywhere.md),
//! driven by real keystrokes and modifier events through both default
//! keymaps in a headless main window over a project on disk.

use std::rc::Rc;
use std::time::Duration;

use editor::Editor;
use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
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
        assert_eq!(s["tool_windows"], json!(["Project"]));
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
