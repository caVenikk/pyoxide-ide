//! Breakpoints in the editor and the Breakpoints dialog (#164), through
//! real mouse events on the line numbers and real keystrokes of the macOS
//! keymap in a headless main window: the hover icon and its tooltip,
//! clicks with and without modifiers, Toggle Line Breakpoint, the
//! properties popup, the icons by state, breakpoints following edits and
//! kept with the project, the Breakpoints dialog and its keys, and the
//! check badge of breakpoints a debug session verified.

use crate::support;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use editor::Editor;
use gpui::{
    AnyWindowHandle, AppContext as _, CursorStyle, Entity, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, WindowHandle,
};
use serde_json::{Value, json};
use ui::icons::IconName;
use workbench::keymap::{self, Keymap};
use workbench::{DebugHook, FileType, Product, Workbench};

const MAIN: &str = "import os\n\n# note\nx = 1\ny = 2\nz = 3\nprint(x)\n";

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
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

    fn debug(&mut self) -> Value {
        self.invoke("debug.state", json!(null))
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn press_in(&mut self, window: AnyWindowHandle, keys: &str) {
        self.cx.simulate_keystrokes(window, keys);
        self.draw();
    }

    fn vcx(&mut self) -> VisualTestContext {
        VisualTestContext::from_window(self.window.into(), self.cx)
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.cx
            .read(|cx| self.window.read(cx).unwrap().active_editor().cloned())
            .expect("an editor is open")
    }

    /// The middle of the line numbers' cell of a line (zero-based).
    fn cell(&mut self, line: usize) -> gpui::Point<gpui::Pixels> {
        let editor = self.editor();
        self.cx
            .read(|cx| editor.read(cx).line_number_bounds(line))
            .unwrap_or_else(|| panic!("line {line} is drawn"))
            .center()
    }

    fn hover_line(&mut self, line: usize) {
        let at = self.cell(line);
        self.vcx().simulate_mouse_move(at, None, Modifiers::none());
        self.draw();
    }

    fn click_line(&mut self, line: usize, modifiers: Modifiers) {
        let at = self.cell(line);
        self.vcx().simulate_mouse_move(at, None, modifiers);
        self.vcx().simulate_click(at, modifiers);
        self.draw();
    }

    fn press_line(&mut self, line: usize, button: MouseButton) {
        let at = self.cell(line);
        let none = Modifiers::none();
        self.vcx().simulate_mouse_down(at, button, none);
        self.vcx().simulate_mouse_up(at, button, none);
        self.draw();
    }

    fn click(&mut self, selector: &'static str) {
        let at = self
            .vcx()
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
        self.vcx().simulate_click(at, Modifiers::none());
        self.draw();
    }

    fn pointer(&mut self) -> CursorStyle {
        self.vcx().update(|window, _| ui::pointer::resolve(window))
    }

    /// The line breakpoints as (line, icon, enabled, suspend).
    fn breakpoints(&mut self) -> Vec<(u64, String, bool, String)> {
        self.debug()["breakpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                (
                    b["line"].as_u64().unwrap(),
                    b["icon"].as_str().unwrap().to_owned(),
                    b["enabled"].as_bool().unwrap(),
                    b["suspend"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// The gutter icons of the active editor as (zero-based line, icon,
    /// badge, background).
    fn gutter(&mut self) -> Vec<(u64, String, Option<String>, bool)> {
        let state = self.invoke("editor.state", json!(null));
        let mut out: Vec<_> = state["decorations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["group"] == "breakpoints")
            .map(|d| {
                (
                    d["lines"][0].as_u64().unwrap(),
                    d["gutter_icon"].as_str().unwrap_or_default().to_owned(),
                    d["badge"].as_str().map(str::to_owned),
                    !d["background"].is_null(),
                )
            })
            .collect();
        out.sort();
        out
    }

    fn set_caret(&mut self, line: usize) {
        self.invoke(
            "editor.set-selection",
            json!({"anchor": {"line": line, "column": 0}, "head": {"line": line, "column": 0}}),
        );
    }

    fn wait_loaded(&mut self) {
        let start = Instant::now();
        while self.debug()["loaded"] != json!(true) {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "the debugger opens"
            );
            support::tick(self.cx);
        }
    }

    fn open_project(&mut self) {
        let root = self.root.display().to_string();
        self.invoke("project.open-folder", json!({ "path": root }));
        self.wait_loaded();
    }

    fn open(&mut self, file: &str) {
        self.invoke("project.open-file", json!({ "path": file }));
        self.draw();
    }

    fn dialog(&mut self) -> Option<AnyWindowHandle> {
        self.cx
            .update(|cx| ui::dialog_window::find(debugger_ui::breakpoints::dialog::CONTEXT, cx))
    }
}

fn icon(name: IconName) -> String {
    name.0.to_owned()
}

fn python() -> FileType {
    FileType {
        line_comment: Some("# "),
        ..FileType::plain("Python", &["py"], IconName::FILE_TEXT)
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, hook: DebugHook) -> (H<'a>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    for (path, text) in [
        ("main.py", MAIN),
        ("other.py", "a = 1\nb = 2\n"),
        ("notes.txt", "note\n"),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        file_types: vec![python()],
        debug: Some(hook),
        ..Product::default()
    };
    let keymap = Keymap::parse(keymap::MACOS).unwrap();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &keymap);
        assert!(errors.is_empty(), "keymap errors: {errors:?}");
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.update_window(window.into(), |_, window, _| window.activate_window())
        .unwrap();
    let mut h = H { cx, window, root };
    h.draw();
    let view =
        h.cx.read(|cx| h.window.read(cx).unwrap().debugger_view().clone());
    h.cx.update(|cx| {
        view.update(cx, |v, _| {
            v.set_tooltip_delay(Duration::ZERO);
            v.set_activate(false);
        })
    });
    h.open_project();
    h.open("main.py");
    (h, dir)
}

/// Scenario `debugger-breakpoints.md`, steps 1 to 6: the hover icon on
/// code lines only, clicks, Cmd+F8, Alt+click, Shift+click, the tooltip,
/// a middle click and removing by a click.
#[gpui::test]
fn gutter_clicks_keys_and_icons(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, support::debug_hook());
    let bp = icon(debugger_ui::icons::BREAKPOINT);

    // Step 1: a code line shows the icon with the hand and "Add
    // Breakpoint"; a blank line and a comment show nothing.
    h.hover_line(0);
    assert_eq!(
        h.debug()["tooltip"],
        json!({"line": 0, "lines": ["Add Breakpoint"]})
    );
    assert_eq!(h.pointer(), CursorStyle::PointingHand);
    let editor = h.editor();
    assert_eq!(
        h.cx.read(|cx| editor.read(cx).gutter_hovered_line()),
        Some(0)
    );
    h.hover_line(1);
    assert_eq!(h.debug()["tooltip"], Value::Null);
    assert_ne!(h.pointer(), CursorStyle::PointingHand);
    h.hover_line(2);
    assert_eq!(h.debug()["tooltip"], Value::Null);

    // Step 2: a click and Cmd+F8 at the caret add breakpoints that suspend
    // the thread (the type's default), with the line background.
    h.click_line(0, Modifiers::none());
    h.set_caret(3);
    h.press("cmd-f8");
    assert_eq!(
        h.breakpoints(),
        [
            (1, bp.clone(), true, "thread".into()),
            (4, bp.clone(), true, "thread".into())
        ]
    );
    assert_eq!(
        h.gutter(),
        [(0, bp.clone(), None, true), (3, bp.clone(), None, true)]
    );
    // Not on a blank line or a comment.
    h.set_caret(1);
    h.press("cmd-f8");
    h.click_line(2, Modifiers::none());
    assert_eq!(h.breakpoints().len(), 2);

    // Step 3: Alt+click disables, again enables.
    let alt = Modifiers {
        alt: true,
        ..Modifiers::none()
    };
    h.click_line(0, alt);
    assert_eq!(
        h.gutter()[0],
        (0, icon(debugger_ui::icons::DISABLED), None, false)
    );
    h.click_line(0, alt);
    assert_eq!(h.gutter()[0], (0, bp.clone(), None, true));

    // Step 4: Shift+click adds a breakpoint that logs and does not
    // suspend, and opens its popup.
    let shift = Modifiers {
        shift: true,
        ..Modifiers::none()
    };
    h.click_line(4, shift);
    let state = h.debug();
    let logging = &state["breakpoints"][2];
    assert_eq!(logging["line"], 5);
    assert_eq!(logging["suspend"], "none");
    assert_eq!(logging["log_message"], true);
    assert_eq!(logging["icon"], icon(debugger_ui::icons::UNSUSPENDENT));
    assert_eq!(state["popup"]["title"], "main.py:5");
    h.press("escape");
    assert_eq!(h.debug()["popup"], Value::Null);
    assert_eq!(
        h.gutter()[2],
        (4, icon(debugger_ui::icons::UNSUSPENDENT), None, false)
    );

    // Step 5: a breakpoint's tooltip.
    h.hover_line(3);
    assert_eq!(
        h.debug()["tooltip"]["lines"],
        json!(["main.py:4", "Right-click to edit"])
    );
    assert_eq!(h.pointer(), CursorStyle::PointingHand);
    h.hover_line(4);
    assert_eq!(
        h.debug()["tooltip"]["lines"],
        json!(["main.py:5", "Suspend: none", "Log message: yes"])
    );

    // A middle click enables or disables; Shift+click on a breakpoint does
    // not remove it.
    h.press_line(3, MouseButton::Middle);
    assert!(!h.breakpoints()[1].2);
    h.press_line(3, MouseButton::Middle);
    h.click_line(3, shift);
    assert_eq!(h.breakpoints().len(), 3);

    // Step 6: a click removes.
    h.click_line(0, Modifiers::none());
    assert_eq!(
        h.breakpoints().iter().map(|b| b.0).collect::<Vec<_>>(),
        [4, 5]
    );

    // Alt+click on a line without one adds a temporary breakpoint; Toggle
    // Temporary Line Breakpoint at the caret too.
    h.click_line(6, alt);
    assert_eq!(h.debug()["breakpoints"][2]["temporary"], true);
    h.set_caret(5);
    h.press("cmd-alt-shift-f8");
    assert_eq!(h.debug()["breakpoints"][2]["line"], 6);
    assert_eq!(h.debug()["breakpoints"][2]["temporary"], true);

    // Mute Breakpoints: muted icons.
    h.invoke("debug.mute-breakpoints", json!({}));
    assert!(
        h.gutter()
            .iter()
            .all(|g| g.1.starts_with("breakpoints/breakpointMuted"))
    );
    h.invoke("debug.mute-breakpoints", json!({"muted": false}));

    // A file of another type takes no breakpoints.
    h.open("notes.txt");
    h.hover_line(0);
    assert_eq!(h.debug()["tooltip"], Value::Null);
    h.press("cmd-f8");
    assert_eq!(h.breakpoints().len(), 4);
}

/// Scenario `debugger-breakpoints.md`, step 7: breakpoints follow edits,
/// go with their lines and are kept with the project.
#[gpui::test]
fn breakpoints_follow_edits_and_stay_with_the_project(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, support::debug_hook());
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 4}),
    );
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 5}),
    );
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "other.py", "line": 2}),
    );
    h.invoke(
        "debug.edit-breakpoint",
        json!({"file": "main.py", "line": 5, "condition": "y > 1", "enabled": false}),
    );
    // Two lines typed above move them down.
    h.set_caret(0);
    h.invoke("editor.type", json!({"text": "a = 0\nb = 0\n"}));
    let lines: Vec<u64> = h.breakpoints().iter().map(|b| b.0).collect();
    assert_eq!(lines, [6, 7, 2]);
    assert_eq!(h.gutter().iter().map(|g| g.0).collect::<Vec<_>>(), [5, 6]);
    // The line of the first one deleted: it goes.
    h.set_caret(5);
    h.invoke("editor.delete-line", json!(null));
    let state = h.debug();
    let left: Vec<(String, u64)> = state["breakpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["file"].as_str().unwrap().to_owned(),
                b["line"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(left, [("main.py".into(), 6), ("other.py".into(), 2)]);
    assert_eq!(state["breakpoints"][0]["condition"], "y > 1");
    assert_eq!(
        state["breakpoints"][0]["badge"],
        icon(debugger_ui::icons::QUESTION_BADGE)
    );

    // The project opened again: the breakpoints come back with their state.
    h.invoke("workbench.save-all", json!(null));
    let view =
        h.cx.read(|cx| h.window.read(cx).unwrap().debugger_view().clone());
    h.cx.read(|cx| view.read(cx).debugger().unwrap().flush());
    h.open_project();
    let state = h.debug();
    assert_eq!(state["breakpoints"].as_array().unwrap().len(), 2);
    assert_eq!(state["breakpoints"][0]["line"], 6);
    assert_eq!(state["breakpoints"][0]["enabled"], false);
    assert_eq!(state["breakpoints"][0]["condition"], "y > 1");
    assert_eq!(state["breakpoints"][0]["suspend"], "thread");
    h.open("main.py");
    assert_eq!(
        h.gutter(),
        [(
            5,
            icon(debugger_ui::icons::DISABLED),
            Some(icon(debugger_ui::icons::QUESTION_BADGE)),
            false
        )]
    );
}

/// Scenario `debugger-conditional-breakpoint.md`, steps 2 and 4: the
/// properties popup of a right-click.
#[gpui::test]
fn the_properties_popup(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, support::debug_hook());
    h.click_line(3, Modifiers::none());
    h.press_line(3, MouseButton::Right);
    let popup = h.debug()["popup"].clone();
    assert_eq!(popup["title"], "main.py:4");
    assert_eq!(popup["enabled"], true);
    assert_eq!(popup["suspend"], true);
    assert_eq!(popup["policy"], "thread");
    assert_eq!(popup["default_policy"], "thread");
    assert_eq!(popup["make_default"], false);
    assert_eq!(popup["condition_check"], false);
    // The focus is in the condition field: typing and Enter (Done).
    h.cx.simulate_input(h.window.into(), "i == 3");
    h.press("enter");
    let state = h.debug();
    assert_eq!(state["popup"], Value::Null);
    assert_eq!(state["breakpoints"][0]["condition"], "i == 3");
    assert_eq!(
        h.gutter()[0].2.as_deref(),
        Some(debugger_ui::icons::QUESTION_BADGE.0)
    );
    // Again: the condition has its check box; Suspend off makes it a
    // non-suspending breakpoint at once.
    h.press_line(3, MouseButton::Right);
    assert_eq!(h.debug()["popup"]["condition_check"], true);
    assert_eq!(h.debug()["popup"]["condition"], "i == 3");
    h.click("bp-popup-suspend");
    assert_eq!(h.breakpoints()[0].3, "none");
    assert_eq!(h.breakpoints()[0].1, icon(debugger_ui::icons::UNSUSPENDENT));
    h.click("bp-popup-suspend");
    assert_eq!(h.breakpoints()[0].3, "thread");
    // All, then Make Default: new breakpoints suspend all threads.
    h.click("bp-popup-all");
    assert_eq!(h.breakpoints()[0].3, "all");
    assert_eq!(h.debug()["popup"]["make_default"], true);
    h.click("bp-popup-make-default");
    assert_eq!(h.debug()["default_suspend"], "all");
    // Enabled off.
    h.click("bp-popup-enabled");
    assert!(!h.breakpoints()[0].2);
    // More (Cmd+Shift+F8): the dialog with the breakpoint selected.
    assert_eq!(h.debug()["popup"]["title"], "main.py:4");
    h.press("cmd-shift-f8");
    let state = h.debug();
    assert_eq!(state["popup"], Value::Null);
    assert_eq!(state["dialog"]["detail"]["title"], "main.py:4");
    let dialog = h.dialog().expect("the dialog is open");
    h.press_in(dialog, "escape");
    assert!(h.dialog().is_none());
    // A new breakpoint takes the new default.
    h.click_line(4, Modifiers::none());
    assert_eq!(h.breakpoints()[1].3, "all");
    // Edit Breakpoint at the caret opens the popup; Escape closes it.
    h.set_caret(4);
    h.press("cmd-shift-f8");
    assert_eq!(h.debug()["popup"]["title"], "main.py:5");
    h.press("escape");
    assert_eq!(h.debug()["popup"], Value::Null);
}

