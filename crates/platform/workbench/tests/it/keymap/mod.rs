//! Every binding of both default keymaps, pressed as real keystrokes
//! (`simulate_keystrokes`) in a headless main window: the keystroke must
//! resolve to its command in its key context, and the command must have
//! its effect. A binding without a check here fails the test, so a new
//! binding comes with its check.
//!
//! The keymaps are folders of area files (`keymaps/macos/<area>.toml`,
//! `keymaps/default/<area>.toml`). A module per area starts its own app with
//! the complete keymap and presses only the bindings of its area file, so
//! the areas run in parallel and a failure names the area. This module is
//! the harness they share; a new area file needs a module here (listed in
//! `areas!`) with its `area_tests!` and a `check` of its commands.

use crate::support;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use ::editor::Editor;
use code_intel::fake::FakeCodeIntel;
use code_intel::{CodeIntel, CompletionItem, Completions};
use gpui::{AppContext as _, ClipboardItem, Entity, Focusable as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use text::{Buffer, Selection, SelectionSet};
use ui::icons::IconName;
use workbench::keymap::{self, Binding, Keymap};
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Product, Workbench};

/// The two tests of an area, `every_macos_binding_works` and
/// `every_windows_and_linux_binding_works`, for the area file `$file`; the
/// module defines `check`.
macro_rules! area_tests {
    ($file:literal) => {
        pub(super) const FILE: &str = $file;

        #[gpui::test]
        fn every_macos_binding_works(cx: &mut TestAppContext) {
            super::check_area(
                cx,
                keymap::MACOS,
                include_str!(concat!("../../../keymaps/macos/", $file, ".toml")),
                check,
            );
        }

        #[gpui::test]
        fn every_windows_and_linux_binding_works(cx: &mut TestAppContext) {
            super::check_area(
                cx,
                keymap::DEFAULT,
                include_str!(concat!("../../../keymaps/default/", $file, ".toml")),
                check,
            );
        }
    };
}

/// The area modules, and the files they check.
macro_rules! areas {
    ($($module:ident),* $(,)?) => {
        $(mod $module;)*

        const TESTED: &[&str] = &[$($module::FILE),*];
    };
}

areas! {
    editor,
    code_insight,
    window,
    run,
    debug,
    vcs_markers,
    vcs_branches,
    vcs_commit,
    vcs_diff,
    vcs_push,
    completion,
    navigation,
    find,
    find_in_files,
    search_everywhere,
    console,
    run_configurations,
    project,
    settings,
    breakpoints,
    terminal,
    welcome,
}

/// What the version control areas share.
mod vcs;

/// Caret, or anchor and head of a selection, written into the text.
const CARET: char = '|';
const ANCHOR: char = '‹';
const HEAD: char = '›';

fn parse_marked(marked: &str) -> (String, Selection) {
    let mut text = String::new();
    let (mut caret, mut anchor, mut head) = (None, None, None);
    for c in marked.chars() {
        let pos = text.chars().count();
        match c {
            CARET => caret = Some(pos),
            ANCHOR => anchor = Some(pos),
            HEAD => head = Some(pos),
            c => text.push(c),
        }
    }
    let selection = match (caret, anchor, head) {
        (Some(p), None, None) => Selection::caret(p),
        (None, Some(a), Some(h)) => Selection::new(a, h),
        _ => panic!("bad markers in {marked:?}"),
    };
    (text, selection)
}

fn render_marked(text: &str, s: &Selection) -> String {
    let mut marks = if s.is_empty() {
        vec![(s.head, CARET)]
    } else {
        vec![(s.anchor, ANCHOR), (s.head, HEAD)]
    };
    marks.sort_by_key(|m| std::cmp::Reverse(m.0));
    let mut chars: Vec<char> = text.chars().collect();
    for (pos, mark) in marks {
        chars.insert(pos, mark);
    }
    chars.into_iter().collect()
}

