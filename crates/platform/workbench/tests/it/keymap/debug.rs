//! Debug sessions, steps, frames and variables: the bindings of `keymaps/*/debug.toml`.

use super::*;

area_tests!("debug");

/// The debug session commands with keys (the breakpoint ones are checked
/// by [`check_breakpoints`]).
pub(super) const SESSION_COMMANDS: [&str; 13] = [
    "debug.debug",
    "debug.current-file",
    "debug.resume",
    "debug.step-over",
    "debug.step-into",
    "debug.step-into-my-code",
    "debug.step-out",
    "debug.run-to-cursor",
    "debug.force-run-to-cursor",
    "debug.show-execution-point",
    "debug.rerun",
    "debug.stop",
    "debug.close-tab",
];

/// The session keys with a scripted adapter, in `a.py` (`a = 1`, an empty
/// second line): a session suspended on line 1; a key resumes, steps or
/// runs to line 2, stops or closes the session.
pub(super) fn check_debug_session(h: &mut H, command: &str, keys: &str, what: &str, console: bool) {
    let a = h.root.join("a.py");
    let request = |c: &str| -> Option<&'static str> {
        Some(match c {
            "debug.step-over" => "next",
            "debug.step-into" | "debug.step-into-my-code" => "stepIn",
            "debug.step-out" => "stepOut",
            "debug.resume" | "debug.run-to-cursor" | "debug.force-run-to-cursor" => "continue",
            _ => return None,
        })
    };
    let tabs = |s: &Value| s["tool_window"]["tabs"].as_array().map_or(0, Vec::len);
    match command {
        "debug.debug" | "debug.current-file" => {
            let mut script = support::debug_start(&h.root, Some(support::quick_program()));
            script.extend(support::debug_end(0));
            support::push_debug_script(script);
            h.press(keys);
            let s = h.wait_debug("the session to end", |s| {
                s["tool_window"]["tabs"][0]["exit_code"] == json!(0)
                    && s["tool_window"]["tabs"][0]["state"] == json!("ended")
            });
            assert_eq!(s["tool_window"]["tabs"][0]["title"], json!("a"), "{what}");
            assert_eq!(s["visible"], json!(true), "{what}");
            // Debug Context Configuration selects the file's configuration.
            let selected = h.run()["configurations"]["selected"].clone();
            if command == "debug.current-file" {
                assert_eq!(selected["name"], json!("a"), "{what}");
            } else {
                assert!(selected.is_null(), "{what}: {selected}");
            }
        }
        c if request(c).is_some() => {
            let mut after = support::debug_answer(request(c).unwrap());
            after.extend(support::debug_stop(&a, 2, "step"));
            h.start_debug(1, Some(after));
            if c.contains("run-to-cursor") {
                h.set_editor(
                    "a = 1
|",
                );
            }
            h.press(keys);
            h.wait_debug("the next stop", |s| {
                s["execution_point"]["line"] == json!(2)
                    && s["tool_window"]["tabs"][0]["state"] == json!("suspended")
            });
        }
        "debug.show-execution-point" => {
            h.start_debug(1, None);
            h.invoke("tabs.select", json!({"index": 1}));
            assert!(
                h.state()["editor"]["path"]
                    .as_str()
                    .unwrap()
                    .ends_with("b.py")
            );
            h.focus_editor();
            h.press(keys);
            let path = h.state()["editor"]["path"].as_str().unwrap().to_owned();
            assert!(path.ends_with("a.py"), "{what}: {path}");
        }
        "debug.rerun" => {
            h.start_debug(1, None);
            let first = h.debug()["tool_window"]["tabs"][0]["session"].clone();
            let mut script = support::debug_start(&h.root, None);
            script.extend(support::debug_stop(&a, 1, "breakpoint"));
            support::push_debug_script(script);
            h.focus_debug(console);
            h.press(keys);
            let s = h.wait_debug("the session again", |s| {
                s["tool_window"]["tabs"][0]["session"] != first
                    && s["tool_window"]["tabs"][0]["state"] == json!("suspended")
            });
            assert_eq!(tabs(&s), 1, "{what}: in the same tab");
        }
        "debug.stop" => {
            h.start_debug(1, None);
            h.focus_debug(console);
            h.press(keys);
            h.wait_debug("the session to end", |s| {
                s["tool_window"]["tabs"][0]["state"] == json!("ended")
            });
        }
        "debug.close-tab" => {
            h.start_debug(1, None);
            h.focus_debug(console);
            h.press(keys);
            let s = h.debug();
            assert_eq!(tabs(&s), 0, "{what}");
            assert_eq!(s["registered"], json!(false), "{what}");
        }
        _ => panic!("{what}: no check"),
    }
}

