//! Servers' lifecycle and their requests to the client, against the
//! scripted fake server built with the crate (`src/bin/lsp-fake-server.rs`),
//! through the public API.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use code_intel::{
    CodeIntel, DocumentId, Features, IntelEvent, MessageLevel, Point, Rope, ServerId, ServerState,
    ServerStatus, TextEdit,
};
use futures::executor::block_on;
use lsp_client::{LspCodeIntel, ServerCustomizer, ServerSpec, Timeouts};
use process::CommandLine;
use serde_json::{Value, json};

use crate::common::*;

fn open(intel: &dyn CodeIntel, root: &Path) {
    let doc = DocumentId::new(root.join("main.py"));
    intel.did_open(&doc, "python", 1, &Rope::from_str("import os\n"));
}

#[derive(Default)]
struct Recording {
    initialized: Mutex<Vec<ServerStatus>>,
}

impl ServerCustomizer for Recording {
    fn configuration(
        &self,
        section: Option<&str>,
        scope: Option<&Path>,
        settings: &Value,
    ) -> Value {
        match section {
            Some("python") => json!({
                "scope": scope.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()),
            }),
            _ => lsp_client::lookup_section(settings, section),
        }
    }

    fn initialized(&self, status: &ServerStatus) {
        self.initialized.lock().unwrap().push(status.clone());
    }
}

#[test]
fn a_server_starts_with_the_first_document_and_shuts_down() {
    let root = root("start");
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    let script = steps([vec![
        json!({"expect": "initialize", "params": {
            "processId": "$any",
            "clientInfo": {"name": "Pyoxide IDE", "version": "$any"},
            "rootUri": file_uri(&root),
            "workspaceFolders": [{"uri": file_uri(&root), "name": name}],
            "initializationOptions": {"settings": {"lineLength": 100}},
            "capabilities": {
                "general": {"positionEncodings": ["utf-8", "utf-16"]},
                "workspace": {
                    "configuration": true,
                    "applyEdit": true,
                    "didChangeWatchedFiles": {"dynamicRegistration": true, "relativePatternSupport": true},
                    "diagnostics": {"refreshSupport": true},
                },
                "textDocument": {
                    "completion": {"completionItem": {
                        "snippetSupport": true,
                        "labelDetailsSupport": true,
                        "resolveSupport": {"properties": ["documentation"]},
                    }},
                    "hover": {"contentFormat": ["markdown", "plaintext"]},
                    "codeAction": {"resolveSupport": {"properties": ["edit"]}},
                    "rename": {"prepareSupport": true},
                    "diagnostic": {"dynamicRegistration": true},
                    "publishDiagnostics": {"versionSupport": true, "tagSupport": {"valueSet": [1, 2]}},
                },
                "window": {"workDoneProgress": true},
            },
        }}),
        json!({"reply": {
            "capabilities": {"positionEncoding": "utf-8"},
            "serverInfo": {"name": "fake", "version": "1.2.3"},
        }}),
        json!({"expect": "initialized", "params": {}}),
        json!({"log": "ready"}),
        json!({"expect": "shutdown"}),
        json!({"log": "shutdown received"}),
        json!({"reply": null}),
        json!({"expect": "exit"}),
        json!({"exit": 0}),
    ]]);
    let recording = Arc::new(Recording::default());
    let mut spec = spec("fake", &root, script).with_customizer(recording.clone());
    spec.initialization_options = Some(json!({"settings": {"lineLength": 100}}));
    let timeouts = Timeouts {
        shutdown: Duration::from_secs(10),
        exit: Duration::from_secs(5),
        ..QUICK
    };
    let intel = LspCodeIntel::with_timeouts(vec![spec], timeouts);
    let events = Events::of(intel.as_ref());

    assert_eq!(intel.status()[0].state, ServerState::Stopped);
    // Another language: nothing starts.
    intel.did_open(
        &DocumentId::new(root.join("a.txt")),
        "plaintext",
        1,
        &Rope::new(),
    );
    assert_eq!(intel.status()[0].state, ServerState::Stopped);

    open(intel.as_ref(), &root);
    let initializing = events.status("fake", |s| *s == ServerState::Initializing);
    assert_eq!(initializing.name, "fake");
    assert_eq!(initializing.version, None);
    let running = events.running("fake");
    assert_eq!(running.label(), "fake 1.2.3");
    assert_eq!(running.roots, [root.to_path_buf()]);
    events.log("ready");
    assert_eq!(recording.initialized.lock().unwrap().len(), 1);
    assert_eq!(
        recording.initialized.lock().unwrap()[0].version.as_deref(),
        Some("1.2.3")
    );
    assert_eq!(intel.status()[0].state, ServerState::Running);

    // A second document does not start a second process.
    open(intel.as_ref(), &root);
    let started = Instant::now();
    assert_eq!(block_on(intel.shutdown()), Ok(()));
    events.log("shutdown received");
    // It answered and exited: no waiting for the timeouts.
    assert!(started.elapsed() < timeouts.exit, "{:?}", started.elapsed());
    let stopped = &intel.status()[0];
    assert_eq!(stopped.state, ServerState::Stopped);
    // The last name and version stay for the widget.
    assert_eq!(stopped.label(), "fake 1.2.3");
    // After shutdown nothing starts again.
    open(intel.as_ref(), &root);
    assert_eq!(intel.status()[0].state, ServerState::Stopped);
}

