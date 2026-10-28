//! `Introspector::load` on SQLite: the same data as the per-object reads, the
//! optional details, scopes, and a fixed number of statements.

use std::time::Instant;

use crate::common::{memory, new_file, run};
use crate::introspection::fixture;
use db_core::schema::{
    ConstraintKind, GeneratedColumn, Level, ObjectDetail, ObjectKind, Scope, introspector,
};
use db_testkit::{assert_load_matches_per_object, load_statements};

fn all(_: &str) -> bool {
    true
}

#[tokio::test]
async fn bulk_load_equals_the_per_object_reads() {
    let conn = fixture().await;
    for level in [Level::Names, Level::Structure, Level::Full] {
        let snapshot = assert_load_matches_per_object(&*conn, &Scope::current(), level, &all).await;
        assert_eq!(snapshot.schemas.len(), 1);
        assert_eq!(snapshot.schemas[0].info.name, "main");
    }
}

#[tokio::test]
async fn names_level_reads_objects_only() {
    let conn = fixture().await;
    let snapshot = introspector(&*conn)
        .load(&Scope::current(), Level::Names)
        .await
        .unwrap();
    let main = &snapshot.schemas[0];
    assert!(main.relations.is_empty() && main.sources.is_empty());
    assert!(main.objects.iter().all(|o| o.detail.is_none()));
    assert_eq!(snapshot.level, Level::Names);
    assert!(snapshot.token.is_some(), "a load leaves a change token");
}

#[tokio::test]
async fn details_of_tables_triggers_columns_indexes_and_keys() {
    let conn = memory().await;
    for sql in [
        "CREATE TABLE p (id INTEGER PRIMARY KEY, a INT, b INT GENERATED ALWAYS AS (a * 2) STORED, \
         c INT AS (a + 1), UNIQUE (a, b)) STRICT",
        "CREATE TABLE ch (x INT REFERENCES p (id) ON UPDATE CASCADE ON DELETE SET NULL, y INT, \
         PRIMARY KEY (x, y), FOREIGN KEY (y) REFERENCES p) WITHOUT ROWID",
        "CREATE INDEX ip ON p (a) WHERE a > 3 AND b IS NOT NULL",
        "CREATE INDEX ie ON p (lower(a), b DESC)",
        "CREATE VIRTUAL TABLE docs USING fts5(body)",
        "CREATE VIEW v AS SELECT * FROM p",
        "CREATE TRIGGER trg INSTEAD OF DELETE ON v WHEN old.a > CASE WHEN 1 THEN 2 ELSE 3 END \
         BEGIN SELECT CASE WHEN 1 THEN 1 END; END",
        "CREATE TRIGGER trg2 AFTER UPDATE OF a ON p BEGIN SELECT 1; END",
        "CREATE TRIGGER trg3 DELETE ON p BEGIN SELECT 1; END",
        "CREATE TEMP TABLE scratch (t INT)",
    ] {
        run(&conn, sql).await;
    }
    let intro = introspector(&*conn);
    let snapshot = intro.load(&Scope::all(), Level::Structure).await.unwrap();
    let names: Vec<_> = snapshot
        .schemas
        .iter()
        .map(|s| s.info.name.as_str())
        .collect();
    assert_eq!(names, ["main", "temp"]);
    let main = snapshot.schema("main").unwrap();
    let object = |name: &str| main.objects.iter().find(|o| o.name == name).unwrap();

    let ObjectDetail::Table(p) = object("p").detail.clone().unwrap() else {
        panic!("a table detail");
    };
    assert!(!p.without_rowid && !p.virtual_table && !p.temporary);
    let ObjectDetail::Table(ch) = object("ch").detail.clone().unwrap() else {
        panic!("a table detail");
    };
    assert!(ch.without_rowid);
    let ObjectDetail::Table(docs) = object("docs").detail.clone().unwrap() else {
        panic!("a table detail");
    };
    assert!(docs.virtual_table);
    let temp = snapshot.schema("temp").unwrap();
    let ObjectDetail::Table(scratch) = temp
        .objects
        .iter()
        .find(|o| o.name == "scratch")
        .unwrap()
        .detail
        .clone()
        .unwrap()
    else {
        panic!("a table detail");
    };
    assert!(scratch.temporary);

    let trigger = |name: &str| match object(name).detail.clone() {
        Some(ObjectDetail::Trigger(t)) => t,
        other => panic!("{name}: {other:?}"),
    };
    let trg = trigger("trg");
    assert_eq!(trg.timing.as_deref(), Some("INSTEAD OF"));
    assert_eq!(trg.events, ["DELETE"]);
    assert!(trg.each_row);
    assert_eq!(
        trg.condition.as_deref(),
        Some("old.a > CASE WHEN 1 THEN 2 ELSE 3 END")
    );
    assert_eq!(object("trg").parent.as_deref(), Some("v"));
    let trg2 = trigger("trg2");
    assert_eq!(trg2.timing.as_deref(), Some("AFTER"));
    assert_eq!(trg2.events, ["UPDATE"]);
    assert_eq!(trg2.condition, None);
    let trg3 = trigger("trg3");
    assert_eq!(
        trg3.timing.as_deref(),
        Some("BEFORE"),
        "no time means BEFORE"
    );
    assert_eq!(trg3.events, ["DELETE"]);

    let rel = main.relation("p").unwrap();
    let col = |name: &str| rel.columns.iter().find(|c| c.name == name).unwrap();
    assert_eq!(col("a").generated, None);
    assert_eq!(
        col("b").generated,
        Some(GeneratedColumn {
            expression: Some("a * 2".into()),
            stored: true
        })
    );
    assert_eq!(
        col("c").generated,
        Some(GeneratedColumn {
            expression: Some("a + 1".into()),
            stored: false
        })
    );
    assert_eq!(col("id").identity, None);

    let index = |name: &str| rel.indexes.iter().find(|i| i.name == name).unwrap();
    assert_eq!(
        index("ip").predicate.as_deref(),
        Some("a > 3 AND b IS NOT NULL")
    );
    assert!(index("ip").expressions.is_empty());
    assert_eq!(index("ie").columns, ["lower(a)", "b"]);
    assert_eq!(index("ie").expressions, [0]);
    assert_eq!(index("ie").predicate, None);

    let ch_rel = main.relation("ch").unwrap();
    let fks: Vec<_> = ch_rel
        .constraints
        .iter()
        .filter(|c| c.kind == ConstraintKind::ForeignKey)
        .collect();
    assert_eq!(fks.len(), 2);
    let cascade = fks.iter().find(|c| c.columns == ["x"]).unwrap();
    assert_eq!(cascade.on_update.as_deref(), Some("CASCADE"));
    assert_eq!(cascade.on_delete.as_deref(), Some("SET NULL"));
    let plain = fks.iter().find(|c| c.columns == ["y"]).unwrap();
    assert_eq!(plain.on_update.as_deref(), Some("NO ACTION"));
    assert_eq!(plain.ref_columns, ["id"], "REFERENCES parent means its key");

    // The per-object reads keep their output.
    let per_object = intro.constraints("main", "ch").await.unwrap();
    assert!(per_object.iter().all(|c| c.on_update.is_none()));
    assert!(
        intro
            .columns("main", "p")
            .await
            .unwrap()
            .iter()
            .all(|c| c.generated.is_none())
    );
}

