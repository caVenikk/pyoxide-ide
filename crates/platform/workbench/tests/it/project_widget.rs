//! The project widget's popup and the recent projects (#272): a project
//! that opens goes first in the recent projects (kept in the
//! configuration folder); the popup shows Open…, Clone Repository…, the
//! project of the window under "Open Projects" and the others under
//! "Recent Projects", each with its avatar and its folder; choosing a
//! recent project opens it in the window.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
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