/// Scenario `debugger-breakpoints-dialog.md`.
#[gpui::test]
fn the_breakpoints_dialog(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx, support::debug_hook());
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 4}),
    );
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "other.py", "line": 2}),
    );
    // Step 1: Cmd+Shift+F8 on a line without a breakpoint.
    h.set_caret(0);
    h.press("cmd-shift-f8");
    let dialog = h.dialog().expect("the dialog is open");
    let state = h.debug()["dialog"].clone();
    let rows: Vec<&str> = state["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        rows,
        [
            "Python Line Breakpoint",
            "main.py:4",
            "other.py:2",
            "Python Exception Breakpoint",
            "Any exception"
        ]
    );
    assert_eq!(state["detail"]["title"], "main.py:4");
    assert_eq!(state["detail"]["preview"]["line"], 4);
    assert_eq!(state["detail"]["preview"]["caret_line"], 4);
    assert_eq!(state["detail"]["preview"]["viewer"], true);

    // Step 2: Down, a condition, Enabled off (Space): the gutter follows.
    h.press_in(dialog, "down");
    assert_eq!(h.debug()["dialog"]["detail"]["title"], "other.py:2");
    h.invoke("breakpoints-dialog.set", json!({"condition": "b > 1"}));
    h.press_in(dialog, "space");
    let state = h.debug();
    assert_eq!(state["breakpoints"][1]["condition"], "b > 1");
    assert_eq!(state["breakpoints"][1]["enabled"], false);
    assert_eq!(state["dialog"]["rows"][2]["checked"], false);
    h.open("other.py");
    assert_eq!(
        h.gutter(),
        [(
            1,
            icon(debugger_ui::icons::DISABLED),
            Some(icon(debugger_ui::icons::QUESTION_BADGE)),
            false
        )]
    );

    // Step 3: Any exception and its activation policy.
    h.invoke(
        "breakpoints-dialog.select",
        json!({"class": "Any exception"}),
    );
    let detail = h.debug()["dialog"]["detail"].clone();
    assert_eq!(detail["title"], "Any exception");
    assert_eq!(
        detail["options"],
        json!([
            {"label": "On termination", "checked": true},
            {"label": "On raise", "checked": false},
            {"label": "Ignore library files", "checked": false},
        ])
    );
    h.invoke(
        "breakpoints-dialog.set",
        json!({"option": "notifyOnlyOnFirst", "value": true}),
    );
    assert_eq!(h.debug()["dialog"]["detail"]["options"][1]["checked"], true);

    // Step 4: an exception breakpoint for KeyError, selected.
    h.invoke(
        "breakpoints-dialog.add-exception",
        json!({"class": "KeyError"}),
    );
    let state = h.debug();
    assert_eq!(state["dialog"]["detail"]["title"], "KeyError");
    assert_eq!(state["exception_breakpoints"][1]["class"], "KeyError");
    assert_eq!(state["exception_breakpoints"][1]["enabled"], true);
    // Delete removes it; the default one stays.
    h.press_in(dialog, "delete");
    assert_eq!(
        h.debug()["exception_breakpoints"].as_array().unwrap().len(),
        1
    );
    h.invoke(
        "breakpoints-dialog.select",
        json!({"class": "Any exception"}),
    );
    h.press_in(dialog, "delete");
    assert_eq!(
        h.debug()["exception_breakpoints"].as_array().unwrap().len(),
        1
    );

    // Step 5: F4 opens the source at the line and closes the dialog.
    h.invoke(
        "breakpoints-dialog.select",
        json!({"file": "main.py", "line": 4}),
    );
    h.press_in(dialog, "f4");
    assert!(h.dialog().is_none());
    let state = h.invoke("workbench.state", json!(null));
    assert_eq!(
        state["tabs"][state["active_tab"].as_u64().unwrap() as usize]["path"],
        "main.py"
    );
    assert_eq!(state["editor"]["caret"]["line"], 3);
    // Delete in the reopened dialog removes a line breakpoint and its icon.
    h.invoke("debug.view-breakpoints", json!(null));
    let dialog = h.dialog().expect("the dialog is open again");
    h.invoke(
        "breakpoints-dialog.select",
        json!({"file": "main.py", "line": 4}),
    );
    h.press_in(dialog, "backspace");
    assert_eq!(h.breakpoints().len(), 1);
    assert!(h.gutter().is_empty());
    // Done.
    h.click_in_dialog(dialog, "breakpoints-done");
    assert!(h.dialog().is_none());
}

