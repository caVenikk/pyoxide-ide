//! Explain Plan and Explain Analyse of a console over a scripted
//! PostgreSQL server: the statement at the caret goes to the server wrapped
//! in `EXPLAIN (FORMAT TEXT)`, the answer shows as a Query Plan tab with a
//! tree and the raw text; the analyzed statement runs only where that is
//! allowed.

use db_core::Purpose;
use db_session::fake::{FakeSource, Reaction};
use gpui::TestAppContext;
use serde_json::{Value, json};

use super::console::{
    console_state, harness, harness_with, idle, open_console, output, wait_console,
};
use super::grid::NAME;
use super::*;

const PLAN: &[&str] = &[
    "Hash Join  (cost=6.70..43.26 rows=300 width=22)",
    "  Hash Cond: (o.customer_id = c.id)",
    "  ->  Seq Scan on customers  (cost=0.00..35.75 rows=300 width=10)",
    "  ->  Hash  (cost=5.20..5.20 rows=120 width=20)",
];
const ANALYZED: &[&str] = &[
    "Seq Scan on customers  (cost=0.00..35.75 rows=300 width=10) (actual time=0.045..0.504 rows=300.00 loops=1)",
    "Planning Time: 0.827 ms",
    "Execution Time: 1.025 ms",
];

fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    source.on("explain (analyze", Reaction::column("QUERY PLAN", ANALYZED));
    source.on(
        "explain (format text)",
        Reaction::column("QUERY PLAN", PLAN),
    );
    source.on("from demo.customers", Reaction::column("id", &["1"]));
    source.on("delete from", Reaction::Affected(1));
    source
}

