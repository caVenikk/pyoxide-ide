//! Resolution: the four results, for every dialect's folding.
use sql_intel::{Analyzer, RefKind, Resolution, Scope, SqlContext, Suggestion, Target, analyze};
use sql_syntax::{Dialect, HOLE, TextEdit};

use crate::fixture::*;

#[test]
fn tables_and_columns_resolve_through_the_search_path() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text =
            "select o.id, total, c.name from orders o join customers c on o.customer_id = c.id";
        let all = refs(text, &ctx);
        assert!(
            all.iter().all(|(_, _, r)| *r == R::Resolved),
            "{d:?}: {all:#?}"
        );
        assert_eq!(
            res("select * from order_totals", &ctx, "order_totals"),
            R::Resolved
        );
        assert_eq!(
            res("select email from app.users", &ctx, "email"),
            R::Resolved
        );
    }
}

#[test]
fn unresolved_names_with_similar_ones() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let a = analyze("select * from custmers", &ctx);
        let r = &a.references()[0];
        assert_eq!(r.resolution, Resolution::Unresolved, "{d:?}");
        assert_eq!(r.kind, RefKind::Table);
        assert_eq!(
            r.suggestions[0],
            Suggestion::Rename("customers".into()),
            "{d:?}"
        );

        assert_eq!(res("select nope from orders", &ctx, "nope"), R::Unresolved);
        assert_eq!(
            res("select o.nope from orders o", &ctx, "nope"),
            R::Unresolved
        );
        assert_eq!(res("select x.id from orders o", &ctx, "x"), R::Unresolved);
        assert_eq!(res("select * from app.nope", &ctx, "nope"), R::Unresolved);
    }
    // In the case the user writes in.
    let a = analyze("SELECT * FROM CUSTMERS", &scoped(Dialect::Oracle));
    assert_eq!(
        a.references()[0].suggestions[0],
        Suggestion::Rename("CUSTOMERS".into())
    );
    // A table in another schema: qualify.
    let a = analyze("select * from users", &scoped(Dialect::Postgres));
    assert!(
        a.references()[0]
            .suggestions
            .contains(&Suggestion::Qualify("app".into())),
        "{:?}",
        a.references()
    );
    // A table under an alias.
    let a = analyze(
        "select customers.id from customers c",
        &scoped(Dialect::Postgres),
    );
    let q = a.reference_at(7).unwrap();
    assert_eq!(q.resolution, Resolution::Unresolved);
    assert_eq!(q.suggestions, [Suggestion::UseAlias("c".into())]);
}

#[test]
fn case_folding_per_dialect() {
    // PostgreSQL folds to lower case: a quoted upper-case name is another.
    let pg = scoped(Dialect::Postgres);
    assert_eq!(res("select * from ORDERS", &pg, "ORDERS"), R::Resolved);
    assert_eq!(
        res("select * from \"orders\"", &pg, "\"orders\""),
        R::Resolved
    );
    assert_eq!(
        res("select * from \"ORDERS\"", &pg, "\"ORDERS\""),
        R::Unresolved
    );
    assert_eq!(
        res("select \"Total\" from orders", &pg, "\"Total\""),
        R::Unresolved
    );
    // Oracle folds to upper case.
    let ora = scoped(Dialect::Oracle);
    assert_eq!(res("select * from orders", &ora, "orders"), R::Resolved);
    assert_eq!(
        res("select * from \"ORDERS\"", &ora, "\"ORDERS\""),
        R::Resolved
    );
    assert_eq!(
        res("select * from \"orders\"", &ora, "\"orders\""),
        R::Unresolved
    );
    // SQLite compares ignoring case, quoted or not.
    let lite = scoped(Dialect::Sqlite);
    assert_eq!(res("select * from ORDERS", &lite, "ORDERS"), R::Resolved);
    assert_eq!(
        res("select * from \"Orders\"", &lite, "\"Orders\""),
        R::Resolved
    );
    assert_eq!(res("select TOTAL from orders", &lite, "TOTAL"), R::Resolved);
}

#[test]
fn ambiguous_columns_per_dialect() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "select id from orders o join customers c on o.customer_id = c.id";
        let a = analyze(text, &ctx);
        let r = a.reference_at(7).unwrap();
        let Resolution::Ambiguous(targets) = &r.resolution else {
            panic!("{d:?}: {r:?}");
        };
        let qualifiers: Vec<&str> = targets
            .iter()
            .map(|t| match t {
                Target::SourceColumn { qualifier, .. } => qualifier.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(qualifiers, ["o", "c"], "{d:?}");
        // Merged by USING or a natural join: not ambiguous.
        assert_eq!(
            res(
                "select id from orders join customers using (id)",
                &ctx,
                "id"
            ),
            R::Resolved
        );
        assert_eq!(
            res("select id from orders natural join customers", &ctx, "id"),
            R::Resolved
        );
    }
}

