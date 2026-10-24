//! The product's language services in the main window (ADR 0010,
//! tests/scenarios/python-intel/completion.md): a project gets its
//! services from the product's provider; the editors of its tabs become
//! documents whose changes reach the services; diagnostics become the
//! editors' marks; the completion popup opens with Basic Completion and
//! closes with Escape while the focus stays in the editor, filters on the
//! client, drops stale answers and inserts items in one undo step; typing
//! opens it too, except in strings, comments and numbers; functions get
//! parentheses. The Language Services widget shows the servers with their
//! states and actions (tests/scenarios/python-intel/
//! language-services-widget.md). Driven against the in-memory
//! `FakeCodeIntel` with real keystrokes.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::fake::{Call, FakeCodeIntel, RequestKind};
use code_intel::{
    CodeIntel, CompletionItem, CompletionTrigger, Completions, Diagnostic, DocumentId, Feature,
    Features, FileChange, FileChangeKind, Point, ServerId, ServerState, ServerStatus, Severity,
    SymbolKind, TextEdit,
};
use code_intel_ui::completion::Parentheses;
use editor::{Analysis, Editor, Highlighter, MarkSeverity};
use gpui::{
    App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext, WindowHandle,
};
use serde_json::{Value, json};
use text::{Buffer, SelectionSet};
use themes::TextRole;
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Product, Workbench};

struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }

    /// The tool's name: the server's, capitalised (`ruff` is `Ruff`).
    fn server_title(&self, server: &ServerStatus) -> String {
        let mut chars = server.name.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    }

    fn parentheses(&self, language: &str) -> Option<Parentheses> {
        (language == "python").then_some(Parentheses {
            implicit_first: &["self", "cls"],
        })
    }
}

/// Colours `#` comments and quoted strings, line by line (as a language's
/// lexer does).
#[derive(Default)]
struct Lexer {
    lines: Vec<String>,
}

impl Highlighter for Lexer {
    fn reset(&mut self, buffer: &Buffer) {
        self.lines = (0..buffer.len_lines())
            .map(|l| buffer.line_text(l).into_owned())
            .collect();
    }

    fn edited(&mut self, buffer: &Buffer, _: &[text::AppliedEdit]) {
        self.reset(buffer);
    }

    fn line_spans(&self, line: usize, out: &mut Vec<editor::Span>) {
        let Some(text) = self.lines.get(line) else {
            return;
        };
        let mut quote = None;
        for (i, c) in text.chars().enumerate() {
            match (quote, c) {
                (None, '#') => {
                    out.push((i..text.chars().count(), TextRole::Comment));
                    return;
                }
                (None, '"') => quote = Some(i),
                (Some(start), '"') => {
                    out.push((start..i + 1, TextRole::String));
                    quote = None;
                }
                _ => {}
            }
        }
        if let Some(start) = quote {
            out.push((start..text.chars().count(), TextRole::String));
        }
    }
}

fn open(cx: &mut TestAppContext, hook: Option<CodeIntelHook>) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            highlighter: Some(Arc::new(|| Box::new(Lexer::default()))),
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        code_intel: hook.into_iter().collect(),
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Result<Value, ui::command::CommandError> {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    out
}

