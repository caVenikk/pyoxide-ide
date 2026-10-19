//! Debug sessions with the real debugpy helper and a real Python. Ignored
//! by default: they need Python 3.8 or later and the helper.
//!
//! ```sh
//! python3 scripts/fetch-debugpy.py          # once: the helper into the app cache
//! cargo test -p py-debug --test it debugpy:: -- --ignored
//! ```
//!
//! - `PYOXIDE_TEST_PYTHON`: the interpreter (default `python3`, `python` on
//!   Windows); an interpreter of a virtual environment also runs a
//!   configuration with `uv run` on, which is debugged with that
//!   interpreter directly;
//! - `PYOXIDE_CACHE_DIR`: an application cache other than the user's;
//! - `PYOXIDE_DEBUG_DEMO`: a copy of a project with the demo's `main.py`
//!   (`describe`, `fail`) for [`the_demo_project`]; skipped without it;
//! - `PYOXIDE_TEST_FASTAPI_PYTHON`: an interpreter with FastAPI and uvicorn
//!   for the web application tests; without it each makes a virtual
//!   environment with `uv` (`fastapi[standard]`, from uv's cache or the
//!   network).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use debugger::{DebugRequest, Debugger, Phase, Session};
use futures::executor::block_on;
use process::{ProcessEvent, ProcessHandle, Receiver};
use py_debug::{DebugTarget, Helper, PythonDebugProvider, StoppableLines, configuration_target};
use py_run::UvRun;
use py_run::config::{LaunchContext, PythonOptions};
use run_config::{EnvVar, RunConfiguration, Storage};

const WAIT: Duration = Duration::from_secs(60);

fn python() -> PathBuf {
    std::env::var_os("PYOXIDE_TEST_PYTHON")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "python" } else { "python3" }))
}

fn helper() -> Helper {
    Helper::locate().unwrap_or_else(|missing| panic!("{missing}"))
}

/// A provider that runs configurations with [`python`], through `uv run`
/// when `uv` is on and the interpreter has an environment.
fn provider(uv: bool) -> Arc<PythonDebugProvider> {
    provider_with(python(), uv)
}

/// [`provider`] with another interpreter.
fn provider_with(python: PathBuf, uv: bool) -> Arc<PythonDebugProvider> {
    let resolve = move |request: &DebugRequest| -> Result<DebugTarget, String> {
        let uv = uv
            .then(|| py_env::paths::venv_root_of(&python))
            .flatten()
            .map(|environment| UvRun {
                // Never started: a uv run is debugged with the interpreter.
                uv: PathBuf::from("uv-is-not-run"),
                environment,
            });
        let cx = LaunchContext {
            root: &request.root,
            source_roots: &[],
            python: python.clone(),
            uv,
        };
        configuration_target(request, &cx, None)
    };
    Arc::new(PythonDebugProvider::new(resolve).with_helper(helper()))
}

fn configuration(script: &str, edit: impl FnOnce(&mut PythonOptions)) -> RunConfiguration {
    let mut config = py_run::config::new_configuration("main", Storage::Personal);
    let mut options = PythonOptions::read(&config);
    options.script = script.to_owned();
    edit(&mut options);
    options.write(&mut config);
    config
}

