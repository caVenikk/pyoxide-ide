//! Debug sessions against scripted adapters in the test's process
//! (`dap_client::fake`), the storage of the debugger state and its import.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use commands::Registry;
use debugger::{
    Adapter, AdapterLaunch, DebugAdapterProvider, DebugError, DebugRequest, Debugger,
    DebuggerEvent, EvaluateContext, ExceptionBreakpoint, ExceptionFilter, ExceptionFilterSetting,
    HistoryKind, IdeaTypes, LaunchKind, Phase, Session, StepIntoMyCode, SuspendPolicy,
};
use futures::executor::block_on;
use run_config::{RunConfiguration, Storage};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);

/// A provider whose adapter runs the next script.
#[derive(Default)]
struct Provider {
    script: Mutex<Option<Vec<Value>>>,
    /// Step Into My Code sends `evaluate` requests around the step.
    my_code: bool,
    /// Each process gets an `evaluate` request before its first step.
    prepares: bool,
    /// Child sessions are offered, with these scripts for the ones the
    /// adapter asks for, in order.
    offers_children: bool,
    children: Mutex<Vec<Vec<Value>>>,
    /// How long an evaluation waits for its answer.
    evaluation_timeout: Option<Duration>,
}

impl Provider {
    fn with(script: Value) -> Arc<Self> {
        let provider = Arc::new(Self::default());
        provider.next(script);
        provider
    }

    fn next(&self, script: Value) {
        let Value::Array(steps) = script else {
            panic!("a script is an array")
        };
        *self.script.lock().unwrap() = Some(steps);
    }
}

impl DebugAdapterProvider for Provider {
    fn prepare(&self, request: &DebugRequest) -> Result<AdapterLaunch, String> {
        let script = self.script.lock().unwrap().take().ok_or("no script")?;
        Ok(AdapterLaunch {
            title: request.configuration.name.clone(),
            adapter: Adapter::Transport(dap_client::fake::adapter(script)),
            adapter_id: "fake".to_owned(),
            kind: LaunchKind::Launch,
            arguments: json!({"program": "main.py"}),
            input_file: None,
            evaluation_timeout: self.evaluation_timeout,
        })
    }

    fn exception_filters(
        &self,
        breakpoints: &[&ExceptionBreakpoint],
        _available: &[ExceptionFilter],
    ) -> Vec<ExceptionFilterSetting> {
        breakpoints
            .iter()
            .map(|b| ExceptionFilterSetting::new(if b.default { "uncaught" } else { "raised" }))
            .collect()
    }

    fn default_exception_breakpoints(&self) -> Vec<ExceptionBreakpoint> {
        vec![ExceptionBreakpoint::new(
            "python-exception",
            "BaseException",
        )]
    }

    fn child_sessions(&self) -> bool {
        self.offers_children
    }

    fn child_adapter(&self, configuration: &Value) -> Result<Adapter, String> {
        assert_eq!(configuration["subProcessId"], 7, "{configuration}");
        let mut children = self.children.lock().unwrap();
        if children.is_empty() {
            return Err("no child script".to_owned());
        }
        Ok(Adapter::Transport(dap_client::fake::adapter(
            children.remove(0),
        )))
    }

    fn step_into_my_code(&self) -> Option<StepIntoMyCode> {
        self.my_code.then(|| StepIntoMyCode {
            before: ("evaluate".to_owned(), json!({"expression": "filter on"})),
            after: ("evaluate".to_owned(), json!({"expression": "filter off"})),
        })
    }

    fn prepare_steps(&self) -> Option<(String, Value)> {
        self.prepares
            .then(|| ("evaluate".to_owned(), json!({"expression": "prepare"})))
    }

    fn idea_types(&self) -> IdeaTypes {
        IdeaTypes {
            line: vec!["python-line".into()],
            exception: vec!["python-exception".into()],
            exception_class: "exception".into(),
            configuration_types: vec![("PythonConfigurationType".into(), "python".into())],
        }
    }
}

fn request(root: &Path) -> DebugRequest {
    DebugRequest::new(
        root,
        RunConfiguration::new("python", "main", Storage::Personal),
    )
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_phase(session: &Session, phase: Phase) {
    wait_until(&format!("{phase:?}"), || {
        let snapshot = session.snapshot();
        assert!(
            snapshot.error.is_none() || phase == Phase::Ended,
            "{:?}",
            snapshot.error
        );
        snapshot.phase == phase
    });
}

/// What every session starts with: capabilities, `launch` while
/// `initialized` comes, the configuration, then the launch answer.
fn start_steps(breakpoints: Value, filters: Value) -> Vec<Value> {
    vec![
        json!({"expect": "initialize", "arguments": {"adapterID": "fake"}}),
        json!({"respond": {"supportsConfigurationDoneRequest": true, "supportsSetVariable": true,
            "supportsTerminateRequest": true, "supportsExceptionInfoRequest": true,
            "exceptionBreakpointFilters": [{"filter": "uncaught", "label": "Uncaught", "default": true},
                                           {"filter": "raised", "label": "Raised"}]}}),
        json!({"expect": "launch", "arguments": {"program": "main.py"}}),
        json!({"event": "initialized"}),
        json!({"expect": "setBreakpoints", "arguments": {"breakpoints": breakpoints}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 3, "id": 11},
            {"verified": false, "line": 8, "id": 12, "message": "moved"}]}}),
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": filters}}),
        json!({"respond": null}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "launch"}),
    ]
}

