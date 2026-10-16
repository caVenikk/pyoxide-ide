//! Every binding of both default keymaps, pressed as real keystrokes
//! (`simulate_keystrokes`) in a headless main window: the keystroke must
//! resolve to its command in its key context, and the command must have
//! its effect. A binding without a check here fails the test, so a new
//! binding comes with its check.

mod support;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::fake::FakeCodeIntel;
use code_intel::{CodeIntel, CompletionItem, Completions};
use editor::Editor;
use gpui::{AppContext as _, ClipboardItem, Entity, Focusable as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use text::{Buffer, Selection, SelectionSet};
use ui::icons::IconName;
use workbench::keymap::{self, Binding, Keymap};
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Product, Workbench};

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

    /// The completion popup's state (`null` without one).
    fn completion(&mut self) -> Value {
        self.invoke("code.state", json!(null))["completion"].clone()
    }

    /// Opens the completion popup in the active editor at the marked
    /// caret.
    fn open_completion(&mut self, marked: &str) {
        self.set_editor(marked);
        self.invoke("code.complete", json!(null));
        assert_eq!(self.completion()["shown"], json!(true), "the popup shows");
    }

    /// Presses and releases a mouse button over the editor.
    fn click_button(&mut self, button: gpui::MouseButton) {
        let at = gpui::point(gpui::px(700.0), gpui::px(400.0));
        let mut cx = gpui::VisualTestContext::from_window(self.window.into(), self.cx);
        cx.simulate_mouse_down(at, button, gpui::Modifiers::none());
        cx.simulate_mouse_up(at, button, gpui::Modifiers::none());
        self.draw();
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

    /// The Run console, or the Run tool window itself, gets the focus.
    fn focus_run(&mut self, console: bool) {
        let run = self.workbench(|w, _| w.run_window().clone());
        let handle = self.cx.update(|cx| {
            if console {
                run.read(cx).content_focus(cx)
            } else {
                run.focus_handle(cx)
            }
        });
        self.focus(handle);
    }

    /// The Run/Debug Configurations dialog's state.
    fn run_dialog(&mut self) -> Value {
        self.run()["edit_dialog"].clone()
    }

    /// Opens the Run/Debug Configurations dialog with a new "Script"
    /// configuration selected and the tree focused.
    fn open_run_dialog(&mut self) {
        self.invoke("run.configurations.edit", json!(null));
        self.invoke("run.configurations.dialog.add", json!({"type": "script"}));
        self.focus_run_dialog(None);
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

    fn run_dialog_selected(&mut self) -> Value {
        self.run_dialog()["selected"]["name"].clone()
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

    fn run_focused(&mut self) -> bool {
        let run = self.workbench(|w, _| w.run_window().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                run.focus_handle(cx).contains_focused(window, cx)
            })
            .unwrap()
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

    fn tree_rows(&mut self) -> Vec<Value> {
        self.state()["tree"]["rows"].as_array().unwrap().clone()
    }

    fn tree_row(&mut self, path: &str) -> Value {
        self.tree_rows()
            .into_iter()
            .find(|r| r["path"] == json!(path))
            .unwrap_or_else(|| panic!("no tree row {path}"))
    }

    fn select_in_tree(&mut self, path: &str) {
        self.invoke("project.tree.reveal", json!({ "path": path }));
        self.focus_tree();
    }

    /// Opens the context menu of a tree row; nothing is selected in it.
    fn open_menu(&mut self, path: &str) {
        self.select_in_tree(path);
        self.invoke("project.tree.context-menu", json!(null));
        assert_eq!(self.menu_levels().len(), 1, "the context menu is open");
    }

    fn menu_levels(&mut self) -> Vec<Value> {
        self.state()["tree"]["context_menu"]["levels"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn find_state(&mut self) -> Value {
        self.state()["find"].clone()
    }

    fn query_focused(&mut self) -> bool {
        let bar = self
            .workbench(|w, _| w.active_find_bar().cloned())
            .expect("a find bar");
        let field = self.cx.update(|cx| bar.read(cx).query_field().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn find_bar(&mut self) -> Entity<editor::FindBar> {
        self.workbench(|w, _| w.active_find_bar().cloned())
            .expect("a find bar")
    }

    fn replace_focused(&mut self) -> bool {
        let bar = self.find_bar();
        let field = self.cx.update(|cx| bar.read(cx).replace_field().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    fn focus_replace(&mut self) {
        let bar = self.find_bar();
        let handle = self
            .cx
            .update(|cx| bar.read(cx).replace_field().focus_handle(cx));
        self.focus(handle);
    }

    /// Opens the find bar in replace mode with a plain search for `text`
    /// and `replace` in the replace field; the focus is in the search field.
    fn search_replace(&mut self, text: &str, replace: &str) {
        self.invoke(
            "find.search",
            json!({ "text": text, "case_sensitive": false, "whole_words": false, "regex": false, "replace": replace }),
        );
        assert_eq!(self.find_state()["replace_mode"], json!(true));
    }

    /// Opens the find bar with a plain, case-insensitive search for
    /// `text`; the focus is in its field.
    fn search(&mut self, text: &str) {
        self.invoke(
            "find.search",
            json!({ "text": text, "case_sensitive": false, "whole_words": false, "regex": false }),
        );
        assert!(self.query_focused(), "the search field has the focus");
    }

    fn popup(&mut self) -> Value {
        self.invoke("find-in-files.state", json!(null))
    }

    /// Searches the project with the Find in Files popup (plain, without a
    /// mask); the focus is in its search field.
    /// Waits for the popup's preview and puts the focus and the caret in
    /// it.
    fn focus_preview(&mut self, caret: usize) {
        self.cx
            .executor()
            .advance_clock(workbench::find_in_files::PREVIEW_DELAY * 2);
        self.draw();
        let popup = self.workbench(|w, _| w.find_in_files().clone());
        let preview = self.cx.update(|cx| popup.read(cx).preview_editor().clone());
        self.cx.update(|cx| {
            preview.update(cx, |e, cx| e.set_selections(SelectionSet::caret(caret), cx))
        });
        let handle = self.cx.update(|cx| preview.focus_handle(cx));
        self.focus(handle);
    }

    /// The preview's selection, as text.
    fn preview_selection(&mut self) -> String {
        let popup = self.workbench(|w, _| w.find_in_files().clone());
        self.cx.update(|cx| {
            let e = popup.read(cx).preview_editor().read(cx);
            e.buffer().slice(e.selections().primary().range())
        })
    }

    fn se(&mut self) -> Value {
        self.invoke("search-everywhere.state", json!(null))
    }

    /// Opens Search Everywhere on a tab with a text, searched.
    fn search_everywhere(&mut self, tab: &str, text: &str) {
        self.invoke(
            "search-everywhere.search",
            json!({ "text": text, "tab": tab }),
        );
        self.cx
            .executor()
            .advance_clock(workbench::search_everywhere::DEBOUNCE * 2);
        self.draw();
    }

    /// Shift down and up twice, quickly (a while after the last key: a tap
    /// right after typing does not count).
    fn double_shift(&mut self) {
        self.cx
            .executor()
            .advance_clock(std::time::Duration::from_millis(200));
        for _ in 0..2 {
            for modifiers in [gpui::Modifiers::shift(), gpui::Modifiers::none()] {
                gpui::VisualTestContext::from_window(self.window.into(), self.cx)
                    .simulate_modifiers_change(modifiers);
                self.cx
                    .executor()
                    .advance_clock(std::time::Duration::from_millis(50));
            }
        }
        self.draw();
    }

    fn search_in_files(&mut self, text: &str) {
        self.invoke(
            "find-in-files.search",
            json!({ "text": text, "case_sensitive": false, "whole_words": false, "regex": false, "mask": "" }),
        );
        let popup = self.workbench(|w, _| w.find_in_files().clone());
        let field = self.cx.update(|cx| popup.read(cx).query_field().clone());
        let focused = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap();
        assert!(focused, "the popup's search field has the focus");
    }

    /// The Settings window, when it is open.
    fn settings_window(&mut self) -> Option<gpui::WindowHandle<settings_ui::SettingsWindow>> {
        self.cx.update(|cx| settings_ui::find(cx))
    }

    /// Opens the Settings window on `page` and gives the focus to its tree
    /// (`tree`), its search field (`search`) or the page's first field
    /// (`page`).
    fn open_settings(
        &mut self,
        page: &str,
        target: &str,
    ) -> gpui::WindowHandle<settings_ui::SettingsWindow> {
        self.invoke("settings.open", json!({ "page": page }));
        self.focus_settings(target)
    }

    /// Gives the focus to the open Settings window's tree, search field or
    /// page field.
    fn focus_settings(&mut self, target: &str) -> gpui::WindowHandle<settings_ui::SettingsWindow> {
        let settings = self.settings_window().expect("the Settings window");
        let focus = self
            .cx
            .update(|cx| {
                let view = settings.read(cx).ok()?;
                match target {
                    "page" => view.page_focus(cx),
                    "search" => Some(view.search_focus(cx)),
                    _ => Some(view.tree_focus()),
                }
            })
            .expect("a focus target");
        self.cx
            .update_window(settings.into(), |_, window, _| window.focus(&focus))
            .unwrap();
        self.draw();
        settings
    }

    /// Presses keys in the Settings window.
    fn press_in_settings(&mut self, keys: &str) {
        let settings = self.settings_window().expect("the Settings window");
        self.dispatched.borrow_mut().clear();
        self.cx.simulate_keystrokes(settings.into(), keys);
        self.draw();
    }

    fn settings_state(&mut self) -> Value {
        self.invoke("settings.state", json!(null))
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
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str) -> (H<'a>, tempfile::TempDir) {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
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

/// Text and caret before and after commands that only edit or move.
fn editing_case(command: &str) -> Option<(&'static str, &'static str)> {
    Some(match command {
        "editor.move-left" => ("ab c|d", "ab |cd"),
        "editor.move-right" => ("ab |cd", "ab c|d"),
        "editor.move-up" => ("ab\ncd|", "ab|\ncd"),
        "editor.move-down" => ("ab|\ncd", "ab\ncd|"),
        "editor.move-word-left" => ("ab cd|", "ab |cd"),
        "editor.move-word-right" => ("|ab", "ab|"),
        "editor.move-line-start" => ("  ab|", "  |ab"),
        "editor.move-line-end" => ("|ab", "ab|"),
        "editor.move-document-start" => ("ab\ncd|", "|ab\ncd"),
        "editor.move-document-end" => ("|ab\ncd", "ab\ncd|"),
        "editor.select-left" => ("ab|", "a›b‹"),
        "editor.select-right" => ("|ab", "‹a›b"),
        "editor.select-up" => ("ab\ncd|", "ab›\ncd‹"),
        "editor.select-down" => ("|ab\ncd", "‹ab\n›cd"),
        "editor.select-word-left" => ("ab cd|", "ab ›cd‹"),
        "editor.select-word-right" => ("|ab", "‹ab›"),
        "editor.select-line-start" => ("  ab|", "  ›ab‹"),
        "editor.select-line-end" => ("|ab", "‹ab›"),
        "editor.select-document-start" => ("ab\ncd|", "›ab\ncd‹"),
        "editor.select-document-end" => ("|ab\ncd", "‹ab\ncd›"),
        "editor.select-all" => ("a|b\ncd", "‹ab\ncd›"),
        "editor.select-word" => ("ab c|d ef", "ab ‹cd› ef"),
        "editor.escape" => ("‹ab›", "ab|"),
        "editor.newline" => ("def f():|", "def f():\n    |"),
        "editor.start-new-line" => ("a|b", "ab\n|"),
        "editor.tab" => ("|ab", "    |ab"),
        "editor.unindent" => ("    a|b", "a|b"),
        "editor.backspace" => ("ab|", "a|"),
        "editor.delete" => ("|ab", "|b"),
        "editor.delete-word-left" => ("ab cd|", "ab |"),
        "editor.delete-word-right" => ("|ab", "|"),
        "editor.delete-line" => ("ab\nc|d\nef", "ab\ne|f"),
        "editor.duplicate" => ("a|b", "ab\na|b"),
        "editor.move-line-up" => ("ab\nc|d", "c|d\nab"),
        "editor.move-line-down" => ("a|b\ncd", "cd\na|b"),
        "editor.move-statement-up" => ("def f():\n    pass\nx| = 1", "x| = 1\ndef f():\n    pass"),
        "editor.move-statement-down" => {
            ("d|ef f():\n    pass\nx = 1", "x = 1\nd|ef f():\n    pass")
        }
        "editor.join-lines" => ("a|b\n    cd", "ab| cd"),
        "editor.toggle-comment" => ("    a|b\ncd", "    # ab\ncd|"),
        _ => return None,
    })
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    if let Some((before, after)) = editing_case(&b.command) {
        h.set_editor(before);
        h.press(keys);
        assert_eq!(h.editor_marked(), after, "{what}");
        return true;
    }
    let in_popup = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("FindInFiles"));
    // The Run tool window and its console.
    let run_context = b.context.as_deref().is_some_and(|c| c.starts_with("Run"));
    let console_context = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("RunConsole"));
    match b.command.as_str() {
        "find.toggle-case" | "find.toggle-words" | "find.toggle-regex" if in_popup => {
            h.search_in_files(" = ");
            let option = match b.command.as_str() {
                "find.toggle-case" => "case_sensitive",
                "find.toggle-words" => "whole_words",
                _ => "regex",
            };
            h.press(keys);
            assert_eq!(h.popup()[option], json!(true), "{what}");
        }
        "find.show-history" if in_popup => {
            for text in ["alpha", "beta"] {
                h.search_in_files(text);
                h.invoke("find-in-files.close", json!(null));
            }
            h.search_in_files("gamma");
            h.press(keys);
            let d = h.popup()["dropdown"].clone();
            assert_eq!(d["kind"], json!("search-history"), "{what}");
            assert_eq!(d["items"][0], json!("beta"), "{what}");
            h.invoke("find.history-close", json!(null));
        }
        "find-in-files.open-replace" => {
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.popup();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["replace_mode"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
        }
        "find-in-files.scope-project"
        | "find-in-files.scope-module"
        | "find-in-files.scope-directory"
        | "find-in-files.scope-named" => {
            h.search_in_files(" = ");
            if b.command.ends_with("project") {
                h.invoke("find-in-files.scope-directory", json!(null));
                h.invoke("find-in-files.open", json!(null));
            }
            h.press(keys);
            let expected = match b.command.as_str() {
                "find-in-files.scope-project" => "project",
                "find-in-files.scope-module" => "module",
                "find-in-files.scope-directory" => "directory",
                _ => "Project Files",
            };
            assert_eq!(h.popup()["scope"], json!(expected), "{what}");
            h.invoke("find-in-files.scope-project", json!(null));
        }
        "find-in-files.toggle-recursive" => {
            h.search_in_files(" = ");
            let before = h.popup()["recursive"].as_bool().unwrap();
            h.press(keys);
            assert_eq!(h.popup()["recursive"], json!(!before), "{what}");
            h.invoke("find-in-files.toggle-recursive", json!(null));
        }
        "find-in-files.next-field" | "find-in-files.previous-field" => {
            // Search and replace fields: either key goes to the other one.
            h.invoke(
                "find-in-files.search",
                json!({ "text": " = ", "replace": "x", "mask": "" }),
            );
            h.press(keys);
            let popup = h.workbench(|w, _| w.find_in_files().clone());
            let field = h.cx.update(|cx| popup.read(cx).replace_field().clone());
            let focused =
                h.cx.update_window(h.window.into(), |_, window, cx| {
                    field.focus_handle(cx).is_focused(window)
                })
                .unwrap();
            assert!(focused, "{what}: the replace field has the focus");
        }
        "find-in-files.replace-selected" | "find-in-files.replace-all" => {
            for file in ["rep1.txt", "rep2.txt"] {
                std::fs::write(h.root.join(file), "qq_token qq_token\n").unwrap();
            }
            h.invoke(
                "find-in-files.search",
                json!({ "text": "qq_token", "replace": "done", "mask": "", "case_sensitive": false, "whole_words": false, "regex": false }),
            );
            assert_eq!(h.popup()["matches"], json!(4));
            h.press(keys);
            if b.command.ends_with("replace-all") {
                h.cx.simulate_prompt_answer("Replace");
                h.draw();
                for file in ["rep1.txt", "rep2.txt"] {
                    let disk = std::fs::read_to_string(h.root.join(file)).unwrap();
                    assert_eq!(disk, "done done\n", "{what}");
                }
            } else {
                let changed = ["rep1.txt", "rep2.txt"]
                    .iter()
                    .filter(|f| {
                        std::fs::read_to_string(h.root.join(f)).unwrap() == "done qq_token\n"
                    })
                    .count();
                assert_eq!(changed, 1, "{what}: the selected match");
                assert_eq!(h.popup()["rows"].as_array().unwrap().len(), 3, "{what}");
            }
            for file in ["rep1.txt", "rep2.txt"] {
                std::fs::remove_file(h.root.join(file)).unwrap();
            }
            if h.popup()["open"] == json!(true) {
                h.invoke("find-in-files.close", json!(null));
            }
        }
        "find-in-files.browse-directory" => {
            h.search_in_files(" = ");
            h.invoke("find-in-files.scope-directory", json!(null));
            let before = h.popup()["browse_requests"].as_u64().unwrap();
            h.press(keys);
            assert_eq!(
                h.popup()["browse_requests"],
                json!(before + 1),
                "{what}: the folder chooser"
            );
            h.invoke("find-in-files.scope-project", json!(null));
        }
        "find.new-line" if in_popup => {
            h.search_in_files("a");
            h.press("end");
            h.press(keys);
            h.cx.simulate_input(h.window.into(), "b");
            h.draw();
            assert_eq!(h.popup()["text"], json!("a\nb"), "{what}");
        }
        "find-in-files.open" => {
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.popup();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
        }
        "find-in-files.close" => {
            h.search_in_files(" = ");
            h.press(keys);
            assert_eq!(h.popup()["open"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes back");
        }
        "find-in-files.next-result"
        | "find-in-files.previous-result"
        | "find-in-files.down"
        | "find-in-files.up"
        | "find-in-files.page-down"
        | "find-in-files.page-up" => {
            // The `x = n` lines of the project's files (some may be open
            // with other text).
            h.search_in_files(" = ");
            let n = h.popup()["rows"].as_array().unwrap().len();
            assert!(n >= 3, "{what}: {n} rows");
            let forward = b.command.ends_with("next-result") || b.command.ends_with("down");
            if !forward {
                h.invoke("find-in-files.page-down", json!(null));
                assert_eq!(h.popup()["selected"], json!(n - 1));
            }
            h.press(keys);
            let expected = match b.command.as_str() {
                "find-in-files.page-down" => n - 1,
                "find-in-files.page-up" => 0,
                _ if forward => 1,
                _ => n - 2,
            };
            assert_eq!(h.popup()["selected"], json!(expected), "{what}");
        }
        "find-in-files.open-result" => {
            h.search_in_files(" = ");
            let row = h.popup()["rows"][1].clone();
            h.invoke("find-in-files.next-result", json!(null));
            h.press(keys);
            assert_eq!(h.popup()["open"], json!(false), "{what}");
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], row["path"], "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "find-in-files.edit-source" => {
            h.search_in_files(" = ");
            let row = h.popup()["rows"][1].clone();
            h.invoke("find-in-files.next-result", json!(null));
            h.press(keys);
            assert_eq!(h.popup()["open"], json!(false), "{what}");
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], row["path"], "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "find.next" | "find.previous"
            if b.context.as_deref() == Some("FindInFilesPreview > Editor") =>
        {
            // The editor's Find Next in the preview: the next `=` of the
            // last search in a file ("x = 1"); the rows stay.
            h.cx.update(|cx| {
                cx.set_global(editor::find::LastSearch {
                    text: "=".into(),
                    performed: true,
                    ..Default::default()
                })
            });
            h.search_in_files(" = ");
            let forward = b.command == "find.next";
            h.focus_preview(if forward { 0 } else { 5 });
            let selected = h.popup()["selected"].clone();
            h.press(keys);
            assert_eq!(h.preview_selection(), "=", "{what}");
            assert_eq!(h.popup()["selected"], selected, "{what}");
            h.cx.update(|cx| cx.set_global(editor::find::LastSearch::default()));
        }
        "search-everywhere.open" => {
            assert_eq!(keys, "shift shift", "{what}");
            h.double_shift();
            let s = h.se();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["tab"], json!("all"), "{what}");
        }
        "search-everywhere.go-to-file"
        | "search-everywhere.go-to-action"
        | "search-everywhere.go-to-text" => {
            h.press(keys);
            let s = h.se();
            assert_eq!(s["open"], json!(true), "{what}");
            let tab = b.command.rsplit('-').next().unwrap();
            let tab = match tab {
                "file" => "files",
                "action" => "actions",
                other => other,
            };
            assert_eq!(s["tab"], json!(tab), "{what}");
        }
        "search-everywhere.toggle-case"
        | "search-everywhere.toggle-words"
        | "search-everywhere.toggle-regex" => {
            h.search_everywhere("text", " = ");
            let option = match b.command.as_str() {
                "search-everywhere.toggle-case" => "case_sensitive",
                "search-everywhere.toggle-words" => "whole_words",
                _ => "regex",
            };
            h.press(keys);
            assert_eq!(h.se()["options"][option], json!(true), "{what}");
            // Back off: the options are shared with Find in Files.
            h.press(keys);
            assert_eq!(h.se()["options"][option], json!(false), "{what}");
        }
        "recent-files.open" => {
            h.press(keys);
            let s = h.invoke("recent-files.state", json!(null));
            assert_eq!(s["open"], json!(true), "{what}");
        }
        "recent-files.down" | "recent-files.up" | "recent-files.left" | "recent-files.right" => {
            h.invoke("recent-files.open", json!(null));
            let rf = |h: &mut H| h.invoke("recent-files.state", json!(null));
            assert_eq!(rf(h)["selected"], json!(1), "{what}");
            match b.command.as_str() {
                "recent-files.down" => {
                    h.press(keys);
                    assert_eq!(rf(h)["selected"], json!(2), "{what}");
                }
                "recent-files.up" => {
                    h.press(keys);
                    assert_eq!(rf(h)["selected"], json!(0), "{what}");
                }
                "recent-files.left" => {
                    h.press(keys);
                    assert_eq!(rf(h)["list"], json!("tool_windows"), "{what}");
                }
                _ => {
                    h.invoke("recent-files.left", json!(null));
                    h.press(keys);
                    assert_eq!(rf(h)["list"], json!("files"), "{what}");
                }
            }
        }
        "recent-files.select" => {
            h.invoke("recent-files.open", json!(null));
            let file = h.invoke("recent-files.state", json!(null))["files"][1].clone();
            h.press(keys);
            let s = h.state();
            assert_eq!(s["recent_files"], json!(false), "{what}");
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], file, "{what}");
        }
        "recent-files.close" => {
            h.invoke("recent-files.open", json!(null));
            h.press(keys);
            assert_eq!(h.state()["recent_files"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes back");
        }
        "recent-files.delete" => {
            h.invoke("recent-files.open", json!(null));
            let before = h.invoke("recent-files.state", json!(null))["files"]
                .as_array()
                .unwrap()
                .len();
            h.press(keys);
            let after = h.invoke("recent-files.state", json!(null))["files"]
                .as_array()
                .unwrap()
                .len();
            assert_eq!(after, before - 1, "{what}");
        }
        "search-everywhere.next-tab" | "search-everywhere.previous-tab" => {
            h.search_everywhere("files", "a.py");
            h.press(keys);
            let tab = if b.command.ends_with("next-tab") {
                "actions"
            } else {
                "all"
            };
            assert_eq!(h.se()["tab"], json!(tab), "{what}");
        }
        "search-everywhere.down"
        | "search-everywhere.up"
        | "search-everywhere.next-group"
        | "search-everywhere.previous-group" => {
            // The `.py` files: a.py, b.py, m.py, n.py.
            h.search_everywhere("files", ".py");
            let n = h.se()["rows"].as_array().unwrap().len();
            assert!(n >= 3, "{what}: {n} rows");
            h.press(keys);
            let expected = match b.command.as_str() {
                "search-everywhere.down" => 1,
                "search-everywhere.up" | "search-everywhere.next-group" => n - 1,
                _ => 0,
            };
            assert_eq!(h.se()["selected"], json!(expected), "{what}");
        }
        "search-everywhere.select" => {
            h.search_everywhere("files", "n.py:1:3");
            h.press(keys);
            assert_eq!(h.se()["open"], json!(false), "{what}");
            h.draw();
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert!(
                s["tabs"][active]["path"]
                    .as_str()
                    .unwrap()
                    .ends_with("n.py"),
                "{what}"
            );
            let editor = h.editor();
            let caret = h.cx.update(|cx| editor.read(cx).caret_point());
            assert_eq!((caret.line, caret.column), (0, 2), "{what}");
        }
        "search-everywhere.close" => {
            h.search_everywhere("files", "a.py");
            h.press(keys);
            assert_eq!(h.se()["open"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes back");
        }
        "search-everywhere.history-previous" | "search-everywhere.history-next" => {
            for text in ["alpha", "beta"] {
                h.search_everywhere("files", text);
                h.invoke("search-everywhere.close", json!(null));
            }
            h.search_everywhere("files", "gamma");
            // Previous: from the newest back; next: from the oldest on.
            h.press(keys);
            let first = h.se()["text"].clone();
            h.press(keys);
            let second = h.se()["text"].clone();
            if b.command.ends_with("previous") {
                assert_eq!((first, second), (json!("beta"), json!("alpha")), "{what}");
            } else {
                assert_ne!(first, second, "{what}");
            }
        }
        "search-everywhere.choose-scope" => {
            h.search_everywhere("files", "a.py");
            h.press(keys);
            assert_eq!(h.se()["dropdown"], json!("scope"), "{what}");
            h.invoke("search-everywhere.close", json!(null));
        }
        "search-everywhere.toggle-everywhere" => {
            h.search_everywhere("all", "a.py");
            h.press(keys);
            assert_eq!(h.se()["everywhere"], json!(true), "{what}");
        }
        "find-in-files.toggle-mask" => {
            h.search_in_files(" = ");
            h.press(keys);
            assert_eq!(h.popup()["mask"]["on"], json!(true), "{what}");
            h.invoke("find-in-files.toggle-mask", json!(null));
        }
        "editor.page-up"
        | "editor.page-down"
        | "editor.select-page-up"
        | "editor.select-page-down" => {
            let text: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
            let mut marked = text.join("\n");
            let at = marked.find("line 150").unwrap();
            marked.insert(at, CARET);
            h.set_editor(&marked);
            h.press(keys);
            let (anchor, head) = h.caret_line();
            let down = b.command.ends_with("down");
            assert!(
                if down { head > 155 } else { head < 145 },
                "{what}: caret on line {head}"
            );
            let select = b.command.contains("select");
            assert_eq!(anchor == 150, select, "{what}: anchor on line {anchor}");
        }
        "editor.shrink-selection" => {
            h.set_editor("ab c|d");
            h.invoke("editor.select-word", json!(null));
            assert_eq!(h.editor_marked(), "ab ‹cd›");
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "ab c|d", "{what}");
        }
        "editor.undo" => {
            h.set_editor("ab|");
            h.invoke("editor.type", json!({ "text": "x" }));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "ab|", "{what}");
        }
        "editor.redo" => {
            h.set_editor("ab|");
            h.invoke("editor.type", json!({ "text": "x" }));
            h.invoke("editor.undo", json!(null));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "abx|", "{what}");
        }
        "editor.copy" | "editor.cut" => {
            h.set_editor("a|b\ncd");
            h.press(keys);
            let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert_eq!(copied.as_deref(), Some("ab\n"), "{what}");
            let after = if b.command == "editor.cut" {
                "c|d"
            } else {
                "a|b\ncd"
            };
            assert_eq!(h.editor_marked(), after, "{what}");
        }
        "editor.paste" => {
            h.cx.write_to_clipboard(ClipboardItem::new_string("xy".into()));
            h.set_editor("a|b");
            h.press(keys);
            assert_eq!(h.editor_marked(), "axy|b", "{what}");
        }
        "workbench.go-to-line" => {
            h.set_editor("a\nb|\nc");
            h.press(keys);
            assert_eq!(h.state()["go_to_line"], json!("2:2"), "{what}");
        }
        "code.next-problem" | "code.previous-problem" => {
            // Errors before the nearer warning; forward from the caret, or
            // back around the start.
            h.set_editor("a = 1|\nb = undefined\nc = x == None");
            let editor = h.editor();
            h.cx.update(|cx| {
                editor.update(cx, |e, cx| {
                    let problem =
                        |range: std::ops::Range<usize>, severity| editor::DiagnosticMark {
                            range,
                            severity,
                            unnecessary: false,
                            deprecated: false,
                            messages: Vec::new(),
                        };
                    e.set_diagnostics(
                        vec![
                            problem(0..1, editor::MarkSeverity::Warning),
                            problem(10..19, editor::MarkSeverity::Error),
                            problem(24..33, editor::MarkSeverity::Error),
                        ],
                        cx,
                    );
                })
            });
            h.press(keys);
            let expected = if b.command == "code.next-problem" {
                "a = 1\nb = |undefined\nc = x == None"
            } else {
                "a = 1\nb = undefined\nc = |x == None"
            };
            assert_eq!(h.editor_marked(), expected, "{what}");
        }
        "go-to-line.confirm" => {
            h.set_editor("a\nb\nc|");
            h.invoke("workbench.go-to-line", json!(null));
            // The field's text is selected; typing replaces it.
            h.cx.simulate_input(h.window.into(), "2:2");
            h.press(keys);
            assert_eq!(h.state()["go_to_line"], Value::Null, "{what}");
            assert_eq!(h.editor_marked(), "a\nb|\nc", "{what}");
            assert!(h.editor_focused(), "{what}: the editor has the focus");
        }
        "go-to-line.cancel" => {
            h.set_editor("a\nb\nc|");
            h.invoke("workbench.go-to-line", json!(null));
            h.press(keys);
            assert_eq!(h.state()["go_to_line"], Value::Null, "{what}");
            assert_eq!(h.editor_marked(), "a\nb\nc|", "{what}");
            assert!(h.editor_focused(), "{what}: the editor has the focus");
        }
        "find.open" => {
            // The selected text becomes the query, selected in the field.
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
            assert_eq!(s["matches"], json!(2), "{what}");
            assert_eq!(s["current"], json!(1), "{what}");
            assert!(h.query_focused(), "{what}: the field has the focus");
        }
        "find.replace" => {
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["replace_mode"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
            assert!(h.query_focused(), "{what}: the search field has the focus");
        }
        "find.next" | "find.previous" | "find.down" | "find.up" => {
            let forward = b.command == "find.next" || b.command == "find.down";
            h.set_editor(if forward {
                "|foo foo foo"
            } else {
                "foo foo foo|"
            });
            h.search("foo");
            let start = if forward {
                "‹foo› foo foo"
            } else {
                "foo foo ‹foo›"
            };
            assert_eq!(h.editor_marked(), start, "{what}");
            if b.context.as_deref() != Some("FindBar > Editor") {
                // From the editor, with the bar open.
                h.focus_editor();
            }
            h.press(keys);
            assert_eq!(h.editor_marked(), "foo ‹foo› foo", "{what}");
            assert_eq!(h.find_state()["current"], json!(2), "{what}");
        }
        "find.close" => {
            h.set_editor("|foo foo");
            h.search("foo");
            h.press(keys);
            assert_eq!(h.find_state()["open"], json!(false), "{what}");
            // The current match stays selected; the editor has the focus.
            assert_eq!(h.editor_marked(), "‹foo› foo", "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "find.toggle-case" | "find.toggle-words" | "find.toggle-regex" => {
            h.set_editor("|Foo foo food f.o");
            h.search("f.o");
            let option = match b.command.as_str() {
                "find.toggle-case" => "case_sensitive",
                "find.toggle-words" => "whole_words",
                _ => "regex",
            };
            assert_eq!(h.find_state()["matches"], json!(1), "{what}");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s[option], json!(true), "{what}");
            let expected = match option {
                // Case: still only the literal `f.o`.
                "case_sensitive" => 1,
                // Words: `f.o` stands alone.
                "whole_words" => 1,
                // Regex: `f.o` matches Foo, foo, foo(d) and f.o.
                _ => 4,
            };
            assert_eq!(s["matches"], json!(expected), "{what}");
            assert!(h.query_focused(), "{what}: the focus stays in the field");
        }
        "find.toggle-in-selection" => {
            // Replace opened with lines selected: the search stays within them.
            h.set_editor("‹foo\nfoo›\nfoo");
            h.invoke("find.replace", json!(null));
            h.search("foo");
            let s = h.find_state();
            assert_eq!(s["in_selection"], json!(true), "{what}");
            assert_eq!(s["matches"], json!(2), "{what}");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["in_selection"], json!(false), "{what}");
            assert_eq!(s["matches"], json!(3), "{what}");
        }
        "find.replace-current" => {
            h.set_editor("|foo foo");
            h.search_replace("foo", "x");
            if b.context.as_deref() == Some("FindReplace > Editor") {
                h.focus_replace();
            }
            h.press(keys);
            // Replaced; the next match is current and selected.
            assert_eq!(h.editor_marked(), "x ‹foo›", "{what}");
            assert_eq!(h.find_state()["matches"], json!(1), "{what}");
        }
        "find.replace-all" => {
            h.set_editor("|foo foo foo");
            h.search_replace("foo", "x");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x x x|", "{what}");
            assert_eq!(h.find_state()["matches"], json!(0), "{what}");
            // One undo step in the editor brings all of them back.
            h.focus_editor();
            h.invoke("editor.undo", json!(null));
            let undo_state = h.state()["editor"]["can_undo"].clone();
            let text = h.invoke("editor.text", json!(null));
            assert_eq!(text, json!("foo foo foo"), "{what}");
            assert_eq!(undo_state, json!(false), "{what}: one undo step");
        }
        "find.exclude" => {
            h.set_editor("|foo foo foo");
            h.search_replace("foo", "x");
            h.press(keys);
            let s = h.find_state();
            assert_eq!(s["excluded"], json!(1), "{what}");
            assert_eq!(s["current"], json!(2), "{what}");
            // Replace All leaves the excluded match alone.
            h.invoke("find.replace-all", json!(null));
            let text = h.invoke("editor.text", json!(null));
            assert_eq!(text, json!("foo x x"), "{what}");
        }
        "find.new-line" => {
            h.set_editor("|ab\ncd ab\ncd");
            h.search("ab");
            h.press("end");
            h.press(keys);
            h.cx.simulate_input(h.window.into(), "cd");
            h.draw();
            let s = h.find_state();
            assert_eq!(s["text"], json!("ab\ncd"), "{what}");
            assert_eq!(s["matches"], json!(2), "{what}");
        }
        "find.focus-editor" => {
            h.set_editor("|foo foo");
            h.search("foo");
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
            assert_eq!(h.find_state()["open"], json!(true), "{what}");
        }
        "find.switch-field" => {
            h.set_editor("|foo");
            h.search_replace("foo", "x");
            h.press(keys);
            assert!(h.replace_focused(), "{what}: to the replace field");
            h.press(keys);
            assert!(h.query_focused(), "{what}: and back");
        }
        "find.show-history" => {
            h.set_editor("|foo bar");
            h.search("foo");
            h.invoke("find.next", json!(null));
            h.search("bar");
            h.invoke("find.next", json!(null));
            h.focus(h.cx.update(|cx| {
                let bar = h
                    .window
                    .read(cx)
                    .unwrap()
                    .active_find_bar()
                    .cloned()
                    .unwrap();
                bar.read(cx).query_field().focus_handle(cx)
            }));
            h.press(keys);
            let items = h.find_state()["history"]["items"].clone();
            assert_eq!(items[0], json!("bar"), "{what}");
            assert_eq!(items[1], json!("foo"), "{what}");
        }
        "find.history-up" | "find.history-down" | "find.history-pick" | "find.history-close" => {
            h.set_editor("|foo bar");
            h.search("foo");
            h.invoke("find.next", json!(null));
            h.search("bar");
            h.invoke("find.next", json!(null));
            h.invoke("find.show-history", json!(null));
            assert_eq!(h.find_state()["history"]["selected"], json!(0));
            h.press(keys);
            let s = h.find_state();
            match b.command.as_str() {
                "find.history-down" => {
                    assert_eq!(s["history"]["selected"], json!(1), "{what}")
                }
                "find.history-up" => {
                    // From the first item up: the last one.
                    let len = s["history"]["items"].as_array().unwrap().len();
                    assert_eq!(s["history"]["selected"], json!(len - 1), "{what}");
                }
                "find.history-pick" => {
                    assert_eq!(s["history"], Value::Null, "{what}");
                    assert_eq!(s["text"], json!("bar"), "{what}");
                    assert!(h.query_focused(), "{what}");
                }
                _ => {
                    assert_eq!(s["history"], Value::Null, "{what}");
                    assert!(h.query_focused(), "{what}");
                }
            }
        }
        "app.quit" => {
            h.press(keys);
        }
        "workbench.toggle-project" => {
            h.press(keys);
            assert!(h.tree_focused(), "{what}: first press focuses the tree");
            h.press(keys);
            let s = h.state();
            assert_eq!(
                s["tool_windows"]["project"]["visible"],
                json!(false),
                "{what}: second press hides it"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.toggle-run" => {
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["run"]["visible"], json!(true), "{what}");
            assert!(
                h.run_focused(),
                "{what}: first press focuses the Run tool window"
            );
            assert!(!h.editor_focused(), "{what}");
            h.press(keys);
            let s = h.state();
            assert_eq!(
                s["tool_windows"]["run"]["visible"],
                json!(false),
                "{what}: second press hides it"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.hide-tool-window" if run_context => {
            h.invoke("workbench.toggle-run", json!(null));
            h.focus_run(false);
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["run"]["visible"], json!(false), "{what}");
            assert_eq!(
                s["tool_windows"]["project"]["visible"],
                json!(true),
                "{what}"
            );
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.hide-tool-window" => {
            h.focus_tree();
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tool_windows"]["project"]["visible"], json!(false));
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.focus-editor" if run_context => {
            h.start_run(&support::script("echo done", "echo done"), "");
            h.focus_run(console_context);
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
        }
        "workbench.focus-editor" => {
            h.focus_tree();
            h.press(keys);
            assert!(h.editor_focused(), "{what}");
        }
        "run.run" | "run.current-file" => {
            support::set_script(&support::script("echo done", "echo done"));
            h.press(keys);
            let s = h.wait_run("the run", |s| s["tabs"][0]["status"] == json!("terminated"));
            assert_eq!(s["tabs"][0]["title"], json!("a"), "{what}");
            assert_eq!(s["visible"], json!(true), "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "run.configurations" => {
            // Choose Run Configuration opens the run widget's dropdown.
            assert!(h.run()["menu"].is_null(), "{what}");
            h.press(keys);
            assert!(!h.run()["menu"].is_null(), "{what}: the dropdown opens");
        }
        "run.stop" => {
            h.start_run(&support::long_running(), "started");
            h.press(keys);
            h.wait_run("the stop", |s| s["tabs"][0]["status"] != json!("running"));
        }
        "run.rerun" => {
            let first = h.start_run(&support::long_running(), "started");
            h.focus_run(console_context);
            h.press(keys);
            h.wait_run("the rerun", |s| {
                s["tabs"][0]["pid"].is_number() && s["tabs"][0]["pid"] != first
            });
            assert_eq!(h.run()["tabs"].as_array().unwrap().len(), 1, "{what}");
        }
        "run.close-tab" => {
            h.start_run(&support::script("echo done", "echo done"), "");
            h.focus_run(console_context);
            h.press(keys);
            assert!(h.run()["tabs"].as_array().unwrap().is_empty(), "{what}");
        }
        "run.console.previous-link" | "run.console.next-link" => {
            let a = h.root.join("a.py").display().to_string();
            let b_path = h.root.join("b.py").display().to_string();
            h.start_run(
                &support::script(
                    &format!("echo 'at {a}:1'; echo 'at {b_path}:1'"),
                    &format!("echo at {a}:1& echo at {b_path}:1"),
                ),
                "",
            );
            h.focus_run(false);
            h.press(keys);
            // From no link, Down goes to the first link, Up to the last.
            let expected = if b.command.ends_with("next-link") {
                "a.py"
            } else {
                "b.py"
            };
            let path = h.state()["editor"]["path"].as_str().unwrap().to_owned();
            assert!(path.ends_with(expected), "{what}: {path}");
        }
        "run.console.send-eof" => {
            h.start_run(
                &support::script(
                    "echo started; cat; echo done",
                    "echo started& more& echo done",
                ),
                "started",
            );
            h.focus_run(true);
            h.press(keys);
            h.wait_run("the end of the input", |s| {
                s["tabs"][0]["status"] == json!("terminated")
            });
        }
        "run.configurations.dialog.ok"
        | "run.configurations.dialog.cancel"
        | "run.configurations.dialog.modify-options" => {
            let before = h.run()["configurations"]["configurations"]
                .as_array()
                .unwrap()
                .len();
            h.open_run_dialog();
            if b.context.as_deref().is_some_and(|c| c.ends_with("Editor")) {
                h.focus_run_dialog(Some("command"));
            }
            h.press(keys);
            match b.command.as_str() {
                "run.configurations.dialog.modify-options" => {
                    assert!(!h.run_dialog()["menu"].is_null(), "{what}");
                }
                command => {
                    assert!(h.run_dialog().is_null(), "{what}");
                    let count = h.run()["configurations"]["configurations"]
                        .as_array()
                        .unwrap()
                        .len();
                    let expected = if command.ends_with(".ok") {
                        before + 1
                    } else {
                        before
                    };
                    assert_eq!(count, expected, "{what}");
                }
            }
        }
        "settings.open" => {
            h.press(keys);
            let s = h.invoke("settings.state", json!(null));
            assert_eq!(s["open"], json!(true), "{what}");
        }
        "settings.quick-switch" => {
            h.press(keys);
            let popup = h.state()["switch_popup"].clone();
            assert_eq!(popup["title"], json!("Switch"), "{what}: {popup}");
            h.invoke("menu.cancel", json!(null));
            assert!(h.state()["switch_popup"].is_null(), "{what}");
        }
        "settings.ok" | "settings.escape" => {
            let in_field = b.context.as_deref().is_some_and(|c| c.ends_with("Editor"));
            let target = if in_field { "page" } else { "tree" };
            if b.command == "settings.escape" {
                // A search is cleared first; the window stays.
                h.open_settings("system", target);
                h.invoke("settings.search", json!({"text": "idle"}));
                h.focus_settings(target);
                h.press_in_settings(keys);
                let s = h.settings_state();
                assert_eq!(s["open"], json!(true), "{what}: the search goes first");
                assert_eq!(s["search"]["text"], json!(""), "{what}");
            }
            h.invoke("settings.open", json!({"page": "system"}));
            h.invoke(
                "settings.edit",
                json!({"field": "save-on-deactivation", "value": false}),
            );
            h.focus_settings(target);
            h.press_in_settings(keys);
            assert!(h.settings_window().is_none(), "{what}: the window closes");
            let saved = h.workbench(|w, _| w.save_settings().save_on_deactivation);
            assert_eq!(saved, b.command == "settings.escape", "{what}");
        }
        "settings.find" => {
            let in_field = b.context.as_deref().is_some_and(|c| c.ends_with("Editor"));
            h.open_settings("system", if in_field { "page" } else { "tree" });
            h.press_in_settings(keys);
            assert_eq!(
                h.settings_state()["search"]["focused"],
                json!(true),
                "{what}"
            );
        }
        "settings.back" | "settings.forward" => {
            h.open_settings("system", "tree");
            h.invoke("settings.select", json!({"page": "appearance-behavior"}));
            if b.command == "settings.forward" {
                h.invoke("settings.back", json!(null));
            }
            h.focus_settings("tree");
            let before = h.settings_state()["page"].clone();
            h.press_in_settings(keys);
            let after = h.settings_state()["page"].clone();
            let expected = if b.command == "settings.back" {
                json!("system")
            } else {
                json!("appearance-behavior")
            };
            assert_ne!(before, after, "{what}");
            assert_eq!(after, expected, "{what}");
        }
        "settings.select-next" | "settings.select-previous" => {
            let in_search = b
                .context
                .as_deref()
                .is_some_and(|c| c.starts_with("SettingsSearch"));
            let target = if in_search { "search" } else { "tree" };
            let (from, to) = if b.command == "settings.select-next" {
                ("appearance-behavior", "appearance")
            } else {
                ("appearance", "appearance-behavior")
            };
            // System Settings first: its group is expanded.
            h.open_settings("system", target);
            h.invoke("settings.select", json!({ "page": from }));
            h.focus_settings(target);
            h.press_in_settings(keys);
            let s = h.settings_state();
            assert_eq!(s["page"], json!(to), "{what}");
            assert_eq!(
                s["search"]["focused"],
                json!(in_search),
                "{what}: the focus stays"
            );
        }
        "settings.collapse" | "settings.expand" => {
            // System Settings first: its group is expanded.
            h.open_settings("system", "tree");
            h.invoke("settings.select", json!({"page": "appearance-behavior"}));
            h.focus_settings("tree");
            if b.command == "settings.expand" {
                h.invoke("settings.collapse", json!(null));
            }
            let rows = |h: &mut H| h.settings_state()["rows"].as_array().unwrap().len();
            let before = rows(h);
            h.press_in_settings(keys);
            let after = rows(h);
            // Appearance & Behavior with Appearance and System Settings.
            if b.command == "settings.collapse" {
                assert_eq!((before, after), (3, 1), "{what}");
            } else {
                assert_eq!((before, after), (1, 3), "{what}");
            }
        }
        "settings.go-to-page" => {
            let settings = h.open_settings("system", "search");
            h.press_in_settings(keys);
            let field =
                h.cx.update(|cx| settings.read(cx).ok()?.page_focus(cx))
                    .expect("the page's field");
            let focused =
                h.cx.update_window(settings.into(), |_, window, _| field.is_focused(window))
                    .unwrap();
            assert!(focused, "{what}: the page's field has the focus");
        }
        "run.configurations.dialog.next" | "run.configurations.dialog.previous" => {
            h.open_run_dialog();
            let first = h.run_dialog_selected();
            h.invoke("run.configurations.dialog.add", json!({"type": "script"}));
            let second = h.run_dialog_selected();
            let (from, to) = if b.command.ends_with("next") {
                (first, second)
            } else {
                (second, first)
            };
            h.invoke("run.configurations.dialog.select", json!({"name": from}));
            h.focus_run_dialog(None);
            h.press(keys);
            assert_eq!(h.run_dialog_selected(), to, "{what}");
        }
        "run.configurations.dialog.remove-tag" => {
            h.open_run_dialog();
            let tagged = |h: &mut H| -> Vec<String> {
                h.run_dialog()["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|o| o["tag"] == json!(true) && o["checked"] == json!(true))
                    .map(|o| o["id"].as_str().unwrap().to_owned())
                    .collect()
            };
            let option = tagged(h).first().cloned().expect("an option tag");
            let dialog = h
                .workbench(|w, cx| w.run_window().read(cx).edit_dialog().cloned())
                .expect("the dialog is open");
            let handle =
                h.cx.update(|cx| dialog.read(cx).tag_focus(&option).unwrap());
            h.focus(handle);
            h.press(keys);
            assert!(!tagged(h).contains(&option), "{what}");
        }
        "run.configurations.dialog.toggle-option" => {
            h.open_run_dialog();
            if b.context.as_deref().is_some_and(|c| c.ends_with("Editor")) {
                h.focus_run_dialog(Some("command"));
            }
            let option = b
                .args
                .as_ref()
                .and_then(|a| a["option"].as_str())
                .unwrap_or_default()
                .to_owned();
            let checked = |h: &mut H| {
                h.run_dialog()["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|o| o["id"] == json!(option))
                    .map(|o| o["checked"].clone())
            };
            let before = checked(h);
            h.press(keys);
            // The test's Script type has the common options only.
            if option == "allow-parallel" {
                assert_ne!(checked(h), before, "{what}");
            }
        }
        "run.configurations.dialog.expand" | "run.configurations.dialog.collapse" => {
            h.open_run_dialog();
            // The type of the new configuration: collapse hides it, expand
            // shows it again.
            h.invoke(
                "run.configurations.dialog.select",
                json!({"type": "script"}),
            );
            let expand = b.command.ends_with("expand");
            if expand {
                h.invoke("run.configurations.dialog.collapse", json!(null));
            }
            h.focus_run_dialog(None);
            h.press(keys);
            let tree = h.run_dialog()["tree"].clone();
            let node = tree
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["kind"] == json!("type") && n["type"] == json!("script"))
                .cloned()
                .unwrap();
            assert_eq!(node["expanded"], json!(expand), "{what}");
        }
        "run.configurations.dialog.add" => {
            h.open_run_dialog();
            h.press(keys);
            assert_eq!(
                h.run_dialog()["menu"]["title"],
                json!("Add New Configuration"),
                "{what}"
            );
        }
        "run.configurations.dialog.remove" => {
            h.open_run_dialog();
            let name = h.run_dialog_selected();
            h.press(keys);
            let tree = h.run_dialog()["tree"].clone();
            assert!(
                !tree.as_array().unwrap().iter().any(|n| n["text"] == name),
                "{what}"
            );
        }
        "run.configurations.dialog.copy" => {
            h.open_run_dialog();
            let name = h.run_dialog_selected();
            let count = h.run_dialog()["tree"].as_array().unwrap().len();
            h.press(keys);
            assert_ne!(h.run_dialog_selected(), name, "{what}");
            assert_eq!(
                h.run_dialog()["tree"].as_array().unwrap().len(),
                count + 1,
                "{what}"
            );
        }
        "dialog.confirm" | "dialog.cancel" => {
            let first = h.start_run(&support::long_running(), "started");
            h.invoke("run.run", json!(null));
            assert!(!h.state()["dialog"].is_null(), "{what}");
            h.press(keys);
            assert!(h.state()["dialog"].is_null(), "{what}");
            if b.command == "dialog.confirm" {
                h.wait_run("Stop and Rerun", |s| {
                    s["tabs"][0]["pid"].is_number() && s["tabs"][0]["pid"] != first
                });
            } else {
                assert_eq!(h.run()["tabs"][0]["pid"], first, "{what}");
            }
        }
        "workbench.save-all" => {
            h.set_editor("|");
            h.invoke("editor.type", json!({ "text": "x = 9" }));
            h.focus_editor();
            h.press(keys);
            let disk = std::fs::read_to_string(h.root.join("a.py")).unwrap();
            assert_eq!(disk, "x = 9", "{what}");
            std::fs::write(h.root.join("a.py"), "a = 1\n").unwrap();
        }
        "workbench.synchronize" => {
            // Saves a.py, then reloads b.py, changed outside.
            h.set_editor("|");
            h.invoke("editor.type", json!({ "text": "s = 1" }));
            std::fs::write(h.root.join("b.py"), "b = 3\n").unwrap();
            h.focus_editor();
            h.press(keys);
            let disk = std::fs::read_to_string(h.root.join("a.py")).unwrap();
            assert_eq!(disk, "s = 1", "{what}");
            h.invoke("tabs.select", json!({ "index": 1 }));
            let text = h.invoke("editor.text", json!(null));
            assert_eq!(text, json!("b = 3\n"), "{what}");
            std::fs::write(h.root.join("a.py"), "a = 1\n").unwrap();
            std::fs::write(h.root.join("b.py"), "b = 2\n").unwrap();
        }
        "tabs.close" => {
            h.press(keys);
            let s = h.state();
            assert_eq!(s["tabs"].as_array().unwrap().len(), 2, "{what}");
        }
        "tabs.next" => {
            h.press(keys);
            assert_eq!(h.state()["active_tab"], json!(1), "{what}");
        }
        "tabs.previous" => {
            h.press(keys);
            assert_eq!(h.state()["active_tab"], json!(2), "{what}");
        }
        // The reset opens a.py, b.py and c.txt and selects a.py again:
        // Back returns to c.txt, Forward to a.py.
        "workbench.back" | "workbench.forward" => {
            let active =
                |s: &Value| s["tabs"][s["active_tab"].as_u64().unwrap() as usize]["path"].clone();
            if b.command == "workbench.forward" {
                h.invoke("workbench.back", json!(null));
                h.focus_editor();
            }
            match keymap::parse_mouse_button(keys) {
                Some(button) => h.click_button(button),
                None => h.press(keys),
            }
            let expected = if b.command == "workbench.back" {
                "c.txt"
            } else {
                "a.py"
            };
            assert_eq!(active(&h.state()), json!(expected), "{what}");
        }
        "project.tree.up" | "project.tree.down" => {
            h.select_in_tree("src/m.py");
            let rows = h.tree_rows();
            let i = rows
                .iter()
                .position(|r| r["path"] == json!("src/m.py"))
                .unwrap();
            let expected = if b.command.ends_with("up") {
                i - 1
            } else {
                i + 1
            };
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["selected"],
                rows[expected]["path"],
                "{what}"
            );
        }
        "project.tree.expand" => {
            h.select_in_tree("src/pkg");
            h.invoke("project.tree.collapse-all", json!(null));
            h.select_in_tree("src/pkg");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(false));
            h.press(keys);
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true), "{what}");
        }
        "project.tree.collapse" => {
            h.select_in_tree("src/pkg/n.py");
            h.select_in_tree("src/pkg");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true));
            h.press(keys);
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(false), "{what}");
        }
        "project.tree.open" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], json!("src/m.py"), "{what}");
            h.invoke("tabs.close", json!(null));
        }
        "project.tree.collapse-all" => {
            h.select_in_tree("src/pkg/n.py");
            h.press(keys);
            assert_eq!(h.tree_row("src")["expanded"], json!(false), "{what}");
        }
        "project.tree.expand-all" => {
            h.select_in_tree("src/pkg/n.py");
            h.invoke("project.tree.collapse-all", json!(null));
            h.select_in_tree("src");
            h.press(keys);
            assert_eq!(h.tree_row("src")["expanded"], json!(true), "{what}");
            assert_eq!(h.tree_row("src/pkg")["expanded"], json!(true), "{what}");
        }
        "project.tree.context-menu" => {
            h.select_in_tree("src");
            h.press(keys);
            let levels = h.menu_levels();
            assert_eq!(levels[0]["items"][0]["label"], json!("New"), "{what}");
            assert_eq!(
                levels[0]["items"][7]["label"],
                json!("Find in Files…"),
                "{what}"
            );
            assert_eq!(
                levels[0]["items"][17]["label"],
                json!("Mark Directory As"),
                "{what}"
            );
        }
        "project.tree.new" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let menu = h.state()["tree"]["context_menu"].clone();
            assert_eq!(menu["title"], json!("New"), "{what}");
            let items = &menu["levels"][0]["items"];
            assert_eq!(items[0]["label"], json!("File"), "{what}");
            assert_eq!(items[1]["label"], json!("Directory"), "{what}");
        }
        "project.tree.rename" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            assert_eq!(
                h.name_popup()["label"],
                json!("Rename file 'm.py' to:"),
                "{what}"
            );
        }
        "project.tree.delete" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            assert_eq!(
                h.state()["dialog"]["message"],
                json!("Delete file \"m.py\"?"),
                "{what}"
            );
            assert!(h.root.join("src/m.py").exists(), "{what}: only asked");
        }
        "name-input.confirm" => {
            h.select_in_tree("src");
            h.invoke("project.tree.new-directory", json!(null));
            h.invoke("editor.type", json!({ "text": "made" }));
            h.press(keys);
            assert!(h.name_popup().is_null(), "{what}");
            assert!(h.root.join("src/made").is_dir(), "{what}");
            h.invoke("project.tree.delete", json!({ "paths": ["src/made"] }));
            h.invoke("dialog.confirm", json!(null));
            assert!(!h.root.join("src/made").exists(), "{what}");
        }
        "name-input.next-kind" | "name-input.previous-kind" => {
            // Two kinds: either way the second one, going round.
            h.select_in_tree("src");
            h.invoke("project.tree.new-item", json!({ "item": "Python File" }));
            h.press(keys);
            assert_eq!(
                h.name_popup()["selected_kind"],
                json!("Python unit test"),
                "{what}"
            );
        }
        "name-input.next-field" | "name-input.previous-field" => {
            // The Copy dialog of one file: New name, then To directory.
            h.select_in_tree("src/m.py");
            h.invoke("project.tree.copy-element", json!(null));
            let dialog = h.workbench(|w, cx| w.panel().read(cx).transfer_dialog().cloned());
            let dialog = dialog.expect("the Copy dialog");
            h.press(keys);
            let directory =
                h.cx.update(|cx| dialog.read(cx).directory_field().focus_handle(cx));
            let focused =
                h.cx.update_window(h.window.into(), |_, window, _| directory.is_focused(window))
                    .unwrap();
            assert!(focused, "{what}: the directory field has the focus");
        }
        "project.tree.cut" | "project.tree.copy" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["clipboard"],
                json!({ "paths": ["src/m.py"], "cut": b.command.ends_with("cut") }),
                "{what}"
            );
        }
        "project.tree.paste" => {
            h.select_in_tree("src/m.py");
            h.invoke("project.tree.copy", json!(null));
            h.select_in_tree("src/pkg");
            h.press(keys);
            assert_eq!(
                h.state()["tree"]["transfer"]["title"],
                json!("Copy"),
                "{what}"
            );
        }
        "project.tree.move" | "project.tree.copy-element" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let title = if b.command.ends_with("move") {
                "Move"
            } else {
                "Copy"
            };
            assert_eq!(
                h.state()["tree"]["transfer"]["title"],
                json!(title),
                "{what}"
            );
        }
        "project.tree.copy-paths" => {
            h.select_in_tree("src/m.py");
            h.press(keys);
            let text =
                h.cx.update(|cx| cx.read_from_clipboard())
                    .and_then(|c| c.text());
            assert_eq!(
                text,
                Some(h.root.join("src").join("m.py").display().to_string()),
                "{what}"
            );
        }
        "name-input.cancel" => {
            h.select_in_tree("src");
            h.invoke("project.tree.new-file", json!(null));
            assert!(!h.name_popup().is_null(), "{what}");
            h.press(keys);
            assert!(h.name_popup().is_null(), "{what}");
            assert!(h.tree_focused(), "{what}: the tree has the focus back");
        }
        "menu.down" | "menu.up" => {
            h.open_menu("src");
            h.press(keys);
            // Down from no row selects the first one, Up the last one.
            let expected = if b.command == "menu.down" { 0 } else { 17 };
            assert_eq!(h.menu_levels()[0]["selected"], json!(expected), "{what}");
        }
        "menu.open-submenu" => {
            h.open_menu("src");
            h.invoke("menu.up", json!(null));
            h.press(keys);
            let levels = h.menu_levels();
            assert_eq!(levels.len(), 2, "{what}");
            assert_eq!(levels[1]["selected"], json!(0), "{what}");
        }
        "menu.close-submenu" => {
            h.open_menu("src");
            h.invoke("menu.up", json!(null));
            h.invoke("menu.open-submenu", json!(null));
            h.press(keys);
            assert_eq!(h.menu_levels().len(), 1, "{what}");
        }
        "menu.confirm" => {
            h.open_menu("src");
            h.invoke("menu.up", json!(null));
            h.press(keys);
            // On a submenu row: the submenu opens.
            assert_eq!(h.menu_levels().len(), 2, "{what}");
        }
        "menu.cancel" => {
            h.open_menu("src");
            h.press(keys);
            assert!(h.menu_levels().is_empty(), "{what}");
            assert!(h.tree_focused(), "{what}: the tree has the focus back");
        }
        "code.complete" => {
            h.set_editor("x = al|");
            h.press(keys);
            let c = h.completion();
            assert_eq!(c["shown"], json!(true), "{what}");
            assert_eq!(c["items"].as_array().unwrap().len(), 2, "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "completion.accept" => {
            h.open_completion("x = al|pz");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x = alpha|pz", "{what}");
            assert!(h.completion().is_null(), "{what}");
        }
        "completion.replace" => {
            h.open_completion("x = al|pz");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x = alpha|", "{what}");
        }
        "completion.cancel" => {
            h.open_completion("x = al|");
            h.press(keys);
            assert!(h.completion().is_null(), "{what}");
            assert_eq!(h.editor_marked(), "x = al|", "{what}");
        }
        "completion.select-next" | "completion.select-previous" => {
            // Two items: either way the second one, going round.
            h.open_completion("x = al|");
            h.press(keys);
            assert_eq!(h.completion()["selected"], json!(1), "{what}");
            assert_eq!(h.editor_marked(), "x = al|", "{what}");
        }
        "completion.page-down" => {
            h.open_completion("x = |");
            h.press(keys);
            assert_eq!(h.completion()["selected"], json!(11), "{what}");
        }
        "completion.page-up" => {
            h.open_completion("x = |");
            h.invoke("completion.last", json!(null));
            h.press(keys);
            assert_eq!(h.completion()["selected"], json!(31 - 11), "{what}");
        }
        "code.quick-doc"
        | "documentation.close"
        | "code.parameter-info"
        | "parameter-info.close"
        | "code.error-description"
        | "code.apply-first-fix"
        | "code.show-intentions"
        | "code.go-to-declaration"
        | "code.show-usages"
        | "code.reformat"
        | "code.optimize-imports" => check_docs(h, &b.command, keys, &what),
        _ => return false,
    }
    true
}