#[test]
fn the_server_asks_and_the_client_answers() {
    let root = root("requests");
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    let script = steps([
        handshake(json!({"capabilities": {}})),
        vec![
            json!({"request": "workspace/configuration", "id": 10, "params": {"items": [
                {"section": "fake.analysis"},
                {"section": "python", "scopeUri": file_uri(&root.join("pkg").join("a.py"))},
                {},
            ]}}),
            json!({"expect_response": 10, "result": [
                {"mode": "strict"},
                {"scope": "a.py"},
                {"fake": {"analysis": {"mode": "strict"}}},
            ]}),
            json!({"request": "client/registerCapability", "id": 11, "params": {"registrations": [
                {"id": "w1", "method": "workspace/didChangeWatchedFiles",
                 "registerOptions": {"watchers": [{"globPattern": "**/pyproject.toml"}]}},
            ]}}),
            json!({"expect_response": 11, "result": null}),
            json!({"request": "client/unregisterCapability", "id": 12, "params": {"unregisterations": [
                {"id": "w1", "method": "workspace/didChangeWatchedFiles"},
            ]}}),
            json!({"expect_response": 12, "result": null}),
            json!({"request": "window/workDoneProgress/create", "id": 13, "params": {"token": "t1"}}),
            json!({"expect_response": 13, "result": null}),
            json!({"notify": "$/progress", "params": {"token": "t1", "value": {"kind": "begin", "title": "Indexing", "percentage": 0}}}),
            json!({"notify": "$/progress", "params": {"token": "t1", "value": {"kind": "report", "message": "3/10", "percentage": 30}}}),
            json!({"notify": "$/progress", "params": {"token": "t1", "value": {"kind": "end"}}}),
            json!({"notify": "$/progress", "params": {"token": "unknown", "value": {"kind": "report", "message": "x"}}}),
            json!({"notify": "window/showMessage", "params": {"type": 2, "message": "careful"}}),
            json!({"request": "workspace/diagnostic/refresh", "id": 14}),
            json!({"expect_response": 14, "result": null}),
            json!({"request": "workspace/workspaceFolders", "id": 15}),
            json!({"expect_response": 15, "result": [{"uri": file_uri(&root), "name": name}]}),
            json!({"request": "window/showMessageRequest", "id": "s16", "params": {"type": 1, "message": "pick", "actions": [{"title": "A"}]}}),
            json!({"expect_response": "s16", "result": null}),
            json!({"request": "workspace/semanticTokens/refresh", "id": 17}),
            json!({"expect_response": 17, "result": null}),
            json!({"request": "fake/unknown", "id": 18, "params": {}}),
            json!({"expect_response": 18, "error": {"code": -32601}}),
            json!({"notify": "telemetry/event", "params": {"x": 1}}),
            json!({"log": "requests done"}),
        ],
    ]);
    let spec = spec("fake", &root, script)
        .with_settings(json!({"fake": {"analysis": {"mode": "strict"}}}))
        .with_customizer(Arc::new(Recording::default()));
    let intel = LspCodeIntel::with_timeouts(vec![spec], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("fake");

    let mut progress = Vec::new();
    let mut messages = Vec::new();
    events.next("the script's end", |event| match event {
        IntelEvent::Progress {
            server,
            title,
            message,
            percentage,
            done,
        } => {
            assert_eq!(*server, ServerId::new("fake"));
            progress.push((title.clone(), message.clone(), *percentage, *done));
            None
        }
        IntelEvent::Message { level, text, .. } if text == "requests done" => {
            assert_eq!(*level, MessageLevel::Log);
            Some(())
        }
        IntelEvent::Message { level, text, .. } => {
            messages.push((*level, text.clone()));
            None
        }
        _ => None,
    });
    assert_eq!(
        progress,
        [
            ("Indexing".to_owned(), None, Some(0), false),
            (
                "Indexing".to_owned(),
                Some("3/10".to_owned()),
                Some(30),
                false
            ),
            (
                "Indexing".to_owned(),
                Some("3/10".to_owned()),
                Some(30),
                true
            ),
        ]
    );
    assert_eq!(
        messages,
        [
            (MessageLevel::Warning, "careful".to_owned()),
            (MessageLevel::Error, "pick".to_owned()),
        ]
    );
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn edits_the_server_asks_for_go_to_the_application() {
    let root = root("apply-edit");
    let file = root.join("b.py");
    std::fs::write(&file, "x = \"🦀\"\r\ny = 2\r\n").unwrap();
    let change = |id: u32| {
        json!({"request": "workspace/applyEdit", "id": id, "params": {
            "label": "Fix it",
            "edit": {"changes": {file_uri(&file): [
                {"range": {"start": {"line": 0, "character": 5}, "end": {"line": 0, "character": 7}}, "newText": "z"},
                {"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 0}}, "newText": "# "},
            ]}},
        }})
    };
    let script = steps([
        handshake(json!({"capabilities": {"positionEncoding": "utf-16"}})),
        vec![
            change(20),
            json!({"expect_response": 20, "result": {"applied": true}}),
            change(21),
            json!({"expect_response": 21, "result": {"applied": false}}),
            json!({"request": "workspace/applyEdit", "id": 22, "params": {"edit": {"documentChanges": [
                {"kind": "create", "uri": file_uri(&root.join("new.py"))},
            ]}}}),
            json!({"expect_response": 22, "result": {"applied": false, "failureReason": "$any"}}),
            json!({"log": "edits done"}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("fake");
    for applied in [true, false] {
        events.next("an edit", |event| match event {
            IntelEvent::ApplyEdit {
                server,
                label,
                edit,
                reply,
            } => {
                assert_eq!(*server, ServerId::new("fake"));
                assert_eq!(label.as_deref(), Some("Fix it"));
                assert_eq!(edit.files.len(), 1);
                assert_eq!(edit.files[0].path, file);
                // UTF-16 columns 5..7 are the crab: one char.
                assert_eq!(
                    edit.files[0].edits,
                    [
                        TextEdit::new(Point::new(0, 5)..Point::new(0, 6), "z"),
                        TextEdit::insert(Point::new(1, 0), "# "),
                    ]
                );
                reply.send(applied);
                Some(())
            }
            _ => None,
        });
    }
    events.log("edits done");
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn a_crash_terminates_with_the_error_output_and_no_restart() {
    let root = root("crash");
    let script = steps([
        handshake(json!({"capabilities": {}})),
        vec![
            json!({"stderr": "thread 'main' panicked: something broke\n"}),
            json!({"delay": 50}),
            json!({"exit": 3}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("fake");
    let error = events.terminated("fake");
    assert!(error.contains("panicked: something broke"), "{error}");
    assert!(
        error.ends_with("Process finished with exit code 3"),
        "{error}"
    );
    // Opening another document does not restart it.
    open(intel.as_ref(), &root);
    std::thread::sleep(Duration::from_millis(200));
    assert!(matches!(
        intel.status()[0].state,
        ServerState::Terminated { .. }
    ));
    // An explicit restart does.
    intel.restart(Some(&ServerId::new("fake")));
    events.status("fake", |s| *s == ServerState::Initializing);
    events.running("fake");
    events.terminated("fake");
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn servers_that_cannot_start_are_terminated() {
    let root = root("cannot-start");
    let missing = ServerSpec::new(
        "missing",
        CommandLine::new(root.join("no-such-server")),
        &*root,
    )
    .with_languages(["python"]);
    let failing = spec(
        "failing",
        &root,
        vec![
            json!({"expect": "initialize"}),
            json!({"reply_error": {"code": -32603, "message": "no workspace"}}),
            json!({"hang": true}),
        ],
    );
    let silent = spec(
        "silent",
        &root,
        vec![json!({"expect": "initialize"}), json!({"hang": true})],
    );
    let intel = LspCodeIntel::with_timeouts(vec![missing, failing, silent], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    // Each in its own time, in any order.
    let errors = events.terminated_all(&["missing", "failing", "silent"]);
    let error = &errors["missing"];
    assert!(error.starts_with("Cannot run program"), "{error}");
    let error = &errors["failing"];
    assert!(
        error.contains("failed to initialize: no workspace"),
        "{error}"
    );
    let error = &errors["silent"];
    assert!(
        error.contains("did not answer initialize in 1.5 s"),
        "{error}"
    );
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn a_server_that_ignores_shutdown_is_stopped_anyway() {
    let root = root("hung");
    let script = steps([
        handshake(json!({"capabilities": {}})),
        vec![json!({"hang": true})],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("fake");
    let started = Instant::now();
    assert_eq!(block_on(intel.shutdown()), Ok(()));
    let took = started.elapsed();
    // `shutdown` unanswered, then `exit` and end of input ignored: the
    // process was interrupted after the exit timeout.
    assert!(took >= QUICK.shutdown + QUICK.exit, "{took:?}");
    assert!(took < WAIT, "{took:?}");
    assert_eq!(intel.status()[0].state, ServerState::Stopped);
}

#[test]
fn a_stopped_server_starts_again_when_needed() {
    let root = root("stop");
    let script = steps([
        handshake(json!({"capabilities": {}})),
        vec![json!({"log": "up"})],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("fake");
    events.log("up");
    intel.stop(&ServerId::new("fake"));
    events.status("fake", |s| *s == ServerState::Stopped);
    open(intel.as_ref(), &root);
    events.status("fake", |s| *s == ServerState::Initializing);
    events.running("fake");
    events.log("up");
    // Restart while running: stopped and started.
    intel.restart(None);
    events.status("fake", |s| *s == ServerState::Stopped);
    events.status("fake", |s| *s == ServerState::Initializing);
    events.running("fake");
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn malformed_output_of_a_real_process_is_survived() {
    let root = root("malformed");
    let message = json!({"jsonrpc": "2.0", "method": "window/logMessage",
        "params": {"type": 4, "message": "after the noise é🦀"}});
    let body = serde_json::to_vec(&message).unwrap();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    // Cut inside the header and inside the crab.
    let cut = body.len() - 4;
    let script = steps([
        handshake(json!({"capabilities": {}})),
        vec![
            json!({"raw": ["some debug print\n", "Content-Length: 3\r\n\r\n{x}", "Foo: bar\r\n\r\n"]}),
            json!({"raw": [&header[..7], &header[7..], body[..cut].to_vec(), body[cut..].to_vec()], "pause": 30}),
            json!({"delay": 100}),
            json!({"stderr": "the end\n"}),
            json!({"exit": 1}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("fake");
    events.log("after the noise é🦀");
    let error = events.terminated("fake");
    assert!(
        error.contains("Invalid server output: a message that is not JSON"),
        "{error}"
    );
    assert!(
        error.contains("Invalid server output: a message header without Content-Length: Foo: bar"),
        "{error}"
    );
    assert!(
        error.contains("the end\nProcess finished with exit code 1"),
        "{error}"
    );
}

#[test]
fn specs_are_replaced_while_documents_are_open() {
    let root = root("set-specs");
    let script = || {
        steps([
            handshake(json!({"capabilities": {}})),
            vec![json!({"log": "up"})],
        ])
    };
    let intel = LspCodeIntel::with_timeouts(Vec::new(), QUICK);
    let events = Events::of(intel.as_ref());
    assert!(intel.status().is_empty());
    assert!(intel.specs().is_empty());
    // No servers yet: opening a document starts nothing.
    open(intel.as_ref(), &root);

    // The product found its servers: they start for the open document.
    intel.set_specs(vec![spec("a", &root, script())]);
    events.status("a", |s| *s == ServerState::Initializing);
    events.running("a");
    events.log("up");

    // Only the name and the features changed: it keeps running.
    let mut renamed = spec("a", &root, script()).with_features(Features::NONE);
    renamed.name = "renamed".into();
    intel.set_specs(vec![renamed]);
    let status = events.status_where("a", |s| s.name == "renamed");
    assert_eq!(status.state, ServerState::Running);
    assert!(status.features.is_empty());
    assert_eq!(intel.specs()[0].name, "renamed");

    // New settings restart it; a new server starts next to it.
    let resettled = spec("a", &root, script()).with_settings(json!({"x": 1}));
    intel.set_specs(vec![resettled, spec("b", &root, script())]);
    events.status("a", |s| *s == ServerState::Stopped);
    // The two start on threads of their own: either may run first.
    events.running_all(&["a", "b"]);
    let ids: Vec<String> = intel.status().iter().map(|s| s.id.to_string()).collect();
    assert_eq!(ids, ["a", "b"]);

    // Gone from the specs: stopped and no longer listed.
    intel.set_specs(vec![spec("b", &root, script())]);
    events.status("a", |s| *s == ServerState::Stopped);
    assert_eq!(intel.status().len(), 1);
    assert_eq!(intel.status()[0].state, ServerState::Running);

    // With the document closed a new server does not start.
    intel.did_close(&DocumentId::new(root.join("main.py")));
    intel.set_specs(vec![spec("c", &root, script())]);
    events.status("b", |s| *s == ServerState::Stopped);
    assert_eq!(intel.status()[0].id, ServerId::new("c"));
    assert_eq!(intel.status()[0].state, ServerState::Stopped);
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn an_empty_configuration_change_once_running() {
    let root = root("empty-configuration");
    let script = steps([
        handshake(json!({"capabilities": {}})),
        vec![
            json!({"expect": "workspace/didChangeConfiguration", "params": {"settings": {}}}),
            json!({"request": "workspace/configuration", "id": 1, "params": {"items": [
                {"section": "python"},
                {"section": "basedpyright.analysis", "scopeUri": file_uri(&root)},
            ]}}),
            json!({"expect_response": 1, "result": [
                {"pythonPath": "/venv/bin/python"},
                {"typeCheckingMode": "standard"},
            ]}),
            json!({"log": "configured"}),
        ],
    ]);
    let spec = spec("basedpyright", &root, script)
        .with_empty_configuration_change()
        .with_settings(json!({
            "python": {"pythonPath": "/venv/bin/python"},
            "basedpyright": {"analysis": {"typeCheckingMode": "standard"}},
        }));
    let intel = LspCodeIntel::with_timeouts(vec![spec], QUICK);
    let events = Events::of(intel.as_ref());
    open(intel.as_ref(), &root);
    events.running("basedpyright");
    events.log("configured");
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}
