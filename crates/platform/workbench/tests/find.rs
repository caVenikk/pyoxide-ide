//! Find and replace in the editor (tests/scenarios/editor/find-replace.md),
//! driven by real keystrokes through both default keymaps in a headless
//! main window.

use std::rc::Rc;

use editor::Editor;
use gpui::{
    AppContext as _, Entity, Focusable as _, Keystroke, Modifiers, TestAppContext, WindowHandle,
};
use serde_json::{Value, json};
use text::{Buffer, Selection, SelectionSet};
use workbench::keymap::{self, Keymap};
use workbench::{Product, Workbench};

/// Caret, or anchor and head of a selection, written into the text.
const CARET: char = '|';
const ANCHOR: char = '‹';
const HEAD: char = '›';

fn parse_marked(marked: &str) -> (String, Selection) {
    let mut text = String::new();
    let (mut caret, mut anchor, mut head) = (None, None, None);
    for c in marked.chars() {
        let pos = text.chars().count();
        match c {
            CARET => caret = Some(pos),
            ANCHOR => anchor = Some(pos),
            HEAD => head = Some(pos),
            c => text.push(c),
        }
    }
    let selection = match (caret, anchor, head) {
        (Some(p), None, None) => Selection::caret(p),
        (None, Some(a), Some(h)) => Selection::new(a, h),
        _ => panic!("bad markers in {marked:?}"),
    };
    (text, selection)
}

fn render_marked(text: &str, s: &Selection) -> String {
    let mut marks = if s.is_empty() {
        vec![(s.head, CARET)]
    } else {
        vec![(s.anchor, ANCHOR), (s.head, HEAD)]
    };
    marks.sort_by_key(|m| std::cmp::Reverse(m.0));
    let mut chars: Vec<char> = text.chars().collect();
    for (pos, mark) in marks {
        chars.insert(pos, mark);
    }
    chars.into_iter().collect()
}

/// The keys of one default keymap.
struct Keys {
    find: &'static str,
    replace: &'static str,
    next: &'static str,
    previous: &'static str,
    undo: &'static str,
    to_editor: &'static str,
    match_case: &'static str,
    words: &'static str,
    regex: &'static str,
    in_selection: &'static str,
    replace_all: &'static str,
}

const MAC: Keys = Keys {
    find: "cmd-f",
    replace: "cmd-r",
    next: "cmd-g",
    previous: "cmd-shift-g",
    undo: "cmd-z",
    to_editor: "cmd-enter",
    match_case: "alt-c",
    words: "alt-w",
    regex: "alt-x",
    in_selection: "ctrl-alt-e",
    replace_all: "ctrl-alt-a",
};

