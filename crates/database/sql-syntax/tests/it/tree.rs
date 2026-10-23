//! The syntax tree: shapes, error recovery, expected sets, holes, queries.
use sql_syntax::{
    Dialect, Expected, HOLE, NameKind, Script, SyntaxErrorKind, SyntaxKind, SyntaxNode, parse,
};

use SyntaxKind as K;

fn dump(script: &Script) -> String {
    script.statements().map(|r| r.dump()).collect()
}

/// The first node of `kind` in the script.
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

fn child_kinds(node: SyntaxNode<'_>) -> Vec<SyntaxKind> {
    node.child_nodes().map(|n| n.kind()).collect()
}

fn select_clauses(script: &Script) -> Vec<SyntaxKind> {
    child_kinds(find(script, K::Select).expect("a select"))
}

#[test]
fn mid_typing_qualifier_keeps_from_intact() {
    let src = "select c. from customers c";
    let expected_tree = "\
Root@0..26
  Query@0..26
    Select@0..26
      Word \"select\"
      SelectList@7..9
        SelectItem@7..9
          ColumnRef@7..9
            TableRef@7..8
              Word \"c\"
            Dot \".\"
            Missing@9..9 ['*', column name]
      FromClause@10..26
        Word \"from\"
        TableSource@15..26
          TableRef@15..24
            Word \"customers\"
          AliasDef@25..26
            Word \"c\"
";
    for d in Dialect::ALL {
        let s = parse(src, d);
        assert_eq!(dump(&s), expected_tree, "{d:?}");
        assert_eq!(
            s.expected_at(9),
            vec![Expected::Punct("*"), Expected::Name(NameKind::Column)],
            "{d:?}"
        );
        let errors = s.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, SyntaxErrorKind::Missing);
        assert_eq!(errors[0].range, 9..9);
    }
}

#[test]
fn unclosed_parenthesis_keeps_the_rest() {
    for src in [
        "select (a + b from t where x = 1",
        "select count(a from t where x = 1",
        "select coalesce(a, b from t where x = 1",
    ] {
        let s = parse(src, Dialect::Postgres);
        assert_eq!(
            select_clauses(&s),
            [K::SelectList, K::FromClause, K::WhereClause],
            "{src}"
        );
        let errors = s.errors();
        assert_eq!(errors.len(), 1, "{src}: {errors:?}");
        assert!(
            errors[0].message(src).starts_with("Expected: ')'"),
            "{errors:?}"
        );
        let at = src.find(" from").unwrap();
        assert_eq!(errors[0].range, at..at);
        assert_eq!(find(&s, K::TableRef).unwrap().text(), "t");
    }
    // In FROM too.
    let s = parse(
        "select * from (select a from t where x = 1",
        Dialect::Postgres,
    );
    assert_eq!(count(&s, K::SubquerySource), 1);
    assert_eq!(s.errors().len(), 1);
}

#[test]
fn missing_join_condition_keeps_the_rest() {
    let s = parse("select * from a join b where x = 1", Dialect::Postgres);
    assert_eq!(
        select_clauses(&s),
        [K::SelectList, K::FromClause, K::WhereClause]
    );
    let join = find(&s, K::Join).unwrap();
    assert_eq!(join.text(), "a join b");
    let missing = join.child(K::Missing).unwrap();
    assert!(missing.expected().contains(&Expected::Keyword("ON")));
    assert!(missing.expected().contains(&Expected::Keyword("USING")));

    let s = parse("select * from a join b on where x = 1", Dialect::Postgres);
    let on = find(&s, K::OnClause).unwrap();
    assert_eq!(child_kinds(on), [K::Missing]);
    assert_eq!(count(&s, K::WhereClause), 1);
    assert_eq!(s.errors().len(), 1);

    // SQLite and generic SQL allow a join without a condition.
    for d in [Dialect::Sqlite, Dialect::Generic] {
        assert!(
            parse("select * from a join b where x = 1", d)
                .errors()
                .is_empty()
        );
    }
}

