//! The Settings window from a main window (tests/scenarios/settings/
//! settings-window.md, settings-search.md, settings-apply-cancel.md): one
//! non-modal window per application, the page tree and its search, the
//! banner with Back and Forward, Apply, OK, Cancel and "Revert changes",
//! what the window keeps between openings, Unsaved Changes, the gear menu,
//! and the store the main windows follow.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, CursorStyle, IntoElement, Modifiers, Render, TestAppContext,
    VisualTestContext, Window, WindowHandle, div,
};
use serde_json::{Value, json};
use settings_ui::{PageContext, PageSpec, SettingsPage};
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    file: PathBuf,
    /// `settings-window.toml`.
    window_file: PathBuf,
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

    fn settings(&mut self) -> WindowHandle<settings_ui::SettingsWindow> {
        self.cx
            .update(|cx| settings_ui::find(cx))
            .expect("the Settings window")
    }

    fn rows(&mut self) -> Vec<String> {
        self.state()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// The pointer shape with the mouse over the element of the Settings
    /// window with this debug selector.
    fn shape_over(&mut self, selector: &'static str) -> CursorStyle {
        let settings = self.settings();
        let at = VisualTestContext::from_window(settings.into(), self.cx)
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
            .center();
        VisualTestContext::from_window(settings.into(), self.cx).simulate_mouse_move(
            at,
            None,
            Modifiers::none(),
        );
        self.draw();
        VisualTestContext::from_window(settings.into(), self.cx)
            .update(|window, _| ui::pointer::resolve(window))
    }

    /// Lets the search run after typing (it waits for a pause).
    fn pause(&mut self) {
        self.cx.executor().advance_clock(Duration::from_millis(150));
        self.draw();
    }
}

/// The text handed to the test page's own search.
#[derive(Default, Clone)]
struct Searched(Rc<RefCell<Vec<String>>>);

/// A page with a search of its own, as Keymap.
struct OwnSearchPage {
    searched: Searched,
}

impl Render for OwnSearchPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl SettingsPage for OwnSearchPage {
    fn is_modified(&self, _: &gpui::App) -> bool {
        false
    }

    fn apply(&mut self, _: &mut Window, _: &mut Context<Self>) -> Result<(), String> {
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn search(&mut self, text: &str, _: &mut Window, _: &mut Context<Self>) {
        self.searched.0.borrow_mut().push(text.to_string());
    }

    fn has_search(&self) -> bool {
        true
    }
}

/// Pages of the searches: Keymap with its own search, Editor with
/// General and Font, and many pages under Many (the tree scrolls).
fn test_pages(searched: &Searched) -> Vec<PageSpec> {
    let searched = searched.clone();
    let create: settings_ui::CreatePage = Rc::new(move |_: &PageContext, _, cx| {
        let searched = searched.clone();
        settings_ui::PageHandle::new(cx.new(|_| OwnSearchPage { searched }))
    });
    let mut pages = vec![
        PageSpec {
            weight: 90,
            search_terms: &["Soft-Wrap", "Main Menu"],
            create: Some(create),
            ..PageSpec::group("keymap", None, "Keymap")
        },
        PageSpec {
            weight: 80,
            ..PageSpec::group("editor", None, "Editor")
        },
        PageSpec {
            weight: 10,
            search_terms: &["Use soft wraps in editor"],
            ..PageSpec::group("editor.general", Some("editor"), "General")
        },
        PageSpec {
            weight: 5,
            search_terms: &["Size:"],
            ..PageSpec::group("editor.font", Some("editor"), "Font")
        },
        PageSpec {
            weight: 1,
            ..PageSpec::group("many", None, "Many")
        },
    ];
    const NAMES: [&str; 40] = [
        "p00", "p01", "p02", "p03", "p04", "p05", "p06", "p07", "p08", "p09", "p10", "p11", "p12",
        "p13", "p14", "p15", "p16", "p17", "p18", "p19", "p20", "p21", "p22", "p23", "p24", "p25",
        "p26", "p27", "p28", "p29", "p30", "p31", "p32", "p33", "p34", "p35", "p36", "p37", "p38",
        "p39",
    ];
    for (n, name) in NAMES.iter().enumerate() {
        pages.push(PageSpec {
            weight: -(n as i32),
            ..PageSpec::group(name, Some("many"), name)
        });
    }
    pages
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    open_with(cx, Vec::new())
}

fn open_with(cx: &mut TestAppContext, pages: Vec<PageSpec>) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).unwrap();
    let file = dir.path().join("config").join("settings.toml");
    let window_file = dir.path().join("config").join("settings-window.toml");
    let product = Product {
        name: "Test IDE",
        settings_pages: pages,
        ..Product::default()
    };
    let store_file = file.clone();
    let state_file = window_file.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store_file);
        settings_ui::window_state::load(cx, state_file);
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
        window_file,
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

