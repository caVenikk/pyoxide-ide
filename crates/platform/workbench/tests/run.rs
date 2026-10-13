//! Running the current file (tests/scenarios/run-debug/): the run widget,
//! the Run tool window, its console, Stop and Rerun, the dialogs, driven by
//! the default keymap's keystrokes with real processes (shell scripts in
//! place of Python, see tests/support).

mod support;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Focusable as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    TestAppContext, VisualTestContext, WindowHandle,
};
use serde_json::{Value, json};
use workbench::keymap::{self, Keymap};
use workbench::workbench::ToolWindow;
use workbench::{FileType, Product, Workbench};

use support::{long_running, script, set_script, tick};

const MAC: bool = cfg!(target_os = "macos");

/// Keystrokes of the platform's default keymap.
fn keys(mac: &'static str, other: &'static str) -> &'static str {
    if MAC { mac } else { other }
}

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

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    /// The run state (a query: no frame is drawn).
    fn run(&mut self) -> Value {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke("run.state", json!(null), window, cx)
            })
            .unwrap()
            .unwrap()
    }

    fn state(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))
    }

    /// Waits (at most about 20 s) until `f` holds for the run state.
    fn wait(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            tick(self.cx);
            last = self.run();
            if f(&last) {
                self.draw();
                return last;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    fn wait_status(&mut self, tab: usize, status: &str) -> Value {
        self.wait(&format!("tab {tab} {status}"), |s| {
            s["tabs"][tab]["status"] == json!(status)
        })
    }

    /// Waits until the console of `tab` shows `text`.
    fn wait_text(&mut self, tab: usize, text: &str) -> Value {
        self.wait(&format!("{text:?} in tab {tab}"), |s| {
            console_text(s, tab).contains(text)
        })
    }

    fn open(&mut self, file: &str) {
        self.invoke("project.open-file", json!({ "path": file }));
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self
            .window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor");
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
    }

    fn focused_tool_window(&mut self) -> Option<ToolWindow> {
        self.cx
            .update_window(self.window.into(), |root, window, cx| {
                let w = root.downcast::<Workbench>().unwrap();
                w.read(cx).focused_tool_window(window, cx)
            })
            .unwrap()
    }

    fn focus_console(&mut self) {
        let run = self
            .window
            .read_with(self.cx, |w, _| w.run_window().clone())
            .unwrap();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                let focus = run.read(cx).content_focus(cx);
                window.focus(&focus);
            })
            .unwrap();
        self.draw();
    }

    fn active_path(&mut self) -> Option<String> {
        self.state()["editor"]["path"].as_str().map(str::to_owned)
    }

    /// Stops every run and closes the tabs.
    fn stop_all(&mut self) {
        self.invoke("run.stop-all", json!(null));
        self.invoke("run.stop-all", json!(null));
        self.wait("no process alive", |s| s["alive"] == json!(false));
    }
}

fn console_lines(state: &Value, tab: usize) -> Vec<String> {
    state["tabs"][tab]["console"]["lines"]
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .map(|l| l["text"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn console_text(state: &Value, tab: usize) -> String {
    console_lines(state, tab).join("\n")
}

/// The kinds of the runs of a console line.
fn line_kinds(state: &Value, tab: usize, line: usize) -> Vec<String> {
    state["tabs"][tab]["console"]["lines"][line]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap().to_owned())
        .collect()
}

fn setup(cx: &mut TestAppContext) -> (H<'_>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for (path, text) in [("a.py", "a = 1\n"), ("b.py", "b = 2\n"), ("c.txt", "c\n")] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        run: Some(support::run_hook()),
        ..Product::default()
    };
    let text = if MAC { keymap::MACOS } else { keymap::DEFAULT };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &Keymap::parse(text).unwrap());
        assert!(errors.is_empty(), "{errors:?}");
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
    (h, dir)
}

