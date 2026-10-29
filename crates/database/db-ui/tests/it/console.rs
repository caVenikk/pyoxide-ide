//! SQL consoles over a scripted connection source: a console of a data
//! source opens in an editor tab from the tree, Execute runs the statement
//! at the caret in its own session, the output log gets the lines of the
//! statement and a result tab shows the rows in the grid. The bindings of
//! `keymaps/*/console.toml` are pressed in it as well.

use std::time::{Duration, Instant};

use db_core::Backend;
use db_session::fake::{FakeSource, Reaction};
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::keymap::Binding;
use workbench::test_support::keymap::{Harness, check_area};

use super::grid::{NAME, rows, store_with_model};
use super::*;

fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    source.on(
        "demo.nope",
        Reaction::query_error("42P01", "relation \"demo.nope\" does not exist"),
    );
    source.on(
        "from demo.customers",
        rows(&[
            ("1", "a@example.test"),
            ("2", "b@example.test"),
            ("3", "c@example.test"),
        ]),
    );
    source.on("select 1", Reaction::column("a", &["1"]));
    source.on("insert into", Reaction::Affected(2));
    source
}

pub(super) fn console_state(h: &mut Harness) -> Value {
    h.invoke("db.console.state", Value::Null)["consoles"][0].clone()
}

/// Draws until the console's state satisfies `f`.
pub(super) fn wait_console(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = console_state(h);
        if f(&s) {
            return s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}: {s}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

pub(super) fn idle(s: &Value) -> bool {
    s["running"] == false && s["output"].as_array().is_some_and(|o| !o.is_empty())
}

pub(super) fn output(s: &Value) -> String {
    s["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("")
}

/// Shows the tree with the cached model and opens the console of the
/// data source.
pub(super) fn open_console(h: &mut Harness) -> Value {
    show_tool_window(h);
    h.invoke("db.explorer.expand", json!({ "path": NAME }));
    let started = Instant::now();
    while state(h)["explorer"]["tree"]["data_sources"][NAME]["has_model"] != true {
        assert!(started.elapsed() < Duration::from_secs(10), "the model");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
    h.invoke("db.console.new", json!({ "source": NAME }));
    let started = Instant::now();
    loop {
        let s = console_state(h);
        let open = h.state()["tabs"].to_string().contains("console.sql");
        if !s.is_null() && open {
            return s;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the console");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

pub(super) fn harness<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
    server: &FakeSource,
) -> (Harness<'a>, tempfile::TempDir, tempfile::TempDir) {
    let (store, cache) = store_with_model(root, server);
    let consoles = tempfile::tempdir().unwrap();
    store.set_console_folder(consoles.path());
    let h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(root));
    (h, cache, consoles)
}

#[gpui::test]
fn a_console_runs_the_statement_at_the_caret_and_logs_it(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, consoles) = harness(cx, &root, &server);
    let s = open_console(&mut h);
    assert_eq!(s["source"], NAME);
    assert!(
        consoles.path().join(NAME).join("console.sql").exists(),
        "the console file: {s}"
    );

    h.invoke(
        "editor.type",
        json!({ "text": "select 1;\nselect * from demo.customers;\nselect 3;" }),
    );
    h.invoke("editor.move-up", Value::Null);
    h.invoke("db.console.execute", Value::Null);
    let s = wait_console(&mut h, "the statement", idle);
    let log = output(&s);
    assert!(log.contains("Connected"), "{log}");
    assert!(log.contains("> select * from demo.customers"), "{log}");
    assert!(log.contains("3 rows retrieved starting from 1 in"), "{log}");
    assert!(log.contains("(execution:"), "{log}");
    // Only the statement at the caret ran.
    assert!(!log.contains("select 3"), "{log}");
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["tabs"][0]["title"], "pyoxide.demo.customers", "{s}");
    assert_eq!(s["tabs"][0]["grid"]["rows"], 3, "{s}");
    assert_eq!(s["tabs"][0]["grid"]["columns"][0]["name"], "id", "{s}");

    // The next statement reuses the tab that is not pinned.
    h.invoke("editor.move-document-start", Value::Null);
    h.invoke("db.console.execute", Value::Null);
    let s = wait_console(&mut h, "the second statement", |s| {
        idle(s) && output(s).matches("retrieved starting").count() == 2
    });
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["tabs"][0]["title"], "a:text", "{s}");
}

#[gpui::test]
fn a_failing_statement_is_logged_with_its_error(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select * from demo.nope;" }));
    h.invoke("db.console.execute", Value::Null);
    let s = wait_console(&mut h, "the error", idle);
    let log = output(&s);
    assert!(
        log.contains("[42P01] relation \"demo.nope\" does not exist"),
        "{log}"
    );
    assert!(s["tabs"].as_array().unwrap().is_empty(), "{s}");
}

#[gpui::test]
fn a_statement_without_rows_logs_the_affected_count(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "insert into demo.customers values (1);" }),
    );
    h.invoke("db.console.execute", Value::Null);
    let s = wait_console(&mut h, "the insert", idle);
    let log = output(&s);
    assert!(log.contains("2 rows affected in"), "{log}");
    assert!(s["tabs"].as_array().unwrap().is_empty(), "{s}");
}

