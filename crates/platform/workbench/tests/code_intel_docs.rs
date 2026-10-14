//! Documentation, problems and parameter info in the main window (ADR
//! 0010; tests/scenarios/python-intel/documentation-and-parameter-info.md,
//! diagnostics.md): Quick Documentation at the caret, the popup over the
//! text under the mouse after 500 ms with the problems there, their first
//! quick fix and the symbol's documentation, Error Description, and
//! parameter info above the call, following the caret and opening a second
//! after `(`. Driven against the in-memory `FakeCodeIntel`; the mouse over
//! the text is the editor's `Hovered` event.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use code_intel::fake::{Call, FakeCodeIntel};
use code_intel::{
    CodeAction, CodeActionKind, CodeIntel, Diagnostic, DocumentId, FileEdits, Hover, Location,
    Markup, ParameterInfo, Point, Severity, Signature, SignatureHelp, SignatureTrigger, TextEdit,
    WorkspaceEdit,
};
use code_intel_ui::DocLocation;
use code_intel_ui::syntax::{LexSpan, Lexer};
use editor::{Editor, EditorEvent};
use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext,
    WindowHandle, point, px,
};
use serde_json::{Value, json};
use text::SelectionSet;
use themes::TextRole;
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Product, Workbench};

/// Colours `def` as a keyword.
struct Keywords;

impl Lexer for Keywords {
    fn lex_line(&self, line: &str, state: u32, out: &mut Vec<LexSpan>) -> u32 {
        for (i, _) in line.match_indices("def") {
            out.push((i..i + 3, TextRole::Keyword));
        }
        state
    }
}

struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }

    fn lexer(&self, language: &str) -> Option<Arc<dyn Lexer>> {
        (language == "python").then(|| Arc::new(Keywords) as Arc<dyn Lexer>)
    }

    fn doc_location(&self, path: &Path) -> Option<DocLocation> {
        // A package is the folder of its `__init__`.
        let stem = path.file_stem()?.to_string_lossy();
        let name = if stem == "__init__" {
            path.parent()?.file_name()?.to_string_lossy().into_owned()
        } else {
            stem.into_owned()
        };
        Some(DocLocation {
            name,
            icon: ui::icons::IconName::MODULE,
        })
    }
}

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
        let root = dir.path().canonicalize().unwrap();
        for (path, text) in files {
            std::fs::write(root.join(path), text).unwrap();
        }
        let fake = FakeCodeIntel::new();
        let product = Product {
            name: "Test IDE",
            file_types: vec![FileType {
                language_id: Some("python"),
                ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
            }],
            code_intel: Some(CodeIntelHook::new(Provider(fake.clone()))),
            ..Product::default()
        };
        let window = cx.update(|cx| {
            workbench::init(cx, &product);
            workbench::open_main_window(cx, Rc::new(product))
        });
        cx.run_until_parked();
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
        self.invoke("code.state", json!(null))
    }

    fn popup(&mut self) -> Value {
        self.state()["popup"].clone()
    }

    fn signature(&mut self) -> Value {
        self.state()["signature"].clone()
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(self.window.into(), text);
        self.draw();
    }

    /// Lets time pass on the test clock.
    fn wait(&mut self, ms: u64) {
        self.cx.executor().advance_clock(Duration::from_millis(ms));
        self.draw();
    }

    fn doc(&self, name: &str) -> DocumentId {
        DocumentId::new(self.root.join(name))
    }

    /// Opens a file of the project with the caret right after `marker`.
    fn open(&mut self, name: &str, caret_after: &str) {
        let path = self.root.join(name).display().to_string();
        self.invoke("project.open-file", json!({ "path": path }));
        self.caret_after(caret_after);
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn offset_after(&mut self, marker: &str) -> usize {
        let editor = self.editor();
        self.cx.update(|cx| {
            let text = editor.read(cx).text();
            let byte = text.find(marker).expect("marker") + marker.len();
            text[..byte].chars().count()
        })
    }

    fn caret_after(&mut self, marker: &str) {
        let offset = self.offset_after(marker);
        let editor = self.editor();
        self.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                e.set_selections(SelectionSet::caret(offset), cx)
            })
        });
        self.draw();
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor is open")
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self.editor();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn text(&mut self) -> String {
        let editor = self.editor();
        self.cx.update(|cx| editor.read(cx).text())
    }

    /// The mouse over the char at `offset` of the editor's text (`None`:
    /// off the text), as the editor reports it.
    fn hover(&mut self, offset: Option<usize>) {
        let editor = self.editor();
        self.cx.update(|cx| {
            editor.update(cx, |_, cx| {
                cx.emit(EditorEvent::Hovered {
                    offset,
                    position: point(px(0.0), px(0.0)),
                    modifiers: Modifiers::default(),
                })
            })
        });
        self.draw();
    }

    fn bounds(&mut self, selector: &'static str) -> Option<gpui::Bounds<gpui::Pixels>> {
        VisualTestContext::from_window(self.window.into(), self.cx).debug_bounds(selector)
    }

    fn caret_bounds(&mut self) -> gpui::Bounds<gpui::Pixels> {
        let editor = self.editor();
        self.cx.update(|cx| editor.read(cx).caret_bounds().unwrap())
    }

    fn mouse_to(&mut self, position: gpui::Point<gpui::Pixels>) {
        let mut cx = VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_mouse_move(position, None, Modifiers::default());
        self.draw();
    }

    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .bounds(selector)
            .unwrap_or_else(|| panic!("{selector} drawn"));
        let mut cx = VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_click(bounds.center(), Modifiers::default());
        self.draw();
    }

    fn calls_of(&self, f: impl Fn(&Call) -> bool) -> Vec<Call> {
        self.fake.calls().into_iter().filter(|c| f(c)).collect()
    }
}

