//! The main menu (#286): the menu bar built from the registry's commands
//! in the reference's order, with the shortcuts of the keymap in use and
//! the rows enabled where their commands are available; Close Project.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, Focusable as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::main_menu::{self, MenuSpec};
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

    /// A project folder of the test's own with a file in it.
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join("projects").join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.py"), "print('hi')\n").unwrap();
        native_path::canonicalize(&path).unwrap()
    }

    fn open(&mut self, path: &Path) {
        self.invoke(
            "project.open-folder",
            json!({ "path": path.to_string_lossy() }),
        );
    }

    /// The macOS menu bar of the window as its active window, as text,
    /// enabled as in the window now.
    fn describe(&mut self) -> String {
        let window = self.window;
        let spec = self.cx.update(|cx| MenuSpec {
            macos: true,
            ..main_menu::spec_for(Some(window), cx)
        });
        let menus = main_menu::model(&spec);
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                main_menu::describe(&menus, window, cx)
            })
            .unwrap()
    }
}

fn open(cx: &mut TestAppContext) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config").join(recent_projects::FILE);
    let product = Product {
        name: "Pyoxide",
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        // The menu of macOS shows the macOS keymap on every platform here.
        workbench::keymap::active::activate(workbench::keymap::MACOS_NAME, cx).unwrap();
        recent_projects::load(cx, file);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H { cx, window, dir };
    h.draw();
    h
}

/// The menu bar with a project and a file open in the editor, which has
/// the focus: the reference's groups (`MainMenu` of `PlatformActions.xml`
/// and what joins it) with the rows that have a command, the shortcuts of
/// the macOS keymap as the native menu shows them (single keystrokes) and
/// the rows not available there marked.
#[gpui::test]
fn the_menu_bar_follows_the_reference_groups(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let other = h.folder("other");
    let demo = h.folder("demo");
    h.open(&other);
    h.open(&demo);
    h.invoke("project.open-file", json!({ "path": "main.py" }));
    h.invoke("workbench.focus-editor", json!(null));
    let text = h.describe();
    println!("{text}");
    // A Windows checkout may turn the file's line ends into CRLF.
    let expected = include_str!("main_menu_macos.txt").replace("\r\n", "\n");
    assert_eq!(text, expected);
}

/// The control channel's `main-menu.state` describes the window's menu.
#[gpui::test]
fn the_state_query_describes_the_windows_menu(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let demo = h.folder("demo");
    h.open(&demo);
    let state = h.invoke("main-menu.state", json!(null));
    let text = state["text"].as_str().unwrap();
    assert!(
        text.starts_with(if cfg!(target_os = "macos") {
            "Pyoxide\n"
        } else {
            "File\n"
        }),
        "{text}"
    );
    assert!(text.contains("\n  Close Project\n"), "{text}");
    assert!(!text.contains("    demo\n"), "{text}");
}

/// Every row runs a command of the registry, on macOS and elsewhere.
#[gpui::test]
fn every_row_has_a_registered_command(cx: &mut TestAppContext) {
    let h = open(cx);
    for macos in [true, false] {
        let spec = MenuSpec {
            product: "Pyoxide".into(),
            macos,
            recent: vec![("demo".into(), PathBuf::from("/p/demo"))],
        };
        let menus = main_menu::model(&spec);
        h.cx.update(|cx| {
            let registered: Vec<&str> = ui::command::list(cx).iter().map(|c| c.meta.id).collect();
            for command in main_menu::commands(&menus) {
                assert!(registered.contains(&command), "{command} is not registered");
                assert!(
                    main_menu::build(command, &None, cx).is_some()
                        || command == "project.open-recent",
                    "{command} builds without arguments"
                );
            }
        });
    }
}

/// Recent Projects lists the recent projects without the window's own.
#[gpui::test]
fn recent_projects_leave_out_the_windows_project(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let alpha = h.folder("alpha");
    let beta = h.folder("beta");
    h.open(&alpha);
    h.open(&beta);
    // The test platform has no active window: the spec for this one.
    let window = h.window;
    let spec = h.cx.update(|cx| main_menu::spec_for(Some(window), cx));
    let names: Vec<&str> = spec.recent.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["alpha"]);
    assert_eq!(spec.recent[0].1, alpha);
}

/// Close Project: the last main window gives way to one without a project
/// that shows the Welcome tab; without a project there is nothing to
/// close.
#[gpui::test]
fn close_project_shows_the_welcome_tab_in_a_window_without_a_project(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let demo = h.folder("demo");
    h.open(&demo);
    h.invoke("project.open-file", json!({ "path": "main.py" }));
    h.invoke("project.close", json!(null));
    h.cx.run_until_parked();
    let windows: Vec<WindowHandle<Workbench>> = h.cx.update(|cx| {
        cx.windows()
            .into_iter()
            .filter_map(|w| w.downcast::<Workbench>())
            .collect()
    });
    assert_eq!(windows.len(), 1, "one main window");
    assert_ne!(windows[0], h.window, "a new window");
    h.window = windows[0];
    h.draw();
    let s = h.invoke("workbench.state", json!(null));
    assert!(s["project"].is_null(), "{s}");
    let tabs = s["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 1, "{s}");
    assert_eq!(tabs[0]["view"]["kind"], json!("welcome"), "{s}");
    // The project stays a recent project, listed in the menu now.
    let spec = h.cx.update(|cx| main_menu::current_spec(cx));
    let names: Vec<&str> = spec.recent.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["demo"]);
    // Nothing to close here: the command is not available.
    let available =
        h.cx.update_window(h.window.into(), |_, window, cx| {
            let focus = window
                .root::<Workbench>()
                .flatten()
                .unwrap()
                .focus_handle(cx);
            window.focus(&focus);
            ui::command::invoke("project.close", json!(null), window, cx)
        })
        .unwrap();
    assert!(available.is_err(), "{available:?}");
}
