//! The client against real language servers: ruff, ty and basedpyright on
//! a small sample project written by each test. Ignored by default (they
//! need the tools installed); see the crate's README for how to run them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use code_intel::{
    CodeActionContext, CodeActionKind, CodeIntel, CompletionTrigger, Diagnostic, DocumentId,
    Feature, Features, FormatOptions, IntelEvent, Point, ServerState, SignatureContext,
};
use futures::executor::block_on;
use lsp_client::{LspCodeIntel, ServerSpec};
use process::CommandLine;
use serde_json::json;
use text::{Buffer, CaretPlacement, Edit, EditKind, SelectionSet};

const WAIT: Duration = Duration::from_secs(60);

/// A tool's executable: the environment variable, else the name on PATH.
fn tool(variable: &str, name: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        return PathBuf::from(path);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| panic!("{name} not found: set {variable} or put it on PATH"))
}

struct Project(PathBuf);

impl Project {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("lsp-client-real-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        Self(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ruff(root: &Path) -> ServerSpec {
    ServerSpec::new(
        "ruff",
        CommandLine::new(tool("LSP_CLIENT_RUFF", "ruff")).arg("server"),
        root,
    )
    .with_languages(["python"])
    .with_features(Features::of(&[
        Feature::Diagnostics,
        Feature::CodeActions,
        Feature::Formatting,
        Feature::OrganizeImports,
    ]))
}

fn type_server_features() -> Features {
    Features::all()
        .without(Feature::Formatting)
        .without(Feature::OrganizeImports)
}

fn ty(root: &Path) -> ServerSpec {
    ServerSpec::new(
        "ty",
        CommandLine::new(tool("LSP_CLIENT_TY", "ty")).arg("server"),
        root,
    )
    .with_languages(["python"])
    .with_features(type_server_features())
}

fn basedpyright(root: &Path) -> ServerSpec {
    ServerSpec::new(
        "basedpyright",
        CommandLine::new(tool("LSP_CLIENT_BASEDPYRIGHT", "basedpyright-langserver")).arg("--stdio"),
        root,
    )
    .with_languages(["python"])
    .with_features(type_server_features())
    .with_empty_configuration_change()
    .with_settings(json!({
        "basedpyright": {"analysis": {"typeCheckingMode": "standard", "diagnosticMode": "openFilesOnly"}},
    }))
}

struct Editor {
    doc: DocumentId,
    buffer: Buffer,
    version: u64,
}

impl Editor {
    fn open(intel: &dyn CodeIntel, path: PathBuf) -> Self {
        let text = std::fs::read_to_string(&path).unwrap();
        let editor = Self {
            doc: DocumentId::new(path),
            buffer: Buffer::new(&text),
            version: 1,
        };
        intel.did_open(&editor.doc, "python", 1, editor.buffer.rope());
        editor
    }

    fn edit(&mut self, intel: &dyn CodeIntel, edits: Vec<Edit>) {
        let applied = self.buffer.transact(
            &mut SelectionSet::caret(0),
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        self.version += 1;
        intel.did_change(&self.doc, self.version, &applied, self.buffer.rope());
    }

    fn offset(&self, needle: &str) -> usize {
        let byte = self.buffer.text().find(needle).expect("in the text");
        self.buffer.rope().byte_to_char(byte)
    }

    fn point(&self, needle: &str) -> Point {
        self.buffer.offset_to_point(self.offset(needle))
    }
}

/// The first diagnostics of `doc` (in its current version) `accept` takes.
fn diagnostics(
    events: &flume::Receiver<IntelEvent>,
    editor: &Editor,
    accept: impl Fn(&[Diagnostic]) -> bool,
) -> Vec<Diagnostic> {
    let deadline = Instant::now() + WAIT;
    let mut last = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(IntelEvent::Diagnostics {
                doc,
                version,
                items,
            }) if doc == editor.doc && version == editor.version => {
                if accept(&items) {
                    return items;
                }
                last = items;
            }
            Ok(IntelEvent::Status(status)) => {
                if let ServerState::Terminated { error_output } = status.state {
                    panic!("{} terminated: {error_output}", status.id);
                }
            }
            Ok(_) => {}
            Err(_) => panic!("no such diagnostics within {WAIT:?}; last: {last:#?}"),
        }
    }
}