/// The keys of Threads & Variables (#166).
const VIEW_COMMANDS: [&str; 6] = [
    "debug.previous-frame",
    "debug.next-frame",
    "debug.variables.up",
    "debug.variables.down",
    "debug.variables.left",
    "debug.variables.right",
];

/// The keys of the frames list and the variables view with a scripted
/// adapter: a stop in `a.py` with two frames whose first frame has a
/// Locals scope (`a = 1`, `b = [1]`).
fn check_debug_views(h: &mut H, command: &str, keys: &str, what: &str, context: Option<&str>) {
    let a = h.root.join("a.py");
    let frame = |id: i64, name: &str, line: u32| json!({"id": id, "name": name, "line": line, "column": 1, "source": {"path": a}});
    let script = vec![
        json!({"ignore": ["setBreakpoints", "setExceptionBreakpoints", "source"]}),
        json!({"expect": "initialize"}),
        json!({"respond": {"supportsConfigurationDoneRequest": true}}),
        json!({"expect": "launch"}),
        json!({"event": "initialized"}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "launch"}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1}}),
        json!({"expect": "threads"}),
        json!({"respond": {"threads": [{"id": 1, "name": "MainThread"}]}}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": {"stackFrames": [frame(101, "f", 2), frame(102, "<module>", 1)],
            "totalFrames": 2}}),
        json!({"expect": "scopes", "arguments": {"frameId": 101}}),
        json!({"respond": {"scopes": [{"name": "Locals", "variablesReference": 5,
            "expensive": false}]}}),
        json!({"expect": "variables", "arguments": {"variablesReference": 5}}),
        json!({"respond": {"variables": [
            {"name": "a", "value": "1", "type": "int", "variablesReference": 0},
            {"name": "b", "value": "[1]", "type": "list", "variablesReference": 6}]}}),
    ];
    support::push_debug_script(script);
    h.invoke("debug.debug", json!(null));
    let variables = |s: &Value| s["tool_window"]["tabs"][0]["variables"].clone();
    h.wait_debug("the stop and its values", |s| {
        variables(s)["rows"]
            .as_array()
            .is_some_and(|r| r.iter().any(|r| r["name"] == json!("b")))
    });
    let view = h.workbench(|w, _| w.debugger_view().clone());
    let tool_window = h.cx.update(|cx| view.read(cx).tool_window().clone());
    let selected_frame =
        |h: &mut H| h.debug()["tool_window"]["tabs"][0]["frames_view"]["selected"].clone();
    let selected = |h: &mut H| variables(&h.debug())["selected"].clone();
    match command {
        "debug.previous-frame" | "debug.next-frame" => {
            if context == Some("DebugFrames") {
                let focus =
                    h.cx.update(|cx| tool_window.read(cx).frames_focus().clone());
                h.focus(focus);
            } else {
                h.focus_editor();
            }
            if command == "debug.previous-frame" {
                h.invoke("debug.select-frame", json!({"index": 1}));
                h.press(keys);
                assert_eq!(selected_frame(h), json!(0), "{what}");
            } else {
                h.press(keys);
                assert_eq!(selected_frame(h), json!(1), "{what}");
                // The editor shows the frame's line.
                assert_eq!(h.debug()["execution_point"]["line"], json!(1), "{what}");
            }
        }
        _ => {
            let focus = h.cx.update(|cx| {
                tool_window
                    .read(cx)
                    .selected_variables()
                    .unwrap()
                    .focus_handle(cx)
            });
            h.focus(focus);
            h.invoke("debug.variables.select", json!({"path": ["Locals", "a"]}));
            match command {
                "debug.variables.down" => {
                    h.press(keys);
                    assert_eq!(selected(h), json!(["Locals", "b"]), "{what}");
                }
                "debug.variables.up" => {
                    h.press(keys);
                    assert_eq!(selected(h), json!(["Locals"]), "{what}");
                }
                "debug.variables.left" => {
                    // To the parent, then the parent collapses.
                    h.press(keys);
                    assert_eq!(selected(h), json!(["Locals"]), "{what}");
                    h.press(keys);
                    let rows = variables(&h.debug())["rows"].clone();
                    assert_eq!(rows.as_array().map(Vec::len), Some(1), "{what}: {rows}");
                }
                "debug.variables.right" => {
                    h.invoke("debug.variables.select", json!({"path": ["Locals", "b"]}));
                    h.press(keys);
                    let rows = variables(&h.debug())["rows"].clone();
                    let b = rows
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["name"] == json!("b"))
                        .unwrap();
                    assert_eq!(b["expanded"], json!(true), "{what}: {rows}");
                }
                _ => panic!("{what}: no check"),
            }
        }
    }
}

