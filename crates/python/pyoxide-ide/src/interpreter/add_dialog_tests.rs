//! The Add Python Interpreter dialog, headless (GPUI's test platform),
//! driven through command ids, on a machine of the test: fake
//! interpreters and a fake uv (shell scripts) in a folder of the test.

use std::path::{Path, PathBuf};

use gpui::TestAppContext;
use serde_json::{Value as Json, json};

use super::add_dialog::MachineOverride;
use super::candidates::Machine;
use super::page::PAGE;
use super::page_tests::{Fixture, fixture, invoke, page, script, settle, wait_for};
use super::{configuring, list, table};

/// The fake machine: `python3.12` and `python3.7` on its `PATH` (the
/// first makes environments with `-m venv`), a uv that makes
/// environments, lists Python 3.14 and 3.13 (none installed), can
/// download CPython 3.14.0, 3.13.5 and 3.12.7, installs 3.13.5 (which
/// makes environments too) and logs its arguments.
struct FakeMachine {
    bin: PathBuf,
    python312: PathBuf,
    python37: PathBuf,
    uv: PathBuf,
    log: PathBuf,
    /// Where `uv python install 3.13.5` puts that Python.
    installed313: PathBuf,
}

/// A probe answer: `prefix` is the folder above the script's own folder,
/// so a copy in an environment describes that environment.
fn env_python(path: &Path, version: (u32, u32, u32), base_prefix: &Path) {
    let (a, b, c) = version;
    script(
        path,
        &format!(
            "d=$(cd \"$(dirname \"$0\")/..\" && pwd)\necho \"{{\\\"version\\\": [{a}, {b}, {c}, \\\"final\\\", 0], \\\"implementation\\\": \\\"cpython\\\", \\\"prefix\\\": \\\"$d\\\", \\\"base_prefix\\\": \\\"{}\\\"}}\"",
            base_prefix.display()
        ),
    );
}

/// An installation: answers the probe, makes an environment with
/// `-m venv <path>` (its interpreter a copy of `template`).
fn base_python(path: &Path, version: (u32, u32, u32), template: &Path) {
    let (a, b, c) = version;
    let prefix = path.parent().unwrap().parent().unwrap();
    script(
        path,
        &format!(
            "if [ \"$1\" = \"-m\" ] && [ \"$2\" = \"venv\" ]; then\n  for last; do :; done\n  mkdir -p \"$last/bin\"\n  printf 'home = {bin}\\nversion = {a}.{b}.{c}\\n' > \"$last/pyvenv.cfg\"\n  cp \"{template}\" \"$last/bin/python\"\n  exit 0\nfi\necho '{{\"version\": [{a}, {b}, {c}, \"final\", 0], \"implementation\": \"cpython\", \"prefix\": \"{prefix}\", \"base_prefix\": \"{prefix}\"}}'",
            bin = path.parent().unwrap().display(),
            template = template.display(),
            prefix = prefix.display(),
        ),
    );
}