const DUMPS: &str = "```python\ndef dumps(obj: Any, *, skipkeys: bool = False, ensure_ascii: bool = True, **kwds: Any) -> str\n```\n---\nSerialize ``obj`` to a JSON formatted ``str``.\n\nSee [the docs](https://docs.python.org/3/library/json.html).";

fn answer_dumps(fake: &FakeCodeIntel, root: &Path) {
    fake.on_hover(|_, at| {
        Ok((at.line == 1).then(|| Hover {
            contents: Markup::markdown(DUMPS),
            range: Some(Point::new(1, 5)..Point::new(1, 10)),
        }))
    });
    let json = root.join("stubs/json/__init__.pyi");
    fake.on_definition(move |_, _| {
        Ok(vec![Location {
            path: json.clone(),
            range: Point::new(3, 4)..Point::new(3, 9),
        }])
    });
}

#[gpui::test]
fn quick_documentation_at_the_caret(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import json\njson.dumps(1)\n")]);
    h.open("app.py", "json.dumps");
    answer_dumps(&h.fake, &h.root);
    h.invoke("code.quick-doc", json!(null));

    // Asked at the name before the caret (the caret is after it).
    let doc = h.doc("app.py");
    assert!(h.fake.calls().contains(&Call::Hover {
        doc: doc.clone(),
        at: Point::new(1, 9),
    }));
    let popup = h.popup();
    assert_eq!(popup["kind"], "quick-doc");
    assert_eq!(popup["markdown"], DUMPS);
    let documentation = &popup["documentation"];
    // The module of the declaration, the signature with one parameter per
    // line, then the description without the rule after the signature.
    assert_eq!(documentation["location"], "json");
    assert_eq!(
        documentation["definition"],
        "def dumps(obj: Any,\n          *,\n          skipkeys: bool = False,\n          ensure_ascii: bool = True,\n          **kwds: Any) -> str"
    );
    let blocks = documentation["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["text"], "Serialize obj to a JSON formatted str.");
    assert_eq!(
        blocks[1]["links"][0]["href"],
        "https://docs.python.org/3/library/json.html"
    );

    // Drawn under the caret's line; the focus stays in the editor.
    let bounds = h.bounds("documentation-popup").expect("the popup is drawn");
    let caret = h.caret_bounds();
    assert!(bounds.top() >= caret.bottom() - px(1.0));
    assert!(h.bounds("doc-location").is_some());
    assert!(h.bounds("doc-definition").is_some());
    assert!(h.editor_focused());

    // Escape closes the popup.
    h.press("escape");
    assert!(h.popup().is_null());
    assert_eq!(h.text(), "import json\njson.dumps(1)\n");

    // Moving the caret closes it as well.
    h.invoke("code.quick-doc", json!(null));
    assert!(!h.popup().is_null());
    h.press("left");
    assert!(h.popup().is_null());

    // Nothing to show: no popup.
    h.caret_after("import");
    h.invoke("code.quick-doc", json!(null));
    assert!(h.popup().is_null());
}

/// A file with an unused import (with a fix that removes it, and a second
/// quick fix) and a documented call.
fn problems_fixture(h: &mut H<'_>) {
    let doc = h.doc("app.py");
    let mut unused = Diagnostic::new(
        Point::new(0, 7)..Point::new(0, 9),
        Severity::Warning,
        "`os` imported but unused",
    );
    unused.source = Some("Ruff".into());
    unused.code = Some("unused-import".into());
    unused.code_href = Some("https://docs.astral.sh/ruff/rules/unused-import".into());
    unused.unnecessary = true;
    h.fake.publish_diagnostics(&doc, 1, vec![unused]);
    let path = doc.path().to_owned();
    h.fake.on_code_actions(move |_, _, context| {
        // The fixes of the problem: found through the diagnostic sent back
        // (the bulb asks elsewhere too).
        if context.diagnostics.len() != 1 {
            return Ok(Vec::new());
        }
        let mut fix = CodeAction::new("Remove unused import: `os`", CodeActionKind::QuickFix);
        fix.edit = Some(WorkspaceEdit {
            files: vec![FileEdits {
                path: path.clone(),
                edits: vec![TextEdit::new(Point::new(0, 0)..Point::new(1, 0), "")],
            }],
        });
        let disable = CodeAction::new("Disable for this line", CodeActionKind::QuickFix);
        let organize = CodeAction::new("Organize imports", CodeActionKind::organize_imports());
        Ok(vec![organize, fix, disable])
    });
    h.fake.on_hover(|_, at| {
        Ok((at.line == 0).then(|| Hover {
            contents: Markup::markdown("```python\n<module 'os'>\n```\n---\nOS routines."),
            range: Some(Point::new(0, 7)..Point::new(0, 9)),
        }))
    });
}

#[gpui::test]
fn the_popup_over_the_text_after_half_a_second(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import os\nx = 1\n")]);
    h.open("app.py", "x = 1");
    problems_fixture(&mut h);
    h.draw();

    let os = h.offset_after("import o") - 1;
    h.hover(Some(os));
    h.wait(499);
    assert!(h.popup().is_null(), "not before the delay");
    assert!(h.calls_of(|c| matches!(c, Call::Hover { .. })).is_empty());
    h.wait(1);
    let popup = h.popup();
    assert_eq!(popup["kind"], "hover");
    // The problem with its source and rule, the first quick fix and More
    // actions, then the documentation: one popup.
    assert_eq!(popup["problems"][0]["text"], "`os` imported but unused");
    assert_eq!(popup["problems"][0]["source"], "Ruff");
    assert_eq!(popup["problems"][0]["code"], "unused-import");
    assert_eq!(popup["fix"], "Remove unused import: `os`");
    assert_eq!(popup["more_actions"], true);
    assert_eq!(popup["documentation"]["definition"], "<module 'os'>");
    assert!(h.bounds("doc-problem-0").is_some());
    assert!(h.bounds("doc-actions").is_some());
    assert!(h.bounds("doc-documentation").is_some());
    assert!(h.editor_focused());

    // The mouse stays over the name: the popup stays.
    h.hover(Some(os + 1));
    h.wait(1000);
    assert!(!h.popup().is_null());

    // The mouse leaves the name and enters the popup: it stays.
    h.hover(None);
    let inside = h.bounds("documentation-popup").unwrap().center();
    h.mouse_to(inside);
    h.wait(1000);
    assert!(
        !h.popup().is_null(),
        "the popup stays while the mouse is inside"
    );
    // ... and closes a moment after the mouse left it.
    h.mouse_to(point(px(1.0), px(1.0)));
    h.wait(299);
    assert!(!h.popup().is_null());
    h.wait(1);
    assert!(h.popup().is_null());

    // Over plain text without problems nothing is asked for.
    let calls = h.fake.calls().len();
    let space = h.offset_after("import");
    h.hover(Some(space));
    h.wait(600);
    assert!(h.popup().is_null());
    assert_eq!(h.fake.calls().len(), calls);
}

#[gpui::test]
fn the_rule_link_opens_in_the_browser_and_the_fix_applies(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import os\nx = 1\n")]);
    h.open("app.py", "x = 1");
    problems_fixture(&mut h);
    h.draw();
    let os = h.offset_after("import o") - 1;
    h.hover(Some(os));
    h.wait(500);
    assert!(!h.popup().is_null());

    // The rule name is a link at the end of the problem's line.
    let problem = h.bounds("doc-problem-0").unwrap();
    let mut probe = problem.right() - px(14.0);
    let mut opened = None;
    while probe > problem.left() && opened.is_none() {
        let mut cx = VisualTestContext::from_window(h.window.into(), h.cx);
        cx.simulate_click(point(probe, problem.center().y), Modifiers::default());
        opened = h.cx.opened_url();
        probe -= px(6.0);
    }
    assert_eq!(
        opened.as_deref(),
        Some("https://docs.astral.sh/ruff/rules/unused-import")
    );

    // The fix of the actions row: one undo step.
    h.click("doc-fix");
    assert_eq!(h.text(), "x = 1\n");
    assert!(h.popup().is_null());
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "import os\nx = 1\n");
}

