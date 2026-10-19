//! Soft wraps in the main window (tests/scenarios/editor/soft-wraps.md):
//! files matching "Soft-wrap these files" open with soft wraps, and the
//! Soft-Wrap command of the active editor turns them on and off, also when
//! another component has the focus.

use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::editor_settings::SoftWrapSettings;
use workbench::{FileType, Product, Workbench};

fn open(cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType::plain(
            "Python",
            &["py"],
            ui::icons::IconName::FILE_TEXT,
        )],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<Workbench>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn invoke(cx: &mut TestAppContext, window: WindowHandle<Workbench>, id: &str, args: Value) {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke(id, args, window, cx)
    })
    .unwrap()
    .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    draw(cx, window);
}

fn editor_state(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Value {
    cx.update_window(window.into(), |_, window, cx| {
        ui::command::invoke("workbench.state", json!(null), window, cx)
    })
    .unwrap()
    .unwrap()["editor"]
        .clone()
}

#[gpui::test]
fn masked_files_open_wrapped_and_the_command_toggles_the_active_editor(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let long = "word ".repeat(200);
    std::fs::write(dir.path().join("README.md"), format!("{long}\n")).unwrap();
    std::fs::write(dir.path().join("main.py"), format!("x = '{long}'\n")).unwrap();
    let window = open(cx);
    cx.update(|cx| {
        workbench::editor_settings::set_soft_wraps(
            cx,
            &SoftWrapSettings {
                enabled: true,
                files: None,
                ..SoftWrapSettings::default()
            },
        )
    });
    // Every file once turned on without masks; only the masked ones with.
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": dir.path().display().to_string()}),
    );
    let open_file = |cx: &mut TestAppContext, name: &str| {
        let path = dir.path().join(name).display().to_string();
        invoke(cx, window, "project.open-file", json!({ "path": path }));
    };
    open_file(cx, "main.py");
    let s = editor_state(cx, window);
    assert_eq!(s["soft_wraps"], true);
    assert!(s["rows"].as_u64().unwrap() > s["lines"].as_u64().unwrap());
    invoke(cx, window, "tabs.close", json!(null));
    cx.update(|cx| {
        workbench::editor_settings::set_soft_wraps(
            cx,
            &SoftWrapSettings {
                enabled: true,
                files: Some(workbench::editor_settings::DEFAULT_SOFT_WRAP_FILES.into()),
                ..SoftWrapSettings::default()
            },
        )
    });
    open_file(cx, "README.md");
    assert_eq!(editor_state(cx, window)["soft_wraps"], true);
    open_file(cx, "main.py");
    let s = editor_state(cx, window);
    assert_eq!(s["soft_wraps"], false);
    assert_eq!(s["rows"], s["lines"]);
    // The command reaches the active editor through the window (the
    // project tree has the focus after opening the folder).
    invoke(cx, window, "editor.toggle-soft-wraps", json!(null));
    let s = editor_state(cx, window);
    assert_eq!(s["soft_wraps"], true);
    assert!(s["rows"].as_u64().unwrap() > s["lines"].as_u64().unwrap());
    invoke(cx, window, "editor.toggle-soft-wraps", json!(null));
    assert_eq!(editor_state(cx, window)["soft_wraps"], false);
}

/// Settings > Editor > General: the Soft Wraps group edits
/// `[editor.soft-wraps]` of the store, and the open editors follow on
/// Apply (tests/scenarios/settings/editor-general.md).
#[gpui::test]
fn the_soft_wraps_group_edits_the_store_and_open_editors_follow(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    std::fs::create_dir(&root).unwrap();
    let long = "word ".repeat(200);
    std::fs::write(root.join("README.md"), format!("{long}\n")).unwrap();
    let file = dir.path().join("config").join("settings.toml");
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let store = file.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store);
        workbench::open_main_window(cx, Rc::new(product))
    });
    draw(cx, window);
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({"path": "README.md"}),
    );
    assert_eq!(editor_state(cx, window)["soft_wraps"], false);

    let settings = |cx: &mut TestAppContext| -> Value {
        cx.update_window(window.into(), |_, window, cx| {
            ui::command::invoke("settings.state", json!(null), window, cx)
        })
        .unwrap()
        .unwrap()
    };
    let edit = |cx: &mut TestAppContext, field: &str, value: Value| {
        invoke(
            cx,
            window,
            "settings.edit",
            json!({"field": field, "value": value}),
        )
    };
    let file_text = |cx: &mut TestAppContext| {
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        std::fs::read_to_string(&file).unwrap_or_default()
    };

    invoke(
        cx,
        window,
        "settings.open",
        json!({"page": "editor.general"}),
    );
    let page = settings(cx)["fields"].clone();
    assert_eq!(page["soft-wraps"], false, "{page}");
    assert_eq!(page["files"], "*.md; *.txt; *.rst; *.adoc", "{page}");
    assert_eq!(page["files-enabled"], false, "{page}");
    assert_eq!(page["original-indent"], true, "{page}");
    assert_eq!(page["indent"], "0", "{page}");
    assert_eq!(page["current-line-only"], true, "{page}");

    // Turned on: the open editor waits for Apply.
    edit(cx, "soft-wraps", json!(true));
    let s = settings(cx);
    assert_eq!(s["modified"], json!(["editor.general"]), "{s}");
    assert_eq!(s["fields"]["files-enabled"], true, "{s}");
    assert_eq!(editor_state(cx, window)["soft_wraps"], false);
    invoke(cx, window, "settings.apply", json!(null));
    let e = editor_state(cx, window);
    assert_eq!(e["soft_wraps"], true, "{e}");
    assert!(e["rows"].as_u64().unwrap() > e["lines"].as_u64().unwrap());
    let text = file_text(cx);
    assert!(text.contains("[editor.soft-wraps]"), "{text}");
    assert!(text.contains("enabled = true"), "{text}");
    // The masks shown are kept: without them, soft wraps turned on mean
    // every file (`getSoftWrapFileMasks`).
    assert!(
        text.contains("files = \"*.md; *.txt; *.rst; *.adoc\""),
        "{text}"
    );

    // An indent that is no number keeps the page with an error.
    edit(cx, "indent", json!("x"));
    invoke(cx, window, "settings.apply", json!(null));
    let s = settings(cx);
    assert!(s["errors"]["editor.general"].is_string(), "{s}");
    edit(cx, "indent", json!("4"));
    edit(cx, "current-line-only", json!(false));
    invoke(cx, window, "settings.apply", json!(null));
    let text = file_text(cx);
    assert!(text.contains("indent = 4"), "{text}");
    assert!(text.contains("show-all = true"), "{text}");

    // The window's search finds the group's options (S2: "soft").
    invoke(cx, window, "settings.search", json!({"text": "soft"}));
    let s = settings(cx);
    assert_eq!(s["page"], "editor.general", "{s}");
    assert_eq!(s["search"]["no_match"], false, "{s}");
    invoke(cx, window, "settings.search", json!({"text": ""}));

    // Masks without Markdown: the open README.md is no longer wrapped.
    edit(cx, "files", json!("*.txt"));
    invoke(cx, window, "settings.ok", json!(null));
    assert_eq!(editor_state(cx, window)["soft_wraps"], false);
    assert!(file_text(cx).contains("files = \"*.txt\""));
}
