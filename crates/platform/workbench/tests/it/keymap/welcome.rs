//! Welcome: the bindings of `keymaps/*/welcome.toml`, the keyboard of the
//! Projects and Files list. The window has no project there (the list
//! shows while none is open), so this area starts its own app instead of
//! `setup`'s, with four recent projects: `alpha` and `beta` (folders that
//! exist) and `gone-a` and `gone-b` (folders that are gone), in that order.

use super::*;
use workbench::recent_projects;

pub(super) const FILE: &str = "welcome";

#[gpui::test]
fn every_macos_binding_works(cx: &mut TestAppContext) {
    run(
        cx,
        keymap::MACOS,
        include_str!("../../../keymaps/macos/welcome.toml"),
    );
}

#[gpui::test]
fn every_windows_and_linux_binding_works(cx: &mut TestAppContext) {
    run(
        cx,
        keymap::DEFAULT,
        include_str!("../../../keymaps/default/welcome.toml"),
    );
}

fn run(cx: &mut TestAppContext, keymap_text: &str, area_text: &str) {
    let (mut h, _dir) = setup(cx, keymap_text);
    super::press_area(&mut h, area_text, check);
}

fn setup<'a>(cx: &'a mut TestAppContext, keymap_text: &str) -> (H<'a>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    let file = root.join("config").join(recent_projects::FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut list = recent_projects::RecentProjects::default();
    // The one opened last goes first.
    for (ix, name) in ["gone-b", "gone-a", "beta", "alpha"].into_iter().enumerate() {
        let path = if name.starts_with("gone") {
            PathBuf::from("/nonexistent").join(name)
        } else {
            let path = root.join("projects").join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        };
        list.opened(&path, name, ix as u64, 0, |_| false);
    }
    std::fs::write(&file, list.to_toml()).unwrap();
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let keymap = Keymap::parse(keymap_text).unwrap();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        recent_projects::load(cx, file);
        cx.clear_key_bindings();
        let errors = keymap::bind(cx, &keymap);
        assert!(errors.is_empty(), "keymap errors: {errors:?}");
        workbench::open_main_window(cx, Rc::new(product))
    });
    let dispatched = cx.update(Dispatched::observe);
    let mut h = H {
        cx,
        window,
        root,
        dispatched,
    };
    h.draw();
    (h, dir)
}