impl H<'_> {
    fn click_in_dialog(&mut self, dialog: AnyWindowHandle, selector: &'static str) {
        let mut vcx = VisualTestContext::from_window(dialog, self.cx);
        let at = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
        vcx.simulate_click(at, Modifiers::none());
        self.draw();
    }
}

/// A debugger whose adapter verifies the breakpoints it gets.
struct Verifying {
    script: Mutex<Option<Vec<Value>>>,
}

impl debugger::DebugAdapterProvider for Verifying {
    fn prepare(&self, request: &debugger::DebugRequest) -> Result<debugger::AdapterLaunch, String> {
        let script = self.script.lock().unwrap().take().ok_or("no script")?;
        Ok(debugger::AdapterLaunch {
            title: request.configuration.name.clone(),
            adapter: debugger::Adapter::Transport(dap_client::fake::adapter(script)),
            adapter_id: "fake".to_owned(),
            kind: debugger::LaunchKind::Launch,
            arguments: json!({}),
            input_file: None,
            evaluation_timeout: None,
        })
    }

    fn line_breakpoint_type(&self) -> Option<debugger::LineBreakpointType> {
        support::TestDebug.line_breakpoint_type()
    }

    fn stoppable_lines(&self, file: &Path, text: &str) -> Option<Vec<bool>> {
        support::TestDebug.stoppable_lines(file, text)
    }
}

