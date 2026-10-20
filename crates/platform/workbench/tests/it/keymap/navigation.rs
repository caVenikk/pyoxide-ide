//! Back and forward, Go to Line and Recent Files: the bindings of `keymaps/*/navigation.toml`.

use super::*;

area_tests!("navigation");

impl H<'_> {
    /// Presses and releases a mouse button over the editor.
    fn click_button(&mut self, button: gpui::MouseButton) {
        let at = gpui::point(gpui::px(700.0), gpui::px(400.0));
        let mut cx = gpui::VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_mouse_down(at, button, gpui::Modifiers::none());
        cx.simulate_mouse_up(at, button, gpui::Modifiers::none());
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
        "recent-files.open" => {
            h.press(keys);
            let s = h.invoke("recent-files.state", json!(null));
            assert_eq!(s["open"], json!(true), "{what}");
        }
        "recent-files.down" | "recent-files.up" | "recent-files.left" | "recent-files.right" => {
            h.invoke("recent-files.open", json!(null));
            let rf = |h: &mut H| h.invoke("recent-files.state", json!(null));
            assert_eq!(rf(h)["selected"], json!(1), "{what}");
            match b.command.as_str() {
                "recent-files.down" => {
                    h.press(keys);
                    assert_eq!(rf(h)["selected"], json!(2), "{what}");
                }
                "recent-files.up" => {
                    h.press(keys);
                    assert_eq!(rf(h)["selected"], json!(0), "{what}");
                }
                "recent-files.left" => {
                    h.press(keys);
                    assert_eq!(rf(h)["list"], json!("tool_windows"), "{what}");
                }
                _ => {
                    h.invoke("recent-files.left", json!(null));
                    h.press(keys);
                    assert_eq!(rf(h)["list"], json!("files"), "{what}");
                }
            }
        }
        "recent-files.select" => {
            h.invoke("recent-files.open", json!(null));
            let file = h.invoke("recent-files.state", json!(null))["files"][1].clone();
            h.press(keys);
            let s = h.state();
            assert_eq!(s["recent_files"], json!(false), "{what}");
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], file, "{what}");
        }
        "recent-files.close" => {
            h.invoke("recent-files.open", json!(null));
            h.press(keys);
            assert_eq!(h.state()["recent_files"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes back");
        }
        "recent-files.delete" => {
            h.invoke("recent-files.open", json!(null));
            let before = h.invoke("recent-files.state", json!(null))["files"]
                .as_array()
                .unwrap()
                .len();
            h.press(keys);
            let after = h.invoke("recent-files.state", json!(null))["files"]
                .as_array()
                .unwrap()
                .len();
            assert_eq!(after, before - 1, "{what}");
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
        // The reset opens a.py, b.py and c.txt and selects a.py again:
        // Back returns to c.txt, Forward to a.py.
        "workbench.back" | "workbench.forward" => {
            let active =
                |s: &Value| s["tabs"][s["active_tab"].as_u64().unwrap() as usize]["path"].clone();
            if b.command == "workbench.forward" {
                h.invoke("workbench.back", json!(null));
                h.focus_editor();
            }
            match keymap::parse_mouse_button(keys) {
                Some(button) => h.click_button(button),
                None => h.press(keys),
            }
            let expected = if b.command == "workbench.back" {
                "c.txt"
            } else {
                "a.py"
            };
            assert_eq!(active(&h.state()), json!(expected), "{what}");
        }
        _ => return false,
    }
    true
}
