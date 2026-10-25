//! DDL: shapes, the objects a statement names, error recovery, expected sets.
use sql_syntax::{
    Dialect, Expected, NameKind, Script, SyntaxErrorKind, SyntaxKind, SyntaxNode, classify_text,
    parse,
};

use crate::statements::cases;
use SyntaxKind as K;

const DDL: &str = include_str!("corpus/ddl.sql");
const PLSQL: &str = include_str!("corpus/plsql.sql");
const PLPGSQL: &str = include_str!("corpus/plpgsql.sql");

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

#[test]
fn create_table_has_columns_and_constraints() {
    let src = "create table s.t (id int primary key, name varchar(10) not null default 'x', constraint fk foreign key (id) references u (a) on delete cascade) tablespace x";
    let s = parse(src, Dialect::Postgres);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let table = statement(&s);
    assert_eq!(table.kind(), K::CreateTable);
    assert_eq!(
        kinds(table),
        [
            K::TableRef,
            K::ColumnDef,
            K::ColumnDef,
            K::TableConstraint,
            K::Opaque
        ]
    );
    let columns: Vec<SyntaxNode<'_>> = table
        .child_nodes()
        .filter(|n| n.kind() == K::ColumnDef)
        .collect();
    assert_eq!(
        kinds(columns[0]),
        [K::DefName, K::TypeRef, K::ColumnConstraint]
    );
    assert_eq!(
        kinds(columns[1]),
        [
            K::DefName,
            K::TypeRef,
            K::ColumnConstraint,
            K::ColumnConstraint
        ]
    );
    let fk = table.child_nodes().nth(3).unwrap();
    assert_eq!(kinds(fk), [K::DefName, K::ColumnList, K::ReferencesClause]);
    // The name of the table is qualified by a schema.
    let name = table.child(K::TableRef).unwrap();
    assert_eq!(name.child(K::SchemaRef).unwrap().text(), "s");
}

#[test]
fn a_default_value_does_not_swallow_the_next_constraint() {
    for d in [Dialect::Postgres, Dialect::Sqlite, Dialect::Oracle] {
        let s = parse(
            "create table t (a int default 0 not null, b text default 'x' null)",
            d,
        );
        assert!(s.errors().is_empty(), "{d:?}: {:?}", s.errors());
        assert_eq!(count(&s, K::ColumnConstraint), 4, "{d:?}");
    }
}

