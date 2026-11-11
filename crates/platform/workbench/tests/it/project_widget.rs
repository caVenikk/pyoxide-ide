//! The project widget's popup and the recent projects (#272): a project
//! that opens goes first in the recent projects (kept in the
//! configuration folder); the popup shows Open…, Clone Repository…, the
//! project of the window under "Open Projects" and the others under
//! "Recent Projects", each with its avatar and its folder; choosing a
//! recent project opens it in the window.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, Focusable as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::recent_projects;
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    dir: tempfile::TempDir,
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        let _ = self
            .cx
            .update_window(self.window.into(), |_, window, cx| window.draw(cx).clear());
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

    /// A folder of the test's own.
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join("projects").join(name);
        std::fs::create_dir_all(&path).unwrap();
        native_path::canonicalize(&path).unwrap()
    }

    fn open(&mut self, path: &Path) {
        self.invoke(
            "project.open-folder",
            json!({ "path": path.to_string_lossy() }),
        );
    }

    /// The popup's rows: (label or header, comment, avatar).
    fn popup(&mut self) -> Vec<(String, String, String)> {
        let s = self.invoke("project-widget.state", json!(null));
        assert_eq!(s["open"], json!(true), "{s}");
        s["popup"]["levels"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                let text = |key: &str| i[key].as_str().unwrap_or("").to_owned();
                match i["header"].as_str() {
                    Some(h) => (format!("# {h}"), String::new(), String::new()),
                    None => (text("label"), text("comment"), text("avatar")),
                }
            })
            .collect()
    }

    fn recent(&mut self) -> Vec<String> {
        self.cx.update(|cx| {
            recent_projects::list(cx)
                .projects()
                .iter()
                .map(|p| p.name.clone())
                .collect()
        })
    }

    fn file(&self) -> PathBuf {
        self.dir.path().join("config").join(recent_projects::FILE)
    }
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config").join(recent_projects::FILE);
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        recent_projects::load(cx, file);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H { cx, window, dir };
    h.draw();
    h
}

#[gpui::test]
fn opened_projects_are_recent_and_the_popup_lists_them(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let alpha = h.folder("alpha");
    let beta = h.folder("my-sample-app");
    h.open(&alpha);
    h.open(&beta);
    assert_eq!(h.recent(), ["my-sample-app", "alpha"]);
    // Written to the configuration folder, the one opened last first.
    let text = std::fs::read_to_string(h.file()).unwrap();
    let stored = recent_projects::RecentProjects::parse(&text);
    let names: Vec<&str> = stored.projects().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["my-sample-app", "alpha"]);
    assert_eq!(stored.projects()[0].path, beta);

    h.invoke("project-widget.show", json!(null));
    let rows = h.popup();
    let display = |p: &Path| project::display_path(p);
    assert_eq!(
        rows,
        [
            ("Open\u{2026}".into(), String::new(), String::new()),
            (
                "Clone Repository\u{2026}".into(),
                String::new(),
                String::new()
            ),
            ("# Open Projects".into(), String::new(), String::new()),
            ("my-sample-app".into(), display(&beta), "MA".into()),
            ("# Recent Projects".into(), String::new(), String::new()),
            ("alpha".into(), display(&alpha), "A".into()),
        ]
    );
    let s = h.invoke("project-widget.state", json!(null));
    let items = &s["popup"]["levels"][0]["items"];
    assert_eq!(items[0]["command"], json!("project.open-folder"));
    assert_eq!(items[1]["enabled"], json!(false), "no clone dialog yet");
    assert_eq!(items[3]["command"], json!("project.open-recent"));
    // The first row is selected.
    assert_eq!(s["popup"]["levels"][0]["selected"], json!(0));

    // The widget again closes it.
    h.invoke("project-widget.show", json!(null));
    let s = h.invoke("project-widget.state", json!(null));
    assert_eq!(s["open"], json!(false));
}

