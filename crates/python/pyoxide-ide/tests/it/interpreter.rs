//! The interpreter widget of the status bar and its popup, headless
//! (GPUI's test platform), driven through commands, keystrokes and clicks.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, Modifiers, Point, TestAppContext, VisualTestContext, WindowHandle};
use serde_json::{Value, json};
use workbench::Workbench;

/// Opens `root` as the project in a new main window, and `file` in it.
fn open(cx: &mut TestAppContext, root: &Path, file: Option<&str>) -> WindowHandle<Workbench> {
    let product = Rc::new(pyoxide_ide::product_with_tools(Default::default()));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    cx.run_until_parked();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    if let Some(file) = file {
        invoke(cx, window, "project.open-file", json!({"path": file}));
    }
    window
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx, window);
    out
}

fn state(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    invoke(cx, window, "python.interpreter.state", Value::Null)
}

fn bounds(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    draw(cx, window);
    let name: &'static str = Box::leak(selector.to_owned().into_boxed_str());
    VisualTestContext::from_window(window.into(), cx).debug_bounds(name)
}

fn centre(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    selector: &str,
) -> Point<gpui::Pixels> {
    bounds(cx, window, selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Workbench>, selector: &str) {
    let at = centre(cx, window, selector);
    let vcx = &mut VisualTestContext::from_window(window.into(), cx);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_click(at, Modifiers::none());
    draw(cx, window);
}

fn press(cx: &mut TestAppContext, window: WindowHandle<Workbench>, keys: &str) {
    cx.simulate_keystrokes(window.into(), keys);
    draw(cx, window);
}

fn venv(dir: &Path, cfg: &str) -> PathBuf {
    let python = if cfg!(windows) {
        dir.join("Scripts").join("python.exe")
    } else {
        dir.join("bin").join("python")
    };
    std::fs::create_dir_all(python.parent().unwrap()).unwrap();
    std::fs::write(&python, "").unwrap();
    std::fs::write(dir.join("pyvenv.cfg"), cfg).unwrap();
    python
}

/// A uv project `demo` with its `.venv` and a plain `venv`.
fn uv_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap().join("demo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(root.join("main.py"), "print('hi')\n").unwrap();
    std::fs::write(root.join("README.md"), "# demo\n").unwrap();
    let uv = venv(
        &root.join(".venv"),
        "home = /opt/python/bin\nimplementation = CPython\nuv = 0.9.2\nversion_info = 3.13.1\ninclude-system-site-packages = false\nprompt = demo\n",
    );
    let plain = venv(
        &root.join("venv"),
        "home = /usr/bin\ninclude-system-site-packages = false\nversion = 3.12.13\n",
    );
    (dir, root, uv, plain)
}

/// A popup row's text: the environment's path (`~` for the home folder)
/// and its version.
fn long(root: &Path, env: &str, version: &str) -> String {
    format!("{} [{version}]", project::display_path(&root.join(env)))
}

fn labels(popup: &Value) -> Vec<String> {
    popup["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            let label = i["label"].as_str().unwrap_or("---").to_owned();
            if i["enabled"] == json!(false) {
                format!("({label})")
            } else {
                label
            }
        })
        .collect()
}

fn for_project(
    cx: &mut TestAppContext,
    root: &Path,
) -> Option<pyoxide_ide::interpreter::SelectedInterpreter> {
    let task = cx.update(|cx| pyoxide_ide::interpreter::for_project(root, cx));
    cx.run_until_parked();
    cx.executor().block_test(task)
}

#[gpui::test]
fn the_widget_shows_the_project_environment(cx: &mut TestAppContext) {
    let (_dir, root, uv, _) = uv_project();
    let window = open(cx, &root, Some("main.py"));
    let s = state(cx, window);
    assert_eq!(s["visible"], json!(true));
    assert_eq!(s["text"], json!(".venv [3.13.1]"));
    assert_eq!(
        s["tooltip"],
        json!(format!("Current Interpreter: {}", uv.display()))
    );
    assert_eq!(s["icon"], json!("python/uvStatus"));
    assert_eq!(s["chosen"], json!(false));
    assert!(bounds(cx, window, "status-interpreter").is_some());
    let selected = for_project(cx, &root).unwrap();
    assert_eq!(selected.python, uv);
    assert_eq!(selected.label, ".venv [3.13.1]");
    assert_eq!(selected.version, "3.13.1");
    assert_eq!(selected.uv_project.as_deref(), Some("demo"));
    // Nothing is written while nothing was chosen.
    assert!(!root.join(".oxide").exists());
}