/// Run (⌃R / Shift+F10) runs the file in the editor: the Run tool window
/// shows without the focus; a tab named after the file without its
/// extension; the command line, the output (standard error in the error
/// colour), an empty line and the exit code.
#[gpui::test]
fn run_shows_output_and_the_exit_code(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    let widget = h.run()["widget"].clone();
    assert_eq!(widget["run_enabled"], json!(true));
    assert_eq!(widget["run_tooltip"], json!("Run 'a.py'"));
    assert_eq!(widget["stop_visible"], json!(false));
    set_script(&script(
        "echo hello; echo oops >&2; exit 3",
        "echo hello& echo oops 1>&2& exit 3",
    ));
    h.press(keys("ctrl-r", "shift-f10"));
    let s = h.wait_status(0, "terminated");
    assert_eq!(s["tabs"][0]["title"], json!("a"));
    assert_eq!(s["tabs"][0]["exit_code"], json!(3));
    assert_eq!(s["visible"], json!(true));
    assert!(h.editor_focused(), "the editor keeps the focus");
    let lines = console_lines(&s, 0);
    assert!(lines[0].ends_with(' '), "the command line: {lines:?}");
    assert_eq!(line_kinds(&s, 0, 0), ["system"]);
    // Standard output and standard error are separate pipes: their lines
    // may arrive in either order.
    let hello = lines.iter().position(|l| l.trim_end() == "hello").unwrap();
    assert_eq!(line_kinds(&s, 0, hello), ["normal"]);
    let oops = lines.iter().position(|l| l.trim_end() == "oops").unwrap();
    assert_eq!(line_kinds(&s, 0, oops), ["error"]);
    let n = lines.len();
    assert_eq!(lines[n - 2], "Process finished with exit code 3");
    assert_eq!(lines[n - 3], "", "an empty line before the exit line");
    assert_eq!(line_kinds(&s, 0, n - 2), ["system"]);
    // The Run tool window exists now: its stripe button shows.
    assert_eq!(s["registered"], json!(true));
}