#[gpui::test]
fn a_project_gets_the_products_services(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("app.py"), "import os\n").unwrap();
    let fake = FakeCodeIntel::new();
    let mut ruff = ServerStatus::new(ServerId::new("ruff"), "ruff");
    ruff.version = Some("0.15.20".into());
    fake.set_status(vec![ruff]);
    let window = open(cx, Some(CodeIntelHook::new(Provider(fake.clone()))));

    // No project yet: the view exists, without services.
    let state = invoke(cx, window, "code.state", json!(null)).unwrap();
    assert_eq!(state["services"], false);

    let path = root.display().to_string();
    invoke(cx, window, "project.open-folder", json!({ "path": path })).unwrap();
    let state = invoke(cx, window, "code.state", json!(null)).unwrap();
    assert_eq!(state["services"], true);
    let servers = invoke(cx, window, "code.servers", json!(null)).unwrap();
    assert_eq!(servers[0]["id"], "ruff");
    assert_eq!(servers[0]["version"], "0.15.20");

    let file = root.join("app.py").display().to_string();
    invoke(cx, window, "project.open-file", json!({ "path": file })).unwrap();
    invoke(
        cx,
        window,
        "code.restart-servers",
        json!({ "server": "ruff" }),
    )
    .unwrap();
    invoke(cx, window, "code.stop-server", json!({ "server": "ty" })).unwrap();
    invoke(cx, window, "tabs.close", json!(null)).unwrap();
    let doc = DocumentId::new(root.join("app.py"));
    assert_eq!(
        fake.calls(),
        [
            Call::DidOpen {
                doc: doc.clone(),
                language: "python".into(),
                version: 1,
                text: "import os\n".into(),
            },
            Call::Restart(Some(ServerId::new("ruff"))),
            Call::Stop(ServerId::new("ty")),
            Call::DidClose { doc },
        ]
    );
    // Popup commands are not available without a popup, Basic Completion
    // not without an editor.
    assert!(invoke(cx, window, "completion.accept", json!(null)).is_err());
    assert!(invoke(cx, window, "code.complete", json!(null)).is_err());
}

#[gpui::test]
fn without_services_the_commands_are_not_available(cx: &mut TestAppContext) {
    let window = open(cx, None);
    let error = invoke(cx, window, "code.state", json!(null)).unwrap_err();
    assert_eq!(error.code, "not_available");
    let error = invoke(cx, window, "code.complete", json!(null)).unwrap_err();
    assert_eq!(error.code, "not_available");
}

/// A window with a project of `files` and the fake services.
struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
    fake: Arc<FakeCodeIntel>,
    _dir: tempfile::TempDir,
}

impl<'a> H<'a> {
    fn new(cx: &'a mut TestAppContext, files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = native_path::canonicalize(dir.path()).unwrap();
        for (path, text) in files {
            std::fs::write(root.join(path), text).unwrap();
        }
        let fake = FakeCodeIntel::new();
        let window = open(cx, Some(CodeIntelHook::new(Provider(fake.clone()))));
        let mut h = Self {
            cx,
            window,
            root,
            fake,
            _dir: dir,
        };
        let path = h.root.display().to_string();
        h.invoke("project.open-folder", json!({ "path": path }));
        h
    }

    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = invoke(self.cx, self.window, id, args).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
        out
    }

    fn state(&mut self) -> Value {
        self.invoke("code.state", json!(null))
    }

    fn completion(&mut self) -> Value {
        self.state()["completion"].clone()
    }

    fn labels(&mut self) -> Vec<String> {
        self.completion()["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|i| i["label"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn popup_shown(&mut self) -> bool {
        self.state()["popups"] == json!(["Completion"])
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(self.window.into(), text);
        self.draw();
    }

    fn doc(&self, name: &str) -> DocumentId {
        DocumentId::new(self.root.join(name))
    }

    /// Opens a file of the project in a tab, with the caret at the end of
    /// `marker` (the editor has the focus).
    fn open(&mut self, name: &str, caret_after: &str) {
        let path = self.root.join(name).display().to_string();
        self.invoke("project.open-file", json!({ "path": path }));
        let editor = self.editor();
        let offset = self.cx.update(|cx| {
            let text = editor.read(cx).text();
            let byte = text.find(caret_after).expect("marker") + caret_after.len();
            text[..byte].chars().count()
        });
        self.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                e.set_selections(SelectionSet::caret(offset), cx)
            })
        });
        self.focus_editor();
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self.editor();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    /// The editor's text with `|` at the caret.
    fn text(&mut self) -> String {
        let editor = self.editor();
        self.cx.update(|cx| {
            let e = editor.read(cx);
            let mut chars: Vec<char> = e.text().chars().collect();
            chars.insert(e.selections().primary().head, '|');
            chars.into_iter().collect()
        })
    }

    /// Answers completion requests with these labels (fields).
    fn complete_with(&mut self, labels: &[&str], incomplete: bool) {
        let items: Vec<CompletionItem> = labels
            .iter()
            .map(|l| CompletionItem::new(*l).with_kind(SymbolKind::Field))
            .collect();
        self.fake.on_completion(move |_, _, _| {
            Ok(Completions {
                items: items.clone(),
                incomplete,
            })
        });
    }

    fn completion_requests(&self) -> Vec<(Point, CompletionTrigger)> {
        self.fake
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Completion { at, trigger, .. } => Some((at, trigger)),
                _ => None,
            })
            .collect()
    }
}

