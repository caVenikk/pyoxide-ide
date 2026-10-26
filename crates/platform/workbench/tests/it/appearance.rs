//! Settings > Appearance and the theme switchers of a main window
//! (tests/scenarios/settings/theme-switch.md, theme-import.md): the Theme
//! combo box switches at once and Cancel keeps it, the Theme popup previews
//! the theme its selection rests on and Escape restores the starting one,
//! the gear menu and Quick Switch Scheme open it, the theme is stored, and
//! imported themes are listed under "Custom".

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, VisualTestContext, WindowHandle};
use serde_json::{Value, json};
use settings_ui::theme_switch;
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    /// `settings.toml`.
    file: PathBuf,
    /// The folder of imported themes.
    themes: PathBuf,
    dir: tempfile::TempDir,
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

    /// Runs a command in the Settings window (its lists' rows).
    fn invoke_in_settings(&mut self, id: &str, args: Value) -> Value {
        let settings = self
            .cx
            .update(|cx| settings_ui::find(cx))
            .expect("the Settings window");
        let out = self
            .cx
            .update_window(settings.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
        out
    }

    /// The theme the windows show.
    fn shown(&mut self) -> String {
        self.invoke("workbench.state", json!(null))["theme"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// `settings.state`'s `appearance`.
    fn appearance(&mut self) -> Value {
        self.invoke("settings.state", json!(null))["appearance"].clone()
    }

    /// The open switcher popup.
    fn popup(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))["switch_popup"].clone()
    }

    fn file_text(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }

    fn wait(&mut self, ms: u64) {
        self.cx.executor().advance_clock(Duration::from_millis(ms));
        self.draw();
    }

    fn balloons(&mut self) -> Vec<(String, String, String)> {
        self.invoke("workbench.state", json!(null))["notifications"]["balloons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                let text = |k: &str| b[k].as_str().unwrap_or_default().to_owned();
                (text("kind"), text("title"), text("content"))
            })
            .collect()
    }

    /// Reads the settings and the imported themes again, as at start-up.
    fn restart(&mut self, pinned: Option<&str>) {
        let file = self.file.clone();
        let themes = self.themes.clone();
        let pinned = pinned.map(str::to_owned);
        self.cx.update(|cx| {
            settings_ui::store::load(cx, file);
            theme_switch::load(cx, Some(themes), pinned);
        });
        self.draw();
    }
}

/// The labels of a popup level, `-` for a line, `[title]` for a titled one.
fn labels(level: &Value) -> Vec<String> {
    level["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            if i["separator"] == json!(true) {
                "-".to_owned()
            } else if let Some(h) = i["header"].as_str() {
                format!("[{h}]")
            } else {
                i["label"].as_str().unwrap().to_owned()
            }
        })
        .collect()
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    open_with(cx, "")
}

/// A main window with `settings` in `settings.toml` at start-up.
fn open_with<'a>(cx: &'a mut TestAppContext, settings: &str) -> H<'a> {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).unwrap();
    let config = dir.path().join("config");
    let file = config.join("settings.toml");
    let themes = config.join("themes");
    if !settings.is_empty() {
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(&file, settings).unwrap();
    }
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let (store_file, theme_dir) = (file.clone(), themes.clone());
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store_file);
        theme_switch::load(cx, Some(theme_dir), None);
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
        themes,
        dir,
    };
    h.draw();
    h
}

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../themes/tests/fixtures")
        .join(path)
}

const BUILTIN: [&str; 6] = [
    "Islands Dark",
    "Islands Light",
    "-",
    "App Dark",
    "App Light",
    "-",
];