fn request(root: &Path, configuration: RunConfiguration) -> DebugRequest {
    DebugRequest::new(root, configuration)
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits for a stop of `reason` at `line` of the top frame.
fn wait_stop(session: &Session, reason: &str, line: u32) {
    wait_until(&format!("a {reason} stop at line {line}"), || {
        let s = session.snapshot();
        assert!(s.error.is_none(), "{:?}", s.error);
        assert_ne!(s.phase, Phase::Ended, "the session ended");
        s.phase == Phase::Suspended
            && s.stop.as_ref().is_some_and(|stop| stop.reason == reason)
            && s.frames.first().is_some_and(|f| f.line == line)
    });
}

/// The program's console: the `runInTerminal` process and its output.
struct Console {
    handle: ProcessHandle,
    events: Receiver<ProcessEvent>,
    output: String,
}

impl Console {
    fn of(session: &Session) -> Console {
        let mut console = None;
        wait_until("the program's console", || {
            console = console.take().or_else(|| session.take_console());
            console.is_some()
        });
        let (handle, events) = console.unwrap();
        Console {
            handle,
            events,
            output: String::new(),
        }
    }

    /// Reads the output until it contains `text`.
    fn wait_for(&mut self, text: &str) {
        let start = Instant::now();
        while !self.output.contains(text) {
            let left = WAIT.saturating_sub(start.elapsed());
            match self.events.recv_timeout(left) {
                Ok(ProcessEvent::Stdout(chunk) | ProcessEvent::Stderr(chunk)) => {
                    self.output.push_str(&String::from_utf8_lossy(&chunk));
                }
                Ok(ProcessEvent::Exited(_)) | Err(_) => {
                    panic!("{text:?} was not printed; the output:\n{}", self.output)
                }
            }
        }
    }

    /// Reads the output to the end.
    fn finish(&mut self) {
        while let Ok(event) = self.events.recv_timeout(WAIT) {
            match event {
                ProcessEvent::Stdout(chunk) | ProcessEvent::Stderr(chunk) => {
                    self.output.push_str(&String::from_utf8_lossy(&chunk));
                }
                ProcessEvent::Exited(_) => return,
            }
        }
        panic!("the console did not end; the output:\n{}", self.output);
    }
}

/// The value of a variable of the top frame's first scope.
fn local(session: &Session, name: &str) -> String {
    let frame = session.snapshot().frames[0].id;
    let scopes = block_on(session.scopes(frame)).unwrap();
    let locals = block_on(session.children(scopes[0].reference)).unwrap();
    locals
        .items
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no {name} in {:?}", locals.items))
        .value
        .clone()
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
}

const SCRIPT: &str = "\
def describe(items):
    summary = {\"count\": len(items), \"sum\": 0}
    for value in items:
        summary[\"sum\"] += value
    return summary


if __name__ == \"__main__\":
    numbers = [1, 2, 3, 4]
    result = describe(numbers)
    print(\"result\", result)
";

#[test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn launch_breakpoint_variables_step_and_exit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    std::fs::write(&main, SCRIPT).unwrap();
    let lines = StoppableLines::of(SCRIPT);
    assert!(lines.contains(2) && lines.contains(4) && !lines.contains(6));

    let debugger = Debugger::open(root, provider(false));
    debugger.toggle_breakpoint(&main, 2, false);
    let session = debugger
        .start(request(root, configuration("main.py", |_| {})))
        .unwrap();
    assert_eq!(session.name(), "main");
    wait_stop(&session, "breakpoint", 2);
    let two = debugger.breakpoints().at(&main, 2).unwrap().id;
    assert!(debugger.breakpoint_status(two).unwrap().verified);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.frames[0].name, "describe");
    assert_eq!(snapshot.threads[0].name, "MainThread");
    assert_eq!(local(&session, "items"), "[1, 2, 3, 4]");

    block_on(session.step_over()).unwrap();
    wait_stop(&session, "step", 3);
    assert_eq!(local(&session, "summary"), "{'count': 4, 'sum': 0}");
    block_on(session.step_over()).unwrap();
    wait_stop(&session, "step", 4);
    assert_eq!(local(&session, "value"), "1");

    let mut console = Console::of(&session);
    block_on(session.resume()).unwrap();
    wait_until("the end", || session.is_ended());
    console.finish();
    assert!(
        console.output.contains("result {'count': 4, 'sum': 10}"),
        "{}",
        console.output
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.exit_code, Some(0));
    assert_eq!(snapshot.error, None);
    wait_until("the processes end", || {
        !console.handle.is_running() && snapshot.adapter_pid.is_none_or(|pid| !alive(pid))
    });
}

#[test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn an_uncaught_exception_stops_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("main.py"),
        "def fail(value):\n    if value > 2:\n        raise ValueError(f\"too big: {value}\")\n    return value\n\n\nfail(1)\nfail(3)\n",
    )
    .unwrap();
    // The default "Any exception" breakpoint stops on termination.
    let debugger = Debugger::open(root, provider(false));
    let session = debugger
        .start(request(root, configuration("main.py", |_| {})))
        .unwrap();
    let mut console = Console::of(&session);
    wait_stop(&session, "exception", 3);
    let stop = session.snapshot().stop.unwrap();
    let exception = stop.exception.expect("the exception is told");
    assert!(
        exception.id.contains("ValueError") || exception.type_name.as_deref() == Some("ValueError"),
        "{exception:?}"
    );
    block_on(session.resume()).unwrap();
    wait_until("the end", || session.is_ended());
    console.finish();
    assert!(
        console.output.contains("ValueError: too big: 3"),
        "{}",
        console.output
    );
    assert_eq!(session.snapshot().exit_code, Some(1));
}