#[test]
fn unknown_never_unresolved() {
    for d in RESOLVING {
        let ctx = scoped(d);
        // A schema outside the model (the introspection scope names some).
        assert_eq!(
            res("select * from nosuch.t", &ctx, "t"),
            R::Unknown,
            "{d:?}"
        );
        // Loaded to names only: the table is known, its columns are not.
        let text = "select anything from audit.log";
        assert_eq!(res(text, &ctx, "log"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "anything"), R::Unknown, "{d:?}");
        // A function's columns.
        assert_eq!(
            res("select x from total(1) t where y = 1", &ctx, "x"),
            R::Unknown,
            "{d:?}"
        );
        // A hole in FROM may add a source.
        let text = format!("select nope from orders {HOLE}");
        assert_eq!(res(&text, &ctx, "nope"), R::Unknown, "{d:?}");
        // A hole as a name.
        let text = format!("select * from {HOLE} where nope = 1");
        assert_eq!(res(&text, &ctx, "nope"), R::Unknown, "{d:?}");
    }
    // A complete model: a schema it lacks does not exist.
    let ctx = SqlContext::new(
        Dialect::Postgres,
        Some(Scope::new(shop(Dialect::Postgres)).complete(true)),
    );
    assert_eq!(res("select * from nosuch.t", &ctx, "nosuch"), R::Unresolved);
    assert_eq!(
        res("select * from pg_catalog.pg_class", &ctx, "pg_class"),
        R::Unknown
    );
    // A search path schema outside the model.
    let ctx = SqlContext::new(
        Dialect::Postgres,
        Some(Scope::new(shop(Dialect::Postgres)).with_search_path(["elsewhere"])),
    );
    assert_eq!(res("select * from anything", &ctx, "anything"), R::Unknown);
}

#[test]
fn no_scope_and_generic_dialect_resolve_nothing() {
    for d in Dialect::ALL {
        let ctx = unscoped(d);
        let all = refs("select nope from nothing", &ctx);
        assert!(
            all.iter().all(|(_, _, r)| *r == R::Unknown),
            "{d:?}: {all:?}"
        );
    }
    let ctx = SqlContext::new(Dialect::Generic, Some(Scope::new(shop(Dialect::Generic))));
    let all = refs("select nope from nothing join orders o on o.x = 1", &ctx);
    assert!(
        all.iter()
            .all(|(_, _, r)| matches!(r, R::Unknown | R::Resolved)),
        "{all:?}"
    );
}

#[test]
fn ctes_subqueries_and_star_expansion() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "select x.total, x.nope from (select * from orders) x";
        assert_eq!(res(text, &ctx, "total"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "nope"), R::Unresolved, "{d:?}");

        let text = "with big as (select id, total as amount from orders) \
                    select amount, total from big";
        assert_eq!(res(text, &ctx, "big"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "amount"), R::Resolved, "{d:?}");
        assert_eq!(
            refs(text, &ctx).last().map(|r| r.2),
            Some(R::Unresolved),
            "{d:?}"
        );

        let text = "with c (a, b) as (select id, name from customers) select a, b, c.a from c";
        assert!(
            refs(text, &ctx).iter().all(|r| r.2 == R::Resolved),
            "{d:?}: {:?}",
            refs(text, &ctx)
        );
        // `t.*` and a subquery column alias list.
        let text = "select s.p from (select o.*, 1 from orders o) s(p, q, r, w)";
        assert_eq!(res(text, &ctx, "p"), R::Resolved, "{d:?}");
        // Correlated subqueries see the outer sources.
        let text = "select * from orders o where exists \
                    (select 1 from customers c where c.id = o.customer_id and total > 0)";
        assert!(refs(text, &ctx).iter().all(|r| r.2 == R::Resolved), "{d:?}");
        // ORDER BY sees select-list aliases.
        assert_eq!(
            res("select total as t1 from orders order by t1", &ctx, "t1"),
            R::Resolved
        );
    }
}

#[test]
fn objects_created_earlier_in_the_script() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "create table tmp (a int, b int);\nselect a, nope from tmp;";
        assert_eq!(res(text, &ctx, "tmp"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "a"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "nope"), R::Unresolved, "{d:?}");
        let text = "create view v as select id, total from orders;\nselect total from v;";
        assert_eq!(
            refs(text, &ctx).last().map(|r| r.2),
            Some(R::Resolved),
            "{d:?}"
        );
        // Not before it is created.
        let text = "select * from later;\ncreate table later (a int);";
        assert_eq!(res(text, &ctx, "later"), R::Unresolved, "{d:?}");
    }
}

#[test]
fn names_the_dialect_knows() {
    let ora = scoped(Dialect::Oracle);
    assert!(
        refs("select sysdate, rownum, dummy from dual", &ora)
            .iter()
            .all(|r| r.2 == R::Resolved)
    );
    assert_eq!(
        res("select * from all_tables", &ora, "all_tables"),
        R::Unknown
    );
    let pg = scoped(Dialect::Postgres);
    assert!(
        refs("select ctid, o.xmin, current_date from orders o", &pg)
            .iter()
            .all(|r| r.2 == R::Resolved)
    );
    assert_eq!(res("select * from pg_class", &pg, "pg_class"), R::Unknown);
    let lite = scoped(Dialect::Sqlite);
    assert_eq!(
        refs("select rowid, name from sqlite_master", &lite),
        [
            ("sqlite_master".to_string(), RefKind::Table, R::Resolved),
            ("rowid".to_string(), RefKind::Column, R::Unknown),
            ("name".to_string(), RefKind::Column, R::Unknown),
        ]
    );
    assert_eq!(res("select rowid from orders", &lite, "rowid"), R::Resolved);
    // A double-quoted name that is no column is a string in SQLite.
    assert_eq!(res("select \"x\" from orders", &lite, "\"x\""), R::Unknown);
}

