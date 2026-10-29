//! Settings > Tools > Database > Query Execution: the page (its place in the
//! tree, the Execute tabs, the stored options) and what the options change
//! in a console: what Execute runs for the caret and for a selection, the
//! question it may ask, and the result tab.

use std::time::{Duration, Instant};

use db_core::{Backend, Purpose};
use db_session::fake::{FakeSource, Reaction};
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::console::{console_state, harness, idle, open_console, wait_console};
use super::*;

const PAGE: &str = "tools.database.query-execution";
const KEY: &str = "database.query-execution";

fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    source.on("select 1", Reaction::column("a", &["1"]));
    source.on("select 2", Reaction::column("b", &["2"]));
    source
}

fn set(h: &mut Harness, key: &str, value: Value) {
    h.invoke(
        "settings.set",
        json!({ "key": format!("{KEY}.{key}"), "value": value }),
    );
}

fn page(h: &mut Harness) -> Value {
    h.invoke("settings.state", Value::Null)["fields"].clone()
}

/// The statements the console sent, without what its session sends itself
/// (the transaction mode, the current schema).
fn ran(server: &FakeSource) -> Vec<String> {
    server
        .statements(Purpose::Console)
        .into_iter()
        .filter(|q| !q.starts_with("<tx") && !q.starts_with("SELECT current_schema"))
        .collect()
}

/// Draws for a while so that something that should not happen could.
fn settle(h: &mut Harness) {
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(5));
        h.draw();
    }
}