#[gpui::test]
fn toolbar_explain_split_opens_its_menu_or_runs_the_plain_plan(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, VisualTestContext};

    let (_dir, root) = project();
    let source = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &source);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({"text": "select * from demo.customers;"}),
    );
    for selector in ["sql-explain-menu", "sql-explain"] {
        let mut view = VisualTestContext::from_window(h.window.into(), h.cx);
        let center = view
            .debug_bounds(selector)
            .expect("split button part")
            .center();
        view.simulate_mouse_down(center, MouseButton::Left, Modifiers::none());
        view.simulate_mouse_up(center, MouseButton::Left, Modifiers::none());
        h.draw();
        if selector == "sql-explain-menu" {
            let state = h.invoke("sql.state", Value::Null);
            assert_eq!(state["popup"]["levels"][0]["selected"], 0);
            let items = &state["popup"]["levels"][0]["items"];
            assert_eq!(items[0]["label"], "Explain Plan");
            assert_eq!(items[1]["label"], "Explain Analyse");
            assert!(
                source
                    .statements(Purpose::Console)
                    .iter()
                    .all(|q| !q.starts_with("EXPLAIN"))
            );
            h.press("escape");
            assert!(h.invoke("sql.state", Value::Null)["popup"].is_null());
        }
    }
    assert_eq!(plan(&mut h)["analyze"], false);
    assert_eq!(
        source
            .statements(Purpose::Console)
            .iter()
            .filter(|q| q.starts_with("EXPLAIN"))
            .count(),
        1
    );
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
            .any(|q| q == "EXPLAIN (FORMAT TEXT) select * from demo.customers where id = 1"),
        "{sent:?}"
    );
    assert!(
        !sent.iter().any(|q| q.starts_with("select")),
        "the statement did not run: {sent:?}"
    );
    let log = output(&s);
    assert!(log.contains("> EXPLAIN (FORMAT TEXT) select"), "{log}");
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
            .contains(r#"["Hash Cond","(o.customer_id = c.id)"]"#),
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
    assert_eq!(plan["raw"], PLAN.join("\n"));
    let s = wait_console(&mut h, "raw grid rows", |s| {
        s["tabs"][0]["plan"]["raw_grid"]["rows"] == PLAN.len()
    });
    let plan = &s["tabs"][0]["plan"];
    assert_eq!(plan["raw_grid"]["rows"], PLAN.len());
    assert_eq!(plan["raw_grid"]["columns"][0]["name"], "QUERY PLAN");
    let longest = PLAN.iter().map(|line| line.chars().count()).max().unwrap();
    assert!(longest > 50);
    assert_eq!(plan["raw_grid"]["columns"][0]["auto_width_chars"], longest);
    assert_eq!(plan["raw_grid"]["cells"][1][0], PLAN[1]);
    h.press("down");
    h.press("down");
    assert_eq!(
        console_state(&mut h)["tabs"][0]["plan"]["raw_grid"]["selection"]["lead"][0],
        1
    );
    h.press("tab");
    assert_eq!(console_state(&mut h)["tabs"][0]["plan"]["mode"], "tree");
    h.press("left");
    assert_eq!(
        console_state(&mut h)["tabs"][0]["plan"]["rows"][1]["collapsed"],
        true
    );
    h.press("right");
    assert_eq!(
        console_state(&mut h)["tabs"][0]["plan"]["rows"][1]["collapsed"],
        false
    );
    h.press("tab");
    h.press("escape");
    h.press("x");
    assert!(
        h.invoke("editor.text", Value::Null)
            .as_str()
            .unwrap()
            .ends_with('x')
    );
    h.invoke("db.console.close-result", json!({"index":0}));
    assert_eq!(console_state(&mut h)["pane"], "output");
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
            .contains(r#"["Execution Time","1.025 ms"]"#),
        "{plan}"
    );
    for mode in ["text", "tree", "text"] {
        h.invoke("db.plan.view", json!({"mode": mode}));
    }
    assert_eq!(
        server
            .statements(Purpose::Console)
            .iter()
            .filter(|q| q.starts_with("EXPLAIN"))
            .count(),
        1
    );
    let sent = server.statements(Purpose::Console);
    assert!(
        sent.iter()
            .any(|q| q.starts_with("EXPLAIN (ANALYZE, FORMAT TEXT) select")),
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

fn open_file(h: &mut Harness, path: &std::path::Path) {
    h.invoke(
        "project.open-file",
        json!({ "path": path.display().to_string() }),
    );
}

/// The first-level rows of the editor's context menu: (label, has submenu).
fn menu(h: &mut Harness) -> Vec<(String, bool)> {
    h.state()["editor_menu"]["menu"]["levels"][0]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| {
            (
                r["label"].as_str().unwrap_or("---").to_owned(),
                r["submenu"] == true,
            )
        })
        .collect()
}

#[gpui::test]
fn the_editor_menu_of_a_console_and_an_attached_file_has_explain_plan_and_execute(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    std::fs::write(root.join("scratch.sql"), "select * from demo.customers").unwrap();
    std::fs::write(root.join("loose.sql"), "select 1").unwrap();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    let rows = menu(&mut h);
    let at = |label: &str| rows.iter().position(|r| r.0 == label);
    let explain = at("Explain Plan").unwrap_or_else(|| panic!("{rows:?}"));
    assert!(rows[explain].1, "a submenu");
    assert_eq!(at("Execute"), Some(explain + 1), "{rows:?}");
    assert!(at("Paste").unwrap() < explain);
    assert_eq!(rows.last().unwrap().0, "Open In");
    h.invoke("menu.cancel", Value::Null);

    // The same in a file attached to the data source.
    open_file(&mut h, &root.join("scratch.sql"));
    h.invoke("sql.attach-data-source", json!({ "name": NAME }));
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    let rows = menu(&mut h);
    assert!(
        rows.iter().any(|r| r.0 == "Explain Plan" && r.1),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.0 == "Execute"), "{rows:?}");
    h.invoke("menu.cancel", Value::Null);

    // A file with no data source has neither.
    open_file(&mut h, &root.join("loose.sql"));
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    let rows = menu(&mut h);
    assert!(
        !rows
            .iter()
            .any(|r| r.0 == "Explain Plan" || r.0 == "Execute"),
        "{rows:?}"
    );
}

#[gpui::test]
fn the_explain_row_of_the_editor_menu_explains_the_statement_at_the_caret(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({ "text": "select * from demo.customers where id = 1;" }),
    );
    h.invoke("editor.context-menu", Value::Null);
    h.draw();
    // Down to Explain Plan, open its submenu, choose the first row.
    let items = menu(&mut h);
    let index = items.iter().position(|r| r.0 == "Explain Plan").unwrap();
    let downs = items[..=index].iter().filter(|r| r.0 != "---").count();
    for _ in 0..downs {
        h.invoke("menu.down", Value::Null);
    }
    h.invoke("menu.open-submenu", Value::Null);
    let state = h.state();
    let submenu = &state["editor_menu"]["menu"]["levels"][1]["items"];
    assert_eq!(submenu[0]["label"], "Explain Plan");
    assert_eq!(submenu[1]["label"], "Explain Analyse");
    h.invoke("menu.confirm", Value::Null);
    let plan = plan(&mut h);
    assert!(!rows(&plan).is_empty(), "{plan}");
    assert!(h.state()["editor_menu"].is_null());
}

