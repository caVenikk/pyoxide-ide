//! Closing the main window ends what it started (#401): the running
//! processes are asked about one at a time ("Process 'x' Is Running":
//! Terminate, Disconnect or Cancel), the window's dialogs close with it,
//! and Close Project does the same. Real processes (shell scripts) for
//! runs, scripted debug adapters for debug sessions (see tests/support).

use crate::support;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AnyWindowHandle, AppContext as _, TestAppContext, VisualTestContext, WindowHandle};
use serde_json::{Value, json};
use ui::icons::IconName;
use workbench::{FileType, Product, Workbench};

use crate::support::{debug_start, debug_stop, long_program, push_debug_script, tick};

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

    fn debug(&mut self) -> Value {
        self.invoke("debug.state", json!(null))
    }

    fn run(&mut self) -> Value {
        self.invoke("run.state", json!(null))
    }

    /// Waits (at most about 20 s) until `f` holds.
    fn wait_for(&mut self, what: &str, mut f: impl FnMut(&mut Self) -> bool) {
        for _ in 0..2000 {
            tick(self.cx);
            if f(self) {
                self.draw();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}");
    }

    fn is_open(&mut self) -> bool {
        let main: AnyWindowHandle = self.window.into();
        self.cx.update(|cx| cx.windows().contains(&main))
    }

    /// The window's close button; a window that may close at once goes,
    /// as the platform's would.
    fn close(&mut self) -> bool {
        let main: AnyWindowHandle = self.window.into();
        let close = VisualTestContext::from_window(main, self.cx).simulate_close();
        if close {
            self.cx
                .update_window(main, |_, window, _| window.remove_window())
                .unwrap();
        }
        self.draw();
        close
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

    /// Runs other.py with a script that runs until it is stopped; its
    /// process id.
    fn run_long(&mut self) -> u64 {
        self.invoke("project.open-file", json!({ "path": "other.py" }));
        support::set_script(&support::long_running());
        self.invoke("run.run", json!(null));
        let mut pid = None;
        self.wait_for("the run", |h| {
            pid = h.run()["tabs"][0]["pid"].as_u64();
            pid.is_some()
        });
        pid.unwrap()
    }

    /// Debugs main.py with an adapter that starts a long program and
    /// stops on line 2; the program's process id.
    fn debug_long(&mut self) -> u64 {
        self.invoke("project.open-file", json!({ "path": "main.py" }));
        let main = self.root.join("main.py");
        let mut script = debug_start(&self.root, Some(long_program()));
        script.extend(debug_stop(&main, 2, "breakpoint"));
        push_debug_script(script);
        self.invoke("debug.debug", json!(null));
        let mut pid = None;
        self.wait_for("the program", |h| {
            pid = h.debug()["sessions"][0]["processes"]["debuggee"].as_u64();
            pid.is_some()
        });
        pid.unwrap()
    }
}

fn setup(cx: &mut TestAppContext) -> (H<'_>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("main.py"), MAIN).unwrap();
    std::fs::write(root.join("other.py"), "x = 1\n").unwrap();
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain("Python", &["py"], IconName::FILE_TEXT)],
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
    h.wait_for("the project's debugger", |h| {
        h.debug()["loaded"] == json!(true)
    });
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

/// Nothing runs: the window closes at once, its Breakpoints window with
/// it.
#[gpui::test]
fn the_window_closes_with_its_dialogs(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    h.invoke("debug.view-breakpoints", json!(null));
    assert_eq!(h.cx.update(|cx| cx.windows()).len(), 2);
    assert!(h.close());
    assert!(
        h.cx.update(|cx| cx.windows()).is_empty(),
        "the Breakpoints window closed with the main window"
    );
}

/// A run and a debug session: each is asked about; Cancel keeps the
/// window and the processes; Terminate ends both, then the window closes
/// with its Breakpoints window.
#[gpui::test]
fn closing_asks_about_each_running_process(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let run = h.run_long();
    let program = h.debug_long();
    h.invoke("debug.view-breakpoints", json!(null));
    assert!(!h.close(), "the window waits for the answers");
    let dialog = h.state()["dialog"].clone();
    assert_eq!(dialog["title"], json!("Process 'other' Is Running"));
    assert_eq!(
        dialog["message"],
        json!("Do you want to terminate the process 'other'?")
    );
    assert_eq!(
        dialog["buttons"],
        json!(["Cancel", "Disconnect", "Terminate"])
    );
    assert_eq!(dialog["option"]["label"], json!("Don't ask again"));
    // Cancel: nothing ends.
    h.choose("Cancel");
    assert!(h.state()["dialog"].is_null());
    assert!(h.is_open());
    assert_eq!(h.run()["tabs"][0]["status"], json!("running"));
    // Asked again; Terminate, then the debug session.
    assert!(!h.close());
    h.choose("Terminate");
    assert_eq!(
        h.state()["dialog"]["title"],
        json!("Process 'main' Is Running")
    );
    h.choose("Terminate");
    h.wait_for("the window to close", |h| !h.is_open());
    assert!(
        h.cx.update(|cx| cx.windows()).is_empty(),
        "the Breakpoints window closed too"
    );
    #[cfg(unix)]
    for pid in [run, program] {
        for _ in 0..400 {
            if !alive(pid) {
                break;
            }
            tick(h.cx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!alive(pid), "{pid} is gone");
    }
    #[cfg(not(unix))]
    let _ = (run, program);
}

/// Disconnect leaves the run's process running; "Don't ask again" answers
/// the next one the same way. Close Project closes the window as well.
#[gpui::test]
fn disconnect_leaves_the_process_and_is_remembered(cx: &mut TestAppContext) {
    let (mut h, _dir) = setup(cx);
    let run = h.run_long();
    h.debug_long();
    h.invoke("project.close", json!(null));
    assert_eq!(
        h.state()["dialog"]["title"],
        json!("Process 'other' Is Running")
    );
    h.invoke("dialog.toggle-option", json!(null));
    h.choose("Disconnect");
    // The debug session is not asked about: disconnected too.
    h.wait_for("the window to close", |h| !h.is_open());
    #[cfg(unix)]
    {
        assert!(alive(run), "the run's process goes on");
        std::process::Command::new("kill")
            .args(["-9", &run.to_string()])
            .status()
            .unwrap();
    }
    #[cfg(not(unix))]
    let _ = run;
}