#[test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn stop_ends_the_adapter_the_launcher_and_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    std::fs::write(
        &main,
        "import os, time\nprint(\"pid\", os.getpid(), flush=True)\nwhile True:\n    time.sleep(0.05)\n",
    )
    .unwrap();
    let debugger = Debugger::open(root, provider(false));
    debugger.toggle_breakpoint(&main, 4, false);
    let session = debugger
        .start(request(root, configuration("main.py", |_| {})))
        .unwrap();
    let mut console = Console::of(&session);
    console.wait_for("pid ");
    wait_stop(&session, "breakpoint", 4);
    console.wait_for("\n");
    let program: u32 = console
        .output
        .lines()
        .find_map(|l| l.strip_prefix("pid "))
        .and_then(|pid| pid.trim().parse().ok())
        .unwrap();
    let snapshot = session.snapshot();
    let adapter = snapshot.adapter_pid.unwrap();
    let launcher = snapshot.debuggee_pid.unwrap();
    assert!(alive(adapter) && alive(launcher) && alive(program));

    session.stop().recv_timeout(WAIT).unwrap();
    wait_until("the end", || session.is_ended());
    wait_until("every process ends", || {
        !alive(adapter) && !alive(launcher) && !alive(program)
    });
    assert_eq!(session.snapshot().error, None);
}

#[test]
#[ignore = "needs Python and the debugpy helper (scripts/fetch-debugpy.py)"]
fn a_module_with_arguments_and_environment() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let package = root.join("tool");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("__init__.py"), "").unwrap();
    std::fs::write(
        package.join("__main__.py"),
        "import os, sys\nprint(\"args\", sys.argv[1:])\nprint(\"var\", os.environ.get(\"PYOXIDE_DEBUG_VAR\"))\nprint(\"venv\", os.environ.get(\"VIRTUAL_ENV\") is not None)\n",
    )
    .unwrap();
    let mut config = configuration("tool", |o| {
        o.module_mode = true;
        o.parameters = "one \"two words\"".to_owned();
    });
    config
        .env
        .push(EnvVar::new("PYOXIDE_DEBUG_VAR", "set", true));
    // `uv run` on: debugged with the environment's interpreter.
    let venv = py_env::paths::venv_root_of(&python()).is_some();
    let debugger = Debugger::open(root, provider(true));
    let session = debugger.start(request(root, config)).unwrap();
    let mut console = Console::of(&session);
    wait_until("the end", || session.is_ended());
    console.finish();
    let output = console.output.replace("\r\n", "\n");
    assert!(output.contains("args ['one', 'two words']"), "{output}");
    assert!(output.contains("var set"), "{output}");
    assert!(
        output.contains(&format!("venv {}", if venv { "True" } else { "False" })),
        "{output}"
    );
    assert_eq!(session.snapshot().exit_code, Some(0));
}