#[test]
fn a_session_stops_and_shows_frames_variables_and_values() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    // A program that ends at once on every OS: this test binary listing
    // its tests.
    let program = std::env::current_exe().unwrap();
    let mut script = start_steps(
        json!([{"line": 3}, {"line": 7, "condition": "x > 1"}]),
        json!(["uncaught"]),
    );
    script.extend([
        json!({"request": "runInTerminal", "arguments": {"kind": "integrated", "cwd": root,
            "args": [program, "--list"], "env": {"PYOXIDE_DEBUG_TEST": "1"}}}),
        json!({"expect_response": "runInTerminal", "success": true, "body": {"processId": "$any"}}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1, "hitBreakpointIds": [11]}}),
        json!({"expect": "threads"}),
        json!({"respond": {"threads": [{"id": 1, "name": "MainThread"}]}}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1, "startFrame": 0, "levels": 20}}),
        json!({"respond": {"stackFrames": [
            {"id": 100, "name": "describe", "line": 3, "column": 1, "source": {"path": main}},
            {"id": 101, "name": "<module>", "line": 9, "column": 1, "source": {"path": main}}],
            "totalFrames": 2}}),
        json!({"expect": "scopes", "arguments": {"frameId": 100}}),
        json!({"respond": {"scopes": [{"name": "Locals", "variablesReference": 5, "expensive": false}]}}),
        json!({"expect": "variables", "arguments": {"variablesReference": 5, "start": "$absent", "count": "$absent"}}),
        json!({"respond": {"variables": [
            {"name": "x", "value": "1", "type": "int", "variablesReference": 0},
            {"name": "items", "value": "[1, 2]", "type": "list", "variablesReference": 6, "indexedVariables": 2}]}}),
        json!({"expect": "variables", "arguments": {"variablesReference": 6}}),
        json!({"respond": {"variables": [
            {"name": "0", "value": "1", "type": "int", "variablesReference": 0},
            {"name": "1", "value": "2", "type": "int", "variablesReference": 0}]}}),
        json!({"expect": "setVariable", "arguments": {"variablesReference": 6, "name": "0", "value": "5"}}),
        json!({"respond": {"value": "5", "type": "int"}}),
        json!({"expect": "evaluate", "arguments": {"expression": "x + 1", "frameId": 100, "context": "repl"}}),
        json!({"respond": {"result": "2", "type": "int", "variablesReference": 0}}),
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        json!({"respond": {"allThreadsContinued": true}}),
        json!({"event": "exited", "body": {"exitCode": 0}}),
        json!({"event": "terminated"}),
    ]);
    let provider = Provider::with(Value::Array(script));
    let debugger = Debugger::open(root, provider);
    let events = debugger.subscribe();
    debugger.toggle_breakpoint(&main, 3, false);
    debugger.toggle_breakpoint(&main, 7, false);
    let seven = debugger.breakpoints().at(&main, 7).unwrap().id;
    debugger.update_breakpoint(seven, |b| b.condition = Some("x > 1".into()));
    // Disabled and logging-only breakpoints the adapter cannot do are not
    // sent.
    debugger.toggle_breakpoint(&main, 9, false);
    let nine = debugger.breakpoints().at(&main, 9).unwrap().id;
    debugger.update_breakpoint(nine, |b| b.enabled = false);

    let session = debugger.start(request(root)).unwrap();
    assert_eq!(session.name(), "main");
    assert_eq!(session.watches_key(), "python");
    wait_phase(&session, Phase::Suspended);
    wait_until("frames", || session.snapshot().selected_frame == Some(100));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.threads[0].name, "MainThread");
    assert_eq!(snapshot.frames.len(), 2);
    assert_eq!(snapshot.frames_total, Some(2));
    assert_eq!(snapshot.stop.as_ref().unwrap().reason, "breakpoint");
    let three = debugger.breakpoints().at(&main, 3).unwrap().id;
    assert!(debugger.breakpoint_status(three).unwrap().verified);
    let moved = debugger.breakpoint_status(seven).unwrap();
    assert!(!moved.verified);
    assert_eq!(moved.line, Some(8));
    assert_eq!(debugger.breakpoint_status(nine), None);

    let (console, _output) = session.take_console().expect("the program was started");
    assert!(session.take_console().is_none());

    let scopes = block_on(session.scopes(100)).unwrap();
    assert_eq!(scopes[0].name, "Locals");
    let locals = block_on(session.children(5)).unwrap();
    assert_eq!(locals.items.len(), 2);
    assert!(!locals.more);
    let (parent, variable) =
        block_on(session.find_variable(vec!["items".into(), "0".into()])).unwrap();
    assert_eq!((parent, variable.value.as_str()), (6, "1"));
    let set = block_on(session.set_value(parent, &variable, "5")).unwrap();
    assert_eq!(set.value, "5");
    let value = block_on(session.evaluate("x + 1", EvaluateContext::Repl)).unwrap();
    assert_eq!(value.value, "2");

    let state = debugger.state_json();
    assert_eq!(state["sessions"][0]["state"], "suspended");
    assert_eq!(state["sessions"][0]["frames"][0]["file"], "main.py");
    assert_eq!(state["sessions"][0]["variables"][0]["scope"], "Locals");
    assert_eq!(state["breakpoints"][1]["condition"], "x > 1");
    assert_eq!(state["breakpoints"][1]["adapter_line"], 8);

    block_on(session.resume()).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().exit_code, Some(0));
    assert_eq!(session.snapshot().error, None);
    wait_until("the program ends", || !console.is_running());
    let seen: Vec<DebuggerEvent> = events.try_iter().collect();
    assert!(seen.contains(&DebuggerEvent::SessionStarted(session.id())));
    assert!(seen.contains(&DebuggerEvent::Console(session.id())));
    assert!(seen.contains(&DebuggerEvent::SessionEnded(session.id())));
}

