//! The scenario `tests/scenarios/sql/resolution-and-quick-fixes.md` and the
//! other inspections, with their fixes applied and the text checked again.
use sql_intel::{Inspection, InspectionLevel, InspectionProfile, analyze};
use sql_syntax::{Dialect, HOLE};

use crate::fixture::*;

#[test]
fn step_1_2_unresolved_table_and_change_to_similar_name() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "select * from custmers";
        assert_eq!(
            summary(text, &ctx),
            [(
                "SqlResolve",
                "custmers".to_string(),
                "Cannot resolve table 'custmers'".to_string()
            )],
            "{d:?}"
        );
        let fixed = apply_fix(text, &ctx, "Change to 'customers'");
        assert_eq!(fixed, "select * from customers");
        assert!(problems(&fixed, &ctx).is_empty(), "{d:?}");
    }
}

#[test]
fn step_3_ambiguous_column_qualified() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "select id from orders o join customers c on o.customer_id = c.id";
        assert_eq!(
            summary(text, &ctx),
            [(
                "SqlAmbiguousColumn",
                "id".to_string(),
                "Ambiguous column 'id': it is in 'o' and 'c'".to_string()
            )],
            "{d:?}"
        );
        for q in ["o", "c"] {
            let fixed = apply_fix(text, &ctx, &format!("Qualify with '{q}'"));
            assert!(fixed.starts_with(&format!("select {q}.id from")), "{fixed}");
            assert!(problems(&fixed, &ctx).is_empty(), "{d:?}");
        }
    }
}

#[test]
fn step_4_delete_and_update_without_where() {
    for d in Dialect::ALL {
        let ctx = scoped(d);
        for (text, message) in [
            (
                "delete from orders",
                "DELETE without WHERE deletes every row of the table",
            ),
            (
                "update orders set total = 0",
                "UPDATE without WHERE changes every row of the table",
            ),
        ] {
            let ps = summary(text, &ctx);
            assert_eq!(ps.len(), 1, "{d:?} {ps:?}");
            assert_eq!(ps[0].0, "SqlWithoutWhere");
            assert_eq!(ps[0].2, message);
            let fixed = apply_fix(text, &ctx, "Add WHERE clause");
            assert_eq!(fixed, format!("{text} where "));
        }
        // Before RETURNING; not with WHERE CURRENT OF.
        let fixed = apply_fix("delete from orders returning id", &ctx, "Add WHERE clause");
        assert_eq!(fixed, "delete from orders where returning id");
        // Also without a scope: no metadata needed.
        assert_eq!(problems("delete from orders", &unscoped(d)).len(), 1);
    }
    let ora = scoped(Dialect::Oracle);
    assert!(problems("begin delete from orders where current of c; end;", &ora).is_empty());
}

#[test]
fn step_5_comparison_with_null() {
    let ctx = scoped(Dialect::Postgres);
    let text = "select * from orders where total = null";
    assert_eq!(
        summary(text, &ctx),
        [(
            "SqlConstantExpression",
            "= null".to_string(),
            "Comparison with NULL is never true; use IS NULL".to_string()
        )]
    );
    assert_eq!(
        apply_fix(text, &ctx, "Replace with IS NULL"),
        "select * from orders where total is null"
    );
    assert_eq!(
        apply_fix(
            "select * from orders where total <> NULL",
            &ctx,
            "Replace with IS NOT NULL"
        ),
        "select * from orders where total IS NOT NULL"
    );
    assert_eq!(
        apply_fix(
            "select * from orders where null != total",
            &ctx,
            "Replace with IS NOT NULL"
        ),
        "select * from orders where total is not null"
    );
    // Assignments are no comparisons.
    assert!(problems("update orders set total = null where id = 1", &ctx).is_empty());
}

#[test]
fn step_6_unused_cte_greyed_and_removed() {
    for d in Dialect::ALL {
        let ctx = scoped(d);
        let text = "with unused as (select 1) select 2";
        let ps = problems(text, &ctx);
        assert_eq!(ps.len(), 1, "{d:?}: {ps:?}");
        assert_eq!(ps[0].inspection, Inspection::UnusedCte);
        assert!(ps[0].unnecessary);
        assert_eq!(&text[ps[0].range.clone()], "unused");
        assert_eq!(apply_fix(text, &ctx, "Remove CTE"), "select 2");
    }
    let ctx = scoped(Dialect::Postgres);
    let text = "with a as (select 1), b as (select 2), c as (select 3) select * from a, c";
    assert_eq!(
        apply_fix(text, &ctx, "Remove CTE"),
        "with a as (select 1), c as (select 3) select * from a, c"
    );
    let text = "with a as (select 1), b as (select 2) select * from a";
    assert_eq!(
        apply_fix(text, &ctx, "Remove CTE"),
        "with a as (select 1) select * from a"
    );
    // Used from a nested query, or by itself recursively: used.
    assert!(
        problems(
            "with a as (select 1 x) select * from orders where id in (select x from a)",
            &ctx
        )
        .is_empty()
    );
}