/// The demo project (`PYOXIDE_DEBUG_DEMO`): a breakpoint in the loop of
/// `describe`, its values, then the end.
#[test]
#[ignore = "needs a copy of the demo project (PYOXIDE_DEBUG_DEMO) and the debugpy helper"]
fn the_demo_project() {
    let Some(root) = std::env::var_os("PYOXIDE_DEBUG_DEMO").map(PathBuf::from) else {
        eprintln!("PYOXIDE_DEBUG_DEMO is not set: skipped");
        return;
    };
    let main = root.join("main.py");
    let text = std::fs::read_to_string(&main).unwrap();
    let lines = StoppableLines::of(&text);
    assert_eq!(
        lines.lines().take(6).collect::<Vec<_>>(),
        [1, 4, 5, 6, 7, 8],
        "the demo's stoppable lines"
    );
    let debugger = Debugger::open(&root, provider(true));
    debugger.toggle_breakpoint(&main, 7, false);
    let session = debugger
        .start(request(&root, configuration("main.py", |_| {})))
        .unwrap();
    wait_stop(&session, "breakpoint", 7);
    assert_eq!(local(&session, "items"), "[1, 2, 3, 4]");
    assert_eq!(local(&session, "summary"), "{'count': 4, 'sum': 0}");
    let id = debugger.breakpoints().at(&main, 7).unwrap().id;
    debugger.remove_breakpoint(id);
    let mut console = Console::of(&session);
    block_on(session.resume()).unwrap();
    wait_until("the end", || session.is_ended());
    console.finish();
    assert!(
        console.output.contains("{'count': 4, 'sum': 10}"),
        "{}",
        console.output
    );
}

// ---- a web application: FastAPI under uvicorn ------------------------------

/// An interpreter with FastAPI and uvicorn: `PYOXIDE_TEST_FASTAPI_PYTHON`,
/// else one of a virtual environment made with `uv` in `dir` (with
/// `fastapi[standard]`: uvloop, watchfiles and the `fastapi` command).
fn fastapi_python(dir: &Path) -> PathBuf {
    if let Some(python) = std::env::var_os("PYOXIDE_TEST_FASTAPI_PYTHON").filter(|p| !p.is_empty())
    {
        return PathBuf::from(python);
    }
    let venv = dir.join(".venv");
    let uv = |args: &[&str]| {
        let status = std::process::Command::new("uv")
            .args(args)
            .env("VIRTUAL_ENV", &venv)
            .status()
            .unwrap_or_else(|e| panic!("uv is needed without PYOXIDE_TEST_FASTAPI_PYTHON: {e}"));
        assert!(status.success(), "uv {args:?} failed");
    };
    uv(&["venv", "--quiet", &venv.to_string_lossy()]);
    uv(&["pip", "install", "--quiet", "fastapi[standard]"]);
    py_env::paths::environment_python(&venv).expect("the environment's interpreter")
}

/// The application: an `async def` endpoint that answers with its
/// process, its parent and a variable of the `.env` file.
const API: &str = "\
import os

from fastapi import FastAPI

app = FastAPI()


@app.get(\"/hello\")
async def hello():
    pid = os.getpid()
    return {\"pid\": pid, \"ppid\": os.getppid(), \"greeting\": os.environ.get(\"GREETING\")}
";
/// The breakpoint's line in [`API`].
const API_LINE: u32 = 10;

/// A port nothing listens on.
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Calls `GET /hello` as soon as the server listens; the body comes
/// through the receiver.
fn call_hello(port: u16) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let start = Instant::now();
        let mut stream = loop {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) if start.elapsed() < WAIT => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("the server did not listen on {port}: {e}"),
            }
        };
        stream
            .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or(response);
        let _ = tx.send(body);
    });
    rx
}

/// How the server is started.
enum Server {
    /// `run.py`: `uvicorn.run(app)`.
    Run,
    /// `run.py`: `uvicorn.run("service.api:app", reload=True)`.
    RunReload,
    /// `python -m uvicorn service.api:app --reload`.
    ModuleReload,
    /// `python -m fastapi dev service/api.py`.
    FastapiDev,
}

