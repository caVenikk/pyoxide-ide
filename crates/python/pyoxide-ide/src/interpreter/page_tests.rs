//! Settings > Python > Interpreter and `python.interpreter.add`, headless
//! (GPUI's test platform), driven through command ids, with the list of
//! configured interpreters in a file of the test.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value as Json, json};
use workbench::Workbench;

use super::page::{NO_INTERPRETER, PAGE, SHOW_ALL};
use super::{configuring, list, table};

pub(super) fn venv(dir: &Path, cfg: &str) -> PathBuf {
    let python = py_env::paths::venv_python(dir);
    std::fs::create_dir_all(python.parent().unwrap()).unwrap();
    std::fs::write(&python, "").unwrap();
    std::fs::write(dir.join("pyvenv.cfg"), cfg).unwrap();
    python
}

pub(super) struct Fixture {
    _dir: tempfile::TempDir,
    #[cfg(unix)]
    pub base: PathBuf,
    pub root: PathBuf,
    pub uv: PathBuf,
    pub plain: PathBuf,
    pub file: PathBuf,
    pub window: WindowHandle<Workbench>,
}

/// A uv project `demo` (`.venv` by uv, a plain `venv`), and a list of
/// configured interpreters: an installation for every project, an
/// environment of another project and one of `demo` kept outside it.
pub(super) fn fixture(cx: &mut TestAppContext) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let root = base.join("demo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(root.join("main.py"), "print('hi')\n").unwrap();
    let uv = venv(&root.join(".venv"), "uv = 0.9.2\nversion_info = 3.13.1\n");
    let plain = venv(&root.join("venv"), "version = 3.12.13\n");
    let system = base.join("python/bin/python3.14");
    std::fs::create_dir_all(system.parent().unwrap()).unwrap();
    std::fs::write(&system, "").unwrap();
    let other = venv(&base.join("other/.venv"), "version = 3.11.1\n");
    let mine = venv(&base.join("envs/mine"), "version = 3.10.2\n");
    let file = base.join("config").join(table::FILE);
    let entry = |name: &str, python: &Path, kind: &str, version: &str, project: Option<&Path>| {
        let project = project
            .map(|p| format!("associated-project = {:?}\n", p.display().to_string()))
            .unwrap_or_default();
        format!(
            "[[interpreter]]\nname = {name:?}\npython = {:?}\nkind = \"{kind}\"\nversion = \"{version}\"\n{project}\n",
            python.display().to_string()
        )
    };
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        [
            "version = 1\n\n".to_owned(),
            entry("Python 3.14", &system, "system", "3.14.0", None),
            entry(
                "other env",
                &other,
                "venv",
                "3.11.1",
                Some(&base.join("other")),
            ),
            entry("mine", &mine, "venv", "3.10.2", Some(&root)),
        ]
        .concat(),
    )
    .unwrap();

    cx.update(|cx| list::set_file(cx, file.clone(), Vec::new()));
    let product = Rc::new(crate::product_with_tools(Default::default()));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    cx.run_until_parked();
    let f = Fixture {
        _dir: dir,
        #[cfg(unix)]
        base,
        root,
        uv,
        plain,
        file,
        window,
    };
    invoke(
        cx,
        &f,
        "project.open-folder",
        json!({"path": f.root.display().to_string()}),
    );
    invoke(cx, &f, "project.open-file", json!({"path": "main.py"}));
    f
}

pub(super) fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
    for handle in cx.update(|cx| cx.windows()) {
        let _ = cx.update_window(handle, |_, window, cx| window.draw(cx).clear());
    }
    cx.run_until_parked();
}

pub(super) fn invoke(cx: &mut TestAppContext, f: &Fixture, id: &str, args: Json) -> Json {
    let shown = args.to_string();
    let out = cx
        .update_window(f.window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id} {shown}: {e:?}"));
    settle(cx);
    out
}

pub(super) fn page(cx: &mut TestAppContext, f: &Fixture) -> Json {
    invoke(cx, f, "settings.state", Json::Null)["fields"].clone()
}

pub(super) fn edit(cx: &mut TestAppContext, f: &Fixture, field: &str, value: Json) {
    invoke(
        cx,
        f,
        "settings.edit",
        json!({"field": field, "value": value}),
    );
}