impl H<'_> {
    /// The list's state (`projects_and_files`).
    fn welcome_state(&mut self) -> Value {
        let s = self.state()["projects_and_files"].clone();
        assert!(s.is_object(), "no Projects and Files: {s}");
        s
    }

    fn welcome_names(&mut self) -> Vec<String> {
        self.welcome_state()["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The name of the selected row.
    fn welcome_selected(&mut self) -> Option<String> {
        let w = self.welcome_state();
        let folder = w["selected"].as_str()?.to_owned();
        w["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == json!(folder))
            .map(|p| p["name"].as_str().unwrap().to_owned())
    }

    fn welcome_search(&mut self) -> Entity<Editor> {
        let panel = self.workbench(|w, _| w.panel().clone());
        self.cx
            .update(|cx| panel.read(cx).welcome_search(cx))
            .expect("the search field")
    }

    fn welcome_focus_search(&mut self) {
        let search = self.welcome_search();
        let handle = self.cx.update(|cx| search.focus_handle(cx));
        self.focus(handle);
    }

    fn welcome_search_text(&mut self) -> String {
        let search = self.welcome_search();
        self.cx.update(|cx| search.read(cx).text())
    }

    fn welcome_set_search_text(&mut self, text: &str) {
        let search = self.welcome_search();
        self.cx
            .update(|cx| search.update(cx, |e, cx| e.set_text_selected(text, cx)));
        self.draw();
    }

    /// The list itself has the focus, not the search field.
    fn welcome_list_focused(&mut self) -> bool {
        let panel = self.workbench(|w, _| w.panel().clone());
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                panel.focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    /// No dialog, no text in the search field, the list focused with the
    /// first row selected.
    fn welcome_start(&mut self) {
        if !self.state()["dialog"].is_null() {
            self.invoke("dialog.cancel", json!(null));
        }
        self.welcome_set_search_text("");
        self.focus_tree();
        self.invoke("welcome-projects.first", json!(null));
        assert_eq!(self.welcome_selected().as_deref(), Some("alpha"));
    }

    /// Selects the row `name` with the commands, from the list.
    fn welcome_select(&mut self, name: &str) {
        self.focus_tree();
        let ix = self.welcome_names().iter().position(|n| n == name).unwrap();
        self.invoke("welcome-projects.first", json!(null));
        for _ in 0..ix {
            self.invoke("welcome-projects.down", json!(null));
        }
        assert_eq!(self.welcome_selected().as_deref(), Some(name));
    }

    fn welcome_clipboard(&mut self) -> Option<String> {
        self.cx.read_from_clipboard().and_then(|c| c.text())
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    let in_field = b.context.as_deref() == Some("WelcomeProjects > Editor");
    h.welcome_start();
    // Where the keys are pressed: the list, or the search field.
    let focus = |h: &mut H| {
        if in_field {
            h.welcome_focus_search();
        } else {
            h.focus_tree();
        }
    };
    match b.command.as_str() {
        "welcome-projects.up" => {
            h.welcome_select("gone-a");
            focus(h);
            h.press(keys);
            assert_eq!(h.welcome_selected().as_deref(), Some("beta"), "{what}");
            h.press(keys);
            h.press(keys);
            assert_eq!(h.welcome_selected().as_deref(), Some("alpha"), "{what}: no wrapping");
        }
        "welcome-projects.down" => {
            focus(h);
            h.press(keys);
            assert_eq!(h.welcome_selected().as_deref(), Some("beta"), "{what}");
            h.welcome_select("gone-b");
            focus(h);
            h.press(keys);
            assert_eq!(h.welcome_selected().as_deref(), Some("gone-b"), "{what}: no wrapping");
        }
        "welcome-projects.first" => {
            h.welcome_select("beta");
            h.press(keys);
            assert_eq!(h.welcome_selected().as_deref(), Some("alpha"), "{what}");
        }
        "welcome-projects.last" => {
            h.press(keys);
            assert_eq!(h.welcome_selected().as_deref(), Some("gone-b"), "{what}");
        }
        "welcome-projects.open" => {
            // A folder that is gone asks "Reopen Project" first.
            let name = if in_field { "gone-b" } else { "gone-a" };
            h.welcome_select(name);
            focus(h);
            h.press(keys);
            let s = h.state();
            assert_eq!(s["dialog"]["title"], json!("Reopen Project"), "{what}");
            let message = s["dialog"]["message"].as_str().unwrap_or_default();
            assert!(message.contains(&format!("{name} does not exist.")), "{what}: {s}");
        }
        "welcome-projects.remove" => {
            h.welcome_select("beta");
            focus(h);
            h.press(keys);
            let s = h.state();
            assert_eq!(s["dialog"]["title"], json!("Remove Recent Project"), "{what}");
            assert_eq!(
                s["dialog"]["message"],
                json!("Remove 'beta' from recent projects list?"),
                "{what}"
            );
            h.invoke("dialog.cancel", json!(null));
            assert_eq!(h.welcome_names().len(), 4, "{what}: asked, not removed");
        }
        "welcome-projects.copy-path" => {
            h.welcome_select("beta");
            h.cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
            // Not in the search field.
            h.welcome_focus_search();
            h.press(keys);
            assert_eq!(h.welcome_clipboard().as_deref(), Some("before"), "{what}: in the field");
            h.focus_tree();
            h.press(keys);
            let want = h.root.join("projects").join("beta");
            assert_eq!(h.welcome_clipboard().as_deref(), want.to_str(), "{what}");
        }
        "welcome-projects.escape" => {
            h.welcome_focus_search();
            h.welcome_set_search_text("alp");
            h.welcome_focus_search();
            h.press(keys);
            assert_eq!(h.welcome_search_text(), "", "{what}: clears the text");
            assert!(!h.welcome_list_focused(), "{what}: stays in the field");
            h.press(keys);
            assert!(h.welcome_list_focused(), "{what}: then moves to the list");
        }
        _ => return false,
    }
    true
}
