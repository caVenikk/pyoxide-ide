//! Soft wraps in the main window (tests/scenarios/editor/soft-wraps.md):
//! files matching "Soft-wrap these files" open with soft wraps, and the
//! Soft-Wrap command of the active editor turns them on and off, also when
//! another component has the focus.

use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::editor_settings::{EditorSettings, SoftWrapSettings};
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
        workbench::editor_settings::set_settings(
            cx,
            EditorSettings {
                soft_wraps: SoftWrapSettings {
                    enabled: true,
                    files: None,
                    ..SoftWrapSettings::default()
                },
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
        workbench::editor_settings::set_settings(
            cx,
            EditorSettings {
                soft_wraps: SoftWrapSettings {
                    enabled: true,
                    files: Some(workbench::editor_settings::DEFAULT_SOFT_WRAP_FILES.into()),
                    ..SoftWrapSettings::default()
                },
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
