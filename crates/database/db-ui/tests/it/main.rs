//! The database module's interface in a headless main window: the key
//! bindings of `keymaps/*/db.toml` (`keymap`) and the Database tool window
//! with the Data Sources dialog over a fake store (`data_sources`).

mod data_sources;
mod keymap;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use db_core::{Backend, Profile};
use db_ui::store::fake::FakeStore;
use serde_json::{Value, json};
use workbench::Product;
use workbench::test_support::keymap::Harness;

/// A product with only the database module, over `store`.
pub fn product(store: Arc<FakeStore>) -> Product {
    Product {
        name: "Test IDE",
        ..Product::default()
    }
    .with_module(db_ui::module(db_ui::DbUiConfig { store }))
}

/// A project folder (its real path: the workbench opens that one).
pub fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = native_path::canonicalize(dir.path()).unwrap();
    (dir, root)
}

/// A saved PostgreSQL data source.
pub fn pg() -> Profile {
    let mut p = Profile::new(Backend::Postgres);
    p.host = "localhost".into();
    p.port = Some(55432);
    p.database = Some("pyoxide".into());
    p.user = "pyoxide".into();
    p
}

/// `db.state`.
pub fn state(h: &mut Harness) -> Value {
    h.invoke("db.state", Value::Null)
}

/// Shows the Database tool window with the focus in it.
pub fn show_tool_window(h: &mut Harness) {
    h.invoke("workbench.activate-tool-window", json!({"id": "database"}));
}

/// Runs a command in the open dialog and draws.
pub fn dialog(h: &mut Harness, id: &str, args: Value) {
    h.invoke(id, args);
}

pub fn set(h: &mut Harness, field: &str, value: &str) {
    dialog(
        h,
        "db.data-sources.set",
        json!({"field": field, "value": value}),
    );
}

/// Chooses a row of the dialog's open list: the row's command runs in the
/// dialog, as a click on it does (commands sent to the window go to the
/// popup over the dialog).
pub fn choose_in_list(h: &mut Harness, list: &str, value: &str) {
    use gpui::AppContext as _;
    let main: gpui::AnyWindowHandle = h.window.into();
    let dialog =
        h.cx.update(|cx| ui::dialog_window::modal_of(main, cx))
            .expect("the dialog");
    let action = db_ui::dialog::DialogSet {
        field: list.to_owned(),
        value: value.to_owned(),
    };
    h.cx.update_window(dialog, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .unwrap();
    h.draw();
}