fn fake_machine(f: &Fixture) -> FakeMachine {
    let bin = f.base.join("machine/bin");
    let templates = f.base.join("machine/templates");
    let template312 = templates.join("python312/bin/python");
    let template313 = templates.join("python313/bin/python");
    env_python(&template312, (3, 12, 7), &f.base.join("machine"));
    env_python(&template313, (3, 13, 5), &f.base.join("uv-python"));
    let python312 = bin.join("python3.12");
    base_python(&python312, (3, 12, 7), &template312);
    let python37 = bin.join("python3.7");
    base_python(&python37, (3, 7, 9), &template312);
    // The CPython 3.13.5 uv installs: makes environments with `-m venv`.
    let download313 = templates.join("download313/bin/python3.13");
    base_python(&download313, (3, 13, 5), &template313);
    let installed313 = f.base.join("uv-python/cpython-3.13.5/bin/python3.13");
    let uv = f.base.join("machine/tools/uv");
    let log = f.base.join("machine/uv.log");
    let list = r#"[{"key": "cpython-3.14.0-macos-aarch64-none", "version": "3.14.0", "path": null}, {"key": "cpython-3.13.5-macos-aarch64-none", "version": "3.13.5", "path": null}, {"key": "cpython-3.13.1-macos-aarch64-none", "version": "3.13.1", "path": null}]"#;
    let downloads = r#"[{"key": "cpython-3.14.0-macos-aarch64-none", "version": "3.14.0", "path": null, "implementation": "cpython", "variant": "default"}, {"key": "cpython-3.14.0+freethreaded-macos-aarch64-none", "version": "3.14.0", "path": null, "implementation": "cpython", "variant": "freethreaded"}, {"key": "cpython-3.13.5-macos-aarch64-none", "version": "3.13.5", "path": null, "implementation": "cpython", "variant": "default"}, {"key": "cpython-3.13.1-macos-aarch64-none", "version": "3.13.1", "path": null, "implementation": "cpython", "variant": "default"}, {"key": "cpython-3.12.7-macos-aarch64-none", "version": "3.12.7", "path": null, "implementation": "cpython", "variant": "default"}, {"key": "pypy-3.11.13-macos-aarch64-none", "version": "3.11.13", "path": null, "implementation": "pypy", "variant": "default"}]"#;
    let python = format!(
        "    case \"$2\" in\n      list)\n        case \" $* \" in\n          *' --only-downloads '*) echo '{downloads}' ;;\n          *) echo '{list}' ;;\n        esac ;;\n      dir) echo \"{dir}\" ;;\n      install)\n        mkdir -p \"{installed_dir}\"\n        cp \"{download}\" \"{installed}\" ;;\n      find)\n        if [ -f \"{installed}\" ]; then echo \"{installed}\"; else echo 'error: No interpreter found' >&2; exit 2; fi ;;\n    esac ;;\n",
        dir = f.base.join("uv-python").display(),
        installed_dir = installed313.parent().unwrap().display(),
        installed = installed313.display(),
        download = download313.display(),
    );
    script(
        &uv,
        &format!(
            "echo \"$@\" >> \"{log}\"\ncase \"$1\" in\n  --version) echo 'uv 0.9.2 (test)' ;;\n  python)\n{python}  venv)\n    for last; do :; done\n    mkdir -p \"$last/bin\"\n    printf 'uv = 0.9.2\\nversion_info = 3.13.5\\n' > \"$last/pyvenv.cfg\"\n    cp \"{template}\" \"$last/bin/python\" ;;\nesac",
            log = log.display(),
            template = template313.display(),
        ),
    );
    FakeMachine {
        bin,
        python312,
        python37,
        uv,
        log,
        installed313,
    }
}

/// The dialog looks at the fake machine (with or without its uv).
fn use_machine(cx: &mut TestAppContext, f: &Fixture, m: &FakeMachine, with_uv: bool) {
    let machine = Machine {
        search: py_env::SearchEnv {
            path_dirs: vec![m.bin.clone()],
            home: Some(f.base.join("home")),
            ..Default::default()
        },
        tools: py_env::Tools {
            uv: with_uv.then(|| m.uv.clone()),
            ..Default::default()
        },
        venv_root: None,
    };
    cx.update(|cx| cx.set_global(MachineOverride(machine)));
}

fn dialog(cx: &mut TestAppContext, f: &Fixture) -> Json {
    invoke(cx, f, "python.interpreter.state", Json::Null)["add-dialog"].clone()
}

fn set(cx: &mut TestAppContext, f: &Fixture, field: &str, value: &str) {
    invoke(
        cx,
        f,
        "python.interpreter.add-dialog.set",
        json!({"field": field, "value": value}),
    );
}

/// Waits until nothing of the dialog is looked up any more.
fn wait_checked(cx: &mut TestAppContext, f: &Fixture) -> Json {
    wait_for(cx, "the dialog's checks", |cx| {
        let d = dialog(cx, f);
        !d.is_null()
            && !d.to_string().contains("\"validating\"")
            && !d.to_string().contains("\"loading\":true")
    });
    dialog(cx, f)
}

fn texts(list: &Json) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap().to_owned())
        .collect()
}

fn path(p: &Path) -> String {
    p.display().to_string()
}

