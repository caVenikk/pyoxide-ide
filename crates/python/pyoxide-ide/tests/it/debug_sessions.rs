//! Debug sessions of the Pyoxide window with the real debugpy helper and a
//! real Python (tests/scenarios/run-debug/debugger-*.md). Ignored by
//! default: they need Python 3.8 or later and the helper.
//!
//! ```sh
//! python3 scripts/fetch-debugpy.py          # once: the helper into the app cache
//! cargo test -p pyoxide-ide --test it debug_sessions:: -- --ignored
//! ```
//!
//! `PYOXIDE_TEST_PYTHON` names the interpreter that makes the project's
//! virtual environment (default `python3`, `python` on Windows).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::Workbench;

const MAIN: &str = r#"import json


def f(a):
    b = a + 1
    return b


def mine():
    return 2


print("start")
f(1)
text = json.dumps({"a": 1})
value = json.loads(text)
n = mine()
line = input()
print("got", line)
"#;

fn python() -> PathBuf {
    std::env::var_os("PYOXIDE_TEST_PYTHON")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "python" } else { "python3" }))
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
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

    /// Waits (at most about a minute) for a debug state that `f` accepts.
    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..3000 {
            self.cx.executor().advance_clock(Duration::from_millis(20));
            self.cx.run_until_parked();
            last = self.debug();
            if f(&last) {
                return last;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    /// Waits for a stop at a line of main.py (from 1).
    fn wait_line(&mut self, line: u32) -> Value {
        self.wait(&format!("a stop on line {line}"), |s| {
            s["tool_window"]["tabs"][0]["state"] == json!("suspended")
                && s["execution_point"]["file"] == json!("main.py")
                && s["execution_point"]["line"] == json!(line)
        })
    }

    fn console(&mut self) -> String {
        self.debug()["tool_window"]["tabs"][0]["console"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn top_frame(&mut self) -> String {
        self.debug()["tool_window"]["tabs"][0]["frames"][0]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Steps and waits for the next stop.
    fn step(&mut self, command: &str) -> String {
        self.invoke(command, json!(null));
        self.wait(&format!("the stop after {command}"), |s| {
            s["tool_window"]["tabs"][0]["state"] == json!("suspended")
                && !s["tool_window"]["tabs"][0]["frames"]
                    .as_array()
                    .is_none_or(|f| f.is_empty())
        });
        self.top_frame()
    }
}

/// A project with main.py and a virtual environment made by [`python`].
fn project() -> (tempfile::TempDir, PathBuf) {
    project_with(MAIN)
}

/// A project whose main.py is `text`.
fn project_with(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("main.py"), text).unwrap();
    let made = std::process::Command::new(python())
        .args(["-m", "venv", "--without-pip", ".venv"])
        .current_dir(&root)
        .status()
        .expect("python runs");
    assert!(made.success(), "the virtual environment is made");
    (dir, root)
}

fn open<'a>(cx: &'a mut TestAppContext, root: &Path) -> H<'a> {
    cx.executor().allow_parking();
    let product = Rc::new(workbench::Product {
        code_intel: Vec::new(),
        ..pyoxide_ide::product()
    });
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    let mut h = H { cx, window };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    h.wait("the project's debugger", |s| s["loaded"] == json!(true));
    h.invoke("project.open-file", json!({"path": "main.py"}));
    h
}

/// The program of the frames and variables test: a stop in `describe`
/// (line 4) called from the module (line 11).
const VARIABLES: &str = r#"def describe(items):
    summary = {"count": len(items), "sum": 0}
    for value in items:
        summary["sum"] += value
    return summary


numbers = [1, 2, 3, 4]
big = list(range(1000))
text = "x" * 10000
print(describe(numbers))
"#;

impl H<'_> {
    /// The Threads & Variables tab of the first session.
    fn tab(&mut self) -> Value {
        self.debug()["tool_window"]["tabs"][0].clone()
    }

    /// Waits for the variables view to show every row it is loading.
    fn variables(&mut self) -> Vec<Value> {
        let s = self.wait("the variables", |s| {
            let rows = &s["tool_window"]["tabs"][0]["variables"]["rows"];
            rows.as_array()
                .is_some_and(|r| !r.is_empty() && r.iter().all(|r| r["kind"] != json!("loading")))
        });
        s["tool_window"]["tabs"][0]["variables"]["rows"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// The texts of the variables view's rows at a depth.
    fn variable_texts(&mut self, depth: u64) -> Vec<String> {
        self.variables()
            .iter()
            .filter(|r| r["depth"] == json!(depth))
            .map(|r| r["text"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// Threads & Variables at a stop: the thread combo, the frames (the
/// library's frames marked), the values of the selected frame with their
/// types and sizes; a node's children load when it is expanded, a large
/// list in pages; another frame shows its own values; a step marks the
/// values that changed.
#[gpui::test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn frames_and_variables_at_a_stop(cx: &mut TestAppContext) {
    let (_dir, root) = project_with(VARIABLES);
    let mut h = open(cx, &root);
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 4}),
    );
    h.invoke("debug.debug", json!(null));
    h.wait_line(4);
    let tab = h.tab();
    assert_eq!(tab["content"], json!("threads-variables"), "{tab:#}");
    let frames = &tab["frames_view"];
    assert_eq!(frames["thread"], json!("MainThread"), "{frames:#}");
    assert_eq!(frames["selected"], json!(0), "{frames:#}");
    let rows = frames["rows"].as_array().unwrap();
    assert_eq!(
        rows[0]["text"],
        json!("describe, main.py:4:1"),
        "{frames:#}"
    );
    assert_eq!(rows[0]["library"], json!(false));
    assert_eq!(
        rows[1]["text"],
        json!("<module>, main.py:11:1"),
        "{frames:#}"
    );
    assert!(
        rows[2..].iter().all(|r| r["library"] == json!(true)),
        "the interpreter's frames are library frames: {frames:#}"
    );
    // The locals of describe, by name, with types and sizes.
    let texts = h.variable_texts(0);
    assert_eq!(
        texts,
        [
            "items = {list: 4} [1, 2, 3, 4]",
            "summary = {dict: 2} {'count': 4, 'sum': 0}",
            "value = {int} 1",
        ],
        "{texts:?}"
    );
    // A list's items, its __len__ and its protected attributes, in this order.
    h.invoke(
        "debug.variables.expand",
        json!({"path": ["items"], "expanded": true}),
    );
    let texts = h.variable_texts(1);
    assert_eq!(
        texts,
        [
            "0 = {int} 1",
            "1 = {int} 2",
            "2 = {int} 3",
            "3 = {int} 4",
            "__len__ = {int} 4",
            "Protected Attributes",
        ],
        "{texts:?}"
    );
    // The caller's frame: its values; the editor shows its line.
    h.invoke("debug.next-frame", json!(null));
    let s = h.wait("the caller's frame", |s| {
        s["tool_window"]["tabs"][0]["frames_view"]["selected"] == json!(1)
    });
    assert_eq!(s["execution_point"]["line"], json!(11), "{s:#}");
    let texts = h.variable_texts(0);
    assert!(
        texts.contains(&"numbers = {list: 4} [1, 2, 3, 4]".to_owned()),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("big = {list: 1000} [0, 1, 2")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.starts_with("text = {str} 'xxx")),
        "{texts:?}"
    );
    // A large list: the first hundred items, then the rest on demand.
    h.invoke(
        "debug.variables.expand",
        json!({"path": ["big"], "expanded": true}),
    );
    let texts = h.variable_texts(1);
    assert_eq!(texts[0], "000 = {int} 0", "{texts:?}");
    assert_eq!(texts[99], "099 = {int} 99", "{texts:?}");
    assert_eq!(
        texts[100], "... (900 more items. Double-click to see)",
        "{texts:?}"
    );
    let more = h
        .variables()
        .iter()
        .find(|r| r["kind"] == json!("more"))
        .unwrap()["path"]
        .clone();
    h.invoke("debug.variables.expand", json!({"path": more}));
    let texts = h.variable_texts(1);
    assert_eq!(texts[999], "999 = {int} 999", "{texts:?}");
    assert_eq!(texts[1000], "__len__ = {int} 1000", "{texts:?}");
    // Back to the top frame, then a step: the sum changed.
    h.invoke("debug.previous-frame", json!(null));
    h.wait("the top frame", |s| {
        s["tool_window"]["tabs"][0]["frames_view"]["selected"] == json!(0)
    });
    h.variables();
    h.step("debug.step-over");
    h.wait_line(3);
    let rows = h.variables();
    let summary = rows.iter().find(|r| r["name"] == json!("summary")).unwrap();
    assert_eq!(
        summary["text"],
        json!("summary = {dict: 2} {'count': 4, 'sum': 1}")
    );
    assert_eq!(summary["changed"], json!(true), "{rows:#?}");
    let value = rows.iter().find(|r| r["name"] == json!("value")).unwrap();
    assert_eq!(value["changed"], json!(false), "{rows:#?}");
    // The expanded list stays expanded.
    assert!(
        rows.iter().any(|r| r["path"] == json!(["items", "0"])),
        "{rows:#?}"
    );
    h.invoke("run.stop", json!(null));
    h.wait("the end", |s| {
        s["tool_window"]["tabs"][0]["state"] == json!("ended")
    });
}

impl H<'_> {
    /// The variables view's row with these names, once its value is there.
    fn row(&mut self, names: Value) -> Value {
        let s = self.wait(&format!("the row {names}"), |s| {
            s["tool_window"]["tabs"][0]["variables"]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| {
                    r["path"] == names
                        && r["kind"] != json!("loading")
                        && r["state"] != "pending"
                        && r["state"] != "none"
                })
        });
        s["tool_window"]["tabs"][0]["variables"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["path"] == names)
            .unwrap()
            .clone()
    }
}

