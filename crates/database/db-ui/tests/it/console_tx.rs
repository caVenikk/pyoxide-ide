//! A console's transactions, schema and safety checks over a scripted
//! connection source (scenarios console-transactions, console-schema-switch
//! and console-safety-and-parameters): the transaction popup and its
//! commands, Commit and Rollback with the pending count, the question when
//! the mode switches to auto with changes pending, the schema switch, the
//! unsafe-query banner and the Parameters dialog with bound values.

use std::time::{Duration, Instant};

use db_core::{Backend, Purpose, Value as Db};
use db_session::fake::{FakeSource, Reaction};
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::console::{console_state, harness, idle, open_console, output, wait_console};
use super::grid::NAME;
use super::*;

fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    source.on(
        "select 1/0",
        Reaction::query_error("22012", "division by zero"),
    );
    source.on("insert into", Reaction::Affected(1));
    source.on("delete from", Reaction::Affected(5));
    source.on("select 1", Reaction::column("a", &["1"]));
    source.on(
        "current_schema",
        Reaction::column("current_schema", &["public"]),
    );
    source
}

fn tx(s: &Value) -> Value {
    s["tx"].clone()
}

/// Draws until `f` holds of the state of the console.
fn wait_for(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    wait_console(h, what, f)
}

fn run_all(h: &mut Harness, text: &str) {
    h.invoke("editor.select-all", Value::Null);
    h.invoke("editor.type", json!({ "text": text }));
    h.invoke("editor.select-all", Value::Null);
    h.invoke("db.console.execute", Value::Null);
}

fn decision(h: &mut Harness) -> Value {
    h.invoke("db.console.state", Value::Null)["tx_decision"].clone()
}