/// The group of the editor decorations of an executed statement.
const MARKS: &str = "db-console-run";

fn marks(h: &mut Harness) -> Vec<Value> {
    h.state()["editor"]["decorations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d["group"] == MARKS)
        .collect()
}

#[gpui::test]
fn an_executed_statement_gets_a_check_a_time_and_a_box_until_it_is_edited(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select 1;\nselect * from demo.customers;" }),
    );
    h.invoke("db.console.execute", Value::Null);
    wait_console(&mut h, "the statement", idle);
    let marks_now = marks(&mut h);
    assert_eq!(marks_now.len(), 1, "{marks_now:?}");
    assert_eq!(marks_now[0]["lane_icon"], "db/statementOk", "{marks_now:?}");
    assert_eq!(marks_now[0]["lines"], json!([1, 2]), "{marks_now:?}");
    let time = marks_now[0]["after_text"].as_str().unwrap();
    assert!(time.ends_with(" ms"), "{time}");
    // The statement is boxed: `select * from demo.customers;` after the
    // first line's 10 chars.
    let outline = h.state()["editor"]["range_outline"].clone();
    assert_eq!(outline, json!([10, 38]), "{outline}");

    // A line inserted above moves the marks with the statement.
    h.invoke("editor.move-document-start", Value::Null);
    h.invoke("editor.type", json!({ "text": "\n" }));
    let outline = h.state()["editor"]["range_outline"].clone();
    assert_eq!(outline, json!([11, 39]), "{outline}");
    assert_eq!(marks(&mut h)[0]["lines"], json!([2, 3]));

    // An edit of the statement removes them.
    h.invoke("editor.move-document-end", Value::Null);
    h.invoke("editor.type", json!({ "text": " " }));
    assert_eq!(marks(&mut h).len(), 1, "after the statement: kept");
    h.invoke("editor.move-left", json!({}));
    h.invoke("editor.move-left", json!({}));
    h.invoke("editor.type", json!({ "text": "x" }));
    assert!(marks(&mut h).is_empty());
    assert!(h.state()["editor"]["range_outline"].is_null());
}

#[gpui::test]
fn a_failed_statement_gets_the_error_mark(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select * from demo.nope;" }));
    h.invoke("db.console.execute", Value::Null);
    wait_console(&mut h, "the error", idle);
    let marks_now = marks(&mut h);
    assert_eq!(marks_now[0]["lane_icon"], "db/error", "{marks_now:?}");
}

