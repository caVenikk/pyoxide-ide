use sql_syntax::{Dialect, ExplainOptions, explain_statements, supports_analyze};

fn plain() -> ExplainOptions {
    ExplainOptions::default()
}

#[test]
fn per_dialect() {
    let e = |s: &str, d| explain_statements(s, d, plain());
    assert_eq!(
        e("select 1;", Dialect::Postgres),
        ["EXPLAIN (FORMAT JSON) select 1"]
    );
    assert_eq!(
        e(" select 1 ", Dialect::Sqlite),
        ["EXPLAIN QUERY PLAN select 1"]
    );
    assert_eq!(e("select 1", Dialect::Generic), ["EXPLAIN select 1"]);
    assert_eq!(
        e("select 1 from dual", Dialect::Oracle),
        [
            "EXPLAIN PLAN FOR select 1 from dual",
            "SELECT plan_table_output FROM TABLE(DBMS_XPLAN.DISPLAY())"
        ]
    );
}

#[test]
fn analyze_only_where_the_engine_has_it() {
    let opts = ExplainOptions { analyze: true };
    assert_eq!(
        explain_statements("select 1", Dialect::Postgres, opts),
        ["EXPLAIN (ANALYZE, FORMAT JSON) select 1"]
    );
    assert!(
        supports_analyze(Dialect::Postgres)
            && !supports_analyze(Dialect::Oracle)
            && !supports_analyze(Dialect::Sqlite)
    );
    assert_eq!(
        explain_statements("select 1", Dialect::Sqlite, opts),
        explain_statements("select 1", Dialect::Sqlite, plain())
    );
}

#[test]
fn trailing_semicolons_and_comments_are_dropped_with_care() {
    let e = |s: &str| explain_statements(s, Dialect::Postgres, plain());
    assert_eq!(
        e("-- c\nselect ';' ;; "),
        ["EXPLAIN (FORMAT JSON) select ';'"]
    );
    assert_eq!(
        e("select 1 -- tail ;\n;"),
        ["EXPLAIN (FORMAT JSON) select 1"]
    );
    assert!(e("  ; ").is_empty() && e("").is_empty() && e("-- only").is_empty());
}
