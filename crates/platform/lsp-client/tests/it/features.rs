//! The language features over several servers, against the scripted fake
//! server: routing by the specs' features and the servers' capabilities,
//! conversion to the model, merging, cancellation and stale answers.

use std::ops::Range;
use std::sync::Arc;

use code_intel::{
    CodeAction, CodeActionContext, CodeActionKind, CodeIntel, CompletionTrigger, DocumentId,
    Feature, Features, FormatOptions, InsertText, IntelError, IntelEvent, Location, MarkupKind,
    Point, Rope, ServerId, SignatureContext, SignatureTrigger, TextEdit,
};
use futures::FutureExt;
use futures::executor::block_on;
use lsp_client::{LspCodeIntel, ServerCustomizer};
use serde_json::json;
use text::Edit;

use crate::common::*;

const TEXT: &str = "import os\nos.\nprint(x)\n";

#[test]
fn completion_with_triggers_incomplete_lists_and_resolve() {
    let root = root("completion");
    let script = steps([
        handshake(json!({"capabilities": {
            "textDocumentSync": 2,
            "completionProvider": {"triggerCharacters": [".", "("], "resolveProvider": true},
        }})),
        vec![
            json!({"expect": "textDocument/completion", "params": {
                "position": {"line": 1, "character": 3},
                "context": {"triggerKind": 2, "triggerCharacter": "."},
            }}),
            json!({"reply": {"isIncomplete": true, "items": [
                {"label": "path", "kind": 9, "data": {"id": 1},
                 "textEdit": {"range": r(1, 3, 1, 3), "newText": "path"}},
            ]}}),
            json!({"expect": "completionItem/resolve", "params": {"label": "path", "data": {"id": 1}}}),
            json!({"reply": {"label": "path", "kind": 9, "data": {"id": 1},
                "documentation": {"kind": "markdown", "value": "Common pathname manipulations."}}}),
            // A character that is not one of the server's: invoked.
            json!({"expect": "textDocument/completion", "params": {"context": {"triggerKind": 1}}}),
            json!({"reply": [{"label": "print"}]}),
            json!({"expect": "textDocument/completion", "params": {"context": {"triggerKind": 3}}}),
            json!({"reply": null}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("ty", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), TEXT);
    events.opened_all(&["ty"]);
    assert_eq!(intel.completion_triggers(&editor.doc), ['.', '(']);

    let at = Point::new(1, 3);
    let answer =
        block_on(intel.completion(&editor.doc, at, CompletionTrigger::Character('.'))).unwrap();
    assert!(answer.incomplete);
    let path = &answer.items[0];
    assert_eq!(path.label, "path");
    assert_eq!(path.server, Some(ServerId::new("ty")));
    assert_eq!(
        path.insert,
        InsertText::Edit {
            insert: at..at,
            replace: at..at,
            text: "path".into()
        }
    );
    assert!(path.documentation.is_none());
    let resolved = block_on(intel.resolve_completion(&editor.doc, path.clone())).unwrap();
    let docs = resolved.documentation.unwrap();
    assert_eq!(docs.kind, MarkupKind::Markdown);
    assert_eq!(docs.text, "Common pathname manipulations.");

    let answer =
        block_on(intel.completion(&editor.doc, at, CompletionTrigger::Character('x'))).unwrap();
    assert_eq!(answer.items[0].label, "print");
    assert!(!answer.incomplete);
    let answer =
        block_on(intel.completion(&editor.doc, at, CompletionTrigger::Incomplete)).unwrap();
    assert!(answer.items.is_empty());
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

/// A product that shows its own text for the server's items and drops
/// some.
struct Presenting;

impl ServerCustomizer for Presenting {
    fn completion_items(&self, doc: &DocumentId, items: &mut Vec<code_intel::CompletionItem>) {
        assert!(doc.path().ends_with("a.py"));
        items.retain(|i| i.label != "hidden");
        for item in items.iter_mut() {
            item.type_text = item.detail.as_ref().map(|d| format!("<{d}>"));
        }
    }
}

#[test]
fn completion_items_as_the_product_presents_them() {
    let root = root("completion-customized");
    let script = steps([
        handshake(json!({"capabilities": {
            "textDocumentSync": 2,
            "completionProvider": {"resolveProvider": true},
        }})),
        vec![
            json!({"expect": "textDocument/completion"}),
            json!({"reply": [
                {"label": "path", "detail": "module", "data": 1},
                {"label": "hidden"},
            ]}),
            json!({"expect": "completionItem/resolve"}),
            json!({"reply": {"label": "path", "detail": "module os.path", "data": 1}}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(
        vec![spec("ty", &root, script).with_customizer(Arc::new(Presenting))],
        QUICK,
    );
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), TEXT);
    events.opened_all(&["ty"]);
    let answer =
        block_on(intel.completion(&editor.doc, Point::new(1, 3), CompletionTrigger::Invoked))
            .unwrap();
    assert_eq!(answer.items.len(), 1);
    assert_eq!(answer.items[0].type_text.as_deref(), Some("<module>"));
    let resolved =
        block_on(intel.resolve_completion(&editor.doc, answer.items[0].clone())).unwrap();
    assert_eq!(resolved.type_text.as_deref(), Some("<module os.path>"));
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn documentation_signatures_and_navigation() {
    let root = root("navigation");
    let other = root.join("other.py");
    std::fs::write(&other, "x = \"🦀\"\r\ndef helper(): pass\r\n").unwrap();
    let other_uri = file_uri(&other);
    // `a` answers hover with nothing; `b` has the documentation.
    let a = spec(
        "a",
        &root,
        steps([
            handshake(
                json!({"capabilities": {"textDocumentSync": 2, "hoverProvider": true, "positionEncoding": "utf-16"}}),
            ),
            vec![
                json!({"expect": "textDocument/hover"}),
                json!({"reply": null}),
            ],
        ]),
    );
    let b = spec(
        "b",
        &root,
        steps([
            handshake(json!({"capabilities": {
                "textDocumentSync": 2, "positionEncoding": "utf-16",
                "hoverProvider": true, "definitionProvider": true, "referencesProvider": true,
                "signatureHelpProvider": {"triggerCharacters": ["("]},
            }})),
            vec![
                json!({"expect": "textDocument/hover", "params": {"position": {"line": 2, "character": 1}}}),
                json!({"reply": {"contents": {"kind": "markdown", "value": "```python\ndef print()\n```"}, "range": r(2, 0, 2, 5)}}),
                json!({"expect": "textDocument/signatureHelp", "params": {
                    "context": {"triggerKind": 2, "triggerCharacter": "(", "isRetrigger": false},
                }}),
                json!({"reply": {"signatures": [{"label": "print(*values)", "parameters": [{"label": [6, 13]}]}], "activeParameter": 0}}),
                json!({"expect": "textDocument/definition"}),
                json!({"reply": [{"targetUri": other_uri, "targetRange": r(1, 0, 1, 18), "targetSelectionRange": r(1, 4, 1, 10)}]}),
                json!({"expect": "textDocument/references", "params": {"context": {"includeDeclaration": false}}}),
                json!({"reply": [
                    {"uri": other_uri, "range": r(0, 5, 0, 7)},
                    {"uri": file_uri(&root.join("a.py")), "range": r(2, 0, 2, 5)},
                    {"uri": other_uri, "range": r(0, 5, 0, 7)},
                ]}),
            ],
        ]),
    );
    let intel = LspCodeIntel::with_timeouts(vec![a, b], QUICK);
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), TEXT);
    events.opened_all(&["a", "b"]);
    let at = Point::new(2, 1);

    let hover = block_on(intel.hover(&editor.doc, at)).unwrap().unwrap();
    assert_eq!(hover.contents.kind, MarkupKind::Markdown);
    assert_eq!(hover.range, Some(Point::new(2, 0)..Point::new(2, 5)));

    let context = SignatureContext {
        trigger: SignatureTrigger::Character('('),
        active: None,
    };
    let help = block_on(intel.signature_help(&editor.doc, Point::new(2, 6), context))
        .unwrap()
        .unwrap();
    let print = &help.signatures[0];
    assert_eq!(&print.label[print.parameters[0].label.clone()], "*values");

    // Positions in a file that is not open: from its text on disk (UTF-16
    // columns after the crab, CRLF line breaks).
    let definition = block_on(intel.definition(&editor.doc, at)).unwrap();
    assert_eq!(
        definition,
        [Location {
            path: other.clone(),
            range: Point::new(1, 4)..Point::new(1, 10)
        }]
    );
    let references = block_on(intel.references(&editor.doc, at, false)).unwrap();
    assert_eq!(
        references,
        [
            Location {
                path: other.clone(),
                range: Point::new(0, 5)..Point::new(0, 6)
            },
            Location {
                path: root.join("a.py"),
                range: Point::new(2, 0)..Point::new(2, 5)
            },
        ]
    );
    // `a` serves no definitions: only `b` was asked.
    assert_eq!(
        block_on(intel.type_definition(&editor.doc, at)),
        Err(IntelError::Unsupported)
    );
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

/// A product that renames the server's quick fixes and adds its own action.
struct Renaming;

impl ServerCustomizer for Renaming {
    fn code_actions(
        &self,
        _: &DocumentId,
        text: &Rope,
        _: Range<Point>,
        _: &CodeActionContext,
        actions: &mut Vec<CodeAction>,
    ) {
        for action in actions.iter_mut() {
            action.title = format!("Lint: {}", action.title);
        }
        // The editor's text, not the file's.
        let first_line = text.line(0).to_string();
        actions.push(CodeAction::new(
            format!("Disable for this file ({})", first_line.trim()),
            CodeActionKind::QuickFix,
        ));
    }
}

#[test]
fn code_actions_of_two_servers_formatting_from_one() {
    let root = root("actions");
    let uri = file_uri(&root.join("a.py"));
    let lint = spec(
        "lint",
        &root,
        steps([
            handshake(json!({"capabilities": {
                "textDocumentSync": 2,
                "codeActionProvider": {"resolveProvider": true},
                "documentFormattingProvider": true,
                "executeCommandProvider": {"commands": ["lint.fixAll"]},
            }})),
            vec![
                json!({"expect": "textDocument/didOpen"}),
                json!({"notify": "textDocument/publishDiagnostics", "params": {"uri": uri, "version": 1, "diagnostics": [
                    {"range": r(0, 7, 0, 9), "message": "unused", "code": "F401", "data": {"fix": true}},
                ]}}),
                json!({"expect": "textDocument/codeAction", "params": {
                    "range": r(0, 7, 0, 9),
                    "context": {"diagnostics": [{"code": "F401", "data": {"fix": true}}], "triggerKind": 1},
                }}),
                json!({"reply": [
                    {"title": "Organize imports", "kind": "source.organizeImports.lint", "data": 1},
                    {"title": "Remove import", "kind": "quickfix", "isPreferred": true,
                     // Echoed normalised: another order, no `data`, a severity.
                     "diagnostics": [{"severity": 2, "code": "F401", "message": "unused", "range": r(0, 7, 0, 9)}],
                     "edit": {"changes": {uri.clone(): [{"range": r(0, 0, 1, 0), "newText": ""}]}}},
                    {"title": "Fix all", "command": "lint.fixAll", "arguments": [uri]},
                ]}),
                json!({"expect": "codeAction/resolve", "params": {"title": "Organize imports", "data": 1}}),
                json!({"reply": {"title": "Organize imports", "kind": "source.organizeImports.lint",
                    "edit": {"changes": {uri.clone(): [{"range": r(0, 0, 0, 0), "newText": "# sorted\n"}]}}}}),
                json!({"expect": "workspace/executeCommand", "params": {"command": "lint.fixAll", "arguments": [uri]}}),
                json!({"reply": null}),
                json!({"expect": "textDocument/formatting", "params": {"options": {"tabSize": 2, "insertSpaces": false}}}),
                json!({"reply": [{"range": r(2, 0, 2, 8), "newText": "print(y)"}]}),
            ],
        ]),
    )
    .with_features(Features::of(&[Feature::Diagnostics, Feature::CodeActions, Feature::Formatting]))
    .with_customizer(Arc::new(Renaming));
    let types = spec(
        "types",
        &root,
        steps([
            handshake(json!({"capabilities": {
                "textDocumentSync": 2, "codeActionProvider": true, "documentFormattingProvider": true,
            }})),
            vec![
                json!({"expect": "textDocument/codeAction", "params": {"context": {"diagnostics": []}}}),
                json!({"reply": [
                    {"title": "Extract variable", "kind": "refactor.extract"},
                    {"title": "Import x", "kind": "quickfix"},
                ]}),
                // Formatting goes to the first server only.
                json!({"forbid": "textDocument/formatting", "for": 1500}),
            ],
        ]),
    );
    let intel = LspCodeIntel::with_timeouts(vec![lint, types], QUICK);
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), TEXT);
    // The first server's diagnostics and the second's document, in any
    // order.
    let mut diagnostics = None;
    let mut opened = false;
    events.next("diagnostics, and the document open on types", |event| {
        match event {
            IntelEvent::Diagnostics { items, .. } if !items.is_empty() => {
                diagnostics = Some(items.clone());
            }
            IntelEvent::Status(s) if s.id.as_str() == "types" && !s.documents.is_empty() => {
                opened = true;
            }
            _ => {}
        }
        (diagnostics.is_some() && opened).then_some(())
    });
    let diagnostics = diagnostics.unwrap();

    let context = CodeActionContext {
        diagnostics: diagnostics.clone(),
        ..CodeActionContext::default()
    };
    let actions =
        block_on(intel.code_actions(&editor.doc, diagnostics[0].range.clone(), context)).unwrap();
    let titles: Vec<&str> = actions.iter().map(|a| a.title.as_str()).collect();
    // Quick fixes first, each server's order kept.
    assert_eq!(
        titles,
        [
            "Lint: Remove import",
            "Disable for this file (import os)",
            "Import x",
            "Lint: Organize imports",
            "Lint: Fix all",
            "Extract variable",
        ]
    );
    let remove = &actions[0];
    assert!(remove.preferred);
    assert_eq!(remove.diagnostics, diagnostics);
    assert_eq!(
        remove.edit.as_ref().unwrap().files[0].edits,
        [TextEdit::new(Point::new(0, 0)..Point::new(1, 0), "")]
    );
    // Resolved on demand; the product's title stays.
    let organize = block_on(intel.resolve_code_action(&editor.doc, actions[3].clone())).unwrap();
    assert_eq!(organize.title, "Lint: Organize imports");
    assert_eq!(
        organize.edit.unwrap().files[0].edits,
        [TextEdit::insert(Point::new(0, 0), "# sorted\n")]
    );
    // An action with an edit already needs nothing.
    assert_eq!(
        block_on(intel.resolve_code_action(&editor.doc, remove.clone())).unwrap(),
        *remove
    );
    let command = actions[4].command.clone().unwrap();
    assert_eq!(block_on(intel.execute_command(command)), Ok(()));

    let options = FormatOptions {
        tab_size: 2,
        insert_spaces: false,
    };
    let edits = block_on(intel.format(&editor.doc, None, options)).unwrap();
    assert_eq!(
        edits,
        [TextEdit::new(
            Point::new(2, 0)..Point::new(2, 8),
            "print(y)"
        )]
    );
    // No range formatting anywhere.
    assert_eq!(
        block_on(intel.format(
            &editor.doc,
            Some(Point::new(0, 0)..Point::new(1, 0)),
            options
        )),
        Err(IntelError::Unsupported)
    );
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn routing_errors_cancellation_and_stale_answers() {
    let root = root("routing");
    let script = steps([
        handshake(
            json!({"capabilities": {"textDocumentSync": 2, "hoverProvider": true, "definitionProvider": true, "completionProvider": {}}}),
        ),
        vec![
            // A hover dropped before its answer is cancelled.
            json!({"expect": "textDocument/hover"}),
            json!({"expect": "$/cancelRequest", "params": {"id": "$last_id"}}),
            json!({"reply_error": {"code": -32800, "message": "cancelled"}}),
            // An answer for a text that changed meanwhile is dropped.
            json!({"expect": "textDocument/completion"}),
            json!({"log": "completion asked"}),
            json!({"expect": "textDocument/didChange"}),
            json!({"reply": [{"label": "late"}], "to": "textDocument/completion"}),
            // Errors of the server.
            json!({"expect": "textDocument/definition"}),
            json!({"reply_error": {"code": -32603, "message": "boom"}}),
            json!({"expect": "textDocument/definition"}),
            json!({"reply_error": {"code": -32801, "message": "content modified"}}),
            json!({"log": "done"}),
        ],
    ]);
    let without_hover = spec(
        "plain",
        &root,
        steps([handshake(json!({"capabilities": {"textDocumentSync": 2}}))]),
    )
    .with_features(Features::of(&[Feature::Diagnostics]));
    let intel =
        LspCodeIntel::with_timeouts(vec![spec("fake", &root, script), without_hover], QUICK);
    let events = Events::of(intel.as_ref());
    let mut editor = Editor::open(intel.as_ref(), root.join("a.py"), TEXT);
    events.opened_all(&["fake", "plain"]);
    let at = Point::new(0, 1);

    let mut hover = intel.hover(&editor.doc, at);
    assert!((&mut hover).now_or_never().is_none());
    drop(hover);

    let completion = intel.completion(&editor.doc, at, CompletionTrigger::Invoked);
    events.log("completion asked");
    editor.edit(intel.as_ref(), vec![Edit::insert(0, "#")]);
    assert_eq!(block_on(completion), Err(IntelError::Cancelled));

    assert_eq!(
        block_on(intel.definition(&editor.doc, at)),
        Err(IntelError::Server {
            server: ServerId::new("fake"),
            code: -32603,
            message: "boom".into()
        })
    );
    assert_eq!(
        block_on(intel.definition(&editor.doc, at)),
        Err(IntelError::Cancelled)
    );
    events.log("done");

    // Nothing serves these: unsupported.
    assert_eq!(
        block_on(intel.references(&editor.doc, at, true)),
        Err(IntelError::Unsupported)
    );
    let other = DocumentId::new(root.join("notes.txt"));
    intel.did_open(&other, "plaintext", 1, &Rope::from_str("x"));
    assert_eq!(
        block_on(intel.hover(&other, at)),
        Err(IntelError::Unsupported)
    );
    // The server that would answer is not running.
    intel.stop(&ServerId::new("fake"));
    assert_eq!(
        block_on(intel.hover(&editor.doc, at)),
        Err(IntelError::NotRunning)
    );
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
fn rename_highlights_and_configuration_changes() {
    let root = root("rename");
    let uri = file_uri(&root.join("a.py"));
    let script = steps([
        handshake(json!({"capabilities": {
            "textDocumentSync": 2, "renameProvider": true, "documentHighlightProvider": true,
        }})),
        vec![
            json!({"expect": "textDocument/rename", "params": {"newName": "sys", "position": {"line": 0, "character": 8}}}),
            json!({"reply": {"changes": {uri.clone(): [
                {"range": r(0, 7, 0, 9), "newText": "sys"},
                {"range": r(1, 0, 1, 2), "newText": "sys"},
            ]}}}),
            json!({"expect": "textDocument/documentHighlight"}),
            json!({"reply": [{"range": r(0, 7, 0, 9), "kind": 3}, {"range": r(1, 0, 1, 2), "kind": 2}]}),
            json!({"expect": "workspace/didChangeConfiguration", "params": {"settings": {}}}),
            json!({"log": "configured"}),
        ],
    ]);
    let intel = LspCodeIntel::with_timeouts(vec![spec("fake", &root, script)], QUICK);
    let events = Events::of(intel.as_ref());
    let editor = Editor::open(intel.as_ref(), root.join("a.py"), TEXT);
    events.opened_all(&["fake"]);
    let at = Point::new(0, 8);
    let edit = block_on(intel.rename(&editor.doc, at, "sys".into())).unwrap();
    assert_eq!(
        edit.edits_for(&root.join("a.py"))
            .cloned()
            .collect::<Vec<_>>(),
        [
            TextEdit::new(Point::new(0, 7)..Point::new(0, 9), "sys"),
            TextEdit::new(Point::new(1, 0)..Point::new(1, 2), "sys"),
        ]
    );
    let highlights = block_on(intel.document_highlights(&editor.doc, at)).unwrap();
    assert_eq!(highlights.len(), 2);
    intel.configuration_changed();
    events.log("configured");
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}
