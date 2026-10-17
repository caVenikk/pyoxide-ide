//! Choosing the keymap (tests/scenarios/settings/keymap-switch.md, ADR
//! 0018): the derived keymaps resolve (every binding to a registered
//! command, a keystroke once per context, the unmapped actions of the VS
//! Code keymaps as checked expectations), switching changes the bindings
//! pressed as keystrokes, the Keymap page chooses with Apply and searches
//! by name and by shortcut, the gear menu and Quick Switch Scheme open the
//! Keymap popup, and a keymap of the reference imports.
//!
//! The expected unmapped actions are in `tests/keymaps/`; after a change of
//! the action table, `PYOXIDE_BLESS=1 cargo test -p workbench --test
//! keymaps` writes them anew for review.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{Action, AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::keymap::active::{self as keymaps, platform_default};
use workbench::keymap::derived::canonical_keys;
use workbench::keymap::{MACOS_NAME, VSCODE_MACOS_NAME, VSCODE_NAME, WINDOWS_NAME};
use workbench::{Product, Workbench};

const MAC: bool = cfg!(target_os = "macos");

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    /// `settings.toml`.
    file: PathBuf,
    /// The folder of custom keymaps.
    keymaps: PathBuf,
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

    /// Runs a command in the Settings window.
    fn invoke_in_settings(&mut self, id: &str, args: Value) -> Value {
        let settings = self.settings();
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

    fn settings(&mut self) -> WindowHandle<settings_ui::SettingsWindow> {
        self.cx
            .update(|cx| settings_ui::find(cx))
            .expect("the Settings window")
    }

    fn settings_open(&mut self) -> bool {
        self.invoke("settings.state", json!(null))["open"] == json!(true)
    }

    /// The Keymap page's fields.
    fn page(&mut self) -> Value {
        let s = self.invoke("settings.state", json!(null));
        assert_eq!(s["page"], json!("keymap"), "{s}");
        s["fields"].clone()
    }

    fn active(&mut self) -> String {
        self.invoke("workbench.state", json!(null))["keymap"]["name"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn popup(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))["switch_popup"].clone()
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }

    fn press_in_settings(&mut self, keys: &str) {
        let settings = self.settings();
        self.cx.simulate_keystrokes(settings.into(), keys);
        self.draw();
    }

    fn file_text(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }

    fn balloons(&mut self) -> Vec<(String, String)> {
        self.invoke("workbench.state", json!(null))["notifications"]["balloons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                let text = |k: &str| b[k].as_str().unwrap_or_default().to_owned();
                (text("title"), text("content"))
            })
            .collect()
    }

    /// The keys of the binding of `action` that wins in the main window.
    fn shortcut_of(&mut self, action: Box<dyn Action>) -> Option<String> {
        self.cx
            .update_window(self.window.into(), |_, window, _| {
                window
                    .highest_precedence_binding_for_action(action.as_ref())
                    .map(|b| {
                        b.keystrokes()
                            .iter()
                            .map(|k| k.unparse())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
            })
            .unwrap()
    }

    /// Closes the Settings window if it is open.
    fn close_settings(&mut self) {
        if self.settings_open() {
            self.invoke_in_settings("settings.cancel", json!(null));
        }
    }
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("main.py"), "print(1)\n").unwrap();
    let config = dir.path().join("config");
    let file = config.join("settings.toml");
    let keymaps_dir = config.join("keymaps");
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let store_file = file.clone();
    let state_file = config.join("settings-window.toml");
    let keymap_dir = keymaps_dir.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store_file);
        settings_ui::window_state::load(cx, state_file);
        keymaps::load(cx, Some(keymap_dir));
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
        keymaps: keymaps_dir,
        dir,
    };
    h.draw();
    h
}

/// The keys of Settings… in a base keymap.
fn settings_keys(name: &str) -> &'static str {
    if name == MACOS_NAME {
        "cmd-,"
    } else {
        "ctrl-alt-s"
    }
}

