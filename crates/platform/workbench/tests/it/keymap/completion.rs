//! Code completion: the bindings of `keymaps/*/completion.toml`.

use super::*;

area_tests!("completion");

impl H<'_> {
    /// The completion popup's state (`null` without one).
    fn completion(&mut self) -> Value {
        self.invoke("code.state", json!(null))["completion"].clone()
    }

    /// Opens the completion popup in the active editor at the marked
    /// caret.
    fn open_completion(&mut self, marked: &str) {
        self.set_editor(marked);
        self.invoke("code.complete", json!(null));
        assert_eq!(self.completion()["shown"], json!(true), "the popup shows");
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        "code.complete" => {
            h.set_editor("x = al|");
            h.press(keys);
            let c = h.completion();
            assert_eq!(c["shown"], json!(true), "{what}");
            assert_eq!(c["items"].as_array().unwrap().len(), 2, "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "completion.accept" => {
            h.open_completion("x = al|pz");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x = alpha|pz", "{what}");
            assert!(h.completion().is_null(), "{what}");
        }
        "completion.replace" => {
            h.open_completion("x = al|pz");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x = alpha|", "{what}");
        }
        "completion.cancel" => {
            h.open_completion("x = al|");
            h.press(keys);
            assert!(h.completion().is_null(), "{what}");
            assert_eq!(h.editor_marked(), "x = al|", "{what}");
        }
        "completion.select-next" | "completion.select-previous" => {
            // Two items: either way the second one, going round.
            h.open_completion("x = al|");
            h.press(keys);
            assert_eq!(h.completion()["selected"], json!(1), "{what}");
            assert_eq!(h.editor_marked(), "x = al|", "{what}");
        }
        "completion.page-down" => {
            h.open_completion("x = |");
            h.press(keys);
            assert_eq!(h.completion()["selected"], json!(11), "{what}");
        }
        "completion.page-up" => {
            h.open_completion("x = |");
            h.invoke("completion.last", json!(null));
            h.press(keys);
            assert_eq!(h.completion()["selected"], json!(31 - 11), "{what}");
        }
        _ => return false,
    }
    true
}