#[gpui::test]
fn error_description_and_the_first_fix_at_the_caret(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import os\nx = 1\n")]);
    h.open("app.py", "import os");
    problems_fixture(&mut h);
    h.draw();
    // The caret at the end of the problem's range counts.
    h.invoke("code.error-description", json!(null));
    let popup = h.popup();
    assert_eq!(popup["kind"], "error-description");
    assert_eq!(popup["problems"][0]["code"], "unused-import");
    assert!(popup["documentation"].is_null());
    assert!(popup["fix"].is_null());
    h.press("escape");
    assert!(h.popup().is_null());

    // Apply First Quick Fix: the first quick fix of the problem.
    h.invoke("code.apply-first-fix", json!(null));
    assert_eq!(h.text(), "x = 1\n");
    h.invoke("editor.undo", json!(null));
    assert_eq!(h.text(), "import os\nx = 1\n");

    // No problem at the caret: nothing.
    h.caret_after("x = ");
    h.invoke("code.error-description", json!(null));
    assert!(h.popup().is_null());
}

/// Answers signature help for `dumps(obj, *, skipkeys, **kwds)` with the
/// parameter by the commas before the position.
fn answer_signatures(h: &mut H<'_>) {
    // The text as the services see it when they are asked (ASCII).
    let fake = Arc::downgrade(&h.fake);
    h.fake.on_signature_help(move |doc, at, _| {
        let text = fake.upgrade().and_then(|f| f.text(doc)).unwrap_or_default();
        let line = text.lines().nth(at.line).unwrap_or("");
        let before = &line[..at.column.min(line.len())];
        let Some(open) = before.rfind('(') else {
            return Ok(None);
        };
        if before[open..].contains(')') {
            return Ok(None);
        }
        let commas = before[open..].matches(',').count();
        let label = "(obj: Any, *, skipkeys: bool = False, **kwds: Any) -> str";
        let param = |p: &str| {
            let start = label.find(p).unwrap();
            ParameterInfo {
                label: start..start + p.len(),
                documentation: None,
            }
        };
        Ok(Some(SignatureHelp {
            signatures: vec![Signature {
                label: label.into(),
                documentation: None,
                parameters: vec![
                    param("obj: Any"),
                    param("*"),
                    param("skipkeys: bool = False"),
                    param("**kwds: Any"),
                ],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: Some(if commas == 0 { 0 } else { commas + 1 }),
        }))
    });
}

fn signature_triggers(h: &H<'_>) -> Vec<(Point, SignatureTrigger)> {
    h.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::SignatureHelp { at, context, .. } => Some((at, context.trigger)),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn parameter_info_above_the_call_follows_the_caret(cx: &mut TestAppContext) {
    let mut h = H::new(
        cx,
        &[("app.py", "import json\njson.dumps(1, skipkeys=True)\n")],
    );
    h.open("app.py", "dumps(");
    answer_signatures(&mut h);
    h.invoke("code.parameter-info", json!(null));
    let signature = h.signature();
    assert_eq!(signature["shown"], true);
    assert_eq!(signature["lines"], json!(["obj, *, skipkeys, kwds"]));
    assert_eq!(signature["current"], "obj");
    assert_eq!(signature["active_parameter"], 0);
    // Above the call's line, at its bracket; the focus stays in the editor.
    let popup = h.bounds("parameter-info-popup").expect("drawn");
    let caret = h.caret_bounds();
    assert!(popup.bottom() <= caret.top() + px(1.0));
    assert!(h.editor_focused());

    // The caret moves to the next argument: the popup follows.
    h.caret_after("1, ");
    assert_eq!(h.signature()["current"], "skipkeys");
    assert_eq!(
        signature_triggers(&h).last().unwrap(),
        &(Point::new(1, 14), SignatureTrigger::ContentChange)
    );
    // The caret leaves the call: it closes.
    h.caret_after("json.");
    assert!(h.signature().is_null());

    // Escape closes it.
    h.caret_after("dumps(");
    h.invoke("code.parameter-info", json!(null));
    assert_eq!(h.signature()["shown"], true);
    h.press("escape");
    assert!(h.signature().is_null());
    assert_eq!(h.text(), "import json\njson.dumps(1, skipkeys=True)\n");
}

#[gpui::test]
fn parameter_info_opens_a_second_after_the_bracket(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "import json\njson.dumps\n")]);
    h.open("app.py", "json.dumps");
    answer_signatures(&mut h);
    h.type_text("(");
    h.wait(999);
    assert!(signature_triggers(&h).is_empty());
    assert!(h.signature().is_null());
    h.wait(1);
    // Asked after the bracket the editor inserted with its pair.
    let triggers = signature_triggers(&h);
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].1, SignatureTrigger::Character('('));
    assert_eq!(h.signature()["current"], "obj");

    // Typing on: it follows.
    h.type_text("1,");
    assert_eq!(h.signature()["current"], "skipkeys");

    // A bracket typed and the caret gone before the delay: nothing more.
    h.press("escape");
    let before = signature_triggers(&h).len();
    h.caret_after("import json");
    h.type_text("(");
    h.press("left");
    h.press("left");
    h.wait(1000);
    assert_eq!(signature_triggers(&h).len(), before);
}

