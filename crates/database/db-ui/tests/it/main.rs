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

#[allow(dead_code)]
pub fn root_of(h: &mut Harness) -> PathBuf {
    let s = state(h);
    Path::new(s["explorer"]["root"].as_str().unwrap()).to_owned()
}