/// The base keymap that is not the platform's.
fn other_base() -> &'static str {
    if MAC { WINDOWS_NAME } else { MACOS_NAME }
}

// ---- resolution ----------------------------------------------------------

fn expectation_file(name: &str) -> PathBuf {
    let slug = name.to_lowercase().replace(" (", "-").replace(')', "");
    let file = format!("unmapped-{slug}.txt");
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("keymaps")
        .join(file)
}

/// Every binding of a keymap builds its command's action, and a keystroke
/// is bound once in a context.
fn check_resolved(cx: &mut TestAppContext, name: &str) {
    let keymap = cx.update(|cx| keymaps::resolve(name, cx)).unwrap();
    let mut seen: HashMap<(Option<String>, String), String> = HashMap::new();
    let mut errors = Vec::new();
    for b in &keymap.bindings {
        if let Err(e) = cx.update(|cx| cx.build_action(&b.command, b.args.clone())) {
            errors.push(format!("{} -> {}: {e}", b.keys, b.command));
        }
        let key = (b.context.clone(), canonical_keys(&b.keys));
        if let Some(other) = seen.insert(key, b.command.clone()) {
            errors.push(format!(
                "`{}` in {:?}: {other} and {}",
                b.keys, b.context, b.command
            ));
        }
    }
    assert!(errors.is_empty(), "{name}: {errors:#?}");
}

#[gpui::test]
fn every_keymap_resolves_to_registered_commands_once_per_keystroke(cx: &mut TestAppContext) {
    let _h = open(cx);
    for name in [MACOS_NAME, WINDOWS_NAME, VSCODE_NAME, VSCODE_MACOS_NAME] {
        check_resolved(cx, name);
    }
}

#[gpui::test]
fn the_unmapped_actions_of_the_vs_code_keymaps_are_expected(_cx: &mut TestAppContext) {
    for (name, macos) in [(VSCODE_NAME, false), (VSCODE_MACOS_NAME, true)] {
        let converted = keymaps::vscode(macos);
        let mut actual: Vec<String> = converted
            .unmapped
            .iter()
            .map(|u| u.action.clone())
            .collect();
        actual.sort();
        let file = expectation_file(name);
        let text = actual.join("\n") + "\n";
        if std::env::var_os("PYOXIDE_BLESS").is_some() {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, &text).unwrap();
        }
        let expected =
            std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let expected: Vec<&str> = expected.lines().filter(|l| !l.is_empty()).collect();
        let now_mapped: Vec<&&str> = expected
            .iter()
            .filter(|id| !actual.iter().any(|a| a == **id))
            .collect();
        let now_unmapped: Vec<&String> = actual
            .iter()
            .filter(|a| !expected.contains(&a.as_str()))
            .collect();
        assert!(
            now_mapped.is_empty() && now_unmapped.is_empty(),
            "{name}: mapped now {now_mapped:?}, unmapped now {now_unmapped:?}; \
             review and run with PYOXIDE_BLESS=1"
        );
        // Every shortcut of a mapped action that Pyoxide cannot bind is a
        // mouse gesture.
        for s in &converted.unsupported {
            assert!(
                s.contains("button") || s.contains("Force touch"),
                "{name}: {s}"
            );
        }
    }
}