#[gpui::test]
fn other_overloads_show_when_asked_for(cx: &mut TestAppContext) {
    let mut h = H::new(cx, &[("app.py", "f(1)\n")]);
    h.open("app.py", "f(");
    h.fake.on_signature_help(|_, _, _| {
        let signature = |label: &str| Signature {
            label: label.into(),
            documentation: None,
            parameters: vec![ParameterInfo {
                label: 1..2,
                documentation: None,
            }],
            active_parameter: None,
        };
        Ok(Some(SignatureHelp {
            signatures: vec![signature("(a: int)"), signature("(b: str)")],
            active_signature: 1,
            active_parameter: Some(0),
        }))
    });
    // The active overload only, and a link to the others.
    h.invoke("code.parameter-info", json!(null));
    let signature = h.signature();
    assert_eq!(signature["lines"], json!(["b"]));
    assert_eq!(signature["link"], "Show 1 more overload");
    assert!(h.bounds("parameter-info-overloads").is_some());
    // Parameter Info again at the same place shows them all, and again
    // hides them; so does the link.
    h.invoke("code.parameter-info", json!(null));
    let signature = h.signature();
    assert_eq!(signature["lines"], json!(["a", "b"]));
    assert_eq!(signature["link"], "Show less");
    assert_eq!(signature["current"], "b");
    h.invoke("code.parameter-info", json!(null));
    assert_eq!(h.signature()["lines"], json!(["b"]));
    let link = h.bounds("parameter-info-overloads").unwrap();
    let mut vcx = VisualTestContext::from_window(h.window.into(), h.cx);
    vcx.simulate_click(link.center(), Modifiers::default());
    h.draw();
    assert_eq!(h.signature()["lines"], json!(["a", "b"]));
    assert!(h.editor_focused());
}