#[gpui::test]
fn the_editors_of_tabs_are_documents(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "x = 1\n"), ("notes.txt", "hi\n")]);
    h.open("notes.txt", "hi");
    h.open("app.py", "x = 1");
    let doc = h.doc("app.py");
    // Only files of a language become documents.
    assert_eq!(h.fake.documents(), std::slice::from_ref(&doc));
    assert_eq!(h.fake.language(&doc).as_deref(), Some("python"));
    assert_eq!(h.fake.version(&doc), Some(1));

    // Edits reach the services with a higher version each, and leave their
    // copy of the text equal to the editor's.
    h.type_text("0");
    h.press("enter");
    h.type_text("y = 2");
    assert_eq!(h.fake.text(&doc).unwrap(), "x = 10\ny = 2\n");
    assert!(h.fake.in_sync(&doc));
    let version = h.fake.version(&doc).unwrap();
    assert!(version >= 4, "one version per change: {version}");
    let state = h.state();
    assert_eq!(state["documents"][0]["version"], json!(version));
    assert_eq!(
        state["active"],
        json!(h.root.join("app.py").display().to_string())
    );

    // Saving and replacing the whole text are reported too.
    h.fake.take_calls();
    h.invoke("workbench.save-all", json!(null));
    assert!(h.fake.calls().contains(&Call::DidSave { doc: doc.clone() }));
    let editor = h.editor();
    h.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            *e.buffer_mut() = Buffer::new("z = 3\n");
            e.buffer_reloaded(cx);
        })
    });
    h.draw();
    assert!(h.fake.calls().contains(&Call::DidReload {
        doc: doc.clone(),
        version: version + 1,
        text: "z = 3\n".into(),
    }));
    assert!(h.fake.in_sync(&doc));

    // Closing the tab closes the document.
    h.invoke("tabs.close", json!(null));
    assert!(h.fake.calls().contains(&Call::DidClose { doc }));
    assert!(h.fake.documents().is_empty());
}

#[gpui::test]
fn diagnostics_become_marks_of_the_editor(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import os\nfoo()\n")]);
    h.open("app.py", "import os");
    let doc = h.doc("app.py");
    let unused = |line| {
        let mut d = Diagnostic::new(
            Point::new(line, 7)..Point::new(line, 9),
            Severity::Warning,
            "`os` imported but unused",
        );
        d.source = Some("Ruff".into());
        d.unnecessary = true;
        d
    };
    let undefined = |line, severity, source: &str| {
        let mut d = Diagnostic::new(
            Point::new(line, 0)..Point::new(line, 3),
            severity,
            "undefined",
        );
        d.source = Some(source.into());
        d
    };
    h.fake.publish_diagnostics(
        &doc,
        1,
        vec![
            unused(0),
            undefined(1, Severity::Warning, "Ruff"),
            undefined(1, Severity::Error, "ty"),
        ],
    );
    h.draw();
    let editor = h.editor();
    let marks = h.cx.update(|cx| editor.read(cx).diagnostics().to_vec());
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0].range, 7..9);
    assert!(marks[0].unnecessary);
    // Two servers' problems at one range: one mark with both messages.
    assert_eq!(marks[1].range, 10..13);
    assert_eq!(marks[1].severity, MarkSeverity::Error);
    assert_eq!(marks[1].messages.len(), 2);
    assert_eq!(h.state()["problems"], json!(3));

    // A line is added at the top; a report for the text before it is moved
    // to where its ranges are now.
    h.cx.update(|cx| editor.update(cx, |e, cx| e.set_selections(SelectionSet::caret(0), cx)));
    h.type_text("# top\n");
    h.fake.publish_diagnostics(&doc, 1, vec![unused(0)]);
    h.draw();
    let marks = h.cx.update(|cx| editor.read(cx).diagnostics().to_vec());
    let text = h.cx.update(|cx| editor.read(cx).text());
    assert_eq!(marks.len(), 1);
    let range = marks[0].range.clone();
    assert_eq!(&text.chars().collect::<Vec<_>>()[range], &['o', 's']);
    // A report for a version the window never had is ignored.
    h.fake
        .publish_diagnostics(&doc, 99, vec![unused(0), unused(1)]);
    h.draw();
    let marks = h.cx.update(|cx| editor.read(cx).diagnostics().len());
    assert_eq!(marks, 1);
}