/// The run widget: disabled without an editor and for files that cannot
/// run, with the reference's tooltips; while the file runs, Run shows
/// Rerun and Stop appears; with two processes Stop offers both.
#[gpui::test]
fn the_run_widget_follows_the_editor_and_the_processes(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let w = h.run()["widget"].clone();
    assert_eq!(w["run_enabled"], json!(false));
    assert_eq!(
        w["run_tooltip"],
        json!("Cannot run the current file: the editor is not in focus")
    );
    h.open("c.txt");
    let w = h.run()["widget"].clone();
    assert_eq!(w["run_enabled"], json!(false));
    assert_eq!(
        w["run_tooltip"],
        json!("The file in the editor is not runnable")
    );
    // Run does nothing for it.
    h.press(keys("ctrl-r", "shift-f10"));
    assert!(h.run()["tabs"].as_array().unwrap().is_empty());

    set_script(&long_running());
    h.open("a.py");
    h.invoke("run.run", json!(null));
    h.wait_text(0, "started");
    let w = h.run()["widget"].clone();
    assert_eq!(w["run_icon"], json!("rerun"));
    assert_eq!(w["stop_visible"], json!(true));
    assert_eq!(w["stop_tooltip"], json!("Stop 'a'"));
    assert!(h.run()["tabs"][0]["pid"].is_number());
    h.open("b.py");
    assert_eq!(h.run()["widget"]["run_icon"], json!("run"));
    h.invoke("run.run", json!(null));
    h.wait_text(1, "started");
    let w = h.run()["widget"].clone();
    assert_eq!(w["stop_count"], json!(2));
    assert_eq!(w["stop_tooltip"], json!("Stop..."));
    // Stop with two processes offers them.
    h.invoke("run.stop", json!(null));
    let items: Vec<Value> = h.run()["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].clone())
        .collect();
    assert_eq!(
        items,
        [
            json!("Stop \u{2018}a\u{2019}"),
            json!("Stop \u{2018}b\u{2019}"),
            Value::Null,
            json!("Stop All 2")
        ]
    );
    h.invoke("menu.cancel", json!(null));
    h.stop_all();
}

/// Stop (⌘F2 / Ctrl+F2) interrupts first; the button turns into Kill; a
/// second Stop kills the process.
#[cfg(unix)]
#[gpui::test]
fn stop_interrupts_then_kills(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    // The shell ignores the interrupt.
    set_script("trap '' INT; echo started; while :; do sleep 0.05; done");
    h.invoke("run.run", json!(null));
    h.wait_text(0, "started");
    h.press(keys("cmd-f2", "ctrl-f2"));
    let s = h.wait("stopping", |s| s["tabs"][0]["status"] == json!("stopping"));
    assert_eq!(s["tabs"][0]["status"], json!("stopping"));
    h.press(keys("cmd-f2", "ctrl-f2"));
    let s = h.wait_status(0, "terminated");
    let text = s["tabs"][0]["exit_text"].as_str().unwrap().to_owned();
    assert_eq!(text, "137 (interrupted by signal 9:SIGKILL)");
    let lines = console_lines(&s, 0);
    assert_eq!(
        lines[lines.len() - 2],
        "Process finished with exit code 137 (interrupted by signal 9:SIGKILL)"
    );
}

/// Without an interrupt helper (the tests set none; the application sets
/// itself) Stop cannot send Ctrl+C on Windows and kills at once, as the
/// reference falls back to a hard destroy.
#[cfg(windows)]
#[gpui::test]
fn stop_without_an_interrupt_helper_kills(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(&long_running());
    h.invoke("run.run", json!(null));
    h.wait_text(0, "started");
    h.press(keys("cmd-f2", "ctrl-f2"));
    let s = h.wait_status(0, "terminated");
    assert_eq!(s["tabs"][0]["exit_text"], json!("-1"));
    let lines = console_lines(&s, 0);
    assert_eq!(lines[lines.len() - 2], "Process finished with exit code -1");
}

/// Rerun (⌘R / Ctrl+F5) in the Run tool window stops the process and runs
/// it again in the same tab without asking; in the editor ⌘R stays Replace.
#[gpui::test]
fn rerun_runs_again_in_the_same_tab(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(&long_running());
    h.invoke("run.run", json!(null));
    let first = h.wait_text(0, "started")["tabs"][0]["pid"].clone();
    h.focus_console();
    h.press(keys("cmd-r", "ctrl-f5"));
    let s = h.wait("a new process", |s| {
        s["tabs"][0]["pid"].is_number() && s["tabs"][0]["pid"] != first
    });
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1);
    assert!(s["dialog"].is_null());
    assert!(console_lines(&s, 0)[0].ends_with(' '), "a fresh console");
    h.stop_all();
}

/// Run while the same file runs asks "Process 'a' Is Running"; Stop and
/// Rerun (Enter) runs it again in its tab, Escape cancels.
#[gpui::test]
fn running_a_running_file_asks_first(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(&long_running());
    h.invoke("run.run", json!(null));
    let first = h.wait_text(0, "started")["tabs"][0]["pid"].clone();
    h.press(keys("ctrl-r", "shift-f10"));
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Process 'a' Is Running"));
    assert_eq!(
        dialog["message"],
        json!("'a' is not allowed to run in parallel.\nWould you like to stop the running one?")
    );
    assert_eq!(dialog["buttons"], json!(["Cancel", "Stop and Rerun"]));
    assert_eq!(dialog["default"], json!("Stop and Rerun"));
    assert_eq!(
        dialog["option"]["label"],
        json!("Do not show this dialog in the future")
    );
    h.press("escape");
    assert!(h.state()["dialog"].is_null());
    assert_eq!(h.run()["tabs"][0]["pid"], first);
    assert!(h.editor_focused(), "the focus is back");
    h.press(keys("ctrl-r", "shift-f10"));
    h.press("enter");
    let s = h.wait("a new process", |s| {
        s["tabs"][0]["pid"].is_number() && s["tabs"][0]["pid"] != first
    });
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1);
    h.stop_all();
}

