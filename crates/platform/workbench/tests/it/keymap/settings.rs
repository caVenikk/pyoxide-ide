//! The Settings window and the first-run window: the bindings of `keymaps/*/settings.toml`.

use super::*;

area_tests!("settings");

/// The first-run window over the main window (`settings.import-reference`)
/// with nothing to import: it opens on the import page, where Enter
/// imports the newest folder (there is none) and Escape closes it.
fn check_first_run(h: &mut H, command: &str, keys: &str, what: &str) {
    h.cx.update(|cx| settings_ui::first_run::set_reference_root(cx, None));
    h.invoke("settings.import-reference", json!(null));
    let dialog =
        h.cx.update(|cx| ui::dialog_window::child_of(h.window.into(), cx))
            .expect("the first-run window");
    let state = |h: &mut H| h.invoke("first-run.state", json!(null));
    assert_eq!(state(h)["page"], json!("import"), "{what}");
    h.dispatched.borrow_mut().clear();
    h.cx.simulate_keystrokes(dialog, keys);
    h.draw();
    if command == "first-run.continue" {
        assert_eq!(state(h)["page"], json!("import"), "{what}: nothing to import");
        // Escape closes the window again.
        h.cx.simulate_keystrokes(dialog, "escape");
        h.draw();
    }
    let open =
        h.cx.update(|cx| ui::dialog_window::child_of(h.window.into(), cx));
    assert!(open.is_none(), "{what}: the window closes");
}

impl H<'_> {
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
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    match b.command.as_str() {
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
            // Appearance & Behavior with Appearance and System Settings:
            // two rows come and go (the other groups stay as they are).
            if b.command == "settings.collapse" {
                assert_eq!(before, after + 2, "{what}");
            } else {
                assert_eq!(before + 2, after, "{what}");
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
        "first-run.continue" | "first-run.cancel" => check_first_run(h, &b.command, keys, &what),
        _ => return false,
    }
    true
}