#[gpui::test]
fn every_field_is_validated_as_the_reference_does(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let m = fake_machine(&f);
    use_machine(cx, &f, &m, true);
    invoke(cx, &f, "settings.open", json!({"page": PAGE}));
    invoke(cx, &f, "python.interpreter.add-local", Json::Null);
    let d = wait_checked(cx, &f);

    // Generate new / Virtualenv (S7_2): the newest installation as the
    // base, the project's `.venv` as the location, which holds Python.
    // After the installations, the Pythons uv can download of the
    // versions none of them has (I3_1).
    assert_eq!(d["title"], json!("Add Python Interpreter"), "{d}");
    assert_eq!(d["environment"], json!("generate-new"));
    assert_eq!(d["type"], json!("virtualenv"));
    assert_eq!(d["types"], json!(["Virtualenv", "uv"]));
    assert_eq!(
        texts(&d["base-python"]["items"]),
        [
            path(&m.python312),
            path(&m.python37),
            "download:3.14.0".to_owned(),
            "download:3.13.5".to_owned()
        ]
    );
    assert_eq!(d["base-python"]["selected"]["text"], json!("Python 3.12"));
    assert_eq!(
        d["base-python"]["selected"]["detail"],
        json!(format!(" ({}) system", project::display_path(&m.python312)))
    );
    assert_eq!(d["location"]["text"], json!(path(&f.root.join(".venv"))));
    assert_eq!(
        d["location"]["alert"],
        json!({"kind": "error", "text": "Already contains Python with version 3.13", "links": ["Select existing interpreter"]})
    );
    assert_eq!(
        d["rows"],
        json!([
            "Environment:",
            "Type:",
            "Base Python:",
            "Location:",
            "alert",
            "Inherit packages from base interpreter",
            "Make available to all projects"
        ])
    );
    assert_eq!(d["ok-enabled"], json!(false));

    // A free location: OK is enabled. A field filled in takes its text as
    // a text field does: the caret at the end, nothing selected.
    set(cx, &f, "location", &path(&f.base.join("envs/new")));
    let d = wait_checked(cx, &f);
    let end = path(&f.base.join("envs/new")).chars().count();
    assert_eq!(d["location"]["selection"], json!([end, end]), "{d}");
    assert_eq!(d["location"]["error"], Json::Null, "{d}");
    assert_eq!(d["location"]["alert"], Json::Null);
    assert_eq!(d["ok-enabled"], json!(true));
    // A folder without Python: a field error, no alert.
    std::fs::create_dir_all(f.base.join("envs/folder")).unwrap();
    std::fs::write(f.base.join("envs/folder/notes.txt"), "").unwrap();
    set(cx, &f, "location", &path(&f.base.join("envs/folder")));
    let d = wait_checked(cx, &f);
    assert_eq!(
        d["location"]["error"],
        json!("The specified folder already exists")
    );
    assert_eq!(d["location"]["alert"], Json::Null);
    assert!(!d["rows"].to_string().contains("alert"), "{d}");
    assert_eq!(d["ok-enabled"], json!(false));
    // A base older than 3.8: the error under it, OK disabled.
    set(cx, &f, "location", &path(&f.base.join("envs/new")));
    set(cx, &f, "base-python", &path(&m.python37));
    let d = wait_checked(cx, &f);
    assert_eq!(d["base-python"]["selected"]["text"], json!("Python 3.7 "));
    assert_eq!(
        d["base-python"]["notice"]["text"],
        json!(
            "New virtual environments require Python 3.8+.\nFor older versions, select an existing environment from disk."
        )
    );
    assert_eq!(d["base-python"]["notice"]["kind"], json!("error"));
    assert_eq!(d["rows"][3], json!("notice"), "{d}");
    assert_eq!(d["ok-enabled"], json!(false));
    set(cx, &f, "base-python", &path(&m.python312));
    set(cx, &f, "inherit-site-packages", "true");
    set(cx, &f, "available-to-all", "true");
    let d = wait_checked(cx, &f);
    assert_eq!(d["inherit-site-packages"], json!(true));
    assert_eq!(d["available-to-all"], json!(true));
    assert_eq!(d["ok-enabled"], json!(true));

    // The Type list (S7_4, without the managers not offered).
    invoke(
        cx,
        &f,
        "python.interpreter.add-dialog.open-list",
        json!({"list": "type"}),
    );
    let d = dialog(cx, &f);
    assert_eq!(d["list"], json!("type"));
    let labels: Vec<&str> = d["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Virtualenv", "uv"]);
    // The list is a popup over the dialog: commands go to it now.
    invoke(cx, &f, "menu.cancel", Json::Null);
    assert_eq!(dialog(cx, &f)["list"], Json::Null);
    assert_eq!(dialog(cx, &f)["type"], json!("virtualenv"));

    // "Select existing interpreter" under the `.venv` location: Select
    // existing / Python with that interpreter (S7_3).
    set(cx, &f, "location", &path(&f.root.join(".venv")));
    wait_checked(cx, &f);
    invoke(
        cx,
        &f,
        "python.interpreter.add-dialog.select-existing",
        Json::Null,
    );
    let d = wait_checked(cx, &f);
    assert_eq!(d["environment"], json!("select-existing"), "{d}");
    assert_eq!(d["type"], json!("python"));
    assert_eq!(d["rows"], json!(["Environment:", "Type:", "Python path:"]));
    assert_eq!(
        d["python"]["selected"]["path"],
        json!(path(&py_env::paths::venv_python(&f.root.join(".venv"))))
    );
    assert_eq!(d["python"]["selected"]["text"], json!("Python 3.13"));
    // The project's configured one first, the shared one, the others;
    // the other project's environment is not offered.
    let items = texts(&d["python"]["items"]);
    assert_eq!(
        items[..2],
        [
            path(&py_env::paths::venv_python(&f.base.join("envs/mine"))),
            path(&f.base.join("python/bin/python3.14")),
        ],
        "{items:?}"
    );
    assert!(!items.iter().any(|i| i.contains("other")), "{items:?}");
    assert_eq!(d["ok-enabled"], json!(true));
    // An old interpreter: a warning only.
    set(cx, &f, "python", &path(&m.python37));
    let d = wait_checked(cx, &f);
    assert_eq!(d["python"]["notice"]["kind"], json!("warning"), "{d}");
    assert_eq!(d["ok-enabled"], json!(true));

    // Select existing / uv (S7_5): uv found and checked, the project's uv
    // environment.
    set(cx, &f, "type", "uv");
    let d = wait_checked(cx, &f);
    assert_eq!(
        d["rows"],
        json!(["Environment:", "Type:", "Path to uv:", "Environment:"])
    );
    assert_eq!(d["uv-path"]["text"], json!(path(&m.uv)));
    assert_eq!(d["uv-path"]["found"], json!(true));
    assert_eq!(d["uv-path"]["version"], json!("0.9.2"));
    assert_eq!(texts(&d["uv-environment"]["items"]), [path(&f.uv)], "{d}");
    assert_eq!(
        d["uv-environment"]["selected"]["detail"],
        json!(format!(" ({}) uv", project::display_path(&f.uv)))
    );
    assert_eq!(d["ok-enabled"], json!(true));
    // A uv path that is wrong: the field's error, the environment row
    // hidden.
    set(cx, &f, "uv-path", &path(&f.base.join("machine/missing-uv")));
    let d = wait_checked(cx, &f);
    assert_eq!(d["uv-path"]["error"], json!("Executable does not exist"));
    assert_eq!(d["rows"], json!(["Environment:", "Type:", "Path to uv:"]));
    assert_eq!(d["ok-enabled"], json!(false));
    set(cx, &f, "uv-path", &path(&f.base.join("machine")));
    let d = wait_checked(cx, &f);
    assert_eq!(d["uv-path"]["error"], json!("Path cannot be a directory"));
    set(cx, &f, "uv-path", &path(&m.uv));
    let d = wait_checked(cx, &f);
    let end = path(&m.uv).chars().count();
    assert_eq!(d["uv-path"]["selection"], json!([end, end]), "{d}");

    // Generate new / uv: uv's Python versions, the alert of the project's
    // environment, Override.
    set(cx, &f, "environment", "generate-new");
    set(cx, &f, "type", "uv");
    let d = wait_checked(cx, &f);
    assert_eq!(
        d["rows"],
        json!([
            "Environment:",
            "Type:",
            "Python version:",
            "Path to uv:",
            "Location:",
            "alert"
        ]),
        "{d}"
    );
    assert_eq!(
        d["python-version"]["items"],
        json!(["Default", "3.14", "3.13"])
    );
    assert_eq!(d["python-version"]["selected"], json!("Default"));
    // None of them is on the machine: uv downloads the one chosen.
    assert_eq!(d["python-version"]["download"], json!(["3.14", "3.13"]));
    invoke(
        cx,
        &f,
        "python.interpreter.add-dialog.open-list",
        json!({"list": "python-version"}),
    );
    let d = dialog(cx, &f);
    let rows: Vec<(String, Json)> = d["menu"]["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["label"].as_str().unwrap().to_owned(),
                i["secondary"].clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Default".to_owned(), Json::Null),
            ("3.14".to_owned(), json!(" download and install")),
            ("3.13".to_owned(), json!(" download and install")),
        ]
    );
    invoke(cx, &f, "menu.cancel", Json::Null);
    assert_eq!(
        d["location"]["alert"],
        json!({"kind": "error", "text": "Environment at \".venv\" already exists", "links": ["Override existing environment", "Select existing interpreter"]})
    );
    assert_eq!(d["ok-enabled"], json!(false));
    invoke(cx, &f, "python.interpreter.add-dialog.override", Json::Null);
    let d = dialog(cx, &f);
    assert_eq!(
        d["location"]["alert"],
        json!({"kind": "warning", "text": "Existing environment at \".venv\" will be overridden", "links": ["Select existing interpreter"]})
    );
    assert_eq!(d["ok-enabled"], json!(true));
    // "Select existing interpreter" of a uv location: Select existing / uv.
    invoke(
        cx,
        &f,
        "python.interpreter.add-dialog.select-existing",
        Json::Null,
    );
    let d = dialog(cx, &f);
    assert_eq!(
        (d["environment"].clone(), d["type"].clone()),
        (json!("select-existing"), json!("uv"))
    );

    // Cancel closes it, nothing is added.
    let before = cx.update(|cx| list::table(cx).entries.len());
    invoke(cx, &f, "python.interpreter.add-dialog.cancel", Json::Null);
    assert_eq!(dialog(cx, &f), Json::Null);
    assert_eq!(cx.update(|cx| list::table(cx).entries.len()), before);
}

