//! The Python Interpreters dialog (Show All…), headless (GPUI's test
//! platform), driven through command ids and keys, with the list of
//! configured interpreters of `page_tests::fixture`.

use std::path::{Path, PathBuf};

use gpui::{AnyWindowHandle, TestAppContext};
use serde_json::{Value as Json, json};

use super::list_dialog::{InterpretersDialog, RenameDialog};
use super::page::{NO_INTERPRETER, PAGE, SHOW_ALL};
use super::page_tests::{Fixture, fixture, invoke, page, settle};
use super::{list, table};

fn dialog(cx: &mut TestAppContext, f: &Fixture) -> Json {
    invoke(cx, f, "python.interpreter.state", Json::Null)["interpreters-dialog"].clone()
}

fn names(d: &Json) -> Vec<String> {
    d["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The test's folder (`Fixture::base` is there on Unix only).
fn base(f: &Fixture) -> PathBuf {
    f.root.parent().unwrap().to_owned()
}

fn path(p: &Path) -> String {
    p.display().to_string()
}

/// The name the reference gives an environment of the project.
fn env_name(f: &Fixture, dir: &str) -> String {
    project::display_path(&f.root.join(dir))
}

fn select(cx: &mut TestAppContext, f: &Fixture, name: &str) {
    invoke(
        cx,
        f,
        "python.interpreters.dialog.select",
        json!({ "name": name }),
    );
}

fn set(cx: &mut TestAppContext, f: &Fixture, field: &str, value: &str) {
    invoke(
        cx,
        f,
        "python.interpreters.dialog.set",
        json!({"field": field, "value": value}),
    );
}

fn rename_to(cx: &mut TestAppContext, f: &Fixture, text: &str) -> Json {
    invoke(
        cx,
        f,
        "python.interpreters.rename-dialog.set",
        json!({ "text": text }),
    );
    dialog(cx, f)["rename"].clone()
}

/// The window of a dialog of the test.
fn window_of<V: 'static>(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| w.downcast::<V>().is_some())
            .expect("the dialog's window")
    })
}

fn open_from_page(cx: &mut TestAppContext, f: &Fixture) -> Json {
    invoke(cx, f, "settings.open", json!({"page": PAGE}));
    // "Show All…" of the combo opens it; the combo keeps its selection.
    let before = page(cx, f)["selected"].clone();
    let items = page(cx, f)["items"].clone();
    assert_eq!(
        items.as_array().unwrap().last().unwrap()["text"],
        json!(SHOW_ALL)
    );
    invoke(cx, f, "python.interpreters.show-all", Json::Null);
    assert_eq!(page(cx, f)["selected"], before);
    dialog(cx, f)
}

