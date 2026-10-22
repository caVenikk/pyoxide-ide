//! Debug sessions in the window (tests/scenarios/run-debug/debugger-*.md)
//! with scripted debug adapters in the test's process
//! (`dap_client::fake`, see tests/support): the Debug tool window, its tab,
//! toolbar and console, the execution point in the editor, Stop and Rerun
//! shared with Run, "Process 'x' Is Running", the run widget's Debug,
//! Resume and Pause, the dropdown's and the Run/Debug Configurations
//! dialog's Debug.

use crate::support;

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use ui::icons::IconName;
use workbench::{FileType, Product, Workbench};

use crate::support::{
    debug_answer, debug_end, debug_start, debug_stop, long_program, push_debug_script,
    quick_program, tick,
};

const MAIN: &str = "def f(a):\n    b = a + 1\n    return b\n\nprint(\"start\")\nf(1)\n";

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        for window in self.cx.update(|cx| cx.windows()) {
            let _ = self
                .cx
                .update_window(window, |_, window, cx| window.draw(cx).clear());
        }
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

    fn state(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))
    }

    fn run(&mut self) -> Value {
        self.invoke("run.state", json!(null))
    }

    fn debug(&mut self) -> Value {
        self.invoke("debug.state", json!(null))
    }

    fn tab(&mut self) -> Value {
        self.debug()["tool_window"]["tabs"][0].clone()
    }

    /// Waits (at most about 20 s) for a debug state that `f` accepts.
    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            tick(self.cx);
            last = self.debug();
            if f(&last) {
                self.draw();
                return last;
            }
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    /// Waits for a run state that `f` accepts.
    fn wait_run(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            tick(self.cx);
            last = self.run();
            if f(&last) {
                return last;
            }
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    fn wait_state(&mut self, state: &str) -> Value {
        self.wait(&format!("the session {state}"), |s| {
            s["tool_window"]["tabs"][0]["state"] == json!(state)
        })
    }

    /// The text of the first session's console, line by line.
    fn console(&mut self) -> Vec<String> {
        self.tab()["console"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["text"].as_str().unwrap().to_owned())
            .collect()
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Debugs main.py with an adapter that starts `program` and stops on
    /// line 2, then runs `after`.
    fn debug_main(&mut self, program: Option<Vec<String>>, after: Vec<Value>) -> Value {
        let main = self.file("main.py");
        let mut script = debug_start(&self.root, program);
        script.extend(debug_stop(&main, 2, "breakpoint"));
        script.extend(after);
        push_debug_script(script);
        self.invoke("debug.debug", json!(null));
        self.wait("the stop on line 2", |s| {
            s["tool_window"]["tabs"][0]["state"] == json!("suspended")
                && s["execution_point"]["line"] == json!(2)
        })
    }

    /// Chooses a button of the open message dialog by its label.
    fn choose(&mut self, button: &str) {
        let dialog = self
            .window
            .read_with(self.cx, |w, cx| w.dialog_focus(cx))
            .unwrap()
            .expect("a dialog is open");
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&dialog))
            .unwrap();
        self.invoke("dialog.choose", json!({ "button": button }));
    }
}

fn python() -> FileType {
    FileType::plain("Python", &["py"], IconName::FILE_TEXT)
}

