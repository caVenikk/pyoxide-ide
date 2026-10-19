//! The pointer shape over the main window's elements (#181), read from
//! `ui::pointer::resolve` after synthetic mouse moves in a headless
//! window: links and link-like texts show the hand, lists of popups show
//! the hand, buttons keep the arrow, and a button over an editor does not
//! show the editor's text cursor.

use std::rc::Rc;

use gpui::{
    AppContext as _, CursorStyle, Modifiers, Pixels, Point, TestAppContext, VisualTestContext,
    WindowHandle,
};
use serde_json::{Value, json};
use workbench::keymap::{self, Keymap};
use workbench::search_everywhere::DEBOUNCE;
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    _dir: tempfile::TempDir,
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

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(self.window.into(), text);
        self.draw();
    }

    fn settle(&mut self) {
        self.cx.executor().advance_clock(DEBOUNCE * 2);
        self.draw();
    }

    fn vcx(&mut self) -> VisualTestContext {
        VisualTestContext::from_window(self.window.into(), self.cx)
    }

    fn center(&mut self, selector: &'static str) -> Point<Pixels> {
        self.vcx()
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
            .center()
    }

    /// The shape with the mouse at `at`.
    fn shape_at(&mut self, at: Point<Pixels>) -> CursorStyle {
        self.vcx().simulate_mouse_move(at, None, Modifiers::none());
        self.draw();
        self.vcx().update(|window, _| ui::pointer::resolve(window))
    }

    /// The shape with the mouse over the element with this debug selector.
    fn shape_over(&mut self, selector: &'static str) -> CursorStyle {
        let at = self.center(selector);
        self.shape_at(at)
    }

    fn click(&mut self, selector: &'static str) {
        let at = self.center(selector);
        self.vcx().simulate_click(at, Modifiers::none());
        self.draw();
    }

    /// Opens Go to File with `text` typed.
    fn go_to_file(&mut self, text: &str) {
        self.press("cmd-shift-o");
        self.type_text(text);
        self.settle();
    }
}

fn setup(cx: &mut TestAppContext) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    for (path, text) in [
        ("app.py", "import os\n\ndef main():\n    print(\"x\")\n"),
        ("notes.txt", "a note\n"),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let keymap = Keymap::parse(keymap::MACOS).unwrap();
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
        _dir: dir,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    // The index is built in the background.
    h.draw();
    let state = h.invoke("workbench.state", json!(null));
    assert!(
        state["file_index"].as_u64().is_some_and(|n| n > 0),
        "{state}"
    );
    h
}

#[gpui::test]
fn the_links_of_search_everywhere_show_the_hand(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    // Nothing matches: the empty text with its links.
    h.go_to_file("zzzz");
    // In the project scope, the empty text offers the scope "all places".
    h.invoke("search-everywhere.toggle-everywhere", json!(null));
    h.settle();
    for link in [
        "search-everywhere-set-scope",
        "search-everywhere-find-in-files",
    ] {
        assert_eq!(h.shape_over(link), CursorStyle::PointingHand, "{link}");
    }
    // Text next to them is not a link.
    let empty = h.vcx().debug_bounds("search-everywhere-empty").unwrap();
    let beside = gpui::point(empty.left() + gpui::px(10.0), empty.center().y);
    assert_eq!(h.shape_at(beside), CursorStyle::Arrow);
    // The footer's "Open In Right Split".
    h.invoke("search-everywhere.close", json!(null));
    h.go_to_file("app");
    assert_eq!(
        h.shape_over("search-everywhere-right-split"),
        CursorStyle::PointingHand
    );
    // The history icon in the field; the buttons of the header keep the
    // arrow.
    assert_eq!(
        h.shape_over("search-everywhere-history"),
        CursorStyle::PointingHand
    );
    assert_eq!(
        h.shape_over("search-everywhere-preview"),
        CursorStyle::Arrow
    );
    assert_eq!(
        h.shape_over("search-everywhere-tab-files"),
        CursorStyle::Arrow
    );
}

#[gpui::test]
fn the_lists_of_search_everywhere_popups_show_the_hand(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.go_to_file("app");
    // The scope list: the hand over its rows.
    h.invoke("search-everywhere.choose-scope", json!(null));
    assert_eq!(
        h.shape_over("search-everywhere-scope-0"),
        CursorStyle::PointingHand
    );
    h.invoke("search-everywhere.close", json!(null));
    // The filter list: its rows keep the arrow, All, None and Invert are
    // links.
    h.go_to_file("app");
    h.click("search-everywhere-filter");
    assert_eq!(
        h.shape_over("search-everywhere-filter-0"),
        CursorStyle::Arrow
    );
    for link in [
        "search-everywhere-filter-all",
        "search-everywhere-filter-none",
        "search-everywhere-filter-invert",
    ] {
        assert_eq!(h.shape_over(link), CursorStyle::PointingHand, "{link}");
    }
}

#[gpui::test]
fn the_soft_wrap_button_over_a_preview_does_not_show_the_text_cursor(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    h.go_to_file("app.py");
    h.invoke("search-everywhere.toggle-preview", json!(null));
    h.cx.executor()
        .advance_clock(workbench::find_in_files::PREVIEW_DELAY * 2);
    h.draw();
    // The button shows while the mouse moves over the preview.
    let area = h.center("search-everywhere-preview-area");
    assert_eq!(h.shape_at(area), CursorStyle::IBeam, "the preview's editor");
    assert_eq!(
        h.shape_over("search-everywhere-preview-soft-wrap"),
        CursorStyle::Arrow
    );
    // The same over the button's frame, next to the button.
    let button = h.vcx().debug_bounds("search-everywhere-preview-soft-wrap");
    let button = button.expect("the button is drawn");
    let next_to = gpui::point(button.right() + gpui::px(2.0), button.center().y);
    assert_eq!(h.shape_at(next_to), CursorStyle::Arrow);
}
