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

/// The keys of the watches, the values and the evaluate field (#166).
const WATCH_COMMANDS: [&str; 11] = [
    "debug.new-watch",
    "debug.remove-watch",
    "debug.variables.set-value",
    "debug.variables.copy-value",
    "debug.evaluate-expression",
    "debug.variables.jump-to-source",
    "debug.variables.jump-to-type-source",
    "debug.variables.evaluate",
    "debug.variables.watch-expression",
    "debug.variables.commit-edit",
    "debug.variables.cancel-edit",
];

/// A stop in `a.py` with two frames whose first frame has a Locals scope
/// (`a = 1`, `b = [1]`), for the views of Threads & Variables; `after`
/// follows the values.
fn views_script(h: &H, after: Vec<Value>) -> Vec<Value> {
    let a = h.root.join("a.py");
    let frame = |id: i64, name: &str, line: u32| json!({"id": id, "name": name, "line": line, "column": 1, "source": {"path": a}});
    let mut script = vec![
        json!({"ignore": ["setBreakpoints", "setExceptionBreakpoints", "source"]}),
        json!({"expect": "initialize"}),
        json!({"respond": {"supportsConfigurationDoneRequest": true,
            "supportsSetVariable": true}}),
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
    script.extend(after);
    script
}

/// Debugs a.py with [`views_script`] and waits for the values.
fn start_views(h: &mut H, after: Vec<Value>) {
    let script = views_script(h, after);
    support::push_debug_script(script);
    h.invoke("debug.debug", json!(null));
    h.wait_debug("the stop and its values", |s| {
        s["tool_window"]["tabs"][0]["variables"]["rows"]
            .as_array()
            .is_some_and(|r| r.iter().any(|r| r["name"] == json!("b")))
    });
}

/// The keys of the frames list and the variables view with a scripted
/// adapter ([`views_script`]).
fn check_debug_views(h: &mut H, command: &str, keys: &str, what: &str, context: Option<&str>) {
    start_views(h, Vec::new());
    let variables = |s: &Value| s["tool_window"]["tabs"][0]["variables"].clone();
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

/// The keys of the watches, the values and the evaluate field with a
/// scripted adapter ([`views_script`]); the watches of an earlier check are
/// removed first. The adapter answers `a + 1` with 2 (Jump To Type Source:
/// where the type of `a` is); other evaluations fail (the script has
/// ended).
fn check_debug_watches(h: &mut H, command: &str, keys: &str, what: &str) {
    let after = if command == "debug.variables.jump-to-type-source" {
        let source = format!("{}:2", h.root.join("a.py").display());
        vec![
            json!({"expect": "evaluate", "arguments": {"expression": "type_source(a)"}}),
            json!({"respond": {"result": source, "type": "str", "variablesReference": 0}}),
        ]
    } else {
        vec![
            json!({"expect": "evaluate", "arguments": {"expression": "a + 1"}}),
            json!({"respond": {"result": "2", "type": "int", "variablesReference": 0}}),
        ]
    };
    start_views(h, after);
    let variables = |h: &mut H| h.debug()["tool_window"]["tabs"][0]["variables"].clone();
    while variables(h)["watches"]
        .as_array()
        .is_some_and(|w| !w.is_empty())
    {
        h.invoke("debug.remove-watch", json!({"index": 0}));
    }
    let view = h.workbench(|w, _| w.debugger_view().clone());
    let tool_window = h.cx.update(|cx| view.read(cx).tool_window().clone());
    let tree = h.cx.update(|cx| {
        tool_window
            .read(cx)
            .selected_variables()
            .unwrap()
            .focus_handle(cx)
    });
    let watches = |h: &mut H| variables(h)["watches"].clone();
    let editing = |h: &mut H| variables(h)["editing"].clone();
    match command {
        "debug.new-watch" => {
            h.focus(tree);
            h.press(keys);
            assert_eq!(editing(h)["target"], json!("new-watch"), "{what}");
            h.invoke("debug.variables.cancel-edit", json!(null));
        }
        "debug.remove-watch" => {
            h.invoke("debug.new-watch", json!({"expression": "a"}));
            assert_eq!(watches(h), json!(["a"]), "{what}");
            h.focus(tree);
            h.press(keys);
            assert_eq!(watches(h), json!([]), "{what}");
        }
        "debug.variables.set-value" => {
            h.focus(tree);
            h.invoke("debug.variables.select", json!({"path": ["Locals", "a"]}));
            h.press(keys);
            let edit = editing(h);
            assert_eq!(edit["target"], json!("value"), "{what}: {edit}");
            assert_eq!(edit["text"], json!("1"), "{what}: {edit}");
            h.invoke("debug.variables.cancel-edit", json!(null));
        }
        "debug.variables.copy-value" => {
            h.focus(tree);
            h.invoke("debug.variables.select", json!({"path": ["Locals", "b"]}));
            h.press(keys);
            let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert_eq!(copied.as_deref(), Some("[1]"), "{what}");
        }
        "debug.evaluate-expression" => {
            h.focus(tree);
            h.invoke("debug.variables.select", json!({"path": ["Locals", "b"]}));
            h.press(keys);
            // The Evaluate dialog opens with the value's expression.
            let dialog = h.debug()["evaluate_dialog"].clone();
            assert_eq!(dialog["expression"], json!("b"), "{what}: {dialog}");
            assert_eq!(dialog["mode"], json!("expression"), "{what}: {dialog}");
            // From the editor: the word at the caret.
            h.invoke("tabs.select", json!({"index": 0}));
            h.focus_editor();
            h.set_editor("|a = 1\n");
            h.press(keys);
            let dialog = h.debug()["evaluate_dialog"].clone();
            assert_eq!(dialog["expression"], json!("a"), "{what}: {dialog}");
            if let Some(open) =
                h.cx.update(|cx| ui::dialog_window::find(debugger_ui::evaluate::CONTEXT, cx))
            {
                h.cx.update(|cx| ui::dialog_window::close(open, cx));
                h.draw();
            }
        }
        "debug.variables.jump-to-source" | "debug.variables.jump-to-type-source" => {
            // From another file: a.py opens at the declaration (of the
            // value, of its type).
            h.invoke("tabs.select", json!({"index": 1}));
            h.focus(tree);
            h.invoke("debug.variables.select", json!({"path": ["Locals", "a"]}));
            h.press(keys);
            for _ in 0..200 {
                support::tick(h.cx);
                let path = h.state()["editor"]["path"].as_str().unwrap_or_default().to_owned();
                if path.ends_with("a.py") {
                    return;
                }
            }
            panic!("{what}: a.py was not opened");
        }
        "debug.variables.evaluate" => {
            h.invoke("debug.variables.set-expression", json!({"text": "a + 1"}));
            h.press(keys);
            let s = h.wait_debug("the result", |s| {
                s["tool_window"]["tabs"][0]["variables"]["rows"][0]["state"] == json!("value")
            });
            let row = &s["tool_window"]["tabs"][0]["variables"]["rows"][0];
            assert_eq!(row["text"], json!("result = {int} 2"), "{what}: {row}");
            assert_eq!(row["path"], json!(["result"]), "{what}: {row}");
        }
        "debug.variables.watch-expression" => {
            h.invoke("debug.variables.set-expression", json!({"text": "b"}));
            h.press(keys);
            assert_eq!(watches(h), json!(["b"]), "{what}");
        }
        "debug.variables.commit-edit" => {
            h.focus(tree);
            h.invoke("debug.new-watch", json!(null));
            h.cx.simulate_input(h.window.into(), "b");
            h.draw();
            assert_eq!(editing(h)["text"], json!("b"), "{what}");
            h.press(keys);
            assert_eq!(watches(h), json!(["b"]), "{what}");
            assert!(editing(h).is_null(), "{what}");
        }
        "debug.variables.cancel-edit" => {
            h.focus(tree);
            h.invoke("debug.new-watch", json!(null));
            h.cx.simulate_input(h.window.into(), "b");
            h.draw();
            h.press(keys);
            assert!(editing(h).is_null(), "{what}");
            assert_eq!(watches(h), json!([]), "{what}");
        }
        _ => panic!("{what}: no check"),
    }
}

/// The keys of the values in the editor (#167) with a scripted adapter
/// ([`views_script`]): Quick Evaluate Expression on `a` of `a = 1` (the
/// adapter answers 1), Escape in the popup of the value after line 1.
fn check_value_popups(h: &mut H, command: &str, keys: &str, what: &str) {
    let after = vec![
        json!({"expect": "evaluate", "arguments": {"expression": "a"}}),
        json!({"respond": {"result": "1", "type": "int", "variablesReference": 0}}),
    ];
    start_views(h, after);
    let hint = |h: &mut H| h.debug()["value_hint"].clone();
    match command {
        "debug.quick-evaluate" => {
            h.invoke("tabs.select", json!({"index": 0}));
            h.focus_editor();
            h.set_editor("|a = 1\n");
            h.press(keys);
            let s = h.wait_debug("the value", |s| s["value_hint"]["kind"] == json!("value"));
            let hint = &s["value_hint"];
            assert_eq!(hint["expression"], json!("a"), "{what}: {hint}");
            assert_eq!(hint["text"], json!("{int} 1"), "{what}: {hint}");
        }
        "debug.value-popup.close" => {
            h.invoke("tabs.select", json!({"index": 0}));
            h.wait_debug("the values after the lines", |s| !s["inline_values"].is_null());
            h.invoke("debug.inline-value.open", json!({"line": 1, "name": "a"}));
            assert_eq!(hint(h)["kind"], json!("tree"), "{what}: {}", hint(h));
            h.press(keys);
            assert!(hint(h).is_null(), "{what}: {}", hint(h));
        }
        _ => panic!("{what}: no check"),
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
        c if WATCH_COMMANDS.contains(&c) => check_debug_watches(h, c, keys, &what),
        c if EVALUATE_COMMANDS.contains(&c) => {
            check_evaluate_dialog(h, c, keys, &what, b.context.as_deref())
        }
        "debug.quick-evaluate" | "debug.value-popup.close" => {
            check_value_popups(h, b.command.as_str(), keys, &what)
        }
        _ => return false,
    }
    true
}

/// The keys of the Evaluate dialog (#293).
const EVALUATE_COMMANDS: [&str; 5] = [
    "evaluate-dialog.evaluate",
    "evaluate-dialog.add-to-watches",
    "evaluate-dialog.switch-mode",
    "evaluate-dialog.focus-result",
    "evaluate-dialog.close",
];

/// The keys of the Evaluate dialog with a scripted adapter
/// ([`views_script`]) that answers `a + 1` twice. In the context of the
/// dialog itself (not its editors) the result tree has the focus.
fn check_evaluate_dialog(h: &mut H, command: &str, keys: &str, what: &str, context: Option<&str>) {
    let answer = || {
        vec![
            json!({"expect": "evaluate", "arguments": {"expression": "a + 1"}}),
            json!({"respond": {"result": "2", "type": "int", "variablesReference": 0}}),
        ]
    };
    let mut after = answer();
    after.extend(answer());
    start_views(h, after);
    // The watches of an earlier check are removed.
    while h.debug()["tool_window"]["tabs"][0]["variables"]["watches"]
        .as_array()
        .is_some_and(|w| !w.is_empty())
    {
        h.invoke("debug.remove-watch", json!({"index": 0}));
    }
    h.invoke("debug.evaluate-expression", json!({"expression": "a + 1"}));
    let find = |h: &mut H| {
        h.cx.update(|cx| ui::dialog_window::find(debugger_ui::evaluate::CONTEXT, cx))
    };
    let dialog = find(h).expect("the Evaluate dialog is open");
    let state = |h: &mut H| h.debug()["evaluate_dialog"].clone();
    let in_dialog = |h: &mut H, keys: &str| {
        h.dispatched.borrow_mut().clear();
        h.cx.simulate_keystrokes(dialog, keys);
        h.draw();
    };
    let result = |h: &mut H| {
        h.wait_debug("the result", |s| {
            s["evaluate_dialog"]["result"]["state"] == json!("value")
        })["evaluate_dialog"]
            .clone()
    };
    let plain = context == Some("EvaluateDialog") && command != "evaluate-dialog.focus-result";
    if plain {
        // The tree has the focus: the keys of the dialog itself.
        in_dialog(h, "enter");
        result(h);
        let focus = h
            .cx
            .update(|cx| {
                let handle = dialog.downcast::<debugger_ui::evaluate::EvaluateDialog>()?;
                handle.read(cx).ok()?.result_focus(cx)
            })
            .expect("the result tree");
        h.cx.update_window(dialog, |_, window, _| window.focus(&focus))
            .unwrap();
        h.draw();
        assert_eq!(state(h)["result_focused"], json!(true), "{what}");
    }
    let evaluations = state(h)["evaluations"].as_u64().unwrap();
    match command {
        "evaluate-dialog.evaluate" => {
            in_dialog(h, keys);
            let d = result(h);
            assert_eq!(
                d["evaluations"].as_u64().unwrap(),
                evaluations + 1,
                "{what}"
            );
            assert_eq!(
                d["result"]["tree"]["rows"][0]["text"],
                json!("result = {int} 2"),
                "{what}"
            );
        }
        "evaluate-dialog.add-to-watches" => {
            in_dialog(h, keys);
            assert_eq!(
                h.debug()["tool_window"]["tabs"][0]["variables"]["watches"],
                json!(["a + 1"]),
                "{what}"
            );
        }
        "evaluate-dialog.switch-mode" => {
            in_dialog(h, keys);
            assert_eq!(state(h)["mode"], json!("fragment"), "{what}");
            in_dialog(h, keys);
            assert_eq!(state(h)["mode"], json!("expression"), "{what}");
        }
        "evaluate-dialog.focus-result" => {
            in_dialog(h, "enter");
            result(h);
            assert_eq!(state(h)["result_focused"], json!(false), "{what}");
            in_dialog(h, keys);
            assert_eq!(state(h)["result_focused"], json!(true), "{what}");
        }
        "evaluate-dialog.close" => {
            in_dialog(h, keys);
            assert!(find(h).is_none(), "{what}");
            return;
        }
        _ => panic!("{what}: no check"),
    }
    // Closed without a key, which would be the last one dispatched.
    h.cx.update(|cx| ui::dialog_window::close(dialog, cx));
    h.draw();
}