#[gpui::test]
fn the_theme_combo_switches_at_once_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert_eq!(h.shown(), "islands-dark");
    h.invoke("settings.open", json!({"page": "appearance"}));
    let page = h.invoke("settings.state", json!(null))["fields"].clone();
    assert_eq!(page["theme"], json!("islands-dark"), "{page}");
    assert_eq!(page["editor-scheme"], json!("islands-dark"), "{page}");
    // The list: the groups apart, the current theme selected, Import
    // Theme… at the end.
    h.invoke("settings.edit", json!({"field": "list", "value": "theme"}));
    let menu = h.invoke("settings.state", json!(null))["fields"]["menu"].clone();
    let level = &menu["levels"][0];
    let mut expected: Vec<&str> = BUILTIN[..5].to_vec();
    expected.extend(["-", "Import Theme\u{2026}"]);
    assert_eq!(labels(level), expected, "{menu}");
    assert_eq!(level["selected"], json!(0), "{menu}");
    // Choosing switches at once, before Apply.
    h.invoke_in_settings("menu.down", json!(null));
    h.invoke_in_settings("menu.confirm", json!(null));
    assert_eq!(h.shown(), "islands-light");
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["fields"]["theme"], json!("islands-light"));
    assert_eq!(s["apply_enabled"], json!(false), "{s}");
    // Cancel does not switch back; the theme is stored.
    h.invoke("settings.cancel", json!(null));
    assert_eq!(h.shown(), "islands-light");
    h.wait(1000);
    assert!(
        h.file_text().contains("theme = \"islands-light\""),
        "{}",
        h.file_text()
    );
    // By field, as an agent does.
    h.invoke("settings.open", json!({"page": "appearance"}));
    h.invoke(
        "settings.edit",
        json!({"field": "theme", "value": "app-light"}),
    );
    assert_eq!(h.shown(), "app-light");
    // An id the catalog does not know changes nothing.
    h.invoke(
        "settings.edit",
        json!({"field": "theme", "value": "no-such-theme"}),
    );
    assert_eq!(h.shown(), "app-light");
}

#[gpui::test]
fn the_editor_scheme_waits_for_apply_and_follows_the_theme(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "appearance"}));
    h.invoke(
        "settings.edit",
        json!({"field": "editor-scheme", "value": "islands-light"}),
    );
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["modified"], json!(["appearance"]), "{s}");
    assert_eq!(h.appearance()["editor_scheme"], json!("islands-dark"));
    h.invoke("settings.apply", json!(null));
    let a = h.appearance();
    assert_eq!(a["theme"], json!("islands-dark"));
    assert_eq!(a["editor_scheme"], json!("islands-light"));
    // The UI stays dark, the editor takes the light scheme.
    let editor_background = |h: &mut H| {
        h.cx.update(|cx| {
            use ui::theme::ThemeExt;
            cx.theme().color(themes::UiRole::EditorBackground)
        })
    };
    let light = themes::Registry::with_builtin()
        .resolve("islands-light")
        .unwrap()
        .ui[themes::UiRole::EditorBackground];
    assert_eq!(h.shown(), "islands-dark");
    assert_eq!(editor_background(&mut h), ui::theme::color(light));
    // Another theme brings its own scheme.
    h.invoke("settings.switch-theme", json!({"id": "app-dark"}));
    assert_eq!(h.appearance()["editor_scheme"], json!("app-dark"));
}

#[gpui::test]
fn the_theme_popup_previews_the_selection_and_escape_restores(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.switch-theme", json!(null));
    let popup = h.popup();
    assert_eq!(popup["title"], json!("Theme"), "{popup}");
    let level = &popup["levels"][0];
    let mut expected: Vec<&str> = BUILTIN.to_vec();
    expected.push("Import Theme\u{2026}");
    assert_eq!(labels(level), expected, "{popup}");
    // It opens on the theme in use.
    assert_eq!(level["selected"], json!(0), "{popup}");
    assert_eq!(h.appearance()["previewing"], json!(true));
    // Half a second after the selection rests the IDE shows the theme.
    h.invoke("menu.down", json!(null));
    h.wait(300);
    assert_eq!(h.shown(), "islands-dark");
    // Moving on starts the wait again.
    h.invoke("menu.down", json!(null));
    h.wait(300);
    assert_eq!(h.shown(), "islands-dark");
    h.wait(300);
    assert_eq!(h.shown(), "app-dark");
    // Previewed, not stored.
    assert_eq!(h.appearance()["theme"], json!("islands-dark"));
    assert_eq!(h.appearance()["shown"], json!("app-dark"));
    // Escape brings back the starting theme.
    h.invoke("menu.cancel", json!(null));
    assert!(h.popup().is_null());
    assert_eq!(h.shown(), "islands-dark");
    assert_eq!(h.appearance()["previewing"], json!(false));
    h.wait(1000);
    assert_eq!(h.shown(), "islands-dark");
    assert!(!h.file_text().contains("theme"), "{}", h.file_text());
}

