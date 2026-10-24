//! Back and Forward in a headless main window (tests/scenarios/editor/
//! navigation-history.md): the places that opening files, switching and
//! closing tabs, jumps of the caret (Go to Line, a click elsewhere, Text
//! Start and End, the next problem, Find Next from the editor, Go to
//! Declaration) leave,
//! going back and forward to them by command and with the mouse's back
//! and forward buttons, and the caret's line put back where it was in the
//! view.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::fake::FakeCodeIntel;
use code_intel::{CodeIntel, Location};
use editor::{DiagnosticMark, Editor, MarkMessage, MarkSeverity};
use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, NavigationDirection,
    TestAppContext, VisualTestContext, WindowHandle, point, px,
};
use serde_json::{Value, json};
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
        self.draw();
    }

    fn back(&mut self) {
        self.invoke("workbench.back", json!(null));
    }

    fn forward(&mut self) {
        self.invoke("workbench.forward", json!(null));
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    /// Clicks into the active editor's text at a zero-based line and
    /// column.
    fn click_text(&mut self, line: usize, column: usize, modifiers: Modifiers) {
        let editor = self.editor();
        let (origin, line_height, char_width) = self
            .cx
            .update(|cx| editor.read(cx).text_geometry())
            .expect("the editor is drawn");
        let at = point(
            origin.x + char_width * (column as f32 + 0.2),
            origin.y + line_height * (line as f32 + 0.5),
        );
        let mut cx = VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_mouse_down(at, MouseButton::Left, modifiers);
        cx.simulate_mouse_up(at, MouseButton::Left, modifiers);
        self.draw();
    }

    /// Presses and releases a mouse button over the editor.
    fn mouse_button(&mut self, direction: NavigationDirection) {
        let at = point(px(700.0), px(400.0));
        let button = MouseButton::Navigate(direction);
        let mut cx = VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_mouse_down(at, button, Modifiers::none());
        cx.simulate_mouse_up(at, button, Modifiers::none());
        self.draw();
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

/// A project with a.py, b.py and c.py of 200 lines each (`line N`) and
/// f.py with `needle` on lines 5, 50 and 120; Python files have language
/// services (an in-memory fake).
fn setup(cx: &mut TestAppContext) -> (H<'_>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    for name in ["a.py", "b.py", "c.py"] {
        std::fs::write(root.join(name), &text).unwrap();
    }
    let needles: String = (0..200)
        .map(|i| {
            if [5, 50, 120].contains(&i) {
                "needle\n".to_owned()
            } else {
                format!("hay {i}\n")
            }
        })
        .collect();
    std::fs::write(root.join("f.py"), needles).unwrap();
    let fake = FakeCodeIntel::new();
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        code_intel: vec![CodeIntelHook::new(Provider(fake.clone()))],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    let mut h = H {
        cx,
        window,
        root,
        fake,
    };
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
fn closing_another_tab_records_the_current_place(cx: &mut TestAppContext) {
    // The reference selects the current tab again, which is a navigation.
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.open("c.py");
    h.back();
    assert_eq!(h.history(), (vec![p("a.py", 0)], vec![p("c.py", 0)]));
    h.caret_to(30, 0);
    h.invoke("tabs.close", json!({ "index": 0 }));
    assert_eq!(h.place(), ("b.py".into(), 30, 0));
    assert_eq!(h.history(), (vec![p("a.py", 0), p("b.py", 30)], vec![]));
}

#[gpui::test]
fn closing_the_last_tab_records_nothing(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.caret_to(30, 0);
    h.invoke("tabs.close", json!(null));
    assert_eq!(h.history(), (vec![], vec![]));
}

#[gpui::test]
fn places_in_removed_files_are_skipped(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.open("c.py");
    h.invoke("tabs.select", json!({ "index": 1 }));
    // b.py closes (its place is recorded) and the file goes away.
    h.invoke("tabs.close", json!(null));
    std::fs::remove_file(h.root.join("b.py")).unwrap();
    assert_eq!(
        h.history().0,
        [p("a.py", 0), p("b.py", 0), p("c.py", 0), p("b.py", 0)]
    );

    h.back();
    assert_eq!(h.place().0, "c.py");
    assert_eq!(h.history(), (vec![p("a.py", 0)], vec![p("a.py", 0)]));
}

#[gpui::test]
fn another_project_forgets_the_places(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    assert!(!h.history().0.is_empty());
    let other = tempfile::tempdir().unwrap();
    let folder = native_path::canonicalize(other.path()).unwrap();
    h.invoke(
        "project.open-folder",
        json!({ "path": folder.display().to_string() }),
    );
    assert_eq!(h.history(), (vec![], vec![]));
}

#[gpui::test]
fn text_start_and_end_record_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.caret_to(30, 0);
    // Caret keys are no navigation.
    h.invoke("editor.move-down", json!(null));
    assert_eq!(h.history().0, []);
    h.invoke("editor.move-document-end", json!(null));
    h.invoke("editor.move-document-start", json!(null));
    assert_eq!(h.history().0, [p("a.py", 31), p("a.py", 200)]);
    // With a selection they are no navigation.
    h.invoke("editor.select-document-end", json!(null));
    assert_eq!(h.history().0, [p("a.py", 31), p("a.py", 200)]);

    h.caret_to(0, 0);
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 200, 0));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 31, 0));
}