/// The project's colour belongs to the project (#336): the same project has
/// the same avatar in every theme, and the same toolbar colour in the themes
/// of one kind, whatever their controls.
#[gpui::test]
fn the_project_colour_does_not_follow_the_theme(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let alpha = h.folder("alpha");
    h.open(&alpha);
    let color =
        h.cx.update(|cx| recent_projects::color(&alpha, cx))
            .expect("the opened project has a colour");
    let mut classic_dark = themes::builtin::dark();
    classic_dark.controls = themes::Controls::Classic;
    let mut classic_light = themes::builtin::light();
    classic_light.controls = themes::Controls::Classic;
    let all = [
        themes::builtin::default_theme(themes::ThemeKind::Dark),
        themes::builtin::dark(),
        classic_dark,
        themes::builtin::default_theme(themes::ThemeKind::Light),
        classic_light,
    ];
    let seen: Vec<_> = all
        .into_iter()
        .map(|theme| {
            h.cx.update(|cx| {
                let dark = theme.kind == themes::ThemeKind::Dark;
                ui::theme::set(cx, theme);
                assert_eq!(recent_projects::color(&alpha, cx), Some(color));
                (
                    dark,
                    recent_projects::avatar("alpha", color),
                    recent_projects::toolbar_color(color, cx),
                )
            })
        })
        .collect();
    for (dark, avatar, toolbar) in &seen {
        assert_eq!(avatar, &seen[0].1);
        let same_kind = seen.iter().find(|s| s.0 == *dark).unwrap();
        assert_eq!(toolbar, &same_kind.2);
    }
}

/// In the Islands themes the project's colour is the background of the
/// whole frame (#285): one layer from the window's top left corner, 600
/// right of the avatar's centre and 300 high, under the toolbar, the
/// stripes, the islands and the status bar, which paint no background of
/// their own. The other themes paint it on the main toolbar.
#[gpui::test]
fn the_project_colour_is_the_frame_background_in_the_islands_themes(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let alpha = h.folder("alpha");
    h.open(&alpha);
    let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), h.cx);
    let frame = vcx
        .debug_bounds("frame-project-color")
        .expect("the frame's project colour");
    assert_eq!(frame.origin, gpui::point(gpui::px(0.0), gpui::px(0.0)));
    assert_eq!(frame.size.height, gpui::px(300.0));
    assert!(frame.size.width > gpui::px(600.0), "{frame:?}");
    assert!(vcx.debug_bounds("toolbar-project-color").is_none());

    let mut classic = themes::builtin::dark();
    classic.controls = themes::Controls::Classic;
    h.cx.update(|cx| ui::theme::set(cx, classic));
    h.draw();
    // (Debug bounds outlive the frames that drew them: only what is drawn
    // is checked.)
    let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), h.cx);
    let toolbar = vcx
        .debug_bounds("toolbar-project-color")
        .expect("the toolbar's project colour");
    assert_eq!(toolbar.origin.y, gpui::px(0.0));
}

#[gpui::test]
fn a_recent_project_opens_in_the_window(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let alpha = h.folder("alpha");
    let beta = h.folder("beta");
    h.open(&alpha);
    h.open(&beta);
    h.invoke("project-widget.show", json!(null));
    // Typing filters the rows; Enter opens the one selected.
    for key in ["a", "l", "p"] {
        h.cx.simulate_keystrokes(h.window.into(), key);
    }
    h.draw();
    let s = h.invoke("project-widget.state", json!(null));
    assert_eq!(s["popup"]["filter"], json!("alp"), "{s}");
    h.cx.simulate_keystrokes(h.window.into(), "enter");
    h.draw();
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(s["project"]["root"], json!(alpha.to_string_lossy()), "{s}");
    assert_eq!(h.recent(), ["alpha", "beta"]);
    let s = h.invoke("project-widget.state", json!(null));
    assert_eq!(s["open"], json!(false));

    // The window's own project: nothing happens.
    h.invoke(
        "project.open-recent",
        json!({ "path": alpha.to_string_lossy() }),
    );
    assert_eq!(h.recent(), ["alpha", "beta"]);
    // Another one opens here.
    h.invoke(
        "project.open-recent",
        json!({ "path": beta.to_string_lossy() }),
    );
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(s["project"]["root"], json!(beta.to_string_lossy()));
    assert_eq!(h.recent(), ["beta", "alpha"]);
}