fn check_keymap(cx: &mut TestAppContext, text: &str) {
    let keymap = Keymap::parse(text).unwrap();
    let (mut h, _dir) = setup(cx, text);
    let mut missing = Vec::new();
    for b in &keymap.bindings {
        eprintln!("checking `{}` -> {}", b.keys, b.command);
        if !check(&mut h, b) {
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

#[gpui::test]
fn every_macos_binding_works(cx: &mut TestAppContext) {
    check_keymap(cx, keymap::MACOS);
}

#[gpui::test]
fn every_windows_and_linux_binding_works(cx: &mut TestAppContext) {
    check_keymap(cx, keymap::DEFAULT);
}

// ---- documentation, parameter info and quick fixes ----------------------

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

/// A warning on `os` of `import os` in the active editor.
fn mark_os(h: &mut H) {
    let editor = h.editor();
    let mark = editor::DiagnosticMark {
        range: 7..9,
        severity: editor::MarkSeverity::Warning,
        unnecessary: true,
        deprecated: false,
        messages: vec![editor::MarkMessage {
            severity: editor::MarkSeverity::Warning,
            text: "`os` imported but unused".into(),
            source: Some("Ruff".into()),
            code: None,
            href: None,
        }],
    };
    h.cx.update(|cx| editor.update(cx, |e, cx| e.set_diagnostics(vec![mark], cx)));
    h.draw();
}

fn check_docs(h: &mut H, command: &str, keys: &str, what: &str) {
    let state = |h: &mut H| h.invoke("code.state", json!(null));
    match command {
        "code.quick-doc" => {
            h.set_editor("x = al|pha");
            h.press(keys);
            assert_eq!(state(h)["popup"]["kind"], "quick-doc", "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "documentation.close" => {
            h.set_editor("x = al|pha");
            h.invoke("code.quick-doc", json!(null));
            assert!(!state(h)["popup"].is_null(), "{what}");
            h.press(keys);
            assert!(state(h)["popup"].is_null(), "{what}");
            assert_eq!(h.editor_marked(), "x = al|pha", "{what}");
        }
        "code.parameter-info" => {
            h.set_editor("f(a, |b)");
            h.press(keys);
            assert_eq!(state(h)["signature"]["current"], "b", "{what}");
            assert!(h.editor_focused(), "{what}: the editor keeps the focus");
        }
        "parameter-info.close" => {
            h.set_editor("f(a, |b)");
            h.invoke("code.parameter-info", json!(null));
            assert_eq!(state(h)["signature"]["shown"], true, "{what}");
            h.press(keys);
            assert!(state(h)["signature"].is_null(), "{what}");
            assert_eq!(h.editor_marked(), "f(a, |b)", "{what}");
        }
        "code.error-description" => {
            h.set_editor("import os|\n");
            mark_os(h);
            h.press(keys);
            assert_eq!(state(h)["popup"]["kind"], "error-description", "{what}");
        }
        "code.apply-first-fix" => {
            h.set_editor("import os|\nx\n");
            mark_os(h);
            h.press(keys);
            assert_eq!(h.editor_marked(), "|x\n", "{what}");
        }
        "code.show-intentions" => {
            h.set_editor("import os|\nx\n");
            mark_os(h);
            h.press(keys);
            assert_eq!(state(h)["list"]["name"], "intentions", "{what}");
            assert_eq!(
                state(h)["list"]["selected"],
                "Remove unused import",
                "{what}"
            );
        }
        "code.go-to-declaration" => {
            h.set_editor("a|bc\nx\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "abc\n|x\n", "{what}");
        }
        "code.show-usages" => {
            h.set_editor("a|bc\nx\n");
            h.press(keys);
            assert_eq!(state(h)["list"]["name"], "usages", "{what}");
            assert_eq!(state(h)["list"]["items"][0]["text"], "x", "{what}");
        }
        "code.reformat" => {
            h.set_editor("x=1|\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "x = 1|\n", "{what}");
        }
        "code.optimize-imports" => {
            h.set_editor("import b|\nimport a\n");
            h.press(keys);
            assert_eq!(h.editor_marked(), "import a\nimport b\n|", "{what}");
        }
        _ => unreachable!("{command}"),
    }
}