#[gpui::test]
fn a_search_keeps_the_pages_found_and_selects_the_first(cx: &mut TestAppContext) {
    let searched = Searched::default();
    let mut h = open_with(cx, test_pages(&searched));
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke("settings.search", json!({"text": "soft"}));
    let s = h.state();
    assert_eq!(s["search"]["filter"], json!("soft"));
    assert_eq!(s["search"]["no_match"], json!(false));
    // Keymap (its Soft-Wrap action) and Editor > General, expanded.
    assert_eq!(h.rows(), ["keymap", "editor", "editor.general"]);
    assert_eq!(h.state()["page"], json!("keymap"));
    // Keymap was not found by its name: the text goes to its own search.
    assert_eq!(*searched.0.borrow(), ["soft"]);
    // Deleting keeps a selected page that is still shown.
    h.invoke("settings.search", json!({"text": "sof"}));
    assert_eq!(h.state()["page"], json!("keymap"));
    // A name finds the page, everything under it, and its parents.
    h.invoke("settings.search", json!({"text": "edito"}));
    // System Settings has "editor tab" among its labels.
    assert_eq!(
        h.rows(),
        [
            "appearance-behavior",
            "system",
            "editor",
            "editor.general",
            "editor.font"
        ]
    );
    assert_eq!(h.state()["page"], json!("editor"));
    // A path.
    h.invoke("settings.search", json!({"text": "Editor | Font"}));
    assert_eq!(h.rows(), ["editor", "editor.font"]);
    assert_eq!(h.state()["page"], json!("editor.font"));
    // Nothing found: the field turns red, the page stays.
    h.invoke("settings.search", json!({"text": "zzzz"}));
    let s = h.state();
    assert_eq!(s["search"]["no_match"], json!(true));
    assert_eq!(h.rows(), Vec::<String>::new());
    assert_eq!(s["page"], json!("editor.font"));
    // Escape clears the search, then closes the window.
    h.invoke("settings.escape", json!(null));
    let s = h.state();
    assert_eq!(s["open"], json!(true));
    assert_eq!(s["search"]["text"], json!(""));
    assert_eq!(h.rows().len(), 7);
    h.invoke("settings.escape", json!(null));
    assert_eq!(h.state()["open"], json!(false));
}

#[gpui::test]
fn typing_in_the_tree_searches_after_a_pause(cx: &mut TestAppContext) {
    let mut h = open_with(cx, test_pages(&Searched::default()));
    h.invoke("settings.open", json!({"page": "system"}));
    let settings = h.settings();
    h.cx.simulate_keystrokes(settings.into(), "w r a p");
    let s = h.state();
    assert_eq!(s["search"]["text"], json!("wrap"));
    assert_eq!(s["search"]["focused"], json!(true));
    // Not yet: the search waits for the typing to pause.
    assert_eq!(s["search"]["filter"], json!(""));
    h.pause();
    let s = h.state();
    assert_eq!(s["search"]["filter"], json!("wrap"));
    // `Soft-Wrap` is one word: Keymap is not found by `wrap`.
    assert_eq!(h.rows(), ["editor", "editor.general"]);
    assert_eq!(s["page"], json!("editor.general"));
    // Up moves the tree's selection; the focus stays in the field.
    h.cx.simulate_keystrokes(settings.into(), "up");
    h.draw();
    let s = h.state();
    assert_eq!(s["page"], json!("editor"));
    assert_eq!(s["search"]["focused"], json!(true));
    // The search is remembered when the field loses the focus.
    h.cx.simulate_keystrokes(settings.into(), "enter");
    h.draw();
    assert_eq!(h.state()["search"]["history"], json!(["wrap"]));
    // Cmd+F / Ctrl+F comes back to it.
    h.invoke("settings.find", json!(null));
    assert_eq!(h.state()["search"]["focused"], json!(true));
    h.invoke("settings.search-history", json!(null));
    assert_eq!(h.state()["search"]["history_open"], json!(true));
}