/// Without uv: "No uv executable found" with its links; a virtual
/// environment is made with the base's `venv` module.
#[gpui::test]
fn without_uv_environments_come_from_the_venv_module(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let m = fake_machine(&f);
    use_machine(cx, &f, &m, false);
    invoke(cx, &f, "settings.open", json!({"page": PAGE}));
    invoke(cx, &f, "python.interpreter.add-local", Json::Null);
    wait_checked(cx, &f);
    set(cx, &f, "type", "uv");
    let d = wait_checked(cx, &f);
    assert_eq!(
        d["rows"],
        json!([
            "Environment:",
            "Type:",
            "Python version:",
            "No uv executable found",
            "Path to uv:",
            "Location:",
            "alert"
        ]),
        "{d}"
    );
    assert_eq!(
        d["uv-path"]["missing"],
        json!({"kind": "warning", "text": "No uv executable found", "links": ["Install uv", "Select path"]})
    );
    assert_eq!(d["python-version"]["items"], json!(["Default"]));
    assert_eq!(d["ok-enabled"], json!(false));

    // Generate new / Virtualenv at a free location: OK makes it with
    // `python3.12 -m venv`, the page selects it until Apply.
    set(cx, &f, "type", "virtualenv");
    let location = f.base.join("envs/made");
    set(cx, &f, "location", &path(&location));
    set(cx, &f, "inherit-site-packages", "true");
    let d = wait_checked(cx, &f);
    assert_eq!(d["ok-enabled"], json!(true), "{d}");
    invoke(cx, &f, "python.interpreter.add-dialog.ok", Json::Null);
    assert_eq!(dialog(cx, &f), Json::Null);
    let python = py_env::paths::venv_python(&location);
    wait_for(cx, "the new environment", |cx| {
        page(cx, &f)["selected"]["path"] == json!(path(&python))
    });
    let entry = cx.update(|cx| list::table(cx).find_python(&python).cloned().unwrap());
    assert_eq!(entry.flavor, table::Flavor::Venv);
    assert_eq!(entry.version.as_deref(), Some("3.12.7"));
    assert_eq!(entry.associated_project.as_deref(), Some(f.root.as_path()));
    // The base is remembered for the next environment.
    let preferred = cx.update(|cx| list::table(cx).preferred.base_python.clone());
    assert_eq!(preferred.as_deref(), Some(m.python312.as_path()));
    let s = invoke(cx, &f, "settings.state", Json::Null);
    assert_eq!(s["modified"], json!([PAGE]), "{s}");
    assert!(!cx.update(|cx| configuring::is_configuring(&f.root, cx)));
}