#[gpui::test]
fn the_list_is_read_from_the_configuration_folder(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config").join(recent_projects::FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    // A folder that is gone stays listed.
    std::fs::write(
        &file,
        "last-color = 2\n\n[[project]]\npath = \"/nonexistent/old-app\"\nname = \"old-app\"\nopened = 5\ncolor = 7\n",
    )
    .unwrap();
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        recent_projects::load(cx, file.clone());
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H { cx, window, dir };
    h.draw();
    assert_eq!(h.recent(), ["old-app"]);
    let fresh = h.folder("fresh");
    h.open(&fresh);
    assert_eq!(h.recent(), ["fresh", "old-app"]);
    let stored = recent_projects::RecentProjects::parse(&std::fs::read_to_string(&file).unwrap());
    let new = &stored.projects()[0];
    assert_eq!(
        (new.name.as_str(), new.color),
        ("fresh", 3),
        "the next colour"
    );
    assert_eq!(stored.projects()[1].color, 7, "kept");
    h.invoke("project-widget.show", json!(null));
    let rows = h.popup();
    assert_eq!(rows[5].0, "old-app");
    assert_eq!(
        rows[5].1,
        project::display_path(Path::new("/nonexistent/old-app"))
    );
}

#[gpui::test]
fn the_welcome_list_shows_the_recent_projects(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config").join(recent_projects::FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let here = dir.path().join("projects").join("my-app");
    std::fs::create_dir_all(&here).unwrap();
    let here = native_path::canonicalize(&here).unwrap();
    let mut list = recent_projects::RecentProjects::default();
    list.opened(Path::new("/nonexistent/old-app"), "old-app", 1, 0, |_| {
        false
    });
    list.opened(&here, "my-app", 2, 0, |_| false);
    std::fs::write(&file, list.to_toml()).unwrap();
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        recent_projects::load(cx, file.clone());
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H { cx, window, dir };
    h.draw();
    // Without a project: Projects and Files lists them, a gone folder
    // greyed.
    let panel = h.invoke("workbench.state", json!(null))["projects_and_files"].clone();
    let rows: Vec<(String, String, bool)> = panel["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_owned(),
                p["avatar"].as_str().unwrap().to_owned(),
                p["gone"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("my-app".to_owned(), "MA".to_owned(), false),
            ("old-app".to_owned(), "OA".to_owned(), true),
        ],
        "{panel}"
    );

    // Copy Path of the row's menu.
    h.invoke(
        "project.copy-recent-path",
        json!({ "path": here.to_string_lossy() }),
    );
    let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some(here.to_str().unwrap()));

    // Remove from Recent Projects… asks first.
    let old = Path::new("/nonexistent/old-app")
        .to_string_lossy()
        .into_owned();
    h.invoke("project.remove-recent", json!({ "path": old }));
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(s["dialog"]["title"], json!("Remove Recent Project"), "{s}");
    assert_eq!(
        s["dialog"]["message"],
        json!("Remove 'old-app' from recent projects list?")
    );
    h.invoke("dialog.cancel", json!(null));
    assert_eq!(h.recent(), ["my-app", "old-app"]);
    h.invoke("project.remove-recent", json!({ "path": old }));
    h.invoke("dialog.choose", json!({ "button": "Remove" }));
    assert_eq!(h.recent(), ["my-app"]);
    let panel = h.invoke("workbench.state", json!(null))["projects_and_files"].clone();
    assert_eq!(panel["projects"].as_array().unwrap().len(), 1, "{panel}");

    // Open Selected opens it in the window.
    h.invoke(
        "project.open-recent",
        json!({ "path": here.to_string_lossy() }),
    );
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(s["project"]["root"], json!(here.to_string_lossy()));
}

