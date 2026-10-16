//! The Settings window from a main window (tests/scenarios/settings/
//! settings-window.md, settings-apply-cancel.md): one non-modal window per
//! application, the page tree, the banner, Apply, OK, Cancel and "Revert
//! changes", and the store the main windows follow.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
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

    fn state(&mut self) -> Value {
        self.invoke("settings.state", json!(null))
    }

    fn windows(&mut self) -> usize {
        self.cx.update(|cx| cx.windows().len())
    }

    fn file_text(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).unwrap();
    let file = dir.path().join("config").join("settings.toml");
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let store_file = file.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store_file);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
        .update(cx, |w, window, cx| w.open_folder(project, window, cx))
        .unwrap();
    let mut h = H {
        cx,
        window,
        file,
        _dir: dir,
    };
    h.draw();
    h
}

#[gpui::test]
fn one_window_opens_on_the_page_with_the_project_in_its_title(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert_eq!(h.state()["open"], json!(false));
    let before = h.windows();
    h.invoke("settings.open", json!({"page": "system"}));
    let s = h.state();
    assert_eq!(s["open"], json!(true));
    assert_eq!(s["title"], json!("Settings \u{2013} demo"));
    assert_eq!(s["page"], json!("system"));
    assert_eq!(
        s["breadcrumbs"],
        json!(["Appearance & Behavior", "System Settings"])
    );
    assert_eq!(s["apply_enabled"], json!(false));
    assert_eq!(s["tree_focused"], json!(true));
    let height = settings_ui::HEIGHT + ui::dialog_window::title_height(true);
    assert_eq!(
        s["window"],
        json!({"width": settings_ui::WIDTH, "height": height})
    );
    assert_eq!(h.windows(), before + 1);
    // Again: the same window, on the page asked for.
    h.invoke("settings.open", json!({"page": "appearance-behavior"}));
    assert_eq!(h.windows(), before + 1);
    assert_eq!(h.state()["page"], json!("appearance-behavior"));
    // The main window keeps its input (non-modal): its own commands run.
    h.invoke("workbench.toggle-project", json!(null));
    h.invoke("settings.cancel", json!(null));
    assert_eq!(h.state()["open"], json!(false));
    assert_eq!(h.windows(), before);
}

#[gpui::test]
fn a_change_marks_the_page_and_revert_drops_it(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": 30}),
    );
    let s = h.state();
    assert_eq!(s["modified"], json!(["system"]));
    assert_eq!(s["apply_enabled"], json!(true));
    assert_eq!(s["fields"]["idle-seconds"], json!("30"));
    h.invoke("settings.revert", json!(null));
    let s = h.state();
    assert_eq!(s["modified"], json!([]));
    assert_eq!(s["fields"]["idle-seconds"], json!("2"));
}

#[gpui::test]
fn apply_stores_the_settings_and_the_main_window_follows(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": 30}),
    );
    h.invoke(
        "settings.edit",
        json!({"field": "sync-on-activation", "value": false}),
    );
    h.invoke("settings.apply", json!(null));
    let s = h.state();
    assert_eq!(s["open"], json!(true));
    assert_eq!(s["modified"], json!([]));
    assert_eq!(s["apply_enabled"], json!(false));
    let (save, sync) = h
        .window
        .read_with(h.cx, |w, _| (w.save_settings(), w.sync_settings()))
        .unwrap();
    assert_eq!(save.idle_seconds, 30);
    assert!(!sync.sync_on_activation);
    assert_eq!(
        h.invoke("settings.get", json!({"key": "system.files.idle-seconds"})),
        json!(30)
    );
    let text = h.file_text();
    assert!(text.contains("[system.files]"), "{text}");
    assert!(text.contains("idle-seconds = 30"), "{text}");
    assert!(text.contains("sync-on-activation = false"), "{text}");
    // OK applies and closes.
    h.invoke(
        "settings.edit",
        json!({"field": "save-on-idle", "value": false}),
    );
    h.invoke("settings.ok", json!(null));
    assert_eq!(h.state()["open"], json!(false));
    let save = h.window.read_with(h.cx, |w, _| w.save_settings()).unwrap();
    assert!(!save.save_on_idle);
}

#[gpui::test]
fn cancel_drops_the_changes(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "save-on-deactivation", "value": false}),
    );
    h.invoke("settings.cancel", json!(null));
    h.invoke("settings.open", json!({"page": "system"}));
    let s = h.state();
    assert_eq!(s["fields"]["save-on-deactivation"], json!(true));
    assert_eq!(s["modified"], json!([]));
    assert_eq!(h.file_text(), "");
}

#[gpui::test]
fn an_invalid_value_keeps_the_page_with_its_error(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": "0"}),
    );
    h.invoke("settings.select", json!({"page": "appearance-behavior"}));
    h.invoke("settings.ok", json!(null));
    let s = h.state();
    assert_eq!(s["open"], json!(true));
    assert_eq!(s["page"], json!("system"));
    assert_eq!(
        s["errors"],
        json!({"system": "Please enter a number from 1 to 300"})
    );
    let save = h.window.read_with(h.cx, |w, _| w.save_settings()).unwrap();
    assert_eq!(save.idle_seconds, 2);
}

#[gpui::test]
fn settings_set_changes_the_store_and_the_main_window(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke(
        "settings.set",
        json!({"key": "system.files.save-on-deactivation", "value": false}),
    );
    let save = h.window.read_with(h.cx, |w, _| w.save_settings()).unwrap();
    assert!(!save.save_on_deactivation);
    assert!(h.file_text().contains("save-on-deactivation = false"));
    assert!(
        h.try_invoke(
            "settings.set",
            json!({"key": "system.files.nothing", "value": 1})
        )
        .is_err()
    );
    assert!(
        h.try_invoke(
            "settings.set",
            json!({"key": "system.files.idle-seconds", "value": "x"})
        )
        .is_err()
    );
}