#[gpui::test]
fn basic_completion_opens_the_popup_and_escape_closes_it(cx: &mut TestAppContext) {
    let mut h = H::new(
        cx,
        &[("app.py", "class P:\n    def f(self):\n        self.n\n")],
    );
    h.open("app.py", "self.n");
    h.complete_with(&["norm", "__ne__", "x", "name"], false);
    h.press("ctrl-space");
    // The request is made at the caret; the popup shows the items that
    // match the identifier typed (names that start with it first, then in
    // the server's order), the first one selected.
    assert_eq!(
        h.completion_requests(),
        [(Point::new(2, 14), CompletionTrigger::Invoked)]
    );
    assert!(h.popup_shown());
    assert_eq!(h.labels(), ["norm", "name", "__ne__"]);
    let completion = h.completion();
    assert_eq!(completion["selected"], json!(0));
    assert_eq!(completion["prefix"], json!("n"));
    assert_eq!(completion["items"][0]["matched"], json!([[0, 1]]));
    assert_eq!(completion["items"][0]["kind"], json!("field"));
    let (insert, replace) = if cfg!(target_os = "macos") {
        ("⏎", "⇥")
    } else {
        ("Enter", "Tab")
    };
    assert_eq!(
        completion["hint"],
        json!(format!("Press {insert} to insert, {replace} to replace"))
    );
    // It is drawn under the caret's line, and the focus stays in the
    // editor: keys go to the popup through the editor.
    let popup = VisualTestContext::from_window(h.window.into(), h.cx)
        .debug_bounds("completion-popup")
        .expect("the popup is drawn");
    let editor = h.editor();
    let caret = h.cx.update(|cx| editor.read(cx).caret_bounds().unwrap());
    assert!(popup.top() >= caret.bottom() - gpui::px(1.0));
    assert!(h.editor_focused());
    h.press("down");
    assert_eq!(h.completion()["selected"], json!(1));
    h.press("up");
    h.press("up");
    assert_eq!(h.completion()["selected"], json!(2), "Up goes round");
    assert_eq!(h.text(), "class P:\n    def f(self):\n        self.n|\n");

    h.press("escape");
    assert!(!h.popup_shown());
    assert!(h.completion().is_null());
    assert_eq!(h.text(), "class P:\n    def f(self):\n        self.n|\n");
    assert!(h.editor_focused());
    // Without the popup the keys are the editor's again.
    h.press("down");
    assert_eq!(h.text(), "class P:\n    def f(self):\n        self.n\n|");
}