#[test]
fn changes_reach_a_running_session_and_stop_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let util = root.join("util.py");
    let mut script = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    script.extend([
        // A new breakpoint in another file.
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 2}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 2, "id": 21}]}}),
        // Muted: nothing suspends.
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "main.py"}, "breakpoints": []}}),
        json!({"respond": {"breakpoints": []}}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"}, "breakpoints": []}}),
        json!({"respond": {"breakpoints": []}}),
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": []}}),
        json!({"respond": null}),
        // Unmuted with a temporary breakpoint, which goes once hit.
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "main.py"},
            "breakpoints": [{"line": 3}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 3, "id": 31}]}}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 2}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 2, "id": 32}]}}),
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": ["uncaught"]}}),
        json!({"respond": null}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 2}, {"line": 5}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 2, "id": 41},
            {"verified": true, "line": 5, "id": 42}]}}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1, "hitBreakpointIds": [42]}}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 2}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 2, "id": 51}]}}),
        json!({"ignore": ["threads", "stackTrace"]}),
        // Stop: terminate, then disconnect.
        json!({"expect": "terminate"}),
        json!({"respond": null}),
        json!({"expect": "disconnect", "arguments": {"terminateDebuggee": true}}),
        json!({"respond": null}),
    ]);
    let debugger = Debugger::open(root, Provider::with(Value::Array(script)));
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    wait_phase(&session, Phase::Running);
    debugger.toggle_breakpoint(&util, 2, false);
    debugger.set_muted(true);
    debugger.set_muted(false);
    debugger.toggle_breakpoint(&util, 5, true);
    wait_phase(&session, Phase::Suspended);
    wait_until("the temporary breakpoint goes", || {
        debugger.breakpoints().at(&util, 5).is_none()
    });
    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
    assert!(debugger.current_session().is_none());
}

/// The ids a `setBreakpoints` answer tells are recorded after a stop that
/// hit one of them (the client runs a callback attached after its answer
/// came on the caller's thread, after the events that followed the answer):
/// the temporary breakpoint still goes. The adapter here sends the stop
/// before the answer.
#[test]
fn a_late_breakpoints_answer_still_removes_the_temporary_breakpoint_it_hit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let util = root.join("util.py");
    let mut script = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    script.extend([
        json!({"ignore": ["threads", "stackTrace"]}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 5}]}}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1, "hitBreakpointIds": [42]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 5, "id": 42}]}}),
        // The temporary breakpoint is gone: the file's breakpoints again.
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": []}}),
        json!({"respond": {"breakpoints": []}}),
        json!({"expect": "terminate"}),
        json!({"respond": null}),
        json!({"expect": "disconnect", "arguments": {"terminateDebuggee": true}}),
        json!({"respond": null}),
    ]);
    let debugger = Debugger::open(root, Provider::with(Value::Array(script)));
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    wait_phase(&session, Phase::Running);
    debugger.toggle_breakpoint(&util, 5, true);
    wait_phase(&session, Phase::Suspended);
    wait_until("the temporary breakpoint goes", || {
        debugger.breakpoints().at(&util, 5).is_none()
    });
    assert!(debugger.breakpoints().at(&main, 3).is_some());
    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
}

/// Run to Cursor adds a breakpoint of the session on the line, Force Run to
/// Cursor sends only that one and no exception filter; the next stop takes
/// them back. Step Into My Code sends the product's request before the
/// step and the other one at the next stop, before the frames.
#[test]
fn run_to_cursor_force_and_step_into_my_code() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    // Each stop tells its breakpoint in its description.
    let stop = |id: i64| {
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1,
            "hitBreakpointIds": [id], "description": id.to_string()}})
    };
    let breakpoints = |lines: Value, answer: Value| {
        [
            json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "main.py"},
                "breakpoints": lines}}),
            json!({"respond": {"breakpoints": answer}}),
        ]
    };
    let mut script = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    script.push(json!({"ignore": ["threads", "stackTrace", "scopes", "variables"]}));
    script.push(stop(11));
    // Run to Cursor: line 7 on top of line 3.
    script.extend(breakpoints(
        json!([{"line": 3}, {"line": 7}]),
        json!([{"verified": true, "line": 3, "id": 61}, {"verified": true, "line": 7, "id": 62}]),
    ));
    script.extend([
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        json!({"respond": {"allThreadsContinued": true}}),
        stop(62),
    ]);
    script.extend(breakpoints(
        json!([{"line": 3}]),
        json!([{"verified": true, "line": 3, "id": 63}]),
    ));
    // Force Run to Cursor: line 9 alone, no exception filter.
    script.extend(breakpoints(
        json!([{"line": 9}]),
        json!([{"verified": true, "line": 9, "id": 64}]),
    ));
    script.extend([
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": []}}),
        json!({"respond": null}),
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        json!({"respond": {"allThreadsContinued": true}}),
        stop(64),
    ]);
    script.extend(breakpoints(
        json!([{"line": 3}]),
        json!([{"verified": true, "line": 3, "id": 65}]),
    ));
    script.extend([
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": ["uncaught"]}}),
        json!({"respond": null}),
        // Step Into My Code.
        json!({"expect": "evaluate", "arguments": {"expression": "filter on"}}),
        json!({"respond": {"result": "None", "variablesReference": 0}}),
        json!({"expect": "stepIn", "arguments": {"threadId": 1}}),
        json!({"respond": null}),
        json!({"event": "stopped", "body": {"reason": "step", "threadId": 1}}),
        json!({"expect": "evaluate", "arguments": {"expression": "filter off"}}),
        json!({"respond": {"result": "None", "variablesReference": 0}}),
        json!({"expect": "terminate"}),
        json!({"respond": null}),
        json!({"expect": "disconnect"}),
        json!({"respond": null}),
    ]);
    let provider = Arc::new(Provider {
        script: Mutex::new(Some(script)),
        my_code: true,
        ..Provider::default()
    });
    let debugger = Debugger::open(root, provider);
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    let wait_stop = |description: Option<&str>, reason: &str| {
        wait_until(&format!("a stop ({reason} {description:?})"), || {
            let s = session.snapshot();
            s.phase == Phase::Suspended
                && s.stop.as_ref().is_some_and(|stop| {
                    stop.reason == reason && stop.description.as_deref() == description
                })
        })
    };
    wait_stop(Some("11"), "breakpoint");
    block_on(session.run_to_position(&main, 7, false)).unwrap();
    wait_stop(Some("62"), "breakpoint");
    // The project's breakpoints are untouched.
    assert!(debugger.breakpoints().at(&main, 7).is_none());
    assert!(debugger.breakpoints().at(&main, 3).is_some());
    block_on(session.run_to_position(&main, 9, true)).unwrap();
    wait_stop(Some("64"), "breakpoint");
    block_on(session.step_into_my_code()).unwrap();
    wait_stop(None, "step");
    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
}

