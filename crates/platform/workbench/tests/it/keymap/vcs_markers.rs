//! Version control change markers: the bindings of `keymaps/*/vcs-markers.toml`.

use super::*;

use super::vcs::{close_diffs, wait_diff};

area_tests!("vcs-markers");

impl H<'_> {
    /// Puts marked text into the active editor with change markers against
    /// `base` (its committed text).
    fn set_editor_with_base(&mut self, marked: &str, base: &str) {
        self.set_editor(marked);
        let path = self
            .workbench(|w, cx| w.active_file(cx))
            .expect("a file is open");
        self.window
            .update(self.cx, |w, _, cx| {
                w.vcs_set_base_text(&path, Some(base), cx)
            })
            .unwrap();
        self.draw();
    }

    /// The marker popup's state (`null` without one).
    fn marker_popup(&mut self) -> Value {
        self.state()["vcs"]["marker_popup"].clone()
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        // Lines 1 and 3 differ from the committed text.
        "vcs.next-change" => {
            h.set_editor_with_base("|a\nB\nc\nD\ne\n", "a\nb\nc\nd\ne\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "a\n|B\nc\nD\ne\n", "{what}");
            assert_eq!(h.marker_popup()["lines"], json!([1, 2]), "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "vcs.previous-change" => {
            h.set_editor_with_base("a\nB\nc\nD\n|e\n", "a\nb\nc\nd\ne\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "a\nB\nc\n|D\ne\n", "{what}");
            assert_eq!(h.marker_popup()["lines"], json!([3, 4]), "{what}");
        }
        "vcs.rollback-lines" => {
            h.set_editor_with_base("a\nB|\nc\nD\n", "a\nb\nc\nd\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "a\n|b\nc\nD\n", "{what}");
            // One undoable edit.
            h.invoke("editor.undo", json!(null));
            assert_eq!(
                h.editor().read_with(h.cx, |e, _| e.text()),
                "a\nB\nc\nD\n",
                "{what}"
            );
        }
        "vcs.show-diff-lines" | "marker-popup.close" => {
            h.set_editor_with_base("a\nB|\nc\n", "a\nb\nc\n");
            h.invoke("vcs.show-current-change", json!(null));
            assert!(!h.marker_popup().is_null(), "{what}: the popup shows");
            let state = h.state();
            let file =
                state["tabs"][state["active_tab"].as_u64().unwrap() as usize]["path"].clone();
            h.press(keys);
            if b.command == "marker-popup.close" {
                assert!(h.marker_popup().is_null(), "{what}");
            } else {
                // The popup's key: the line is not duplicated, the file's
                // diff opens at the popup's change.
                assert!(h.marker_popup().is_null(), "{what}");
                assert_eq!(
                    h.dispatched.borrow().last().map(String::as_str),
                    Some("vcs.show-diff-lines"),
                    "{what}"
                );
                let diff = wait_diff(h, &what, |d| d["right"]["caret"] == json!([1, 0]));
                assert_eq!(diff["status"], "1 difference", "{what}");
                close_diffs(h);
                // Back to the file's tab.
                if let Some(path) = file.as_str() {
                    h.invoke("project.open-file", json!({ "path": path }));
                }
            }
            assert_eq!(h.editor_marked(), "a\n|B\nc\n", "{what}");
        }
        _ => return false,
    }
    true
}
