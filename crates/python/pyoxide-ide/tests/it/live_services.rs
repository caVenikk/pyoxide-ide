//! The product with real language servers, headless (GPUI's test
//! platform), driven through command ids. Ignored by default: it needs
//! ruff and ty installed. The folder `PYOXIDE_PY_TOOLS` names is searched
//! for the tools, else `PATH`:
//!
//! ```sh
//! PYOXIDE_PY_TOOLS=/tmp/py-tools/bin cargo test -p pyoxide-ide --test it live_services:: -- --ignored
//! ```

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use py_lsp::SearchPaths;
use serde_json::{Value, json};
use workbench::Workbench;

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
    cx.run_until_parked();
    out
}

/// Runs `check` on the answer of the query `id` until it holds, while the
/// servers work on their own threads.
fn wait_for(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    check: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let state = invoke(cx, window, id, Value::Null);
        if check(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "{id}: {state:#}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[gpui::test]
#[ignore = "needs ruff and ty installed"]
fn quick_documentation_of_a_standard_library_function(cx: &mut TestAppContext) {
    // The servers answer from their own threads, also while the
    // application quits and waits for them to exit.
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("app.py"), "import json\n\njson.dumps({})\n").unwrap();
    let mut search = SearchPaths::from_env();
    if let Some(tools) = std::env::var_os("PYOXIDE_PY_TOOLS") {
        search.path.insert(0, PathBuf::from(tools));
    }
    let product = Rc::new(pyoxide_ide::product_with_tools(search));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, product.clone())
    });
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    invoke(cx, window, "project.open-file", json!({"path": "app.py"}));
    // ty runs and serves the file.
    wait_for(cx, window, "code.servers", |servers| {
        servers.as_array().is_some_and(|servers| {
            servers
                .iter()
                .any(|s| s["id"] == "ty" && s["state"]["state"] == "running")
        })
    });
    invoke(
        cx,
        window,
        "editor.set-selection",
        json!({"anchor": {"line": 2, "column": 8}, "head": {"line": 2, "column": 8}}),
    );
    // F1 on `dumps`, again while ty is still reading the standard library.
    let deadline = Instant::now() + Duration::from_secs(60);
    let documentation = loop {
        invoke(cx, window, "code.quick-doc", Value::Null);
        let shown = Instant::now() + Duration::from_secs(5);
        let mut found = None;
        while Instant::now() < shown {
            let state = invoke(cx, window, "code.state", Value::Null);
            if state["popup"]["documentation"]["location"].is_string() {
                found = Some(state["popup"]["documentation"].clone());
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Some(found) = found {
            break found;
        }
        assert!(Instant::now() < deadline, "no documentation of json.dumps");
    };
    // The header is the module, as the reference shows it; the
    // declaration is Python (coloured by the product's lexer).
    assert_eq!(documentation["location"], "json", "{documentation:#}");
    let definition = documentation["definition"].as_str().unwrap();
    assert!(definition.starts_with("def dumps("), "{definition}");

    // Quit while waiting for the servers is allowed: they exit.
    cx.quit();
}