/// Typed text is echoed as input and sent on Enter; Send EOF (⌘D /
/// Ctrl+D) closes the standard input.
#[gpui::test]
fn typing_in_the_console_feeds_standard_input(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(&script(
        "printf 'x? '; read x; echo \"got $x\"; cat; echo done",
        "set /p x=x? & echo got !x!& more & echo done",
    ));
    h.invoke("run.run", json!(null));
    h.wait_text(0, "x?");
    h.focus_console();
    h.cx.simulate_input(h.window.into(), "42");
    h.draw();
    let s = h.run();
    assert_eq!(s["tabs"][0]["console"]["pending_input"], json!("42"));
    h.press("enter");
    let s = h.wait_text(0, "got 42");
    let lines = console_lines(&s, 0);
    let prompt = lines.iter().position(|l| l.starts_with("x? ")).unwrap();
    assert_eq!(lines[prompt], "x? 42");
    assert_eq!(line_kinds(&s, 0, prompt), ["normal", "input"]);
    h.press(keys("cmd-d", "ctrl-d"));
    let s = h.wait_status(0, "terminated");
    assert!(console_text(&s, 0).contains("done"));
    assert_eq!(s["tabs"][0]["exit_code"], json!(0));
}

/// A console link opens its file at its line on a click; Up and Down the
/// Stack Trace go through the links.
#[gpui::test]
fn console_links_open_files(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let b = h.root.join("b.py").display().to_string();
    let c = h.root.join("a.py").display().to_string();
    h.open("a.py");
    set_script(&script(
        &format!("echo 'one at {b}:1'; echo 'two at {c}:1'"),
        &format!("echo one at {b}:1& echo two at {c}:1"),
    ));
    h.invoke("run.run", json!(null));
    let s = h.wait_status(0, "terminated");
    let links = &s["tabs"][0]["console"]["lines"][1]["links"];
    assert_eq!(links[0]["target"]["file"], json!(b));
    // A click on the link opens b.py.
    let at = {
        let run = h
            .window
            .read_with(h.cx, |w, _| w.run_window().clone())
            .unwrap();
        let editor =
            h.cx.update(|cx| run.read(cx).tabs()[0].console().read(cx).editor().clone());
        let (origin, lh, cw) = editor.read_with(h.cx, |e, _| e.text_geometry().unwrap());
        gpui::point(
            origin.x + cw * 10.0 + gpui::px(1.0),
            origin.y + lh * 1.0 + lh / 2.0,
        )
    };
    let mut vcx = VisualTestContext::from_window(h.window.into(), h.cx);
    vcx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    vcx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    h.draw();
    assert!(h.active_path().unwrap().ends_with("b.py"));
    assert!(h.editor_focused());
    // Down the Stack Trace goes on to the next link.
    h.focus_console();
    h.press(keys("cmd-alt-down", "ctrl-alt-down"));
    assert!(h.active_path().unwrap().ends_with("a.py"));
    h.focus_console();
    h.press(keys("cmd-alt-up", "ctrl-alt-up"));
    assert!(h.active_path().unwrap().ends_with("b.py"));
}

