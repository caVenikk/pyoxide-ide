use sql_syntax::{
    Danger, DdlAction, DdlTarget, Dialect, ObjectKind, StatementKind, classify_text, split,
};

const PG: Dialect = Dialect::Postgres;
const ORA: Dialect = Dialect::Oracle;
const LITE: Dialect = Dialect::Sqlite;

fn kind(text: &str, d: Dialect) -> StatementKind {
    classify_text(text, d).kind
}

fn ddl(text: &str, d: Dialect) -> DdlTarget {
    classify_text(text, d)
        .ddl
        .unwrap_or_else(|| panic!("no target in {text}"))
}

fn names(t: &DdlTarget, d: Dialect) -> Vec<String> {
    t.names.iter().map(|n| n.names(d).join(".")).collect()
}

#[test]
fn kinds_by_the_first_words() {
    use StatementKind::*;
    let cases = [
        ("select 1", Select),
        ("  -- c\n SELECT 1", Select),
        ("(select 1) union (select 2)", Select),
        ("values (1)", Select),
        ("table t", Select),
        ("with a as (select 1) select * from a", With),
        ("insert into t values (1)", Dml),
        ("update t set a = 1", Dml),
        ("delete from t", Dml),
        (
            "merge into t using s on (1=1) when matched then delete",
            Dml,
        ),
        ("create table t (a int)", Ddl),
        ("alter table t add b int", Ddl),
        ("drop table t", Ddl),
        ("truncate table t", Ddl),
        ("comment on table t is 'x'", Ddl),
        ("grant select on t to u", Ddl),
        ("commit", Tcl),
        ("rollback to savepoint s", Tcl),
        ("savepoint s", Tcl),
        ("release savepoint s", Tcl),
        ("start transaction", Tcl),
        ("set transaction read only", Tcl),
        ("explain select 1", Other),
        ("set search_path = a", Other),
        ("pragma foreign_keys = on", Other),
        ("show tables", Other),
        ("", Other),
        ("-- only", Other),
    ];
    for (text, expected) in cases {
        assert_eq!(kind(text, PG), expected, "{text}");
    }
}

#[test]
fn blocks_and_transactions_depend_on_the_dialect() {
    use StatementKind::*;
    assert_eq!(kind("begin", PG), Tcl);
    assert_eq!(kind("begin transaction", LITE), Tcl);
    assert_eq!(kind("end", PG), Tcl);
    assert_eq!(kind("begin null; end;", ORA), AnonymousBlock);
    assert_eq!(
        kind("declare n number; begin null; end;", ORA),
        AnonymousBlock
    );
    assert_eq!(kind("<<l>> begin null; end;", ORA), AnonymousBlock);
    assert_eq!(kind("do $$ begin null; end $$", PG), AnonymousBlock);
    assert_eq!(
        kind("begin not atomic select 1; end", Dialect::Generic),
        AnonymousBlock
    );
    assert_eq!(kind("declare c cursor for select 1", PG), Other);
    assert_eq!(kind("replace into t values (1)", LITE), Dml);
    assert_eq!(kind("replace('a', 'b', 'c')", ORA), Other);
}

#[test]
fn data_changing_statements() {
    let w = |t: &str| classify_text(t, PG).writes;
    assert!(w("insert into t values (1)") && w("update t set a = 1") && w("delete from t"));
    assert!(w("with x as (select 1) insert into t select * from x"));
    assert!(w(
        "with x as (select 1) delete from t where a in (select * from x)"
    ));
    assert!(!w("with x as (select 1) select * from x"));
    assert!(!w("select 1") && !w("create table t (a int)") && !w("commit"));
    assert!(classify_text("begin null; end;", ORA).writes);
}

#[test]
fn delete_and_update_without_where() {
    let danger = |t: &str| classify_text(t, PG).danger;
    assert_eq!(danger("delete from t"), Some(Danger::DeleteWithoutWhere));
    assert_eq!(danger("DELETE FROM t;"), Some(Danger::DeleteWithoutWhere));
    assert_eq!(danger("update t set a=1"), Some(Danger::UpdateWithoutWhere));
    assert_eq!(
        danger("update t set a = (select max(b) from u where u.id = 1)"),
        Some(Danger::UpdateWithoutWhere)
    );
    assert_eq!(danger("delete from t where id in (select id from u)"), None);
    assert_eq!(danger("delete from t using u where t.id = u.id"), None);
    assert_eq!(danger("update t set a = 1 where b = 2"), None);
    assert_eq!(danger("update t set a = 1 from u where t.id = u.id"), None);
    assert_eq!(danger("delete from t where current of c"), None);
    // The word inside a string, a comment or a name is no `WHERE`.
    assert_eq!(
        danger("delete from t -- where x"),
        Some(Danger::DeleteWithoutWhere)
    );
    assert_eq!(
        danger("update t set a = 'where'"),
        Some(Danger::UpdateWithoutWhere)
    );
    assert_eq!(
        danger("update t set \"where\" = 1"),
        Some(Danger::UpdateWithoutWhere)
    );
    // A subquery with `WHERE` only inside.
    assert_eq!(
        danger("delete from t where_ x"),
        Some(Danger::DeleteWithoutWhere)
    );
    assert_eq!(
        danger("delete from (select * from t where a = 1)"),
        Some(Danger::DeleteWithoutWhere)
    );
    // After a common table expression.
    assert_eq!(
        danger("with x as (select 1 where true) delete from t"),
        Some(Danger::DeleteWithoutWhere)
    );
    assert_eq!(
        danger("with x as (select 1) delete from t where a = 1"),
        None
    );
    // Other statements never are.
    assert_eq!(danger("select * from t"), None);
    assert_eq!(danger("insert into t select * from u"), None);
    assert_eq!(danger("truncate t"), None);
}