/// A window without a project, the recent projects `names` (folders that
/// exist, the first name the one opened last) loaded from the
/// configuration folder.
fn open_listed<'a>(cx: &'a mut TestAppContext, names: &[&str]) -> (H<'a>, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config").join(recent_projects::FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut list = recent_projects::RecentProjects::default();
    let mut folders = Vec::new();
    for (ix, name) in names.iter().enumerate().rev() {
        let path = dir.path().join("projects").join(name);
        std::fs::create_dir_all(&path).unwrap();
        let path = native_path::canonicalize(&path).unwrap();
        list.opened(&path, name, ix as u64, 0, |_| false);
        folders.push(path);
    }
    folders.reverse();
    std::fs::write(&file, list.to_toml()).unwrap();
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        recent_projects::load(cx, file);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H { cx, window, dir };
    h.draw();
    (h, folders)
}

impl H<'_> {
    /// Projects and Files, for `workbench.state`.
    fn welcome(&mut self) -> Value {
        let s = self.invoke("workbench.state", json!(null))["projects_and_files"].clone();
        assert!(s.is_object(), "no Projects and Files: {s}");
        s
    }

    /// The name of the selected row.
    fn selected(&mut self) -> Option<String> {
        let w = self.welcome();
        let folder = w["selected"].as_str()?.to_owned();
        w["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == json!(folder))
            .map(|p| p["name"].as_str().unwrap().to_owned())
    }

    /// The tool window's focus (the list) or the search field's.
    fn focus_welcome(&mut self, search: bool) {
        let panel = self.workbench_panel();
        let handle = self.cx.update(|cx| {
            if search {
                let field = panel.read(cx).welcome_search(cx).expect("the search field");
                field.focus_handle(cx)
            } else {
                panel.focus_handle(cx)
            }
        });
        self.cx
            .update_window(self.window.into(), |_, window, _| window.focus(&handle))
            .unwrap();
        self.draw();
    }

    fn workbench_panel(&mut self) -> gpui::Entity<workbench::project_panel::ProjectPanel> {
        self.window
            .read_with(self.cx, |w, _| w.panel().clone())
            .unwrap()
    }

    fn press(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(self.window.into(), keys);
        self.draw();
    }
}

#[gpui::test]
fn the_search_field_selects_and_opens_with_the_keys(cx: &mut TestAppContext) {
    let (mut h, folders) = open_listed(cx, &["alpha", "beta", "gamma"]);
    // The first row is selected at the start.
    assert_eq!(h.selected().as_deref(), Some("alpha"));
    // Up and Down in the search field move the selection, without wrapping.
    h.focus_welcome(true);
    h.press("down");
    assert_eq!(h.selected().as_deref(), Some("beta"));
    h.press("down down");
    assert_eq!(h.selected().as_deref(), Some("gamma"), "no wrapping");
    h.press("up");
    assert_eq!(h.selected().as_deref(), Some("beta"));
    // The selection stays while it is listed; else the first row is
    // selected.
    h.press("b");
    assert_eq!(h.welcome()["search"], json!("b"));
    assert_eq!(h.selected().as_deref(), Some("beta"));
    h.press("backspace g a m m a");
    assert_eq!(h.welcome()["search"], json!("gamma"));
    assert_eq!(h.selected().as_deref(), Some("gamma"));
    // Enter opens it.
    h.press("enter");
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(
        s["project"]["root"],
        json!(folders[2].to_string_lossy()),
        "{s}"
    );
}