#[test]
fn trailing_where_and_trailing_comma_keep_the_rest() {
    let s = parse("select * from t where", Dialect::Postgres);
    assert_eq!(
        select_clauses(&s),
        [K::SelectList, K::FromClause, K::WhereClause]
    );
    let where_ = find(&s, K::WhereClause).unwrap();
    assert_eq!(child_kinds(where_), [K::Missing]);
    assert!(
        where_
            .child(K::Missing)
            .unwrap()
            .expected()
            .contains(&Expected::Expression)
    );

    let s = parse("select a, b, from t order by a", Dialect::Postgres);
    assert_eq!(select_clauses(&s), [K::SelectList, K::FromClause]);
    let list = find(&s, K::SelectList).unwrap();
    assert_eq!(list.child_nodes().count(), 3);
    assert_eq!(count(&s, K::OrderByClause), 1);
    assert_eq!(s.errors().len(), 1);
    assert_eq!(s.errors()[0].range, 12..12);
}

#[test]
fn stray_tokens_become_errors_and_parsing_resumes() {
    let s = parse("select a b c d from t where x = 1", Dialect::Postgres);
    assert_eq!(count(&s, K::FromClause), 1);
    assert_eq!(count(&s, K::WhereClause), 1);
    let errors = s.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, SyntaxErrorKind::Unexpected);
    assert_eq!(errors[0].message(s.text()), "Unexpected 'c'");

    // A clause out of its place is parsed inside an error.
    let s = parse("select a from t group by a where b = 1", Dialect::Postgres);
    let err = find(&s, K::Error).unwrap();
    assert_eq!(child_kinds(err), [K::WhereClause]);

    // A stray `)`.
    let s = parse("select a) from t", Dialect::Postgres);
    assert_eq!(s.errors()[0].message(s.text()), "Unexpected ')'");
    assert_eq!(count(&s, K::FromClause), 1);

    // Between items of a list, up to the next comma.
    let s = parse("select a, b c d, e from t", Dialect::Postgres);
    assert_eq!(
        find(&s, K::SelectList)
            .unwrap()
            .child(K::Error)
            .unwrap()
            .text(),
        "d"
    );
    assert_eq!(count(&s, K::SelectItem), 3);
}

#[test]
fn unknown_statements_are_one_opaque_node_without_errors() {
    let cases = [
        (
            Dialect::Postgres,
            "create table t (a int primary key, b text)",
        ),
        (
            Dialect::Postgres,
            "insert into t (a) values (1) on conflict do nothing",
        ),
        (Dialect::Postgres, "do $$ begin raise notice 'x'; end $$"),
        (Dialect::Postgres, "explain analyze select 1"),
        (Dialect::Oracle, "begin\n  dbms_output.put_line('x');\nend;"),
        (
            Dialect::Oracle,
            "create or replace procedure p is begin null; end;",
        ),
        (Dialect::Sqlite, "pragma table_info(t)"),
        (Dialect::Generic, "frobnicate the widgets ((("),
    ];
    for (d, src) in cases {
        let s = parse(src, d);
        assert!(s.errors().is_empty(), "{d:?} {src}: {:?}", s.errors());
        let root = s.statements().next().unwrap();
        assert_eq!(child_kinds(root), [K::Opaque], "{src}");
        assert_eq!(s.print(), src);
    }
    // A data-modifying statement after WITH: the WITH clause is parsed.
    let s = parse(
        "with x as (select 1) insert into t select * from x",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty());
    let opaque = find(&s, K::Opaque).unwrap();
    assert_eq!(child_kinds(opaque), [K::WithClause]);
    assert_eq!(count(&s, K::CteDef), 1);
}