/// A new run reuses a terminated, unpinned tab (the one with the same
/// name first); a running or pinned tab is kept.
#[gpui::test]
fn tabs_are_reused_unless_running_or_pinned(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(&script("echo done", "echo done"));
    h.invoke("run.run", json!(null));
    h.wait_status(0, "terminated");
    h.open("b.py");
    h.invoke("run.run", json!(null));
    let s = h.wait("b in the reused tab", |s| {
        s["tabs"][0]["title"] == json!("b") && s["tabs"][0]["status"] == json!("terminated")
    });
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1);
    // Pinned: the next run opens a new tab.
    h.invoke("run.toggle-pin", json!(null));
    h.open("a.py");
    h.invoke("run.run", json!(null));
    let s = h.wait("a new tab", |s| {
        s["tabs"].as_array().unwrap().len() == 2 && s["tabs"][1]["status"] == json!("terminated")
    });
    assert_eq!(s["tabs"][0]["title"], json!("b"));
    assert_eq!(s["tabs"][0]["pinned"], json!(true));
    assert_eq!(s["tabs"][1]["title"], json!("a"));
    // Running: not reused either.
    set_script(&long_running());
    h.invoke("run.run", json!(null));
    h.wait_text(1, "started");
    h.open("b.py");
    set_script(&script("echo done", "echo done"));
    h.invoke("run.run", json!(null));
    let s = h.wait("a third tab", |s| s["tabs"].as_array().unwrap().len() == 3);
    assert_eq!(s["tabs"][2]["title"], json!("b"));
    h.stop_all();
}

/// Closing a tab whose process runs (⌘W / Ctrl+F4 in the Run tool window)
/// asks; Terminate stops the process and closes the tab; the last tab
/// closed hides the tool window and removes its stripe button.
#[gpui::test]
fn closing_a_running_tab_asks_to_terminate(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(&long_running());
    h.invoke("run.run", json!(null));
    h.wait_text(0, "started");
    h.focus_console();
    h.press(keys("cmd-w", "ctrl-f4"));
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Process 'a' Is Running"));
    assert_eq!(
        dialog["message"],
        json!("Do you want to terminate the process 'a'?")
    );
    assert_eq!(
        dialog["buttons"],
        json!(["Cancel", "Disconnect", "Terminate"])
    );
    assert_eq!(dialog["option"]["label"], json!("Don't ask again"));
    assert_eq!(dialog["default"], json!("Terminate"));
    h.press("enter");
    let s = h.run();
    assert!(s["tabs"].as_array().unwrap().is_empty());
    assert_eq!(s["visible"], json!(false));
    assert_eq!(s["registered"], json!(false));
    assert!(h.state()["dialog"].is_null());
}

/// ⌘4 / Alt+4 shows the Run tool window (the empty state before any run)
/// and focuses it; Escape goes back to the editor; Shift+Escape hides it.
#[gpui::test]
fn the_run_tool_window_by_keys(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    h.press(keys("cmd-4", "alt-4"));
    assert_eq!(h.run()["visible"], json!(true));
    assert_eq!(h.focused_tool_window(), Some(ToolWindow::Run));
    set_script(&script("echo done", "echo done"));
    h.invoke("run.run", json!(null));
    h.wait_status(0, "terminated");
    h.focus_console();
    h.press("escape");
    assert!(h.editor_focused());
    h.focus_console();
    h.press("shift-escape");
    assert_eq!(h.run()["visible"], json!(false));
    assert!(h.editor_focused());
}

/// Output beyond the console's cycle buffer (1024 KB) drops the oldest
/// lines; Clear All empties the console.
#[cfg(unix)]
#[gpui::test]
fn a_long_output_keeps_the_last_megabyte(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.open("a.py");
    set_script(
        "i=0; while [ $i -lt 14000 ]; do echo \"line $i ................................................................................\"; i=$((i+1)); done",
    );
    h.invoke("run.run", json!(null));
    let s = h.wait_status(0, "terminated");
    let lines = console_lines(&s, 0);
    assert!(!lines[0].ends_with(' '), "the command line was dropped");
    assert!(console_text(&s, 0).contains("line 13999"));
    let size: usize = lines.iter().map(|l| l.len() + 1).sum();
    assert!(size <= 1024 * 1024 + 200, "{size}");
    h.invoke("run.console.clear", json!(null));
    assert_eq!(console_text(&h.run(), 0), "");
}
