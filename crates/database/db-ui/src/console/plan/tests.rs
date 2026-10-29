use std::collections::HashSet;

use db_core::Value;
use sql_syntax::Dialect;

use super::*;

const PG_ANALYZE: &str = r#"[
  {
    "Plan": {
      "Node Type": "Hash Join",
      "Parallel Aware": false,
      "Join Type": "Inner",
      "Startup Cost": 6.70,
      "Total Cost": 43.26,
      "Plan Rows": 300,
      "Actual Startup Time": 0.678,
      "Actual Total Time": 1.221,
      "Actual Rows": 300.00,
      "Hash Cond": "(o.customer_id = c.id)",
      "Plans": [
        {
          "Node Type": "Seq Scan",
          "Parent Relationship": "Outer",
          "Relation Name": "orders",
          "Alias": "o",
          "Startup Cost": 0.00,
          "Total Cost": 35.75,
          "Plan Rows": 300,
          "Actual Startup Time": 0.045,
          "Actual Total Time": 0.504,
          "Actual Rows": 300.00,
          "Filter": "(status = 'paid'::demo.order_status)"
        },
        {
          "Node Type": "Hash",
          "Parent Relationship": "Inner",
          "Startup Cost": 5.20,
          "Total Cost": 5.20,
          "Plan Rows": 120,
          "Plans": [
            {
              "Node Type": "Index Scan",
              "Relation Name": "customers",
              "Index Name": "customers_pkey",
              "Startup Cost": 0.00,
              "Total Cost": 5.20,
              "Plan Rows": 120
            }
          ]
        }
      ]
    },
    "Planning": {"Shared Hit Blocks": 10},
    "Planning Time": 0.827,
    "Triggers": [],
    "Execution Time": 1.025
  }
]"#;

fn text_rows(lines: &[&str]) -> Vec<Vec<Value>> {
    lines
        .iter()
        .map(|l| vec![Value::Text((*l).into())])
        .collect()
}

fn paths(plan: &Plan) -> Vec<(usize, String)> {
    plan.lines(&HashSet::new())
        .iter()
        .map(|l| (l.path.len(), l.node.operation.clone()))
        .collect()
}

#[test]
fn a_postgres_plan_is_the_statement_over_the_plan_tree() {
    let rows = vec![vec![Value::Json(PG_ANALYZE.into())]];
    let plan = Plan::from_rows(
        Dialect::Postgres,
        "select * from demo.orders o join demo.customers c on c.id = o.customer_id",
        &rows,
        true,
    );
    assert_eq!(
        paths(&plan),
        [
            (0, "Select".to_owned()),
            (1, "Hash Join".to_owned()),
            (2, "Full Scan (Seq Scan)".to_owned()),
            (2, "Transformation (Hash)".to_owned()),
            (3, "Index Scan (Index Scan)".to_owned()),
        ]
    );
    let join = plan.node(&[0]).unwrap();
    assert_eq!(join.kind, NodeKind::Join);
    assert_eq!(
        (join.rows, join.total_cost, join.startup_cost),
        (Some(300.0), Some(43.26), Some(6.7))
    );
    assert_eq!(
        (
            join.actual_rows,
            join.actual_total_time,
            join.actual_startup_time
        ),
        (Some(300.0), Some(1.221), Some(0.678))
    );
    // What the columns do not show is the raw description.
    assert_eq!(
        join.raw_desc,
        "Parallel Aware = false; Join Type = Inner; Hash Cond = (o.customer_id = c.id)"
    );
    let scan = plan.node(&[0, 0]).unwrap();
    assert_eq!(scan.params, "table: orders;");
    assert_eq!(
        plan.node(&[0, 1, 0]).unwrap().params,
        "table: customers; index: customers_pkey;"
    );
    assert_eq!(plan.node(&[0, 1, 0]).unwrap().kind, NodeKind::IndexScan);
    // The statement carries the document's own properties.
    let root = &plan.root;
    assert_eq!(root.props[0], ("Operation".into(), "select".into()));
    assert!(
        root.props
            .iter()
            .any(|(k, v)| k == "Execution Time" && v == "1.025")
    );
    assert!(
        root.raw_desc.starts_with("Planning = {"),
        "{}",
        root.raw_desc
    );
    // The raw text is the answer, indented.
    assert!(
        plan.raw.starts_with("[\n  {\n    \"Plan\": {"),
        "{}",
        plan.raw
    );
}

