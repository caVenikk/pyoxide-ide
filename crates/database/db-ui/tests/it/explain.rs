//! Explain Plan and Explain Analyze of a console over a scripted
//! PostgreSQL server: the statement at the caret goes to the server wrapped
//! in `EXPLAIN (FORMAT JSON)`, the answer shows as a Query Plan tab with a
//! tree and the raw text; the analyzed statement runs only where that is
//! allowed.

use db_core::Purpose;
use db_session::fake::{FakeSource, Reaction};
use gpui::TestAppContext;
use serde_json::{Value, json};

use super::console::{
    console_state, harness, harness_with, idle, open_console, output, wait_console,
};
use super::*;

const PLAN: &str = r#"[{"Plan": {"Node Type": "Hash Join", "Startup Cost": 6.70, "Total Cost": 43.26, "Plan Rows": 300, "Join Type": "Inner",
  "Plans": [
    {"Node Type": "Seq Scan", "Relation Name": "customers", "Startup Cost": 0.00, "Total Cost": 35.75, "Plan Rows": 300},
    {"Node Type": "Hash", "Startup Cost": 5.20, "Total Cost": 5.20, "Plan Rows": 120}
  ]}}]"#;

const ANALYZED: &str = r#"[{"Plan": {"Node Type": "Seq Scan", "Relation Name": "customers", "Startup Cost": 0.00, "Total Cost": 35.75, "Plan Rows": 300,
  "Actual Startup Time": 0.045, "Actual Total Time": 0.504, "Actual Rows": 300.00},
  "Planning Time": 0.827, "Execution Time": 1.025}]"#;

fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    source.on(
        "explain (analyze",
        Reaction::column("QUERY PLAN", &[ANALYZED]),
    );
    source.on(
        "explain (format json)",
        Reaction::column("QUERY PLAN", &[PLAN]),
    );
    source.on("from demo.customers", Reaction::column("id", &["1"]));
    source.on("delete from", Reaction::Affected(1));
    source
}

/// The plan of the first tab, once the console is idle with one.
fn plan(h: &mut Harness) -> Value {
    let s = wait_console(h, "the plan", |s| {
        idle(s) && s["tabs"].as_array().is_some_and(|t| !t.is_empty())
    });
    s["tabs"][0]["plan"].clone()
}

