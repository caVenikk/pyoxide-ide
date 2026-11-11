//! Breakpoints: the bindings of `keymaps/*/breakpoints.toml`.

use super::*;

area_tests!("breakpoints");

/// The breakpoint keys in `a.py` (`a = 1`, `b = 2`, the caret on line 2).
fn check_breakpoints(h: &mut H, command: &str, keys: &str, what: &str, context: Option<&str>) {
    h.clear_breakpoints();
    h.set_editor("a = 1\nb = 2|\n");
    let a2 = ("a.py".to_owned(), 2);
    match command {
        "debug.toggle-breakpoint" => {
            h.press(keys);
            assert_eq!(h.line_breakpoints(), std::slice::from_ref(&a2), "{what}");
            h.press(keys);
            assert!(h.line_breakpoints().is_empty(), "{what}");
        }
        "debug.toggle-temporary-breakpoint" => {
            h.press(keys);
            assert_eq!(h.line_breakpoints(), std::slice::from_ref(&a2), "{what}");
            assert_eq!(h.debug()["breakpoints"][0]["temporary"], true, "{what}");
        }
        "debug.view-breakpoints" => {
            // Where the editor does not take the key for Edit Breakpoint.
            h.focus_tree();
            h.press(keys);
            assert!(h.breakpoints_dialog().is_some(), "{what}");
        }
        "debug.edit-breakpoint" => {
            h.invoke("debug.toggle-breakpoint", json!(null));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.debug()["popup"]["title"], "a.py:2", "{what}");
        }
        "breakpoint-popup.done" | "breakpoint-popup.more" => {
            h.invoke("debug.toggle-breakpoint", json!(null));
            h.invoke("debug.edit-breakpoint", json!(null));
            assert_eq!(h.debug()["popup"]["title"], "a.py:2", "{what}");
            if context == Some("BreakpointPopup") {
                // A click in the popup takes the focus from its field.
                let at = gpui::VisualTestContext::from_window(h.window.into(), h.cx)
                    .debug_bounds("bp-popup-title")
                    .expect("the popup's title is drawn")
                    .center();
                gpui::VisualTestContext::from_window(h.window.into(), h.cx)
                    .simulate_click(at, gpui::Modifiers::none());
                h.draw();
            }
            h.press(keys);
            let state = h.debug();
            assert!(state["popup"].is_null(), "{what}: the popup closes");
            if command == "breakpoint-popup.more" {
                assert_eq!(state["dialog"]["detail"]["title"], "a.py:2", "{what}");
            }
        }
        c => {
            h.invoke(
                "debug.toggle-breakpoint",
                json!({"file": "a.py", "line": 1}),
            );
            h.invoke(
                "debug.toggle-breakpoint",
                json!({"file": "a.py", "line": 2}),
            );
            h.invoke("debug.view-breakpoints", json!(null));
            let dialog = h.breakpoints_dialog().expect("the dialog is open");
            assert_eq!(h.debug()["dialog"]["detail"]["title"], "a.py:1", "{what}");
            match c {
                "breakpoints-dialog.done" => {
                    if context.is_some_and(|c| c.ends_with("Editor")) {
                        // The condition field takes the focus.
                        h.invoke("breakpoints-dialog.set", json!({"condition_enabled": true}));
                    }
                    h.press_in(dialog, keys);
                    assert!(h.breakpoints_dialog().is_none(), "{what}");
                }
                "breakpoints-dialog.next" => {
                    h.press_in(dialog, keys);
                    assert_eq!(h.debug()["dialog"]["detail"]["title"], "a.py:2", "{what}");
                }
                "breakpoints-dialog.previous" => {
                    h.invoke(
                        "breakpoints-dialog.select",
                        json!({"file": "a.py", "line": 2}),
                    );
                    h.press_in(dialog, keys);
                    assert_eq!(h.debug()["dialog"]["detail"]["title"], "a.py:1", "{what}");
                }
                "breakpoints-dialog.toggle-enabled" => {
                    h.press_in(dialog, keys);
                    assert_eq!(h.debug()["breakpoints"][0]["enabled"], false, "{what}");
                }
                "breakpoints-dialog.remove" => {
                    h.press_in(dialog, keys);
                    assert_eq!(h.line_breakpoints(), std::slice::from_ref(&a2), "{what}");
                }
                "breakpoints-dialog.go-to-source" => {
                    h.invoke(
                        "breakpoints-dialog.select",
                        json!({"file": "a.py", "line": 2}),
                    );
                    h.press_in(dialog, keys);
                    assert!(h.breakpoints_dialog().is_none(), "{what}");
                    assert_eq!(h.state()["editor"]["caret"]["line"], 1, "{what}");
                }
                _ => panic!("{what}: no check"),
            }
        }
    }
    h.clear_breakpoints();
}