#[gpui::test]
fn typing_filters_on_the_client_unless_the_answer_is_incomplete(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "p.\n")]);
    h.open("app.py", "p.");
    h.complete_with(&["norm", "name", "x"], false);
    h.press("ctrl-space");
    // Without sort texts: the server's order.
    assert_eq!(h.labels(), ["norm", "name", "x"]);
    h.type_text("n");
    assert_eq!(h.labels(), ["norm", "name"]);
    h.type_text("o");
    assert_eq!(h.labels(), ["norm"]);
    // Backspace widens the list again; still one request.
    h.press("backspace");
    assert_eq!(h.labels(), ["norm", "name"]);
    assert_eq!(h.completion_requests().len(), 1);
    // A character that is not part of an identifier closes the popup.
    h.type_text(" ");
    assert!(!h.popup_shown());

    // An incomplete answer is asked for again as the prefix grows.
    h.complete_with(&["norm", "name"], true);
    h.press("backspace");
    h.press("ctrl-space");
    h.type_text("a");
    let requests = h.completion_requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[2],
        (Point::new(0, 4), CompletionTrigger::Incomplete)
    );
    assert_eq!(h.labels(), ["name"]);
}

#[gpui::test]
fn stale_answers_are_dropped(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "p.no\n")]);
    h.open("app.py", "p.no");
    h.complete_with(&["norm"], false);
    h.fake.hold(RequestKind::Completion);
    h.press("ctrl-space");
    assert_eq!(h.fake.pending(RequestKind::Completion), 1);
    assert!(!h.popup_shown());
    // The caret leaves the identifier: the request is cancelled, its late
    // answer shows nothing.
    h.press("left");
    h.press("left");
    h.press("left");
    assert_eq!(h.fake.dropped(RequestKind::Completion), 1);
    h.fake.release(RequestKind::Completion);
    h.draw();
    assert!(!h.popup_shown());
    assert!(h.completion().is_null());

    // A newer request replaces an older one: only its answer counts.
    h.press("right");
    h.press("right");
    h.press("right");
    h.press("ctrl-space");
    h.press("ctrl-space");
    assert_eq!(h.fake.dropped(RequestKind::Completion), 1);
    assert_eq!(h.fake.pending(RequestKind::Completion), 1);
    h.fake.release(RequestKind::Completion);
    h.draw();
    assert!(h.popup_shown());
    assert_eq!(h.labels(), ["norm"]);
}

#[gpui::test]
fn enter_inserts_and_tab_replaces_in_one_undo_step(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "p.noxyz\nx = Pa\n")]);
    h.open("app.py", "p.no");
    h.complete_with(&["norm"], false);
    h.press("ctrl-space");
    h.press("enter");
    assert_eq!(h.text(), "p.norm|xyz\nx = Pa\n");
    assert!(!h.popup_shown());
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "p.no|xyz\nx = Pa\n");
    h.press("ctrl-space");
    h.press("tab");
    assert_eq!(h.text(), "p.norm|\nx = Pa\n");

    // An item with an import: both in one change, undone together.
    let mut path = CompletionItem::new("Path").with_kind(SymbolKind::Class);
    path.additional_edits = vec![TextEdit::insert(
        Point::new(0, 0),
        "from pathlib import Path\n",
    )];
    h.fake.on_completion(move |_, _, _| {
        Ok(Completions {
            items: vec![path.clone()],
            incomplete: false,
        })
    });
    let editor = h.editor();
    h.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            let end = e.text().chars().count() - 1;
            e.set_selections(SelectionSet::caret(end), cx)
        })
    });
    h.press("ctrl-space");
    h.press("enter");
    assert_eq!(h.text(), "from pathlib import Path\np.norm\nx = Path|\n");
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "p.norm\nx = Pa|\n");
}