#[gpui::test]
fn the_list_its_details_and_apply(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let d = open_from_page(cx, &f);
    assert_eq!(d["title"], json!("Python Interpreters"), "{d}");
    assert_eq!(d["origin"], json!("page"));
    // The configured interpreters and the project's environments by name;
    // the other project's environment is hidden.
    assert_eq!(
        names(&d),
        [
            env_name(&f, ".venv"),
            env_name(&f, "venv"),
            "mine".into(),
            "Python 3.14".into()
        ]
    );
    assert_eq!(d["items"][0]["configured"], json!(false));
    assert_eq!(d["items"][0]["icon"], json!("python/uv"));
    assert_eq!(d["items"][2]["configured"], json!(true));
    // The page's interpreter is selected: a uv environment has no
    // association rows.
    assert_eq!(d["selected"], json!(env_name(&f, ".venv")));
    assert_eq!(
        d["details"],
        json!({"label": "Interpreter path:", "path": path(&f.uv), "association": null})
    );
    let toolbar: Vec<(String, bool)> = d["toolbar"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["tooltip"].as_str().unwrap().to_owned(),
                t["enabled"] == json!(true),
            )
        })
        .collect();
    assert_eq!(
        toolbar,
        [
            ("Add New Interpreter".to_owned(), true),
            ("Remove Interpreter".to_owned(), true),
            ("Rename".to_owned(), true),
            (
                "Hide virtual environments associated with other projects".to_owned(),
                true
            ),
            ("Show Interpreter Paths".to_owned(), true),
        ]
    );
    assert_eq!(d["toolbar"][3]["on"], json!(true));
    assert_eq!(d["modified"], json!(false));
    // The empty "Python 3.14" is an interpreter whose file exists.
    assert_eq!(d["items"][3]["invalid"], json!(false));

    // The filter off: the other project's environment, its association
    // fixed until "Remove association".
    invoke(
        cx,
        &f,
        "python.interpreters.dialog.toggle-filter",
        Json::Null,
    );
    let d = dialog(cx, &f);
    assert!(names(&d).contains(&"other env".to_owned()), "{d}");
    assert_eq!(d["toolbar"][3]["on"], json!(false));
    select(cx, &f, "other env");
    let d = dialog(cx, &f);
    assert_eq!(
        d["details"]["association"],
        json!({
            "text": format!("Associate this virtual environment with {}", path(&base(&f).join("other"))),
            "checked": true,
            "enabled": false,
            "remove-link": "Remove association",
        })
    );
    invoke(
        cx,
        &f,
        "python.interpreters.dialog.remove-association",
        Json::Null,
    );
    let d = dialog(cx, &f);
    assert_eq!(
        d["details"]["association"],
        json!({
            "text": "Associate this virtual environment with the current project",
            "checked": false,
            "enabled": true,
            "remove-link": null,
        })
    );
    assert_eq!(d["modified"], json!(true));

    // "mine" is associated with this project; untick it and give it
    // another path.
    select(cx, &f, "mine");
    let d = dialog(cx, &f);
    assert_eq!(d["details"]["association"]["checked"], json!(true), "{d}");
    set(cx, &f, "associated", "false");
    let moved = base(&f).join("envs/moved/bin/python");
    set(cx, &f, "path", &path(&moved));
    let d = dialog(cx, &f);
    assert_eq!(d["details"]["association"]["checked"], json!(false));
    assert_eq!(d["details"]["path"], json!(path(&moved)));
    // Remove "Python 3.14": the next row is selected; nothing changes
    // before Apply.
    select(cx, &f, "Python 3.14");
    invoke(cx, &f, "python.interpreters.dialog.remove", Json::Null);
    let d = dialog(cx, &f);
    assert!(!names(&d).contains(&"Python 3.14".to_owned()), "{d}");
    assert_eq!(d["selected"], json!("other env"));
    assert!(cx.update(|cx| list::table(cx).find("Python 3.14").is_some()));

    invoke(cx, &f, "python.interpreters.dialog.apply", Json::Null);
    let d = dialog(cx, &f);
    assert_eq!(d["modified"], json!(false), "{d}");
    let t = cx.update(|cx| list::table(cx));
    assert!(t.find("Python 3.14").is_none());
    assert_eq!(t.find("other env").unwrap().associated_project, None);
    let mine = t.find("mine").unwrap();
    assert_eq!(mine.associated_project, None);
    assert_eq!(mine.python, moved);
    // And in the file.
    let file = std::fs::read_to_string(&f.file).unwrap();
    assert!(!file.contains("Python 3.14"), "{file}");
    assert!(file.contains(&path(&moved)), "{file}");

    // Cancel: the page keeps its interpreter.
    invoke(cx, &f, "python.interpreters.dialog.cancel", Json::Null);
    assert_eq!(dialog(cx, &f), Json::Null);
    assert_eq!(page(cx, &f)["selected"]["path"], json!(path(&f.uv)));
}