/// The answer to a step or a resume that is handled after the next stop
/// (the client runs a callback attached after its answer came at once, so
/// on a busy machine after the events that followed the answer) leaves the
/// program at that stop. The adapter here sends the stop first.
#[test]
fn a_late_answer_to_a_step_or_a_resume_keeps_the_next_stop() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let stop = |reason: &str, description: &str| {
        json!({"event": "stopped", "body": {"reason": reason, "threadId": 1,
            "description": description}})
    };
    let mut script = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    script.extend([
        json!({"ignore": ["threads", "stackTrace", "scopes", "variables"]}),
        stop("breakpoint", "first"),
        json!({"expect": "next", "arguments": {"threadId": 1}}),
        stop("step", "stepped"),
        json!({"respond": null, "to": "next"}),
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        stop("breakpoint", "resumed"),
        json!({"respond": {"allThreadsContinued": true}, "to": "continue"}),
    ]);
    let debugger = Debugger::open(root, Provider::with(Value::Array(script)));
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    let stopped_at = |session: &Session| {
        let s = session.snapshot();
        (s.phase, s.stop.and_then(|stop| stop.description))
    };
    wait_until("the first stop", || {
        stopped_at(&session) == (Phase::Suspended, Some("first".to_owned()))
    });
    // The answer's callback has run once the request's future is ready.
    block_on(session.step_over()).unwrap();
    assert_eq!(
        stopped_at(&session),
        (Phase::Suspended, Some("stepped".to_owned()))
    );
    block_on(session.resume()).unwrap();
    assert_eq!(
        stopped_at(&session),
        (Phase::Suspended, Some("resumed".to_owned()))
    );
    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
}

#[test]
fn an_evaluation_the_adapter_does_not_answer_fails_after_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let mut script = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    script.extend([
        json!({"ignore": ["threads", "stackTrace", "scopes", "variables"]}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1}}),
        // Never answered; the one after it is.
        json!({"expect": "evaluate", "arguments": {"expression": "hang()"}}),
        json!({"expect": "evaluate", "arguments": {"expression": "1 + 1"}}),
        json!({"respond": {"result": "2", "variablesReference": 0}}),
    ]);
    let after = Duration::from_millis(150);
    let provider = Arc::new(Provider {
        script: Mutex::new(Some(script)),
        evaluation_timeout: Some(after),
        ..Provider::default()
    });
    let debugger = Debugger::open(root, provider);
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    wait_phase(&session, Phase::Suspended);
    let started = Instant::now();
    let error = block_on(session.evaluate("hang()", EvaluateContext::Repl)).unwrap_err();
    assert!(started.elapsed() >= after);
    assert!(
        matches!(&error, DebugError::Adapter(m) if m.contains("timeout") && m.contains("150 ms")),
        "{error}"
    );
    let value = block_on(session.evaluate("1 + 1", EvaluateContext::Repl)).unwrap();
    assert_eq!(value.value, "2");
    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
}

#[test]
fn an_adapter_crash_ends_the_session_with_its_log() {
    let dir = tempfile::tempdir().unwrap();
    let script = json!([
        {"expect": "initialize"},
        {"stderr": "Traceback (most recent call last): boom\n"},
        {"exit": 7},
    ]);
    let debugger = Debugger::open(dir.path(), Provider::with(script));
    let session = debugger.start(request(dir.path())).unwrap();
    wait_phase(&session, Phase::Ended);
    let error = session.snapshot().error.unwrap();
    assert!(error.contains("exited with code 7"), "{error}");
    assert!(error.contains("boom"), "{error}");
    let state = debugger.state_json();
    assert_eq!(state["sessions"][0]["state"], "ended");
    assert_eq!(state["running"], false);
}