/// Generate new / uv: `uv venv` at the location with the version asked
/// for, `uv sync` in a project with `pyproject.toml`, `uv init --bare
/// --no-project` before it in one without; from the status bar the new
/// environment is the project's at once.
#[gpui::test]
fn uv_environments_are_made_as_the_reference_makes_them(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let m = fake_machine(&f);
    use_machine(cx, &f, &m, true);
    invoke(cx, &f, "python.interpreter.add-local", Json::Null);
    wait_checked(cx, &f);
    set(cx, &f, "type", "uv");
    let location = f.root.join("env-uv");
    set(cx, &f, "location", &path(&location));
    set(cx, &f, "python-version", "3.13");
    let d = wait_checked(cx, &f);
    assert_eq!(d["python-version"]["selected"], json!("3.13"), "{d}");
    assert_eq!(d["ok-enabled"], json!(true), "{d}");
    invoke(cx, &f, "python.interpreter.add-dialog.ok", Json::Null);
    let python = py_env::paths::venv_python(&location);
    wait_for(cx, "the uv environment", |cx| {
        let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
        w["selected"]["python"] == json!(path(&python)) && w["configuring"] == json!(false)
    });
    let entry = cx.update(|cx| list::table(cx).find_python(&python).cloned().unwrap());
    assert_eq!(entry.flavor, table::Flavor::Uv);
    assert_eq!(entry.uv.as_deref(), Some(m.uv.as_path()));
    assert_eq!(entry.working_dir.as_deref(), Some(f.root.as_path()));
    assert_eq!(entry.version.as_deref(), Some("3.13.5"));
    let log = std::fs::read_to_string(&m.log).unwrap();
    assert!(
        log.contains(&format!("venv --python 3.13 {}", path(&location))),
        "{log}"
    );
    // 3.13 is not on the machine: installed first, the environment names
    // the version.
    let install = log.lines().position(|l| l == "python install 3.13");
    let venv = log.lines().position(|l| l.starts_with("venv "));
    assert!(install.is_some() && install < venv, "{log}");
    assert!(!log.contains("python find"), "{log}");
    assert!(log.lines().any(|l| l.starts_with("sync")), "{log}");
    assert!(!log.lines().any(|l| l.starts_with("init")), "{log}");

    // A project without `pyproject.toml`: `uv init` first, no sync; the
    // progress shows meanwhile.
    std::fs::remove_file(f.root.join("pyproject.toml")).unwrap();
    std::fs::write(&m.log, "").unwrap();
    invoke(cx, &f, "python.interpreter.add-local", Json::Null);
    wait_checked(cx, &f);
    set(cx, &f, "type", "uv");
    let second = f.root.join("env-two");
    set(cx, &f, "location", &path(&second));
    wait_checked(cx, &f);
    invoke(cx, &f, "python.interpreter.add-dialog.ok", Json::Null);
    let python = py_env::paths::venv_python(&second);
    wait_for(cx, "the second uv environment", |cx| {
        let w = invoke(cx, &f, "python.interpreter.state", Json::Null);
        w["selected"]["python"] == json!(path(&python)) && w["configuring"] == json!(false)
    });
    let log = std::fs::read_to_string(&m.log).unwrap();
    assert!(
        log.contains("init --name demo --bare --no-project"),
        "{log}"
    );
    assert!(!log.lines().any(|l| l.starts_with("sync")), "{log}");
    settle(cx);
}