#[gpui::test]
fn the_next_problem_records_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    let editor = h.editor();
    h.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            let marks = [100, 150]
                .into_iter()
                .map(|line| {
                    let start = e.buffer().line_start(line);
                    DiagnosticMark {
                        range: start..start + 4,
                        severity: MarkSeverity::Error,
                        unnecessary: false,
                        deprecated: false,
                        messages: vec![MarkMessage {
                            severity: MarkSeverity::Error,
                            text: "a problem".into(),
                            source: None,
                            code: None,
                            href: None,
                        }],
                    }
                })
                .collect();
            e.set_diagnostics(marks, cx);
        })
    });
    h.invoke("code.next-problem", json!(null));
    h.invoke("code.next-problem", json!(null));
    assert_eq!(h.place(), ("a.py".into(), 150, 0));
    assert_eq!(h.history().0, [p("a.py", 0), p("a.py", 100)]);
    h.invoke("code.previous-problem", json!(null));
    assert_eq!(
        h.history().0,
        [p("a.py", 0), p("a.py", 100), p("a.py", 150)]
    );
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 150, 0));
}

#[gpui::test]
fn a_click_elsewhere_records_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.focus_editor();
    h.click_text(10, 2, Modifiers::none());
    assert_eq!(h.place(), ("a.py".into(), 10, 2));
    assert_eq!(h.history().0, [p("a.py", 0)]);
    h.click_text(20, 1, Modifiers::none());
    // On the caret's line, or extending the selection: no navigation.
    h.click_text(20, 4, Modifiers::none());
    h.click_text(25, 0, Modifiers::shift());
    assert_eq!(h.history().0, [p("a.py", 0), p("a.py", 10)]);

    h.back();
    assert_eq!(h.place(), ("a.py".into(), 10, 2));
}

#[gpui::test]
fn find_next_in_the_editor_records_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("f.py");
    h.invoke("find.search", json!({ "text": "needle" }));
    assert_eq!(h.place().1, 5);
    // In the find bar the keys move between the matches only.
    h.invoke("find.next", json!(null));
    assert_eq!(h.place().1, 50);
    assert_eq!(h.history().0, []);
    // From the editor, Find Next is a navigation.
    h.focus_editor();
    h.invoke("find.next", json!(null));
    assert_eq!(h.place().1, 120);
    assert_eq!(h.history().0, [p("f.py", 50)]);
    h.back();
    assert_eq!(h.place().1, 50);
}

#[gpui::test]
fn go_to_declaration_records_the_place_left(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let (a, b) = (h.root.join("a.py"), h.root.join("b.py"));
    h.fake.on_definition(move |_, at| {
        let to = |path: &PathBuf, line| Location {
            path: path.clone(),
            range: code_intel::Point::new(line, 0)..code_intel::Point::new(line, 4),
        };
        Ok(match at.line {
            100 => vec![to(&a, 10)],
            10 => vec![to(&b, 150)],
            _ => Vec::new(),
        })
    });
    h.open("a.py");
    h.caret_to(100, 2);
    h.focus_editor();
    // In the file, then in another one.
    h.invoke("code.go-to-declaration", json!(null));
    assert_eq!(h.place(), ("a.py".into(), 10, 0));
    h.invoke("code.go-to-declaration", json!(null));
    assert_eq!(h.place(), ("b.py".into(), 150, 0));
    assert_eq!(h.history().0, [p("a.py", 100), p("a.py", 10)]);

    h.back();
    assert_eq!(h.place(), ("a.py".into(), 10, 0));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 100, 2));
    h.forward();
    assert_eq!(h.place(), ("a.py".into(), 10, 0));
}

#[gpui::test]
fn the_mouse_buttons_go_back_and_forward(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.open("b.py");
    h.mouse_button(NavigationDirection::Back);
    assert_eq!(h.place().0, "a.py");
    h.mouse_button(NavigationDirection::Forward);
    assert_eq!(h.place().0, "b.py");
}

#[gpui::test]
fn back_puts_the_line_where_it_was_in_the_view(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.invoke("editor.scroll-to", json!({ "line": 50 }));
    h.caret_to(53, 0);
    let editor = h.editor();
    let view_y = h.cx.update(|cx| editor.read(cx).caret_view_y()).unwrap();
    assert!(view_y > px(0.0));
    h.open("b.py");
    assert_eq!(
        h.state()["navigation"]["back"][0]["view_y"],
        json!(f32::from(view_y))
    );

    h.back();
    assert_eq!(h.place(), ("a.py".into(), 53, 0));
    let state = h.cx.update(|cx| {
        let e = editor.read(cx);
        (e.caret_view_y(), e.visible_lines().map(|l| l.start))
    });
    assert_eq!(state, (Some(view_y), Some(50)));

    // A place in a file read again: the same.
    h.invoke("tabs.close", json!({ "index": 0 }));
    h.back();
    assert_eq!(h.place(), ("a.py".into(), 53, 0));
    let editor = h.editor();
    let reopened = h.cx.update(|cx| {
        let e = editor.read(cx);
        (e.caret_view_y(), e.visible_lines().map(|l| l.start))
    });
    assert_eq!(reopened, (Some(view_y), Some(50)));
}