#[gpui::test]
fn back_and_forward_go_through_the_pages_selected(cx: &mut TestAppContext) {
    let mut h = open_with(cx, test_pages(&Searched::default()));
    h.invoke("settings.open", json!({"page": "system"}));
    assert_eq!(h.state()["can_back"], json!(false));
    h.invoke("settings.select", json!({"page": "keymap"}));
    h.invoke("settings.select", json!({"page": "editor.font"}));
    h.invoke("settings.search", json!({"text": "soft"}));
    // Back clears the search.
    h.invoke("settings.back", json!(null));
    let s = h.state();
    assert_eq!(s["search"]["text"], json!(""));
    // The search selected Keymap again, which is recorded too.
    assert_eq!(s["page"], json!("editor.font"));
    h.invoke("settings.back", json!(null));
    assert_eq!(h.state()["page"], json!("keymap"));
    h.invoke("settings.back", json!(null));
    let s = h.state();
    assert_eq!(s["page"], json!("system"));
    assert_eq!(s["can_back"], json!(false));
    assert_eq!(s["can_forward"], json!(true));
    h.invoke("settings.forward", json!(null));
    assert_eq!(h.state()["page"], json!("keymap"));
    // A new page drops what Forward would go to.
    h.invoke("settings.select", json!({"page": "editor"}));
    assert_eq!(h.state()["can_forward"], json!(false));
}

#[gpui::test]
fn the_window_keeps_its_page_size_and_splitter(cx: &mut TestAppContext) {
    let mut h = open_with(cx, test_pages(&Searched::default()));
    h.invoke("settings.open", json!({"page": "editor.font"}));
    let settings = h.settings();
    let before = h.state()["tree_width"].as_f64().unwrap();
    // Drag the splitter 100 to the right.
    let mut vcx = VisualTestContext::from_window(settings.into(), h.cx);
    let bounds = vcx.debug_bounds("settings-splitter").expect("the splitter");
    let at = bounds.center();
    vcx.simulate_mouse_down(at, gpui::MouseButton::Left, gpui::Modifiers::none());
    vcx.simulate_mouse_move(
        gpui::point(at.x + gpui::px(100.0), at.y),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::none(),
    );
    vcx.simulate_mouse_up(
        gpui::point(at.x + gpui::px(100.0), at.y),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    h.draw();
    let after = h.state()["tree_width"].as_f64().unwrap();
    assert_eq!(after, before + 100.0);
    h.invoke("settings.search", json!({"text": "font"}));
    h.invoke("settings.cancel", json!(null));
    let text = std::fs::read_to_string(&h.window_file).unwrap_or_default();
    assert!(text.contains("page = \"editor.font\""), "{text}");
    assert!(text.contains("split = "), "{text}");
    assert!(text.contains("search-history = [\"font\"]"), "{text}");
    // Opened again without a page: the last one, the same splitter.
    h.invoke("settings.open", json!(null));
    let s = h.state();
    assert_eq!(s["page"], json!("editor.font"));
    assert_eq!(s["tree_width"].as_f64().unwrap(), after);
}

#[gpui::test]
fn the_group_stays_on_top_of_the_scrolled_tree(cx: &mut TestAppContext) {
    let mut h = open_with(cx, test_pages(&Searched::default()));
    h.invoke("settings.open", json!({"page": "p39"}));
    // The tree scrolls to the page once it is laid out.
    h.draw();
    h.draw();
    let s = h.state();
    assert_eq!(s["sticky_header"], json!("Many"), "{s}");
    h.invoke("settings.select", json!({"page": "appearance-behavior"}));
    assert_eq!(h.state()["sticky_header"], json!(null));
}

#[gpui::test]
fn closing_the_project_with_changes_asks_first(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": 30}),
    );
    let main = h.window;
    let close = |h: &mut H| VisualTestContext::from_window(main.into(), h.cx).simulate_close();
    // Cancel: nothing closes.
    assert!(!close(&mut h));
    assert!(h.cx.has_pending_prompt());
    h.cx.simulate_prompt_answer("Cancel");
    h.draw();
    assert_eq!(h.state()["open"], json!(true));
    assert_eq!(h.state()["modified"], json!(["system"]));
    // Apply: stored, then both windows close.
    assert!(!close(&mut h));
    h.cx.simulate_prompt_answer("Apply");
    h.draw();
    assert!(h.file_text().contains("idle-seconds = 30"));
    assert!(h.cx.update(|cx| settings_ui::find(cx)).is_none());
    assert!(!h.cx.update(|cx| cx.windows().contains(&main.into())));
}