#[gpui::test]
fn vs_code_keymaps_take_the_reference_shortcuts(cx: &mut TestAppContext) {
    let _h = open(cx);
    let keys = |cx: &mut TestAppContext, name: &str, command: &str, context: Option<&str>| {
        let keymap = cx.update(|cx| keymaps::resolve(name, cx)).unwrap();
        let mut keys: Vec<String> = keymap
            .bindings
            .iter()
            .filter(|b| b.command == command && b.context.as_deref() == context)
            .map(|b| b.keys.clone())
            .collect();
        keys.sort();
        keys
    };
    // VSCode on Windows: Go to File is Ctrl+P, Duplicate is Shift+Alt+Down
    // (EditorDuplicateLines is not Pyoxide's; EditorDuplicate keeps the
    // parent's), Delete Line is Ctrl+Shift+K, Settings Ctrl+Comma.
    assert_eq!(
        keys(cx, VSCODE_NAME, "search-everywhere.go-to-file", None),
        ["ctrl-p"]
    );
    assert_eq!(
        keys(cx, VSCODE_NAME, "editor.delete-line", Some("Editor")),
        ["ctrl-shift-k"]
    );
    assert_eq!(keys(cx, VSCODE_NAME, "settings.open", None), ["ctrl-,"]);
    assert_eq!(
        keys(cx, VSCODE_NAME, "editor.toggle-soft-wraps", Some("Editor")),
        ["alt-z"]
    );
    // VSCode (macOS) on top of macOS.
    assert_eq!(
        keys(cx, VSCODE_MACOS_NAME, "search-everywhere.go-to-file", None),
        ["cmd-p"]
    );
    // The parent's keystroke taken by another command is gone from the
    // parent's: Cmd+P was Parameter Info in macOS.
    let parameter_info = keys(cx, VSCODE_MACOS_NAME, "code.parameter-info", Some("Editor"));
    assert!(
        !parameter_info.contains(&"cmd-p".to_owned()),
        "{parameter_info:?}"
    );
}

// ---- switching -----------------------------------------------------------

#[gpui::test]
fn switching_the_keymap_changes_the_keystrokes(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let default = platform_default();
    let other = other_base();
    assert_eq!(h.active(), default);
    // The platform's Settings… keys open the Settings window.
    h.press(settings_keys(default));
    assert!(h.settings_open());
    h.close_settings();
    // The other base keymap: its keys open Settings, the platform's do
    // nothing; the menus show its shortcut.
    h.invoke("settings.switch-keymap", json!({"name": other}));
    assert_eq!(h.active(), other);
    assert!(h.file_text().contains(&format!("name = \"{other}\"")));
    h.press(settings_keys(default));
    assert!(!h.settings_open());
    h.press(settings_keys(other));
    assert!(h.settings_open());
    h.close_settings();
    let open_settings = settings_ui::commands::OpenSettings {
        page: None,
        filter: None,
    };
    assert_eq!(
        h.shortcut_of(Box::new(open_settings.clone())).as_deref(),
        Some(settings_keys(other))
    );
    // Back: the platform's keymap again, not written to the settings.
    h.invoke("settings.switch-keymap", json!({"name": default}));
    assert_eq!(h.active(), default);
    assert!(!h.file_text().contains("[keymap]") || !h.file_text().contains("name ="));
    h.press(settings_keys(other));
    assert!(!h.settings_open());
    h.press(settings_keys(default));
    assert!(h.settings_open());
    h.close_settings();
    assert_eq!(
        h.shortcut_of(Box::new(open_settings)).as_deref(),
        Some(settings_keys(default))
    );
    // An unknown keymap is refused and changes nothing.
    h.invoke("settings.switch-keymap", json!({"name": "Nope"}));
    assert_eq!(h.active(), default);
}

#[gpui::test]
fn vs_code_bindings_and_gestures_after_a_switch(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let name = if MAC { VSCODE_MACOS_NAME } else { VSCODE_NAME };
    h.invoke("settings.switch-keymap", json!({"name": name}));
    assert_eq!(h.active(), name);
    // Go to File is Cmd+P (Ctrl+P): Search Everywhere opens on Files.
    h.press(if MAC { "cmd-p" } else { "ctrl-p" });
    let se = h.invoke("search-everywhere.state", json!(null));
    assert_eq!(se["open"], json!(true), "{se}");
    assert_eq!(se["tab"], json!("files"), "{se}");
    h.press("escape");
    // The double tap of Shift and the mouse buttons are still bound.
    let tap = h.cx.update(|cx| keymaps_double_tap(cx));
    assert!(tap, "Search Everywhere keeps its double Shift");
    // The settings name it; a store change (another window) switches too.
    assert!(h.file_text().contains(&format!("name = \"{name}\"")));
    h.invoke(
        "settings.set",
        json!({"key": "keymap.name", "value": other_base()}),
    );
    assert_eq!(h.active(), other_base());
}