/// The project of the failing report: `run.py` at the root run with
/// `uv run`, the working directory a subfolder, a `.env` file, "Debug just
/// my code" on; the endpoint in a package outside the working directory.
/// A breakpoint in the `async def` endpoint stops the session when the
/// endpoint is called (in the server's process, a subprocess with a
/// reloader), stepping works there, and Stop ends the server and its
/// parents.
fn web_app(server: Server) {
    let dir = tempfile::tempdir().unwrap();
    let python = fastapi_python(dir.path());
    let root = dir.path().join("project");
    let service = root.join("service");
    std::fs::create_dir_all(&service).unwrap();
    std::fs::create_dir_all(root.join("application")).unwrap();
    std::fs::write(service.join("__init__.py"), "").unwrap();
    let api = service.join("api.py");
    std::fs::write(&api, API).unwrap();
    std::fs::write(root.join(".env"), "GREETING=hi\n").unwrap();
    let port = free_port();
    let (script, module, parameters) = match server {
        Server::Run => {
            let run = format!(
                "import uvicorn\n\nfrom service.api import app\n\nif __name__ == \"__main__\":\n    uvicorn.run(app, port={port})\n"
            );
            std::fs::write(root.join("run.py"), run).unwrap();
            ("run.py".to_owned(), false, String::new())
        }
        Server::RunReload => {
            let run = format!(
                "import uvicorn\n\nif __name__ == \"__main__\":\n    uvicorn.run(\"service.api:app\", port={port}, reload=True)\n"
            );
            std::fs::write(root.join("run.py"), run).unwrap();
            ("run.py".to_owned(), false, String::new())
        }
        Server::ModuleReload => (
            "uvicorn".to_owned(),
            true,
            format!("service.api:app --port {port} --reload"),
        ),
        Server::FastapiDev => (
            "fastapi".to_owned(),
            true,
            format!("dev \"{}\" --port {port}", api.display()),
        ),
    };
    let config = configuration(&script, |o| {
        o.module_mode = module;
        o.parameters = parameters;
        o.working_directory = root.join("application").to_string_lossy().into_owned();
        o.env_files = vec![root.join(".env").to_string_lossy().into_owned()];
        o.run_with_uv = Some(true);
        o.debug_just_my_code = true;
    });

    let debugger = Debugger::open(&root, provider_with(python, true));
    debugger.toggle_breakpoint(&api, API_LINE, false);
    let session = debugger.start(request(&root, config)).unwrap();
    let mut console = Console::of(&session);
    let answer = call_hello(port);
    wait_stop(&session, "breakpoint", API_LINE);
    assert_eq!(session.snapshot().frames[0].name, "hello");
    block_on(session.step_over()).unwrap();
    wait_stop(&session, "step", API_LINE + 1);
    let pid = local(&session, "pid");
    block_on(session.resume()).unwrap();
    let body = answer.recv_timeout(WAIT).unwrap_or_else(|_| {
        console.finish();
        panic!("no answer; the output:\n{}", console.output)
    });
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["pid"].to_string(), pid, "{body}");
    assert_eq!(body["greeting"], "hi", "{body}");
    let server_pid = body["pid"].as_u64().unwrap() as u32;
    let parent = body["ppid"].as_u64().unwrap() as u32;
    let snapshot = session.snapshot();
    let adapter = snapshot.adapter_pid.unwrap();
    let launcher = snapshot.debuggee_pid.unwrap();
    assert!(alive(server_pid) && alive(parent));

    session.stop().recv_timeout(WAIT).unwrap();
    wait_until("the end", || session.is_ended());
    wait_until("every process ends", || {
        [adapter, launcher, parent, server_pid]
            .iter()
            .all(|pid| !alive(*pid))
    });
    assert_eq!(session.snapshot().error, None);
}

#[test]
#[ignore = "needs the debugpy helper and FastAPI (PYOXIDE_TEST_FASTAPI_PYTHON or uv)"]
fn a_breakpoint_in_an_async_endpoint_with_uvicorn_run() {
    web_app(Server::Run);
}

#[test]
#[ignore = "needs the debugpy helper and FastAPI (PYOXIDE_TEST_FASTAPI_PYTHON or uv)"]
fn a_breakpoint_in_an_async_endpoint_with_uvicorn_run_reload() {
    web_app(Server::RunReload);
}