const WIN: Keys = Keys {
    find: "ctrl-f",
    replace: "ctrl-r",
    next: "f3",
    previous: "shift-f3",
    undo: "ctrl-z",
    to_editor: "ctrl-enter",
    match_case: "alt-c",
    words: "alt-w",
    regex: "alt-x",
    in_selection: "ctrl-alt-e",
    replace_all: "alt-a",
};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    keys: &'static Keys,
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

    fn find(&mut self) -> Value {
        self.invoke("find.state", json!(null))
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn set_editor(&mut self, marked: &str) {
        let (text, selection) = parse_marked(marked);
        let editor = self.editor();
        self.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                *e.buffer_mut() = Buffer::new(&text);
                e.buffer_reloaded(cx);
                e.set_selections(SelectionSet::single(selection), cx);
            })
        });
        self.focus_editor();
    }

    fn marked(&mut self) -> String {
        let editor = self.editor();
        self.cx.update(|cx| {
            let e = editor.read(cx);
            render_marked(&e.text(), e.selections().primary())
        })
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn focus_of(&mut self, field: fn(&editor::FindBar) -> &Entity<Editor>) -> bool {
        let bar = self
            .window
            .read_with(self.cx, |w, _| w.active_find_bar().cloned())
            .unwrap()
            .unwrap();
        let field = self.cx.update(|cx| field(bar.read(cx)).clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn query_focused(&mut self) -> bool {
        self.focus_of(editor::FindBar::query_field)
    }

    fn replace_focused(&mut self) -> bool {
        self.focus_of(editor::FindBar::replace_field)
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self.editor();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn highlighted(&mut self) -> usize {
        let editor = self.editor();
        self.cx.update(|cx| {
            editor
                .read(cx)
                .search_highlights()
                .map_or(0, |h| h.matches.len())
        })
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str, keys: &'static Keys) -> H<'a> {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a.txt"), "").unwrap();
    let product = Product {
        name: "Test IDE",
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
    let mut h = H {
        cx,
        window,
        keys,
        _dir: dir,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    h.invoke("project.open-file", json!({ "path": "a.txt" }));
    h
}

fn both(cx: &mut TestAppContext, test: fn(&mut H)) {
    for (text, keys) in [(keymap::MACOS, &MAC), (keymap::DEFAULT, &WIN)] {
        let mut h = setup(cx, text, keys);
        test(&mut h);
    }
}

#[gpui::test]
fn typing_searches_and_enter_moves_through_matches(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("|alpha beta alpha gamma alpha");
        h.press(h.keys.find);
        let s = h.find();
        assert_eq!(s["open"], json!(true));
        assert_eq!(s["text"], json!(""), "no selection, no earlier search");
        assert_eq!(s["status"], json!("0 results"));
        assert!(h.query_focused());
        // Incremental: every key searches again, the current match stays at
        // the caret.
        h.type_text("a");
        assert_eq!(h.find()["matches"], json!(9));
        h.type_text("lpha");
        let s = h.find();
        assert_eq!(s["matches"], json!(3));
        assert_eq!(s["status"], json!("1/3"));
        assert_eq!(h.highlighted(), 3, "all matches are highlighted");
        assert_eq!(h.marked(), "‹alpha› beta alpha gamma alpha");
        // Enter and Shift+Enter move; the match is selected.
        h.press("enter");
        assert_eq!(h.marked(), "alpha beta ‹alpha› gamma alpha");
        assert_eq!(h.find()["status"], json!("2/3"));
        h.press("shift-enter");
        assert_eq!(h.marked(), "‹alpha› beta alpha gamma alpha");
        // Find Next from the editor while the bar is open.
        h.focus_editor();
        h.press(h.keys.next);
        h.press(h.keys.next);
        assert_eq!(h.marked(), "alpha beta alpha gamma ‹alpha›");
        // Past the last match: a hint first, then from the top.
        h.press(h.keys.next);
        let s = h.find();
        assert_eq!(h.marked(), "alpha beta alpha gamma ‹alpha›");
        let hint = s["hint"].as_str().unwrap_or_default().to_owned();
        assert!(
            hint.starts_with("\"alpha\" not found, press ")
                && hint.ends_with("to search from the top"),
            "{hint}"
        );
        h.press(h.keys.next);
        assert_eq!(h.marked(), "‹alpha› beta alpha gamma alpha");
        assert_eq!(h.find()["hint"], Value::Null);
        // Backwards the same way.
        h.press(h.keys.previous);
        assert!(
            h.find()["hint"]
                .as_str()
                .unwrap()
                .ends_with("from the bottom")
        );
        h.press(h.keys.previous);
        assert_eq!(h.marked(), "alpha beta alpha gamma ‹alpha›");
    });
}

#[gpui::test]
fn the_selection_or_the_last_search_fills_the_field(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("one ‹two› two");
        h.press(h.keys.find);
        let s = h.find();
        assert_eq!(s["text"], json!("two"));
        assert_eq!(s["status"], json!("1/2"));
        // The field text is selected: typing replaces it.
        h.type_text("one");
        assert_eq!(h.find()["text"], json!("one"));
        // Escape closes the bar, keeps the match selected, removes the
        // highlighting and gives the focus back to the editor.
        h.press("escape");
        let s = h.find();
        assert_eq!(s["open"], json!(false));
        assert_eq!(h.marked(), "‹one› two two");
        assert_eq!(h.highlighted(), 0);
        assert!(h.editor_focused());
        // Opening again without a selection: the last search.
        h.set_editor("one two |two");
        h.press(h.keys.find);
        assert_eq!(h.find()["text"], json!("one"));
        // Ctrl/Cmd+Enter goes to the editor and keeps the bar.
        h.press(h.keys.to_editor);
        assert!(h.editor_focused());
        assert_eq!(h.find()["open"], json!(true));
    });
}

#[gpui::test]
fn options_switch_by_keys_and_search_again(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("|Foo foo food FOO f.o");
        h.press(h.keys.find);
        h.type_text("foo");
        assert_eq!(h.find()["matches"], json!(4));
        h.press(h.keys.match_case);
        let s = h.find();
        assert_eq!(s["case_sensitive"], json!(true));
        assert_eq!(s["matches"], json!(2));
        h.press(h.keys.words);
        assert_eq!(h.find()["matches"], json!(1));
        h.press(h.keys.match_case);
        h.press(h.keys.words);
        // Regex: `f.o` matches more than the literal.
        h.press(h.keys.find);
        h.type_text("f.o");
        assert_eq!(h.find()["matches"], json!(1));
        h.press(h.keys.regex);
        assert_eq!(h.find()["matches"], json!(5));
        assert!(h.query_focused(), "the focus stays in the field");
    });
}

#[gpui::test]
fn bad_patterns_and_no_results_are_errors(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("|abc (abc)");
        h.press(h.keys.find);
        h.press(h.keys.regex);
        h.type_text("(");
        let s = h.find();
        assert_eq!(s["status"], json!("Bad pattern"));
        assert_eq!(s["error"], json!(true));
        assert_eq!(h.highlighted(), 0);
        h.type_text("abc)");
        let s = h.find();
        assert_eq!(s["status"], json!("1/2"));
        assert_eq!(s["error"], json!(false));
        h.press(h.keys.find);
        h.type_text("zzz");
        let s = h.find();
        assert_eq!(s["status"], json!("0 results"));
        assert_eq!(s["error"], json!(true));
        // A pattern of bars only finds nothing useful.
        h.press(h.keys.find);
        h.type_text("|");
        assert_eq!(h.find()["status"], json!("Vague pattern"));
    });
}