/// The evaluate field, the watches, Set Value, Copy Value and Jump To
/// Source against debugpy (tests/scenarios/run-debug/debugger-evaluate-
/// watches.md, debugger-variables.md): Enter's result is the first node,
/// an error shows as one; watches are evaluated at every stop, changed
/// values marked; a set value shows in the tree and the watches; the
/// watches are there again in the next session of the configuration type.
#[gpui::test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn evaluate_and_watches_at_a_stop(cx: &mut TestAppContext) {
    let (_dir, root) = project_with(VARIABLES);
    let mut h = open(cx, &root);
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 4}),
    );
    h.invoke("debug.debug", json!(null));
    h.wait_line(4);
    h.variables();
    // Enter in the evaluate field: the result node, first.
    h.invoke(
        "debug.variables.evaluate",
        json!({"expression": "len(items)"}),
    );
    let result = h.row(json!(["result"]));
    assert_eq!(result["text"], json!("result = {int} 4"), "{result:#}");
    assert_eq!(h.variables()[0]["path"], json!(["result"]));
    h.invoke("debug.variables.evaluate", json!({"expression": "summary"}));
    let result = h.row(json!(["result"]));
    assert_eq!(
        result["text"],
        json!("result = {dict: 2} {'count': 4, 'sum': 0}"),
        "{result:#}"
    );
    h.invoke(
        "debug.variables.expand",
        json!({"path": ["result"], "expanded": true}),
    );
    let count = h.row(json!(["result", "'count'"]));
    assert_eq!(count["text"], json!("'count' = {int} 4"), "{count:#}");
    // An error, as pydevd shows it; the field marks the name the frame
    // does not declare.
    h.invoke(
        "debug.variables.evaluate",
        json!({"expression": "undefined_name"}),
    );
    let result = h.row(json!(["result"]));
    assert_eq!(result["state"], json!("error"), "{result:#}");
    assert_eq!(
        result["error"],
        json!("{NameError}NameError(\"name 'undefined_name' is not defined\")"),
        "{result:#}"
    );
    let field = h.wait("the field's unresolved name", |s| {
        s["tool_window"]["tabs"][0]["variables"]["evaluate"]["unresolved"] == json!([[0, 14]])
    });
    let field = &field["tool_window"]["tabs"][0]["variables"]["evaluate"];
    assert_eq!(field["text"], json!("undefined_name"), "{field:#}");
    // Jump To Type Source: where the running program has the type.
    h.invoke(
        "debug.variables.evaluate",
        json!({"expression": "__import__('fractions').Fraction(1, 3)"}),
    );
    h.row(json!(["result"]));
    h.invoke("debug.variables.select", json!({"path": ["result"]}));
    h.invoke("debug.variables.jump-to-type-source", json!(null));
    let mut editor = Value::Null;
    for _ in 0..200 {
        h.cx.executor().advance_clock(Duration::from_millis(20));
        editor = h.invoke("workbench.state", json!(null))["editor"].clone();
        if editor["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("fractions.py"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        editor["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("fractions.py")),
        "{editor:#}"
    );
    h.invoke("project.open-file", json!({"path": "main.py"}));
    h.invoke(
        "debug.variables.evaluate",
        json!({"expression": "undefined_name"}),
    );
    h.row(json!(["result"]));
    // The history keeps the expressions, the latest first.
    let field = h.tab()["variables"]["evaluate"].clone();
    assert_eq!(
        field["history"],
        json!([
            "undefined_name",
            "__import__('fractions').Fraction(1, 3)",
            "summary",
            "len(items)"
        ]),
        "{field:#}"
    );
    // Watches: from the field and New Watch.
    h.invoke(
        "debug.variables.watch-expression",
        json!({"expression": "items[0]"}),
    );
    h.invoke(
        "debug.new-watch",
        json!({"expression": "summary['sum'] + 1"}),
    );
    let first = h.row(json!(["items[0]"]));
    assert_eq!(first["text"], json!("items[0] = {int} 1"), "{first:#}");
    // View as: an integer in hex, then as before.
    h.invoke("debug.variables.select", json!({"path": ["items[0]"]}));
    h.invoke("debug.variables.view-as", json!({"renderer": "Hex"}));
    let first = h.row(json!(["items[0]"]));
    assert_eq!(first["text"], json!("items[0] = {int} 0x1"), "{first:#}");
    h.invoke("debug.variables.view-as", json!({"renderer": "Decimal"}));
    let sum = h.row(json!(["summary['sum'] + 1"]));
    assert_eq!(
        sum["text"],
        json!("summary['sum'] + 1 = {int} 1"),
        "{sum:#}"
    );
    let rows = h.variables();
    let order: Vec<&Value> = rows.iter().take(3).map(|r| &r["path"][0]).collect();
    assert_eq!(
        order,
        [
            &json!("result"),
            &json!("items[0]"),
            &json!("summary['sum'] + 1")
        ]
    );
    // A step: the result goes, the watches are evaluated again, the
    // changed one marked.
    h.step("debug.step-over");
    h.wait_line(3);
    let sum = h.row(json!(["summary['sum'] + 1"]));
    assert_eq!(
        sum["text"],
        json!("summary['sum'] + 1 = {int} 2"),
        "{sum:#}"
    );
    assert_eq!(sum["changed"], json!(true), "{sum:#}");
    assert!(
        !h.variables().iter().any(|r| r["path"] == json!(["result"])),
        "the result goes at a step"
    );
    // Set Value: the tree and the watches show it.
    h.invoke("debug.variables.select", json!({"path": ["items"]}));
    h.invoke(
        "debug.variables.expand",
        json!({"path": ["items"], "expanded": true}),
    );
    h.variables();
    h.invoke("debug.variables.select", json!({"path": ["items", "0"]}));
    h.invoke("debug.variables.set-value", json!({"value": "7"}));
    h.wait("the value set", |s| {
        s["tool_window"]["tabs"][0]["variables"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["text"] == json!("items[0] = {int} 7"))
    });
    let zero = h.row(json!(["items", "0"]));
    assert_eq!(zero["text"], json!("0 = {int} 7"), "{zero:#}");
    // Copy Value: the whole value.
    h.invoke("debug.variables.select", json!({"path": ["summary"]}));
    h.invoke("debug.variables.copy-value", json!(null));
    let mut copied = None;
    for _ in 0..200 {
        h.cx.executor().advance_clock(Duration::from_millis(20));
        h.cx.run_until_parked();
        copied = h.cx.read_from_clipboard().and_then(|c| c.text());
        if copied.as_deref().is_some_and(|c| c.starts_with('{')) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(copied.as_deref(), Some("{'count': 4, 'sum': 1}"));
    // Jump To Source: the declaration of summary in describe.
    h.invoke("debug.variables.jump-to-source", json!(null));
    let mut editor = Value::Null;
    for _ in 0..200 {
        h.cx.executor().advance_clock(Duration::from_millis(20));
        editor = h.invoke("workbench.state", json!(null))["editor"].clone();
        if editor["caret"] == json!({"line": 1, "column": 4}) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        editor["caret"],
        json!({"line": 1, "column": 4}),
        "{editor:#}"
    );
    assert!(editor["path"].as_str().unwrap().ends_with("main.py"));
    // Remove Watch.
    h.invoke("debug.remove-watch", json!({"expression": "items[0]"}));
    assert_eq!(
        h.tab()["variables"]["watches"],
        json!(["summary['sum'] + 1"])
    );
    // The next session of the configuration type has the watches.
    h.invoke("run.stop", json!(null));
    h.wait("the end", |s| {
        s["tool_window"]["tabs"][0]["state"] == json!("ended")
    });
    h.invoke("debug.close-tab", json!({"index": 0}));
    h.invoke("debug.debug", json!(null));
    h.wait_line(4);
    let sum = h.row(json!(["summary['sum'] + 1"]));
    assert_eq!(
        sum["text"],
        json!("summary['sum'] + 1 = {int} 1"),
        "{sum:#}"
    );
    h.invoke("run.stop", json!(null));
    h.wait("the end", |s| {
        s["tool_window"]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["state"] == json!("ended"))
    });
}