#[gpui::test]
fn rename_refuses_blank_and_taken_names(cx: &mut TestAppContext) {
    let f = fixture(cx);
    open_from_page(cx, &f);
    select(cx, &f, "mine");
    invoke(cx, &f, "python.interpreters.dialog.rename", Json::Null);
    let r = dialog(cx, &f)["rename"].clone();
    assert_eq!(
        r,
        json!({"title": "Rename Python Interpreter", "label": "New name:", "text": "mine", "error": null, "ok-enabled": false})
    );
    let r = rename_to(cx, &f, "Python 3.14");
    assert_eq!(
        (r["error"].clone(), r["ok-enabled"].clone()),
        (
            json!("Python interpreter with the same name already exists"),
            json!(false)
        )
    );
    // The project's environments count too.
    let r = rename_to(cx, &f, &env_name(&f, "venv"));
    assert_eq!(r["ok-enabled"], json!(false), "{r}");
    let r = rename_to(cx, &f, "  ");
    assert_eq!(r["error"], json!("Provide a name for a Python interpreter"));
    // OK with an error does nothing.
    invoke(cx, &f, "python.interpreters.rename-dialog.ok", Json::Null);
    assert!(!dialog(cx, &f)["rename"].is_null());
    let r = rename_to(cx, &f, " work ");
    assert_eq!(
        (r["error"].clone(), r["ok-enabled"].clone()),
        (Json::Null, json!(true))
    );
    invoke(cx, &f, "python.interpreters.rename-dialog.ok", Json::Null);
    // Renamed at once, as the reference renames the SDK.
    let d = dialog(cx, &f);
    assert_eq!(d["rename"], Json::Null);
    assert_eq!(d["selected"], json!("work"));
    assert_eq!(d["modified"], json!(false));
    assert!(cx.update(|cx| list::table(cx).find("work").is_some()));
    assert!(cx.update(|cx| list::table(cx).find("mine").is_none()));

    // A project's environment not configured yet joins the list with its
    // new name.
    select(cx, &f, &env_name(&f, "venv"));
    invoke(cx, &f, "python.interpreters.dialog.rename", Json::Null);
    rename_to(cx, &f, "plain");
    invoke(cx, &f, "python.interpreters.rename-dialog.ok", Json::Null);
    let entry = cx.update(|cx| list::table(cx).find("plain").cloned().unwrap());
    assert_eq!(entry.python, f.plain);
    assert_eq!(entry.flavor, table::Flavor::Venv);
    assert_eq!(entry.associated_project.as_deref(), Some(f.root.as_path()));
    let d = dialog(cx, &f);
    assert_eq!(d["selected"], json!("plain"));
    assert_eq!(
        d["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["name"] == json!("plain"))
            .count(),
        1
    );

    // The app-level commands.
    invoke(
        cx,
        &f,
        "python.interpreters.rename",
        json!({"name": "plain", "new_name": "plain env"}),
    );
    invoke(
        cx,
        &f,
        "python.interpreters.remove",
        json!({"name": "work"}),
    );
    let t = cx.update(|cx| list::table(cx));
    assert!(t.find("plain env").is_some() && t.find("work").is_none());
}

#[gpui::test]
fn ok_hands_the_choice_to_the_page(cx: &mut TestAppContext) {
    let f = fixture(cx);
    open_from_page(cx, &f);
    select(cx, &f, "mine");
    invoke(cx, &f, "python.interpreters.dialog.ok", Json::Null);
    assert_eq!(dialog(cx, &f), Json::Null);
    let p = page(cx, &f);
    assert_eq!(p["selected"]["name"], json!("mine"), "{p}");
    let s = invoke(cx, &f, "settings.state", Json::Null);
    assert_eq!(s["modified"], json!([PAGE]), "{s}");

    // Opened again it selects the page's choice; remove it, Apply,
    // Cancel: the page shows "<No interpreter>".
    invoke(cx, &f, "python.interpreters.show-all", Json::Null);
    assert_eq!(dialog(cx, &f)["selected"], json!("mine"));
    invoke(cx, &f, "python.interpreters.dialog.remove", Json::Null);
    invoke(cx, &f, "python.interpreters.dialog.apply", Json::Null);
    invoke(cx, &f, "python.interpreters.dialog.cancel", Json::Null);
    let p = page(cx, &f);
    assert_eq!(
        p["selected"],
        json!({"kind": "none", "text": NO_INTERPRETER}),
        "{p}"
    );
}

#[gpui::test]
fn keys_of_the_list(cx: &mut TestAppContext) {
    let f = fixture(cx);
    // Without the page: over the main window.
    invoke(cx, &f, "python.interpreters.show-all", Json::Null);
    let d = dialog(cx, &f);
    assert_eq!(d["origin"], json!("window"), "{d}");
    assert_eq!(d["selected"], json!(env_name(&f, ".venv")));
    let window = window_of::<InterpretersDialog>(cx);
    cx.simulate_keystrokes(window, "down down");
    settle(cx);
    assert_eq!(dialog(cx, &f)["selected"], json!("mine"));
    cx.simulate_keystrokes(window, "up");
    settle(cx);
    assert_eq!(dialog(cx, &f)["selected"], json!(env_name(&f, "venv")));
    cx.simulate_keystrokes(window, "delete");
    settle(cx);
    let d = dialog(cx, &f);
    assert_eq!(d["removed"], json!([env_name(&f, "venv")]), "{d}");
    assert_eq!(d["selected"], json!("mine"));
    cx.simulate_keystrokes(window, "shift-f6");
    settle(cx);
    assert_eq!(dialog(cx, &f)["rename"]["text"], json!("mine"));
    // Escape closes the Rename dialog, then the dialog.
    let rename = window_of::<RenameDialog>(cx);
    cx.simulate_keystrokes(rename, "escape");
    settle(cx);
    assert_eq!(dialog(cx, &f)["rename"], Json::Null);
    let add = if cfg!(target_os = "macos") {
        "ctrl-n"
    } else {
        "insert"
    };
    cx.simulate_keystrokes(window, add);
    settle(cx);
    let menu = dialog(cx, &f)["menu"].clone();
    assert_eq!(
        menu["levels"][0]["items"][0]["label"],
        json!("Add Local Interpreter\u{2026}"),
        "{menu}"
    );
    invoke(cx, &f, "menu.cancel", Json::Null);
    assert_eq!(dialog(cx, &f)["menu"], Json::Null);
    cx.simulate_keystrokes(window, "escape");
    settle(cx);
    assert_eq!(dialog(cx, &f), Json::Null);
    // Nothing was applied.
    assert!(cx.update(|cx| list::table(cx).find("mine").is_some()));
}

/// Show Interpreter Paths asks the interpreter (a script here) for its
/// paths: the folders that exist, each once.
#[cfg(unix)]
#[gpui::test]
fn interpreter_paths_come_from_the_interpreter(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let lib = base(&f).join("python/lib/python3.14");
    let site = base(&f).join("python/lib/python3.14/site-packages");
    std::fs::create_dir_all(&site).unwrap();
    super::page_tests::script(
        &base(&f).join("python/bin/python3.14"),
        &format!(
            "printf '%s\\n' '{}' '{}' '{}' '{}'",
            path(&base(&f).join("python/lib/python314.zip")),
            path(&lib),
            path(&site),
            path(&lib),
        ),
    );
    open_from_page(cx, &f);
    select(cx, &f, "Python 3.14");
    invoke(cx, &f, "python.interpreters.dialog.show-paths", Json::Null);
    super::page_tests::wait_for(cx, "the paths", |cx| {
        dialog(cx, &f)["paths"]["loading"] == json!(false)
    });
    let p = dialog(cx, &f)["paths"].clone();
    assert_eq!(p["title"], json!("Interpreter Paths"));
    assert_eq!(p["paths"], json!([path(&lib), path(&site)]), "{p}");
    invoke(
        cx,
        &f,
        "python.interpreters.paths-dialog.reload",
        Json::Null,
    );
    super::page_tests::wait_for(cx, "the paths again", |cx| {
        dialog(cx, &f)["paths"]["loading"] == json!(false)
    });
    invoke(cx, &f, "python.interpreters.paths-dialog.ok", Json::Null);
    assert_eq!(dialog(cx, &f)["paths"], Json::Null);
}