#[test]
fn data_modification() {
    for d in RESOLVING {
        let ctx = scoped(d);
        let text = "insert into orders (id, nope) values (1, 2)";
        assert_eq!(res(text, &ctx, "id"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "nope"), R::Unresolved, "{d:?}");
        let text = "update orders set total = 0, nope = 1 where customer_id = 1";
        assert_eq!(res(text, &ctx, "total"), R::Resolved, "{d:?}");
        assert_eq!(res(text, &ctx, "nope"), R::Unresolved, "{d:?}");
        assert_eq!(res(text, &ctx, "customer_id"), R::Resolved, "{d:?}");
        let text = "delete from orders where nope = 1";
        assert_eq!(res(text, &ctx, "nope"), R::Unresolved, "{d:?}");
    }
    let pg = scoped(Dialect::Postgres);
    let text = "insert into orders as o (id, total) values (1, 2) \
                on conflict (id) do update set total = excluded.total returning id";
    assert!(
        refs(text, &pg).iter().all(|r| r.2 == R::Resolved),
        "{:?}",
        refs(text, &pg)
    );
    let text = "update orders o set total = 0 from customers c where c.id = o.customer_id";
    assert!(refs(text, &pg).iter().all(|r| r.2 == R::Resolved));
    let text = "merge into orders o using customers c on (o.customer_id = c.id) \
                when matched then update set total = 1 \
                when not matched then insert (id) values (c.id)";
    assert!(
        refs(text, &pg).iter().all(|r| r.2 == R::Resolved),
        "{:?}",
        refs(text, &pg)
    );
}

#[test]
fn procedural_code_has_variables() {
    let ora = scoped(Dialect::Oracle);
    let text = "begin\n  for r in (select id from orders) loop\n    \
                update orders set total = v_x where id = r.id;\n  end loop;\nend;";
    let all = refs(text, &ora);
    assert_eq!(res(text, &ora, "v_x"), R::Unknown, "{all:?}");
    assert_eq!(res(text, &ora, "r"), R::Unknown);
    assert_eq!(res(text, &ora, "total"), R::Resolved);
    assert_eq!(
        res("begin delete from nope; end;", &ora, "nope"),
        R::Unresolved
    );
    let pg = scoped(Dialect::Postgres);
    let text = "create function f(p int) returns void language plpgsql as $$\n\
                begin update orders set total = p where id = new.id; end $$";
    assert_eq!(res(text, &pg, "p"), R::Unknown, "{:?}", refs(text, &pg));
    assert_eq!(res(text, &pg, "total"), R::Resolved);
}

#[test]
fn an_edit_reanalyses_the_edited_statement_only() {
    let ctx = scoped(Dialect::Postgres);
    let text = "create table tmp (a int);\nselect a from tmp;\nselect * from orders;\nselect * from customers;\n";
    let mut analyzer = Analyzer::new();
    let script = sql_syntax::parse(text, ctx.dialect);
    analyzer.analyze(&script, &ctx, &|| false).unwrap();
    assert_eq!(analyzer.analysed(), 4);

    // Edit the third statement.
    let at = text.find("orders").unwrap();
    let script = script.reparse(&TextEdit {
        range: at..at + "orders".len(),
        text: "custmers".into(),
    });
    let a = analyzer.analyze(&script, &ctx, &|| false).unwrap();
    assert_eq!(analyzer.analysed(), 5);
    assert_eq!(problems_of(&a).len(), 1);

    // An edit of the creating statement that keeps its columns: the rest stays.
    let script = script.reparse(&TextEdit {
        range: 0..0,
        text: "-- temp\n".into(),
    });
    analyzer.analyze(&script, &ctx, &|| false).unwrap();
    assert_eq!(analyzer.analysed(), 6);

    // One that changes them: the statements after it are analysed again.
    let at = script.text().find("a int").unwrap();
    let script = script.reparse(&TextEdit {
        range: at..at + 1,
        text: "b".into(),
    });
    let a = analyzer.analyze(&script, &ctx, &|| false).unwrap();
    assert_eq!(analyzer.analysed(), 10);
    assert!(
        problems_of(&a)
            .iter()
            .any(|p| p.message == "Cannot resolve column 'a'"),
        "{:?}",
        problems_of(&a)
    );

    // A new model: everything again; a cancelled run gives nothing.
    let ctx = scoped(Dialect::Postgres);
    assert!(analyzer.analyze(&script, &ctx, &|| true).is_none());
    analyzer.analyze(&script, &ctx, &|| false).unwrap();
    assert_eq!(analyzer.analysed(), 14);
}
