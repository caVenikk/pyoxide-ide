//! Back and Forward in a headless main window: the places that opening
//! files, switching and closing tabs and jumps within a file leave, and
//! going back and forward to them.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
}

impl H<'_> {
    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        out
    }

    fn state(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))
    }

    fn open(&mut self, path: &str) {
        self.invoke("project.open-file", json!({ "path": path }));
    }

    /// Puts the active editor's caret at a zero-based line and column
    /// (not a navigation).
    fn caret_to(&mut self, line: usize, column: usize) {
        let at = json!({ "line": line, "column": column });
        self.invoke("editor.set-selection", json!({ "anchor": at, "head": at }));
    }

    /// Opens a file with the caret at a place, as a search result or a
    /// declaration does.
    fn open_at(&mut self, path: &str, line: usize, column: usize) {
        let path = self.root.join(path);
        self.cx
            .update_window(self.window.into(), |root, window, cx| {
                let workbench = root.downcast::<Workbench>().unwrap();
                workbench.update(cx, |w, cx| {
                    w.open_file_at(path, text::Point::new(line, column), None, window, cx)
                })
            })
            .unwrap();
        self.cx.run_until_parked();
    }

    fn back(&mut self) {
        self.invoke("workbench.back", json!(null));
    }

    fn forward(&mut self) {
        self.invoke("workbench.forward", json!(null));
    }

    /// The active file (relative to the project) and its caret.
    fn place(&mut self) -> (String, usize, usize) {
        let s = self.state();
        let tab = s["active_tab"].as_u64().expect("a tab is active") as usize;
        let caret = &s["editor"]["caret"];
        (
            s["tabs"][tab]["path"].as_str().unwrap().to_owned(),
            caret["line"].as_u64().unwrap() as usize,
            caret["column"].as_u64().unwrap() as usize,
        )
    }

    /// The places Back and Forward go to, the next one last.
    fn history(&mut self) -> (Places, Places) {
        let s = self.state();
        let list = |v: &Value| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["path"].as_str().unwrap().to_owned(),
                        p["line"].as_u64().unwrap() as usize,
                    )
                })
                .collect()
        };
        (
            list(&s["navigation"]["back"]),
            list(&s["navigation"]["forward"]),
        )
    }
}

/// Places as (file, line).
type Places = Vec<(String, usize)>;

fn p(path: &str, line: usize) -> (String, usize) {
    (path.to_owned(), line)
}

/// A project with a.py, b.py and c.py of 60 lines each.
fn setup(cx: &mut TestAppContext) -> (H<'_>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let text: String = (0..60).map(|i| format!("line {i}\n")).collect();
    for name in ["a.py", "b.py", "c.py"] {
        std::fs::write(root.join(name), &text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    let mut h = H { cx, window, root };
    let folder = h.root.display().to_string();
    h.invoke("project.open-folder", json!({ "path": folder }));
    (h, dir)
}

#[gpui::test]
fn goes_back_and_forward_between_files(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.caret_to(10, 2);
    h.open("b.py");
    h.caret_to(5, 1);
    h.open("c.py");
    assert_eq!(h.history(), (vec![p("a.py", 10), p("b.py", 5)], vec![]));

    h.back();
    assert_eq!(h.place(), ("b.py".into(), 5, 1));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 10, 2));
    assert_eq!(h.history(), (vec![], vec![p("c.py", 0), p("b.py", 5)]));
    // Nothing more to go back to.
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 10, 2));

    h.forward();
    assert_eq!(h.place(), ("b.py".into(), 5, 1));
    h.forward();
    assert_eq!(h.place(), ("c.py".into(), 0, 0));
    h.forward();
    assert_eq!(h.place(), ("c.py".into(), 0, 0));
    assert_eq!(h.history(), (vec![p("a.py", 10), p("b.py", 5)], vec![]));
}