fn keymaps_double_tap(cx: &gpui::App) -> bool {
    workbench::keymap::double_tap_for("search-everywhere.open", cx).is_some()
}

// ---- the Keymap page -----------------------------------------------------

#[gpui::test]
fn the_keymap_page_chooses_with_apply(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let default = platform_default();
    h.invoke("settings.open", json!({"page": "keymap"}));
    let page = h.page();
    assert_eq!(page["keymap"], json!(default));
    let names: Vec<&str> = page["keymaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["name"].as_str().unwrap())
        .collect();
    if MAC {
        assert_eq!(
            names,
            [MACOS_NAME, VSCODE_NAME, VSCODE_MACOS_NAME, WINDOWS_NAME]
        );
    } else {
        assert_eq!(names, [WINDOWS_NAME, VSCODE_NAME]);
    }
    // The tree: the top-level groups, closed.
    let groups: Vec<&str> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["group"].as_str().unwrap())
        .collect();
    assert_eq!(groups, ["Editor Actions", "Main Menu", "Other"]);
    // The combo box's list: predefined keymaps bold, the chosen one
    // selected.
    h.invoke_in_settings("settings.edit", json!({"field": "list", "value": true}));
    let list = h.page()["list"]["levels"][0].clone();
    assert_eq!(list["items"][0]["label"], json!(default));
    assert_eq!(list["items"][0]["bold"], json!(true));
    assert_eq!(list["selected"], json!(0));
    // Choosing a row of the list edits the page; Apply switches.
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "keymap", "value": VSCODE_NAME}),
    );
    h.invoke_in_settings("settings.edit", json!({"field": "list", "value": null}));
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["apply_enabled"], json!(true));
    assert_eq!(s["fields"]["keymap"], json!(VSCODE_NAME));
    assert_eq!(h.active(), default, "not before Apply");
    h.invoke_in_settings("settings.apply", json!(null));
    assert_eq!(h.active(), VSCODE_NAME);
    assert_eq!(
        h.invoke("settings.state", json!(null))["apply_enabled"],
        json!(false)
    );
    // Revert after a choice goes back to the keymap in use.
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "keymap", "value": default}),
    );
    h.invoke_in_settings("settings.revert", json!(null));
    assert_eq!(h.page()["keymap"], json!(VSCODE_NAME));
    h.invoke_in_settings("settings.cancel", json!(null));
    assert_eq!(h.active(), VSCODE_NAME);
}

