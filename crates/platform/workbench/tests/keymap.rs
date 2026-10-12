//! Every binding of both default keymaps, pressed as real keystrokes
//! (`simulate_keystrokes`) in a headless main window: the keystroke must
//! resolve to its command in its key context, and the command must have
//! its effect. A binding without a check here fails the test, so a new
//! binding comes with its check.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use editor::Editor;
use gpui::{AppContext as _, ClipboardItem, Entity, Focusable as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use text::{Buffer, Selection, SelectionSet};
use ui::icons::IconName;
use workbench::keymap::{self, Binding, Keymap};
use workbench::{FileType, Product, Workbench};

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

fn python() -> FileType {
    FileType {
        indent_after: &[':'],
        dedent_after: &["return", "pass"],
        line_comment: Some("# "),
        quotes: &['"', '\''],
        triple_quotes: true,
        string_prefixes: &['f', 'r', 'b'],
        ..FileType::plain("Python", &["py"], IconName::FILE_TEXT)
    }
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
    /// Commands the keystrokes resolved to, in order.
    dispatched: Rc<RefCell<Vec<String>>>,
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

    fn press(&mut self, keys: &str) {
        self.dispatched.borrow_mut().clear();
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn workbench<R>(&mut self, f: impl FnOnce(&Workbench, &gpui::App) -> R) -> R {
        self.window.read_with(self.cx, f).unwrap()
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.workbench(|w, _| w.active_editor().cloned())
            .expect("an editor is open")
    }

    fn focus(&mut self, handle: gpui::FocusHandle) {
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.focus(handle);
    }

    fn focus_tree(&mut self) {
        let panel = self.workbench(|w, _| w.panel().clone());
        let handle = self.cx.update(|cx| panel.focus_handle(cx));
        self.focus(handle);
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self.editor();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn tree_focused(&mut self) -> bool {
        let panel = self.workbench(|w, _| w.panel().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                panel.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
    }

    /// Puts marked text into the active editor and focuses it.
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

    fn editor_marked(&mut self) -> String {
        let editor = self.editor();
        self.cx.update(|cx| {
            let e = editor.read(cx);
            render_marked(&e.text(), e.selections().primary())
        })
    }

    fn caret_line(&mut self) -> (usize, usize) {
        let editor = self.editor();
        self.cx.update(|cx| {
            let e = editor.read(cx);
            let s = e.selections().primary();
            let anchor = e.buffer().offset_to_point(s.anchor).line;
            (anchor, e.caret_point().line)
        })
    }

    fn tree_rows(&mut self) -> Vec<Value> {
        self.state()["tree"]["rows"].as_array().unwrap().clone()
    }

    fn tree_row(&mut self, path: &str) -> Value {
        self.tree_rows()
            .into_iter()
            .find(|r| r["path"] == json!(path))
            .unwrap_or_else(|| panic!("no tree row {path}"))
    }

    fn select_in_tree(&mut self, path: &str) {
        self.invoke("project.tree.reveal", json!({ "path": path }));
        self.focus_tree();
    }

    fn find_state(&mut self) -> Value {
        self.state()["find"].clone()
    }

    fn query_focused(&mut self) -> bool {
        let bar = self
            .workbench(|w, _| w.active_find_bar().cloned())
            .expect("a find bar");
        let field = self.cx.update(|cx| bar.read(cx).query_field().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    /// Opens the find bar with a plain, case-insensitive search for
    /// `text`; the focus is in its field.
    fn search(&mut self, text: &str) {
        self.invoke(
            "find.search",
            json!({ "text": text, "case_sensitive": false, "whole_words": false, "regex": false }),
        );
        assert!(self.query_focused(), "the search field has the focus");
    }

    /// Three tabs (a.py, b.py, c.txt), the first active, the project tool
    /// window shown, the editor focused.
    fn reset(&mut self) {
        if self.state()["go_to_line"].is_string() {
            self.invoke("go-to-line.cancel", json!(null));
        }
        if self.state()["find"]["open"] == json!(true) {
            self.invoke("find.close", json!(null));
        }
        while self.state()["tabs"]
            .as_array()
            .is_some_and(|t| !t.is_empty())
        {
            self.invoke("tabs.close", json!({ "index": 0 }));
        }
        for file in ["a.py", "b.py", "c.txt"] {
            self.invoke("project.open-file", json!({ "path": file }));
        }
        self.invoke("tabs.select", json!({ "index": 0 }));
        if !self.state()["tool_windows"]["project"]["visible"]
            .as_bool()
            .unwrap()
        {
            self.invoke("workbench.toggle-project", json!(null));
        }
        self.focus_editor();
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str) -> (H<'a>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for (path, text) in [
        ("a.py", "a = 1\n"),
        ("b.py", "b = 2\n"),
        ("c.txt", "c\n"),
        ("src/m.py", "m = 3\n"),
        ("src/pkg/n.py", "n = 4\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        file_types: vec![python()],
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
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let sink = dispatched.clone();
    cx.update(|cx| {
        cx.observe_keystrokes(move |event, _, _| {
            if let Some(action) = &event.action {
                sink.borrow_mut().push(action.name().to_owned());
            }
        })
        .detach()
    });
    let mut h = H {
        cx,
        window,
        root: root.clone(),
        dispatched,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    h.reset();
    (h, dir)
}

/// Text and caret before and after commands that only edit or move.
fn editing_case(command: &str) -> Option<(&'static str, &'static str)> {
    Some(match command {
        "editor.move-left" => ("ab c|d", "ab |cd"),
        "editor.move-right" => ("ab |cd", "ab c|d"),
        "editor.move-up" => ("ab\ncd|", "ab|\ncd"),
        "editor.move-down" => ("ab|\ncd", "ab\ncd|"),
        "editor.move-word-left" => ("ab cd|", "ab |cd"),
        "editor.move-word-right" => ("|ab", "ab|"),
        "editor.move-line-start" => ("  ab|", "  |ab"),
        "editor.move-line-end" => ("|ab", "ab|"),
        "editor.move-document-start" => ("ab\ncd|", "|ab\ncd"),
        "editor.move-document-end" => ("|ab\ncd", "ab\ncd|"),
        "editor.select-left" => ("ab|", "a›b‹"),
        "editor.select-right" => ("|ab", "‹a›b"),
        "editor.select-up" => ("ab\ncd|", "ab›\ncd‹"),
        "editor.select-down" => ("|ab\ncd", "‹ab\n›cd"),
        "editor.select-word-left" => ("ab cd|", "ab ›cd‹"),
        "editor.select-word-right" => ("|ab", "‹ab›"),
        "editor.select-line-start" => ("  ab|", "  ›ab‹"),
        "editor.select-line-end" => ("|ab", "‹ab›"),
        "editor.select-document-start" => ("ab\ncd|", "›ab\ncd‹"),
        "editor.select-document-end" => ("|ab\ncd", "‹ab\ncd›"),
        "editor.select-all" => ("a|b\ncd", "‹ab\ncd›"),
        "editor.select-word" => ("ab c|d ef", "ab ‹cd› ef"),
        "editor.escape" => ("‹ab›", "ab|"),
        "editor.newline" => ("def f():|", "def f():\n    |"),
        "editor.start-new-line" => ("a|b", "ab\n|"),
        "editor.tab" => ("|ab", "    |ab"),
        "editor.unindent" => ("    a|b", "a|b"),
        "editor.backspace" => ("ab|", "a|"),
        "editor.delete" => ("|ab", "|b"),
        "editor.delete-word-left" => ("ab cd|", "ab |"),
        "editor.delete-word-right" => ("|ab", "|"),
        "editor.delete-line" => ("ab\nc|d\nef", "ab\ne|f"),
        "editor.duplicate" => ("a|b", "ab\na|b"),
        "editor.move-line-up" => ("ab\nc|d", "c|d\nab"),
        "editor.move-line-down" => ("a|b\ncd", "cd\na|b"),
        "editor.move-statement-up" => ("def f():\n    pass\nx| = 1", "x| = 1\ndef f():\n    pass"),
        "editor.move-statement-down" => {
            ("d|ef f():\n    pass\nx = 1", "x = 1\nd|ef f():\n    pass")
        }
        "editor.join-lines" => ("a|b\n    cd", "ab| cd"),
        "editor.toggle-comment" => ("    a|b\ncd", "    # ab\ncd|"),
        _ => return None,
    })
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    if let Some((before, after)) = editing_case(&b.command) {
        h.set_editor(before);
        h.press(keys);
        assert_eq!(h.editor_marked(), after, "{what}");
        return true;
    }
    match b.command.as_str() {
        "editor.page-up"
        | "editor.page-down"
        | "editor.select-page-up"
        | "editor.select-page-down" => {
            let text: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
            let mut marked = text.join("\n");
            let at = marked.find("line 150").unwrap();
            marked.insert(at, CARET);
            h.set_editor(&marked);
            h.press(keys);
            let (anchor, head) = h.caret_line();
            let down = b.command.ends_with("down");
            assert!(
                if down { head > 155 } else { head < 145 },
                "{what}: caret on line {head}"
            );
            let select = b.command.contains("select");
            assert_eq!(anchor == 150, select, "{what}: anchor on line {anchor}");
        }
        "editor.shrink-selection" => {
            h.set_editor("ab c|d");
            h.invoke("editor.select-word", json!(null));
            assert_eq!(h.editor_marked(), "ab ‹cd›");
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "ab c|d", "{what}");
        }
        "editor.undo" => {
            h.set_editor("ab|");
            h.invoke("editor.type", json!({ "text": "x" }));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "ab|", "{what}");
        }
        "editor.redo" => {
            h.set_editor("ab|");
            h.invoke("editor.type", json!({ "text": "x" }));
            h.invoke("editor.undo", json!(null));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "abx|", "{what}");
        }
        "editor.copy" | "editor.cut" => {
            h.set_editor("a|b\ncd");
            h.press(keys);
            let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert_eq!(copied.as_deref(), Some("ab\n"), "{what}");
            let after = if b.command == "editor.cut" {
                "c|d"
            } else {
                "a|b\ncd"
            };
            assert_eq!(h.editor_marked(), after, "{what}");
        }
        "editor.paste" => {
            h.cx.write_to_clipboard(ClipboardItem::new_string("xy".into()));
            h.set_editor("a|b");
            h.press(keys);
            assert_eq!(h.editor_marked(), "axy|b", "{what}");
        }
        "workbench.go-to-line" => {
            h.set_editor("a\nb|\nc");
            h.press(keys);
            assert_eq!(h.state()["go_to_line"], json!("2:2"), "{what}");
        }
        "go-to-line.confirm" => {
            h.set_editor("a\nb\nc|");
            h.invoke("workbench.go-to-line", json!(null));
            // The field's text is selected; typing replaces it.
            h.cx.simulate_input(h.window.into(), "2:2");
            h.press(keys);
            assert_eq!(h.state()["go_to_line"], Value::Null, "{what}");
            assert_eq!(h.editor_marked(), "a\nb|\nc", "{what}");
            assert!(h.editor_focused(), "{what}: the editor has the focus");
        }
        "go-to-line.cancel" => {
            h.set_editor("a\nb\nc|");
            h.invoke("workbench.go-to-line", json!(null));
            h.press(keys);
            assert_eq!(h.state()["go_to_line"], Value::Null, "{what}");
            assert_eq!(h.editor_marked(), "a\nb\nc|", "{what}");
            assert!(h.editor_focused(), "{what}: the editor has the focus");
        }
        "find.open" => {
            // The selected text becomes the query, selected in the field.
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
            assert_eq!(s["matches"], json!(2), "{what}");
            assert_eq!(s["current"], json!(1), "{what}");
            assert!(h.query_focused(), "{what}: the field has the focus");
        }
        "find.replace" => {
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["replace_mode"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
            assert!(h.query_focused(), "{what}: the search field has the focus");
        }
        "find.next" | "find.previous" => {
            let forward = b.command == "find.next";
            h.set_editor(if forward {
                "|foo foo foo"
            } else {
                "foo foo foo|"
            });
            h.search("foo");
            let start = if forward {
                "‹foo› foo foo"
            } else {
                "foo foo ‹foo›"
            };
            assert_eq!(h.editor_marked(), start, "{what}");
            if b.context.as_deref() != Some("FindBar > Editor") {
                // From the editor, with the bar open.
                h.focus_editor();
            }
            h.press(keys);
            assert_eq!(h.editor_marked(), "foo ‹foo› foo", "{what}");
            assert_eq!(h.find_state()["current"], json!(2), "{what}");
        }
        "find.close" => {
            h.set_editor("|foo foo");
            h.search("foo");
            h.press(keys);
            assert_eq!(h.find_state()["open"], json!(false), "{what}");
            // The current match stays selected; the editor has the focus.
            assert_eq!(h.editor_marked(), "‹foo› foo", "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "find.toggle-case" | "find.toggle-words" | "find.toggle-regex" => {
            h.set_editor("|Foo foo food f.o");
            h.search("f.o");
            let option = match b.command.as_str() {
                "find.toggle-case" => "case_sensitive",
                "find.toggle-words" => "whole_words",
                _ => "regex",
            };
            assert_eq!(h.find_state()["matches"], json!(1), "{what}");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s[option], json!(true), "{what}");
            let expected = match option {
                // Case: still only the literal `f.o`.
                "case_sensitive" => 1,
                // Words: `f.o` stands alone.
                "whole_words" => 1,
                // Regex: `f.o` matches Foo, foo, foo(d) and f.o.
                _ => 4,
            };
            assert_eq!(s["matches"], json!(expected), "{what}");
            assert!(h.query_focused(), "{what}: the focus stays in the field");
        }
        "app.quit" => {
            h.press(keys);
        }
        "workbench.toggle-project" => {
            h.press(keys);
            assert!(h.tree_focused(), "{what}: first press focuses the tree");
            h.press(keys);
            let s = h.state();
            assert_eq!(
                s["tool_windows"]["project"]["visible"],
                json!(false),
                "{what}: second press hides it"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.hide-tool-window" => {
            h.focus_tree();
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["project"]["visible"], json!(false));
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.focus-editor" => {
            h.focus_tree();
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.save-all" => {
            h.set_editor("|");
            h.invoke("editor.type", json!({ "text": "x = 9" }));
            h.focus_editor();
            h.press(keys);
            let disk = std::fs::read_to_string(h.root.join("a.py")).unwrap();
            assert_eq!(disk, "x = 9", "{what}");
            std::fs::write(h.root.join("a.py"), "a = 1\n").unwrap();
        }
        "tabs.close" => {
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tabs"].as_array().unwrap().len(), 2, "{what}");
        }
        "tabs.next" => {
            h.press(keys);
            assert_eq!(h.state()["active_tab"], json!(1), "{what}");
        }
        "tabs.previous" => {
            h.press(keys);
            assert_eq!(h.state()["active_tab"], json!(2), "{what}");
        }
        "project.tree.up" | "project.tree.down" => {
            h.select_in_tree("src/m.py");
            let rows = h.tree_rows();
            let i = rows
                .iter()
                .position(|r| r["path"] == json!("src/m.py"))
                .unwrap();
            let expected = if b.command.ends_with("up") {
                i - 1
            } else {
                i + 1
            };
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["selected"],
                rows[expected]["path"],
                "{what}"
            );
        }
        "project.tree.expand" => {
            h.select_in_tree("src/pkg");
            h.invoke("project.tree.collapse-all", json!(null));
            h.select_in_tree("src/pkg");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(false));
            h.press(keys);
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true), "{what}");
        }
        "project.tree.collapse" => {
            h.select_in_tree("src/pkg/n.py");
            h.select_in_tree("src/pkg");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true));
            h.press(keys);
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(false), "{what}");
        }
        "project.tree.open" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], json!("src/m.py"), "{what}");
            h.invoke("tabs.close", json!(null));
        }
        "project.tree.collapse-all" => {
            h.select_in_tree("src/pkg/n.py");
            h.press(keys);
            assert_eq!(h.tree_row("src")["expanded"], json!(false), "{what}");
        }
        "project.tree.expand-all" => {
            h.select_in_tree("src/pkg/n.py");
            h.invoke("project.tree.collapse-all", json!(null));
            h.select_in_tree("src");
            h.press(keys);
            assert_eq!(h.tree_row("src")["expanded"], json!(true), "{what}");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true), "{what}");
        }
        _ => return false,
    }
    true
}

fn check_keymap(cx: &mut TestAppContext, text: &str) {
    let keymap = Keymap::parse(text).unwrap();
    let (mut h, _dir) = setup(cx, text);
    let mut missing = Vec::new();
    for b in &keymap.bindings {
        eprintln!("checking `{}` -> {}", b.keys, b.command);
        if !check(&mut h, b) {
            missing.push(format!("`{}` -> {}", b.keys, b.command));
            continue;
        }
        let dispatched = h.dispatched.borrow().clone();
        assert!(
            dispatched.contains(&b.command),
            "`{}` should dispatch {} in context {:?}, dispatched {dispatched:?}",
            b.keys,
            b.command,
            b.context
        );
    }
    assert!(missing.is_empty(), "bindings without a check: {missing:?}");
}

#[gpui::test]
fn every_macos_binding_works(cx: &mut TestAppContext) {
    check_keymap(cx, keymap::MACOS);
}

#[gpui::test]
fn every_windows_and_linux_binding_works(cx: &mut TestAppContext) {
    check_keymap(cx, keymap::DEFAULT);
}