fn python() -> FileType {
    FileType {
        indent_after: &[':'],
        dedent_after: &["return", "pass"],
        line_comment: Some("# "),
        quotes: &['"', '\''],
        triple_quotes: true,
        string_prefixes: &['f', 'r', 'b'],
        language_id: Some("python"),
        ..FileType::plain("Python", &["py"], IconName::FILE_TEXT)
    }
}

/// Language services that complete `alpha`, `alpine` and `b00` to `b29`.
struct Intel(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Intel {
    fn start(&self, _: &Path, _: &mut gpui::App) -> Arc<dyn CodeIntel> {
        let fake = self.0.clone();
        fake.on_completion(|_, _, _| {
            let names = ["alpha".to_owned(), "alpine".to_owned()]
                .into_iter()
                .chain((0..30).map(|i| format!("b{i:02}")));
            Ok(Completions {
                items: names.map(CompletionItem::new).collect(),
                incomplete: false,
            })
        });
        script_docs(&fake);
        fake
    }
}

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    root: PathBuf,
    /// Commands the keystrokes resolved to, in order.
    dispatched: Rc<RefCell<Vec<String>>>,
}

impl H<'_> {
    /// Draws every window (the main one and an open dialog).
    fn draw(&mut self) {
        self.cx.run_until_parked();
        for window in self.cx.update(|cx| cx.windows()) {
            let _ = self
                .cx
                .update_window(window, |_, window, cx| window.draw(cx).clear());
        }
    }

    /// The window that takes input: an open dialog, else the main one.
    fn target(&mut self) -> gpui::AnyWindowHandle {
        let main: gpui::AnyWindowHandle = self.window.into();
        self.cx
            .update(|cx| ui::dialog_window::modal_of(main, cx))
            .unwrap_or(main)
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
        self.invoke("workbench.state", json!(null))
    }

    fn press(&mut self, keys: &str) {
        self.dispatched.borrow_mut().clear();
        let target = self.target();
        self.cx.simulate_keystrokes(target, keys);
        self.draw();
    }

    fn workbench<R>(&mut self, f: impl FnOnce(&Workbench, &gpui::App) -> R) -> R {
        self.window.read_with(self.cx, f).unwrap()
    }

    fn editor(&mut self) -> Entity<Editor> {
        self.workbench(|w, _| w.active_editor().cloned())
            .expect("an editor is open")
    }

    fn focus(&mut self, handle: gpui::FocusHandle) {
        let target = self.target();
        self.cx
            .update_window(target, |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn focus_editor(&mut self) {
        let editor = self.editor();
        let handle = self.cx.update(|cx| editor.focus_handle(cx));
        self.focus(handle);
    }

    fn focus_tree(&mut self) {
        let panel = self.workbench(|w, _| w.panel().clone());
        let handle = self.cx.update(|cx| panel.focus_handle(cx));
        self.focus(handle);
    }

    /// The run state (a query: no frame is drawn).
    fn run(&mut self) -> Value {
        self.cx.run_until_parked();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke("run.state", json!(null), window, cx)
            })
            .unwrap()
            .unwrap()
    }

    /// Waits (at most about 20 s) for a run state that `f` accepts.
    fn wait_run(&mut self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        for _ in 0..2000 {
            support::tick(self.cx);
            last = self.run();
            if f(&last) {
                self.draw();
                return last;
            }
        }
        panic!("timed out waiting for {what}: {last:#}");
    }

    /// Runs a.py with `script` until it prints `marker` (or ends when the
    /// marker is empty); returns the process id while it runs.
    fn start_run(&mut self, script: &str, marker: &str) -> Value {
        support::set_script(script);
        self.invoke("run.run", json!(null));
        if marker.is_empty() {
            self.wait_run("the run to end", |s| {
                s["tabs"][0]["status"] == json!("terminated")
            });
            return Value::Null;
        }
        let s = self.wait_run(marker, |s| {
            s["tabs"][0]["console"]["lines"]
                .as_array()
                .is_some_and(|l| {
                    l.iter()
                        .any(|l| l["text"].as_str().unwrap().contains(marker))
                })
        });
        s["tabs"][0]["pid"].clone()
    }

    /// The Run/Debug Configurations dialog's state.
    fn run_dialog(&mut self) -> Value {
        self.run()["edit_dialog"].clone()
    }

    /// Focuses the dialog's tree, or a field of its form.
    fn focus_run_dialog(&mut self, field: Option<&str>) {
        let dialog = self
            .workbench(|w, cx| w.run_window().read(cx).edit_dialog().cloned())
            .expect("the dialog is open");
        let handle = self.cx.update(|cx| match field {
            Some(id) => dialog.read(cx).field_focus(id, cx).expect("the field"),
            None => dialog.read(cx).tree_focus(),
        });
        self.focus(handle);
    }

    fn focus_dialog(&mut self) {
        let dialog = self.workbench(|w, cx| {
            w.dialog_focus(cx)
                .or_else(|| w.panel().read(cx).dialog().map(|d| d.focus_handle(cx)))
        });
        if let Some(handle) = dialog {
            self.focus(handle);
        }
    }

    /// The tree's New File, New Directory or Rename popup (`null` without
    /// one).
    fn name_popup(&mut self) -> Value {
        self.state()["tree"]["name_popup"].clone()
    }

    fn editor_focused(&mut self) -> bool {
        let editor = self.editor();
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                editor.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn tree_focused(&mut self) -> bool {
        let panel = self.workbench(|w, _| w.panel().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                panel.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
    }

    /// Puts marked text into the active editor and focuses it.
    fn set_editor(&mut self, marked: &str) {
        let (text, selection) = parse_marked(marked);
        let editor = self.editor();
        self.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                *e.buffer_mut() = Buffer::new(&text);
                e.buffer_reloaded(cx);
                e.set_selections(SelectionSet::single(selection), cx);
            })
        });
        self.focus_editor();
    }

    fn editor_marked(&mut self) -> String {
        let editor = self.editor();
        self.cx.update(|cx| {
            let e = editor.read(cx);
            render_marked(&e.text(), e.selections().primary())
        })
    }

    fn caret_line(&mut self) -> (usize, usize) {
        let editor = self.editor();
        self.cx.update(|cx| {
            let e = editor.read(cx);
            let s = e.selections().primary();
            let anchor = e.buffer().offset_to_point(s.anchor).line;
            (anchor, e.caret_point().line)
        })
    }

    fn select_in_tree(&mut self, path: &str) {
        self.invoke("project.tree.reveal", json!({ "path": path }));
        self.focus_tree();
    }

    fn menu_levels(&mut self) -> Vec<Value> {
        self.state()["tree"]["context_menu"]["levels"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// The Settings window, when it is open.
    fn settings_window(&mut self) -> Option<gpui::WindowHandle<settings_ui::SettingsWindow>> {
        self.cx.update(|cx| settings_ui::find(cx))
    }

    /// Three tabs (a.py, b.py, c.txt), the first active, the project tool
    /// window shown, the editor focused.
    fn reset(&mut self) {
        if self.settings_window().is_some() {
            self.invoke("settings.cancel", json!(null));
        }
        self.invoke(
            "settings.set",
            json!({"key": "system.files.save-on-deactivation", "value": true}),
        );
        while !self.state()["tree"]["context_menu"].is_null() {
            self.invoke("menu.cancel", json!(null));
        }
        // No dialog, no run menu, no runs.
        if !self.run_dialog().is_null() {
            if !self.run_dialog()["menu"].is_null() {
                self.invoke("menu.cancel", json!(null));
            }
            self.focus_run_dialog(None);
            self.invoke("run.configurations.dialog.cancel", json!(null));
        }
        if self.run()["configurations"]["selected"] != Value::Null {
            self.invoke("run.select-current-file", json!(null));
        }
        // No name popup, no Copy or Move dialog.
        if !self.state()["tree"]["transfer"].is_null() {
            let dialog = self.workbench(|w, cx| {
                w.panel()
                    .read(cx)
                    .transfer_dialog()
                    .map(|d| d.focus_handle(cx))
            });
            if let Some(handle) = dialog {
                self.focus(handle);
            }
            self.invoke("name-input.cancel", json!(null));
        }
        if !self.name_popup().is_null() {
            let popup =
                self.workbench(|w, cx| w.panel().read(cx).name_popup().map(|p| p.focus_handle(cx)));
            if let Some(handle) = popup {
                self.focus(handle);
            }
            self.invoke("name-input.cancel", json!(null));
        }
        if !self.state()["dialog"].is_null() {
            self.focus_dialog();
            self.invoke("dialog.cancel", json!(null));
        }
        if !self.run()["menu"].is_null() {
            self.invoke("menu.cancel", json!(null));
        }
        if self.run()["alive"] == json!(true) {
            self.invoke("run.stop-all", json!(null));
            self.invoke("run.stop-all", json!(null));
            self.wait_run("no process alive", |s| s["alive"] == json!(false));
        }
        if !self.run()["tabs"].as_array().unwrap().is_empty() {
            self.invoke("run.close-all-tabs", json!(null));
        }
        // No debug session: closing a tab stops its session.
        while self.debug()["tool_window"]["tabs"]
            .as_array()
            .is_some_and(|t| !t.is_empty())
        {
            self.invoke("debug.close-tab", json!({"index": 0}));
        }
        let vcs = self.state()["vcs"].clone();
        if !vcs["branches_popup"].is_null() {
            self.invoke("vcs.branches-popup", json!(null));
        }
        if vcs["branch_dialog"]["buttons"].is_array() {
            self.invoke("dialog.cancel", json!(null));
        } else if !vcs["branch_dialog"].is_null() {
            self.invoke("branch-dialog.cancel", json!(null));
        }
        if !self.state()["vcs"]["marker_popup"].is_null() {
            self.invoke("marker-popup.close", json!(null));
        }
        // No diff tab.
        while let Some(i) = self.state()["tabs"]
            .as_array()
            .and_then(|t| t.iter().position(|t| t.get("view").is_some()))
        {
            self.invoke("tabs.close", json!({ "index": i }));
        }
        let commit = self.state()["vcs"]["commit"].clone();
        if let Some(kind) = commit["dialog"]["kind"].as_str() {
            let focus = self.workbench(|w, cx| w.commit_dialog_focus(cx));
            if let Some(focus) = focus {
                self.focus(focus);
            }
            let cancel = match kind {
                "rollback" => "rollback-dialog.cancel",
                "user_name" => "user-name-dialog.cancel",
                _ => "dialog.cancel",
            };
            self.invoke(cancel, json!(null));
        }
        if commit["options"]["open"] == json!(true) {
            self.invoke("commit-options.close", json!(null));
        }
        if commit["amend"] == json!(true) {
            self.invoke("vcs.commit.toggle-amend", json!(null));
        }
        // No Push or Update Project dialog, no VCS Operations popup.
        if !remote_ui(self)["remote_dialog"].is_null() {
            self.invoke("dialog.cancel", json!(null));
        }
        for _ in 0..3 {
            if remote_ui(self)["push_dialog"].is_null() {
                break;
            }
            self.invoke("push-dialog.cancel", json!(null));
        }
        if !remote_ui(self)["update_dialog"].is_null() {
            self.invoke("update-dialog.cancel", json!(null));
        }
        if !remote_ui(self)["operations_popup"].is_null() {
            self.invoke("vcs.operations-popup", json!(null));
        }
        if self.state()["go_to_line"].is_string() {
            self.invoke("go-to-line.cancel", json!(null));
        }
        if self.state()["find"]["open"] == json!(true) {
            self.invoke("find.close", json!(null));
        }
        if self.state()["find_in_files"] == json!(true) {
            // Closed from its field (it may not have the focus).
            self.invoke("find-in-files.open", json!(null));
            self.invoke("find-in-files.close", json!(null));
        }
        if self.state()["search_everywhere"] == json!(true) {
            self.invoke("search-everywhere.close", json!(null));
        }
        while self.state()["recent_files"] == json!(true) {
            self.invoke("recent-files.close", json!(null));
        }
        if self.state()["tool_windows"]["run"]["visible"] == json!(true) {
            // Focused first, then hidden.
            self.invoke("workbench.toggle-run", json!(null));
            if self.state()["tool_windows"]["run"]["visible"] == json!(true) {
                self.invoke("workbench.toggle-run", json!(null));
            }
        }
        while self.state()["tabs"]
            .as_array()
            .is_some_and(|t| !t.is_empty())
        {
            self.invoke("tabs.close", json!({ "index": 0 }));
        }
        for file in ["a.py", "b.py", "c.txt"] {
            self.invoke("project.open-file", json!({ "path": file }));
        }
        self.invoke("tabs.select", json!({ "index": 0 }));
        if !self.state()["tool_windows"]["project"]["visible"]
            .as_bool()
            .unwrap()
        {
            self.invoke("workbench.toggle-project", json!(null));
        }
        self.focus_editor();
    }

    fn debug(&mut self) -> Value {
        self.invoke("debug.state", json!(null))
    }
}

/// The global config of the test binary's calls of git (no identity): the
/// user name check of the Commit tool window can be reached.
fn isolate_git() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    git::use_global_config(dir.path().join("global-config"));
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str) -> (H<'a>, tempfile::TempDir) {
    isolate_git();
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    for (path, text) in [
        ("a.py", "a = 1\n"),
        ("b.py", "b = 2\n"),
        ("c.txt", "c\n"),
        ("src/m.py", "m = 3\n"),
        ("src/pkg/n.py", "n = 4\n"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        file_types: vec![python()],
        run: Some(support::run_hook()),
        code_intel: Some(CodeIntelHook::new(Intel(FakeCodeIntel::new()))),
        debug: Some(support::debug_hook()),
        new_items: vec![workbench::NewItem {
            label: "Python File",
            icon: IconName::FILE_TEXT,
            title: "New Python File",
            kinds: ["Python file", "Python unit test"]
                .into_iter()
                .map(|label| workbench::NewItemKind {
                    label,
                    icon: IconName::FILE_TEXT,
                    template: workbench::NewTemplate::File {
                        extension: "py",
                        text: "",
                    },
                })
                .collect(),
        }],
        ..Product::default()
    };
    let keymap = Keymap::parse(keymap_text).unwrap();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &keymap);
        assert!(errors.is_empty(), "keymap errors: {errors:?}");
        workbench::open_main_window(cx, Rc::new(product))
    });
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let sink = dispatched.clone();
    cx.update(|cx| {
        cx.observe_keystrokes(move |event, _, _| {
            if let Some(action) = &event.action {
                sink.borrow_mut().push(action.name().to_owned());
            }
        })
        .detach()
    });
    let mut h = H {
        cx,
        window,
        root: root.clone(),
        dispatched,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    // Deleting never fills the system trash.
    let panel = h.workbench(|w, _| w.panel().clone());
    h.cx.update(|cx| panel.update(cx, |p, _| p.set_delete_to_trash(false)));
    h.reset();
    (h, dir)
}