#[gpui::test]
fn the_keymap_page_searches_by_name_and_by_shortcut(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "keymap"}));
    // The Settings search hands its text to the page: the tree keeps the
    // matching actions with their groups, open.
    h.invoke_in_settings("settings.search", json!({"text": "comment"}));
    h.cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    h.draw();
    let page = h.page();
    assert_eq!(page["search"], json!("comment"), "{page}");
    let actions: Vec<(String, Value)> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| Some((r["action"].as_str()?.to_owned(), r["shortcuts"].clone())))
        .collect();
    let comment = if MAC { "cmd-/" } else { "ctrl-/" };
    assert_eq!(
        actions,
        [("CommentByLineComment".to_owned(), json!([comment]))],
        "{page}"
    );
    let groups: Vec<&str> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["group"].as_str())
        .collect();
    assert_eq!(groups, ["Main Menu", "Code", "Comment Actions"]);
    h.invoke_in_settings("settings.search", json!({"text": ""}));
    // Find Actions by Shortcut: the keystroke pressed in its popup.
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "find-shortcut", "value": true}),
    );
    assert_eq!(h.page()["find_shortcut_open"], json!(true));
    h.press_in_settings(comment);
    let page = h.page();
    assert_eq!(page["shortcut"], json!(comment), "{page}");
    let actions: Vec<&str> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["action"].as_str())
        .collect();
    assert_eq!(actions, ["CommentByLineComment"]);
    // The keystroke went to the popup, not to the window's bindings.
    assert_eq!(h.invoke("settings.state", json!(null))["open"], json!(true));
    // Two Escapes close the popup; the filter stays until it is cleared.
    h.press_in_settings("escape escape");
    let page = h.page();
    assert_eq!(page["find_shortcut_open"], json!(false));
    assert_eq!(page["shortcut"], json!(comment));
    h.invoke_in_settings("settings.edit", json!({"field": "shortcut", "value": null}));
    assert_eq!(h.page()["rows"].as_array().unwrap().len(), 3);
    // Expand All and Collapse All.
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "expand-all", "value": true}),
    );
    let rows = h.page()["rows"].as_array().unwrap().len();
    assert!(rows > 100, "{rows} rows");
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "collapse-all", "value": true}),
    );
    assert_eq!(h.page()["rows"].as_array().unwrap().len(), 3);
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "toggle", "value": "Main Menu"}),
    );
    let page = h.page();
    let main: Vec<&str> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["depth"] == json!(1))
        .filter_map(|r| r["group"].as_str())
        .collect();
    assert_eq!(
        main,
        [
            "File", "Edit", "View", "Navigate", "Code", "Refactor", "Run", "Window", "Help"
        ]
    );
}

// ---- switchers -----------------------------------------------------------

