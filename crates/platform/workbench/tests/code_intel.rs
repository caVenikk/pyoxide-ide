//! The product's language services in the main window (ADR 0010): a
//! project gets its services from the product's provider, the code
//! intelligence commands and queries reach the window's view, and a
//! product without services has none. Driven against the in-memory
//! `FakeCodeIntel`.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use code_intel::fake::{Call, FakeCodeIntel};
use code_intel::{CodeIntel, ServerId, ServerStatus};
use gpui::{App, AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Product, Workbench};

struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }
}

fn open(cx: &mut TestAppContext, hook: Option<CodeIntelHook>) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            ..FileType::plain("Python", &["py"], ui::icons::IconName::FILE_TEXT)
        }],
        code_intel: hook,
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    window
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Result<Value, ui::command::CommandError> {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    out
}

#[gpui::test]
fn a_project_gets_the_products_services(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("app.py"), "import os\n").unwrap();
    let fake = FakeCodeIntel::new();
    let mut ruff = ServerStatus::new(ServerId::new("ruff"), "ruff");
    ruff.version = Some("0.15.20".into());
    fake.set_status(vec![ruff]);
    let window = open(cx, Some(CodeIntelHook::new(Provider(fake.clone()))));

    // No project yet: the view exists, without services.
    let state = invoke(cx, window, "code.state", json!(null)).unwrap();
    assert_eq!(state["services"], false);

    let path = root.display().to_string();
    invoke(cx, window, "project.open-folder", json!({ "path": path })).unwrap();
    let state = invoke(cx, window, "code.state", json!(null)).unwrap();
    assert_eq!(state["services"], true);
    let servers = invoke(cx, window, "code.servers", json!(null)).unwrap();
    assert_eq!(servers[0]["id"], "ruff");
    assert_eq!(servers[0]["version"], "0.15.20");

    // Opening, saving and closing a tab reach the view; the commands are
    // there and do nothing yet.
    let file = root.join("app.py").display().to_string();
    invoke(cx, window, "project.open-file", json!({ "path": file })).unwrap();
    invoke(cx, window, "code.complete", json!(null)).unwrap();
    invoke(
        cx,
        window,
        "code.restart-servers",
        json!({ "server": "ruff" }),
    )
    .unwrap();
    invoke(cx, window, "code.stop-server", json!({ "server": "ty" })).unwrap();
    invoke(cx, window, "tabs.close", json!(null)).unwrap();
    assert_eq!(
        fake.calls(),
        [
            Call::Restart(Some(ServerId::new("ruff"))),
            Call::Stop(ServerId::new("ty")),
        ]
    );
    // Popup commands are not available without a popup.
    assert!(invoke(cx, window, "completion.accept", json!(null)).is_err());
}

#[gpui::test]
fn without_services_the_commands_are_not_available(cx: &mut TestAppContext) {
    let window = open(cx, None);
    let error = invoke(cx, window, "code.state", json!(null)).unwrap_err();
    assert_eq!(error.code, "not_available");
    let error = invoke(cx, window, "code.complete", json!(null)).unwrap_err();
    assert_eq!(error.code, "not_available");
}
