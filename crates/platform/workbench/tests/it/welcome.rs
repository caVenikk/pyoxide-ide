//! The Welcome tab (tests/scenarios/settings/welcome.md): a main window
//! without a project opens it with `welcome.open`; its Theme and Keymap
//! lists switch the theme and the keymap at once and the combo boxes show
//! the new ones; the main toolbar shows no title for it; it closes like
//! any tab and `welcome.open` brings it back.

use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use settings_ui::theme_switch;
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
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
        self.cx.run_until_parked();
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

    /// The Welcome tab's state.
    fn welcome(&mut self) -> Value {
        let s = self.state();
        let tab = s["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["view"]["kind"] == json!("welcome"))
            .cloned();
        tab.map(|t| t["view"].clone()).unwrap_or(Value::Null)
    }
}

/// The labels of the open list's rows.
fn rows(list: &Value) -> Vec<String> {
    list["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap_or("-").to_owned())
        .collect()
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let file = config.join("settings.toml");
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let themes = config.join("themes");
    let keymaps = config.join("keymaps");
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, file);
        theme_switch::load(cx, Some(themes), None);
        workbench::keymap::active::load(cx, Some(keymaps));
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H {
        cx,
        window,
        _dir: dir,
    };
    h.draw();
    h
}

#[gpui::test]
fn the_welcome_tab_switches_the_theme_and_the_keymap(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert_eq!(h.welcome(), Value::Null, "not open by itself in a test");
    h.invoke("welcome.open", json!(null));
    let s = h.state();
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["tabs"][0]["title"], json!("Welcome to Test IDE"));
    assert_eq!(s["active_tab"], json!(0));
    assert_eq!(s["title_path"], Value::Null, "no title in the toolbar");
    let w = h.welcome();
    assert_eq!(w["subtitle"], json!("Start in one click"));
    assert_eq!(w["tiles"][0]["label"], json!("Open File"));
    assert_eq!(w["tiles"][0]["command"], json!("project.open-file"));
    assert_eq!(w["theme"], json!("Theme: Islands Dark"));
    let keymap = w["keymap"].as_str().unwrap().to_owned();
    assert!(keymap.starts_with("Keymap: "), "{keymap}");
    assert_eq!(w["list"], Value::Null);

    // The Theme list: every theme, the one in use selected.
    h.invoke("welcome.theme", json!(null));
    let list = h.welcome()["list"].clone();
    assert_eq!(list["combo"], json!("theme"), "{list}");
    let names: Vec<String> = h.cx.update(|cx| {
        theme_switch::catalog(cx)
            .themes()
            .map(|e| e.name.clone())
            .collect()
    });
    assert_eq!(rows(&list), names);
    let at = names.iter().position(|n| n == "Islands Light").unwrap();
    // Down to Islands Light and Enter: the theme at once.
    for _ in 0..at {
        h.invoke("menu.down", json!(null));
    }
    h.invoke("menu.confirm", json!(null));
    let w = h.welcome();
    assert_eq!(w["list"], Value::Null, "the list closes");
    assert_eq!(w["theme"], json!("Theme: Islands Light"));
    assert_eq!(h.state()["theme"], json!("islands-light"));

    // The Keymap list: every keymap; the next one is the keymap at once.
    h.invoke("welcome.keymap", json!(null));
    let list = h.welcome()["list"].clone();
    assert_eq!(list["combo"], json!("keymap"), "{list}");
    let entries: Vec<String> = h.cx.update(|cx| {
        workbench::keymap::active::entries(cx)
            .into_iter()
            .map(|e| e.name)
            .collect()
    });
    assert_eq!(rows(&list), entries);
    let current = keymap.trim_start_matches("Keymap: ").to_owned();
    let other = entries.iter().find(|n| **n != current).unwrap().clone();
    let from = entries.iter().position(|n| *n == current).unwrap();
    let to = entries.iter().position(|n| *n == other).unwrap();
    let step = if to > from { "menu.down" } else { "menu.up" };
    for _ in 0..from.abs_diff(to) {
        h.invoke(step, json!(null));
    }
    h.invoke("menu.confirm", json!(null));
    assert_eq!(h.welcome()["keymap"], json!(format!("Keymap: {other}")));
    let active = h.cx.update(|cx| workbench::keymap::active::active_name(cx));
    assert_eq!(active, other);

    // Escape closes a list without a change.
    h.invoke("welcome.theme", json!(null));
    h.invoke("menu.down", json!(null));
    h.invoke("menu.cancel", json!(null));
    let w = h.welcome();
    assert_eq!(w["list"], Value::Null);
    assert_eq!(w["theme"], json!("Theme: Islands Light"));
}

#[gpui::test]
fn the_welcome_tab_closes_and_comes_back(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("welcome.open", json!(null));
    // Once: opening it again selects it.
    h.invoke("welcome.open", json!(null));
    assert_eq!(h.state()["tabs"].as_array().unwrap().len(), 1);
    h.invoke("tabs.close", json!(null));
    assert_eq!(h.state()["tabs"].as_array().unwrap().len(), 0);
    assert_eq!(h.welcome(), Value::Null);
    // The Theme list opens the tab first.
    h.invoke("welcome.theme", json!(null));
    assert_eq!(h.welcome()["list"]["combo"], json!("theme"));
}