#[gpui::test]
fn the_gear_and_quick_switch_open_the_keymap_popup(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let labels = workbench::settings::gear_menu()
        .iter()
        .filter_map(|i| i.label().map(|l| l.to_string()))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["Settings\u{2026}", "Theme\u{2026}", "Keymap\u{2026}"]
    );
    // Quick Switch Scheme: Theme and Keymap.
    h.invoke("settings.quick-switch", json!(null));
    let popup = h.popup();
    let rows: Vec<&str> = popup["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["label"].as_str())
        .collect();
    assert_eq!(rows, ["Theme", "Keymap"]);
    // The Keymap popup: the keymaps with the current one selected, then
    // Configure Keymap… and Import Keymap….
    h.invoke("settings.switch-keymap", json!(null));
    let popup = h.popup();
    let rows: Vec<&str> = popup["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["label"].as_str())
        .collect();
    let keymaps: Vec<&str> = if MAC {
        vec![MACOS_NAME, VSCODE_NAME, VSCODE_MACOS_NAME, WINDOWS_NAME]
    } else {
        vec![WINDOWS_NAME, VSCODE_NAME]
    };
    let mut expected = keymaps.clone();
    expected.extend(["Configure Keymap\u{2026}", "Import Keymap\u{2026}"]);
    assert_eq!(rows, expected);
    assert_eq!(popup["title"], json!("Keymap"));
    assert_eq!(popup["levels"][0]["selected"], json!(0));
    // Choosing a row switches at once.
    let row = keymaps.iter().position(|k| *k == VSCODE_NAME).unwrap();
    for _ in 0..row {
        h.press("down");
    }
    h.press("enter");
    assert_eq!(h.active(), VSCODE_NAME);
    assert!(h.popup().is_null());
}

// ---- import --------------------------------------------------------------

#[gpui::test]
fn a_keymap_of_the_reference_imports_and_becomes_the_keymap(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let source = h.dir.path().join("reference");
    std::fs::create_dir(&source).unwrap();
    let parent = if MAC { "Mac OS X 10.5+" } else { "$default" };
    let file = source.join("Mine.xml");
    std::fs::write(
        &file,
        format!(
            r#"<keymap version="1" name="Mine" parent="{parent}">
  <action id="ShowSettings"><keyboard-shortcut first-keystroke="ctrl alt shift S"/></action>
  <action id="Unknown.Action"><keyboard-shortcut first-keystroke="ctrl alt shift U"/></action>
</keymap>"#
        ),
    )
    .unwrap();
    h.invoke(
        "settings.import-keymap",
        json!({"path": file.display().to_string()}),
    );
    assert_eq!(h.active(), "Mine");
    assert!(h.keymaps.join("Mine.toml").is_file());
    let balloons = h.balloons();
    assert_eq!(balloons.len(), 1, "{balloons:?}");
    assert_eq!(balloons[0].0, "Keymap imported");
    assert!(balloons[0].1.contains("Unknown.Action"), "{balloons:?}");
    // Its shortcut opens Settings; the base keymap's does not.
    h.press(settings_keys(platform_default()));
    assert!(!h.settings_open());
    h.press("ctrl-alt-shift-s");
    assert!(h.settings_open());
    // The page lists it under its parent, indented, with "Based on".
    h.invoke_in_settings("settings.select", json!({"page": "keymap"}));
    let page = h.page();
    let mine = page["keymaps"]
        .as_array()
        .unwrap()
        .iter()
        .position(|k| k["name"] == json!("Mine"))
        .unwrap();
    assert_eq!(page["keymaps"][mine]["predefined"], json!(false));
    assert_eq!(page["keymaps"][mine - 1]["name"], json!(platform_default()));
    assert_eq!(page["based_on"], json!(platform_default()));
    h.close_settings();
    // A broken file is reported.
    let bad = source.join("bad.xml");
    std::fs::write(&bad, "<application/>").unwrap();
    h.invoke(
        "settings.import-keymap",
        json!({"path": bad.display().to_string()}),
    );
    let balloons = h.balloons();
    assert!(
        balloons.iter().any(|b| b.0 == "Keymap import failed"),
        "{balloons:?}"
    );
    assert_eq!(h.active(), "Mine");
    // A new start reads it from the keymaps folder.
    let dir = h.keymaps.clone();
    h.cx.update(|cx| keymaps::load(cx, Some(dir)));
    assert_eq!(h.active(), "Mine");
}

// ---- layout --------------------------------------------------------------

#[gpui::test]
fn the_tree_fills_the_page_below_the_combo_box_and_the_toolbar(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "keymap"}));
    h.invoke_in_settings(
        "settings.edit",
        json!({"field": "expand-all", "value": true}),
    );
    let settings = h.settings();
    let mut vcx = gpui::VisualTestContext::from_window(settings.into(), h.cx);
    let mut bounds = |s: &'static str| {
        vcx.debug_bounds(s)
            .unwrap_or_else(|| panic!("{s} is not drawn"))
    };
    let page = bounds("keymap-page");
    let combo = bounds("keymap-combo");
    let search = bounds("keymap-search");
    let tree = bounds("keymap-tree-box");
    let row = bounds("keymap-row:Editor Actions");
    eprintln!("page {page:?}\ncombo {combo:?}\nsearch {search:?}\ntree {tree:?}\nrow {row:?}");
    assert!(
        combo.bottom() < search.top(),
        "the toolbar is below the combo box"
    );
    assert!(
        search.bottom() <= tree.top(),
        "the tree is below the toolbar"
    );
    assert_eq!(f32::from(row.size.height), 22.0);
    // The tree reaches the page's bottom (the page does not scroll).
    assert!(
        f32::from(page.bottom() - tree.bottom()) < 40.0,
        "page {page:?}, tree {tree:?}"
    );
    assert!(f32::from(tree.size.height) > 250.0, "{tree:?}");
    // The search field sits at the right end of the toolbar.
    assert!(
        f32::from(tree.right() - search.right()) < 80.0,
        "{search:?} {tree:?}"
    );
}