#[tokio::test]
async fn full_level_reads_source_text() {
    let conn = fixture().await;
    let snapshot = introspector(&*conn)
        .load(&Scope::current(), Level::Full)
        .await
        .unwrap();
    let main = &snapshot.schemas[0];
    let source = |kind: ObjectKind, name: &str| {
        main.sources
            .iter()
            .find(|s| s.kind == kind && s.name == name)
            .unwrap_or_else(|| panic!("no source for {name}"))
            .text
            .clone()
    };
    assert!(source(ObjectKind::View, "Item View").starts_with("CREATE VIEW"));
    assert!(source(ObjectKind::Trigger, "trg Log").contains("AFTER INSERT"));
    assert!(source(ObjectKind::Table, "wr").contains("WITHOUT ROWID"));
    assert!(
        main.sources
            .iter()
            .all(|s| !s.name.starts_with("sqlite_autoindex")),
        "automatic indexes have no text"
    );
}

#[tokio::test]
async fn scopes_pick_schemas() {
    let dir = tempfile::tempdir().unwrap();
    let conn = new_file(dir.path(), "main.db").await;
    let other = dir.path().join("other.db");
    // Created through a second connection: the file must exist to attach.
    let seed = new_file(dir.path(), "other.db").await;
    run(&seed, "CREATE TABLE in_other (a)").await;
    seed.close().await.unwrap();
    run(
        &conn,
        &format!("ATTACH DATABASE '{}' AS \"Extra Db\"", other.display()),
    )
    .await;
    run(&conn, "CREATE TABLE in_main (a)").await;
    let intro = introspector(&*conn);

    let names = |scope: Scope| {
        let intro = &intro;
        async move {
            intro
                .load(&scope, Level::Names)
                .await
                .unwrap()
                .schemas
                .into_iter()
                .map(|s| s.info.name)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(names(Scope::current()).await, ["main"]);
    assert_eq!(names(Scope::all()).await, ["main", "Extra Db"]);
    assert_eq!(names(Scope::list(["Extra Db"])).await, ["Extra Db"]);
    assert_eq!(names(Scope::list(["nope"])).await, Vec::<String>::new());
    assert_eq!(names(Scope::patterns(["EXTRA*"])).await, ["Extra Db"]);
    assert_eq!(names(Scope::patterns(["?ain", "x"])).await, ["main"]);

    let snapshot = intro
        .load(&Scope::list(["Extra Db"]), Level::Structure)
        .await
        .unwrap();
    let extra = &snapshot.schemas[0];
    assert_eq!(extra.objects.len(), 1);
    assert_eq!(extra.objects[0].schema, "Extra Db");
    assert_eq!(extra.relation("in_other").unwrap().columns.len(), 1);
}

#[tokio::test]
async fn statements_do_not_depend_on_the_tables() {
    let conn = memory().await;
    let counts = async |conn: &std::sync::Arc<dyn db_core::Connection>| {
        let mut out = Vec::new();
        for level in [Level::Names, Level::Structure, Level::Full] {
            out.push(load_statements(&**conn, &Scope::all(), level).await);
        }
        out
    };
    let empty = counts(&conn).await;
    for i in 0..300 {
        run(
            &conn,
            &format!(
                "CREATE TABLE t{i} (id INTEGER PRIMARY KEY, a TEXT UNIQUE, b INT REFERENCES t0 (id))"
            ),
        )
        .await;
    }
    assert_eq!(counts(&conn).await, empty);
    assert_eq!(empty, [2, 5, 5], "schema list, then one query per family");
}

/// `cargo test -p db-sqlite --release --test it load::bench -- --ignored --nocapture`
#[tokio::test]
#[ignore = "measurement, run on demand"]
async fn bench_load_5000_tables() {
    let conn = memory().await;
    run(&conn, "BEGIN").await;
    for i in 0..5000 {
        run(
            &conn,
            &format!(
                "CREATE TABLE t{i:04} (id INTEGER PRIMARY KEY, parent_id INT REFERENCES t0000 (id), \
                 code TEXT NOT NULL UNIQUE, created TEXT DEFAULT CURRENT_TIMESTAMP, \
                 qty INT CHECK (qty >= 0))"
            ),
        )
        .await;
        run(
            &conn,
            &format!("CREATE INDEX t{i:04}_created ON t{i:04} (created)"),
        )
        .await;
    }
    run(&conn, "COMMIT").await;
    // What the queries alone cost, for comparison with the load.
    for (what, sql) in [
        (
            "columns",
            "SELECT m.name, x.cid, x.name, x.type, x.\"notnull\", x.dflt_value, x.pk, x.hidden \
             FROM main.sqlite_schema AS m, pragma_table_xinfo(m.name, 'main') AS x \
             WHERE m.type = 'table' ORDER BY m.name, x.cid",
        ),
        (
            "indexes",
            "SELECT m.name, il.name, il.\"unique\", il.origin, il.partial, x.seqno, x.cid, x.name \
             FROM main.sqlite_schema AS m, pragma_index_list(m.name, 'main') AS il, \
                  pragma_index_xinfo(il.name, 'main') AS x \
             WHERE m.type = 'table' AND x.key = 1 ORDER BY m.name, il.seq, x.seqno",
        ),
    ] {
        let started = Instant::now();
        let rows = conn.query(sql, &[], usize::MAX).await.unwrap().rows.len();
        println!(
            "sqlite {what} query alone: {rows} rows in {:.0} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    let intro = introspector(&*conn);
    for level in [Level::Names, Level::Structure, Level::Full] {
        let mut times = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            let snapshot = intro.load(&Scope::current(), level).await.unwrap();
            times.push(started.elapsed());
            let main = &snapshot.schemas[0];
            assert_eq!(
                main.objects
                    .iter()
                    .filter(|o| o.kind == ObjectKind::Table)
                    .count(),
                5000
            );
        }
        times.sort();
        println!(
            "sqlite load {level:?}: 5000 tables, 25000 columns: median {:.0} ms, best {:.0} ms",
            times[2].as_secs_f64() * 1000.0,
            times[0].as_secs_f64() * 1000.0
        );
    }
}