#[test]
fn create_index_has_elements_and_a_predicate() {
    let s = parse(
        "create unique index concurrently i on s.t using btree (a desc nulls last, lower(b)) include (c) where a > 0",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let index = statement(&s);
    assert_eq!(index.kind(), K::CreateIndex);
    assert_eq!(
        kinds(index),
        [
            K::ObjectRef,
            K::TableRef,
            K::IndexElement,
            K::IndexElement,
            K::ColumnList,
            K::WhereClause
        ]
    );
}

#[test]
fn alter_table_actions_are_nodes() {
    let s = parse(
        "alter table t add column c int, drop column d, alter column e type text using e::text, rename column a to b, add constraint pk primary key (id)",
        Dialect::Postgres,
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    let alter = statement(&s);
    assert_eq!(alter.kind(), K::AlterTable);
    assert_eq!(
        kinds(alter),
        [
            K::TableRef,
            K::AlterAction,
            K::AlterAction,
            K::AlterAction,
            K::AlterAction,
            K::AlterAction
        ]
    );
    let first = alter.child_nodes().nth(1).unwrap();
    assert_eq!(kinds(first), [K::ColumnDef]);
    assert_eq!(count(&s, K::TableConstraint), 1);
}

#[test]
fn the_tree_names_the_objects_like_the_token_scan() {
    let mut checked = 0;
    let corpora: [(&str, &[Dialect]); 3] = [
        (DDL, &Dialect::ALL),
        (PLSQL, &[Dialect::Oracle]),
        (PLPGSQL, &[Dialect::Postgres, Dialect::Generic]),
    ];
    for (corpus, default) in corpora {
        for c in cases(corpus, default) {
            for &d in &c.dialects {
                let by_tokens = classify_text(&c.text, d).ddl;
                let by_tree = parse(&c.text, d).ddl_target(0);
                assert_eq!(by_tree, by_tokens, "{d:?}: {}", c.text);
                checked += usize::from(by_tree.is_some());
            }
        }
    }
    assert!(checked > 300, "{checked}");
}

#[test]
fn ddl_target_is_for_the_statement_at_the_offset() {
    let src = "create table a (x int);\ndrop table b, c;\nselect 1;";
    let s = parse(src, Dialect::Postgres);
    let first = s.ddl_target(3).unwrap();
    assert_eq!(first.names.len(), 1);
    let second = s.ddl_target(src.find("drop").unwrap() + 2).unwrap();
    assert_eq!(second.names.len(), 2);
    assert_eq!(s.ddl_target(src.len()), None);
}

#[test]
fn unfinished_ddl_keeps_what_was_typed() {
    let cases = [
        ("create table ", "table name"),
        ("create table t (", "column name"),
        ("create table t (a int,", "column name"),
        ("create index on", "table name"),
        ("alter table ", "table name"),
        ("drop table ", "table name"),
        ("create view v as ", "SELECT"),
        ("comment on table t is ", "Syntax error"),
    ];
    for (src, want) in cases {
        let s = parse(src, Dialect::Postgres);
        let errors = s.errors();
        assert!(!errors.is_empty(), "{src}");
        assert_eq!(
            errors[0].kind,
            SyntaxErrorKind::Missing,
            "{src}: {errors:?}"
        );
        let message = errors[0].message(src);
        assert!(message.contains(want), "{src}: {message}");
        assert_eq!(s.print(), src);
    }
}

#[test]
fn unknown_ddl_syntax_is_opaque_and_not_an_error() {
    for (d, src) in [
        (
            Dialect::Postgres,
            "create table t (a int) weird_option = 5 another (x, y)",
        ),
        (
            Dialect::Postgres,
            "create table t (a int visible_thing encrypted)",
        ),
        (
            Dialect::Oracle,
            "create table t (a number) storage (initial 64k next 1m) logging",
        ),
        (Dialect::Postgres, "alter table t frobnicate widgets"),
        (Dialect::Postgres, "create extension if not exists pgcrypto"),
        (
            Dialect::Oracle,
            "create database link l connect to u identified by p using 'x'",
        ),
        (
            Dialect::Postgres,
            "create view v as select 1 with local check option",
        ),
    ] {
        let s = parse(src, d);
        assert!(s.errors().is_empty(), "{d:?} {src}: {:?}", s.errors());
        assert!(count(&s, K::Opaque) >= 1, "{src}");
        assert_eq!(s.print(), src);
    }
}

#[test]
fn expected_sets_in_ddl() {
    let at = |src: &str, d: Dialect| parse(src, d).expected_at(src.len());
    assert!(at("create table ", Dialect::Postgres).contains(&Expected::Name(NameKind::Table)));
    assert!(at("create table t (", Dialect::Postgres).contains(&Expected::Name(NameKind::Column)));
    assert!(at("alter table ", Dialect::Postgres).contains(&Expected::Name(NameKind::Table)));
    let e = at("alter table t ", Dialect::Postgres);
    assert!(e.contains(&Expected::Keyword("ADD")), "{e:?}");
    assert!(e.contains(&Expected::Keyword("DROP")), "{e:?}");
    assert!(at("drop table ", Dialect::Postgres).contains(&Expected::Name(NameKind::Table)));
    assert!(at("create index i on ", Dialect::Postgres).contains(&Expected::Name(NameKind::Table)));
    assert!(
        at("create table t (a int references ", Dialect::Postgres)
            .contains(&Expected::Name(NameKind::Table))
    );
}

#[test]
fn a_broken_statement_does_not_reach_the_next() {
    let src = "create table t (a int,, b int);\ndrop;\ncreate index i on t (a);\nalter table u add;\ncreate view v as select 1;";
    let s = parse(src, Dialect::Postgres);
    let kinds: Vec<SyntaxKind> = s
        .statements()
        .map(|r| r.child_nodes().next().unwrap().kind())
        .collect();
    assert_eq!(
        kinds,
        [
            K::CreateTable,
            K::DropStatement,
            K::CreateIndex,
            K::AlterTable,
            K::CreateView
        ]
    );
    let roots: Vec<_> = s.statements().collect();
    for i in [2, 4] {
        assert!(
            !roots[i]
                .descendants()
                .any(|n| matches!(n.kind(), K::Error | K::Missing)),
            "statement {i}"
        );
    }
}

#[test]
fn schema_objects_by_dialect_flags() {
    // PRAGMA is SQLite's; elsewhere it is an unknown statement.
    let s = parse("pragma foreign_keys = on", Dialect::Postgres);
    assert_eq!(statement(&s).kind(), K::Opaque);
    let s = parse("pragma foreign_keys = on", Dialect::Sqlite);
    assert_eq!(statement(&s).kind(), K::Pragma);
    // DO with a body is PostgreSQL's.
    let s = parse("do $$ begin null; end $$", Dialect::Oracle);
    assert_eq!(statement(&s).kind(), K::Opaque);
}