#[test]
fn breakpoints_and_watches_are_stored_and_imported_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".idea")).unwrap();
    std::fs::write(
        root.join(".idea").join("workspace.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="XDebuggerManager">
    <breakpoint-manager>
      <breakpoints>
        <line-breakpoint enabled="true" suspend="THREAD" type="python-line">
          <url>file://$PROJECT_DIR$/main.py</url>
          <line>4</line>
        </line-breakpoint>
        <line-breakpoint enabled="true" type="python-line">
          <url>file://$PROJECT_DIR$/main.py</url>
          <line>9</line>
        </line-breakpoint>
      </breakpoints>
      <default-breakpoints>
        <breakpoint enabled="true" type="python-exception">
          <properties notifyOnTerminate="true" exception="BaseException" />
        </breakpoint>
      </default-breakpoints>
    </breakpoint-manager>
    <watches-manager>
      <configuration name="PythonConfigurationType">
        <watch expression="len(items)" language="Python" />
      </configuration>
    </watches-manager>
  </component>
</project>"#,
    )
    .unwrap();
    let main = root.join("main.py");
    let debugger = Debugger::open(root, Arc::new(Provider::default()));
    let b = debugger.breakpoints();
    assert_eq!(b.line().len(), 2);
    assert_eq!(b.at(&main, 5).unwrap().suspend, SuspendPolicy::Thread);
    assert_eq!(b.exception().len(), 1, "the default one is not doubled");
    assert!(b.exception()[0].default && b.exception()[0].flag("notifyOnTerminate"));
    assert_eq!(debugger.watches("python"), ["len(items)"]);
    // A breakpoint removed after the import stays removed.
    let ten = b.at(&main, 10).unwrap().id;
    assert!(debugger.remove_breakpoint(ten));
    debugger.add_watch("python", "x", None);
    debugger.set_muted(true);
    // Threads & Variables: the expression history (the latest first, no
    // doubles), the frames' share and the closed hint.
    debugger.remember_expression("len(items)");
    debugger.remember_expression(" x + 1 ");
    debugger.remember_expression("len(items)");
    debugger.remember_expression("");
    // Each editor has a history of its own.
    debugger.remember(HistoryKind::Watch, "items[0]");
    debugger.remember(HistoryKind::SetValue, "5");
    // A paused watch stays paused.
    assert!(debugger.set_watch_paused("python", 1, true));
    debugger.set_frames_split(0.25);
    debugger.close_frames_hint();
    debugger.flush();
    let text = std::fs::read_to_string(project::workspace::workspace_file(root)).unwrap();
    assert!(text.contains("imported-from-idea = true"), "{text}");
    drop(debugger);

    let debugger = Debugger::open(root, Arc::new(Provider::default()));
    let b = debugger.breakpoints();
    assert_eq!(b.line().len(), 1);
    assert!(b.at(&main, 10).is_none());
    assert_eq!(debugger.watches("python"), ["len(items)", "x"]);
    assert!(debugger.is_muted());
    assert_eq!(debugger.expression_history(), ["len(items)", "x + 1"]);
    assert_eq!(debugger.history(HistoryKind::Watch), ["items[0]"]);
    assert_eq!(debugger.history(HistoryKind::SetValue), ["5"]);
    assert_eq!(debugger.watches_paused("python"), [false, true]);
    assert!(debugger.remove_all_watches("python"));
    assert!(debugger.watches("python").is_empty());
    assert!(!debugger.remove_all_watches("python"));
    assert_eq!(debugger.frames_split(), Some(0.25));
    assert!(debugger.frames_hint_closed());
    // `.idea` is only read.
    assert!(
        std::fs::read_to_string(root.join(".idea").join("workspace.xml"))
            .unwrap()
            .contains("<line>9</line>")
    );
}

#[test]
fn engine_commands() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let debugger = Debugger::open(root, Arc::new(Provider::default()));
    let mut registry = Registry::new();
    debugger::commands::register(&mut registry, debugger.clone());
    let run = |id: &str, args: Value| block_on(registry.invoke(id, args));
    assert_eq!(
        run(
            "debug.toggle-breakpoint",
            json!({"file": "main.py", "line": 7})
        )
        .unwrap(),
        json!({"added": true})
    );
    assert_eq!(
        run(
            "debug.toggle-breakpoint-enabled",
            json!({"file": "main.py", "line": 7})
        )
        .unwrap(),
        json!({"enabled": false})
    );
    run(
        "debug.edit-breakpoint",
        json!({"file": "main.py", "line": 7, "condition": "x > 1", "suspend": "thread"}),
    )
    .unwrap();
    assert_eq!(
        run(
            "debug.edit-breakpoint",
            json!({"file": "main.py", "line": 7, "suspend": "some"})
        )
        .unwrap_err()
        .code,
        "invalid_args"
    );
    run(
        "debug.add-exception-breakpoint",
        json!({"class": "ValueError"}),
    )
    .unwrap();
    run(
        "debug.new-watch",
        json!({"expression": "x", "key": "python"}),
    )
    .unwrap();
    assert_eq!(
        run("debug.mute-breakpoints", Value::Null).unwrap(),
        json!({"muted": true})
    );
    let state = run("debug.state", Value::Null).unwrap();
    assert_eq!(state["muted"], true);
    assert_eq!(state["breakpoints"][0]["file"], "main.py");
    assert_eq!(state["breakpoints"][0]["enabled"], false);
    assert_eq!(state["breakpoints"][0]["suspend"], "thread");
    assert_eq!(state["breakpoints"][0]["condition"], "x > 1");
    assert_eq!(state["exception_breakpoints"][1]["class"], "ValueError");
    assert_eq!(state["watches"]["python"], json!(["x"]));
    assert_eq!(
        run(
            "debug.remove-watch",
            json!({"expression": "x", "key": "python"})
        )
        .unwrap()["removed"],
        true
    );
    assert_eq!(
        run("debug.step-over", Value::Null).unwrap_err().code,
        "not_found"
    );
    assert!(registry.list().iter().all(|m| m.id.starts_with("debug.")));
}

/// A product with a line breakpoint type that suspends the thread.
struct Typed;

impl DebugAdapterProvider for Typed {
    fn prepare(&self, _: &DebugRequest) -> Result<AdapterLaunch, String> {
        Err("no adapter".into())
    }

    fn line_breakpoint_type(&self) -> Option<debugger::LineBreakpointType> {
        Some(debugger::LineBreakpointType {
            id: "python-line".into(),
            title: "Python Line Breakpoint".into(),
            default_suspend: SuspendPolicy::Thread,
            suspend_thread: true,
        })
    }

    fn stoppable_lines(&self, file: &Path, _: &str) -> Option<Vec<bool>> {
        (file.extension().is_some_and(|e| e == "py")).then(Vec::new)
    }
}