fn has(items: &[Diagnostic], server: &str, code: &str) -> bool {
    items.iter().any(|d| {
        d.server.as_ref().is_some_and(|s| s.as_str() == server) && d.code.as_deref() == Some(code)
    })
}

const SAMPLE: &str = "import os\n\n\ndef greet(name: str) -> str:\n    return \"hi \" + name\n\n\ngreet(1)\nprint(undefined_name)\n";

#[test]
#[ignore = "needs ruff (LSP_CLIENT_RUFF or PATH)"]
fn ruff_diagnostics_quick_fixes_and_formatting() {
    let project = Project::new(
        "ruff",
        &[("main.py", SAMPLE), ("ugly.py", "x=1\nif x :\n  y =  2\n")],
    );
    // Specs come later, as the product resolves the executables.
    let intel = LspCodeIntel::new(Vec::new());
    let events = intel.events();
    let mut main = Editor::open(intel.as_ref(), project.path("main.py"));
    intel.set_specs(vec![ruff(&project.0)]);

    let items = diagnostics(&events, &main, |items| {
        has(items, "ruff", "F401") && has(items, "ruff", "F821")
    });
    let unused = items
        .iter()
        .find(|d| d.code.as_deref() == Some("F401"))
        .unwrap();
    assert_eq!(unused.range, Point::new(0, 7)..Point::new(0, 9));
    assert_eq!(unused.source.as_deref(), Some("Ruff"));
    assert!(unused.unnecessary);
    assert!(
        unused
            .summary()
            .starts_with("`os` imported but unused  Ruff(F401)"),
        "{}",
        unused.summary()
    );

    // The quick fix for it, first.
    let context = CodeActionContext {
        diagnostics: vec![unused.clone()],
        ..CodeActionContext::default()
    };
    let actions = block_on(intel.code_actions(&main.doc, unused.range.clone(), context)).unwrap();
    assert!(actions[0].kind.is_quick_fix(), "{actions:#?}");
    assert!(
        actions[0].title.contains("Remove unused import"),
        "{}",
        actions[0].title
    );
    assert!(actions[0].edit.is_some());
    // It fixes that problem (ruff sends it back in its own form).
    assert_eq!(actions[0].diagnostics.len(), 1, "{actions:#?}");
    assert_eq!(actions[0].diagnostics[0].code.as_deref(), Some("F401"));
    let organize = block_on(intel.code_actions(
        &main.doc,
        Point::new(0, 0)..Point::new(0, 0),
        CodeActionContext::organize_imports(),
    ))
    .unwrap();
    assert!(
        organize
            .iter()
            .all(|a| a.kind.matches(&CodeActionKind::organize_imports()))
    );

    // Removing the import: after the pull delay its problem is gone and
    // the other one moved up a line.
    let end = main.offset("\n\n\ndef") + 1;
    main.edit(intel.as_ref(), vec![Edit::delete(0..end)]);
    let items = diagnostics(&events, &main, |items| {
        !has(items, "ruff", "F401") && has(items, "ruff", "F821")
    });
    let undefined = items
        .iter()
        .find(|d| d.code.as_deref() == Some("F821"))
        .unwrap();
    assert_eq!(undefined.range.start, main.point("undefined_name"));

    // Formatting a badly formatted file.
    let ugly = Editor::open(intel.as_ref(), project.path("ugly.py"));
    let edits = block_on(intel.format(&ugly.doc, None, FormatOptions::default())).unwrap();
    assert!(!edits.is_empty());
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
#[ignore = "needs ruff and ty (LSP_CLIENT_RUFF, LSP_CLIENT_TY or PATH)"]
fn ty_and_ruff_together() {
    let project = Project::new("ty", &[("main.py", SAMPLE)]);
    let intel = LspCodeIntel::new(Vec::new());
    let events = intel.events();
    let mut main = Editor::open(intel.as_ref(), project.path("main.py"));
    intel.set_specs(vec![ruff(&project.0), ty(&project.0)]);

    // Both servers' items in one event.
    let items = diagnostics(&events, &main, |items| {
        has(items, "ruff", "F401") && has(items, "ty", "invalid-argument-type")
    });
    assert!(
        items
            .iter()
            .any(|d| d.server.as_ref().is_some_and(|s| s.as_str() == "ty")
                && d.range.start == Point::new(7, 6))
    );

    // Navigation and documentation from ty only.
    let call = main.point("greet(1)");
    let definition = block_on(intel.definition(&main.doc, call)).unwrap();
    assert_eq!(definition.len(), 1);
    assert_eq!(definition[0].path, project.path("main.py"));
    assert_eq!(definition[0].range.start.line, 3);
    let hover = block_on(intel.hover(&main.doc, call))
        .unwrap()
        .expect("a hover");
    assert!(
        hover.contents.text.contains("greet"),
        "{}",
        hover.contents.text
    );
    let references = block_on(intel.references(&main.doc, call, true)).unwrap();
    assert_eq!(references.len(), 2, "{references:#?}");

    // Completion after typing `os.pa`.
    let at = main.offset("print(");
    main.edit(intel.as_ref(), vec![Edit::insert(at, "os.pa\n")]);
    let after = Point::new(main.point("os.pa").line, 5);
    assert!(intel.completion_triggers(&main.doc).contains(&'.'));
    let completions =
        block_on(intel.completion(&main.doc, after, CompletionTrigger::Invoked)).unwrap();
    let path = completions
        .items
        .iter()
        .find(|i| i.label == "path")
        .expect("os.path offered");
    let path = block_on(intel.resolve_completion(&main.doc, path.clone())).unwrap();
    assert_eq!(path.label, "path");

    // Parameter info inside the call.
    let inside = Point::new(call.line + 1, 6);
    let help =
        block_on(intel.signature_help(&main.doc, inside, SignatureContext::invoked())).unwrap();
    if let Some(help) = help {
        assert!(help.signatures[0].label.contains("name"), "{help:#?}");
    }

    // A restart of ty: its items go, ruff's stay; then they come back.
    // (`os` is used now: ruff's F401 is gone once it pulls again, F821
    // stays.)
    intel.restart(Some(&code_intel::ServerId::new("ty")));
    diagnostics(&events, &main, |items| {
        !items
            .iter()
            .any(|d| d.server.as_ref().is_some_and(|s| s.as_str() == "ty"))
            && has(items, "ruff", "F821")
    });
    diagnostics(&events, &main, |items| {
        has(items, "ty", "invalid-argument-type") && has(items, "ruff", "F821")
    });
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}

#[test]
#[ignore = "needs basedpyright-langserver (LSP_CLIENT_BASEDPYRIGHT or PATH)"]
fn basedpyright_diagnostics_and_hover() {
    let project = Project::new("basedpyright", &[("main.py", SAMPLE)]);
    let intel = LspCodeIntel::new(vec![basedpyright(&project.0)]);
    let events = intel.events();
    let main = Editor::open(intel.as_ref(), project.path("main.py"));
    let items = diagnostics(&events, &main, |items| {
        has(items, "basedpyright", "reportArgumentType")
    });
    assert!(
        items.iter().any(|d| d.range.start == Point::new(7, 6)),
        "{items:#?}"
    );
    let hover = block_on(intel.hover(&main.doc, main.point("greet(1)"))).unwrap();
    assert!(hover.is_some());
    assert_eq!(block_on(intel.shutdown()), Ok(()));
}
