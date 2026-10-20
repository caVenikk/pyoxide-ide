//! Settings > Python > Debugger, headless (GPUI's test platform), driven
//! through commands and clicks (tests/scenarios/settings/python-debugger.md).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use serde_json::{Value, json};
use workbench::Workbench;

const PAGE: &str = "python.debugger";

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

    fn try_invoke(&mut self, id: &str, args: Value) -> Result<Value, String> {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .map_err(|e| format!("{e:?}"));
        self.draw();
        out
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        self.try_invoke(id, args)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn page(&mut self) -> Value {
        self.invoke("settings.state", Value::Null)["fields"].clone()
    }

    fn edit(&mut self, field: &str, value: Value) {
        self.invoke("settings.edit", json!({"field": field, "value": value}));
    }

    fn workspace(&self) -> String {
        std::fs::read_to_string(project::workspace::workspace_file(&self.root)).unwrap_or_default()
    }

    fn settings(&mut self) -> WindowHandle<settings_ui::SettingsWindow> {
        self.cx
            .update(|cx| settings_ui::find(cx))
            .expect("the Settings window")
    }

    fn bounds(&mut self, selector: &'static str) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.draw();
        let settings = self.settings();
        VisualTestContext::from_window(settings.into(), self.cx).debug_bounds(selector)
    }

    fn click(&mut self, selector: &'static str) {
        let at = self
            .bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
            .center();
        let settings = self.settings();
        let vcx = &mut VisualTestContext::from_window(settings.into(), self.cx);
        vcx.simulate_mouse_move(at, None, Modifiers::none());
        vcx.simulate_click(at, Modifiers::none());
        self.draw();
    }

    /// Lets the writes and the search that wait for a pause run.
    fn settle(&mut self) {
        for _ in 0..50 {
            self.cx.executor().advance_clock(Duration::from_millis(150));
            self.draw();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Closes the Settings window, whatever the page's draft is.
    fn close_settings(&mut self) {
        self.cx.update(|cx| {
            if let Some(handle) = settings_ui::find(cx) {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        });
    }
}

fn open<'a>(cx: &'a mut TestAppContext, root: &Path) -> H<'a> {
    cx.executor().allow_parking();
    let product = Rc::new(pyoxide_ide::product_with_tools(Default::default()));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    let mut h = H {
        cx,
        window,
        root: root.to_owned(),
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    h
}

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("main.py"), "print('hi')\n").unwrap();
    (dir, root)
}

/// What the page needs to have read the options: the next frames.
fn open_page(h: &mut H<'_>) -> Value {
    h.invoke("settings.open", json!({"page": PAGE}));
    h.settle();
    h.page()
}

#[gpui::test]
fn the_page_shows_the_reference_options(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, &root);
    let s = h.invoke("settings.open", json!({"page": PAGE}));
    h.settle();
    let state = h.invoke("settings.state", Value::Null);
    assert_eq!(
        state["breadcrumbs"],
        json!(["Python", "Debugger"]),
        "{state}"
    );
    // After Interpreter and Type Engine (weights 200 and 190), before Tools.
    let rows: Vec<String> = state["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    let at = |id: &str| {
        rows.iter()
            .position(|r| r == id)
            .unwrap_or_else(|| panic!("{rows:?}"))
    };
    assert!(
        at("python.interpreter") < at("python.type-engine"),
        "{rows:?}"
    );
    assert!(at("python.type-engine") < at(PAGE), "{rows:?}");
    assert!(at(PAGE) < at("python.tools"), "{rows:?}");
    drop(s);

    let p = state["fields"].clone();
    assert_eq!(p["loaded"], json!(true), "{p}");
    assert_eq!(p["attach-to-subprocess"], json!(true));
    assert_eq!(p["gevent"], json!(false));
    assert_eq!(p["pyqt"], json!(true));
    assert_eq!(p["pyqt-backend"], json!("auto"));
    assert_eq!(p["evaluation-timeout"], json!("60000"));
    // Not supported yet: drawn, disabled.
    assert_eq!(
        p["collect-types"],
        json!({"checked": false, "enabled": false})
    );
    assert_eq!(
        p["drop-into-debugger"],
        json!({"checked": false, "enabled": false})
    );
    assert_eq!(
        p["attach-filter"],
        json!({"text": "python", "enabled": false})
    );
    assert_eq!(p["mode"]["selected"], json!("debugpy"));
    assert_eq!(p["mode"]["enabled"], json!([false, true]));
    assert_eq!(
        p["mode"]["description"],
        json!(
            "Python debugger backend built on the Debug Adapter Protocol, with a more stable connection model. Lower overhead on Python 3.12+."
        )
    );
    let names: Vec<&str> = p["pyqt-backends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Auto", "PyQt4", "PyQt5", "PyQt6", "PySide", "PySide2", "PySide6"
        ]
    );
    assert_eq!(state["modified"], json!([]), "{state}");
    for field in [
        "collect-types",
        "drop-into-debugger",
        "attach-filter",
        "mode",
    ] {
        // An edit of what is not supported yet changes nothing.
        h.edit(field, json!(true));
        assert_eq!(h.page(), p, "{field}");
    }
    for selector in [
        "debugger:attach-to-subprocess",
        "debugger:collect-types",
        "debugger:clear-caches",
        "debugger:gevent",
        "debugger:drop-into-debugger",
        "debugger:pyqt",
        "debugger:qt-backend",
        "debugger:timeout-up",
        "debugger:timeout-down",
        "debugger-mode:0",
        "debugger-mode:1",
    ] {
        assert!(h.bounds(selector).is_some(), "{selector} is not drawn");
    }
    // Nothing was written by looking at the page.
    assert_eq!(h.workspace(), "");
    h.close_settings();
}