/// Answers of the services for the documentation checks: documentation
/// anywhere, the signature `(a, b)` with the parameter by the column, and a
/// quick fix that removes the first line.
fn script_docs(fake: &FakeCodeIntel) {
    use code_intel::{
        CodeAction, CodeActionKind, FileEdits, Hover, Markup, ParameterInfo, Point, Signature,
        SignatureHelp, TextEdit, WorkspaceEdit,
    };
    fake.on_hover(|_, _| {
        Ok(Some(Hover {
            contents: Markup::markdown("```python\ndef alpha() -> int\n```\n---\nThe first."),
            range: None,
        }))
    });
    fake.on_signature_help(|_, at, _| {
        let parameter = |label| ParameterInfo {
            label,
            documentation: None,
        };
        Ok(Some(SignatureHelp {
            signatures: vec![Signature {
                label: "(a, b)".into(),
                documentation: None,
                parameters: vec![parameter(1..2), parameter(4..5)],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: Some(usize::from(at.column >= 4)),
        }))
    });
    let edit = |doc: &code_intel::DocumentId, edits| {
        Some(WorkspaceEdit {
            files: vec![FileEdits {
                path: doc.path().to_owned(),
                edits,
            }],
        })
    };
    fake.on_code_actions(move |doc, _, context| {
        // Optimize Imports asks for the organize imports action only.
        if context.only.is_some() {
            let mut organize =
                CodeAction::new("Organize imports", CodeActionKind::organize_imports());
            organize.edit = edit(
                doc,
                vec![TextEdit::new(
                    Point::new(0, 0)..Point::new(2, 0),
                    "import a\nimport b\n",
                )],
            );
            return Ok(vec![organize]);
        }
        let mut fix = CodeAction::new("Remove unused import", CodeActionKind::QuickFix);
        fix.edit = edit(
            doc,
            vec![TextEdit::new(Point::new(0, 0)..Point::new(1, 0), "")],
        );
        Ok(vec![fix])
    });
    // The declaration and the usages of anything: the start of line 2.
    let second_line = |doc: &code_intel::DocumentId| code_intel::Location {
        path: doc.path().to_owned(),
        range: Point::new(1, 0)..Point::new(1, 1),
    };
    fake.on_definition(move |doc, _| Ok(vec![second_line(doc)]));
    fake.on_references(move |doc, _, _| Ok(vec![second_line(doc)]));
    fake.on_format(|_, _, _| {
        Ok(vec![TextEdit::new(
            Point::new(0, 1)..Point::new(0, 2),
            " = ",
        )])
    });
}

/// The remote part of `vcs.state`: the Push and Update Project dialogs,
/// Push Rejected and the VCS Operations popup.
fn remote_ui(h: &mut H) -> Value {
    h.invoke("vcs.state", json!(null))["remote"].clone()
}

/// Presses the bindings of one area file (`area_text`) with the complete
/// keymap `keymap_text` installed; `check` presses one and asserts its
/// effect, and returns `false` for a command it has no check for.
fn check_area(
    cx: &mut TestAppContext,
    keymap_text: &str,
    area_text: &str,
    check: fn(&mut H, &Binding) -> bool,
) {
    let (mut h, _dir) = setup(cx, keymap_text);
    press_area(&mut h, area_text, check);
}

/// Presses the bindings of `area_text` in `h`.
fn press_area(h: &mut H, area_text: &str, check: fn(&mut H, &Binding) -> bool) {
    let keymap = Keymap::parse(area_text).unwrap();
    let mut missing = Vec::new();
    for b in &keymap.bindings {
        eprintln!("checking `{}` -> {}", b.keys, b.command);
        if !check(h, b) {
            missing.push(format!("`{}` -> {}", b.keys, b.command));
            continue;
        }
        // A double tap or a mouse button runs its command without a
        // keystroke.
        if keymap::parse_double_tap(&b.keys).is_some()
            || keymap::parse_mouse_button(&b.keys).is_some()
        {
            continue;
        }
        let dispatched = h.dispatched.borrow().clone();
        assert!(
            dispatched.contains(&b.command),
            "`{}` should dispatch {} in context {:?}, dispatched {dispatched:?}",
            b.keys,
            b.command,
            b.context
        );
    }
    assert!(missing.is_empty(), "bindings without a check: {missing:?}");
}

/// Every file of the keymap folders has a module that checks it.
#[test]
fn every_area_file_has_tests() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("keymaps");
    let mut tested: Vec<&str> = TESTED.to_vec();
    tested.sort_unstable();
    assert!(tested.windows(2).all(|w| w[0] != w[1]), "an area twice");
    for keymap in ["macos", "default"] {
        let mut files: Vec<String> = std::fs::read_dir(folder.join(keymap))
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                path.file_stem().unwrap().to_str().unwrap().to_owned()
            })
            .collect();
        files.sort_unstable();
        assert_eq!(files, tested, "keymaps/{keymap}/: the files of the areas");
    }
}
