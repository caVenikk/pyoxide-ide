use sql_syntax::{CandidateKind, Dialect, candidates_at};

const PG: Dialect = Dialect::Postgres;

fn at(script: &str, marker: &str, offset: usize) -> Vec<(CandidateKind, String)> {
    let caret = script.find(marker).unwrap() + offset;
    candidates_at(script, caret, PG)
        .into_iter()
        .map(|c| (c.kind, script[c.range].to_string()))
        .collect()
}

#[test]
fn innermost_select_then_statement_then_script() {
    let s = "select 1;\nselect * from (select a from (select 1 as a) x) y where b in (select 2);\nselect 3";
    let c = at(s, "select 1 as a", 3);
    assert_eq!(
        c,
        [
            (CandidateKind::Subquery, "select 1 as a".to_string()),
            (
                CandidateKind::Subquery,
                "select a from (select 1 as a) x".to_string()
            ),
            (
                CandidateKind::Statement,
                s.lines().nth(1).unwrap().trim_end_matches(';').to_string()
            ),
            (CandidateKind::Script, s.trim_end().to_string()),
        ]
    );
    // Outside the parentheses of the subquery: only the statement and the script.
    let c = at(s, "where b", 0);
    assert_eq!(
        c.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        [CandidateKind::Statement, CandidateKind::Script]
    );
    let c = at(s, "select 2", 2);
    assert_eq!(c[0], (CandidateKind::Subquery, "select 2".to_string()));
}

#[test]
fn the_edges_of_the_parentheses_belong_to_the_subquery() {
    let s = "select * from (select 1) t";
    let open = s.find('(').unwrap();
    assert_eq!(at(s, "(", 0)[0].0, CandidateKind::Statement);
    assert_eq!(
        candidates_at(s, open + 1, PG)[0].kind,
        CandidateKind::Subquery
    );
    let close = s.find(')').unwrap();
    assert_eq!(candidates_at(s, close, PG)[0].kind, CandidateKind::Subquery);
    assert_eq!(
        candidates_at(s, close + 1, PG)[0].kind,
        CandidateKind::Statement
    );
}

#[test]
fn a_single_statement_is_not_offered_twice() {
    let c = at("select 1", "1", 0);
    assert_eq!(c, [(CandidateKind::Statement, "select 1".to_string())]);
}

#[test]
fn parentheses_that_are_not_selects_do_not_count() {
    let c = at(
        "select count(*), (a + b) from t where x in (1, 2)",
        "a + b",
        1,
    );
    assert_eq!(c.len(), 1);
    let c = at("insert into t (a, b) values (1, 2)", "1, 2", 1);
    assert_eq!(c.len(), 1);
    // `WITH` and `VALUES` subqueries do.
    let c = at(
        "select * from (with x as (select 1) select * from x) y",
        "select * from x",
        3,
    );
    assert_eq!(
        c[0],
        (
            CandidateKind::Subquery,
            "with x as (select 1) select * from x".to_string()
        )
    );
    assert_eq!(c[1].0, CandidateKind::Statement);
}

#[test]
fn an_unclosed_parenthesis_reads_to_the_end() {
    let s = "select * from (select a from t where";
    let c = at(s, "a from", 0);
    assert_eq!(
        c[0],
        (CandidateKind::Subquery, "select a from t where".to_string())
    );
}

#[test]
fn no_statements_no_candidates() {
    assert!(candidates_at("", 0, PG).is_empty());
    assert!(candidates_at("-- nothing\n", 4, PG).is_empty());
}

#[test]
fn statement_of_takes_a_range_as_one_statement() {
    use sql_syntax::statement_of;
    let s = "select 1;\n  update t set a = 1;  \nselect 3";
    let from = s.find("select 1").unwrap();
    let to = s.find("select 3").unwrap();
    let one = statement_of(s, from..to, PG).unwrap();
    assert_eq!(one.text, "select 1;\n  update t set a = 1");
    assert_eq!(&s[one.range.clone()], one.text);
    assert!(statement_of(s, 9..12, PG).is_none());
}