#[test]
fn new_breakpoints_take_the_types_policy_and_follow_their_lines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let debugger = Debugger::open(root, Arc::new(Typed));
    assert!(debugger.provider().line_breakpoint_file(&main));
    assert!(
        !debugger
            .provider()
            .line_breakpoint_file(&root.join("notes.txt"))
    );
    assert_eq!(debugger.default_suspend(), SuspendPolicy::Thread);
    debugger.toggle_breakpoint(&main, 3, false);
    assert_eq!(
        debugger.breakpoints().at(&main, 3).unwrap().suspend,
        SuspendPolicy::Thread
    );
    // Make Default: All, kept with the project.
    debugger.set_default_suspend(SuspendPolicy::All);
    debugger.toggle_breakpoint(&main, 5, false);
    assert_eq!(
        debugger.breakpoints().at(&main, 5).unwrap().suspend,
        SuspendPolicy::All
    );
    // The editor moved one and deleted the other's line; a third one
    // lands on a taken line and goes.
    debugger.toggle_breakpoint(&main, 8, false);
    let ids: Vec<_> = debugger
        .breakpoints()
        .in_file(&main)
        .iter()
        .map(|b| b.id)
        .collect();
    debugger.set_lines(
        &main,
        &[(ids[0], Some(4)), (ids[1], None), (ids[2], Some(4))],
    );
    let lines: Vec<u32> = debugger
        .breakpoints()
        .in_file(&main)
        .iter()
        .map(|b| b.line)
        .collect();
    assert_eq!(lines, [4]);
    debugger.flush();
    let again = Debugger::open(root, Arc::new(Typed));
    assert_eq!(again.default_suspend(), SuspendPolicy::All);
    assert_eq!(
        again.breakpoints().at(&main, 4).unwrap().suspend,
        SuspendPolicy::Thread
    );
}

/// A subprocess of the program comes as a child session (`startDebugging`)
/// over a connection of its own and is a process of the same session: it
/// gets the breakpoints, the session shows it when it stops (its frames,
/// stepping), a stop of the program meanwhile waits until it moves on,
/// and it leaves the session when it ends.
#[test]
fn a_subprocess_is_a_process_of_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let util = root.join("util.py");
    let mut program = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    program[0] = json!({"expect": "initialize",
        "arguments": {"adapterID": "fake", "supportsStartDebuggingRequest": true}});
    program.extend([
        json!({"request": "startDebugging", "arguments": {"request": "attach",
            "configuration": {"name": "Subprocess 7", "subProcessId": 7}}}),
        json!({"expect_response": "startDebugging", "success": true}),
        // A breakpoint added while the subprocess is suspended.
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 2}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 2, "id": 21}]}}),
        // The program stops while the subprocess is shown.
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1,
            "hitBreakpointIds": [21]}}),
        // Shown once the subprocess moved on.
        json!({"expect": "threads"}),
        json!({"respond": {"threads": [{"id": 1, "name": "MainThread"}]}}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": {"stackFrames": [{"id": 100, "name": "<module>", "line": 2,
            "column": 1, "source": {"path": util}}], "totalFrames": 1}}),
    ]);
    let child = vec![
        json!({"expect": "initialize",
            "arguments": {"adapterID": "fake", "supportsStartDebuggingRequest": true}}),
        json!({"respond": {"supportsConfigurationDoneRequest": true,
            "exceptionBreakpointFilters": [{"filter": "uncaught", "label": "Uncaught", "default": true}]}}),
        json!({"expect": "attach", "arguments": {"name": "Subprocess 7", "subProcessId": 7}}),
        json!({"event": "initialized"}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "main.py"},
            "breakpoints": [{"line": 3}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 3, "id": 1}]}}),
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": ["uncaught"]}}),
        json!({"respond": null}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "attach"}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1,
            "hitBreakpointIds": [1]}}),
        json!({"expect": "threads"}),
        json!({"respond": {"threads": [{"id": 1, "name": "MainThread"}]}}),
        // The same frame id as the program's: another connection.
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": {"stackFrames": [{"id": 100, "name": "hello", "line": 3,
            "column": 1, "source": {"path": main}}], "totalFrames": 1}}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
            "breakpoints": [{"line": 2}]}}),
        json!({"respond": {"breakpoints": [{"verified": false, "line": 2, "id": 2}]}}),
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        json!({"respond": {"allThreadsContinued": true}}),
        json!({"event": "terminated"}),
        json!({"expect": "disconnect", "arguments": {"terminateDebuggee": false}}),
        json!({"respond": null}),
    ];
    let provider = Arc::new(Provider {
        script: Mutex::new(Some(program)),
        offers_children: true,
        children: Mutex::new(vec![child]),
        ..Provider::default()
    });
    let debugger = Debugger::open(root, provider);
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();

    wait_until("the subprocess's stop", || {
        let s = session.snapshot();
        assert!(s.error.is_none(), "{:?}", s.error);
        s.phase == Phase::Suspended && s.frames.first().is_some_and(|f| f.name == "hello")
    });
    assert_eq!(session.subprocesses(), 1);
    assert_eq!(debugger.state_json()["sessions"][0]["subprocesses"], 1);
    let three = debugger.breakpoints().at(&main, 3).unwrap().id;
    assert!(debugger.breakpoint_status(three).unwrap().verified);

    // The new breakpoint reaches both processes; the subprocess's answer
    // does not hide the program's.
    debugger.toggle_breakpoint(&util, 2, false);
    let two = debugger.breakpoints().at(&util, 2).unwrap().id;
    wait_until("the program's answer", || {
        debugger.breakpoint_status(two).is_some_and(|s| s.verified)
    });
    std::thread::sleep(Duration::from_millis(100));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.phase, Phase::Suspended);
    assert_eq!(
        snapshot.frames[0].name, "hello",
        "the subprocess stays shown"
    );

    // Resume goes to the subprocess; the program's stop is shown next.
    block_on(session.resume()).unwrap();
    wait_until("the program's stop", || {
        let s = session.snapshot();
        s.phase == Phase::Suspended && s.frames.first().is_some_and(|f| f.name == "<module>")
    });
    wait_until("the subprocess leaves", || session.subprocesses() == 0);
    assert_eq!(session.snapshot().phase, Phase::Suspended);
    assert!(debugger.breakpoint_status(two).unwrap().verified);

    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
}