/// During a session a breakpoint the adapter verified has the check badge;
/// after it, the plain icon.
#[gpui::test]
fn verified_breakpoints_during_a_session(cx: &mut TestAppContext) {
    let script = vec![
        json!({"expect": "initialize"}),
        json!({"respond": {"supportsConfigurationDoneRequest": true}}),
        json!({"expect": "launch"}),
        json!({"event": "initialized"}),
        json!({"expect": "setBreakpoints"}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 4, "id": 1}]}}),
        json!({"expect": "setExceptionBreakpoints"}),
        json!({"respond": null}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "launch"}),
        json!({"expect": "terminate"}),
        json!({"respond": null}),
        json!({"event": "terminated"}),
        json!({"expect": "disconnect"}),
        json!({"respond": null}),
    ];
    let provider = Verifying {
        script: Mutex::new(Some(script)),
    };
    let (mut h, _dir) = setup(cx, DebugHook::shared(Arc::new(provider)));
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 4}),
    );
    let view =
        h.cx.read(|cx| h.window.read(cx).unwrap().debugger_view().clone());
    let debugger = h.cx.read(|cx| view.read(cx).debugger().unwrap().clone());
    let request = debugger::DebugRequest::new(
        h.root.clone(),
        run_config::RunConfiguration::new("python", "main", run_config::Storage::Personal),
    );
    let session = debugger.start(request).unwrap();
    let start = Instant::now();
    while h.gutter()[0].1 != icon(debugger_ui::icons::VALID) {
        assert!(start.elapsed() < Duration::from_secs(20), "verified");
        support::tick(h.cx);
    }
    assert_eq!(h.debug()["breakpoints"][0]["verified"], true);
    let done = session.stop();
    let _ = done.recv_timeout(Duration::from_secs(20));
    let start = Instant::now();
    while h.gutter()[0].1 != icon(debugger_ui::icons::BREAKPOINT) {
        assert!(start.elapsed() < Duration::from_secs(20), "plain again");
        support::tick(h.cx);
    }
}