#[gpui::test]
fn the_widget_follows_the_active_file(cx: &mut TestAppContext) {
    let (dir, root, _, _) = uv_project();
    // The project is open, no file is: hidden.
    let window = open(cx, &root, None);
    assert_eq!(state(cx, window)["visible"], json!(false));
    assert!(bounds(cx, window, "status-interpreter").is_none());
    // The command does nothing while it is hidden.
    invoke(cx, window, "python.interpreter.choose", Value::Null);
    assert_eq!(state(cx, window)["popup"], Value::Null);
    // A file of the project, Python or not: shown.
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    assert_eq!(state(cx, window)["text"], json!(".venv [3.13.1]"));
    invoke(cx, window, "project.open-file", json!({"path": "main.py"}));
    assert_eq!(state(cx, window)["visible"], json!(true));
    // A file outside the project: hidden.
    let outside = std::fs::canonicalize(dir.path()).unwrap().join("notes.txt");
    std::fs::write(&outside, "n\n").unwrap();
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": outside.display().to_string()}),
    );
    assert_eq!(state(cx, window)["visible"], json!(false));
    // All files closed: hidden.
    for _ in 0..3 {
        invoke(cx, window, "tabs.close", Value::Null);
    }
    assert_eq!(state(cx, window)["visible"], json!(false));
}

#[gpui::test]
fn the_popup_lists_the_project_environments_and_chooses_by_keys(cx: &mut TestAppContext) {
    let (_dir, root, uv, plain) = uv_project();
    let window = open(cx, &root, Some("main.py"));
    invoke(cx, window, "python.interpreter.choose", Value::Null);
    let popup = state(cx, window)["popup"].clone();
    assert_eq!(popup["title"], json!("Python Interpreter"));
    assert_eq!(
        labels(&popup),
        [
            long(&root, "venv", "3.12.13"),
            "---".into(),
            long(&root, ".venv", "3.13.1"),
            "---".into(),
            "Add New Interpreter".into(),
            "---".into(),
            "Interpreter Settings…".into(),
            "(Manage Packages…)".into(),
        ]
    );
    // The current interpreter is selected.
    assert_eq!(popup["levels"][0]["selected"], json!(2));
    assert_eq!(
        popup["levels"][0]["items"][0]["description"],
        json!(format!("Switch to {}", plain.display()))
    );
    assert_eq!(
        popup["levels"][0]["items"][0]["command"],
        json!("python.interpreter.select")
    );
    // It opens upward from the widget.
    let menu = bounds(cx, window, "menu-level-0").unwrap();
    let widget = bounds(cx, window, "status-interpreter").unwrap();
    assert!(menu.bottom() <= widget.top() + gpui::px(1.0));
    let title = bounds(cx, window, "menu-title").unwrap();
    assert!(title.top() >= menu.top());

    // Up wraps over the disabled rows to the venv; Enter chooses it.
    press(cx, window, "up");
    assert_eq!(
        state(cx, window)["popup"]["levels"][0]["selected"],
        json!(0)
    );
    press(cx, window, "enter");
    let s = state(cx, window);
    assert_eq!(s["popup"], Value::Null);
    assert_eq!(s["text"], json!("venv [3.12.13]"));
    assert_eq!(s["icon"], json!("python/virtualEnvStatus"));
    assert_eq!(s["chosen"], json!(true));
    let selected = for_project(cx, &root).unwrap();
    assert_eq!(selected.python, plain);
    assert_eq!(selected.uv_project, None);
    // Kept in the personal project state, ignored by git.
    let text = std::fs::read_to_string(root.join(".oxide/workspace.toml")).unwrap();
    let rel = if cfg!(windows) {
        "venv/Scripts/python.exe"
    } else {
        "venv/bin/python"
    };
    assert!(text.contains(&format!("interpreter = \"{rel}\"")), "{text}");
    assert_eq!(
        std::fs::read_to_string(root.join(".oxide/.gitignore")).unwrap(),
        "# Default ignored files\n/workspace.toml\n"
    );

    // Typing filters the popup; Enter takes the match.
    invoke(cx, window, "python.interpreter.choose", Value::Null);
    assert_eq!(
        state(cx, window)["popup"]["levels"][0]["selected"],
        json!(0)
    );
    cx.simulate_input(window.into(), "3.13");
    draw(cx, window);
    let popup = state(cx, window)["popup"].clone();
    assert_eq!(popup["filter"], json!("3.13"));
    // The disabled rows do not match; the venv may (camel humps over
    // `3.12.13`), then Down goes on to the uv environment.
    let items = popup["levels"][0]["items"].as_array().unwrap();
    assert!(items[2].get("hidden").is_none());
    for hidden in [4, 6, 7] {
        assert_eq!(items[hidden]["hidden"], json!(true), "row {hidden}");
    }
    if popup["levels"][0]["selected"] == json!(0) {
        press(cx, window, "down");
    }
    assert_eq!(
        state(cx, window)["popup"]["levels"][0]["selected"],
        json!(2)
    );
    press(cx, window, "enter");
    assert_eq!(state(cx, window)["text"], json!(".venv [3.13.1]"));
    assert_eq!(for_project(cx, &root).unwrap().python, uv);

    // Escape closes the popup without a change.
    invoke(cx, window, "python.interpreter.choose", Value::Null);
    press(cx, window, "escape");
    let s = state(cx, window);
    assert_eq!(s["popup"], Value::Null);
    assert_eq!(s["text"], json!(".venv [3.13.1]"));
}