/// A child session the product does not start gets an error answer; the
/// session goes on.
#[test]
fn a_child_session_the_product_does_not_start_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let program = vec![
        json!({"expect": "initialize",
            "arguments": {"adapterID": "fake", "supportsStartDebuggingRequest": false}}),
        json!({"respond": {"supportsConfigurationDoneRequest": true}}),
        json!({"expect": "launch"}),
        json!({"event": "initialized"}),
        json!({"expect": "setExceptionBreakpoints"}),
        json!({"respond": null}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "launch"}),
        json!({"request": "startDebugging", "arguments": {"request": "attach",
            "configuration": {"subProcessId": 7}}}),
        json!({"expect_response": "startDebugging", "success": false}),
        json!({"event": "exited", "body": {"exitCode": 0}}),
        json!({"event": "terminated"}),
    ];
    let debugger = Debugger::open(root, Provider::with(Value::Array(program)));
    let session = debugger.start(request(root)).unwrap();
    wait_phase(&session, Phase::Ended);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.error, None);
    assert_eq!(snapshot.exit_code, Some(0));
    assert_eq!(session.subprocesses(), 0);
}

/// Waits until the session shows a stop whose top frame has this id.
fn wait_frame(session: &Session, frame: i64) {
    wait_until(&format!("a stop in frame {frame}"), || {
        let s = session.snapshot();
        assert!(s.error.is_none(), "{:?}", s.error);
        s.phase == Phase::Suspended && s.frames.first().is_some_and(|f| f.id == frame)
    });
}

/// The product's preparation of steps goes to the program once, answered
/// before its first step; the session shows the thread a step stopped in
/// (the `stopped` event's), not the first one of the program.
#[test]
fn steps_are_prepared_once_and_show_the_thread_they_stop_in() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let threads =
        json!({"threads": [{"id": 1, "name": "MainThread"}, {"id": 2, "name": "worker"}]});
    let frame = |id: i64, name: &str, line: u32| {
        json!({"stackFrames": [{"id": id, "name": name, "line": line, "column": 1,
            "source": {"path": main}}], "totalFrames": 1})
    };
    let mut script = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    script.extend([
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1,
            "allThreadsStopped": true, "hitBreakpointIds": [11]}}),
        json!({"expect": "threads"}),
        json!({"respond": threads}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": frame(100, "hello", 3)}),
        json!({"expect": "evaluate", "arguments": {"expression": "prepare"}}),
        json!({"respond": {"result": "'installed'", "variablesReference": 0}}),
        json!({"expect": "next", "arguments": {"threadId": 1}}),
        json!({"respond": null}),
        json!({"event": "stopped", "body": {"reason": "step", "threadId": 2,
            "allThreadsStopped": true}}),
        json!({"expect": "threads"}),
        json!({"respond": threads}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 2}}),
        json!({"respond": frame(200, "work", 9)}),
        // Step Into My Code: the product's requests around the step only.
        json!({"expect": "evaluate", "arguments": {"expression": "filter on"}}),
        json!({"respond": {"result": "None", "variablesReference": 0}}),
        json!({"expect": "stepIn", "arguments": {"threadId": 2}}),
        json!({"respond": null}),
        json!({"event": "stopped", "body": {"reason": "step", "threadId": 2,
            "allThreadsStopped": true}}),
        json!({"expect": "evaluate", "arguments": {"expression": "filter off"}}),
        json!({"respond": {"result": "None", "variablesReference": 0}}),
        json!({"expect": "threads"}),
        json!({"respond": threads}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 2}}),
        json!({"respond": frame(201, "work", 10)}),
        json!({"forbid": "evaluate", "for": 100}),
    ]);
    let provider = Arc::new(Provider {
        script: Mutex::new(Some(script)),
        my_code: true,
        prepares: true,
        ..Provider::default()
    });
    let debugger = Debugger::open(root, provider);
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    wait_frame(&session, 100);
    assert_eq!(session.snapshot().selected_thread, Some(1));

    block_on(session.step_over()).unwrap();
    wait_frame(&session, 200);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.threads.len(), 2);
    assert_eq!(snapshot.selected_thread, Some(2));
    assert_eq!(snapshot.stop.unwrap().thread_id, Some(2));

    block_on(session.step_into_my_code()).unwrap();
    wait_frame(&session, 201);
    assert_eq!(session.snapshot().selected_thread, Some(2));
    // The script's end: no other preparation came.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(session.snapshot().error, None);
    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
}