/// The combo's rows: names, `-` for separators.
fn rows(page: &Json) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| match i["kind"].as_str().unwrap() {
            "separator" => "-".to_owned(),
            "interpreter" => i["name"].as_str().unwrap().to_owned(),
            _ => i["text"].as_str().unwrap().to_owned(),
        })
        .collect()
}

/// Waits for work on the interpreter runtime (real threads).
#[cfg(unix)]
pub(super) fn wait_for(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    for _ in 0..500 {
        settle(cx);
        if done(cx) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

pub(super) fn workspace(root: &Path) -> String {
    std::fs::read_to_string(project::workspace::workspace_file(root)).unwrap_or_default()
}

#[gpui::test]
fn the_page_lists_the_interpreters_and_applies_a_choice(cx: &mut TestAppContext) {
    let f = fixture(cx);
    invoke(cx, &f, "settings.open", json!({"page": PAGE}));
    let s = invoke(cx, &f, "settings.state", Json::Null);
    assert_eq!(s["breadcrumbs"], json!(["Python", "Interpreter"]), "{s}");
    // Interpreter is the first page of the Python group.
    assert_eq!(s["rows"][1]["id"], json!(PAGE), "{s}");
    let p = s["fields"].clone();
    assert_eq!(p["loaded"], json!(true), "{p}");
    assert_eq!(p["label"], json!("Python Interpreter:"));
    let name = |dir: &str| project::display_path(&f.root.join(dir));
    // "<No interpreter>", the virtual environments, a separator, the
    // others (uv's environment among them), a separator, "Show All…"; the
    // other project's environment is not offered.
    assert_eq!(
        rows(&p),
        [
            NO_INTERPRETER.to_owned(),
            name("venv"),
            "mine".into(),
            "-".into(),
            name(".venv"),
            "Python 3.14".into(),
            "-".into(),
            SHOW_ALL.into(),
        ],
        "{p}"
    );
    let uv_row = &p["items"][4];
    assert_eq!(uv_row["version"], json!("3.13.1"));
    assert_eq!(uv_row["path"], json!(f.uv.display().to_string()));
    assert_eq!(uv_row["icon"], json!("python/uv"));
    assert_eq!(uv_row["group"], json!("system"));
    // The project's environment is selected; nothing is modified.
    assert_eq!(p["selected"]["path"], json!(f.uv.display().to_string()));
    assert_eq!(uv_row["selected"], json!(true));
    assert_eq!(p["combo-enabled"], json!(true));
    assert_eq!(p["link-enabled"], json!(true));
    assert_eq!(
        p["columns"],
        json!(["Package", "Version", "Latest version"])
    );
    assert_eq!(p["empty-text"], json!("Nothing to show"));
    let toolbar: Vec<_> = p["toolbar"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["tooltip"].clone())
        .collect();
    assert_eq!(
        toolbar,
        [
            json!("Install"),
            json!("Uninstall"),
            json!("Upgrade"),
            json!("Show Early Releases")
        ]
    );

    // The combo's list and the "Add Interpreter" popup.
    edit(cx, &f, "list", json!("interpreters"));
    let p = page(cx, &f);
    assert_eq!(p["list"], json!("interpreters"));
    let menu = &p["menu"]["levels"][0];
    assert_eq!(menu["selected"], json!(4), "{p}");
    assert_eq!(menu["items"][0]["label"], json!(NO_INTERPRETER));
    // Under the combo and as wide as it: long paths are cut.
    let settings = cx.update(|cx| settings_ui::find(cx)).unwrap();
    let vcx = &mut gpui::VisualTestContext::from_window(settings.into(), cx);
    let combo = vcx.debug_bounds("interpreter:combo").unwrap();
    let list = vcx.debug_bounds("menu-level-0").unwrap();
    assert!(
        (list.left() - combo.left()).abs() <= gpui::px(1.0),
        "{list:?} {combo:?}"
    );
    assert!(
        list.top() >= combo.bottom() - gpui::px(1.0),
        "{list:?} {combo:?}"
    );
    assert!(
        list.size.width <= combo.size.width + gpui::px(1.0),
        "{list:?} {combo:?}"
    );
    edit(cx, &f, "list", json!("add"));
    let p = page(cx, &f);
    assert_eq!(p["list"], json!("add"));
    assert_eq!(
        p["menu"]["levels"][0]["items"][0]["label"],
        json!("Add Local Interpreter…")
    );
    edit(cx, &f, "list", Json::Null);

    // Choosing the plain environment: a draft until Apply.
    edit(cx, &f, "interpreter", json!(f.plain.display().to_string()));
    let s = invoke(cx, &f, "settings.state", Json::Null);
    assert_eq!(s["modified"], json!([PAGE]), "{s}");
    assert!(!workspace(&f.root).contains("interpreter"));
    invoke(cx, &f, "settings.apply", Json::Null);
    let rel = if cfg!(windows) {
        "venv/Scripts/python.exe"
    } else {
        "venv/bin/python"
    };
    assert!(
        workspace(&f.root).contains(&format!("interpreter = \"{rel}\"")),
        "{}",
        workspace(&f.root)
    );
    let s = invoke(cx, &f, "settings.state", Json::Null);
    assert_eq!(s["modified"], json!([]), "{s}");
    let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
    assert_eq!(w["text"], json!("venv [3.12.13]"), "{w}");
    // The project's environment joined the list, associated with it.
    let entry = w["list"]["interpreters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["python"] == json!(f.plain.display().to_string()))
        .cloned()
        .unwrap_or_else(|| panic!("{w}"));
    assert_eq!(entry["name"], json!(name("venv")));
    assert_eq!(entry["kind"], json!("venv"));
    assert_eq!(
        entry["associated-project"],
        json!(f.root.display().to_string())
    );
    let text = std::fs::read_to_string(&f.file).unwrap();
    assert!(
        text.contains(&format!("name = {:?}", name("venv"))),
        "{text}"
    );
    assert!(text.contains("name = \"other env\""), "{text}");

    // "<No interpreter>" is a choice of its own and survives.
    edit(cx, &f, "interpreter", Json::Null);
    invoke(cx, &f, "settings.apply", Json::Null);
    assert!(
        workspace(&f.root).contains("interpreter = \"none\""),
        "{}",
        workspace(&f.root)
    );
    cx.update(|cx| cx.default_global::<super::Chosen>().0.clear());
    invoke(cx, &f, "settings.revert", Json::Null);
    let p = page(cx, &f);
    assert_eq!(p["selected"]["kind"], json!("none"), "{p}");
    assert_eq!(p["items"][0]["selected"], json!(true));
    let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
    assert_eq!(w["text"], json!(NO_INTERPRETER), "{w}");
    let task = cx.update(|cx| super::for_project(&f.root, cx));
    cx.run_until_parked();
    assert!(cx.executor().block_test(task).is_none());
}

#[gpui::test]
fn the_page_is_disabled_while_an_interpreter_is_configured(cx: &mut TestAppContext) {
    let f = fixture(cx);
    invoke(cx, &f, "settings.open", json!({"page": PAGE}));
    cx.update(|cx| assert!(configuring::begin(&f.root, cx)));
    settle(cx);
    let p = page(cx, &f);
    assert_eq!(p["configuring"], json!(true));
    assert_eq!(p["combo-enabled"], json!(false));
    assert_eq!(p["link-enabled"], json!(false));
    assert_eq!(p["tooltip"], json!("An interpreter is being configured."));
    edit(cx, &f, "list", json!("interpreters"));
    assert_eq!(page(cx, &f)["list"], Json::Null);
    // The status bar's "Add New Interpreter" is greyed meanwhile.
    invoke(cx, &f, "python.interpreter.choose", Json::Null);
    let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
    assert_eq!(w["configuring"], json!(true));
    let add = w["popup"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == json!("Add New Interpreter"))
        .cloned()
        .unwrap();
    assert_eq!(add["enabled"], json!(false), "{add}");
    cx.update(|cx| configuring::end(&f.root, cx));
    settle(cx);
    let p = page(cx, &f);
    assert_eq!(p["combo-enabled"], json!(true));
    assert_eq!(p["tooltip"], Json::Null);
}

/// A fake interpreter and a fake uv: shell scripts answering the probe
/// and `uv --version`.
#[cfg(unix)]
pub(super) fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[gpui::test]
fn existing_interpreters_are_added_and_selected(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let python = f.base.join("tools/bin/python3.12");
    script(
        &python,
        &format!(
            "echo '{{\"version\": [3, 12, 7, \"final\", 0], \"implementation\": \"cpython\", \"prefix\": \"{0}\", \"base_prefix\": \"{0}\"}}'",
            f.base.join("tools").display()
        ),
    );
    // From the status bar's way: the project's interpreter at once.
    invoke(
        cx,
        &f,
        "python.interpreter.add",
        json!({"kind": "existing-python", "python": python.display().to_string()}),
    );
    let name = project::display_path(&python);
    wait_for(cx, "the interpreter to be added", |cx| {
        let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
        w["selected"]["python"] == json!(python.display().to_string())
            && w["configuring"] == json!(false)
    });
    let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
    assert_eq!(w["selected"]["name"], json!(name), "{w}");
    assert_eq!(w["selected"]["version"], json!("3.12.7"), "{w}");
    let entry = w["list"]["interpreters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == json!(name))
        .cloned()
        .unwrap();
    assert_eq!(entry["kind"], json!("system"));
    // An installation is not associated with the project.
    assert_eq!(entry["associated-project"], Json::Null);
    assert!(workspace(&f.root).contains(&python.display().to_string()));
    // Added again: reused, not listed twice.
    invoke(
        cx,
        &f,
        "python.interpreter.add",
        json!({"kind": "existing-python", "python": python.display().to_string()}),
    );
    wait_for(cx, "the second add", |cx| {
        !cx.update(|cx| configuring::is_configuring(&f.root, cx))
    });
    let count = cx.update(|cx| {
        list::table(cx)
            .entries
            .iter()
            .filter(|e| e.python == python)
            .count()
    });
    assert_eq!(count, 1);

    // With the Interpreter page open: a uv environment, selected there
    // until Apply.
    let uv = f.base.join("bin/uv");
    script(&uv, "echo 'uv 0.9.2 (test)'");
    let env = f.base.join("envs/uvenv");
    let env_python = venv(&env, "uv = 0.9.2\nversion_info = 3.13.2\n");
    invoke(cx, &f, "settings.open", json!({"page": PAGE}));
    invoke(
        cx,
        &f,
        "python.interpreter.add",
        json!({"kind": "existing-uv", "python": env.display().to_string(), "uv_path": uv.display().to_string()}),
    );
    wait_for(cx, "the uv environment", |cx| {
        page(cx, &f)["selected"]["path"] == json!(env_python.display().to_string())
    });
    let s = invoke(cx, &f, "settings.state", Json::Null);
    assert_eq!(s["modified"], json!([PAGE]), "{s}");
    let entry = cx.update(|cx| list::table(cx).find_python(&env_python).cloned().unwrap());
    assert_eq!(entry.flavor, table::Flavor::Uv);
    assert_eq!(entry.uv.as_deref(), Some(uv.as_path()));
    assert_eq!(entry.working_dir.as_deref(), Some(f.root.as_path()));
    assert_eq!(entry.associated_project.as_deref(), Some(f.root.as_path()));
    assert_eq!(entry.version.as_deref(), Some("3.13.2"));
    invoke(cx, &f, "settings.ok", Json::Null);
    assert!(workspace(&f.root).contains(&env_python.display().to_string()));

    // A uv that does not run: nothing is added, the balloon says why.
    invoke(
        cx,
        &f,
        "python.interpreter.add",
        json!({"kind": "existing-uv", "python": env.display().to_string(), "uv_path": f.base.join("bin/missing").display().to_string(), "select": "now"}),
    );
    wait_for(cx, "the failure", |cx| {
        !cx.update(|cx| configuring::is_configuring(&f.root, cx))
    });
    let balloons = invoke(cx, &f, "workbench.state", Json::Null)["notifications"].clone();
    assert!(
        balloons.to_string().contains("Cannot find uv executable"),
        "{balloons}"
    );
}