#[test]
fn the_columns_are_the_ones_the_plan_has_a_value_for() {
    let rows = vec![vec![Value::Json(PG_ANALYZE.into())]];
    let analyzed = Plan::from_rows(Dialect::Postgres, "select 1", &rows, true);
    let titles: Vec<_> = analyzed.columns().iter().map(|c| c.title()).collect();
    assert_eq!(
        titles,
        [
            "Operation",
            "Params",
            "Rows",
            "Actual Rows",
            "Total Cost",
            "Actual Total Time",
            "Startup Cost",
            "Actual Startup Time",
            "Raw Desc"
        ]
    );
    // Without analyze the server gives no actual values.
    let plain = r#"[{"Plan": {"Node Type": "Result", "Startup Cost": 0.0, "Total Cost": 0.01, "Plan Rows": 1}}]"#;
    let rows = vec![vec![Value::Text(plain.into())]];
    let plan = Plan::from_rows(Dialect::Postgres, "select 1", &rows, false);
    let titles: Vec<_> = plan.columns().iter().map(|c| c.title()).collect();
    assert_eq!(
        titles,
        ["Operation", "Params", "Rows", "Total Cost", "Startup Cost"]
    );
}

#[test]
fn a_collapsed_node_hides_its_descendants() {
    let rows = vec![vec![Value::Json(PG_ANALYZE.into())]];
    let plan = Plan::from_rows(Dialect::Postgres, "select 1", &rows, true);
    let mut collapsed = HashSet::new();
    collapsed.insert(vec![0, 1]);
    let shown: Vec<_> = plan
        .lines(&collapsed)
        .iter()
        .map(|l| l.node.operation.clone())
        .collect();
    assert_eq!(
        shown,
        [
            "Select",
            "Hash Join",
            "Full Scan (Seq Scan)",
            "Transformation (Hash)"
        ]
    );
}

#[test]
fn the_statement_names_the_root() {
    let rows = vec![vec![Value::Text(
        r#"[{"Plan": {"Node Type": "Result"}}]"#.into(),
    )]];
    let label = |sql: &str| {
        Plan::from_rows(Dialect::Postgres, sql, &rows, false)
            .root
            .operation
    };
    assert_eq!(label("select 1"), "Select");
    assert_eq!(label("with a as (select 1) select * from a"), "Select");
    assert_eq!(label("  -- c\n update t set a = 1"), "Update");
    assert_eq!(label(""), "Statement");
}

#[test]
fn an_answer_that_is_no_plan_stays_as_raw_text() {
    let rows = text_rows(&["this is not json"]);
    let plan = Plan::from_rows(Dialect::Postgres, "select 1", &rows, false);
    assert_eq!(plan.root.operation, "Select");
    assert!(plan.root.children.is_empty());
    assert_eq!(plan.raw, "this is not json");
}