#[test]
fn unknown_clauses_are_opaque() {
    for (d, src) in [
        (
            Dialect::Postgres,
            "select * from t tablesample system (10) where a = 1",
        ),
        (
            Dialect::Postgres,
            "select * from t as x tablesample bernoulli (5) repeatable (1)",
        ),
        (Dialect::Oracle, "select * from t sample (10) where a = 1"),
        (
            Dialect::Oracle,
            "select max(a) keep (dense_rank last order by b) from t",
        ),
        (
            Dialect::Generic,
            "select * from t qualify row_number() over (order by a) = 1",
        ),
        (
            Dialect::Generic,
            "select listagg(a, ',' on overflow truncate) from t",
        ),
    ] {
        let s = parse(src, d);
        assert!(s.errors().is_empty(), "{d:?} {src}: {:?}", s.errors());
        assert!(count(&s, K::Opaque) >= 1, "{src}");
    }
}

#[test]
fn names_are_typed_by_position() {
    let src = "select s.t.c, t.c, c, f(x), s.g(y), t.*, cast(a as s.my_type) from db.s.t join u as v on t.id = v.id";
    let s = parse(src, Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let names: Vec<(SyntaxKind, &str)> = s
        .statements()
        .next()
        .unwrap()
        .descendants()
        .filter(|n| n.kind().is_name())
        .map(|n| (n.kind(), n.text()))
        .collect();
    let expect = [
        (K::ColumnRef, "s.t.c"),
        (K::TableRef, "s.t"),
        (K::SchemaRef, "s"),
        (K::ColumnRef, "t.c"),
        (K::TableRef, "t"),
        (K::ColumnRef, "c"),
        (K::FunctionRef, "f"),
        (K::ColumnRef, "x"),
        (K::FunctionRef, "s.g"),
        (K::SchemaRef, "s"),
        (K::ColumnRef, "y"),
        (K::TableRef, "t"),
        (K::ColumnRef, "a"),
        (K::TypeRef, "s.my_type"),
        (K::SchemaRef, "s"),
        (K::TableRef, "db.s.t"),
        (K::SchemaRef, "db.s"),
        (K::CatalogRef, "db"),
        (K::TableRef, "u"),
        (K::AliasDef, "as v"),
        (K::ColumnRef, "t.id"),
        (K::TableRef, "t"),
        (K::ColumnRef, "v.id"),
        (K::TableRef, "v"),
    ];
    assert_eq!(names, expect);
}

#[test]
fn ctes_and_windows_define_names() {
    let s = parse(
        "with recursive r (n) as (select 1 union all select n + 1 from r where n < 5) \
         select n, sum(n) over w from r window w as (order by n)",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(find(&s, K::CteDef).unwrap().text(), "r");
    assert_eq!(find(&s, K::ColumnAliasList).unwrap().text(), "(n)");
    assert_eq!(find(&s, K::WindowDef).unwrap().text(), "w");
    assert_eq!(find(&s, K::WindowRef).unwrap().text(), "w");
    assert_eq!(count(&s, K::SetOperation), 1);
}

#[test]
fn expected_sets_for_completion() {
    let s = parse("select * from ", Dialect::Postgres);
    let e = s.expected_at(14);
    assert!(e.contains(&Expected::Name(NameKind::Table)), "{e:?}");
    assert!(e.contains(&Expected::Keyword("LATERAL")));
    assert!(!e.contains(&Expected::Expression));

    // A word being typed is the prefix: what may stand there counts.
    let s = parse("select * from cust", Dialect::Postgres);
    assert!(s.expected_at(18).contains(&Expected::Name(NameKind::Table)));
    let s = parse("select c.na from customers c", Dialect::Postgres);
    assert_eq!(
        s.expected_at(11),
        vec![Expected::Punct("*"), Expected::Name(NameKind::Column)]
    );

    // After a complete table: alias, joins, clauses.
    let s = parse("select a from t ", Dialect::Postgres);
    let e = s.expected_at(16);
    for k in [
        "WHERE", "GROUP", "ORDER", "JOIN", "LEFT", "UNION", "LIMIT", "AS",
    ] {
        assert!(e.contains(&Expected::Keyword(k)), "{k} in {e:?}");
    }
    assert!(e.contains(&Expected::Name(NameKind::Alias)));
    assert!(!e.contains(&Expected::Keyword("CONNECT")), "not PostgreSQL");
    let s = parse("select a from t ", Dialect::Oracle);
    assert!(s.expected_at(16).contains(&Expected::Keyword("CONNECT")));

    // Expressions.
    let s = parse("select * from t where a = 1 and ", Dialect::Postgres);
    let e = s.expected_at(32);
    assert!(e.contains(&Expected::Expression));
    assert!(e.contains(&Expected::Keyword("NOT")));
    let s = parse("select ", Dialect::Generic);
    let e = s.expected_at(7);
    assert!(e.contains(&Expected::Expression) && e.contains(&Expected::Punct("*")));
    assert!(e.contains(&Expected::Keyword("DISTINCT")));
    let s = parse("select a from t where a ", Dialect::Postgres);
    let e = s.expected_at(24);
    for k in ["AND", "OR", "IS", "IN", "LIKE", "BETWEEN", "NOT", "ORDER"] {
        assert!(e.contains(&Expected::Keyword(k)), "{k} in {e:?}");
    }

    // Text after the caret does not count.
    let s = parse("select  from customers", Dialect::Generic);
    assert!(s.expected_at(7).contains(&Expected::Expression));

    // Statement starts: empty text, after a terminator.
    for (src, at) in [("", 0), ("select 1; ", 10), ("  -- c\n", 7)] {
        let s = parse(src, Dialect::Postgres);
        let e = s.expected_at(at);
        assert!(e.contains(&Expected::Keyword("SELECT")), "{src:?}: {e:?}");
        assert!(e.contains(&Expected::Keyword("WITH")));
    }

    // Join condition.
    let s = parse("select * from o join c ", Dialect::Postgres);
    let e = s.expected_at(23);
    assert!(e.contains(&Expected::Keyword("ON")) && e.contains(&Expected::Keyword("USING")));
    let s = parse("select * from o join c on ", Dialect::Postgres);
    assert!(s.expected_at(26).contains(&Expected::Expression));
}

#[test]
fn node_at_and_enclosing() {
    let src = "select c.name from customers c where c.id = 1";
    let s = parse(src, Dialect::Postgres);
    let at = src.find("customers").unwrap() + 3;
    let n = s.node_at(at).unwrap();
    assert_eq!(n.kind(), K::TableRef);
    assert_eq!(n.text(), "customers");
    assert_eq!(
        n.enclosing(K::FromClause).unwrap().text(),
        "from customers c"
    );
    assert_eq!(s.enclosing(at, K::Select).unwrap().range(), 0..src.len());
    let at = src.find("c.id").unwrap() + 2;
    assert_eq!(s.node_at(at).unwrap().kind(), K::ColumnRef);
    assert_eq!(s.token_at(at).unwrap().text(), "id");
    assert_eq!(
        s.token_at(at).unwrap().parent().unwrap().kind(),
        K::ColumnRef
    );
    // The missing name after `c.` is found at the caret.
    let s = parse("select c. from customers c", Dialect::Postgres);
    assert_eq!(s.node_at(9).unwrap().kind(), K::Missing);
    assert_eq!(s.enclosing(9, K::ColumnRef).unwrap().text(), "c.");
}

#[test]
fn holes_stand_for_names_expressions_items_and_clauses() {
    let h = HOLE;
    for src in [
        format!("select {h} from {h}"),
        format!("select a, {h} from t where x = {h} and y in ({h}, {h})"),
        format!("select * from {h}.orders o join {h}t on o.id = t.id"),
        format!("select * from t {h}"),
        format!("select * from t where a = 1 {h} order by {h}"),
        format!("select x{h}y, {h}.c from t"),
        format!("{h}"),
    ] {
        for d in Dialect::ALL {
            let s = parse(&src, d);
            assert!(s.errors().is_empty(), "{d:?} {src:?}: {:?}", s.errors());
            assert!(count(&s, K::Hole) >= 1, "{src:?}");
            assert_eq!(s.print(), src);
        }
    }
    let src = format!("select * from {h}.orders");
    let s = parse(&src, Dialect::Postgres);
    let schema = find(&s, K::SchemaRef).unwrap();
    assert_eq!(child_kinds(schema), [K::Hole]);
}

#[test]
fn dialect_constructs() {
    let cases: &[(Dialect, &str, &[SyntaxKind])] = &[
        (
            Dialect::Postgres,
            "select a::int, b::numeric(10,2)[] from t",
            &[K::CastExpr, K::TypeRef],
        ),
        (
            Dialect::Postgres,
            "select distinct on (a) a from t",
            &[K::DistinctClause],
        ),
        (
            Dialect::Postgres,
            "select * from t, lateral (select 1) x",
            &[K::SubquerySource],
        ),
        (
            Dialect::Postgres,
            "select a from t where b ilike 'x'",
            &[K::LikeExpr],
        ),
        (
            Dialect::Postgres,
            "select a[1], a[1:2] from t",
            &[K::SubscriptExpr],
        ),
        (Dialect::Postgres, "select array[1, 2]", &[K::ArrayExpr]),
        (
            Dialect::Postgres,
            "select count(*) filter (where a > 1) from t",
            &[K::FilterClause],
        ),
        (
            Dialect::Postgres,
            "select a from t limit 1 offset 2",
            &[K::LimitClause, K::OffsetClause],
        ),
        (
            Dialect::Postgres,
            "select a from t fetch first 2 rows only",
            &[K::FetchClause],
        ),
        (
            Dialect::Oracle,
            "select level from dual connect by level < 3",
            &[K::HierarchicalClause],
        ),
        (
            Dialect::Oracle,
            "select * from a, b where a.id = b.id(+)",
            &[K::OuterJoinMark],
        ),
        (
            Dialect::Oracle,
            "select * from t pivot (sum(x) for y in (1 as a))",
            &[K::PivotClause],
        ),
        (
            Dialect::Oracle,
            "select a from t minus select a from u",
            &[K::SetOperation],
        ),
        (
            Dialect::Oracle,
            "select listagg(a, ',') within group (order by a) from t",
            &[K::WithinGroup],
        ),
        (
            Dialect::Sqlite,
            "select a from t where a glob 'x*'",
            &[K::LikeExpr],
        ),
        (
            Dialect::Sqlite,
            "select a from t limit 1, 2",
            &[K::LimitClause],
        ),
        (
            Dialect::Sqlite,
            "select a from t where a isnull",
            &[K::IsExpr],
        ),
    ];
    for (d, src, kinds) in cases {
        let s = parse(src, *d);
        assert!(s.errors().is_empty(), "{d:?} {src}: {:?}", s.errors());
        for k in *kinds {
            assert!(count(&s, *k) >= 1, "{k:?} in {src}:\n{}", dump(&s));
        }
    }
    // The same constructs are errors where the dialect does not have them.
    for (d, src) in [
        (
            Dialect::Postgres,
            "select level from dual connect by level < 3",
        ),
        (Dialect::Oracle, "select a from t limit 1"),
        (Dialect::Sqlite, "select a from t where b ilike 'x'"),
    ] {
        assert!(!parse(src, d).errors().is_empty(), "{d:?} {src}");
    }
}

#[test]
fn expressions_follow_operator_precedence() {
    let s = parse("select a or b and not c = d + e * f", Dialect::Postgres);
    let item = find(&s, K::SelectItem).unwrap();
    let top = item.child_nodes().next().unwrap();
    assert_eq!(top.kind(), K::BinaryExpr);
    // `a or (...)`
    let parts: Vec<&str> = top.child_nodes().map(|n| n.text()).collect();
    assert_eq!(parts, ["a", "b and not c = d + e * f"]);
    let s = parse(
        "select x from t where a between 1 and 2 and b",
        Dialect::Postgres,
    );
    let w = find(&s, K::WhereClause)
        .unwrap()
        .child_nodes()
        .next()
        .unwrap();
    let parts: Vec<&str> = w.child_nodes().map(|n| n.text()).collect();
    assert_eq!(parts, ["a between 1 and 2", "b"]);
    let s = parse("select -a.b::int from t", Dialect::Postgres);
    let u = find(&s, K::UnaryExpr).unwrap();
    assert_eq!(child_kinds(u), [K::CastExpr]);
}

#[test]
fn printing_is_lossless_and_errors_lie_on_boundaries() {
    let src = "select 'é;' as \"ñ\", -- note\n  /* c */ x from t where a = $1;\nselect q from u;; \n-- end\n";
    for d in Dialect::ALL {
        let s = parse(src, d);
        assert_eq!(s.print(), src);
        assert_eq!(s.statements().count(), 2);
        let ranges: Vec<_> = s.statements().map(|r| r.range()).collect();
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, ranges[1].start);
        assert_eq!(ranges[1].end, src.len());
    }
}

#[test]
fn deep_nesting_does_not_overflow() {
    let deep = format!("select {}1{} from t", "(".repeat(5000), ")".repeat(5000));
    let minus = format!("select {}1 from t", "-".repeat(5000));
    let not = format!("select * from t where {}a", "not ".repeat(3000));
    let subq = format!("select * from {}t", "(select * from ".repeat(2000));
    for src in [deep, minus, not, subq] {
        for d in Dialect::ALL {
            let s = parse(&src, d);
            assert_eq!(s.print(), src);
        }
    }
}

#[test]
fn reparse_handles_statement_boundaries() {
    let cases: &[(Dialect, &str, std::ops::Range<usize>, &str)] = &[
        // Removing a `;` joins two statements, adding one splits them.
        (
            Dialect::Postgres,
            "select 1;\nselect 2;\nselect 3;\n",
            8..9,
            "",
        ),
        (
            Dialect::Postgres,
            "select 1\nselect 2;\nselect 3;\n",
            8..8,
            ";",
        ),
        // Opening a string swallows the rest; closing it gives it back.
        (
            Dialect::Postgres,
            "select 1;\nselect 2;\nselect 3;\n",
            17..17,
            "'",
        ),
        (
            Dialect::Postgres,
            "select 'a;\nselect 2;\nselect 3;\n",
            9..9,
            "'",
        ),
        // A comment opener at the start of a statement.
        (
            Dialect::Generic,
            "select 1;\nselect 2;\nselect 3;\n",
            10..10,
            "/*",
        ),
        // Oracle: a `/` line changes where a PL/SQL block ends.
        (
            Dialect::Oracle,
            "begin\n  null;\nend;\nselect 1 from dual;\n",
            34..34,
            "\n/\n",
        ),
        (
            Dialect::Oracle,
            "begin\n  null;\nend;\n/\nselect 1 from dual;\n",
            19..21,
            "",
        ),
        // Everything removed, everything typed.
        (Dialect::Sqlite, "select 1; select 2", 0..18, ""),
        (Dialect::Sqlite, "", 0..0, "select 1; select 2"),
    ];
    for (d, src, range, text) in cases {
        let old = parse(src, *d);
        let edit = sql_syntax::TextEdit {
            range: range.clone(),
            text: text.to_string(),
        };
        let mut new_text = src.to_string();
        new_text.replace_range(range.clone(), text);
        assert_eq!(
            old.reparse(&edit),
            parse(&new_text, *d),
            "{d:?} {src:?} {range:?} {text:?}"
        );
    }
}