/// Explain and Execute share a session without losing the selected
/// Execute variant's result-tab policy or treating ordinary rows as plans.
#[gpui::test]
fn execute_variant_keeps_its_result_policy_after_explain(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({"text": "select * from demo.customers;"}),
    );
    h.invoke("db.console.explain", Value::Null);
    let _ = plan(&mut h);
    h.invoke(
        "settings.set",
        json!({
            "key": "database.query-execution.execute-2.new-tab", "value": true
        }),
    );
    h.invoke("db.console.execute-2", Value::Null);
    let s = wait_console(&mut h, "the separate result", |s| {
        idle(s) && s["tabs"].as_array().is_some_and(|tabs| tabs.len() == 2)
    });
    assert!(!s["tabs"][0]["plan"].is_null(), "{s}");
    assert!(s["tabs"][1]["plan"].is_null(), "{s}");
    assert!(!s["tabs"][1]["grid"].is_null(), "{s}");
    assert_eq!(s["pane"], 1);
    assert!(
        server
            .statements(Purpose::Console)
            .contains(&"select * from demo.customers".to_owned())
    );
}

fn parameters(h: &mut Harness) -> Value {
    let started = std::time::Instant::now();
    loop {
        let dialog = h.invoke("db.console.state", Value::Null)["parameters"].clone();
        if !dialog.is_null() {
            return dialog;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "parameters dialog"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn explain_analyze_checks_safety_then_binds_parameters(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({"text": "update demo.customers set id = :id;"}),
    );
    h.invoke("db.console.explain-analyze", Value::Null);
    assert!(!console_state(&mut h)["safety"]["unsafe"].is_null());
    assert!(
        !server
            .statements(Purpose::Console)
            .iter()
            .any(|s| s.starts_with("EXPLAIN"))
    );
    h.invoke("db.console.dismiss-unsafe", Value::Null);
    assert!(console_state(&mut h)["safety"]["unsafe"].is_null());
    assert!(server.bound().is_empty());

    h.invoke("db.console.explain-analyze", Value::Null);
    h.invoke("db.console.run-unsafe", json!({"suppress": false}));
    let dialog = parameters(&mut h);
    assert_eq!(dialog["rows"][0]["name"], "id");
    assert!(
        !server
            .statements(Purpose::Console)
            .iter()
            .any(|s| s.starts_with("EXPLAIN"))
    );
    h.invoke("db.params.set", json!({"name": "id", "value": "7"}));
    h.invoke("db.params.execute", Value::Null);
    let plan = plan(&mut h);
    assert_eq!(plan["analyze"], true);
    let bound = server.bound();
    assert_eq!(bound.len(), 1, "{bound:?}");
    assert!(bound[0].0.starts_with("EXPLAIN (ANALYZE"), "{bound:?}");
    assert!(
        bound[0].0.ends_with("update demo.customers set id = $1"),
        "{bound:?}"
    );
    assert_eq!(bound[0].1, vec![db_core::Value::Int(7)]);
}

#[gpui::test]
fn plain_explain_reviews_parameters_without_warning_and_cancel_leaves_no_plan(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project();
    let server = server();
    let (mut h, _cache, _consoles) = harness(cx, &root, &server);
    open_console(&mut h);
    h.invoke(
        "editor.type",
        json!({"text": "update demo.customers set id = :id;"}),
    );
    h.invoke("db.console.explain", Value::Null);
    parameters(&mut h);
    assert!(console_state(&mut h)["safety"]["unsafe"].is_null());
    h.invoke("db.params.close", Value::Null);
    assert!(server.bound().is_empty());
    h.invoke("editor.select-all", Value::Null);
    h.invoke(
        "editor.type",
        json!({"text": "select * from demo.customers;"}),
    );
    h.invoke("db.console.execute", Value::Null);
    let s = wait_console(&mut h, "ordinary result after cancellation", |s| {
        idle(s) && !s["tabs"][0]["grid"].is_null()
    });
    assert!(s["tabs"][0]["plan"].is_null());

    h.invoke("editor.select-all", Value::Null);
    h.invoke(
        "editor.type",
        json!({"text": "select * from demo.customers where id = :id;"}),
    );
    h.invoke("db.console.explain", Value::Null);
    parameters(&mut h);
    h.invoke("db.params.set", json!({"name": "id", "value": "9"}));
    h.invoke("db.params.execute", Value::Null);
    wait_console(&mut h, "bound plan", |s| {
        idle(s)
            && s["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| !t["plan"].is_null())
    });
    let bound = server.bound();
    assert_eq!(
        bound[0].0,
        "EXPLAIN (FORMAT TEXT) select * from demo.customers where id = $1"
    );
    assert_eq!(bound[0].1, vec![db_core::Value::Int(9)]);
    h.invoke("db.console.explain", Value::Null);
    assert_eq!(parameters(&mut h)["rows"][0]["value"], "9");
    h.invoke("db.params.close", Value::Null);
}