/// A Python to download as the base of a new virtual environment (I3_1):
/// OK installs it with `uv python install`, finds it with `uv python
/// find` and makes the environment with its `venv` module.
#[gpui::test]
fn a_python_to_download_is_installed_before_its_environment(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let m = fake_machine(&f);
    use_machine(cx, &f, &m, true);
    invoke(cx, &f, "settings.open", json!({"page": PAGE}));
    invoke(cx, &f, "python.interpreter.add-local", Json::Null);
    let d = wait_checked(cx, &f);
    // uv's CPython downloads, the newest of each version, no free-threaded
    // or PyPy builds.
    assert_eq!(
        d["downloads"]["versions"],
        json!(["3.14.0", "3.13.5", "3.12.7"]),
        "{d}"
    );
    let row = &d["base-python"]["items"][3];
    assert_eq!(row["path"], json!("download:3.13.5"), "{d}");
    assert_eq!(row["text"], json!("Python 3.13.5"));
    assert_eq!(row["detail"], json!(" download and install"));
    assert_eq!(row["origin"], json!("download"));
    assert_eq!(row["icon"], json!("interpreter/download"));
    // The list shows them with the download icon, fully opaque.
    invoke(
        cx,
        &f,
        "python.interpreter.add-dialog.open-list",
        json!({"list": "base-python"}),
    );
    let d = dialog(cx, &f);
    let items = d["menu"]["levels"][0]["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 4, "{d}");
    assert_eq!(items[2]["label"], json!("Python 3.14.0"));
    assert_eq!(items[2]["secondary"], json!(" download and install"));
    assert!(
        items[2]["icon_alpha"].as_f64().is_none_or(|a| a == 1.0),
        "{d}"
    );
    invoke(cx, &f, "menu.cancel", Json::Null);
    // A version not offered is refused.
    set(cx, &f, "base-python", "download:3.9.1");
    let d = dialog(cx, &f);
    assert_eq!(
        d["base-python"]["selected"]["path"],
        json!(path(&m.python312)),
        "{d}"
    );

    set(cx, &f, "base-python", "download:3.13.5");
    let location = f.base.join("envs/downloaded");
    set(cx, &f, "location", &path(&location));
    let d = wait_checked(cx, &f);
    assert_eq!(
        d["base-python"]["selected"]["path"],
        json!("download:3.13.5")
    );
    assert_eq!(d["base-python"]["notice"], Json::Null, "{d}");
    assert_eq!(d["ok-enabled"], json!(true), "{d}");
    invoke(cx, &f, "python.interpreter.add-dialog.ok", Json::Null);
    let python = py_env::paths::venv_python(&location);
    wait_for(cx, "the environment of the downloaded Python", |cx| {
        page(cx, &f)["selected"]["path"] == json!(path(&python))
            && !cx.update(|cx| configuring::is_configuring(&f.root, cx))
    });
    let log = std::fs::read_to_string(&m.log).unwrap();
    let install = log.lines().position(|l| l == "python install 3.13.5");
    let find = log
        .lines()
        .position(|l| l == "python find --managed-python --no-project 3.13.5");
    assert!(install.is_some() && install < find, "{log}");
    let entry = cx.update(|cx| list::table(cx).find_python(&python).cloned().unwrap());
    assert_eq!(entry.flavor, table::Flavor::Venv);
    assert_eq!(entry.version.as_deref(), Some("3.13.5"));
    // The installed Python is the base remembered for the next one.
    let preferred = cx.update(|cx| list::table(cx).preferred.base_python.clone());
    assert_eq!(preferred.as_deref(), Some(m.installed313.as_path()));
    settle(cx);
}

/// With the real uv of the machine (`uv` on `PATH`): a new uv environment
/// in a temporary project. Needs uv and, without a Python it manages, the
/// network; run with `cargo test -p pyoxide-ide -- --ignored real_uv`.
#[gpui::test]
#[ignore = "needs uv on PATH and may download a Python"]
fn real_uv_makes_an_environment(cx: &mut TestAppContext) {
    let f = fixture(cx);
    std::fs::remove_dir_all(f.root.join(".venv")).unwrap();
    invoke(cx, &f, "python.interpreter.add-local", Json::Null);
    for _ in 0..3000 {
        settle(cx);
        let d = dialog(cx, &f);
        if d["uv-path"]["found"] == json!(true) || d["type"] == json!("virtualenv") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    set(cx, &f, "environment", "generate-new");
    set(cx, &f, "type", "uv");
    let location = f.root.join(".venv");
    set(cx, &f, "location", &path(&location));
    let mut ok = false;
    for _ in 0..3000 {
        settle(cx);
        if dialog(cx, &f)["ok-enabled"] == json!(true) {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ok, "{}", dialog(cx, &f));
    invoke(cx, &f, "python.interpreter.add-dialog.ok", Json::Null);
    let python = py_env::paths::venv_python(&location);
    for _ in 0..6000 {
        settle(cx);
        if !cx.update(|cx| configuring::is_configuring(&f.root, cx)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(python.is_file(), "no {}", python.display());
    let entry = cx.update(|cx| list::table(cx).find_python(&python).cloned().unwrap());
    assert_eq!(entry.flavor, table::Flavor::Uv);
    assert!(entry.version.is_some());
}
