//! Debug sessions of the Pyoxide window with the real debugpy helper and a
//! real Python (tests/scenarios/run-debug/debugger-*.md). Ignored by
//! default: they need Python 3.8 or later and the helper.
//!
//! ```sh
//! python3 scripts/fetch-debugpy.py          # once: the helper into the app cache
//! cargo test -p pyoxide-ide --test debug_sessions -- --ignored
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
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("main.py"), MAIN).unwrap();
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
        code_intel: None,
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
    let root = std::fs::canonicalize(dir.path()).unwrap();
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
