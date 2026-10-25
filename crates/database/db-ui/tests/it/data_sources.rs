//! The Database tool window and the Data Sources dialog over a fake store
//! (scenarios `tests/scenarios/db-gui/data-source-dialog.md` and
//! `data-source-options.md`, the parts without a server).

use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::*;

const SECRET: &str = "correct-horse-battery";

fn open<'a>(cx: &'a mut TestAppContext, store: Arc<FakeStore>, root: &Path) -> Harness<'a> {
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(root));
    show_tool_window(&mut h);
    h
}

/// A new project shows the empty state; its tiles and New open the dialog
/// with a new data source of that backend and the reference's defaults.
#[gpui::test]
fn the_empty_state_creates_a_data_source(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    let mut h = open(cx, store, &root);
    let s = state(&mut h);
    assert_eq!(s["explorer"]["root"], root.display().to_string());
    assert_eq!(s["explorer"]["loaded"], true);
    assert_eq!(s["explorer"]["sources"], json!([]));

    h.invoke("db.explorer.new-data-source", Value::Null);
    let menu = &state(&mut h)["explorer"]["menu"];
    assert!(!menu.is_null(), "the New menu opens");

    h.invoke(
        "db.explorer.add-data-source",
        json!({"backend": "postgres"}),
    );
    let s = state(&mut h);
    let item = &s["dialog"]["selected"];
    assert_eq!(item["name"], "postgres@localhost");
    assert_eq!(item["backend"], "postgres");
    assert_eq!(item["new"], true);
    assert_eq!(item["profile"]["host"], "localhost");
    assert_eq!(item["profile"]["port"], 5432);
    assert_eq!(item["profile"]["database"], "postgres");
    assert_eq!(item["url"], "postgresql://localhost:5432/postgres");
    assert_eq!(item["password"]["placeholder"], "<hidden>");
    assert_eq!(s["dialog"]["tab"], "general");
    assert_eq!(s["dialog"]["test"]["state"], "idle");
}

/// The URL follows the fields, and editing the URL sets the fields.
#[gpui::test]
fn the_url_and_the_fields_follow_each_other(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, FakeStore::new(SECRET), &root);
    h.invoke(
        "db.explorer.add-data-source",
        json!({"backend": "postgres"}),
    );
    set(&mut h, "port", "55432");
    set(&mut h, "database", "pyoxide");
    set(&mut h, "user", "app");
    let s = state(&mut h);
    assert_eq!(
        s["dialog"]["selected"]["url"],
        "postgresql://app@localhost:55432/pyoxide"
    );

    set(&mut h, "url", "postgresql://db.example.com:6543/sales");
    let p = state(&mut h)["dialog"]["selected"]["profile"].clone();
    assert_eq!(p["host"], "db.example.com");
    assert_eq!(p["port"], 6543);
    assert_eq!(p["database"], "sales");

    // A URL of another database is reported and changes nothing.
    set(&mut h, "url", "mysql://x");
    let s = state(&mut h);
    assert!(s["dialog"]["selected"]["url_error"].is_string(), "{s}");
    assert_eq!(s["dialog"]["selected"]["profile"]["host"], "db.example.com");
}

/// A wrong password reports the server's reason and a hint and saves
/// nothing; the right one shows the server's version. The password never
/// shows in the state.
#[gpui::test]
fn test_connection_reports_the_reason_and_the_version(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    let mut h = open(cx, store.clone(), &root);
    h.invoke(
        "db.explorer.add-data-source",
        json!({"backend": "postgres"}),
    );
    set(&mut h, "user", "pyoxide");
    set(&mut h, "password", "wrong-password");
    dialog(&mut h, "db.data-sources.test-connection", Value::Null);
    let s = state(&mut h);
    assert_eq!(s["dialog"]["test"]["state"], "failed", "{s}");
    assert_eq!(s["dialog"]["test"]["auth"], true);
    let text = s["dialog"]["failure"]["text"].as_str().unwrap();
    assert!(
        text.contains("28P01") && text.contains("password"),
        "{text}"
    );
    assert!(text.contains("The server refused"), "{text}");
    assert_eq!(store.saves(), 0, "a test saves nothing");
    assert!(store.sources(&root).is_empty());

    // Change Credentials: the message closes.
    dialog(&mut h, "db.data-sources.change-credentials", Value::Null);
    assert!(state(&mut h)["dialog"]["failure"].is_null());

    set(&mut h, "password", SECRET);
    dialog(&mut h, "db.data-sources.test-connection", Value::Null);
    let s = state(&mut h);
    assert_eq!(s["dialog"]["test"]["state"], "ok", "{s}");
    assert_eq!(s["dialog"]["test"]["version"], "18.6");
    assert_eq!(s["dialog"]["result_shown"], true);
    let result = s["dialog"]["result"].as_str().unwrap();
    assert!(result.contains("PostgreSQL 18.6"), "{result}");
    assert_eq!(store.saves(), 0);

    let dump = s.to_string();
    assert!(!dump.contains(SECRET) && !dump.contains("wrong-password"));
}