#[cfg(unix)]
fn alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A breakpoint stops the program; the console shows its output; Step Over,
/// Step Into, Step Out, Step Into My Code (past library code), Run to
/// Cursor; the console's input reaches the program, which ends.
#[gpui::test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn stop_step_and_run_to_the_end(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, &root);
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 5}),
    );
    h.invoke("debug.debug", json!(null));
    let s = h.wait_line(5);
    assert_eq!(s["visible"], json!(true));
    assert_eq!(s["tool_window"]["tabs"][0]["title"], json!("main"));
    assert!(
        h.top_frame().starts_with("f, main.py:5"),
        "{}",
        h.top_frame()
    );
    assert!(h.console().contains("start"), "{}", h.console());
    // Step Over, then Step Out to the caller.
    h.step("debug.step-over");
    h.wait_line(6);
    h.step("debug.step-out");
    let frame = h.top_frame();
    assert!(frame.starts_with("<module>, main.py:"), "{frame}");
    // Run to Cursor: the line of json.dumps.
    h.invoke(
        "debug.run-to-cursor",
        json!({"file": "main.py", "line": 15}),
    );
    h.wait_line(15);
    // Step Into My Code skips json's code.
    h.step("debug.step-into-my-code");
    let frame = h.top_frame();
    assert!(frame.contains("main.py:"), "Step Into My Code: {frame}");
    // Step Into enters it.
    h.wait_line(16);
    let frame = h.step("debug.step-into");
    assert!(!frame.contains("main.py"), "Step Into: {frame}");
    h.step("debug.step-out");
    // The rest runs: the program waits for a line from the console.
    h.invoke("debug.resume", json!(null));
    h.wait("the program to run", |s| {
        s["tool_window"]["tabs"][0]["state"] == json!("running")
    });
    h.invoke("debug.select-content", json!({"content": "console"}));
    h.invoke("debug.console.type", json!({"text": "hi\n"}));
    let s = h.wait("the end", |s| {
        s["tool_window"]["tabs"][0]["exit_code"] == json!(0)
    });
    assert_eq!(s["tool_window"]["tabs"][0]["state"], json!("ended"));
    let console = h.console();
    assert!(console.contains("got hi"), "{console}");
    assert!(
        console.contains("Process finished with exit code 0"),
        "{console}"
    );
}

