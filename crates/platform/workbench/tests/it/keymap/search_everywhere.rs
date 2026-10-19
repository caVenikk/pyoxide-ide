//! Search Everywhere: the bindings of `keymaps/*/search-everywhere.toml`.

use super::*;

area_tests!("search-everywhere");

impl H<'_> {
    fn se(&mut self) -> Value {
        self.invoke("search-everywhere.state", json!(null))
    }

    /// Opens Search Everywhere on a tab with a text, searched.
    fn search_everywhere(&mut self, tab: &str, text: &str) {
        self.invoke(
            "search-everywhere.search",
            json!({ "text": text, "tab": tab }),
        );
        self.cx
            .executor()
            .advance_clock(workbench::search_everywhere::DEBOUNCE * 2);
        self.draw();
    }

    /// Shift down and up twice, quickly (a while after the last key: a tap
    /// right after typing does not count).
    fn double_shift(&mut self) {
        self.cx
            .executor()
            .advance_clock(std::time::Duration::from_millis(200));
        for _ in 0..2 {
            for modifiers in [gpui::Modifiers::shift(), gpui::Modifiers::none()] {
                gpui::VisualTestContext::from_window(self.window.into(), self.cx)
                    .simulate_modifiers_change(modifiers);
                self.cx
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(50));
            }
        }
        self.draw();
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "search-everywhere.open" => {
            assert_eq!(keys, "shift shift", "{what}");
            h.double_shift();
            let s = h.se();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["tab"], json!("all"), "{what}");
        }
        "search-everywhere.go-to-file"
        | "search-everywhere.go-to-action"
        | "search-everywhere.go-to-text" => {
            h.press(keys);
            let s = h.se();
            assert_eq!(s["open"], json!(true), "{what}");
            let tab = b.command.rsplit('-').next().unwrap();
            let tab = match tab {
                "file" => "files",
                "action" => "actions",
                other => other,
            };
            assert_eq!(s["tab"], json!(tab), "{what}");
        }
        "search-everywhere.toggle-case"
        | "search-everywhere.toggle-words"
        | "search-everywhere.toggle-regex" => {
            h.search_everywhere("text", " = ");
            let option = match b.command.as_str() {
                "search-everywhere.toggle-case" => "case_sensitive",
                "search-everywhere.toggle-words" => "whole_words",
                _ => "regex",
            };
            h.press(keys);
            assert_eq!(h.se()["options"][option], json!(true), "{what}");
            // Back off: the options are shared with Find in Files.
            h.press(keys);
            assert_eq!(h.se()["options"][option], json!(false), "{what}");
        }
        "search-everywhere.next-tab" | "search-everywhere.previous-tab" => {
            h.search_everywhere("files", "a.py");
            h.press(keys);
            let tab = if b.command.ends_with("next-tab") {
                "actions"
            } else {
                "all"
            };
            assert_eq!(h.se()["tab"], json!(tab), "{what}");
        }
        "search-everywhere.down"
        | "search-everywhere.up"
        | "search-everywhere.next-group"
        | "search-everywhere.previous-group" => {
            // The `.py` files: a.py, b.py, m.py, n.py.
            h.search_everywhere("files", ".py");
            let n = h.se()["rows"].as_array().unwrap().len();
            assert!(n >= 3, "{what}: {n} rows");
            h.press(keys);
            let expected = match b.command.as_str() {
                "search-everywhere.down" => 1,
                "search-everywhere.up" | "search-everywhere.next-group" => n - 1,
                _ => 0,
            };
            assert_eq!(h.se()["selected"], json!(expected), "{what}");
        }
        "search-everywhere.select" => {
            h.search_everywhere("files", "n.py:1:3");
            h.press(keys);
            assert_eq!(h.se()["open"], json!(false), "{what}");
            h.draw();
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert!(
                s["tabs"][active]["path"]
                    .as_str()
                    .unwrap()
                    .ends_with("n.py"),
                "{what}"
            );
            let editor = h.editor();
            let caret = h.cx.update(|cx| editor.read(cx).caret_point());
            assert_eq!((caret.line, caret.column), (0, 2), "{what}");
        }
        "search-everywhere.close" => {
            h.search_everywhere("files", "a.py");
            h.press(keys);
            assert_eq!(h.se()["open"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes back");
        }
        "search-everywhere.history-previous" | "search-everywhere.history-next" => {
            for text in ["alpha", "beta"] {
                h.search_everywhere("files", text);
                h.invoke("search-everywhere.close", json!(null));
            }
            h.search_everywhere("files", "gamma");
            // Previous: from the newest back; next: from the oldest on.
            h.press(keys);
            let first = h.se()["text"].clone();
            h.press(keys);
            let second = h.se()["text"].clone();
            if b.command.ends_with("previous") {
                assert_eq!((first, second), (json!("beta"), json!("alpha")), "{what}");
            } else {
                assert_ne!(first, second, "{what}");
            }
        }
        "search-everywhere.choose-scope" => {
            h.search_everywhere("files", "a.py");
            h.press(keys);
            assert_eq!(h.se()["dropdown"], json!("scope"), "{what}");
            h.invoke("search-everywhere.close", json!(null));
        }
        "search-everywhere.toggle-everywhere" => {
            h.search_everywhere("all", "a.py");
            h.press(keys);
            assert_eq!(h.se()["everywhere"], json!(true), "{what}");
        }
        _ => return false,
    }
    true
}