#[test]
fn step_7_no_scope_no_unresolved_but_syntax_errors() {
    for d in Dialect::ALL {
        let ctx = unscoped(d);
        assert!(problems("select * from nothing", &ctx).is_empty(), "{d:?}");
        let ps = summary("select * from nothing where", &ctx);
        assert_eq!(ps.len(), 1, "{d:?}: {ps:?}");
        assert_eq!(ps[0].0, "SqlSyntax");
        assert!(ps[0].2.starts_with("Expected: "), "{ps:?}");
    }
}

#[test]
fn step_8_names_only_schema_column_is_unknown() {
    for d in RESOLVING {
        let ctx = scoped(d);
        assert!(
            problems("select anything from audit.log", &ctx).is_empty(),
            "{d:?}"
        );
    }
}

#[test]
fn step_9_suppression_comments() {
    let ctx = scoped(Dialect::Postgres);
    let text = "-- noinspection SqlResolve\nselect * from nothing;\nselect * from nothing2;";
    let ps = summary(text, &ctx);
    assert_eq!(ps.len(), 1, "{ps:?}");
    assert_eq!(ps[0].1, "nothing2");
    let text = "select 1;\n-- noinspection SqlWithoutWhere, SqlResolve\ndelete from nothing;";
    assert!(problems(text, &ctx).is_empty());
    let text = "-- noinspection SqlResolveForFile\nselect * from a;\nselect * from b;";
    assert!(problems(text, &ctx).is_empty());
    // Another inspection is not suppressed; syntax errors never are.
    let text = "-- noinspection SqlResolve\ndelete from nothing";
    assert_eq!(summary(text, &ctx)[0].0, "SqlWithoutWhere");
    let text = "-- noinspection SqlSyntax\nselect * from";
    assert_eq!(summary(text, &ctx)[0].0, "SqlSyntax");
}

#[test]
fn profile_levels_and_disabled_inspections() {
    let ctx = scoped(Dialect::Postgres);
    let a = analyze("delete from nothing", &ctx);
    let mut profile = InspectionProfile::default();
    assert_eq!(a.problems(&profile).len(), 2);
    profile.disable(Inspection::Resolve);
    let ps = a.problems(&profile);
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].inspection, Inspection::WithoutWhere);
    profile.set(Inspection::WithoutWhere, InspectionLevel::Off);
    assert!(a.problems(&profile).is_empty());
    assert_eq!(a.all_problems().len(), 2);
}

#[test]
fn insert_values_count() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "insert into orders (id, total) values (1, 2), (3)";
        let ps = summary(text, &ctx);
        assert_eq!(
            ps,
            [(
                "SqlInsertValues",
                "(3)".to_string(),
                "2 target columns but 1 value".to_string()
            )],
            "{d:?}"
        );
        // Without a column list: the table's columns.
        let ps = summary("insert into orders values (1, 2)", &ctx);
        assert_eq!(ps[0].2, "3 target columns but 2 values", "{d:?}");
        // From a query.
        let ps = summary("insert into customers select id from orders", &ctx);
        assert_eq!(ps[0].2, "2 target columns but 1 value", "{d:?}");
        assert!(problems("insert into customers select * from orders", &ctx).is_empty());
        assert!(problems("insert into customers values (1, 'a')", &ctx).is_empty());
    }
    // No scope: only a column list counts.
    let ctx = unscoped(Dialect::Postgres);
    assert_eq!(problems("insert into t (a) values (1, 2)", &ctx).len(), 1);
    assert!(problems("insert into t values (1, 2)", &ctx).is_empty());
}

#[test]
fn syntax_errors_and_missing_parenthesis() {
    let ctx = unscoped(Dialect::Postgres);
    let text = "select (a + b from t";
    let ps = summary(text, &ctx);
    assert_eq!(ps[0].2, "Expected: ')'");
    assert_eq!(
        apply_fix(text, &ctx, "Add missing ')'"),
        "select (a + b) from t"
    );
    let ps = summary("select 'abc from t", &ctx);
    assert_eq!(ps.len(), 1, "{ps:?}");
    assert_eq!(ps[0].2, "Unterminated string");
    assert_eq!(ps[0].1, "'");
    assert_eq!(summary("select a) from t", &ctx)[0].2, "Unexpected ')'");
}

#[test]
fn holes_hide_problems_they_could_explain() {
    let ctx = scoped(Dialect::Postgres);
    let text = format!("select nope from orders {HOLE}");
    assert!(
        problems(&text, &ctx).is_empty(),
        "{:?}",
        problems(&text, &ctx)
    );
    let text = format!("select * from {HOLE} where x = 1");
    assert!(problems(&text, &ctx).is_empty());
}