/// A test against a server that does not answer leaves the window
/// working, and Test Connection again cancels it.
#[gpui::test]
fn a_hanging_test_is_cancelled(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::hanging();
    let mut h = open(cx, store.clone(), &root);
    h.invoke(
        "db.explorer.add-data-source",
        json!({"backend": "postgres"}),
    );
    set(&mut h, "user", "pyoxide");
    set(&mut h, "password", "x");
    dialog(&mut h, "db.data-sources.test-connection", Value::Null);
    assert_eq!(state(&mut h)["dialog"]["test"]["state"], "running");
    // The dialog still takes commands meanwhile.
    dialog(&mut h, "db.data-sources.tab", json!({"tab": "options"}));
    assert_eq!(state(&mut h)["dialog"]["tab"], "options");
    dialog(&mut h, "db.data-sources.test-connection", Value::Null);
    assert_eq!(state(&mut h)["dialog"]["test"]["state"], "idle");
    h.draw();
    assert_eq!(store.cancelled(), 1, "the connection was given up");
}

/// OK saves the data sources and keeps the password in the store, never in
/// the profile; the tool window lists them. With "Never" the password is
/// not kept, and removing a data source deletes its password.
#[gpui::test]
fn ok_saves_and_remove_deletes_the_secrets(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    let mut h = open(cx, store.clone(), &root);
    h.invoke(
        "db.explorer.add-data-source",
        json!({"backend": "postgres"}),
    );
    set(&mut h, "name", "pyoxide-local");
    set(&mut h, "user", "pyoxide");
    set(&mut h, "password", SECRET);
    dialog(&mut h, "db.data-sources.ok", Value::Null);
    let s = state(&mut h);
    assert!(s["dialog"].is_null(), "OK closes the dialog: {s}");
    assert_eq!(store.password("pyoxide-local").as_deref(), Some(SECRET));
    let saved = store.sources(&root);
    assert_eq!(saved.len(), 1);
    let toml = format!("{:?}", saved[0].profile);
    assert!(!toml.contains(SECRET));
    assert_eq!(s["explorer"]["sources"][0]["name"], "pyoxide-local");

    // Properties: the saved one, its password kept and not shown.
    h.invoke("db.explorer.properties", Value::Null);
    let s = state(&mut h);
    assert_eq!(s["dialog"]["selected"]["name"], "pyoxide-local");
    assert_eq!(s["dialog"]["selected"]["has_kept_password"], true);
    assert_eq!(s["dialog"]["selected"]["password"]["edited"], false);

    // "Never": Apply drops the kept password.
    set(&mut h, "save", "never");
    dialog(&mut h, "db.data-sources.apply", Value::Null);
    assert_eq!(store.password("pyoxide-local"), None);
    set(&mut h, "save", "forever");
    set(&mut h, "password", SECRET);
    dialog(&mut h, "db.data-sources.apply", Value::Null);
    assert_eq!(store.password("pyoxide-local").as_deref(), Some(SECRET));

    // Remove: the data source and its password go.
    dialog(&mut h, "db.data-sources.remove", Value::Null);
    assert_eq!(state(&mut h)["dialog"]["removed"], json!(["pyoxide-local"]));
    dialog(&mut h, "db.data-sources.ok", Value::Null);
    assert!(store.sources(&root).is_empty());
    assert_eq!(store.password("pyoxide-local"), None);
    assert_eq!(state(&mut h)["explorer"]["sources"], json!([]));
}

/// "Never" asks for the password on Test Connection; the answer is used
/// for the test and kept nowhere.
#[gpui::test]
fn never_asks_for_the_password(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    let mut h = open(cx, store.clone(), &root);
    h.invoke(
        "db.explorer.add-data-source",
        json!({"backend": "postgres"}),
    );
    set(&mut h, "user", "pyoxide");
    set(&mut h, "save", "never");
    dialog(&mut h, "db.data-sources.test-connection", Value::Null);
    let s = state(&mut h);
    assert_eq!(s["prompt"]["user"], "pyoxide", "{s}");
    h.invoke("db.password-prompt.set", json!({"value": SECRET}));
    h.invoke("db.password-prompt.ok", Value::Null);
    let s = state(&mut h);
    assert!(s["prompt"].is_null() || s["prompt"]["field"].is_object());
    assert_eq!(s["dialog"]["test"]["state"], "ok", "{s}");
    assert_eq!(store.password("postgres@localhost"), None);
    assert!(!s.to_string().contains(SECRET));
}