#[gpui::test]
fn a_theme_chosen_in_the_popup_stays(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.switch-theme", json!(null));
    h.invoke("menu.down", json!(null));
    h.invoke("menu.confirm", json!(null));
    assert!(h.popup().is_null());
    assert_eq!(h.shown(), "islands-light");
    h.wait(1000);
    assert_eq!(h.shown(), "islands-light");
    assert_eq!(h.appearance()["theme"], json!("islands-light"));
    // The next popup opens on it.
    h.invoke("settings.switch-theme", json!(null));
    assert_eq!(h.popup()["levels"][0]["selected"], json!(1));
    h.invoke("menu.cancel", json!(null));
    assert_eq!(h.shown(), "islands-light");
}

#[gpui::test]
fn a_digit_key_chooses_a_row_of_the_theme_popup(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.switch-theme", json!(null));
    // The rows have digits, the lines none.
    let popup = h.popup();
    let digits: Vec<&str> = popup["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["separator"] != json!(true))
        .map(|i| i["number"].as_str().unwrap_or(""))
        .collect();
    // Four themes and "Import Theme…".
    assert_eq!(digits, ["1", "2", "3", "4", "5"], "{popup}");
    // 2 is the second theme: it is chosen and applied at once.
    h.cx.simulate_input(h.window.into(), "2");
    h.draw();
    assert!(h.popup().is_null());
    assert_eq!(h.shown(), "islands-light");
    h.wait(1000);
    assert_eq!(h.shown(), "islands-light");
    assert_eq!(h.appearance()["theme"], json!("islands-light"));
}

#[gpui::test]
fn the_gear_menu_and_quick_switch_open_the_theme_popup(cx: &mut TestAppContext) {
    let mut h = open(cx);
    // Gear: Settings…, a line, Theme…, Keymap….
    let main = h.window;
    let mut vcx = VisualTestContext::from_window(main.into(), h.cx);
    let gear = vcx.debug_bounds("toolbar-settings").expect("the gear");
    vcx.simulate_click(gear.center(), gpui::Modifiers::none());
    h.draw();
    let menu = h.invoke("run.state", json!(null))["menu"].clone();
    assert_eq!(
        labels(&menu["levels"][0]),
        ["Settings\u{2026}", "-", "Theme\u{2026}", "Keymap\u{2026}"],
        "{menu}"
    );
    h.invoke("menu.down", json!(null));
    h.invoke("menu.down", json!(null));
    h.invoke("menu.confirm", json!(null));
    assert_eq!(h.popup()["title"], json!("Theme"));
    h.invoke("menu.cancel", json!(null));
    assert!(h.popup().is_null());
    // Quick Switch Scheme: Switch with Theme, which opens the same list,
    // and Keymap.
    h.invoke("settings.quick-switch", json!(null));
    let popup = h.popup();
    assert_eq!(popup["title"], json!("Switch"), "{popup}");
    assert_eq!(labels(&popup["levels"][0]), ["Theme", "Keymap"], "{popup}");
    h.invoke("menu.confirm", json!(null));
    let popup = h.popup();
    assert_eq!(popup["title"], json!("Theme"), "{popup}");
    // Its preview works as from the gear.
    h.invoke("menu.down", json!(null));
    h.wait(600);
    assert_eq!(h.shown(), "islands-light");
    h.invoke("menu.cancel", json!(null));
    assert_eq!(h.shown(), "islands-dark");
}

#[gpui::test]
fn the_stored_theme_is_used_at_start_and_pyoxide_theme_overrides_it(cx: &mut TestAppContext) {
    let mut h = open_with(cx, "[appearance]\ntheme = \"app-light\"\n");
    assert_eq!(h.shown(), "app-light");
    // `workbench.set-theme` stores the theme as the switchers do.
    h.invoke("workbench.set-theme", json!({"id": "islands-light"}));
    h.wait(1000);
    assert!(
        h.file_text().contains("theme = \"islands-light\""),
        "{}",
        h.file_text()
    );
    h.restart(None);
    assert_eq!(h.shown(), "islands-light");
    // `APP_THEME=dark` wins for the run, without being stored.
    h.restart(Some("dark"));
    assert_eq!(h.shown(), "islands-dark");
    assert_eq!(h.appearance()["pinned"], json!("islands-dark"));
    assert!(h.file_text().contains("islands-light"));
    // Until a theme is chosen.
    h.invoke("settings.switch-theme", json!({"id": "app-dark"}));
    assert_eq!(h.shown(), "app-dark");
    assert_eq!(h.appearance()["pinned"], Value::Null);
    // A theme the catalog does not know gives the default.
    h.invoke(
        "settings.set",
        json!({"key": "appearance.theme", "value": "gone"}),
    );
    assert_eq!(h.shown(), "islands-dark");
    // Switching to it changes nothing.
    h.invoke("settings.switch-theme", json!({"id": "app-light"}));
    h.invoke("settings.switch-theme", json!({"id": "gone"}));
    assert_eq!(h.shown(), "app-light");
}

#[gpui::test]
fn an_imported_theme_is_listed_under_custom_and_selected(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let plugin = fixture("plugin");
    h.invoke(
        "settings.import-theme",
        json!({"path": plugin.display().to_string()}),
    );
    // The plugin has four UI themes: Custom lists them by name, the
    // first one imported is the theme.
    let a = h.appearance();
    let custom: Vec<(String, String)> = a["themes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["group"] == json!("custom"))
        .map(|t| {
            let text = |k: &str| t[k].as_str().unwrap().to_owned();
            (text("id"), text("name"))
        })
        .collect();
    let names: Vec<&str> = custom.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["Dusk", "Nightfall", "Orphan", "Plain"], "{a}");
    let id = a["theme"].as_str().unwrap().to_owned();
    let (_, name) = custom
        .iter()
        .find(|(i, _)| *i == id)
        .expect("an imported theme is in use")
        .clone();
    assert_eq!(h.shown(), id);
    for (id, _) in &custom {
        assert!(h.themes.join(format!("{id}.toml")).is_file(), "{id}");
    }
    // The report: a short balloon that says what came in (#201); the keys
    // that could not be mapped go to the log.
    let balloons = h.balloons();
    let (kind, title, content) = balloons.first().expect("a balloon").clone();
    assert_eq!(kind, "information", "{balloons:?}");
    assert_eq!(title, "Themes imported", "{balloons:?}");
    assert_eq!(
        content,
        format!("Imported 4 themes. {name} is now the theme."),
        "{balloons:?}"
    );
    // The Theme popup lists it after a line, before Import Theme….
    h.invoke("settings.switch-theme", json!(null));
    let popup = h.popup();
    let rows = labels(&popup["levels"][0]);
    let n = rows.len();
    let mut tail = vec!["-"];
    tail.extend(&names);
    tail.extend(["-", "Import Theme\u{2026}"]);
    assert_eq!(rows[n - tail.len()..], tail, "{rows:?}");
    h.invoke("menu.cancel", json!(null));
    // The combo box gives the group its title.
    h.invoke("settings.open", json!({"page": "appearance"}));
    h.invoke("settings.edit", json!({"field": "list", "value": "theme"}));
    let menu = h.invoke("settings.state", json!(null))["fields"]["menu"].clone();
    let rows = labels(&menu["levels"][0]);
    assert!(rows.contains(&"[Custom]".to_owned()), "{rows:?}");
    h.invoke("settings.cancel", json!(null));
    // A scheme alone is offered as an editor colour scheme only.
    let icls = fixture("schemes/Skyline.icls");
    h.invoke(
        "settings.import-theme",
        json!({"path": icls.display().to_string()}),
    );
    let a = h.appearance();
    assert_eq!(a["theme"], json!(id), "the theme stays");
    let schemes = a["schemes"].as_array().unwrap();
    assert_eq!(schemes.len(), 1, "{a}");
    let balloons = h.balloons();
    assert_eq!(balloons[0].1, "Color scheme added", "{balloons:?}");
    assert_eq!(
        balloons[0].2,
        format!(
            "{} is offered as an editor color scheme.",
            schemes[0]["name"].as_str().unwrap()
        ),
        "{balloons:?}"
    );
    assert_eq!(schemes[0]["scheme_only"], json!(true));
    assert!(
        !a["themes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == schemes[0]["id"])
    );
    // After a restart the theme is listed and in use.
    h.restart(None);
    let a = h.appearance();
    assert_eq!(a["theme"], json!(id));
    assert_eq!(h.shown(), id);
    // Not a theme: an error balloon; nothing changes.
    let text = h.dir.path().join("notes.txt");
    std::fs::write(&text, "hello").unwrap();
    let before = std::fs::read_dir(&h.themes).unwrap().count();
    h.invoke(
        "settings.import-theme",
        json!({"path": text.display().to_string()}),
    );
    let balloons = h.balloons();
    assert_eq!(balloons[0].0, "error", "{balloons:?}");
    assert_eq!(balloons[0].1, "Theme import failed");
    assert_eq!(h.appearance()["theme"], json!(id));
    assert_eq!(std::fs::read_dir(&h.themes).unwrap().count(), before);
}