#[gpui::test]
fn applied_options_go_to_the_project_state(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, &root);
    open_page(&mut h);
    // A click on the check box and its label toggles; Apply writes.
    h.click("debugger:attach-to-subprocess");
    h.click("debugger:gevent");
    let state = h.invoke("settings.state", Value::Null);
    assert_eq!(state["modified"], json!([PAGE]), "{state}");
    assert_eq!(state["fields"]["attach-to-subprocess"], json!(false));
    assert_eq!(state["fields"]["gevent"], json!(true));
    assert_eq!(h.workspace(), "", "a draft until Apply");

    // The spinner's arrows step by 500; the number stays in range.
    h.click("debugger:timeout-up");
    assert_eq!(h.page()["evaluation-timeout"], json!("60500"));
    h.click("debugger:timeout-down");
    h.click("debugger:timeout-down");
    assert_eq!(h.page()["evaluation-timeout"], json!("59500"));
    h.edit("evaluation-timeout", json!(200));
    h.click("debugger:timeout-down");
    assert_eq!(h.page()["evaluation-timeout"], json!("0"));
    h.click("debugger:timeout-down");
    assert_eq!(h.page()["evaluation-timeout"], json!("0"));
    h.edit("evaluation-timeout", json!(30000));

    // The backend's list: PyQt6 and PySide6 are not for debugpy.
    h.click("debugger:qt-backend");
    let p = h.page();
    let items = p["list"]["levels"][0]["items"].as_array().unwrap().clone();
    let labels: Vec<_> = items.iter().map(|i| i["label"].clone()).collect();
    assert_eq!(
        labels,
        [
            "Auto", "PyQt4", "PyQt5", "PyQt6", "PySide", "PySide2", "PySide6"
        ]
        .map(Value::from),
        "{p}"
    );
    assert_eq!(items[3]["enabled"], json!(false), "{p}");
    assert_eq!(items[2]["enabled"], json!(true), "{p}");
    h.edit("pyqt-backend", json!("pyqt5"));
    assert_eq!(h.page()["list"], Value::Null);
    // debugpy has no Qt 6 support: the choice is refused.
    h.edit("pyqt-backend", json!("pyqt6"));
    assert_eq!(h.page()["pyqt-backend"], json!("pyqt5"));

    h.invoke("settings.apply", Value::Null);
    h.settle();
    let text = h.workspace();
    assert!(text.contains("[python.debugger]"), "{text}");
    assert!(text.contains("attach-to-subprocess = false"), "{text}");
    assert!(text.contains("gevent = true"), "{text}");
    assert!(text.contains("pyqt-backend = \"pyqt5\""), "{text}");
    assert!(text.contains("evaluation-timeout = 30000"), "{text}");
    assert!(!text.contains("pyqt = "), "{text}");
    let stored = py_debug::settings::load(&root).unwrap();
    assert!(!stored.attach_to_subprocess && stored.gevent && stored.pyqt);
    assert_eq!(stored.evaluation_timeout_ms, 30000);
    let state = h.invoke("settings.state", Value::Null);
    assert_eq!(state["modified"], json!([]), "{state}");

    // PyQt compatible off: the combo box is disabled with it.
    h.click("debugger:pyqt");
    let p = h.page();
    assert_eq!(p["pyqt"], json!(false));
    assert_eq!(p["pyqt-backend-enabled"], json!(false));
    h.click("debugger:qt-backend");
    assert_eq!(h.page()["list"], Value::Null, "a disabled combo box");
    // "Revert changes" goes back to what is stored.
    h.invoke("settings.revert", Value::Null);
    h.settle();
    let p = h.page();
    assert_eq!(p["pyqt"], json!(true), "{p}");
    assert_eq!(p["evaluation-timeout"], json!("30000"));
    h.close_settings();
}