/// The class chooser's keys in its name field (classes `KeyError`,
/// `ValueError`).
fn check_chooser(h: &mut H, command: &str, keys: &str, what: &str) {
    h.clear_breakpoints();
    h.invoke("debug.add-exception-breakpoint", json!(null));
    let dialog = h.breakpoints_dialog().expect("the dialog is open");
    let chooser = h
        .cx
        .update(|cx| ui::dialog_window::child_of(dialog, cx))
        .expect("the class chooser is open");
    let chooser_state = |h: &mut H| h.debug()["dialog"]["chooser"].clone();
    match command {
        "exception-class-chooser.next" => {
            h.press_in(chooser, keys);
            assert_eq!(chooser_state(h)["selected"], 1, "{what}");
        }
        "exception-class-chooser.previous" => {
            h.invoke("exception-class-chooser.next", json!(null));
            h.press_in(chooser, keys);
            assert_eq!(chooser_state(h)["selected"], 0, "{what}");
        }
        "exception-class-chooser.ok" => {
            h.press_in(chooser, keys);
            assert!(chooser_state(h).is_null(), "{what}: the chooser closes");
            let state = h.debug();
            assert_eq!(state["exception_breakpoints"][1]["class"], "KeyError", "{what}");
            h.invoke("breakpoints-dialog.remove", json!(null));
        }
        "exception-class-chooser.cancel" => {
            h.press_in(chooser, keys);
            assert!(chooser_state(h).is_null(), "{what}: the chooser closes");
            assert_eq!(
                h.debug()["exception_breakpoints"].as_array().map(Vec::len),
                Some(1),
                "{what}"
            );
        }
        _ => panic!("{what}: no check"),
    }
    if let Some(open) = h.cx.update(|cx| ui::dialog_window::child_of(dialog, cx)) {
        h.cx.update(|cx| ui::dialog_window::close(open, cx));
    }
    h.clear_breakpoints();
}

impl H<'_> {
    fn breakpoints_dialog(&mut self) -> Option<gpui::AnyWindowHandle> {
        self.cx
            .update(|cx| ui::dialog_window::find(debugger_ui::breakpoints::dialog::CONTEXT, cx))
    }

    /// Presses keys in another window (the Breakpoints dialog).
    fn press_in(&mut self, window: gpui::AnyWindowHandle, keys: &str) {
        self.dispatched.borrow_mut().clear();
        self.cx.simulate_keystrokes(window, keys);
        self.draw();
    }

    /// No breakpoints, no popup, no dialog.
    fn clear_breakpoints(&mut self) {
        let start = std::time::Instant::now();
        while self.debug()["loaded"] != json!(true) {
            assert!(start.elapsed() < std::time::Duration::from_secs(20));
            support::tick(self.cx);
        }
        let view = self.workbench(|w, _| w.debugger_view().clone());
        let window = self.window;
        self.cx
            .update_window(window.into(), |_, window, cx| {
                view.update(cx, |v, cx| {
                    v.close_popup(false, window, cx);
                    if let Some(d) = v.debugger() {
                        for b in d.breakpoints().line() {
                            d.remove_breakpoint(b.id);
                        }
                    }
                })
            })
            .unwrap();
        if self.breakpoints_dialog().is_some() {
            self.invoke("breakpoints-dialog.done", json!(null));
        }
        self.draw();
    }

    fn line_breakpoints(&mut self) -> Vec<(String, u64)> {
        self.debug()["breakpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["file"].as_str().unwrap().to_owned(),
                    b["line"].as_u64().unwrap(),
                )
            })
            .collect()
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
        // Breakpoints: the gutter's commands, the popup and the dialog.
        c if c.starts_with("debug.")
            || c.starts_with("breakpoint-popup.")
            || c.starts_with("breakpoints-dialog.") =>
        {
            check_breakpoints(h, c, keys, &what, b.context.as_deref())
        }
        c if c.starts_with("exception-class-chooser.") => {
            check_chooser(h, c, keys, &what)
        }
        _ => return false,
    }
    true
}