#[gpui::test]
fn replace_one_and_all_with_groups(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("|a=1\nb=2\nc=3");
        h.press(h.keys.replace);
        assert_eq!(h.find()["replace_mode"], json!(true));
        assert!(h.query_focused(), "Replace starts in the search field");
        h.press(h.keys.regex);
        h.type_text(r"(\w)=(\d)");
        assert_eq!(h.find()["matches"], json!(3));
        h.press("tab");
        assert!(h.replace_focused());
        h.type_text("$2=$1");
        // Enter in the replace field replaces the current match.
        h.press("enter");
        assert_eq!(h.marked(), "1=a\n‹b=2›\nc=3");
        assert_eq!(h.find()["status"], json!("1/2"));
        // Replace All: the rest, as one undo step.
        h.press(h.keys.replace_all);
        let text = h.invoke("editor.text", json!(null));
        assert_eq!(text, json!("1=a\n2=b\n3=c"));
        let s = h.find();
        assert_eq!(s["status"], json!("0 results"));
        assert!(
            s["hint"]
                .as_str()
                .unwrap()
                .contains("2 occurrences replaced")
        );
        h.focus_editor();
        h.press(h.keys.undo);
        assert_eq!(h.invoke("editor.text", json!(null)), json!("1=a\nb=2\nc=3"));
        h.press(h.keys.undo);
        assert_eq!(h.invoke("editor.text", json!(null)), json!("a=1\nb=2\nc=3"));
        // A malformed template is reported, nothing changes.
        h.invoke("find.search", json!({ "text": r"(\w)=", "replace": "$9" }));
        h.invoke("find.replace-current", json!(null));
        assert!(
            h.find()["hint"]
                .as_str()
                .unwrap()
                .starts_with("Malformed replacement string")
        );
        assert_eq!(h.invoke("editor.text", json!(null)), json!("a=1\nb=2\nc=3"));
        // A read-only file is left alone.
        let editor = h.editor();
        h.cx.update(|cx| editor.update(cx, |e, _| e.buffer_mut().set_read_only(true)));
        h.invoke(
            "find.search",
            json!({ "text": "=", "regex": false, "replace": "-" }),
        );
        h.invoke("find.replace-all", json!(null));
        assert_eq!(h.find()["hint"], json!("The file is read-only"));
        assert_eq!(h.invoke("editor.text", json!(null)), json!("a=1\nb=2\nc=3"));
        h.cx.update(|cx| editor.update(cx, |e, _| e.buffer_mut().set_read_only(false)));
    });
}