fn wait_ran(h: &mut Harness, server: &FakeSource, count: usize) {
    let started = Instant::now();
    while ran(server).len() < count {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {count} statements, got {:?}",
            ran(server)
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
    settle(h);
}

fn popup(h: &mut Harness) -> Value {
    h.invoke("sql.state", Value::Null)["popup"].clone()
}

fn labels(popup: &Value) -> Vec<String> {
    popup["levels"][0]["items"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| r["label"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[gpui::test]
fn the_page_is_under_tools_and_database_with_the_references_defaults(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    h.invoke("settings.open", json!({ "page": PAGE }));
    let s = h.invoke("settings.state", Value::Null);
    assert_eq!(s["page"], PAGE, "{s}");
    assert_eq!(
        s["breadcrumbs"],
        json!(["Tools", "Database", "Query Execution"]),
        "{s}"
    );
    let f = &s["fields"];
    assert_eq!(f["tab"], "execute");
    for tab in ["execute", "execute-2", "execute-3"] {
        assert_eq!(f[tab]["caret-inside"], "ask", "{f}");
        assert_eq!(f[tab]["caret-outside"], "nothing", "{f}");
        assert_eq!(f[tab]["for-selection"], "separate", "{f}");
        assert_eq!(f[tab]["new-tab"], false, "{f}");
    }
    assert_eq!(f["split-script"], "ansi-or-separator");
    assert_eq!(f["review-parameters"], true);
    assert_eq!(f["show-execution-time"], true);
    assert_eq!(f["warn-unsafe-queries"], true);
    // The keymap's shortcut of Execute; the others have none.
    assert!(
        f["execute"]["shortcut"].is_string() == cfg!(target_os = "macos"),
        "{f}"
    );
    assert!(f["execute-2"]["shortcut"].is_null(), "{f}");
}

#[gpui::test]
fn edits_are_drafts_until_apply_and_each_tab_keeps_its_own(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    h.invoke("settings.open", json!({ "page": PAGE }));
    h.invoke(
        "settings.edit",
        json!({ "field": "execute-2.caret-inside", "value": "largest" }),
    );
    h.invoke(
        "settings.edit",
        json!({ "field": "execute-2.new-tab", "value": true }),
    );
    h.invoke(
        "settings.edit",
        json!({ "field": "execute.for-selection", "value": "one" }),
    );
    h.invoke(
        "settings.edit",
        json!({ "field": "execute-3.caret-outside", "value": "ask" }),
    );
    let s = h.invoke("settings.state", Value::Null);
    assert_eq!(s["modified"], json!([PAGE]), "{s}");
    assert_eq!(s["fields"]["execute-2"]["caret-inside"], "largest");
    assert_eq!(s["fields"]["execute"]["caret-inside"], "ask");
    let before = h.invoke("settings.get", json!({ "key": KEY }));
    assert_eq!(
        before["execute-2"]["caret-inside"], "ask",
        "nothing is stored before Apply: {before}"
    );

    // The tab choice is the page's, not a setting.
    h.invoke(
        "settings.edit",
        json!({ "field": "tab", "value": "execute-2" }),
    );
    h.invoke(
        "settings.edit",
        json!({ "field": "split-script", "value": "ansi-or-separator" }),
    );
    h.invoke(
        "settings.edit",
        json!({ "field": "warn-unsafe-queries", "value": false }),
    );
    h.invoke("settings.apply", Value::Null);
    let stored = h.invoke("settings.get", json!({ "key": KEY }));
    assert_eq!(stored["execute-2"]["caret-inside"], "largest", "{stored}");
    assert_eq!(stored["execute-2"]["new-tab"], true, "{stored}");
    assert_eq!(stored["execute"]["for-selection"], "one", "{stored}");
    assert_eq!(stored["execute-3"]["caret-outside"], "ask", "{stored}");
    assert_eq!(stored["warn-unsafe-queries"], false, "{stored}");
    let s = h.invoke("settings.state", Value::Null);
    assert_eq!(s["modified"], json!([]), "{s}");

    h.invoke("settings.revert", Value::Null);
    assert_eq!(page(&mut h)["execute-2"]["caret-inside"], "largest");
}

#[gpui::test]
fn a_subquery_asks_what_to_execute_and_the_answer_runs(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from (select 1) x;" }),
    );
    for _ in 0..4 {
        h.invoke("editor.move-left", Value::Null);
    }
    h.invoke("db.console.execute", Value::Null);
    let started = Instant::now();
    let asked = loop {
        h.draw();
        let p = popup(&mut h);
        if !p.is_null() {
            break p;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the popup");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        labels(&asked),
        ["select 1", "select * from (select 1) x"],
        "{asked}"
    );
    assert!(ran(&server).is_empty(), "nothing runs before the answer");

    h.invoke("db.console.execute-choice", json!({ "index": 0 }));
    wait_ran(&mut h, &server, 1);
    assert_eq!(ran(&server), ["select 1"]);
    wait_console(&mut h, "the result", idle);
}

#[gpui::test]
fn a_plain_statement_runs_at_once_with_ask(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select 1;\nselect 2;" }));
    h.invoke("editor.move-up", Value::Null);
    h.invoke("db.console.execute", Value::Null);
    wait_ran(&mut h, &server, 1);
    assert_eq!(ran(&server), ["select 1"]);
    assert!(popup(&mut h).is_null());
}

#[gpui::test]
fn smallest_and_largest_pick_without_asking(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from (select 1) x;" }),
    );
    for _ in 0..4 {
        h.invoke("editor.move-left", Value::Null);
    }
    set(&mut h, "execute.caret-inside", json!("smallest"));
    h.invoke("db.console.execute", Value::Null);
    wait_ran(&mut h, &server, 1);
    assert_eq!(ran(&server), ["select 1"]);
    wait_console(&mut h, "the first run", idle);

    set(&mut h, "execute.caret-inside", json!("largest"));
    h.invoke("db.console.execute", Value::Null);
    wait_ran(&mut h, &server, 2);
    assert_eq!(ran(&server)[1], "select * from (select 1) x");
    assert!(popup(&mut h).is_null());
}

#[gpui::test]
fn between_statements_the_options_say_nothing_closest_or_ask(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select 1;\n\n\n\nselect 2;" }),
    );
    h.invoke("editor.move-document-start", Value::Null);
    h.invoke("editor.move-down", Value::Null);
    h.invoke("editor.move-down", Value::Null);

    h.invoke("db.console.execute", Value::Null);
    settle(&mut h);
    assert!(ran(&server).is_empty(), "Nothing: {:?}", ran(&server));

    set(&mut h, "execute.caret-outside", json!("closest"));
    h.invoke("db.console.execute", Value::Null);
    wait_ran(&mut h, &server, 1);
    assert_eq!(ran(&server), ["select 1"]);
    wait_console(&mut h, "the run", idle);

    set(&mut h, "execute.caret-outside", json!("ask"));
    h.invoke("db.console.execute", Value::Null);
    let started = Instant::now();
    let asked = loop {
        h.draw();
        let p = popup(&mut h);
        if !p.is_null() {
            break p;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the popup");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(labels(&asked), ["select 1", "All statements"], "{asked}");
    h.invoke("db.console.execute-choice", json!({ "index": 1 }));
    wait_ran(&mut h, &server, 3);
    assert_eq!(ran(&server)[1..], ["select 1", "select 2"]);
}

#[gpui::test]
fn a_selection_runs_as_separate_statements_or_as_one(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select 1;\nselect 2;" }));
    h.invoke("editor.select-all", Value::Null);
    h.invoke("db.console.execute", Value::Null);
    wait_ran(&mut h, &server, 2);
    assert_eq!(ran(&server), ["select 1", "select 2"]);
    wait_console(&mut h, "the script", |s| {
        idle(s) && s["running"] == false && s["tabs"].as_array().is_some_and(|t| !t.is_empty())
    });

    set(&mut h, "execute.for-selection", json!("one"));
    h.invoke("db.console.execute", Value::Null);
    wait_ran(&mut h, &server, 3);
    assert_eq!(ran(&server)[2], "select 1;\nselect 2");
}

#[gpui::test]
fn results_open_in_a_new_tab_only_where_the_action_says_so(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.type", json!({ "text": "select 1" }));
    let tabs = |h: &mut Harness| console_state(h)["tabs"].as_array().unwrap().len();
    let run = |h: &mut Harness, command: &str, total: usize| {
        h.invoke(command, Value::Null);
        wait_ran(h, &server, total);
        wait_console(h, "the run", |s| {
            s["running"] == false && s["tabs"].as_array().is_some_and(|t| !t.is_empty())
        });
    };

    run(&mut h, "db.console.execute", 1);
    run(&mut h, "db.console.execute", 2);
    assert_eq!(tabs(&mut h), 1, "reused by default");

    // Execute (2) has the option; Execute still reuses.
    set(&mut h, "execute-2.new-tab", json!(true));
    run(&mut h, "db.console.execute-2", 3);
    assert_eq!(tabs(&mut h), 2);
    run(&mut h, "db.console.execute-2", 4);
    assert_eq!(tabs(&mut h), 3);
    run(&mut h, "db.console.execute", 5);
    assert_eq!(tabs(&mut h), 3, "Execute reuses the first tab");
}