#[gpui::test]
fn the_list_selects_and_removes_with_the_keys(cx: &mut TestAppContext) {
    let (mut h, _) = open_listed(cx, &["alpha", "beta", "gamma"]);
    h.focus_welcome(false);
    h.press("end");
    assert_eq!(h.selected().as_deref(), Some("gamma"));
    h.press("home");
    assert_eq!(h.selected().as_deref(), Some("alpha"));
    h.press("down");
    assert_eq!(h.selected().as_deref(), Some("beta"));

    // Delete asks first.
    h.press("delete");
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(s["dialog"]["title"], json!("Remove Recent Project"), "{s}");
    assert_eq!(
        s["dialog"]["message"],
        json!("Remove 'beta' from recent projects list?")
    );
    h.invoke("dialog.cancel", json!(null));
    assert_eq!(h.recent(), ["alpha", "beta", "gamma"]);
    assert_eq!(h.selected().as_deref(), Some("beta"));
    h.press("delete");
    h.invoke("dialog.choose", json!({ "button": "Remove" }));
    assert_eq!(h.recent(), ["alpha", "gamma"]);
    // Nothing is selected then; Down selects the first row, Up the last.
    assert_eq!(h.selected(), None);
    h.press("down");
    assert_eq!(h.selected().as_deref(), Some("alpha"));
    h.press("home");
    h.press("up");
    assert_eq!(h.selected().as_deref(), Some("alpha"), "no wrapping");
    h.press("end");
    assert_eq!(h.selected().as_deref(), Some("gamma"));

    // Escape in the search field clears its text, then moves to the list.
    h.focus_welcome(true);
    h.press("g");
    assert_eq!(h.welcome()["search"], json!("g"));
    h.press("escape");
    assert_eq!(h.welcome()["search"], json!(""));
    h.press("escape");
    // The list has the focus now: Home selects the first row.
    h.press("home");
    assert_eq!(h.selected().as_deref(), Some("alpha"));
}

#[gpui::test]
fn the_pointer_selects_the_row_it_is_over(cx: &mut TestAppContext) {
    let (mut h, _) = open_listed(cx, &["alpha", "beta", "gamma"]);
    let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), h.cx);
    let beta = vcx
        .debug_bounds("welcome-project:beta")
        .expect("the row")
        .center();
    vcx.simulate_mouse_move(beta, None, gpui::Modifiers::none());
    assert_eq!(h.selected().as_deref(), Some("beta"));
    // The keys go on from there.
    h.focus_welcome(true);
    h.press("down");
    assert_eq!(h.selected().as_deref(), Some("gamma"));
}

/// Writes a work tree's `HEAD` (no git needed).
fn work_tree(root: &Path, head: &str) {
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git").join("HEAD"), head).unwrap();
}

impl H<'_> {
    /// The open dialog: title, message, buttons, default.
    fn dialog(&mut self) -> Value {
        self.invoke("workbench.state", json!(null))["dialog"].clone()
    }
}