#[gpui::test]
fn searching_within_a_selection(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("x\n‹x x\nx›\nx");
        h.press(h.keys.replace);
        let s = h.find();
        assert_eq!(s["in_selection"], json!(true));
        assert_eq!(s["text"], json!(""));
        h.type_text("x");
        assert_eq!(h.find()["matches"], json!(3));
        // The selection stays; the current match is only outlined.
        assert_eq!(h.marked(), "x\n‹x x\nx›\nx");
        h.press("tab");
        h.type_text("yy");
        h.press(h.keys.replace_all);
        assert_eq!(
            h.invoke("editor.text", json!(null)),
            json!("x\nyy yy\nyy\nx")
        );
        // The selection grew with the replacements.
        assert_eq!(h.marked(), "x\n‹yy yy\nyy›\nx");
        // In Selection off: the whole file.
        h.press("tab");
        h.type_text("x");
        h.press(h.keys.in_selection);
        let s = h.find();
        assert_eq!(s["in_selection"], json!(false));
        assert_eq!(s["matches"], json!(2));
    });
}

#[gpui::test]
fn find_next_with_the_bar_closed_repeats_the_last_search(cx: &mut TestAppContext) {
    both(cx, |h| {
        // Nothing searched yet: Find Next opens the bar.
        h.set_editor("|ab ab ab");
        h.press(h.keys.next);
        assert_eq!(h.find()["open"], json!(true));
        h.type_text("ab");
        h.press("escape");
        assert_eq!(h.marked(), "‹ab› ab ab");
        h.press(h.keys.next);
        assert_eq!(h.marked(), "ab ‹ab› ab");
        assert_eq!(h.highlighted(), 0, "no highlighting with the bar closed");
        h.press(h.keys.next);
        h.press(h.keys.next);
        assert_eq!(h.marked(), "ab ab ‹ab›");
        assert!(h.find()["hint"].as_str().unwrap().contains("not found"));
        h.press(h.keys.next);
        assert_eq!(h.marked(), "‹ab› ab ab");
        h.press(h.keys.previous);
        h.press(h.keys.previous);
        assert_eq!(h.marked(), "ab ab ‹ab›");
    });
}

#[gpui::test]
fn history_of_searches(cx: &mut TestAppContext) {
    both(cx, |h| {
        h.set_editor("|one two");
        h.press(h.keys.find);
        h.type_text("one");
        h.press("enter");
        // Find again selects the field text: typing replaces it.
        h.press(h.keys.find);
        h.type_text("two");
        h.press("enter");
        h.press("alt-down");
        let s = h.find();
        assert_eq!(s["history"]["items"], json!(["two", "one"]));
        h.press("down enter");
        let s = h.find();
        assert_eq!(s["history"], Value::Null);
        assert_eq!(s["text"], json!("one"));
        assert!(h.query_focused());
        h.press("alt-down escape");
        assert_eq!(h.find()["history"], Value::Null);
        assert!(h.query_focused());
    });
}

/// With a Russian input source, macOS key events still carry the Latin key
/// for shortcuts (GPUI takes the key of the Command layout when the layout
/// is not ASCII), with the typed character in `key_char`: the find keys
/// match by `key`, and Cyrillic text is typed into the field.
#[gpui::test]
fn shortcuts_match_with_a_cyrillic_layout(cx: &mut TestAppContext) {
    let mut h = setup(cx, keymap::MACOS, &MAC);
    h.set_editor("|привет мир, привет");
    let key = |modifiers: Modifiers, key: &str, key_char: Option<&str>| Keystroke {
        modifiers,
        key: key.into(),
        key_char: key_char.map(Into::into),
    };
    h.cx.dispatch_keystroke(h.window.into(), key(Modifiers::command(), "f", None));
    h.draw();
    assert_eq!(h.find()["open"], json!(true));
    h.type_text("привет");
    assert_eq!(h.find()["matches"], json!(2));
    // Option+C on the Russian layout types another character; the binding
    // still matches by its key.
    h.cx.dispatch_keystroke(h.window.into(), key(Modifiers::alt(), "c", Some("≠")));
    h.draw();
    assert_eq!(h.find()["case_sensitive"], json!(true));
    h.focus_editor();
    h.cx.dispatch_keystroke(h.window.into(), key(Modifiers::command(), "g", None));
    h.draw();
    assert_eq!(h.find()["status"], json!("2/2"));
}