#[test]
#[ignore = "needs the debugpy helper and FastAPI (PYOXIDE_TEST_FASTAPI_PYTHON or uv)"]
fn a_breakpoint_in_an_async_endpoint_with_python_m_uvicorn_reload() {
    web_app(Server::ModuleReload);
}

#[test]
#[ignore = "needs the debugpy helper and FastAPI (PYOXIDE_TEST_FASTAPI_PYTHON or uv)"]
fn a_breakpoint_in_an_async_endpoint_with_fastapi_dev() {
    web_app(Server::FastapiDev);
}

// ---- stepping in an async endpoint with background work --------------------

/// An application with a busy background task started in `lifespan` (an
/// `asyncio.Task` polling with `asyncio.sleep`, on the same event loop and
/// thread as the endpoint) and a busy thread.
const BUSY_API: &str = "\
import asyncio
import os
import threading
import time
from contextlib import asynccontextmanager

from fastapi import FastAPI


async def poll(state):
    while True:
        state[\"ticks\"] += 1
        await asyncio.sleep(0)
        state[\"ticks\"] += 1
        await asyncio.sleep(0.001)


def work(state):
    while True:
        state[\"beats\"] += 1
        time.sleep(0.001)


@asynccontextmanager
async def lifespan(app):
    state = {\"ticks\": 0, \"beats\": 0}
    task = asyncio.create_task(poll(state))
    threading.Thread(target=work, args=(state,), daemon=True).start()
    yield
    task.cancel()


app = FastAPI(lifespan=lifespan)


async def plus_one(x):
    await asyncio.sleep(0.01)
    return x + 1


@app.get(\"/hello\")
async def hello():
    a = 1
    await asyncio.sleep(0.01)
    b = await plus_one(a)
    await asyncio.sleep(0)
    c = a + b
    return {\"pid\": os.getpid(), \"c\": c}
";
/// `plus_one`'s first line in [`BUSY_API`].
const PLUS_ONE_LINE: u32 = 37;
/// The endpoint's first line in [`BUSY_API`]: the breakpoint.
const HELLO_LINE: u32 = 43;

/// Where a stop is: its reason, the top frame's function and line, and the
/// thread.
#[derive(Debug, PartialEq, Eq)]
struct At {
    reason: String,
    function: String,
    line: u32,
    thread: i64,
}

/// Runs a step and waits for its stop; the session shows the thread that
/// stopped.
fn step_to(session: &Session, step: debugger::Pending<()>) -> At {
    let before = session.snapshot().generation;
    block_on(step).unwrap();
    let mut found = None;
    wait_until("the step's stop", || {
        let s = session.snapshot();
        assert!(s.error.is_none(), "{:?}", s.error);
        assert_ne!(s.phase, Phase::Ended, "the session ended");
        if s.phase != Phase::Suspended || s.generation <= before {
            return false;
        }
        let (Some(stop), Some(top)) = (s.stop.as_ref(), s.frames.first()) else {
            return false;
        };
        assert_eq!(
            s.selected_thread, stop.thread_id,
            "the stopped thread is shown"
        );
        found = Some(At {
            reason: stop.reason.clone(),
            function: top.name.clone(),
            line: top.line,
            thread: stop.thread_id.unwrap(),
        });
        true
    });
    found.unwrap()
}

/// Step Into My Code, Step Over, Step Out and Step Into through an `async
/// def` endpoint, over `await`s of library code and of a coroutine of the
/// program, with "Debug just my code" on: every stop is in the endpoint's
/// coroutine (or the one it awaits) and thread, never in the background
/// task or thread. pydevd steps with `sys.monitoring` from Python 3.12 on,
/// where a step over an `await` stops in whatever task runs next unless
/// the session prepared the process's steps (`launch::task_steps`).
fn async_steps(server: Server) {
    let dir = tempfile::tempdir().unwrap();
    let python = fastapi_python(dir.path());
    let root = dir.path().join("project");
    let service = root.join("service");
    std::fs::create_dir_all(&service).unwrap();
    std::fs::write(service.join("__init__.py"), "").unwrap();
    let api = service.join("api.py");
    std::fs::write(&api, BUSY_API).unwrap();
    let port = free_port();
    let run = match server {
        Server::Run => format!(
            "import uvicorn\n\nfrom service.api import app\n\nif __name__ == \"__main__\":\n    uvicorn.run(app, port={port})\n"
        ),
        Server::RunReload => format!(
            "import uvicorn\n\nif __name__ == \"__main__\":\n    uvicorn.run(\"service.api:app\", port={port}, reload=True)\n"
        ),
        Server::ModuleReload | Server::FastapiDev => unreachable!("a server of run.py"),
    };
    std::fs::write(root.join("run.py"), run).unwrap();
    let config = configuration("run.py", |o| {
        o.working_directory = root.to_string_lossy().into_owned();
        o.run_with_uv = Some(true);
        o.debug_just_my_code = true;
    });

    let debugger = Debugger::open(&root, provider_with(python, true));
    debugger.toggle_breakpoint(&api, HELLO_LINE, false);
    let session = debugger.start(request(&root, config)).unwrap();
    let mut console = Console::of(&session);
    let answer = call_hello(port);
    wait_stop(&session, "breakpoint", HELLO_LINE);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.frames[0].name, "hello");
    let thread = snapshot.stop.as_ref().and_then(|s| s.thread_id).unwrap();
    assert_eq!(snapshot.selected_thread, Some(thread));
    let at = |function: &str, line: u32| At {
        reason: "step".to_owned(),
        function: function.to_owned(),
        line,
        thread,
    };
    type Step = fn(&Session) -> debugger::Pending<()>;
    let steps: [(&str, Step, At); 8] = [
        (
            "Step Into My Code",
            Session::step_into_my_code,
            at("hello", HELLO_LINE + 1),
        ),
        // Over `await asyncio.sleep(0.01)`.
        (
            "Step Into My Code",
            Session::step_into_my_code,
            at("hello", HELLO_LINE + 2),
        ),
        // Into the awaited coroutine.
        (
            "Step Into My Code",
            Session::step_into_my_code,
            at("plus_one", PLUS_ONE_LINE),
        ),
        (
            "Step Over",
            Session::step_over,
            at("plus_one", PLUS_ONE_LINE + 1),
        ),
        ("Step Out", Session::step_out, at("hello", HELLO_LINE + 2)),
        ("Step Over", Session::step_over, at("hello", HELLO_LINE + 3)),
        // Over `await asyncio.sleep(0)`.
        ("Step Into", Session::step_into, at("hello", HELLO_LINE + 4)),
        ("Step Over", Session::step_over, at("hello", HELLO_LINE + 5)),
    ];
    for (name, step, expected) in steps {
        assert_eq!(step_to(&session, step(&session)), expected, "{name}");
    }
    assert_eq!(local(&session, "c"), "3");
    let breakpoint = debugger.breakpoints().at(&api, HELLO_LINE).unwrap().id;
    debugger.remove_breakpoint(breakpoint);
    block_on(session.resume()).unwrap();
    let body = answer.recv_timeout(WAIT).unwrap_or_else(|_| {
        console.finish();
        panic!("no answer; the output:\n{}", console.output)
    });
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["c"], 3, "{body}");

    session.stop().recv_timeout(WAIT).unwrap();
    wait_until("the end", || session.is_ended());
    assert_eq!(session.snapshot().error, None);
}

#[test]
#[ignore = "needs the debugpy helper and FastAPI (PYOXIDE_TEST_FASTAPI_PYTHON or uv)"]
fn steps_in_an_async_endpoint_stay_in_its_coroutine() {
    async_steps(Server::Run);
}

#[test]
#[ignore = "needs the debugpy helper and FastAPI (PYOXIDE_TEST_FASTAPI_PYTHON or uv)"]
fn steps_in_an_async_endpoint_of_a_reloading_server_stay_in_its_coroutine() {
    async_steps(Server::RunReload);
}