#[gpui::test]
fn the_popup_opens_and_chooses_by_clicks(cx: &mut TestAppContext) {
    let (_dir, root, _, plain) = uv_project();
    let window = open(cx, &root, Some("main.py"));
    click(cx, window, "status-interpreter");
    let popup = state(cx, window)["popup"].clone();
    assert_eq!(popup["levels"][0]["selected"], json!(2));
    click(
        cx,
        window,
        &format!("menu-item:{}", long(&root, "venv", "3.12.13")),
    );
    let s = state(cx, window);
    assert_eq!(s["popup"], Value::Null);
    assert_eq!(s["text"], json!("venv [3.12.13]"));
    assert_eq!(for_project(cx, &root).unwrap().python, plain);
    // A disabled row does nothing.
    click(cx, window, "status-interpreter");
    click(cx, window, "menu-item:Manage Packages…");
    assert!(state(cx, window)["popup"].is_object());
    // "Interpreter Settings…" opens Settings on Python > Interpreter.
    click(cx, window, "menu-item:Interpreter Settings…");
    assert_eq!(state(cx, window)["popup"], Value::Null);
    let settings = invoke(cx, window, "settings.state", Value::Null);
    assert_eq!(settings["open"], json!(true), "{settings}");
    assert_eq!(
        settings["breadcrumbs"],
        json!(["Python", "Interpreter"]),
        "{settings}"
    );
    assert_eq!(
        settings["fields"]["selected"]["path"],
        json!(plain.display().to_string()),
        "{settings}"
    );
    cx.update(|cx| {
        if let Some(handle) = settings_ui::find(cx) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    });
}

#[gpui::test]
fn the_choice_survives_reopening_the_project(cx: &mut TestAppContext) {
    let (_dir, root, _, plain) = uv_project();
    // Chosen before (the personal project state of an earlier session).
    std::fs::create_dir_all(root.join(".oxide")).unwrap();
    std::fs::write(
        root.join(".oxide/workspace.toml"),
        "version = 1\n\n[python]\ninterpreter = \"venv\"\n",
    )
    .unwrap();
    let window = open(cx, &root, Some("main.py"));
    let s = state(cx, window);
    assert_eq!(s["text"], json!("venv [3.12.13]"));
    assert_eq!(s["chosen"], json!(true));
    assert_eq!(for_project(cx, &root).unwrap().python, plain);
    // A choice that no longer exists falls back to the project's
    // environment.
    std::fs::write(
        root.join(".oxide/workspace.toml"),
        "version = 1\n\n[python]\ninterpreter = \"gone/bin/python\"\n",
    )
    .unwrap();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    invoke(cx, window, "project.open-file", json!({"path": "main.py"}));
    assert_eq!(state(cx, window)["text"], json!(".venv [3.13.1]"));
}

#[gpui::test]
fn a_project_without_an_environment(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("main.py"), "print('hi')\n").unwrap();
    let window = open(cx, &root, Some("main.py"));
    let s = state(cx, window);
    assert_eq!(s["text"], json!("<No interpreter>"));
    assert_eq!(s["tooltip"], Value::Null);
    assert_eq!(s["icon"], Value::Null);
    assert!(for_project(cx, &root).is_none());
    // The popup still opens, with nothing to choose.
    click(cx, window, "status-interpreter");
    let popup = state(cx, window)["popup"].clone();
    assert_eq!(
        labels(&popup),
        [
            "Add New Interpreter",
            "---",
            "Interpreter Settings…",
            "(Manage Packages…)",
        ]
    );
    // No interpreter to preselect: the first row that can be chosen.
    assert_eq!(popup["levels"][0]["selected"], json!(0));
    // An environment chosen by path through the command.
    let python = venv(&root.join("env"), "version = 3.11.2\n");
    press(cx, window, "escape");
    invoke(
        cx,
        window,
        "python.interpreter.select",
        json!({"path": "env"}),
    );
    assert_eq!(state(cx, window)["text"], json!("env [3.11.2]"));
    assert_eq!(for_project(cx, &root).unwrap().python, python);
}

#[gpui::test]
fn the_commands_are_registered(cx: &mut TestAppContext) {
    let product = Rc::new(pyoxide_ide::product_with_tools(Default::default()));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    draw(cx, window);
    assert_eq!(state(cx, window)["visible"], json!(false));
    let ids: Vec<&str> = cx.update(|cx| {
        ui::command::list(cx)
            .into_iter()
            .map(|c| c.meta.id)
            .collect()
    });
    for id in [
        "python.interpreter.choose",
        "python.interpreter.select",
        "python.interpreter.state",
    ] {
        assert!(ids.contains(&id), "{id}");
    }
}