impl H<'_> {
    /// Waits (at most about 20 s) for a debug state that `f` accepts.
    pub(super) fn wait_debug(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            support::tick(self.cx);
            last = self.debug();
            if f(&last) {
                return last;
            }
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    /// Debugs a.py with a scripted adapter that stops at `line` (then
    /// `after`, if any) and waits for the stop; the editor gets the focus
    /// back.
    pub(super) fn start_debug(&mut self, line: u32, after: Option<Vec<Value>>) {
        let file = self.root.join("a.py");
        let mut script = support::debug_start(&self.root, None);
        script.extend(support::debug_stop(&file, line, "breakpoint"));
        script.extend(after.unwrap_or_default());
        support::push_debug_script(script);
        self.invoke("debug.debug", json!(null));
        self.wait_debug("the stop", |s| {
            s["tool_window"]["tabs"][0]["state"] == json!("suspended")
                && s["execution_point"]["line"] == json!(line)
        });
        self.focus_editor();
    }

    /// The Debug tool window, or its Console, gets the focus.
    pub(super) fn focus_debug(&mut self, console: bool) {
        if console {
            self.invoke("debug.select-content", json!({"content": "console"}));
        }
        let view = self.workbench(|w, _| w.debugger_view().clone());
        let handle = self.cx.update(|cx| {
            let tool_window = view.read(cx).tool_window().clone();
            if console {
                tool_window.read(cx).content_focus(cx)
            } else {
                tool_window.focus_handle(cx)
            }
        });
        self.focus(handle);
    }

    fn debug_focused(&mut self) -> bool {
        let view = self.workbench(|w, _| w.debugger_view().clone());
        let tool_window = self.cx.update(|cx| view.read(cx).tool_window().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                tool_window.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
    }
}

/// Escape in the Debug tool window or in its console (`console`).
pub(super) fn check_escape(h: &mut H, keys: &str, what: &str, console: bool) {
    h.start_debug(1, None);
    h.focus_debug(console);
    h.press(keys);
    assert!(h.editor_focused(), "{what}");
    assert_eq!(h.debug()["visible"], json!(true), "{what}");
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    // The Debug tool window and its console.
    let debug_context = b.context.as_deref().is_some_and(|c| c.starts_with("Debug"));
    let debug_console = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("DebugConsole"));
    match b.command.as_str() {
        "workbench.hide-tool-window" if debug_context => {
            h.start_debug(1, None);
            h.focus_debug(debug_console);
            h.press(keys);
            assert_eq!(h.debug()["visible"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.focus-editor" if debug_context => check_escape(h, keys, &what, debug_console),
        "workbench.toggle-debug" => {
            h.start_debug(1, None);
            h.invoke("workbench.hide-tool-window", json!(null));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.debug()["visible"], json!(true), "{what}");
            assert!(h.debug_focused(), "{what}: first press focuses it");
            h.press(keys);
            assert_eq!(h.debug()["visible"], json!(false), "{what}");
        }
        c if SESSION_COMMANDS.contains(&c) => check_debug_session(h, c, keys, &what, debug_console),
        c if VIEW_COMMANDS.contains(&c) => {
            check_debug_views(h, c, keys, &what, b.context.as_deref())
        }
        _ => return false,
    }
    true
}
