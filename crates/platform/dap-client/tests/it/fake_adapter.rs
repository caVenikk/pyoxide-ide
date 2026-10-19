//! The client against the `dap-fake-adapter` binary over standard streams,
//! as against a real adapter process (the same on every OS).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dap_client::{
    Capabilities, Client, ClientInfo, DapError, Event, ExceptionFilterSetting, Handler, Responder,
    ReverseRequest, SourceBreakpoint, Transport,
};
use futures::FutureExt;
use futures::executor::block_on;
use process::{CommandLine, ExitStatus};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Recorder {
    log: Mutex<Vec<String>>,
    events: Mutex<Vec<Event>>,
    stderr: Mutex<String>,
    exit: Mutex<Option<Option<ExitStatus>>>,
    /// Answers `runInTerminal` with this process id.
    terminal_pid: Mutex<Option<u32>>,
}

impl Recorder {
    fn log(&self, text: impl Into<String>) {
        self.log.lock().unwrap().push(text.into());
    }

    fn wait_for(&self, what: &str, test: impl Fn(&Event) -> bool) {
        let start = Instant::now();
        while !self.events.lock().unwrap().iter().any(&test) {
            assert!(
                start.elapsed() < WAIT,
                "no {what}; log: {:?}, adapter: {}",
                self.log.lock().unwrap(),
                self.stderr.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_exit(&self) -> Option<ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(status) = *self.exit.lock().unwrap() {
                return status;
            }
            assert!(start.elapsed() < WAIT, "the adapter did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Handler for Recorder {
    fn event(&self, event: Event) {
        self.log(format!("event {event:?}"));
        self.events.lock().unwrap().push(event);
    }

    fn reverse_request(&self, request: ReverseRequest, responder: Responder) {
        self.log(format!("request {request:?}"));
        if let ReverseRequest::RunInTerminal(_) = request {
            let pid = *self.terminal_pid.lock().unwrap();
            responder.run_in_terminal(Ok((pid, None)));
        }
    }

    fn stderr(&self, chunk: &[u8]) {
        self.stderr
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(chunk));
    }

    fn protocol_error(&self, error: String) {
        self.log(format!("error {error}"));
    }

    fn closed(&self, status: Option<ExitStatus>) {
        *self.exit.lock().unwrap() = Some(status);
    }
}

fn start(script: Value) -> (Client, Arc<Recorder>) {
    let command = CommandLine::new(env!("CARGO_BIN_EXE_dap-fake-adapter"))
        .with_env("DAP_FAKE_SCRIPT", script.to_string());
    let transport = Transport::stdio(&command).expect("the fake adapter starts");
    let recorder = Arc::new(Recorder::default());
    let client = Client::start("fake", transport, recorder.clone()).unwrap();
    (client, recorder)
}

fn initialize(client: &Client) -> Capabilities {
    block_on(client.initialize(&ClientInfo::new("fake"))).unwrap()
}

fn assert_exit_code(recorder: &Recorder, code: i32) {
    let status = recorder.wait_exit();
    assert_eq!(
        status.and_then(|s| s.code),
        Some(code),
        "adapter log: {}",
        recorder.stderr.lock().unwrap()
    );
}

/// The order debugpy uses: `initialized` comes while `launch` waits, the
/// client configures, and `launch` is answered after `configurationDone`.
#[test]
fn initialized_comes_before_the_launch_answer() {
    let (client, recorder) = start(json!([
        {"expect": "initialize", "arguments": {"adapterID": "fake", "linesStartAt1": true}},
        {"respond": {"supportsConfigurationDoneRequest": true,
            "exceptionBreakpointFilters": [{"filter": "uncaught", "label": "Uncaught", "default": true}]}},
        {"expect": "launch", "arguments": {"program": "main.py"}},
        {"event": "initialized"},
        {"expect": "setBreakpoints", "arguments": {
            "source": {"path": "$any"}, "breakpoints": [{"line": 3}, {"line": 7, "condition": "x > 1"}]}},
        {"respond": {"breakpoints": [{"verified": true, "line": 3, "id": 1},
            {"verified": false, "line": 8, "id": 2, "message": "moved"}]}},
        {"expect": "setExceptionBreakpoints", "arguments": {"filters": ["uncaught"]}},
        {"respond": null},
        {"forbid": "launch", "for": 50},
        {"expect": "configurationDone"},
        {"respond": null},
        {"respond": null, "to": "launch"},
        {"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1, "hitBreakpointIds": [1]}},
    ]));
    let caps = initialize(&client);
    assert!(caps.configuration_done);
    assert_eq!(caps.exception_filters[0].id, "uncaught");
    let mut launch = client.launch(json!({"program": "main.py"}));
    recorder.wait_for("initialized", |e| *e == Event::Initialized);
    assert!((&mut launch).now_or_never().is_none());
    let breakpoints = block_on(client.set_breakpoints(
        Path::new("main.py"),
        &[
            SourceBreakpoint::at(3),
            SourceBreakpoint {
                condition: Some("x > 1".into()),
                ..SourceBreakpoint::at(7)
            },
        ],
    ))
    .unwrap();
    assert!(breakpoints[0].verified);
    assert_eq!(breakpoints[1].line, Some(8));
    assert_eq!(breakpoints[1].message.as_deref(), Some("moved"));
    block_on(client.set_exception_breakpoints(&[ExceptionFilterSetting::new("uncaught")])).unwrap();
    assert!((&mut launch).now_or_never().is_none());
    block_on(client.configuration_done()).unwrap();
    block_on(launch).unwrap();
    recorder.wait_for(
        "stopped",
        |e| matches!(e, Event::Stopped(s) if s.hit_breakpoint_ids == [1]),
    );
    let log = recorder.log.lock().unwrap().clone();
    let initialized = log.iter().position(|l| l == "event Initialized");
    let stopped = log.iter().position(|l| l.starts_with("event Stopped"));
    assert!(initialized < stopped, "{log:?}");
    let done = client.shutdown(true, WAIT);
    done.recv_timeout(WAIT).unwrap();
    assert_exit_code(&recorder, 0);
}

#[test]
fn a_dropped_request_is_cancelled() {
    let (client, recorder) = start(json!([
        {"expect": "initialize"},
        {"respond": {"supportsCancelRequest": true}},
        {"expect": "evaluate", "arguments": {"expression": "slow()", "context": "repl"}},
        {"expect": "cancel", "arguments": {"requestId": "$seq:evaluate"}},
        {"respond": null},
        {"fail": "cancelled", "to": "evaluate"},
        {"expect": "threads"},
        {"respond": {"threads": [{"id": 1, "name": "MainThread"}]}},
    ]));
    initialize(&client);
    let slow = client.evaluate("slow()", None, dap_client::EvaluateContext::Repl);
    // Sent before it is dropped: the adapter sees it, then `cancel`.
    std::thread::sleep(Duration::from_millis(50));
    drop(slow);
    let threads = block_on(client.threads()).unwrap();
    assert_eq!(threads[0].name, "MainThread");
    assert!(
        recorder.log.lock().unwrap().is_empty(),
        "the late answer is dropped: {:?}",
        recorder.log.lock().unwrap()
    );
    client.shutdown(false, WAIT).recv_timeout(WAIT).unwrap();
    assert_exit_code(&recorder, 0);
}

#[test]
fn an_adapter_crash_fails_pending_and_later_requests() {
    let (client, recorder) = start(json!([
        {"expect": "initialize"},
        {"respond": {}},
        {"expect": "stackTrace", "arguments": {"threadId": 1, "startFrame": 0, "levels": 20}},
        {"stderr": "Traceback: boom\n"},
        {"exit": 3},
    ]));
    initialize(&client);
    let pending = client.stack_trace(1, 0, 20);
    assert_eq!(block_on(pending), Err(DapError::Closed));
    assert_exit_code(&recorder, 3);
    assert!(recorder.stderr.lock().unwrap().contains("Traceback: boom"));
    assert!(client.is_closed());
    assert_eq!(block_on(client.threads()), Err(DapError::Closed));
    client.shutdown(true, WAIT).recv_timeout(WAIT).unwrap();
}

#[test]
fn run_in_terminal_is_answered_with_the_process_id() {
    let (client, recorder) = start(json!([
        {"expect": "initialize", "arguments": {"supportsRunInTerminalRequest": true}},
        {"respond": {}},
        {"request": "runInTerminal", "arguments": {"kind": "integrated", "title": "main",
            "cwd": "/work", "args": ["python", "launcher", "main.py"], "env": {"PYTHONIOENCODING": "utf-8"}}},
        {"expect_response": "runInTerminal", "success": true, "body": {"processId": 4242}},
        {"request": "unknownRequest"},
        {"expect_response": "unknownRequest", "success": false},
        {"event": "terminated"},
    ]));
    *recorder.terminal_pid.lock().unwrap() = Some(4242);
    initialize(&client);
    recorder.wait_for("terminated", |e| *e == Event::Terminated);
    let log = recorder.log.lock().unwrap().clone();
    assert!(
        log[0].contains("RunInTerminal") && log[0].contains("launcher"),
        "{log:?}"
    );
    client.shutdown(true, WAIT).recv_timeout(WAIT).unwrap();
    assert_exit_code(&recorder, 0);
}

#[test]
fn shutdown_terminates_then_disconnects() {
    let (client, recorder) = start(json!([
        {"expect": "initialize"},
        {"respond": {"supportsTerminateRequest": true}},
        {"expect": "terminate"},
        {"respond": null},
        {"event": "terminated"},
        {"expect": "disconnect", "arguments": {"terminateDebuggee": true}},
        {"respond": null},
    ]));
    initialize(&client);
    client.shutdown(true, WAIT).recv_timeout(WAIT).unwrap();
    assert_exit_code(&recorder, 0);
    assert!(!client.process().unwrap().is_running());
}

#[test]
fn shutdown_kills_an_adapter_that_does_not_answer() {
    let (client, recorder) = start(json!([
        {"expect": "initialize"},
        {"respond": {"supportsTerminateRequest": true}},
        {"ignore": ["terminate", "disconnect"]},
        {"delay": 600000},
    ]));
    initialize(&client);
    let start = Instant::now();
    client
        .shutdown(true, Duration::from_millis(300))
        .recv_timeout(WAIT)
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(10));
    recorder.wait_exit();
    assert!(!client.process().unwrap().is_running());
}