fn setup(cx: &mut TestAppContext) -> (H<'_>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("main.py"), MAIN).unwrap();
    std::fs::write(root.join("other.py"), "x = 1\n").unwrap();
    let product = Product {
        name: "Test IDE",
        file_types: vec![python()],
        run: Some(support::run_hook()),
        debug: Some(support::debug_hook()),
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H {
        cx,
        window,
        root: root.clone(),
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    h.wait("the project's debugger", |s| s["loaded"] == json!(true));
    h.invoke("project.open-file", json!({ "path": "main.py" }));
    (h, dir)
}

#[cfg(unix)]
fn alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// debugger-stop-at-breakpoint.md: the session's tab, the stop, the
/// execution point in the editor, the console, then the end.
#[gpui::test]
fn a_session_stops_on_a_line_and_runs_to_the_end(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let mut after = debug_answer("continue");
    after.extend(debug_end(0));
    let s = h.debug_main(Some(quick_program()), after);
    assert_eq!(s["visible"], json!(true));
    assert_eq!(s["registered"], json!(true));
    let tab = &s["tool_window"]["tabs"][0];
    assert_eq!(tab["title"], json!("main"));
    assert_eq!(tab["content"], json!("threads-variables"));
    assert_eq!(tab["frames"], json!(["<module>, main.py:2:1"]));
    assert_eq!(
        tab["toolbar"],
        json!({"stop": true, "resume": true, "pause": false, "step": true})
    );
    assert_eq!(s["execution_point"]["file"], json!("main.py"));
    // The editor shows the line.
    let editor = h.state()["editor"].clone();
    assert!(editor["path"].as_str().unwrap().ends_with("main.py"));
    assert_eq!(editor["caret"]["line"], json!(1));
    // The console starts with the program's command line.
    let console = h.console();
    assert!(console[0].contains("echo debugged"), "{console:?}");
    // The run widget: Debug is Rerun on green, Stop stops the session.
    let widget = h.run()["widget"].clone();
    assert_eq!(widget["debug_icon"], json!("rerun"));
    assert_eq!(widget["debug_tooltip"], json!("Debug 'main.py'"));
    assert_eq!(widget["run_visible"], json!(true));
    assert_eq!(widget["stop_visible"], json!(true));
    assert_eq!(widget["stop_tooltip"], json!("Stop 'main'"));

    h.invoke("debug.resume", json!(null));
    // The program's exit and the adapter's end come apart: both are
    // waited for.
    let s = h.wait("the end", |s| {
        s["tool_window"]["tabs"][0]["exit_code"] == json!(0)
            && s["tool_window"]["tabs"][0]["state"] == json!("ended")
    });
    let tab = &s["tool_window"]["tabs"][0];
    assert_eq!(tab["content"], json!("console"));
    assert_eq!(tab["toolbar"]["stop"], json!(false));
    assert_eq!(tab["toolbar"]["step"], json!(false));
    assert!(s["execution_point"].is_null());
    let console = h.console();
    assert!(
        console
            .iter()
            .any(|l| l.contains("Process finished with exit code 0")),
        "{console:?}"
    );
    let widget = h.run()["widget"].clone();
    assert_eq!(widget["debug_icon"], json!("debug"));
    assert_eq!(widget["stop_visible"], json!(false));
}

/// debugger-stop-rerun-pause.md, step 7: the shared Stop ends the program
/// and its debugger.
#[gpui::test]
fn stop_ends_the_program_and_the_debugger(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.debug_main(Some(long_program()), Vec::new());
    let pid = h.wait("the program", |s| {
        s["sessions"][0]["processes"]["debuggee"].is_number()
    })["sessions"][0]["processes"]["debuggee"]
        .as_u64()
        .unwrap();
    h.invoke("run.stop", json!(null));
    h.wait_state("ended");
    // The program was killed: no exit code, the exit line tells.
    h.wait("the exit line", |s| {
        s["tool_window"]["tabs"][0]["console"]["lines"]
            .as_array()
            .is_some_and(|l| {
                l.iter().any(|l| {
                    l["text"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("Process finished with exit code"))
                })
            })
    });
    #[cfg(unix)]
    assert!(!alive(pid), "the program {pid} is gone");
    #[cfg(not(unix))]
    let _ = pid;
    assert_eq!(h.run()["widget"]["stop_visible"], json!(false));
}

/// debugger-stop-rerun-pause.md, steps 5 and 8: Rerun debugs again in the
/// same tab; closing the tab stops the session.
#[gpui::test]
fn rerun_debugs_again_in_the_tab_and_closing_it_stops(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.debug_main(Some(long_program()), Vec::new());
    let first = h.tab()["session"].clone();
    let main = h.file("main.py");
    let mut script = debug_start(&h.root, Some(long_program()));
    script.extend(debug_stop(&main, 2, "breakpoint"));
    push_debug_script(script);
    h.invoke("debug.rerun", json!(null));
    let s = h.wait("the session again", |s| {
        s["tool_window"]["tabs"][0]["session"] != first
            && s["tool_window"]["tabs"][0]["state"] == json!("suspended")
    });
    assert_eq!(s["tool_window"]["tabs"].as_array().unwrap().len(), 1);
    let pid = h.wait("the program", |s| {
        s["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["state"] == json!("suspended") && x["processes"]["debuggee"].is_number())
    })["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["state"] == json!("suspended"))
        .unwrap()["processes"]["debuggee"]
        .as_u64()
        .unwrap();
    h.invoke("debug.close-tab", json!(null));
    let s = h.debug();
    assert!(s["tool_window"]["tabs"].as_array().unwrap().is_empty());
    assert_eq!(s["registered"], json!(false));
    assert_eq!(s["visible"], json!(false));
    h.wait("no session", |s| {
        s["sessions"]
            .as_array()
            .is_none_or(|l| l.iter().all(|x| x["state"] == json!("ended")))
    });
    #[cfg(unix)]
    {
        for _ in 0..400 {
            if !alive(pid) {
                break;
            }
            tick(h.cx);
        }
        assert!(!alive(pid), "the program {pid} is gone");
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// debugger-run-widget.md, step 5, and Run while debugging: "Process 'x'
/// Is Running", then Stop and Rerun.
#[gpui::test]
fn debug_or_run_again_asks_to_stop_the_session(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.debug_main(None, Vec::new());
    let first = h.tab()["session"].clone();
    h.invoke("debug.debug", json!(null));
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Process 'main' Is Running"));
    assert_eq!(
        dialog["message"],
        json!("'main' is not allowed to run in parallel.\nWould you like to stop the running one?")
    );
    // Cancel leaves it.
    h.choose("Cancel");
    assert_eq!(h.tab()["session"], first);
    assert_eq!(h.tab()["state"], json!("suspended"));
    // Stop and Rerun debugs again in the tab.
    let main = h.file("main.py");
    let mut script = debug_start(&h.root, None);
    script.extend(debug_stop(&main, 2, "breakpoint"));
    push_debug_script(script);
    h.invoke("debug.debug", json!(null));
    h.choose("Stop and Rerun");
    let s = h.wait("the session again", |s| {
        s["tool_window"]["tabs"][0]["session"] != first
            && s["tool_window"]["tabs"][0]["state"] == json!("suspended")
    });
    assert_eq!(s["tool_window"]["tabs"].as_array().unwrap().len(), 1);
    // Run while it is debugged: the session stops, then the file runs.
    support::set_script(&support::script("echo ran", "echo ran"));
    h.invoke("run.run", json!(null));
    assert_eq!(
        h.state()["dialog"]["title"],
        json!("Process 'main' Is Running")
    );
    h.choose("Stop and Rerun");
    h.wait_state("ended");
    h.wait_run("the run", |s| s["tabs"][0]["status"] == json!("terminated"));
}

/// debugger-run-widget.md, steps 2 and 3: a configuration being debugged
/// has Resume or Pause in Run's place, in the widget and its dropdown row.
#[gpui::test]
fn a_debugged_configuration_has_resume_and_pause(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let main = h.file("main.py");
    let mut script = debug_start(&h.root, None);
    script.extend(debug_stop(&main, 2, "breakpoint"));
    script.extend(debug_answer("continue"));
    script.extend([
        json!({"expect": "pause", "arguments": {"threadId": 1}}),
        json!({"respond": null}),
    ]);
    script.extend(debug_stop(&main, 3, "pause"));
    push_debug_script(script);
    // Debug Context Configuration: the file's configuration, selected.
    h.invoke("debug.current-file", json!(null));
    h.wait_state("suspended");
    let run = h.run();
    assert_eq!(run["configurations"]["selected"]["name"], json!("main"));
    let widget = &run["widget"];
    assert_eq!(widget["run_visible"], json!(false));
    assert_eq!(widget["resume"]["action"], json!("resume"));
    assert_eq!(widget["resume"]["tooltip"], json!("Resume 'main'"));
    assert_eq!(widget["debug_tooltip"], json!("Restart Debug 'main'"));
    assert_eq!(widget["debug_icon"], json!("rerun"));
    // The dropdown's row.
    h.invoke("run.configurations", json!(null));
    let menu = h.run()["menu"].clone();
    let row = menu["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == json!("main"))
        .cloned()
        .unwrap();
    let tooltips: Vec<&str> = row["inline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["tooltip"].as_str().unwrap())
        .collect();
    assert_eq!(
        tooltips,
        ["Resume 'main'", "Restart Debug 'main'", "Stop 'main'"]
    );
    h.invoke("menu.cancel", json!(null));
    // Resumed, it shows Pause; Pause suspends it again.
    h.invoke("debug.resume", json!({"index": 0}));
    h.wait_state("running");
    let widget = h.run()["widget"].clone();
    assert_eq!(widget["resume"]["action"], json!("pause"));
    assert_eq!(widget["resume"]["tooltip"], json!("Pause 'main'"));
    h.invoke("debug.pause", json!({"index": 0}));
    let s = h.wait("the pause", |s| s["execution_point"]["line"] == json!(3));
    assert_eq!(s["tool_window"]["tabs"][0]["state"], json!("suspended"));
    assert_eq!(h.run()["widget"]["resume"]["action"], json!("resume"));
}

/// debugger-run-widget.md, step 4: the Run/Debug Configurations dialog's
/// Debug debugs the selected configuration.
#[gpui::test]
fn the_configurations_dialog_debugs(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    // A configuration of the file, selected.
    let main = h.file("main.py");
    let mut script = debug_start(&h.root, None);
    script.extend(debug_end(0));
    push_debug_script(script);
    h.invoke("debug.current-file", json!(null));
    h.wait_state("ended");
    h.invoke("debug.close-tab", json!(null));
    h.invoke("run.configurations.edit", json!(null));
    let dialog = h.run()["edit_dialog"].clone();
    assert_eq!(dialog["debug_enabled"], json!(true));
    let mut script = debug_start(&h.root, None);
    script.extend(debug_stop(&main, 2, "breakpoint"));
    push_debug_script(script);
    let focus = h
        .window
        .read_with(h.cx, |w, cx| {
            w.run_window()
                .read(cx)
                .edit_dialog()
                .map(|d| d.read(cx).tree_focus())
        })
        .unwrap()
        .expect("the dialog is open");
    let dialog_window =
        h.cx.update(|cx| ui::dialog_window::modal_of(h.window.into(), cx))
            .expect("the dialog's window");
    h.cx.update_window(dialog_window, |_, window, cx| {
        window.focus(&focus);
        ui::command::invoke("run.configurations.dialog.debug", json!(null), window, cx)
    })
    .unwrap()
    .unwrap();
    h.draw();
    let s = h.wait_state("suspended");
    assert_eq!(s["tool_window"]["tabs"][0]["title"], json!("main"));
    assert!(h.run()["edit_dialog"].is_null(), "the dialog closed");
}

/// The adapter could not start: the status bar and a balloon say why.
#[gpui::test]
fn a_debugger_that_cannot_start_says_why(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.invoke("debug.debug", json!(null));
    for _ in 0..50 {
        tick(h.cx);
    }
    let state = h.state();
    assert_eq!(
        state["message"],
        json!("Error running 'main': no debug adapter in tests")
    );
    assert!(
        h.debug()["tool_window"]["tabs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// debugger-stop-rerun-pause.md, step 2: a line typed in the Console
/// reaches the program.
#[cfg(unix)]
#[gpui::test]
fn the_console_sends_input_to_the_program(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let program = ["/bin/sh", "-c", "read x; echo got:$x"]
        .map(str::to_owned)
        .to_vec();
    push_debug_script(debug_start(&h.root, Some(program)));
    h.invoke("debug.debug", json!(null));
    h.wait("the program", |s| {
        s["sessions"][0]["processes"]["debuggee"].is_number()
    });
    h.invoke("debug.select-content", json!({"content": "console"}));
    let focus = h
        .window
        .read_with(h.cx, |w, cx| {
            w.debugger_view()
                .read(cx)
                .tool_window()
                .read(cx)
                .content_focus(cx)
        })
        .unwrap();
    h.cx.update_window(h.window.into(), |_, window, _| window.focus(&focus))
        .unwrap();
    h.invoke("debug.console.type", json!({"text": "hi\n"}));
    h.wait("the answer", |s| {
        s["tool_window"]["tabs"][0]["console"]["lines"]
            .as_array()
            .is_some_and(|l| l.iter().any(|l| l["text"] == json!("got:hi")))
    });
}

/// A scripted session for the watches: a stop on line 2 of main.py with
/// the locals `a = 1` and `b = 2`, the watches `a` and `b` evaluated, Step
/// Over to line 3 (`b = 3`), where only `b` is evaluated (`a` is paused),
/// then `a` once more ("… Evaluate") and the evaluate field's `a`.
fn watches_script(root: &std::path::Path) -> Vec<Value> {
    let main = root.join("main.py");
    let frame = |id: i64, line: u32| json!({"id": id, "name": "f", "line": line, "column": 1, "source": {"path": main}});
    let locals = |frame: i64, reference: i64, b: &str| {
        vec![
            json!({"expect": "scopes", "arguments": {"frameId": frame}}),
            json!({"respond": {"scopes": [{"name": "Locals", "variablesReference": reference,
                "expensive": false}]}}),
            json!({"expect": "variables", "arguments": {"variablesReference": reference}}),
            json!({"respond": {"variables": [
                {"name": "a", "value": "1", "type": "int", "variablesReference": 0,
                    "evaluateName": "a"},
                {"name": "b", "value": b, "type": "int", "variablesReference": 0,
                    "evaluateName": "b"}]}}),
        ]
    };
    let answer = |expression: &str, value: &str| {
        vec![
            json!({"expect": "evaluate", "arguments": {"expression": expression}}),
            json!({"respond": {"result": value, "type": "int", "variablesReference": 0}}),
        ]
    };
    let stop = |line: u32| {
        vec![
            json!({"event": "stopped", "body": {"reason": "step", "threadId": 1}}),
            json!({"expect": "threads"}),
            json!({"respond": {"threads": [{"id": 1, "name": "MainThread"}]}}),
            json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
            json!({"respond": {"stackFrames": [frame(100 + i64::from(line), line)],
                "totalFrames": 1}}),
        ]
    };
    let mut script = vec![
        json!({"ignore": ["setBreakpoints", "setExceptionBreakpoints", "source"]}),
        json!({"expect": "initialize"}),
        json!({"respond": {"supportsConfigurationDoneRequest": true,
            "supportsTerminateRequest": true, "supportsSetVariable": true}}),
        json!({"expect": "launch"}),
        json!({"event": "initialized"}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "launch"}),
    ];
    script.extend(stop(2));
    script.extend(locals(102, 5, "2"));
    script.extend(answer("a", "1"));
    script.extend(answer("b", "2"));
    script.extend(debug_answer("next"));
    script.extend(stop(3));
    script.extend(locals(103, 7, "3"));
    script.extend(answer("b", "3"));
    script.extend(answer("a", "1"));
    script.extend(answer("a", "1"));
    script
}

impl H<'_> {
    fn variables(&mut self) -> Value {
        self.tab()["variables"].clone()
    }

    fn watch_row(&mut self, name: &str) -> Value {
        self.variables()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == json!("watch") && r["name"] == json!(name))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    /// The labels of the open context menu, `-` for a line, disabled ones
    /// in brackets, with the shortcut after a tab.
    fn menu(&mut self) -> Vec<String> {
        let menu = self.variables()["menu"].clone();
        menu["levels"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                if i["separator"] == json!(true) {
                    return "-".to_owned();
                }
                let label = i["label"].as_str().unwrap().to_owned();
                let label = if i["enabled"] == json!(true) {
                    label
                } else {
                    format!("({label})")
                };
                match i["shortcut"].as_str() {
                    Some(s) => format!("{label}\t{s}"),
                    None => label,
                }
            })
            .collect()
    }
}

/// debugger-evaluate-watches.md: Pause Watch keeps a watch's value and
/// leaves it out at the next stop ("… Evaluate" evaluates it once), View as
/// renders a value, Remove All Watches; the evaluate field's history list
/// and its keys; the context menus of a value and of a watch.
#[gpui::test]
fn watches_pause_view_as_history_and_menus(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    push_debug_script(watches_script(&h.root));
    h.invoke(
        "debug.toggle-breakpoint",
        json!({"file": "main.py", "line": 2}),
    );
    h.invoke("debug.debug", json!(null));
    h.wait("the values of the stop", |s| {
        s["tool_window"]["tabs"][0]["variables"]["rows"]
            .as_array()
            .is_some_and(|r| r.iter().any(|r| r["name"] == json!("b")))
    });
    h.invoke("debug.new-watch", json!({"expression": "a"}));
    h.invoke("debug.new-watch", json!({"expression": "b"}));
    h.wait("the watches' values", |s| {
        let rows = &s["tool_window"]["tabs"][0]["variables"]["rows"];
        rows[0]["state"] == json!("value") && rows[1]["state"] == json!("value")
    });
    // The menus: a watch's and a value's, as the reference shows them.
    h.invoke("debug.variables.select", json!({"path": ["a"]}));
    h.invoke("debug.variables.context-menu", json!(null));
    let watch_menu = h.menu();
    let expected = [
        "New Watch…",
        "Remove Watch",
        "Remove All Watches",
        "Edit…",
        "Pause Watch",
        "-",
        "(Inspect…)",
        "Copy Value",
        "(Compare Value with Clipboard)",
        "Copy Name",
        "-",
        "Evaluate Expression…",
        "Add to Watches",
        "-",
        "Jump To Source",
        "Jump To Type Source",
        "-",
        "View as",
    ];
    let labels: Vec<&str> = watch_menu
        .iter()
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(labels, expected, "{watch_menu:#?}");
    h.invoke("menu.cancel", json!(null));
    h.invoke("debug.variables.select", json!({"path": ["Locals", "b"]}));
    h.invoke("debug.variables.context-menu", json!(null));
    let value_menu = h.menu();
    let labels: Vec<&str> = value_menu
        .iter()
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "New Watch…",
            "Remove All Watches",
            "-",
            "(Inspect…)",
            "Set Value…",
            "Copy Value",
            "(Compare Value with Clipboard)",
            "Copy Name",
            "-",
            "Evaluate Expression…",
            "Add to Watches",
            "-",
            "Jump To Source",
            "Jump To Type Source",
            "-",
            "View as",
        ],
        "{value_menu:#?}"
    );
    h.invoke("menu.cancel", json!(null));
    // View as: the value in hex, the renderer remembered for the row.
    h.invoke("debug.variables.view-as", json!({"renderer": "Hex"}));
    let b = h.variables()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == json!(["Locals", "b"]))
        .cloned()
        .unwrap();
    assert_eq!(b["text"], json!("b = {int} 0x2"), "{b:#}");
    assert_eq!(b["renderer"], json!("Hex"), "{b:#}");
    // Pause Watch: the value stays, the watch is paused (and stays so).
    h.invoke("debug.pause-watch", json!({"index": 0}));
    let a = h.watch_row("a");
    assert_eq!(a["paused"], json!(true), "{a:#}");
    assert_eq!(a["text"], json!("a = {int} 1"), "{a:#}");
    assert_eq!(h.variables()["paused"], json!([true, false]));
    h.invoke("debug.variables.select", json!({"path": ["a"]}));
    h.invoke("debug.variables.context-menu", json!(null));
    assert!(
        h.menu().iter().any(|l| l == "Resume Watch"),
        "{:#?}",
        h.menu()
    );
    h.invoke("menu.cancel", json!(null));
    // The next stop: only `b` is evaluated.
    h.invoke("debug.step-over", json!(null));
    h.wait("the next stop's watches", |s| {
        let rows = &s["tool_window"]["tabs"][0]["variables"]["rows"];
        rows[1]["text"] == json!("b = {int} 3")
    });
    let a = h.watch_row("a");
    assert_eq!(a["state"], json!("paused"), "{a:#}");
    assert_eq!(a["text"], json!("a The watch is paused"), "{a:#}");
    // "… Evaluate": once, the watch stays paused.
    h.invoke("debug.evaluate-paused-watch", json!({"index": 0}));
    h.wait("the paused watch evaluated", |s| {
        s["tool_window"]["tabs"][0]["variables"]["rows"][0]["state"] == json!("value")
    });
    assert_eq!(h.watch_row("a")["paused"], json!(true));
    // The evaluate field: the result, then the history list and its keys.
    h.invoke("debug.variables.evaluate", json!({"expression": "a"}));
    h.wait("the result", |s| {
        s["tool_window"]["tabs"][0]["variables"]["rows"][0]["state"] == json!("value")
            && s["tool_window"]["tabs"][0]["variables"]["rows"][0]["kind"] == json!("result")
    });
    h.invoke("debug.variables.set-expression", json!({"text": ""}));
    h.invoke("debug.variables.history", json!(null));
    let history = h.variables()["history"].clone();
    assert_eq!(history["of"], json!("field"), "{history:#}");
    assert_eq!(history["items"], json!(["a"]), "{history:#}");
    assert_eq!(history["highlighted"], json!(null), "{history:#}");
    h.press("down");
    assert_eq!(h.variables()["history"]["highlighted"], json!(0));
    h.press("enter");
    let v = h.variables();
    assert_eq!(v["history"], json!(null), "{v:#}");
    assert_eq!(v["evaluate"]["text"], json!("a"), "{v:#}");
    h.invoke("debug.variables.history", json!(null));
    h.press("escape");
    assert_eq!(h.variables()["history"], json!(null));
    // Remove All Watches: the watches and the result go.
    h.invoke("debug.remove-all-watches", json!(null));
    let v = h.variables();
    assert_eq!(v["watches"], json!([]), "{v:#}");
    assert!(
        !v["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == json!("watch") || r["kind"] == json!("result")),
        "{v:#}"
    );
    h.invoke("debug.stop", json!(null));
}