fn wait_decision(h: &mut Harness) -> Value {
    let started = Instant::now();
    loop {
        let d = decision(h);
        if !d.is_null() {
            return d;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the question");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn manual_mode_keeps_the_transaction_through_a_failed_statement_and_counts_what_is_pending(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    let s = open_console(&mut h);
    assert_eq!(tx(&s)["mode"], "auto", "{s}");

    h.invoke("db.console.set-tx-mode", json!({ "mode": "manual" }));
    let s = wait_for(&mut h, "manual mode", |s| tx(s)["mode"] == "manual");
    assert!(output(&s).contains("manual transaction mode ON"), "{s}");

    run_all(
        &mut h,
        "insert into t values (1);\nselect 1/0;\ninsert into t values (2);",
    );
    let s = wait_for(&mut h, "the script", idle_after_error);
    // The failure stopped the script, the first insert is still pending and
    // the transaction continues.
    assert_eq!(tx(&s)["pending"], 1, "{s}");
    assert_eq!(tx(&s)["open"], true, "{s}");
    assert!(output(&s).contains("division by zero"), "{s}");

    h.invoke("db.console.commit", Value::Null);
    let s = wait_for(&mut h, "the commit", |s| tx(s)["pending"] == 0);
    assert!(output(&s).contains("committed"), "{s}");
    assert!(
        server
            .statements(Purpose::Console)
            .contains(&"<commit>".to_owned()),
        "{:?}",
        server.statements(Purpose::Console)
    );

    // The pending count shows next to the buttons.
    run_all(&mut h, "insert into t values (3);");
    wait_for(&mut h, "the insert", |s| tx(s)["pending"] == 1);
    let sql = sql_console(&mut h);
    assert_eq!(sql["tx"], "Tx: Manual", "{sql}");
    assert_eq!(sql["commit-buttons"], true, "{sql}");
    assert_eq!(sql["pending"], 1, "{sql}");

    h.invoke("db.console.rollback", Value::Null);
    wait_for(&mut h, "the rollback", |s| tx(s)["pending"] == 0);
    assert!(
        server
            .statements(Purpose::Console)
            .contains(&"<rollback>".to_owned())
    );
}

fn idle_after_error(s: &Value) -> bool {
    idle(s) && output(s).contains("division by zero")
}

#[gpui::test]
fn switching_to_auto_with_changes_pending_asks_and_the_answer_decides(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("db.console.set-tx-mode", json!({ "mode": "manual" }));
    wait_for(&mut h, "manual mode", |s| tx(s)["mode"] == "manual");
    run_all(&mut h, "insert into t values (1);");
    wait_for(&mut h, "the insert", |s| tx(s)["pending"] == 1);

    h.invoke("db.console.set-tx-mode", json!({ "mode": "auto" }));
    let d = wait_decision(&mut h);
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("Switching to auto-commit ends the transaction with 1 uncommitted change"),
        "{d}"
    );
    // Cancel: nothing changes.
    h.invoke("db.tx-decision.cancel", Value::Null);
    h.draw();
    let s = console_state(&mut h);
    assert_eq!(tx(&s)["mode"], "manual", "{s}");
    assert_eq!(tx(&s)["pending"], 1, "{s}");

    // Rollback: the changes go and the mode switches.
    h.invoke("db.console.set-tx-mode", json!({ "mode": "auto" }));
    wait_decision(&mut h);
    h.invoke("db.tx-decision.rollback", Value::Null);
    let s = wait_for(&mut h, "auto mode", |s| tx(s)["mode"] == "auto");
    assert_eq!(tx(&s)["pending"], 0, "{s}");
    assert!(output(&s).contains("manual transaction mode OFF"), "{s}");
    assert!(
        server
            .statements(Purpose::Console)
            .contains(&"<rollback>".to_owned())
    );
}

#[gpui::test]
fn disconnecting_with_changes_pending_asks_before_it_commits_or_rolls_back(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("db.console.set-tx-mode", json!({ "mode": "manual" }));
    wait_for(&mut h, "manual mode", |s| tx(s)["mode"] == "manual");
    run_all(&mut h, "insert into t values (1);");
    wait_for(&mut h, "the insert", |s| tx(s)["pending"] == 1);

    h.invoke("db.explorer.select", json!({ "name": NAME }));
    h.invoke("db.explorer.disconnect", Value::Null);
    let d = wait_decision(&mut h);
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .starts_with("Disconnecting ends the transaction with 1 uncommitted change"),
        "{d}"
    );
    // Nothing was done while it waited.
    assert_eq!(tx(&console_state(&mut h))["pending"], 1);
    h.invoke("db.tx-decision.commit", Value::Null);
    let started = Instant::now();
    while !server
        .statements(Purpose::Console)
        .contains(&"<commit>".to_owned())
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            server.statements(Purpose::Console)
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn the_isolation_level_is_set_for_the_session_and_refused_where_the_engine_has_none(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    let s = open_console(&mut h);
    assert_eq!(tx(&s)["isolation"], "read-committed", "{s}");
    run_all(&mut h, "select 1;");
    wait_for(&mut h, "the statement", idle);

    h.invoke(
        "db.console.set-isolation",
        json!({ "level": "serializable" }),
    );
    wait_for(&mut h, "the level", |s| {
        tx(s)["isolation"] == "serializable"
    });
    assert!(server.statements(Purpose::Console).contains(
        &"SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL SERIALIZABLE".to_owned()
    ));
}

#[gpui::test]
fn the_schema_is_switched_by_the_session_and_shown_by_the_toolbar(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    // The console connects when it opens: the toolbar shows the current
    // schema before any statement ran.
    wait_for(&mut h, "the schema", |s| s["schema"] == "public");
    h.draw();
    let chip = h.invoke("sql.state", Value::Null)["schema-text"].clone();
    assert!(
        chip.as_str().is_some_and(|t| t.ends_with(".public")),
        "{chip}"
    );
    run_all(&mut h, "select 1;");
    let s = wait_for(&mut h, "the schema", |s| s["schema"] == "public");
    assert!(output(&s).contains("public> select 1"), "{s}");

    h.invoke("db.console.set-schema", json!({ "schema": "sales" }));
    let started = Instant::now();
    while !server
        .statements(Purpose::Console)
        .contains(&"SET search_path TO sales".to_owned())
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            server.statements(Purpose::Console)
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn a_statement_without_where_waits_for_the_banner_and_suppress_turns_the_warning_off(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);

    run_all(&mut h, "delete from t;");
    let s = wait_for(&mut h, "the banner", |s| !s["safety"]["unsafe"].is_null());
    assert!(
        s["safety"]["unsafe"]
            .as_str()
            .unwrap()
            .contains("'Delete' statement without 'where'"),
        "{s}"
    );
    assert!(output(&s).contains("Unsafe query"), "{s}");
    assert!(
        !server
            .statements(Purpose::Console)
            .iter()
            .any(|q| q.starts_with("delete")),
        "nothing ran"
    );
    assert!(sql_console(&mut h)["unsafe-banner"].is_string());

    // Closing the banner runs nothing.
    h.invoke("db.console.dismiss-unsafe", Value::Null);
    assert!(sql_console(&mut h)["unsafe-banner"].is_null());
    assert!(console_state(&mut h)["safety"]["unsafe"].is_null());

    // Execute runs it.
    h.invoke("db.console.execute", Value::Null);
    wait_for(&mut h, "the banner again", |s| {
        !s["safety"]["unsafe"].is_null()
    });
    h.invoke("db.console.run-unsafe", json!({ "suppress": false }));
    let s = wait_for(&mut h, "the delete", |s| {
        idle(s) && output(s).contains("5 rows affected")
    });
    assert!(s["safety"]["unsafe"].is_null(), "{s}");

    // Execute and Suppress runs it and the next one asks no more.
    h.invoke("db.console.execute", Value::Null);
    wait_for(&mut h, "the banner a third time", |s| {
        !s["safety"]["unsafe"].is_null()
    });
    h.invoke("db.console.run-unsafe", json!({ "suppress": true }));
    wait_for(&mut h, "the second delete", |s| {
        output(s).matches("5 rows affected").count() == 2
    });
    h.invoke("db.console.execute", Value::Null);
    wait_for(&mut h, "the third delete", |s| {
        output(s).matches("5 rows affected").count() == 3
    });
}

#[gpui::test]
fn parameters_are_asked_remembered_and_bound_never_pasted(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    server.on("from orders", Reaction::column("id", &["7"]));
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);

    run_all(
        &mut h,
        "select * from orders where id = :id and note = :note;",
    );
    let started = Instant::now();
    let dialog = loop {
        let d = console_state_parameters(&mut h);
        if !d.is_null() {
            break d;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the dialog");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    };
    assert_eq!(dialog["rows"][0]["name"], "id", "{dialog}");
    assert_eq!(dialog["rows"][0]["context"], "id = _", "{dialog}");
    assert_eq!(dialog["rows"][1]["name"], "note", "{dialog}");
    assert_eq!(dialog["can_execute"], false, "{dialog}");

    h.invoke("db.params.set", json!({ "name": "id", "value": "7" }));
    h.invoke(
        "db.params.set",
        json!({ "name": "note", "value": "'it''s'" }),
    );
    h.draw();
    assert_eq!(console_state_parameters(&mut h)["can_execute"], true);
    h.invoke("db.params.execute", Value::Null);
    wait_for(&mut h, "the statement", |s| {
        idle(s) && output(s).contains("retrieved")
    });

    let bound = server.bound();
    assert_eq!(bound.len(), 1, "{bound:?}");
    assert_eq!(
        bound[0].0,
        "select * from orders where id = $1 and note = $2"
    );
    assert_eq!(bound[0].1, vec![Db::Int(7), Db::Text("it's".into())]);
    // The values are in no statement's text.
    assert!(
        !server
            .statements(Purpose::Console)
            .iter()
            .any(|q| q.contains("it's") || q.contains("= 7")),
        "{:?}",
        server.statements(Purpose::Console)
    );

    // The next run offers the values again.
    h.invoke("db.console.execute", Value::Null);
    let started = Instant::now();
    let dialog = loop {
        let d = console_state_parameters(&mut h);
        if !d.is_null() {
            break d;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the dialog");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    };
    assert_eq!(dialog["rows"][0]["value"], "7", "{dialog}");
    assert_eq!(dialog["can_execute"], true, "{dialog}");
    // Close runs nothing.
    h.invoke("db.params.close", Value::Null);
    h.draw();
    assert!(console_state_parameters(&mut h).is_null());
    assert_eq!(server.bound().len(), 1);
}

#[gpui::test]
fn the_parameters_button_asks_for_the_values_of_the_statement_at_the_caret(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    server.on("from orders", Reaction::column("id", &["7"]));
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);

    // A statement without parameters: nothing opens, nothing runs.
    h.invoke("editor.select-all", Value::Null);
    h.invoke("editor.type", json!({ "text": "select 1" }));
    h.invoke("db.console.parameters", Value::Null);
    h.draw();
    assert!(console_state_parameters(&mut h).is_null());
    assert!(server.bound().is_empty());

    h.invoke("editor.select-all", Value::Null);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from orders where id = :id" }),
    );
    h.invoke("db.console.parameters", Value::Null);
    let started = Instant::now();
    let dialog = loop {
        let d = console_state_parameters(&mut h);
        if !d.is_null() {
            break d;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the dialog");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    };
    assert_eq!(dialog["rows"][0]["name"], "id", "{dialog}");
    h.invoke("db.params.set", json!({ "name": "id", "value": "7" }));
    h.invoke("db.params.execute", Value::Null);
    wait_for(&mut h, "the statement", |s| {
        idle(s) && output(s).contains("retrieved")
    });
    assert_eq!(server.bound().len(), 1);
}

/// What the toolbar of the console's editor shows of its transactions.
fn sql_console(h: &mut Harness) -> Value {
    h.invoke("sql.state", Value::Null)["console"].clone()
}

fn console_state_parameters(h: &mut Harness) -> Value {
    h.invoke("db.console.state", Value::Null)["parameters"].clone()
}

#[gpui::test]
fn the_output_names_the_isolation_levels_of_the_menu_and_the_menu_marks_the_current_ones(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    run_all(&mut h, "select 1;");
    wait_for(&mut h, "the statement", idle);
    h.invoke("db.console.tx-menu", Value::Null);
    let started = Instant::now();
    let menu = loop {
        let popup = h.invoke("sql.state", Value::Null)["popup"].clone();
        if !popup.is_null() {
            break popup;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the popup");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    };
    let text = menu.to_string();
    for label in [
        "Transaction Mode",
        "Auto",
        "Manual",
        "Transaction Isolation",
        "Database Default",
        "Read Committed",
        "Repeatable Read",
        "Serializable",
    ] {
        assert!(text.contains(label), "{label}: {menu}");
    }
}