/// Stop ends the adapter, the launcher and the program; Rerun debugs
/// again in the same tab; closing the tab stops the session.
#[gpui::test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn stop_rerun_and_close_leave_no_process(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, &root);
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 5}),
    );
    h.invoke("debug.debug", json!(null));
    h.wait_line(5);
    let processes = h.wait("the processes", |s| {
        s["sessions"][0]["processes"]["debuggee"].is_number()
            && s["sessions"][0]["processes"]["adapter"].is_number()
    })["sessions"][0]["processes"]
        .clone();
    h.invoke("run.stop", json!(null));
    h.wait("the end", |s| {
        s["tool_window"]["tabs"][0]["state"] == json!("ended")
    });
    #[cfg(unix)]
    for pid in ["debuggee", "adapter"].map(|p| processes[p].as_u64().unwrap()) {
        let mut gone = false;
        for _ in 0..250 {
            if !alive(pid) {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(gone, "process {pid} is gone");
    }
    #[cfg(not(unix))]
    let _ = processes;
    // Rerun: the same tab.
    h.invoke("debug.rerun", json!(null));
    h.wait_line(5);
    assert_eq!(
        h.debug()["tool_window"]["tabs"].as_array().unwrap().len(),
        1
    );
    // Closing the tab stops it.
    h.invoke("debug.close-tab", json!(null));
    h.wait("no session", |s| {
        s["sessions"]
            .as_array()
            .is_none_or(|l| l.iter().all(|x| x["state"] == json!("ended")))
    });
}

/// A FastAPI application started by `main.py` with uvicorn's reloader:
/// the server runs in a subprocess, which the debugger attaches to. A
/// breakpoint in an `async def` endpoint stops the session in the Debug
/// tool window when the endpoint is called; Stop ends the server too.
/// The project's environment is made with `uv` (fastapi, uvicorn).
#[gpui::test]
#[ignore = "needs uv (fastapi, uvicorn from its cache or the network) and the debugpy helper"]
fn a_breakpoint_in_a_web_app_with_a_reloader(cx: &mut TestAppContext) {
    use std::io::{Read, Write};

    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::fs::write(
        root.join("api.py"),
        "import os\n\nfrom fastapi import FastAPI\n\napp = FastAPI()\n\n\n@app.get(\"/hello\")\nasync def hello():\n    pid = os.getpid()\n    return {\"pid\": pid}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("main.py"),
        format!(
            "import uvicorn\n\nif __name__ == \"__main__\":\n    uvicorn.run(\"api:app\", port={port}, reload=True)\n"
        ),
    )
    .unwrap();
    let venv = root.join(".venv");
    for args in [
        vec!["venv", "--quiet", ".venv"],
        vec!["pip", "install", "--quiet", "fastapi", "uvicorn"],
    ] {
        let status = std::process::Command::new("uv")
            .args(&args)
            .current_dir(&root)
            .env("VIRTUAL_ENV", &venv)
            .status()
            .expect("uv runs");
        assert!(status.success(), "uv {args:?}");
    }

    let mut h = open(cx, &root);
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "api.py", "line": 10}),
    );
    h.invoke("debug.debug", json!(null));
    let (tx, answer) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..600 {
            if let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                stream
                    .write_all(
                        b"GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
                let mut response = String::new();
                let _ = stream.read_to_string(&mut response);
                let _ = tx.send(response);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let s = h.wait("a stop in the endpoint", |s| {
        s["tool_window"]["tabs"][0]["state"] == json!("suspended")
            && s["execution_point"]["file"] == json!("api.py")
            && s["execution_point"]["line"] == json!(10)
    });
    assert!(
        s["sessions"][0]["subprocesses"].as_u64().unwrap_or(0) >= 1,
        "{s:#}"
    );
    assert!(
        h.top_frame().starts_with("hello, api.py:10"),
        "{}",
        h.top_frame()
    );
    h.invoke("debug.resume", json!(null));
    let response = answer.recv_timeout(Duration::from_secs(60)).unwrap();
    let body = response.split_once("\r\n\r\n").unwrap().1;
    let server: Value = serde_json::from_str(body).unwrap();
    let server = server["pid"].as_u64().unwrap();
    h.invoke("run.stop", json!(null));
    h.wait("the end", |s| {
        s["tool_window"]["tabs"][0]["state"] == json!("ended")
    });
    #[cfg(unix)]
    {
        let mut gone = false;
        for _ in 0..500 {
            if !alive(server) {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(gone, "the server {server} is gone");
    }
    #[cfg(not(unix))]
    let _ = server;
}
