//! Procedural code: PL/SQL blocks, PL/pgSQL bodies as embedded fragments,
//! trigger bodies, containment of errors.
use sql_syntax::{
    Dialect, Expected, FragmentLanguage, Script, SyntaxKind, SyntaxNode, TextEdit, parse, split,
};

use crate::statements::cases;
use SyntaxKind as K;

const PLSQL: &str = include_str!("corpus/plsql.sql");

fn statement(script: &Script) -> SyntaxNode<'_> {
    script
        .statements()
        .next()
        .unwrap()
        .child_nodes()
        .next()
        .unwrap()
}

fn kinds(node: SyntaxNode<'_>) -> Vec<SyntaxKind> {
    node.child_nodes().map(|n| n.kind()).collect()
}

fn count(script: &Script, kind: SyntaxKind) -> usize {
    script
        .statements()
        .map(|r| r.descendants().filter(|n| n.kind() == kind).count())
        .sum()
}

fn find<'a>(script: &'a Script, kind: SyntaxKind) -> Option<SyntaxNode<'a>> {
    script
        .statements()
        .flat_map(|r| r.descendants().collect::<Vec<_>>())
        .find(|n| n.kind() == kind)
}

#[test]
fn a_plsql_block_has_its_sections() {
    let src = "declare\n  x number := 1;\nbegin\n  x := x + 1;\nexception\n  when others then null;\nend;";
    let s = parse(src, Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let block = statement(&s);
    assert_eq!(block.kind(), K::Block);
    assert_eq!(
        kinds(block),
        [K::DeclareSection, K::StatementList, K::ExceptionSection]
    );
    let decl = block.child(K::DeclareSection).unwrap();
    assert_eq!(kinds(decl), [K::VarDecl]);
    let stmts = block.child(K::StatementList).unwrap();
    assert_eq!(kinds(stmts), [K::AssignStmt]);
    let handler = find(&s, K::ExceptionHandler).unwrap();
    assert_eq!(kinds(handler), [K::ObjectRef, K::StatementList]);
}

#[test]
fn control_flow_statements_are_nodes() {
    let src = "begin\n if a then x := 1; elsif b then x := 2; else x := 3; end if;\n case when a then null; end case;\n while a loop exit when b; end loop;\n for i in 1..3 loop continue; end loop;\n loop null; end loop;\nend;";
    let s = parse(src, Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let list = find(&s, K::StatementList).unwrap();
    assert_eq!(
        kinds(list),
        [
            K::IfStmt,
            K::CaseStmt,
            K::WhileStmt,
            K::ForStmt,
            K::LoopStmt
        ]
    );
    let if_stmt = list.child(K::IfStmt).unwrap();
    assert_eq!(
        kinds(if_stmt),
        [
            K::ColumnRef,
            K::StatementList,
            K::ElsifClause,
            K::ElseClause
        ]
    );
}

#[test]
fn a_cursor_loop_and_attributes() {
    let src = "declare cursor c is select 1 from dual; r c%rowtype; begin open c; fetch c into r; exit when c%notfound; close c; end;";
    let s = parse(src, Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(count(&s, K::CursorDecl), 1);
    assert_eq!(count(&s, K::OpenStmt), 1);
    assert_eq!(count(&s, K::FetchStmt), 1);
    assert_eq!(count(&s, K::CloseStmt), 1);
    assert_eq!(count(&s, K::AttributeExpr), 1);
    // `%rowtype` belongs to the type reference.
    let r = find(&s, K::VarDecl).unwrap();
    let ty = r.child(K::TypeRef).unwrap();
    assert_eq!(ty.text(), "c%rowtype");
}

#[test]
fn select_into_collects_variables_in_blocks_only() {
    let s = parse("begin select a, b into x, y from t; end;", Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let into = find(&s, K::IntoClause).unwrap();
    assert_eq!(count(&s, K::TableRef), 1);
    assert_eq!(
        into.child_nodes().map(|n| n.kind()).collect::<Vec<_>>(),
        [K::ColumnRef, K::ColumnRef]
    );
    // `BULK COLLECT INTO`.
    let s = parse(
        "begin select id bulk collect into ids from t; end;",
        Dialect::Oracle,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(count(&s, K::IntoClause), 1);
    // Outside a block, `SELECT INTO` names a table (PostgreSQL).
    let s = parse("select 1 into t2 from t", Dialect::Postgres);
    assert_eq!(
        find(&s, K::IntoClause)
            .unwrap()
            .child_nodes()
            .next()
            .unwrap()
            .kind(),
        K::TableRef
    );
}

#[test]
fn trigger_correlation_names_are_params() {
    let s = parse(
        "create trigger tr before insert on t for each row begin :new.a := :old.a + 1; end;",
        Dialect::Oracle,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(count(&s, K::Param), 2);
    let trigger = statement(&s);
    assert_eq!(
        kinds(trigger),
        [K::ObjectRef, K::TriggerEvent, K::TableRef, K::Block]
    );
}

#[test]
fn a_missing_end_if_stays_inside_the_block() {
    let src = "begin if x then y := 1; end;";
    let s = parse(src, Dialect::Oracle);
    let errors = s.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].message(src), "Expected: END IF");
    // The block still has its END; the IF is the unfinished part.
    let block = statement(&s);
    assert_eq!(block.kind(), K::Block);
    assert_eq!(block.text(), "begin if x then y := 1; end");
    let ifs = find(&s, K::IfStmt).unwrap();
    assert_eq!(ifs.text(), "if x then y := 1;");
}

#[test]
fn errors_inside_a_block_do_not_reach_the_statements_after_it() {
    let src = "begin x := ; if then end if; y := 2; end;\nselect 1 from dual;\nbegin null; end;";
    let s = parse(src, Dialect::Oracle);
    let units: Vec<_> = s.statements().collect();
    assert_eq!(units.len(), 3);
    assert!(units[0].descendants().any(|n| n.kind() == K::Missing));
    for u in &units[1..] {
        assert!(
            !u.descendants()
                .any(|n| matches!(n.kind(), K::Missing | K::Error)),
            "{}",
            u.dump()
        );
    }
    // And the good statements inside the broken block are in the tree.
    assert_eq!(count(&s, K::AssignStmt), 2);
}

#[test]
fn unknown_procedural_syntax_is_opaque_without_errors() {
    let src =
        "begin pipe row (r); commit; goto done; savepoint sp; get diagnostics n = row_count; end;";
    let s = parse(src, Dialect::Oracle);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert!(count(&s, K::Opaque) >= 3);
    assert_eq!(s.print(), src);
    let src = "do $$ begin get diagnostics n = row_count; assert n > 0, 'm'; notify c; end $$";
    let s = parse(src, Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn slash_terminated_plsql_splits_and_parses_as_one_statement_each() {
    let blocks: Vec<String> = cases(PLSQL, &[Dialect::Oracle])
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert!(blocks.len() >= 40);
    let script = blocks.join("\n/\n") + "\n/\n";
    let statements = split(&script, Dialect::Oracle);
    assert_eq!(statements.len(), blocks.len());
    let parsed = parse(&script, Dialect::Oracle);
    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.statements().count(), blocks.len());
    assert_eq!(parsed.print(), script);
    // Without the slashes the blocks are separated by their own `;`.
    let plain = blocks.join("\n");
    let parsed = parse(&plain, Dialect::Oracle);
    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    assert_eq!(parsed.statements().count(), blocks.len());
}

#[test]
fn a_function_body_in_dollar_quotes_is_an_embedded_fragment() {
    let src = "create function f(a int) returns int language plpgsql as $$\nbegin\n  if a > 1 then return a; end if;\n  return 0;\nend\n$$;\nselect 1;";
    let s = parse(src, Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let body = find(&s, K::EmbeddedBody).unwrap();
    let f = body.fragment().unwrap();
    assert_eq!(f.language, FragmentLanguage::PlPgSql);
    // The offsets are the script's: the body is the text inside the quotes.
    assert_eq!(
        &src[f.body.clone()],
        "\nbegin\n  if a > 1 then return a; end if;\n  return 0;\nend\n"
    );
    assert_eq!(f.root.range(), f.body);
    let block = f.root.child_nodes().next().unwrap();
    assert_eq!(block.kind(), K::Block);
    let if_stmt = block.descendants().find(|n| n.kind() == K::IfStmt).unwrap();
    assert_eq!(if_stmt.text(), "if a > 1 then return a; end if");
    assert_eq!(&src[if_stmt.range()], if_stmt.text());
    // The fragment is not part of the tree of the statement, the string is.
    assert!(!kinds(statement(&s)).contains(&K::Block));
    assert_eq!(s.print(), src);
    // node_at and token_at see into the body.
    let at = src.find("return a").unwrap() + 2;
    let node = s.node_at(at).unwrap();
    assert!(
        matches!(node.kind(), K::ReturnStmt | K::ColumnRef | K::Block),
        "{node:?}"
    );
    assert!(node.range().start >= f.body.start);
    assert_eq!(s.statements().count(), 2);
}

#[test]
fn the_language_decides_how_the_body_is_read() {
    let body = |src: &str, d: Dialect| {
        let s = parse(src, d);
        let n = find(&s, K::EmbeddedBody).unwrap_or_else(|| panic!("{src}"));
        n.fragment().map(|f| f.language)
    };
    let pg = Dialect::Postgres;
    assert_eq!(
        body(
            "create function f() returns int as $$ begin return 1; end $$ language plpgsql",
            pg
        ),
        Some(FragmentLanguage::PlPgSql)
    );
    assert_eq!(
        body(
            "create function f() returns int language sql as $$ select 1 $$",
            pg
        ),
        Some(FragmentLanguage::Sql)
    );
    assert_eq!(
        body(
            "create function f() returns int as 'select 1' language sql",
            pg
        ),
        Some(FragmentLanguage::Sql)
    );
    assert_eq!(
        body("do $$ begin null; end $$", pg),
        Some(FragmentLanguage::PlPgSql)
    );
    assert_eq!(
        body("do language plpgsql $x$ begin null; end $x$", pg),
        Some(FragmentLanguage::PlPgSql)
    );
    // Another language, no language, escapes to undo, a quote that is not closed: no fragment.
    assert_eq!(
        body(
            "create function f() returns int as $$ x = 1 $$ language plpython3u",
            pg
        ),
        None
    );
    assert_eq!(
        body("create function f() returns int as $$ select 1 $$", pg),
        None
    );
    assert_eq!(
        body(
            "create function f() returns int as 'select ''x''' language sql",
            pg
        ),
        None
    );
    assert_eq!(
        body(
            "create function f() returns int as $$ select 1 language sql",
            pg
        ),
        None
    );
    // PL/SQL has no strings as bodies.
    let s = parse(
        "create function f return number is begin return 1; end;",
        Dialect::Oracle,
    );
    assert!(find(&s, K::EmbeddedBody).is_none());
}

#[test]
fn a_sql_function_body_is_a_list_of_statements() {
    let src = "create function f() returns void language sql as $$ insert into t values (1); delete from u; $$";
    let s = parse(src, Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let f = find(&s, K::EmbeddedBody).unwrap().fragment().unwrap();
    assert_eq!(f.language, FragmentLanguage::Sql);
    let stmts: Vec<SyntaxKind> = f.root.child_nodes().map(|n| n.kind()).collect();
    assert_eq!(stmts, [K::Insert, K::Delete]);
}

#[test]
fn errors_in_a_body_are_reported_there_and_nowhere_else() {
    let src = "create function f() returns int language plpgsql as $$\nbegin\n  if x then y := 1;\n  return 1;\nend\n$$;\ncreate table t (a int);";
    let s = parse(src, Dialect::Postgres);
    let errors = s.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let body = src.find('$').unwrap()..src.rfind("$$").unwrap();
    assert!(body.contains(&errors[0].range.start), "{errors:?}");
    // The outer statements are clean.
    let outer: Vec<_> = s.statements().collect();
    assert_eq!(outer.len(), 2);
    for u in &outer {
        assert!(
            !u.descendants()
                .any(|n| matches!(n.kind(), K::Missing | K::Error))
        );
    }
}

#[test]
fn expected_sets_inside_a_body() {
    let src = "create function f() returns int language plpgsql as $$ begin return ";
    // The string is not closed here; close it to get a fragment.
    let closed = format!("{src} $$");
    let s = parse(&closed, Dialect::Postgres);
    let at = src.len();
    let e = s.expected_at(at);
    assert!(e.contains(&Expected::Expression), "{e:?}");
    let src = "do $$ begin perform 1; select a from  $$";
    let s = parse(src, Dialect::Postgres);
    let e = s.expected_at(src.len() - 3);
    assert!(e.iter().any(|x| matches!(x, Expected::Name(_))), "{e:?}");
    // Between statements of a body.
    let src = "do $$ begin x := 1;  end $$";
    let s = parse(src, Dialect::Postgres);
    let e = s.expected_at(src.find("  end").unwrap() + 1);
    assert!(
        e.iter()
            .any(|x| matches!(x, Expected::Keyword(_) | Expected::Name(_))),
        "{e:?}"
    );
}

#[test]
fn an_edit_inside_a_body_reparses_to_the_same_tree() {
    let src = "select 1;\ncreate function f() returns int language plpgsql as $$ begin return 1; end $$;\nselect 2;";
    let s = parse(src, Dialect::Postgres);
    let at = src.find("return 1").unwrap() + 7;
    let edit = TextEdit {
        range: at..at + 1,
        text: "42".to_string(),
    };
    let next = s.reparse(&edit);
    let mut text = src.to_string();
    text.replace_range(edit.range.clone(), &edit.text);
    assert_eq!(next, parse(&text, Dialect::Postgres));
    // Closing quote removed and put back.
    let q = src.rfind("$$").unwrap();
    let cut = s.reparse(&TextEdit {
        range: q..q + 2,
        text: String::new(),
    });
    assert_eq!(cut.statements().count(), 2);
    let back = cut.reparse(&TextEdit {
        range: q..q,
        text: "$$".to_string(),
    });
    assert_eq!(back, s);
}

#[test]
fn deep_nesting_is_bounded() {
    let mut src = String::new();
    for _ in 0..400 {
        src.push_str("begin if a then loop ");
    }
    src.push_str("null;");
    for _ in 0..400 {
        src.push_str(" end loop; end if; end;");
    }
    for d in [Dialect::Oracle, Dialect::Postgres] {
        let s = parse(&src, d);
        assert_eq!(s.print(), src);
    }
    let body = format!("do $$ {src} $$");
    let s = parse(&body, Dialect::Postgres);
    assert_eq!(s.print(), body);
}