/// While the shown process steps, the stop of another process waits (it is
/// shown when the stepped one resumes), unless it is at a breakpoint: then
/// the session shows it, and the step's stop waits. Each process gets the
/// product's preparation before its own first step.
#[test]
fn a_step_keeps_to_its_process_unless_another_hits_a_breakpoint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let main = root.join("main.py");
    let util = root.join("util.py");
    let thread = json!({"threads": [{"id": 1, "name": "MainThread"}]});
    let frame = |id: i64, name: &str, path: &Path, line: u32| {
        json!({"stackFrames": [{"id": id, "name": name, "line": line, "column": 1,
            "source": {"path": path}}], "totalFrames": 1})
    };
    let util_breakpoints = |lines: &[u32]| {
        let lines: Vec<Value> = lines.iter().map(|l| json!({"line": l})).collect();
        let answers: Vec<Value> = lines
            .iter()
            .map(|l| json!({"verified": true, "line": l["line"]}))
            .collect();
        [
            json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "util.py"},
                "breakpoints": lines}}),
            json!({"respond": {"breakpoints": answers}}),
        ]
    };
    let mut program = start_steps(json!([{"line": 3}]), json!(["uncaught"]));
    program[0] = json!({"expect": "initialize",
        "arguments": {"adapterID": "fake", "supportsStartDebuggingRequest": true}});
    program.extend([
        json!({"request": "startDebugging", "arguments": {"request": "attach",
            "configuration": {"name": "Subprocess 7", "subProcessId": 7}}}),
        json!({"expect_response": "startDebugging", "success": true}),
    ]);
    // Once the subprocess is shown suspended, the program pauses.
    program.extend(util_breakpoints(&[2]));
    program.extend([
        json!({"event": "stopped", "body": {"reason": "pause", "threadId": 1,
            "allThreadsStopped": true}}),
        // Shown once the subprocess resumes, not at its step.
        json!({"expect": "threads"}),
        json!({"respond": thread}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": frame(100, "<module>", &util, 2)}),
        json!({"expect": "evaluate", "arguments": {"expression": "prepare"}}),
        json!({"respond": {"result": "'installed'", "variablesReference": 0}}),
        json!({"expect": "next", "arguments": {"threadId": 1}}),
        json!({"respond": null}),
    ]);
    program.extend(util_breakpoints(&[2, 5]));
    // The step's stop comes once the subprocess's breakpoint stop is shown.
    program.extend(util_breakpoints(&[2, 5, 6]));
    program.extend([
        json!({"event": "stopped", "body": {"reason": "step", "threadId": 1,
            "allThreadsStopped": true}}),
        json!({"expect": "threads"}),
        json!({"respond": thread}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": frame(101, "<module>", &util, 3)}),
    ]);
    let mut child = vec![
        json!({"expect": "initialize",
            "arguments": {"adapterID": "fake", "supportsStartDebuggingRequest": true}}),
        json!({"respond": {"supportsConfigurationDoneRequest": true,
            "exceptionBreakpointFilters": [{"filter": "uncaught", "label": "Uncaught", "default": true}]}}),
        json!({"expect": "attach", "arguments": {"subProcessId": 7}}),
        json!({"event": "initialized"}),
        json!({"expect": "setBreakpoints", "arguments": {"source": {"name": "main.py"},
            "breakpoints": [{"line": 3}]}}),
        json!({"respond": {"breakpoints": [{"verified": true, "line": 3, "id": 1}]}}),
        json!({"expect": "setExceptionBreakpoints", "arguments": {"filters": ["uncaught"]}}),
        json!({"respond": null}),
        json!({"expect": "configurationDone"}),
        json!({"respond": null}),
        json!({"respond": null, "to": "attach"}),
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1,
            "allThreadsStopped": true, "hitBreakpointIds": [1]}}),
        json!({"expect": "threads"}),
        json!({"respond": thread}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": frame(500, "hello", &main, 3)}),
    ];
    child.extend(util_breakpoints(&[2]));
    child.extend([
        json!({"expect": "evaluate", "arguments": {"expression": "prepare"}}),
        json!({"respond": {"result": "'installed'", "variablesReference": 0}}),
        json!({"expect": "next", "arguments": {"threadId": 1}}),
        json!({"respond": null}),
        json!({"event": "stopped", "body": {"reason": "step", "threadId": 1,
            "allThreadsStopped": true}}),
        json!({"expect": "threads"}),
        json!({"respond": thread}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": frame(501, "hello", &main, 4)}),
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        json!({"respond": {"allThreadsContinued": true}}),
    ]);
    // A breakpoint hit while the program steps.
    child.extend(util_breakpoints(&[2, 5]));
    child.extend([
        json!({"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1,
            "allThreadsStopped": true}}),
        json!({"expect": "threads"}),
        json!({"respond": thread}),
        json!({"expect": "stackTrace", "arguments": {"threadId": 1}}),
        json!({"respond": frame(502, "helper", &util, 5)}),
    ]);
    child.extend(util_breakpoints(&[2, 5, 6]));
    child.extend([
        json!({"expect": "continue", "arguments": {"threadId": 1}}),
        json!({"respond": {"allThreadsContinued": true}}),
    ]);
    let provider = Arc::new(Provider {
        script: Mutex::new(Some(program)),
        offers_children: true,
        children: Mutex::new(vec![child]),
        prepares: true,
        ..Provider::default()
    });
    let debugger = Debugger::open(root, provider);
    debugger.toggle_breakpoint(&main, 3, false);
    let session = debugger.start(request(root)).unwrap();
    let wait_answers = |line: u32| {
        wait_until(&format!("both answers for line {line}"), || {
            let id = debugger.breakpoints().at(&util, line).unwrap().id;
            debugger.breakpoint_status(id).is_some_and(|s| s.verified)
        });
        // The stop each process sends after its answer comes meanwhile.
        std::thread::sleep(Duration::from_millis(100));
    };

    wait_frame(&session, 500);
    debugger.toggle_breakpoint(&util, 2, false);
    wait_answers(2);
    // The program's pause waits through the subprocess's step.
    block_on(session.step_over()).unwrap();
    wait_frame(&session, 501);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(session.snapshot().frames[0].id, 501);
    block_on(session.resume()).unwrap();
    wait_frame(&session, 100);

    // The program steps; the subprocess hits a breakpoint meanwhile.
    block_on(session.step_over()).unwrap();
    debugger.toggle_breakpoint(&util, 5, false);
    wait_frame(&session, 502);
    // The program's step stop waits for the subprocess.
    debugger.toggle_breakpoint(&util, 6, false);
    wait_answers(6);
    assert_eq!(session.snapshot().frames[0].id, 502);
    block_on(session.resume()).unwrap();
    wait_frame(&session, 101);
    assert_eq!(session.snapshot().stop.unwrap().reason, "step");

    session.stop().recv_timeout(WAIT).unwrap();
    wait_phase(&session, Phase::Ended);
    assert_eq!(session.snapshot().error, None);
}