#[test]
fn a_sqlite_plan_hangs_its_rows_on_their_parents() {
    let row = |id: i64, parent: i64, detail: &str| {
        vec![
            Value::Int(id),
            Value::Int(parent),
            Value::Int(0),
            Value::Text(detail.into()),
        ]
    };
    let rows = vec![
        row(2, 0, "SCAN o"),
        row(5, 0, "SEARCH c USING INTEGER PRIMARY KEY (rowid=?)"),
        row(8, 5, "USE TEMP B-TREE FOR ORDER BY"),
    ];
    let plan = Plan::from_rows(Dialect::Sqlite, "select * from o", &rows, false);
    assert_eq!(
        paths(&plan),
        [
            (0, "Select".to_owned()),
            (1, "SCAN".to_owned()),
            (1, "SEARCH".to_owned()),
            (2, "USE TEMP B-TREE FOR ORDER BY".to_owned()),
        ]
    );
    assert_eq!(plan.node(&[0]).unwrap().params, "o");
    assert_eq!(plan.node(&[0]).unwrap().kind, NodeKind::Scan);
    assert_eq!(plan.node(&[1]).unwrap().kind, NodeKind::IndexScan);
    assert_eq!(
        plan.node(&[1]).unwrap().params,
        "c USING INTEGER PRIMARY KEY (rowid=?)"
    );
    // No estimates in SQLite: only the operation and its object show.
    let titles: Vec<_> = plan.columns().iter().map(|c| c.title()).collect();
    assert_eq!(titles, ["Operation", "Params"]);
    assert_eq!(
        plan.raw,
        "QUERY PLAN\n|--SCAN o\n`--SEARCH c USING INTEGER PRIMARY KEY (rowid=?)\n   `--USE TEMP B-TREE FOR ORDER BY"
    );
}

const ORACLE: &str = "Plan hash value: 735651981

---------------------------------------------------------------------------
| Id  | Operation          | Name | Rows  | Bytes | Cost (%CPU)| Time     |
---------------------------------------------------------------------------
|   0 | SELECT STATEMENT   |      |     1 |     4 |     4   (0)| 00:00:01 |
|   1 |  NESTED LOOPS      |      |     1 |     4 |     4   (0)| 00:00:01 |
|*  2 |   TABLE ACCESS FULL| DUAL |    12K|     2 |     2   (0)| 00:00:01 |
|*  3 |   TABLE ACCESS FULL| DUAL |     1 |     2 |     2   (0)| 00:00:01 |
---------------------------------------------------------------------------

Predicate Information (identified by operation id):
---------------------------------------------------

   2 - filter(\"B\".\"DUMMY\"='X')
   3 - filter(\"A\".\"DUMMY\"='X'
              AND \"A\".\"DUMMY\" IS NOT NULL)
";

#[test]
fn an_oracle_plan_nests_by_the_indentation_of_its_operations() {
    let lines: Vec<&str> = ORACLE.lines().collect();
    let rows = text_rows(&lines);
    let plan = Plan::from_rows(Dialect::Oracle, "select * from dual", &rows, false);
    assert_eq!(
        paths(&plan),
        [
            (0, "SELECT STATEMENT".to_owned()),
            (1, "NESTED LOOPS".to_owned()),
            (2, "TABLE ACCESS FULL".to_owned()),
            (2, "TABLE ACCESS FULL".to_owned()),
        ]
    );
    let scan = plan.node(&[0, 0]).unwrap();
    assert_eq!(scan.params, "object: DUAL;");
    assert_eq!(scan.rows, Some(12_000.0));
    assert_eq!(scan.total_cost, Some(2.0));
    assert_eq!(scan.kind, NodeKind::Scan);
    assert!(
        scan.props
            .iter()
            .any(|(k, v)| k == "Predicate" && v == "filter(\"B\".\"DUMMY\"='X')")
    );
    // A predicate that goes on over several lines is one property.
    assert!(
        plan.node(&[0, 1])
            .unwrap()
            .props
            .iter()
            .any(|(k, v)| k == "Predicate"
                && v == "filter(\"A\".\"DUMMY\"='X' AND \"A\".\"DUMMY\" IS NOT NULL)")
    );
    assert_eq!(plan.root.kind, NodeKind::Statement);
    assert!(plan.raw.starts_with("Plan hash value: 735651981"));
}

#[test]
fn numbers_lose_their_trailing_zeros() {
    assert_eq!(number(43.26), "43.26");
    assert_eq!(number(300.0), "300");
    assert_eq!(number(0.9470001), "0.947");
    assert_eq!(number(5.2), "5.2");
}