#[gpui::test]
fn typing_opens_the_popup_except_in_strings_comments_and_numbers(cx: &mut TestAppContext) {
    let mut h = H::new(
        cx,
        &[(
            "app.py",
            "class P:\n    def f(self):\n        self\n# c\ns = \"ab\"\nn = 1\n",
        )],
    );
    h.fake.set_completion_triggers(&['.']);
    h.complete_with(&["norm", "x", "__ne__"], false);
    // A trigger char of the server opens it at once, with the first item
    // preselected only (an automatic popup is not focused).
    h.open("app.py", "        self");
    h.type_text(".");
    assert_eq!(
        h.completion_requests(),
        [(Point::new(2, 13), CompletionTrigger::Character('.'))]
    );
    assert!(h.popup_shown());
    assert_eq!(h.labels(), ["norm", "x", "__ne__"]);
    assert_eq!(h.completion()["focused"], json!(false));
    assert_eq!(h.completion()["items"][0]["icon"], json!("nodes/field"));
    // Down focuses it and moves on; Enter inserts.
    h.press("down");
    assert_eq!(h.completion()["selected"], json!(1));
    assert_eq!(h.completion()["focused"], json!(true));
    h.press("enter");
    assert_eq!(
        h.text(),
        "class P:\n    def f(self):\n        self.x|\n# c\ns = \"ab\"\nn = 1\n"
    );

    // Identifier chars open it too, filtered by what is typed.
    h.press("enter");
    h.fake.take_calls();
    h.type_text("n");
    assert_eq!(
        h.completion_requests(),
        [(Point::new(3, 9), CompletionTrigger::Invoked)]
    );
    assert_eq!(h.labels(), ["norm", "__ne__"]);
    h.press("escape");

    // Not in a comment, a string or a number; no request at all.
    h.fake.take_calls();
    h.open("app.py", "# c");
    h.type_text("x");
    h.open("app.py", "\"ab");
    h.type_text("x");
    h.open("app.py", "n = 1");
    h.type_text("2");
    assert!(h.completion_requests().is_empty());
    assert!(!h.popup_shown());
}

#[gpui::test]
fn automatic_popups_without_items_stay_away_while_typing_on(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "x = \n")]);
    h.complete_with(&["alpha"], false);
    h.open("app.py", "x = ");
    // Nothing matches `z`: no popup, and no new request while the same
    // identifier grows.
    h.type_text("z");
    assert!(!h.popup_shown());
    h.type_text("z");
    assert_eq!(h.completion_requests().len(), 1);
    // A new identifier asks again.
    h.type_text(" ");
    h.type_text("a");
    assert_eq!(h.completion_requests().len(), 2);
    assert_eq!(h.labels(), ["alpha"]);
    // An automatic popup whose only item is typed out already closes.
    h.type_text("lpha");
    assert!(!h.popup_shown());
    // Basic Completion with nothing to offer says so in a hint, which the
    // next key closes.
    h.complete_with(&[], false);
    h.press("ctrl-space");
    assert!(!h.popup_shown());
    assert_eq!(h.state()["hint"], json!("No suggestions"));
    h.press("left");
    assert!(h.state()["hint"].is_null());
}

#[gpui::test]
fn functions_get_parentheses(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "p.no\np.sc\nprint(p.no())\n")]);
    let mut norm = CompletionItem::new("norm").with_kind(SymbolKind::Method);
    norm.type_text = Some("bound method P.norm() -> float".into());
    let mut scaled = CompletionItem::new("scaled").with_kind(SymbolKind::Method);
    scaled.detail = Some("def scaled(self, k: float) -> P".into());
    h.fake.on_completion(move |_, _, _| {
        Ok(Completions {
            items: vec![norm.clone(), scaled.clone()],
            incomplete: false,
        })
    });
    h.open("app.py", "p.no");
    h.press("ctrl-space");
    h.press("enter");
    assert_eq!(h.text(), "p.norm()|\np.sc\nprint(p.no())\n");
    h.open("app.py", "p.sc");
    h.press("ctrl-space");
    h.press("enter");
    assert_eq!(h.text(), "p.norm()\np.scaled(|)\nprint(p.no())\n");
    // Parentheses there already: none added.
    h.open("app.py", "print(p.no");
    h.press("ctrl-space");
    h.press("enter");
    assert_eq!(h.text(), "p.norm()\np.scaled()\nprint(p.norm()|)\n");
}

/// A server status with a version and documents.
fn server(id: &str, version: &str, state: ServerState, docs: &[DocumentId]) -> ServerStatus {
    let mut s = ServerStatus::new(ServerId::new(id), id);
    s.version = Some(version.into());
    s.state = state;
    s.documents = docs.to_vec();
    s
}