#[gpui::test]
fn a_timeout_that_is_no_number_is_an_error(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, &root);
    open_page(&mut h);
    h.edit("evaluation-timeout", json!("soon"));
    let p = h.page();
    assert_eq!(p["evaluation-timeout-valid"], json!(false), "{p}");
    let state = h.invoke("settings.state", Value::Null);
    assert_eq!(state["modified"], json!([PAGE]), "{state}");
    h.invoke("settings.apply", Value::Null);
    let state = h.invoke("settings.state", Value::Null);
    assert_eq!(
        state["errors"][PAGE],
        json!("Please enter a number from 0 to 2147483647"),
        "{state}"
    );
    assert_eq!(h.workspace(), "");
    h.edit("evaluation-timeout", json!(2147483648_u64));
    assert_eq!(h.page()["evaluation-timeout-valid"], json!(false));
    h.edit("evaluation-timeout", json!(2147483647_u64));
    assert_eq!(h.page()["evaluation-timeout-valid"], json!(true));
    h.close_settings();
}

#[gpui::test]
fn the_references_options_of_the_project_are_picked_up_once(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    std::fs::create_dir_all(root.join(".idea")).unwrap();
    std::fs::write(
        root.join(".idea/workspace.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PyDebuggerOptionsProvider">
    <option name="myAttachToSubprocess" value="false" />
    <option name="myEvaluationResponseTimeout" value="45000" />
  </component>
</project>"#,
    )
    .unwrap();
    let mut h = open(cx, &root);
    let p = open_page(&mut h);
    assert_eq!(p["attach-to-subprocess"], json!(false), "{p}");
    assert_eq!(p["evaluation-timeout"], json!("45000"), "{p}");
    let text = h.workspace();
    assert!(text.contains("imported-from-idea = true"), "{text}");
    assert!(text.contains("evaluation-timeout = 45000"), "{text}");
    // `.idea` is never written, and what changes in it later stays out.
    let before = std::fs::read_to_string(root.join(".idea/workspace.xml")).unwrap();
    assert!(before.contains("45000"));
    h.close_settings();
    std::fs::write(
        root.join(".idea/workspace.xml"),
        before.replace("45000", "1000"),
    )
    .unwrap();
    let p = open_page(&mut h);
    assert_eq!(p["evaluation-timeout"], json!("45000"), "{p}");
    h.close_settings();
}

#[gpui::test]
fn the_search_finds_the_page_by_its_labels(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, &root);
    h.invoke("settings.open", Value::Null);
    for (text, found) in [
        ("to subprocess automatically", true),
        ("evaluation response timeout", true),
        ("gevent", true),
        ("Attach To Process", true),
        ("Clear caches", true),
        ("pydevd", true),
        ("Debug Adapter Protocol", true),
        ("zzzz", false),
    ] {
        h.invoke("settings.search", json!({"text": text}));
        h.settle();
        let state = h.invoke("settings.state", Value::Null);
        let rows: Vec<&str> = state["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(rows.contains(&PAGE), found, "{text}: {rows:?}");
        if found {
            assert_eq!(state["page"], json!(PAGE), "{text}: {state}");
        }
    }
    // The first option's row is lit while it is found.
    h.invoke(
        "settings.search",
        json!({"text": "to subprocess automatically"}),
    );
    h.settle();
    assert!(h.bounds("settings-spotlight").is_some());
    h.close_settings();
}