#[gpui::test]
fn the_console_toolbar_has_the_schema_chip_only_and_the_breadcrumb_names_the_console(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select 1;" }));
    h.invoke("db.console.execute", Value::Null);
    wait_console(&mut h, "the statement", idle);
    let started = Instant::now();
    let sql = loop {
        let sql = h.invoke("sql.state", Value::Null);
        if sql["visible"] == true && sql["scope-origin"]["kind"] == "console" {
            break sql;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the chips: {sql}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    };
    assert!(
        !sql["chips"].to_string().contains(NAME),
        "no chip of the data source: {sql}"
    );
    assert!(sql["data-source-text"].is_null(), "{sql}");
    assert_eq!(
        h.state()["status_path"],
        json!(["Database Consoles", NAME, format!("console [{NAME}]")]),
    );
}

#[gpui::test]
fn a_console_file_saves_itself_and_its_tab_is_never_modified(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select 1;" }));
    let tab = h.state()["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"].to_string().contains("console.sql"))
        .cloned()
        .unwrap();
    assert_eq!(tab["autosave"], true, "{tab}");
    let file = consoles.path().join(NAME).join("console.sql");
    h.cx.executor().advance_clock(Duration::from_millis(500));
    let started = Instant::now();
    while std::fs::read_to_string(&file).unwrap() != "select 1;" {
        assert!(started.elapsed() < Duration::from_secs(10), "the autosave");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn the_services_tree_collapses_and_the_output_toolbar_clears_and_wraps(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers;" }),
    );
    h.invoke("db.console.execute", Value::Null);
    let s = wait_console(&mut h, "the statement", idle);
    // The footer says the rows when the first page holds them all.
    assert_eq!(s["tabs"][0]["grid"]["range"], "3 rows", "{s}");

    h.invoke("db.services.collapse-all", Value::Null);
    let state = h.invoke("db.console.state", Value::Null);
    assert_eq!(state["services"]["collapsed"], json!(["", NAME]), "{state}");
    h.invoke("db.services.expand-all", Value::Null);
    let state = h.invoke("db.console.state", Value::Null);
    assert_eq!(state["services"]["collapsed"], json!([]), "{state}");

    h.invoke("db.console.toggle-output-soft-wrap", Value::Null);
    assert_eq!(console_state(&mut h)["soft_wrap"], true);
    h.invoke("db.console.clear-output", Value::Null);
    assert_eq!(console_state(&mut h)["output"], json!([]));
}

/// The consoles of the window, in the order they were made.
fn all_consoles(h: &mut Harness) -> Vec<Value> {
    h.invoke("db.console.state", Value::Null)["consoles"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Draws until `f` holds for the consoles.
fn wait_consoles(h: &mut Harness, what: &str, f: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    let started = Instant::now();
    loop {
        let all = all_consoles(h);
        if f(&all) {
            return all;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}: {all:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

/// The key of Execute on this platform's keymap.
fn execute_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    }
}

fn open_file(h: &mut Harness, path: &Path) {
    h.invoke(
        "project.open-file",
        json!({ "path": path.display().to_string() }),
    );
}

fn services_visible(h: &mut Harness) -> bool {
    h.state()["tool_windows"]["services"]["visible"] == true
}

fn hide_services(h: &mut Harness) {
    // The first toggle focuses a tool window that shows, the second hides
    // it and gives the focus back to the editor.
    for _ in 0..2 {
        if services_visible(h) {
            h.invoke("workbench.toggle-tool-window", json!({"id": "services"}));
        }
    }
    assert!(!services_visible(h));
}

/// Draws until the SQL toolbar shows the file's data source.
fn wait_attached(h: &mut Harness) {
    let started = Instant::now();
    while h.invoke("sql.state", Value::Null)["data-source"] != NAME {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the attachment"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn execute_in_an_attached_file_runs_that_files_statement_in_its_own_session(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    std::fs::write(root.join("scratch.sql"), "select * from demo.customers").unwrap();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select * from demo.nope;" }));

    open_file(&mut h, &root.join("scratch.sql"));
    h.invoke("sql.attach-data-source", json!({ "name": NAME }));
    wait_attached(&mut h);
    // The file has the toolbar's Execute, as an attached file has it.
    h.press(execute_key());
    let all = wait_consoles(&mut h, "the file's console", |all| {
        all.len() == 2 && idle(&all[1])
    });
    // The console did not run its text again: the file's statement ran in
    // a session of its own, with its own node and output.
    let console_log = output(&all[0]);
    assert!(!console_log.contains("nope"), "{console_log}");
    assert_eq!(all[0]["file"], false);
    assert!(all[1]["path"].as_str().unwrap().ends_with("scratch.sql"));
    assert_eq!(all[1]["file"], true);
    assert_eq!(all[1]["label"], "scratch.sql");
    assert_eq!(all[1]["source"], NAME);
    let log = output(&all[1]);
    assert!(log.contains("3 rows retrieved starting from 1 in"), "{log}");
    assert!(!log.contains("nope"), "{log}");
    assert_eq!(all[1]["tabs"][0]["title"], "pyoxide.demo.customers");
    assert_eq!(all[1]["tabs"][0]["grid"]["rows"], 3);
    assert!(services_visible(&mut h));

    // The file stays an ordinary file: no console binding, no autosave.
    let sql = h.invoke("sql.state", Value::Null);
    assert_eq!(sql["scope-origin"]["kind"], "mapping", "{sql}");

    // Another run goes to the same session; Services shows again after it
    // was closed.
    hide_services(&mut h);
    h.press(execute_key());
    let all = wait_consoles(&mut h, "the second run", |all| {
        all.len() == 2 && idle(&all[1]) && output(&all[1]).matches("retrieved").count() == 2
    });
    assert!(!output(&all[0]).contains("nope"), "{all:?}");
    assert!(services_visible(&mut h));

    // Back in the console, the key runs the console's statement.
    open_file(&mut h, &root.join("scratch.sql"));
    h.invoke(
        "project.open-file",
        json!({ "path": all[0]["path"].as_str().unwrap() }),
    );
    h.press(execute_key());
    let all = wait_consoles(&mut h, "the console's run", |all| {
        idle(&all[0]) && output(&all[0]).contains("demo.nope")
    });
    assert_eq!(all.len(), 2);
    assert_eq!(output(&all[1]).matches("retrieved").count(), 2);
}

#[gpui::test]
fn the_execute_key_does_nothing_in_other_editors_and_never_shows_services(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    std::fs::write(root.join("main.py"), "print(1)\n").unwrap();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers" }),
    );
    h.press(execute_key());
    wait_console(&mut h, "the console's statement", idle);
    let before = all_consoles(&mut h);
    hide_services(&mut h);

    open_file(&mut h, &root.join("main.py"));
    assert_eq!(
        h.state()["tabs"].as_array().unwrap().len(),
        2,
        "the Python file is open"
    );
    h.press(execute_key());
    h.press(execute_key());
    assert_eq!(all_consoles(&mut h), before, "no console ran or changed");
    assert!(!services_visible(&mut h), "Services stays closed");
    // The key did not type or do anything to the Python text either.
    assert_eq!(h.invoke("editor.text", Value::Null), json!("print(1)\n"));
}

#[gpui::test]
fn the_execute_key_in_a_file_without_a_data_source_asks_for_one(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    std::fs::write(root.join("free.sql"), "select 1").unwrap();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_file(&mut h, &root.join("free.sql"));
    let started = Instant::now();
    while h.invoke("sql.state", Value::Null)["visible"] != true {
        assert!(started.elapsed() < Duration::from_secs(10), "the toolbar");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
    assert_eq!(h.invoke("sql.state", Value::Null)["popup"], Value::Null);
    h.press(execute_key());
    let sql = h.invoke("sql.state", Value::Null);
    assert!(!sql["popup"].is_null(), "the data source popup: {sql}");
    assert!(all_consoles(&mut h).is_empty(), "nothing ran");
    assert!(!services_visible(&mut h));
}

#[gpui::test]
fn the_sort_mark_of_a_result_orders_its_rows_and_the_third_click_restores_them(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers" }),
    );
    h.press(execute_key());
    let s = wait_console(&mut h, "the result", idle);
    let emails = |s: &Value| -> Vec<String> {
        s["tabs"][0]["grid"]["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[1].as_str().unwrap().to_owned())
            .collect()
    };
    let original = vec!["a@example.test", "b@example.test", "c@example.test"];
    assert_eq!(emails(&s), original);
    assert_eq!(s["tabs"][0]["grid"]["sort"], json!([]));

    h.invoke("db.grid.sort", json!({ "column": 1 }));
    let s = console_state(&mut h);
    assert_eq!(emails(&s), original);
    assert_eq!(
        s["tabs"][0]["grid"]["sort"],
        json!([{"column": 1, "desc": false}])
    );
    h.invoke("db.grid.sort", json!({ "column": 1 }));
    let s = console_state(&mut h);
    assert_eq!(
        emails(&s),
        vec!["c@example.test", "b@example.test", "a@example.test"]
    );
    assert_eq!(
        s["tabs"][0]["grid"]["sort"],
        json!([{"column": 1, "desc": true}])
    );
    // A statement was not run again for it.
    assert_eq!(output(&s).matches("retrieved").count(), 1, "{s}");
    h.invoke("db.grid.sort", json!({ "column": 1 }));
    let s = console_state(&mut h);
    assert_eq!(emails(&s), original);
    assert_eq!(s["tabs"][0]["grid"]["sort"], json!([]));
}

fn run(cx: &mut TestAppContext, macos: bool) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let consoles = tempfile::tempdir().unwrap();
    store.set_console_folder(consoles.path());
    let area = if macos {
        include_str!("../../keymaps/macos/console.toml")
    } else {
        include_str!("../../keymaps/default/console.toml")
    };
    check_area(
        cx,
        product(store),
        macos,
        area,
        Some(&root),
        |h, b: &Binding| match b.command.as_str() {
            "db.console.execute" => {
                if console_state(h).is_null() {
                    open_console(h);
                    h.invoke(
                        "editor.type",
                        json!({ "text": "select * from demo.customers" }),
                    );
                }
                h.press(&b.keys);
                let s = wait_console(h, "the key's statement", idle);
                assert!(output(&s).contains("3 rows retrieved"), "{s}");
                true
            }
            _ => false,
        },
    );
}

#[gpui::test]
fn every_macos_console_binding_works(cx: &mut TestAppContext) {
    run(cx, true);
}

#[gpui::test]
fn every_windows_and_linux_console_binding_works(cx: &mut TestAppContext) {
    run(cx, false);
}