#[gpui::test]
fn the_language_services_widget_shows_the_servers(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "x = 1\n"), ("notes.txt", "n\n")]);
    // No servers: no widget.
    h.open("app.py", "x");
    assert!(h.state()["widget"].is_null());
    let draw = |h: &mut H| {
        h.draw();
        VisualTestContext::from_window(h.window.into(), h.cx)
            .debug_bounds("status-language-services")
            .is_some()
    };
    assert!(!draw(&mut h));

    let app = h.doc("app.py");
    h.fake.set_status(vec![
        server(
            "ruff",
            "0.15.20",
            ServerState::Running,
            std::slice::from_ref(&app),
        ),
        server("ty", "0.0.1", ServerState::Running, &[]),
    ]);
    h.draw();
    let widget = h.state()["widget"].clone();
    assert_eq!(widget["icons"].as_array().unwrap().len(), 1);
    assert_eq!(
        widget["tooltip"],
        json!("Language Services running on the current file:\n- Ruff 0.15.20")
    );
    assert!(draw(&mut h));

    // The popup: both sections, a row per server with its actions.
    h.cx.update_window(h.window.into(), |root, window, cx| {
        let wb = root.downcast::<Workbench>().unwrap();
        let view = wb.read(cx).code_intel().unwrap().clone();
        view.update(cx, |v, cx| v.toggle_services_popup(window, cx));
    })
    .unwrap();
    h.draw();
    let popup = h.state()["services_popup"].clone();
    assert_eq!(popup["title"], json!("Language Services"));
    let rows = popup["rows"].as_array().unwrap().clone();
    assert_eq!(rows[0]["header"], json!("Running on Current File"));
    assert_eq!(rows[1]["label"], json!("ruff 0.15.20"));
    assert_eq!(
        rows[1]["actions"],
        json!(["Restart Server", "Open Settings"])
    );
    assert_eq!(rows[2]["header"], json!("Running on Other Files"));
    assert_eq!(rows[3]["label"], json!("ty 0.0.1"));
    assert_eq!(
        rows[3]["actions"],
        json!(["Stop and Autorun Server as Needed", "Open Settings"])
    );
    // The popup has the focus and the keys of list popups; Escape closes it.
    h.press("down");
    assert_eq!(h.state()["services_popup"]["selected"], json!("ty 0.0.1"));
    h.press("escape");
    assert!(h.state()["services_popup"].is_null());

    // Restart and stop: the services are asked, a notice says so.
    h.fake.take_calls();
    h.invoke("code.restart-servers", json!({ "server": "ruff" }));
    assert!(
        h.fake
            .calls()
            .contains(&Call::Restart(Some(ServerId::new("ruff"))))
    );
    let notice = h.invoke("workbench.state", json!(null))["notice"].clone();
    assert_eq!(notice, json!("Ruff server restarted"));
    h.invoke("code.stop-server", json!({ "server": "ty" }));
    let notice = h.invoke("workbench.state", json!(null))["notice"].clone();
    assert!(notice.as_str().unwrap().starts_with("Ty server stopped"));
    // The notice goes after a while.
    h.cx.executor()
        .advance_clock(std::time::Duration::from_secs(11));
    h.draw();
    assert!(h.invoke("workbench.state", json!(null))["notice"].is_null());

    // Another file: `{}`, the servers under Other Files.
    h.open("notes.txt", "n");
    let widget = h.state()["widget"].clone();
    assert_eq!(widget["icons"][0]["icon"], json!("json/object"));
    assert_eq!(widget["tooltip"], json!("Language Services"));

    // A server that terminated: an error mark, its error output in a tab.
    h.open("app.py", "x");
    h.fake.set_status(vec![server(
        "ruff",
        "0.15.20",
        ServerState::Terminated {
            error_output: "thread main panicked".into(),
        },
        std::slice::from_ref(&app),
    )]);
    h.draw();
    let widget = h.state()["widget"].clone();
    assert_eq!(widget["icons"][0]["error"], json!(true));
    h.cx.update_window(h.window.into(), |root, window, cx| {
        let wb = root.downcast::<Workbench>().unwrap();
        let view = wb.read(cx).code_intel().unwrap().clone();
        view.update(cx, |v, cx| v.toggle_services_popup(window, cx));
    })
    .unwrap();
    h.draw();
    let rows = h.state()["services_popup"]["rows"].clone();
    assert_eq!(rows[1]["label"], json!("ruff 0.15.20 | Terminated"));
    assert_eq!(
        rows[1]["actions"],
        json!([
            "Open Server Error Output",
            "Restart Server",
            "Open Settings"
        ])
    );
    let action = VisualTestContext::from_window(h.window.into(), h.cx)
        .debug_bounds("language-service:ruff 0.15.20 | Terminated")
        .expect("the row is drawn");
    // The first inline button is at the right end of the row, left of the
    // two others: click it.
    let at = gpui::point(
        action.right() - gpui::px(8.0 + 22.0 * 2.5 + 8.0),
        action.center().y,
    );
    VisualTestContext::from_window(h.window.into(), h.cx)
        .simulate_click(at, gpui::Modifiers::none());
    h.draw();
    let state = h.invoke("workbench.state", json!(null));
    let tab = state["tabs"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(tab["path"], json!("Ruff Server Error Output"));
    let editor = h.editor();
    let (text, read_only) = h.cx.update(|cx| {
        let e = editor.read(cx);
        (e.text(), e.buffer().is_read_only())
    });
    assert_eq!(text, "thread main panicked");
    assert!(read_only);
}