#[gpui::test]
fn switching_tabs_records_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.caret_to(7, 0);
    h.invoke("tabs.select", json!({ "index": 0 }));
    h.invoke("tabs.next", json!(null));
    h.invoke("tabs.previous", json!(null));
    assert_eq!(
        h.history().0,
        [p("a.py", 0), p("b.py", 7), p("a.py", 0), p("b.py", 7)]
    );
    h.back();
    assert_eq!(h.place(), ("b.py".into(), 7, 0));
}

#[gpui::test]
fn a_new_navigation_drops_the_forward_places(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.open("c.py");
    h.back();
    h.back();
    assert_eq!(h.place().0, "a.py");
    assert_eq!(h.history().1.len(), 2);

    h.invoke("tabs.select", json!({ "index": 2 }));
    assert_eq!(h.history(), (vec![p("a.py", 0)], vec![]));
    h.forward();
    assert_eq!(h.place().0, "c.py", "nothing to go forward to");
}

#[gpui::test]
fn jumps_within_a_file_record_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open_at("a.py", 30, 4);
    h.open_at("a.py", 32, 0);
    // Less than four lines from the last place: it takes its slot.
    h.open_at("a.py", 34, 0);
    assert_eq!(h.history().0, [p("a.py", 0), p("a.py", 32)]);
    // A jump on the caret's line is no navigation.
    h.open_at("a.py", 34, 6);
    assert_eq!(h.history().0, [p("a.py", 0), p("a.py", 32)]);

    h.back();
    assert_eq!(h.place(), ("a.py".into(), 32, 0));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 0, 0));
    h.forward();
    assert_eq!(h.place(), ("a.py".into(), 32, 0));
    h.forward();
    assert_eq!(h.place(), ("a.py".into(), 34, 6));
}

#[gpui::test]
fn a_jump_to_another_file_records_once(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.invoke("tabs.select", json!({ "index": 0 }));
    h.caret_to(20, 0);
    // An open file and one that is read first.
    h.open_at("b.py", 40, 0);
    h.open_at("c.py", 50, 0);
    assert_eq!(
        h.history().0,
        [p("a.py", 0), p("b.py", 0), p("a.py", 20), p("b.py", 40)]
    );
    h.back();
    assert_eq!(h.place(), ("b.py".into(), 40, 0));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 20, 0));
}

#[gpui::test]
fn go_to_line_records_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.caret_to(3, 0);
    h.invoke("workbench.go-to-line", json!(null));
    // The field's text is selected; typing replaces it.
    h.cx.simulate_input(h.window.into(), "45");
    h.invoke("go-to-line.confirm", json!(null));
    assert_eq!(h.place(), ("a.py".into(), 44, 0));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 3, 0));
}

#[gpui::test]
fn back_opens_a_closed_file_again(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.caret_to(12, 3);
    h.invoke("tabs.close", json!(null));
    assert_eq!(h.place().0, "a.py");

    h.back();
    assert_eq!(h.place(), ("b.py".into(), 12, 3));
    assert_eq!(
        h.history().1,
        [p("a.py", 0)],
        "reading the file kept Forward's place"
    );
    h.forward();
    assert_eq!(h.place().0, "a.py");
}

#[gpui::test]
fn places_in_removed_files_are_skipped(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.open("c.py");
    // b.py's tab closes (not the active one: nothing recorded) and the
    // file goes away.
    h.invoke("tabs.close", json!({ "index": 1 }));
    std::fs::remove_file(h.root.join(Path::new("b.py"))).unwrap();

    h.back();
    assert_eq!(h.place().0, "a.py");
    assert_eq!(h.history(), (vec![], vec![p("c.py", 0)]));
}

#[gpui::test]
fn another_project_forgets_the_places(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    assert!(!h.history().0.is_empty());
    let other = tempfile::tempdir().unwrap();
    let folder = std::fs::canonicalize(other.path()).unwrap();
    h.invoke(
        "project.open-folder",
        json!({ "path": folder.display().to_string() }),
    );
    assert_eq!(h.history(), (vec![], vec![]));
}
