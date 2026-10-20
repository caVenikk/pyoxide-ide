//! The find bar: the bindings of `keymaps/*/find.toml`.

use super::*;

area_tests!("find");

impl H<'_> {
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

    fn find_bar(&mut self) -> Entity<::editor::FindBar> {
        self.workbench(|w, _| w.active_find_bar().cloned())
            .expect("a find bar")
    }

    fn replace_focused(&mut self) -> bool {
        let bar = self.find_bar();
        let field = self.cx.update(|cx| bar.read(cx).replace_field().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn focus_replace(&mut self) {
        let bar = self.find_bar();
        let handle = self
            .cx
            .update(|cx| bar.read(cx).replace_field().focus_handle(cx));
        self.focus(handle);
    }

    /// Opens the find bar in replace mode with a plain search for `text`
    /// and `replace` in the replace field; the focus is in the search field.
    fn search_replace(&mut self, text: &str, replace: &str) {
        self.invoke(
            "find.search",
            json!({ "text": text, "case_sensitive": false, "whole_words": false, "regex": false, "replace": replace }),
        );
        assert_eq!(self.find_state()["replace_mode"], json!(true));
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
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
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
        "find.next" | "find.previous" | "find.down" | "find.up" => {
            let forward = b.command == "find.next" || b.command == "find.down";
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
        "find.toggle-in-selection" => {
            // Replace opened with lines selected: the search stays within them.
            h.set_editor("‹foo\nfoo›\nfoo");
            h.invoke("find.replace", json!(null));
            h.search("foo");
            let s = h.find_state();
            assert_eq!(s["in_selection"], json!(true), "{what}");
            assert_eq!(s["matches"], json!(2), "{what}");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["in_selection"], json!(false), "{what}");
            assert_eq!(s["matches"], json!(3), "{what}");
        }
        "find.replace-current" => {
            h.set_editor("|foo foo");
            h.search_replace("foo", "x");
            if b.context.as_deref() == Some("FindReplace > Editor") {
                h.focus_replace();
            }
            h.press(keys);
            // Replaced; the next match is current and selected.
            assert_eq!(h.editor_marked(), "x ‹foo›", "{what}");
            assert_eq!(h.find_state()["matches"], json!(1), "{what}");
        }
        "find.replace-all" => {
            h.set_editor("|foo foo foo");
            h.search_replace("foo", "x");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x x x|", "{what}");
            assert_eq!(h.find_state()["matches"], json!(0), "{what}");
            // One undo step in the editor brings all of them back.
            h.focus_editor();
            h.invoke("editor.undo", json!(null));
            let undo_state = h.state()["editor"]["can_undo"].clone();
            let text = h.invoke("editor.text", json!(null));
            assert_eq!(text, json!("foo foo foo"), "{what}");
            assert_eq!(undo_state, json!(false), "{what}: one undo step");
        }
        "find.exclude" => {
            h.set_editor("|foo foo foo");
            h.search_replace("foo", "x");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["excluded"], json!(1), "{what}");
            assert_eq!(s["current"], json!(2), "{what}");
            // Replace All leaves the excluded match alone.
            h.invoke("find.replace-all", json!(null));
            let text = h.invoke("editor.text", json!(null));
            assert_eq!(text, json!("foo x x"), "{what}");
        }
        "find.new-line" => {
            h.set_editor("|ab\ncd ab\ncd");
            h.search("ab");
            h.press("end");
            h.press(keys);
            h.cx.simulate_input(h.window.into(), "cd");
            h.draw();
            let s = h.find_state();
            assert_eq!(s["text"], json!("ab\ncd"), "{what}");
            assert_eq!(s["matches"], json!(2), "{what}");
        }
        "find.focus-editor" => {
            h.set_editor("|foo foo");
            h.search("foo");
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
            assert_eq!(h.find_state()["open"], json!(true), "{what}");
        }
        "find.switch-field" => {
            h.set_editor("|foo");
            h.search_replace("foo", "x");
            h.press(keys);
            assert!(h.replace_focused(), "{what}: to the replace field");
            h.press(keys);
            assert!(h.query_focused(), "{what}: and back");
        }
        "find.show-history" => {
            h.set_editor("|foo bar");
            h.search("foo");
            h.invoke("find.next", json!(null));
            h.search("bar");
            h.invoke("find.next", json!(null));
            h.focus(h.cx.update(|cx| {
                let bar = h
                    .window
                    .read(cx)
                    .unwrap()
                    .active_find_bar()
                    .cloned()
                    .unwrap();
                bar.read(cx).query_field().focus_handle(cx)
            }));
            h.press(keys);
            let items = h.find_state()["history"]["items"].clone();
            assert_eq!(items[0], json!("bar"), "{what}");
            assert_eq!(items[1], json!("foo"), "{what}");
        }
        "find.history-up" | "find.history-down" | "find.history-pick" | "find.history-close" => {
            h.set_editor("|foo bar");
            h.search("foo");
            h.invoke("find.next", json!(null));
            h.search("bar");
            h.invoke("find.next", json!(null));
            h.invoke("find.show-history", json!(null));
            assert_eq!(h.find_state()["history"]["selected"], json!(0));
            h.press(keys);
            let s = h.find_state();
            match b.command.as_str() {
                "find.history-down" => {
                    assert_eq!(s["history"]["selected"], json!(1), "{what}")
                }
                "find.history-up" => {
                    // From the first item up: the last one.
                    let len = s["history"]["items"].as_array().unwrap().len();
                    assert_eq!(s["history"]["selected"], json!(len - 1), "{what}");
                }
                "find.history-pick" => {
                    assert_eq!(s["history"], Value::Null, "{what}");
                    assert_eq!(s["text"], json!("bar"), "{what}");
                    assert!(h.query_focused(), "{what}");
                }
                _ => {
                    assert_eq!(s["history"], Value::Null, "{what}");
                    assert!(h.query_focused(), "{what}");
                }
            }
        }
        _ => return false,
    }
    true
}