#[gpui::test]
fn saves_reach_the_services_as_file_changes(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "x = 1\n"), ("pyproject.toml", "[tool]\n")]);
    h.open("pyproject.toml", "[tool]");
    h.type_text("x");
    h.fake.take_calls();
    h.invoke("workbench.save-all", json!(null));
    let path = h.root.join("pyproject.toml");
    assert_eq!(
        h.fake.calls(),
        [Call::FilesChanged(vec![FileChange::new(
            path,
            FileChangeKind::Changed
        )])]
    );
}

#[gpui::test]
fn the_inspection_widget_follows_the_analysis_of_the_servers(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "x = 1\n"), ("notes.txt", "n\n")]);
    h.open("app.py", "x = 1");
    let doc = h.doc("app.py");
    let analysis = |h: &mut H| {
        let editor = h.editor();
        h.cx.update(|cx| editor.read(cx).analysis())
    };
    // No server reports on the file: no widget.
    assert_eq!(analysis(&mut h), Analysis::Off);
    // Two servers report diagnostics on it: running until both did, for
    // the current text.
    let reporting = |id: &str| {
        let mut s = server(id, "1", ServerState::Running, std::slice::from_ref(&doc));
        s.features = Features::of(&[Feature::Diagnostics]);
        s
    };
    h.fake.set_status(vec![reporting("ruff"), reporting("ty")]);
    h.draw();
    assert_eq!(analysis(&mut h), Analysis::Running);
    let version = h.fake.version(&doc).unwrap();
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.draw();
    assert_eq!(analysis(&mut h), Analysis::Running);
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.draw();
    assert_eq!(analysis(&mut h), Analysis::Done);
    // A change: running again; reports for the older text do not count.
    h.type_text("0");
    assert_eq!(analysis(&mut h), Analysis::Running);
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.draw();
    assert_eq!(analysis(&mut h), Analysis::Running);
    let version = h.fake.version(&doc).unwrap();
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.draw();
    assert_eq!(analysis(&mut h), Analysis::Done);
    // A server that terminated reports nothing: the other one is enough.
    h.type_text("0");
    let mut crashed = reporting("ty");
    crashed.state = ServerState::Terminated {
        error_output: String::new(),
    };
    h.fake.set_status(vec![reporting("ruff"), crashed]);
    let version = h.fake.version(&doc).unwrap();
    h.fake.publish_diagnostics(&doc, version, Vec::new());
    h.draw();
    assert_eq!(analysis(&mut h), Analysis::Done);
    // A file no service analyses has none.
    h.open("notes.txt", "n");
    assert_eq!(analysis(&mut h), Analysis::Off);
}