#[gpui::test]
fn closing_the_project_without_saving_drops_the_changes(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": 30}),
    );
    let main = h.window;
    assert!(!VisualTestContext::from_window(main.into(), h.cx).simulate_close());
    h.cx.simulate_prompt_answer("Don't Save");
    h.draw();
    assert_eq!(h.file_text(), "");
    assert!(!h.cx.update(|cx| cx.windows().contains(&main.into())));
}

#[gpui::test]
fn quitting_with_changes_asks_first(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": 30}),
    );
    h.invoke("app.quit", json!(null));
    assert!(h.cx.has_pending_prompt());
    h.cx.simulate_prompt_answer("Cancel");
    h.draw();
    assert_eq!(h.state()["open"], json!(true));
    // Without changes nothing is asked (the window would quit).
    h.invoke("settings.revert", json!(null));
    assert!(!h.cx.has_pending_prompt());
}

#[gpui::test]
fn the_gear_menu_opens_settings(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let main = h.window;
    let mut vcx = VisualTestContext::from_window(main.into(), h.cx);
    let gear = vcx.debug_bounds("toolbar-settings").expect("the gear");
    vcx.simulate_click(gear.center(), gpui::Modifiers::none());
    h.draw();
    let menu = h.invoke("run.state", json!(null))["menu"].clone();
    let items = &menu["levels"][0]["items"];
    assert_eq!(items[0]["label"], json!("Settings\u{2026}"), "{menu}");
    assert_eq!(items.as_array().unwrap().len(), 1, "{menu}");
    h.invoke("menu.down", json!(null));
    h.invoke("menu.confirm", json!(null));
    assert_eq!(h.state()["open"], json!(true));
}

/// #181: links and the icons in the search field show the pointing hand,
/// the divider its resize shape, rows and buttons the arrow.
#[gpui::test]
fn links_show_the_hand_and_rows_and_buttons_the_arrow(cx: &mut TestAppContext) {
    let mut h = open_with(cx, test_pages(&Searched::default()));
    // A group page: links to its children.
    h.invoke("settings.open", json!({"page": "editor"}));
    assert_eq!(
        h.shape_over("settings-link:editor.general"),
        CursorStyle::PointingHand
    );
    // The search field: the history icon, and the clear icon with a text.
    assert_eq!(
        h.shape_over("settings-search-history"),
        CursorStyle::PointingHand
    );
    h.invoke("settings.search", json!({"text": "font"}));
    assert_eq!(
        h.shape_over("settings-search-clear"),
        CursorStyle::PointingHand
    );
    // The divider between the tree and the page.
    assert_eq!(
        h.shape_over("settings-splitter"),
        CursorStyle::ResizeLeftRight
    );
    // Tree rows and the Back button: the arrow.
    assert_eq!(h.shape_over("settings-row:editor.font"), CursorStyle::Arrow);
    h.invoke("settings.search", json!({"text": ""}));
    h.invoke("settings.select", json!({"page": "editor.general"}));
    assert_eq!(h.shape_over("settings-back"), CursorStyle::Arrow);
    // "Revert changes" in the banner of a modified page.
    h.invoke("settings.open", json!({"page": "system"}));
    h.invoke(
        "settings.edit",
        json!({"field": "idle-seconds", "value": 30}),
    );
    assert_eq!(h.shape_over("settings-revert"), CursorStyle::PointingHand);
    // The check boxes of the page keep the arrow.
    assert_eq!(h.shape_over("system:save-on-idle"), CursorStyle::Arrow);
}
