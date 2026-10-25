//! Data modification: shapes, error recovery, expected sets.
use sql_syntax::{
    Dialect, Expected, NameKind, Script, SyntaxErrorKind, SyntaxKind, SyntaxNode, parse,
};

use SyntaxKind as K;

fn find<'a>(script: &'a Script, kind: SyntaxKind) -> Option<SyntaxNode<'a>> {
    script
        .statements()
        .flat_map(|r| r.descendants().collect::<Vec<_>>())
        .find(|n| n.kind() == kind)
}

fn count(script: &Script, kind: SyntaxKind) -> usize {
    script
        .statements()
        .map(|r| r.descendants().filter(|n| n.kind() == kind).count())
        .sum()
}

fn kinds(node: SyntaxNode<'_>) -> Vec<SyntaxKind> {
    node.child_nodes().map(|n| n.kind()).collect()
}

fn statement(script: &Script) -> SyntaxNode<'_> {
    script
        .statements()
        .next()
        .unwrap()
        .child_nodes()
        .next()
        .unwrap()
}

#[test]
fn insert_has_its_parts_as_children() {
    let src = "insert into s.t as x (a, b) values (1, 2) on conflict (a) do update set b = excluded.b where x.b < 3 returning a, b";
    let s = parse(src, Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let insert = statement(&s);
    assert_eq!(insert.kind(), K::Insert);
    assert_eq!(
        kinds(insert),
        [
            K::TableSource,
            K::ColumnList,
            K::Values,
            K::OnConflictClause,
            K::ReturningClause
        ]
    );
    let conflict = find(&s, K::OnConflictClause).unwrap();
    assert_eq!(
        kinds(conflict),
        [K::ConflictTarget, K::SetClause, K::WhereClause]
    );
    let target = find(&s, K::TableSource).unwrap();
    assert_eq!(kinds(target), [K::TableRef, K::AliasDef]);
    let set = find(&s, K::SetClause).unwrap();
    assert_eq!(kinds(set), [K::Assignment]);
    assert_eq!(count(&s, K::ColumnRef), 8);
}

#[test]
fn update_and_delete_have_their_clauses() {
    let s = parse(
        "update t set a = 1, (b, c) = (2, 3) from u where t.id = u.id returning t.id",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let update = statement(&s);
    assert_eq!(update.kind(), K::Update);
    assert_eq!(
        kinds(update),
        [
            K::TableSource,
            K::SetClause,
            K::FromClause,
            K::WhereClause,
            K::ReturningClause
        ]
    );
    let set = find(&s, K::SetClause).unwrap();
    assert_eq!(kinds(set), [K::Assignment, K::Assignment]);
    let second = set.child_nodes().nth(1).unwrap();
    assert_eq!(kinds(second)[0], K::ColumnList);

    let s = parse("delete from t using u where t.id = u.id", Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(
        kinds(statement(&s)),
        [K::TableSource, K::UsingSource, K::WhereClause]
    );
    let s = parse("delete t where id = 1", Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(kinds(statement(&s)), [K::TableSource, K::WhereClause]);
}

#[test]
fn a_cte_is_the_first_child_of_the_statement_it_serves() {
    let s = parse(
        "with x as (select 1) update t set a = 1 from x",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(kinds(statement(&s))[0], K::WithClause);
    assert_eq!(statement(&s).kind(), K::Update);
    // A data-modifying statement inside a CTE is parsed too.
    let s = parse(
        "with d as (delete from t where id < 10 returning *) select * from d",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(count(&s, K::Delete), 1);
    assert_eq!(count(&s, K::Opaque), 0);
}

#[test]
fn merge_clauses_are_nodes() {
    let src = "merge into t using s on (t.id = s.id) when matched then update set a = s.a delete where t.b = 1 when not matched then insert (id) values (s.id)";
    let s = parse(src, Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let merge = statement(&s);
    assert_eq!(merge.kind(), K::Merge);
    assert_eq!(
        kinds(merge),
        [
            K::TableSource,
            K::UsingSource,
            K::OnClause,
            K::MergeClause,
            K::MergeClause
        ]
    );
    let first = merge.child_nodes().nth(3).unwrap();
    assert_eq!(kinds(first), [K::MergeUpdate]);
    let update = first.child_nodes().next().unwrap();
    assert_eq!(kinds(update), [K::SetClause, K::WhereClause]);
    let second = merge.child_nodes().nth(4).unwrap();
    assert_eq!(kinds(second), [K::MergeInsert]);
}

#[test]
fn multi_table_insert_has_into_clauses() {
    let src = "insert all when x > 1 then into t1 (a) values (x) into t1b (a) values (x) else into t3 (c) values (x) select x from s";
    let s = parse(src, Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let insert = statement(&s);
    assert_eq!(kinds(insert), [K::WhenClause, K::ElseClause, K::Query]);
    assert_eq!(count(&s, K::InsertInto), 3);
    // The same text is not an insert in PostgreSQL.
    let s = parse(src, Dialect::Postgres);
    assert!(!s.errors().is_empty());
}

#[test]
fn returning_into_collects_variables() {
    let s = parse(
        "update t set a = 1 returning a, b into v1, :v2",
        Dialect::Oracle,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let returning = find(&s, K::ReturningClause).unwrap();
    assert_eq!(
        kinds(returning),
        [K::SelectItem, K::SelectItem, K::IntoClause]
    );
}

#[test]
fn dialect_flags_turn_constructs_off() {
    // `ON CONFLICT` is not Oracle; `MERGE` is not SQLite.
    let s = parse(
        "insert into t (a) values (1) on conflict do nothing",
        Dialect::Oracle,
    );
    assert!(!s.errors().is_empty());
    let s = parse(
        "merge into t using s on (t.id = s.id) when matched then delete",
        Dialect::Sqlite,
    );
    assert_eq!(statement(&s).kind(), K::Opaque);
    assert!(s.errors().is_empty());
    // `RETURNING` is not an Oracle SQL clause of a `SELECT`, but the DML forms have it.
    let s = parse("delete from t returning a into :b", Dialect::Sqlite);
    assert!(!s.errors().is_empty());
}

#[test]
fn unfinished_statements_keep_what_was_typed() {
    let cases = [
        ("insert into t (a, ", "column name"),
        ("update t set ", "column name"),
        ("update t set a = ", "expression"),
        ("delete from ", "table name"),
        ("delete from t where ", "expression"),
        ("insert into ", "table name"),
    ];
    for (src, want) in cases {
        let s = parse(src, Dialect::Postgres);
        let errors = s.errors();
        assert!(!errors.is_empty(), "{src}");
        assert_eq!(errors[0].kind, SyntaxErrorKind::Missing, "{src}");
        assert!(
            errors[0].message(src).contains(want),
            "{src}: {}",
            errors[0].message(src)
        );
        assert_eq!(s.print(), src);
    }
    // Everything typed before and after the gap stays in the tree.
    let s = parse("update t set a = , b = 2 where id = 1", Dialect::Postgres);
    assert_eq!(count(&s, K::Assignment), 2);
    assert_eq!(count(&s, K::WhereClause), 1);
    let s = parse("insert into t (a, , b) values (1, 2)", Dialect::Postgres);
    assert_eq!(count(&s, K::ColumnRef), 2);
    assert_eq!(count(&s, K::Values), 1);
}

#[test]
fn expected_sets_in_data_modification() {
    let at = |src: &str, d: Dialect| parse(src, d).expected_at(src.len());
    assert!(at("insert into ", Dialect::Postgres).contains(&Expected::Name(NameKind::Table)));
    assert!(at("insert into t (", Dialect::Postgres).contains(&Expected::Name(NameKind::Column)));
    let e = at("insert into t (a) ", Dialect::Postgres);
    assert!(e.contains(&Expected::Keyword("VALUES")), "{e:?}");
    assert!(e.contains(&Expected::Keyword("SELECT")), "{e:?}");
    let e = at("update t ", Dialect::Postgres);
    assert!(e.contains(&Expected::Keyword("SET")), "{e:?}");
    assert!(at("update t set ", Dialect::Postgres).contains(&Expected::Name(NameKind::Column)));
    let e = at("delete ", Dialect::Postgres);
    assert!(e.contains(&Expected::Keyword("FROM")), "{e:?}");
    let e = at("delete from t ", Dialect::Postgres);
    assert!(e.contains(&Expected::Keyword("WHERE")), "{e:?}");
    assert!(e.contains(&Expected::Keyword("USING")), "{e:?}");
    assert!(e.contains(&Expected::Keyword("RETURNING")), "{e:?}");
    assert!(at("delete from t where ", Dialect::Postgres).contains(&Expected::Expression));
}

#[test]
fn an_error_in_one_statement_does_not_reach_the_next() {
    let src = "update t set = 1; insert into u (a) values (1); delete from where; select 1";
    let s = parse(src, Dialect::Postgres);
    let kinds: Vec<SyntaxKind> = s
        .statements()
        .map(|r| r.child_nodes().next().unwrap().kind())
        .collect();
    assert_eq!(kinds, [K::Update, K::Insert, K::Delete, K::Query]);
    // The insert and the select are clean.
    let roots: Vec<_> = s.statements().collect();
    assert!(!roots[1].descendants().any(|n| n.kind() == K::Error));
    assert!(!roots[3].descendants().any(|n| n.kind() == K::Error));
}