/// A SQLite data source has only a file; without one the test says so.
/// An invalid name keeps OK from saving.
#[gpui::test]
fn sqlite_has_a_file_and_names_are_checked(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    let mut h = open(cx, store.clone(), &root);
    h.invoke("db.explorer.add-data-source", json!({"backend": "sqlite"}));
    let s = state(&mut h);
    assert_eq!(s["dialog"]["selected"]["backend"], "sqlite");
    assert_eq!(s["dialog"]["selected"]["name"], "sqlite");
    dialog(&mut h, "db.data-sources.test-connection", Value::Null);
    let s = state(&mut h);
    assert_eq!(s["dialog"]["test"]["state"], "failed");
    assert!(
        s["dialog"]["test"]["message"]
            .as_str()
            .unwrap()
            .contains("path")
    );

    set(&mut h, "path", "/data/app.db");
    set(&mut h, "name", "bad name");
    dialog(&mut h, "db.data-sources.ok", Value::Null);
    let s = state(&mut h);
    assert!(s["dialog"]["save_error"].is_string(), "{s}");
    assert_eq!(store.saves(), 0);
    set(&mut h, "name", "app");
    dialog(&mut h, "db.data-sources.ok", Value::Null);
    let saved = store.sources(&root);
    assert_eq!(saved[0].name, "app");
    assert_eq!(saved[0].profile.path.as_deref(), Some("/data/app.db"));
}

/// The Options tab: read-only, transaction mode, startup script, time
/// zone, keep-alive and auto-disconnect are kept in the profile's IDE
/// options.
#[gpui::test]
fn options_are_kept_with_the_profile(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    store.add(&root, "local", pg());
    let mut h = open(cx, store.clone(), &root);
    h.invoke("db.explorer.properties", Value::Null);
    dialog(&mut h, "db.data-sources.tab", json!({"tab": "options"}));
    set(&mut h, "read-only", "true");
    set(&mut h, "tx-mode", "manual");
    set(&mut h, "startup-script", "set application_name = 'x'");
    set(&mut h, "time-zone", "UTC");
    set(&mut h, "keep-alive", "true");
    set(&mut h, "auto-disconnect", "true");
    set(&mut h, "auto-disconnect-secs", "600");
    set(&mut h, "level", "names");
    dialog(&mut h, "db.data-sources.ok", Value::Null);
    let p = &store.sources(&root)[0].profile;
    assert!(p.read_only);
    let gui = p.gui_options();
    assert_eq!(gui.tx_mode, Some(db_core::TxMode::Manual));
    assert_eq!(
        gui.startup_script.as_deref(),
        Some("set application_name = 'x'")
    );
    assert_eq!(gui.time_zone.as_deref(), Some("UTC"));
    assert_eq!(gui.keep_alive_secs, Some(60));
    assert_eq!(gui.auto_disconnect_secs, Some(600));
    assert_eq!(gui.level, Some(db_core::schema::Level::Names));
}

/// The SSH/SSL tab sets the TLS options; the Schemas tab reads the schemas
/// and sets the introspection scope.
#[gpui::test]
fn ssl_and_schemas(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    store.add(&root, "local", pg());
    let mut h = open(cx, store.clone(), &root);
    h.invoke("db.explorer.properties", Value::Null);
    dialog(&mut h, "db.data-sources.tab", json!({"tab": "ssh-ssl"}));
    set(&mut h, "use-ssl", "true");
    set(&mut h, "ssl-mode", "verify-full");
    set(&mut h, "ca-file", "/etc/ca.pem");

    set(&mut h, "password", SECRET);
    dialog(&mut h, "db.data-sources.tab", json!({"tab": "schemas"}));
    let s = state(&mut h);
    let names: Vec<_> = s["dialog"]["selected"]["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        ["demo", "information_schema", "pg_catalog", "public"]
    );
    set(&mut h, "schema:demo", "true");
    set(&mut h, "show-system", "true");
    dialog(&mut h, "db.data-sources.ok", Value::Null);
    let p = &store.sources(&root)[0].profile;
    let tls = p.tls.clone().unwrap();
    assert_eq!(tls.mode, db_core::TlsMode::VerifyFull);
    assert_eq!(tls.ca_file.as_deref(), Some("/etc/ca.pem"));
    let scope = p.gui_options().scope.unwrap();
    assert_eq!(
        scope,
        db_core::schema::Scope::list(["demo", "public"]).with_system(true)
    );
}

/// Schemas that cannot be read show the error in the banner.
#[gpui::test]
fn a_failed_schema_read_shows_the_banner(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let store = FakeStore::new(SECRET);
    store.add(&root, "local", pg());
    let mut h = open(cx, store, &root);
    h.invoke("db.explorer.properties", Value::Null);
    set(&mut h, "password", "wrong");
    dialog(&mut h, "db.data-sources.tab", json!({"tab": "schemas"}));
    let s = state(&mut h);
    let banner = s["dialog"]["banner"].as_str().unwrap();
    assert!(banner.contains("28P01"), "{banner}");
}

/// Import from a PyCharm project adds its data sources.
#[gpui::test]
fn import_adds_data_sources(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let mut h = open(cx, FakeStore::new(SECRET), &root);
    h.invoke("db.explorer.import-data-sources", Value::Null);
    let s = state(&mut h);
    assert_eq!(s["explorer"]["sources"][0]["name"], "imported", "{s}");
}