fn rows(plan: &Value) -> Vec<(u64, String)> {
    plan["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["depth"].as_u64().unwrap(),
                r["operation"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[gpui::test]
fn explain_plan_shows_the_tree_of_the_statement_without_running_it(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers where id = 1;" }),
    );
    h.invoke("db.console.explain", Value::Null);
    let plan = plan(&mut h);
    let s = console_state(&mut h);
    assert_eq!(s["tabs"][0]["title"], "Query Plan", "{s}");
    assert_eq!(s["pane"], 0, "the plan shows: {s}");
    assert!(s["tabs"][0]["grid"].is_null(), "{s}");

    assert_eq!(
        rows(&plan),
        [
            (0, "Select".to_owned()),
            (1, "Hash Join".to_owned()),
            (2, "Full Scan (Seq Scan)".to_owned()),
            (2, "Transformation (Hash)".to_owned()),
        ],
        "{plan}"
    );
    assert_eq!(
        plan["columns"],
        json!([
            "Operation",
            "Params",
            "Rows",
            "Total Cost",
            "Startup Cost",
            "Raw Desc"
        ])
    );
    assert_eq!(plan["rows"][1]["total_cost"], "43.26");
    assert_eq!(plan["rows"][1]["rows"], "300");
    assert_eq!(plan["rows"][2]["params"], "table: customers;");
    assert_eq!(plan["analyze"], false);

    // Only the explain went to the server, in the console's session.
    let sent = server.statements(Purpose::Console);
    assert!(
        sent.iter()
            .any(|q| q == "EXPLAIN (FORMAT JSON) select * from demo.customers where id = 1"),
        "{sent:?}"
    );
    assert!(
        !sent.iter().any(|q| q.starts_with("select")),
        "the statement did not run: {sent:?}"
    );
    let log = output(&s);
    assert!(log.contains("> EXPLAIN (FORMAT JSON) select"), "{log}");
}

#[gpui::test]
fn the_properties_of_the_selected_node_and_the_raw_text_are_in_the_tab(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers;" }),
    );
    h.invoke("db.console.explain", Value::Null);
    let plan = plan(&mut h);
    assert_eq!(plan["selected"], json!([]));
    assert_eq!(plan["details"], json!([["Operation", "select"]]));

    h.invoke("db.plan.select", json!({"path": [0]}));
    let plan = console_state(&mut h)["tabs"][0]["plan"].clone();
    assert_eq!(plan["selected"], json!([0]));
    assert_eq!(plan["details"][0], json!(["Operation", "Hash Join"]));
    assert!(
        plan["details"]
            .to_string()
            .contains(r#"["Join Type","Inner"]"#),
        "{plan}"
    );

    h.invoke("db.plan.toggle", json!({"path": [0]}));
    let plan = console_state(&mut h)["tabs"][0]["plan"].clone();
    assert_eq!(
        rows(&plan),
        [(0, "Select".to_owned()), (1, "Hash Join".to_owned())]
    );
    assert_eq!(plan["rows"][1]["collapsed"], true);
    h.invoke("db.plan.toggle", json!({"path": [0]}));
    let plan = console_state(&mut h)["tabs"][0]["plan"].clone();
    assert_eq!(rows(&plan).len(), 4);

    assert_eq!(plan["mode"], "tree");
    h.invoke("db.plan.view", json!({"mode": "text"}));
    let plan = console_state(&mut h)["tabs"][0]["plan"].clone();
    assert_eq!(plan["mode"], "text");
    assert!(
        plan["raw"]
            .as_str()
            .unwrap()
            .contains("\"Node Type\": \"Hash Join\""),
        "{plan}"
    );
}

#[gpui::test]
fn explain_analyze_runs_a_select_and_shows_the_measured_values(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers;" }),
    );
    h.invoke("db.console.explain-analyze", Value::Null);
    let plan = plan(&mut h);
    assert_eq!(plan["analyze"], true);
    assert_eq!(
        plan["columns"],
        json!([
            "Operation",
            "Params",
            "Rows",
            "Actual Rows",
            "Total Cost",
            "Actual Total Time",
            "Startup Cost",
            "Actual Startup Time",
            "Raw Desc"
        ])
    );
    assert_eq!(plan["rows"][1]["actual_total_time"], "0.504");
    assert_eq!(plan["rows"][1]["actual_rows"], "300");
    // Execution time and the like are the statement's properties.
    h.invoke("db.plan.select", json!({"path": []}));
    let plan = console_state(&mut h)["tabs"][0]["plan"].clone();
    assert!(
        plan["details"]
            .to_string()
            .contains(r#"["Execution Time","1.025"]"#),
        "{plan}"
    );
    let sent = server.statements(Purpose::Console);
    assert!(
        sent.iter()
            .any(|q| q.starts_with("EXPLAIN (ANALYZE, FORMAT JSON) select")),
        "{sent:?}"
    );

    // The next explain reuses the plan's tab.
    h.invoke("db.console.explain", Value::Null);
    let s = wait_console(&mut h, "the second plan", |s| {
        idle(s) && s["tabs"][0]["plan"]["analyze"] == false
    });
    assert_eq!(s["tabs"].as_array().unwrap().len(), 1, "{s}");
}

#[gpui::test]
fn explain_analyze_on_a_read_only_data_source_runs_selects_only(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let mut profile = pg();
    profile.read_only = true;
    let (mut h, _cache, _consoles) = harness_with(cx, &root, &server, profile);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "delete from demo.customers where id = 1;" }),
    );
    h.invoke("db.console.explain-analyze", Value::Null);
    let s = wait_console(&mut h, "the refusal", |s| {
        output(s).contains("allowed for SELECT only")
    });
    assert!(s["tabs"].as_array().unwrap().is_empty(), "{s}");
    assert!(
        !server
            .statements(Purpose::Console)
            .iter()
            .any(|q| q.contains("ANALYZE")),
        "nothing was sent"
    );

    // The plan alone does not run the statement: allowed.
    h.invoke("db.console.explain", Value::Null);
    let plan = plan(&mut h);
    assert_eq!(rows(&plan)[0], (0, "Delete".to_owned()), "{plan}");

    // A SELECT may be analyzed.
    h.invoke("editor.select-all", Value::Null);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers;" }),
    );
    h.invoke("db.console.explain-analyze", Value::Null);
    wait_console(&mut h, "the analyze", |s| {
        idle(s) && s["tabs"][0]["plan"]["analyze"] == true
    });
}