#[gpui::test]
fn rows_show_the_branch_and_a_gone_folder_asks_before_it_opens(cx: &mut TestAppContext) {
    let (mut h, folders) = open_listed(cx, &["on-branch", "detached", "plain"]);
    work_tree(&folders[0], "ref: refs/heads/main\n");
    work_tree(&folders[1], "376912943768808b0ac3f0bf3c5f4a6db2beda1c\n");
    // A folder that is gone below the first work tree.
    let gone = folders[0].join("gone-app");
    h.cx.update(|cx| recent_projects::opened(cx, &gone, "gone-app"));
    h.draw();
    let w = h.welcome();
    let rows: Vec<(String, Value, bool)> = w["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_owned(),
                p["branch"].clone(),
                p["gone"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("gone-app".to_owned(), json!("main"), true),
            ("on-branch".to_owned(), json!("main"), false),
            ("detached".to_owned(), json!(null), false),
            ("plain".to_owned(), json!(null), false),
        ],
        "{w}"
    );

    // Opening it from the list (Enter, a click): "Reopen Project", Cancel
    // or Remove From List (the default), which removes it without asking
    // again.
    let message = format!(
        "The path {} does not exist.\nIf it is on a removable or network drive, please make sure that the drive is connected.",
        gone.display()
    );
    h.focus_welcome(false);
    h.press("home enter");
    let d = h.dialog();
    assert_eq!(d["title"], json!("Reopen Project"), "{d}");
    assert_eq!(d["message"], json!(message));
    assert_eq!(d["buttons"], json!(["Cancel", "Remove From List"]));
    assert_eq!(d["default"], json!("Remove From List"));
    h.invoke("dialog.cancel", json!(null));
    assert_eq!(h.recent(), ["gone-app", "on-branch", "detached", "plain"]);
    assert_eq!(
        h.invoke("workbench.state", json!(null))["project"],
        json!(null)
    );
    h.invoke(
        "project.open-recent",
        json!({ "path": gone.to_string_lossy(), "welcome": true }),
    );
    h.invoke("dialog.confirm", json!(null));
    assert_eq!(h.dialog(), json!(null));
    assert_eq!(h.recent(), ["on-branch", "detached", "plain"]);

    // From the popup: Remove From List or OK (the default), which keeps it.
    h.cx.update(|cx| recent_projects::opened(cx, &gone, "gone-app"));
    h.open(&folders[2]);
    h.invoke(
        "project.open-recent",
        json!({ "path": gone.to_string_lossy() }),
    );
    let d = h.dialog();
    assert_eq!(d["title"], json!("Reopen Project"), "{d}");
    assert_eq!(d["buttons"], json!(["Remove From List", "OK"]));
    assert_eq!(d["default"], json!("OK"));
    h.invoke("dialog.confirm", json!(null));
    assert_eq!(h.recent(), ["plain", "gone-app", "on-branch", "detached"]);
    let s = h.invoke("workbench.state", json!(null));
    assert_eq!(s["project"]["root"], json!(folders[2].to_string_lossy()));

    // The popup shows the branches as last looked at.
    h.invoke("project-widget.show", json!(null));
    let s = h.invoke("project-widget.state", json!(null));
    let branches: Vec<(String, Value)> = s["popup"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["avatar"].is_string())
        .map(|i| (i["label"].as_str().unwrap().to_owned(), i["branch"].clone()))
        .collect();
    assert_eq!(
        branches,
        [
            ("plain".to_owned(), json!(null)),
            ("gone-app".to_owned(), json!("main")),
            ("on-branch".to_owned(), json!("main")),
            ("detached".to_owned(), json!(null)),
        ],
        "{s}"
    );
}

#[gpui::test]
fn a_row_menu_has_the_reference_actions(cx: &mut TestAppContext) {
    let (mut h, folders) = open_listed(cx, &["alpha", "beta"]);
    let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), h.cx);
    let beta = vcx
        .debug_bounds("welcome-project:beta")
        .expect("the row")
        .center();
    vcx.simulate_mouse_move(beta, None, gpui::Modifiers::none());
    // The selected row's button (a folder that exists: "more").
    let button = vcx
        .debug_bounds("welcome-project-button")
        .expect("the row's button")
        .center();
    vcx.simulate_click(button, gpui::Modifiers::none());
    let w = h.welcome();
    let items = w["menu"]["levels"][0]["items"].as_array().unwrap().clone();
    let rows: Vec<(String, bool)> = items
        .iter()
        .map(|i| match i["label"].as_str() {
            Some(label) => (label.to_owned(), i["enabled"].as_bool().unwrap()),
            None => ("-".to_owned(), false),
        })
        .collect();
    let reveal = if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Show in File Manager"
    };
    assert_eq!(
        rows,
        [
            ("Open Selected".to_owned(), true),
            (reveal.to_owned(), true),
            ("Copy Path".to_owned(), true),
            ("-".to_owned(), false),
            ("New Project Group".to_owned(), false),
            ("Move to Group".to_owned(), false),
            ("-".to_owned(), false),
            ("Set Custom Project Icon\u{2026}".to_owned(), false),
            ("-".to_owned(), false),
            ("Remove from Recent Projects\u{2026}".to_owned(), true),
        ],
        "{w}"
    );
    // The list's shortcuts: Copy Paths and `$Delete`.
    assert!(items[2]["shortcut"].is_string(), "{w}");
    assert!(items[9]["shortcut"].is_string(), "{w}");
    // Copy Path copies the folder of the row.
    for _ in 0..3 {
        h.invoke("menu.down", json!(null));
    }
    assert_eq!(h.welcome()["menu"]["levels"][0]["selected"], json!(2));
    h.invoke("menu.confirm", json!(null));
    assert_eq!(h.welcome()["menu"], json!(null));
    let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some(folders[1].to_str().unwrap()));
}