#[test]
fn the_split_carries_the_classification() {
    let st = split("select 1; delete from t; create table x (a int);", PG);
    assert_eq!(st[0].kind, StatementKind::Select);
    assert_eq!(
        (st[1].kind, st[1].danger),
        (StatementKind::Dml, Some(Danger::DeleteWithoutWhere))
    );
    assert!(st[1].writes && st[2].ddl.is_some());
}

#[test]
fn ddl_targets() {
    let t = ddl("create table if not exists public.orders (id int)", PG);
    assert_eq!((t.action, t.object), (DdlAction::Create, ObjectKind::Table));
    assert_eq!(names(&t, PG), ["public.orders"]);

    let t = ddl("CREATE OR REPLACE VIEW \"Sales\".v AS select 1", PG);
    assert_eq!((t.action, t.object), (DdlAction::Create, ObjectKind::View));
    assert_eq!(names(&t, PG), ["Sales.v"]);
    assert_eq!(names(&t, ORA), ["Sales.V"]);

    let t = ddl("create temp table tmp as select 1", PG);
    assert_eq!(names(&t, PG), ["tmp"]);
    let t = ddl("create global temporary table gt (a int)", ORA);
    assert_eq!(names(&t, ORA), ["GT"]);
    assert_eq!(
        ddl("create materialized view mv as select 1", PG).object,
        ObjectKind::MaterializedView
    );

    let t = ddl("create unique index concurrently ix on s.t (a)", PG);
    assert_eq!(
        (t.object, names(&t, PG)),
        (ObjectKind::Index, vec!["ix".to_string()])
    );
    assert_eq!(t.on.unwrap().names(PG), ["s", "t"]);
    let t = ddl("create index on t (a)", PG);
    assert!(t.names.is_empty());
    assert_eq!(t.on.unwrap().names(PG), ["t"]);

    let t = ddl(
        "create trigger tr before insert on t for each row execute function f()",
        PG,
    );
    assert_eq!(
        (t.object, t.on.unwrap().names(PG)),
        (ObjectKind::Trigger, vec!["t".to_string()])
    );

    let t = ddl("alter table only s.t add column b int", PG);
    assert_eq!(
        (t.action, t.object, names(&t, PG)),
        (DdlAction::Alter, ObjectKind::Table, vec!["s.t".to_string()])
    );

    let t = ddl("drop table if exists a, \"B\", s.c cascade", PG);
    assert_eq!(
        (t.action, names(&t, PG)),
        (
            DdlAction::Drop,
            vec!["a".to_string(), "B".to_string(), "s.c".to_string()]
        )
    );
    assert!(t.names[1].parts[0].quoted && !t.names[0].parts[0].quoted);

    let t = ddl("truncate table a, b", PG);
    assert_eq!(
        (t.action, t.object, names(&t, PG)),
        (
            DdlAction::Truncate,
            ObjectKind::Table,
            vec!["a".into(), "b".into()]
        )
    );
    assert_eq!(names(&ddl("truncate t", PG), PG), ["t"]);

    let t = ddl("comment on column s.t.c is 'x'", PG);
    assert_eq!(
        (t.action, t.object, names(&t, PG)),
        (
            DdlAction::Comment,
            ObjectKind::Column,
            vec!["s.t.c".to_string()]
        )
    );

    let t = ddl("create or replace package body hr.pk as end;", ORA);
    assert_eq!(
        (t.object, names(&t, ORA)),
        (ObjectKind::PackageBody, vec!["HR.PK".to_string()])
    );
    let t = ddl(
        "create or replace editionable procedure p is begin null; end;",
        ORA,
    );
    assert_eq!(
        (t.object, names(&t, ORA)),
        (ObjectKind::Procedure, vec!["P".to_string()])
    );
    assert_eq!(
        ddl("create type body ty as end;", ORA).object,
        ObjectKind::Type
    );
    assert_eq!(
        ddl("create public synonym s for hr.t", ORA).object,
        ObjectKind::Synonym
    );
    assert_eq!(
        ddl("create virtual table v using fts5(a)", LITE).object,
        ObjectKind::Table
    );
    assert_eq!(
        ddl("create function f() returns int as $$ select 1 $$", PG).object,
        ObjectKind::Function
    );
    assert_eq!(ddl("create sequence s", PG).object, ObjectKind::Sequence);
    assert_eq!(ddl("create schema s", PG).object, ObjectKind::Schema);

    // Backtick and bracket quotes of SQLite.
    assert_eq!(
        names(&ddl("drop table `a b`, [c d]", LITE), LITE),
        ["a b", "c d"]
    );

    // No target: grants and what is not understood.
    for t in [
        "grant select on t to u",
        "alter session set x = 1",
        "create",
        "drop",
        "create table",
        "comment on",
    ] {
        assert_eq!(classify_text(t, PG).ddl, None, "{t}");
    }
}
