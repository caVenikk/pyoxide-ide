//! Settings > Editor > Font (tests/scenarios/settings/editor-font.md): the
//! page holds the font of the store over the theme's, its preview follows
//! the fields before Apply, and Apply reaches every editor and the Run
//! console; Restore defaults goes back to JetBrains Mono at 13.
//!
//! The test text system gives every character 0.6 of the font size and an
//! ascent and descent of 1.025 and 0.275 of it.

mod support;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{FileType, Product, Workbench};

use support::{script, set_script, tick};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    /// `settings.toml`.
    file: PathBuf,
    _dir: tempfile::TempDir,
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        for window in self.cx.update(|cx| cx.windows()) {
            let _ = self
                .cx
                .update_window(window, |_, window, cx| window.draw(cx).clear());
        }
    }

    fn try_invoke(&mut self, id: &str, args: Value) -> Result<Value, String> {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .map_err(|e| format!("{e:?}"));
        self.draw();
        out
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        self.try_invoke(id, args)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn edit(&mut self, field: &str, value: Value) {
        self.invoke("settings.edit", json!({"field": field, "value": value}));
    }

    fn page(&mut self) -> Value {
        self.invoke("settings.state", json!(null))["fields"].clone()
    }

    fn editor(&mut self) -> Entity<editor::Editor> {
        self.window
            .read_with(self.cx, |w, _| w.active_editor().cloned())
            .unwrap()
            .expect("an editor")
    }

    fn console(&mut self) -> Entity<editor::Editor> {
        let run = self
            .window
            .read_with(self.cx, |w, _| w.run_window().clone())
            .unwrap();
        self.cx
            .update(|cx| run.read(cx).tabs()[0].console().read(cx).editor().clone())
    }

    /// Line height and character width of an editor's last frame.
    fn metrics(&mut self, editor: &Entity<editor::Editor>) -> (f32, f32) {
        let (_, line, char) = editor.read_with(self.cx, |e, _| e.text_geometry().unwrap());
        (f32::from(line), f32::from(char))
    }

    fn file_text(&mut self) -> String {
        self.cx.executor().advance_clock(Duration::from_secs(1));
        self.draw();
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }
}

fn setup(cx: &mut TestAppContext) -> H<'_> {
    support::allow_processes(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join("demo");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.py"), "a = 1\n").unwrap();
    let file = dir.path().join("config").join("settings.toml");
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        run: Some(support::run_hook()),
        ..Product::default()
    };
    let store = file.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H {
        cx,
        window,
        file,
        _dir: dir,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    h.invoke("project.open-file", json!({"path": "a.py"}));
    h
}

/// Size 13 at line height 1.2: 14 + 4 = 18 high lines times 1.2.
const THEME: (f32, f32) = (22.0, 7.8);
/// Size 20 at line height 1.5: (21 + 6) × 1.5, rounded up.
const CHOSEN: (f32, f32) = (41.0, 12.0);

#[gpui::test]
fn a_font_change_reaches_the_editors_and_the_run_console(cx: &mut TestAppContext) {
    let mut h = setup(cx);
    set_script(&script("echo hello", "echo hello"));
    h.invoke("run.run", json!(null));
    for _ in 0..2000 {
        tick(h.cx);
        let s = h.invoke("run.state", json!(null));
        if s["tabs"][0]["status"] == json!("terminated") {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    h.draw();
    let (editor, console) = (h.editor(), h.console());
    assert_eq!(h.metrics(&editor), THEME);
    assert_eq!(h.metrics(&console), THEME);

    // The page shows the theme's font; the families are listed in the
    // background.
    h.invoke("settings.open", json!({"page": "editor.font"}));
    // Editor > General and Editor > Font, in the reference's order.
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["breadcrumbs"], json!(["Editor", "Font"]), "{s}");
    let ids: Vec<&str> = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    let at = |id: &str| ids.iter().position(|i| *i == id).unwrap();
    assert!(at("editor") < at("editor.general"), "{ids:?}");
    assert_eq!(at("editor.general") + 1, at("editor.font"), "{ids:?}");
    let page = h.page();
    assert_eq!(page["size"], json!("13.0"), "{page}");
    assert_eq!(page["line-height"], json!("1.2"), "{page}");
    assert_eq!(page["ligatures"], json!(false), "{page}");
    assert_eq!(page["restore-defaults"], json!(false), "{page}");
    assert_eq!(page["families-loaded"], json!(true), "{page}");

    // The preview follows the fields; the editors wait for Apply.
    h.edit("size", json!(20));
    h.edit("line-height", json!("1.5"));
    h.edit("ligatures", json!(true));
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["modified"], json!(["editor.font"]), "{s}");
    assert_eq!(s["fields"]["preview"]["size"], json!(20.0), "{s}");
    assert_eq!(s["fields"]["restore-defaults"], json!(true), "{s}");
    assert_eq!(h.metrics(&editor), THEME);

    // Apply: every editor and the console lay their text out again.
    h.invoke("settings.apply", json!(null));
    assert_eq!(h.metrics(&editor), CHOSEN);
    assert_eq!(h.metrics(&console), CHOSEN);
    let text = h.file_text();
    assert!(text.contains("[editor.font]"), "{text}");
    assert!(text.contains("size = 20.0"), "{text}");
    assert!(text.contains("line-height = 1.5"), "{text}");
    assert!(text.contains("ligatures = true"), "{text}");
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["modified"], json!([]), "{s}");

    // A size that is no number keeps the page open with an error.
    h.edit("size", json!("big"));
    h.invoke("settings.apply", json!(null));
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["modified"], json!(["editor.font"]), "{s}");
    assert!(!s["errors"].as_object().unwrap().is_empty(), "{s}");
    assert_eq!(h.metrics(&editor), CHOSEN);

    // Restore defaults: the theme's font again; nothing is stored.
    h.edit("restore-defaults", json!(null));
    let page = h.page();
    assert_eq!(page["size"], json!("13.0"), "{page}");
    assert_eq!(page["ligatures"], json!(false), "{page}");
    assert_eq!(page["restore-defaults"], json!(false), "{page}");
    h.invoke("settings.ok", json!(null));
    assert_eq!(h.metrics(&editor), THEME);
    assert_eq!(h.metrics(&console), THEME);
    let text = h.file_text();
    assert!(!text.contains("size = 20.0"), "{text}");
}

/// The stored font is put over the theme's at start-up, and an editor
/// opened later has it too.
#[gpui::test]
fn the_stored_font_applies_at_start_up(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("b.py"), "b = 2\n").unwrap();
    let file = dir.path().join("settings.toml");
    std::fs::write(
        &file,
        "[editor.font]\nsize = 20.0\nline-height = 1.5\nfamily = \"Menlo\"\n",
    )
    .unwrap();
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let store = file.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H {
        cx,
        window,
        file,
        _dir: dir,
    };
    h.draw();
    h.invoke(
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    h.invoke("project.open-file", json!({"path": "b.py"}));
    let editor = h.editor();
    assert_eq!(h.metrics(&editor), CHOSEN);
    h.invoke("settings.open", json!({"page": "editor.font"}));
    let page = h.page();
    assert_eq!(page["family"], json!("Menlo"), "{page}");
    assert_eq!(page["size"], json!("20.0"), "{page}");
    assert_eq!(page["restore-defaults"], json!(true), "{page}");
}
