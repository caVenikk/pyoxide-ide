//! Document sync, watched files and diagnostics against the scripted fake
//! server (`src/bin/lsp-fake-server.rs`), through the public API.

use code_intel::{CodeIntel, Feature, FileChange, FileChangeKind, Point, ServerState, Severity};
use futures::executor::block_on;
use lsp_client::LspCodeIntel;
use serde_json::{Value, json};
use text::{Edit, EditKind};

use crate::common::*;

/// A server that keeps the documents as a server does and publishes their
/// text back after each change.
fn mirror(id: &str, root: &std::path::Path, encoding: &str, change: u32) -> lsp_client::ServerSpec {
    spec(
        id,
        root,
        steps([
            handshake(json!({"capabilities": {
                "positionEncoding": encoding,
                "textDocumentSync": {"openClose": true, "change": change},
            }})),
            vec![json!({"mirror": encoding})],
        ]),
    )
}

/// Waits until every mirror server's text of the document, as of its
/// current version, is the buffer's.
fn mirrors_agree(events: &Events, editor: &Editor, servers: usize) {
    let text = editor.buffer.text();
    events.diagnostics(&editor.doc, editor.version, |items| {
        items.len() == servers && items.iter().all(|d| d.message == text)
    });
}

/// Random edits, several carets at once, undo and redo of grouped typing,
/// emoji, CJK, tabs and line breaks: every server's copy of the text stays
/// equal to the buffer, with UTF-8 and UTF-16 positions and full sync.
#[test]
fn random_edits_keep_the_servers_copies_equal_to_the_buffer() {
    let root = root("mirror");
    let intel = LspCodeIntel::with_timeouts(
        vec![
            mirror("utf-8", &root, "utf-8", 2),
            mirror("utf-16", &root, "utf-16", 2),
            mirror("full", &root, "utf-16", 1),
        ],
        PATIENT,
    );
    let events = Events::of(intel.as_ref());
    let mut editor = Editor::open(
        intel.as_ref(),
        root.join("m.py"),
        "def f():\n\treturn '中文'  # é\n",
    );
    mirrors_agree(&events, &editor, 3);

    const PIECES: &[&str] = &[
        "a", "xyz", "🦀", "中文", "\t", "\n", "é", "", "x\ny", "😀😀", "\r\n", "\u{0C}", "  ",
    ];
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for step in 0..400 {
        match rng.below(12) {
            0 => {
                if let Some((applied, _)) = editor.buffer.undo() {
                    editor.report(intel.as_ref(), &applied);
                }
            }
            1 => {
                if let Some((applied, _)) = editor.buffer.redo() {
                    editor.report(intel.as_ref(), &applied);
                }
            }
            _ => {
                let len = editor.buffer.len_chars();
                let carets = rng.below(4) + 1;
                let mut points: Vec<usize> = (0..carets * 2).map(|_| rng.below(len + 1)).collect();
                points.sort_unstable();
                let edits = points
                    .chunks(2)
                    .map(|pair| {
                        // Mostly insertions, as typing at several carets.
                        let end = if rng.below(3) == 0 { pair[1] } else { pair[0] };
                        Edit::new(pair[0]..end, *rng.pick(PIECES))
                    })
                    .collect();
                let kind = if rng.below(2) == 0 {
                    EditKind::Typing
                } else {
                    EditKind::Other
                };
                editor.edit_as(intel.as_ref(), edits, kind);
            }
        }
        if step % 100 == 99 {
            mirrors_agree(&events, &editor, 3);
        }
    }

    // A reload replaces the whole text everywhere.
    editor.version += 1;
    let reloaded = text::Rope::from_str("print('reloaded')\n");
    editor.buffer = text::Buffer::new("print('reloaded')\n");
    intel.did_reload(&editor.doc, editor.version, &reloaded);
    mirrors_agree(&events, &editor, 3);

    // Closed and opened again: a new copy from version 1.
    intel.did_close(&editor.doc);
    let mut editor = Editor::open(intel.as_ref(), root.join("m.py"), "x = 1\n");
    mirrors_agree(&events, &editor, 3);
    editor.edit(intel.as_ref(), vec![Edit::insert(0, "🦀")]);
    mirrors_agree(&events, &editor, 3);
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn documents_open_on_the_servers_of_their_language() {
    let root = root("open-close");
    let doc_uri = file_uri(&root.join("a.py"));
    let script = steps([
        handshake(
            json!({"capabilities": {"textDocumentSync": {"openClose": true, "change": 2, "save": {"includeText": true}}}}),
        ),
        vec![
            // The document opened before the server ran.
            json!({"expect": "textDocument/didOpen", "params": {"textDocument": {
                "uri": doc_uri, "languageId": "python", "version": 1, "text": "x = 1\n",
            }}}),
            json!({"expect": "textDocument/didChange", "params": {
                "textDocument": {"uri": doc_uri, "version": 2},
                "contentChanges": [{"range": r(0, 4, 0, 5), "text": "2"}],
            }}),
            json!({"expect": "textDocument/didSave", "params": {"textDocument": {"uri": doc_uri}, "text": "x = 2\n"}}),
            json!({"expect": "textDocument/didClose", "params": {"textDocument": {"uri": doc_uri}}}),
            json!({"log": "closed"}),
        ],
    ]);
    let other = spec(
        "other",
        &root,
        steps([handshake(json!({"capabilities": {}}))]),
    )
    .with_languages(["toml"]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script), other], PATIENT);
    let events = Events::of(intel.as_ref());
    let mut editor = Editor::open(intel.as_ref(), root.join("a.py"), "x = 1\n");
    events.running("fake");
    let with_document = events.status_where("fake", |s| !s.documents.is_empty());
    assert_eq!(with_document.documents, [editor.doc.clone()]);
    let at = editor.offset("1");
    editor.edit(intel.as_ref(), vec![Edit::new(at..at + 1, "2")]);
    intel.did_save(&editor.doc, editor.buffer.rope());
    intel.did_close(&editor.doc);
    events.status_where("fake", |s| s.documents.is_empty());
    events.log("closed");
    // The server of another language never started.
    assert_eq!(intel.status()[1].state, ServerState::Stopped);
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn watched_files_go_to_the_servers_that_watch_them() {
    let root = root("watched");
    let watchers = json!({"watchers": [
        {"globPattern": "**/pyproject.toml"},
        {"globPattern": {"baseUri": file_uri(&root), "pattern": "*.cfg"}, "kind": 1},
    ]});
    let script = steps([
        handshake(json!({"capabilities": {"textDocumentSync": 1}})),
        vec![
            json!({"request": "client/registerCapability", "id": 1, "params": {"registrations": [
                {"id": "w", "method": "workspace/didChangeWatchedFiles", "registerOptions": watchers},
            ]}}),
            json!({"expect_response": 1, "result": null}),
            json!({"log": "watching"}),
            json!({"expect": "workspace/didChangeWatchedFiles", "params": {"changes": [
                {"uri": file_uri(&root.join("sub").join("pyproject.toml")), "type": 2},
                {"uri": file_uri(&root.join("setup.cfg")), "type": 1},
            ]}}),
            // Saving a document: no watcher matches `*.py`.
            json!({"forbid": "workspace/didChangeWatchedFiles", "for": 300}),
            json!({"log": "done"}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], PATIENT);
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), "pass\n");
    events.log("watching");
    intel.files_changed(&[
        FileChange::new(
            root.join("sub").join("pyproject.toml"),
            FileChangeKind::Changed,
        ),
        FileChange::new(root.join("setup.cfg"), FileChangeKind::Created),
        // Only creations of `*.cfg`, and not below the base.
        FileChange::new(root.join("setup.cfg"), FileChangeKind::Deleted),
        FileChange::new(root.join("sub").join("x.cfg"), FileChangeKind::Created),
        FileChange::new(root.join("README.md"), FileChangeKind::Changed),
    ]);
    intel.did_save(&editor.doc, editor.buffer.rope());
    events.log("done");
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn pushed_diagnostics_are_placed_in_the_current_text() {
    let root = root("push");
    let uri = file_uri(&root.join("a.py"));
    let diagnostic = |range: Value, message: &str| json!({"range": range, "message": message, "severity": 1, "source": "fake", "code": "E1"});
    let script = steps([
        handshake(json!({"capabilities": {"textDocumentSync": 2}})),
        vec![
            json!({"expect": "textDocument/didOpen"}),
            json!({"expect": "textDocument/didChange", "params": {"textDocument": {"version": 2}}}),
            // For version 1: moved through the edit that made version 2.
            json!({"notify": "textDocument/publishDiagnostics", "params": {
                "uri": uri, "version": 1, "diagnostics": [diagnostic(r(0, 0, 0, 3), "old")],
            }}),
            // An unknown version: dropped.
            json!({"notify": "textDocument/publishDiagnostics", "params": {
                "uri": uri, "version": 77, "diagnostics": [diagnostic(r(0, 0, 0, 1), "stale")],
            }}),
            // A file that is not open: dropped.
            json!({"notify": "textDocument/publishDiagnostics", "params": {
                "uri": file_uri(&root.join("other.py")), "diagnostics": [diagnostic(r(0, 0, 0, 1), "other")],
            }}),
            // Without a version: the last text sent.
            json!({"notify": "textDocument/publishDiagnostics", "params": {
                "uri": uri, "diagnostics": [diagnostic(r(1, 0, 1, 3), "current")],
            }}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], PATIENT);
    let events = Events::of(intel.as_ref());
    let mut editor = Editor::open(intel.as_ref(), root.join("a.py"), "foo()\n");
    // Open on the server before the edit.
    events.status_where("fake", |s| !s.documents.is_empty());
    editor.edit(intel.as_ref(), vec![Edit::insert(0, "# x\n")]);
    let items = events.diagnostics(&editor.doc, 2, |items| !items.is_empty());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].message, "old");
    assert_eq!(items[0].range, Point::new(1, 0)..Point::new(1, 3));
    assert_eq!(items[0].severity, Severity::Error);
    assert_eq!(items[0].summary(), "old  fake(E1)");
    let items = events.diagnostics(&editor.doc, 2, |items| {
        items.iter().any(|d| d.message == "current")
    });
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].range, Point::new(1, 0)..Point::new(1, 3));

    // Stopping the server takes its diagnostics away.
    intel.stop(&code_intel::ServerId::new("fake"));
    let items = events.diagnostics(&editor.doc, 2, |_| true);
    assert!(items.is_empty());
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn diagnostics_are_pulled_after_opening_after_edits_and_on_refresh() {
    let root = root("pull");
    let uri = file_uri(&root.join("a.py"));
    let report = |id: &str, message: &str| {
        json!({"kind": "full", "resultId": id, "items": [
            {"range": r(0, 0, 0, 3), "message": message, "severity": 2},
        ]})
    };
    let script = steps([
        handshake(json!({"capabilities": {
            "textDocumentSync": 2,
            "diagnosticProvider": {"identifier": "fake", "interFileDependencies": false, "workspaceDiagnostics": false},
        }})),
        vec![
            json!({"expect": "textDocument/didOpen"}),
            json!({"expect": "textDocument/diagnostic", "params": {
                "textDocument": {"uri": uri}, "identifier": "fake", "previousResultId": "$absent",
            }}),
            json!({"log": "pulled once"}),
            // Edits while the pull runs: its answer is for version 1.
            json!({"expect": "textDocument/didChange", "params": {"textDocument": {"version": 2}}}),
            json!({"reply": report("r1", "first"), "to": "textDocument/diagnostic"}),
            // Two quick edits: one pull, 300 ms after the last.
            json!({"expect": "textDocument/didChange", "params": {"textDocument": {"version": 3}}}),
            json!({"expect": "textDocument/didChange", "params": {"textDocument": {"version": 4}}}),
            json!({"expect": "textDocument/diagnostic", "params": {"previousResultId": "r1"}}),
            json!({"forbid": "textDocument/diagnostic", "for": 400}),
            json!({"reply": {"kind": "unchanged", "resultId": "r2"}}),
            // The server asks for fresh results: pulled again, in full.
            json!({"request": "workspace/diagnostic/refresh", "id": 9}),
            json!({"expect_response": 9, "result": null}),
            json!({"expect": "textDocument/diagnostic", "params": {"previousResultId": "$absent"}}),
            json!({"reply": report("r3", "refreshed")}),
            // A new pull registration counts as a refresh.
            json!({"request": "client/registerCapability", "id": 10, "params": {"registrations": [
                {"id": "d", "method": "textDocument/diagnostic", "registerOptions": {"identifier": "again"}},
            ]}}),
            json!({"expect_response": 10, "result": null}),
            json!({"expect": "textDocument/diagnostic", "params": {"identifier": "again"}}),
            json!({"reply": {"kind": "full", "items": []}}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], PATIENT);
    let events = Events::of(intel.as_ref());
    let mut editor = Editor::open(intel.as_ref(), root.join("a.py"), "foo()\n");
    events.log("pulled once");
    editor.edit(intel.as_ref(), vec![Edit::insert(0, "# x\n")]);
    // Moved through the edit made while the server worked.
    let items = events.diagnostics(&editor.doc, 2, |items| !items.is_empty());
    assert_eq!(items[0].message, "first");
    assert_eq!(items[0].range, Point::new(1, 0)..Point::new(1, 3));
    editor.edit(intel.as_ref(), vec![Edit::insert(0, "a")]);
    editor.edit(intel.as_ref(), vec![Edit::insert(0, "b")]);
    let items = events.diagnostics(&editor.doc, 4, |items| {
        items.iter().any(|d| d.message == "refreshed")
    });
    assert_eq!(items[0].range, Point::new(0, 0)..Point::new(0, 3));
    events.diagnostics(&editor.doc, 4, <[_]>::is_empty);
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn diagnostics_of_several_servers_arrive_together() {
    let root = root("two-servers");
    let uri = file_uri(&root.join("a.py"));
    let server = |id: &str, message: &str, delay: u64| {
        spec(
            id,
            &root,
            steps([
                handshake(json!({"capabilities": {"textDocumentSync": 2}})),
                vec![
                    json!({"expect": "textDocument/didOpen"}),
                    json!({"delay": delay}),
                    json!({"notify": "textDocument/publishDiagnostics", "params": {
                        "uri": uri, "version": 1,
                        "diagnostics": [{"range": r(0, 0, 0, 3), "message": message, "tags": [1], "severity": 4}],
                    }}),
                ],
            ]),
        )
    };
    let without_diagnostics = server("quiet", "never shown", 0)
        .with_features(code_intel::Features::all().without(Feature::Diagnostics));
    let intel = LspCodeIntel::with_timeouts(
        vec![
            server("ruff", "from ruff", 0),
            server("ty", "from ty", 200),
            without_diagnostics,
        ],
        PATIENT,
    );
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), "foo()\n");
    let items = events.diagnostics(&editor.doc, 1, |items| items.len() == 2);
    let mut messages: Vec<&str> = items.iter().map(|d| d.message.as_str()).collect();
    messages.sort_unstable();
    assert_eq!(messages, ["from ruff", "from ty"]);
    // Equal ranges stay separate items, each tagged with its server.
    assert!(
        items
            .iter()
            .all(|d| d.range == (Point::new(0, 0)..Point::new(0, 3)))
    );
    assert!(
        items
            .iter()
            .all(|d| d.unnecessary && d.severity == Severity::Information)
    );
    assert!(items.iter().all(|d| d.server.is_some()));
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}
