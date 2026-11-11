//! Incremental refreshes and refreshes of single objects: after any DDL the
//! model an incremental refresh leaves equals the one a full read makes
//! (SQLite in every run; PostgreSQL and Oracle ignored, they need the
//! containers).

use std::sync::Arc;

use db_core::schema::{Level, ObjectKind, ObjectRef, SchemaSnapshot, Scope};
use db_core::{ConnectParams, TxMode, opt};
use db_session::{
    DataSourceOptions, EventKind, RefreshEvent, RefreshMode, RefreshSummary, SessionOptions,
};

use crate::support::{Harness, env, harness, password, unique};

/// The statements that differ between the databases.
struct Dialect {
    add_column: fn(&str, &str) -> String,
    /// Changes of the objects whose names start with the prefix, as seen by
    /// the model, ignoring what other tests do in the same schema.
    drop_table: fn(&str) -> String,
}

fn standard_add(table: &str, column: &str) -> String {
    format!("alter table {table} add column {column} integer")
}

fn oracle_add(table: &str, column: &str) -> String {
    format!("alter table {table} add ({column} integer)")
}

fn plain_drop(table: &str) -> String {
    format!("drop table {table}")
}

/// An incremental refresh that finds nothing to read. Other tests change the
/// same schema at the same time, so it takes a few tries to meet a quiet
/// moment.
fn quiet(h: &Harness, level: Level) -> RefreshSummary {
    let mut last = refresh(h, RefreshMode::Incremental, level);
    for _ in 0..10 {
        if !last.full && (last.schemas_read, last.objects_read, last.objects_dropped) == (0, 0, 0) {
            break;
        }
        last = refresh(h, RefreshMode::Incremental, level);
    }
    last
}

fn refresh(h: &Harness, mode: RefreshMode, level: Level) -> RefreshSummary {
    let job = h.handle.refresh_with(Scope::current(), level, mode);
    finished(h, job)
}

fn finished(h: &Harness, job: db_session::JobId) -> RefreshSummary {
    h.wait_for(|k| match k {
        EventKind::Refresh { job: j, event } if *j == job => match event {
            RefreshEvent::Finished { summary, .. } => Some(*summary),
            RefreshEvent::Started => None,
            other => panic!("refresh: {other:?}"),
        },
        _ => None,
    })
}

/// The current schema of the model reduced to the objects named after
/// `prefix` (case does not matter), in the order the model keeps.
fn own(h: &Harness, prefix: &str) -> SchemaSnapshot {
    let snapshot = h.handle.model().get().to_snapshot();
    let mut schema = snapshot
        .schemas
        .into_iter()
        .find(|s| s.info.is_current)
        .expect("the current schema is in the model");
    let mine = |name: &str| name.to_lowercase().starts_with(prefix);
    schema.objects.retain(|o| mine(&o.name));
    schema.relations.retain(|r| mine(&r.name));
    schema.sources.retain(|s| mine(&s.name));
    schema
}

fn run(h: &Harness, script: &str) {
    let session = h.handle.open_session(SessionOptions::default());
    h.run_ok(&session, script);
}

fn scenario(h: &Harness, fresh: &Harness, d: &Dialect) {
    let tag = unique("px_rf");
    let prefix = tag.to_lowercase();
    let (a, b, c) = (format!("{tag}_a"), format!("{tag}_b"), format!("{tag}_c"));
    run(
        h,
        &format!(
            "create table {a} (id integer primary key, label varchar(20));\n\
             create index {tag}_ix on {a} (label);\n\
             create table {b} (id integer primary key, a_id integer references {a} (id));\n\
             create view {tag}_v as select id from {a}"
        ),
    );

    // The first read is whole, and it leaves a token behind.
    let first = refresh(h, RefreshMode::Incremental, Level::Structure);
    assert!(first.full, "nothing to compare with yet: {first:?}");
    assert!(h.handle.model().get().token().is_some());

    // Nothing changed: nothing is read.
    let nothing = quiet(h, Level::Structure);
    assert!(!nothing.full, "{nothing:?}");
    assert_eq!(
        (
            nothing.schemas_read,
            nothing.objects_read,
            nothing.objects_dropped
        ),
        (0, 0, 0),
        "{nothing:?}"
    );

    // A new column, a new table, a dropped table and a new index.
    run(
        h,
        &format!(
            "{};\ncreate table {c} (id integer primary key);\n{};\ncreate index {tag}_ix2 on {a} (id, label)",
            (d.add_column)(&a, "extra"),
            (d.drop_table)(&b)
        ),
    );
    let changed = refresh(h, RefreshMode::Incremental, Level::Structure);
    assert!(!changed.full, "{changed:?}");
    assert_eq!(changed.schemas_read, 1, "{changed:?}");
    assert!(changed.objects_dropped >= 1, "{changed:?}");

    let held = own(h, &prefix);
    let all = refresh(fresh, RefreshMode::Full, Level::Structure);
    assert!(all.full);
    assert_eq!(held, own(fresh, &prefix), "incremental equals full");
    let table = held
        .relation(&a.to_uppercase())
        .or_else(|| held.relation(&a))
        .expect("the table");
    assert!(
        table
            .columns
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case("extra")),
        "{:?}",
        table.columns
    );
    assert!(
        !held.objects.iter().any(|o| o.name.eq_ignore_ascii_case(&b)),
        "the dropped table is gone"
    );
    assert!(
        held.objects
            .iter()
            .any(|o| o.name.eq_ignore_ascii_case(&format!("{tag}_ix2")))
    );

    // One object on its own.
    run(h, &(d.add_column)(&a, "more"));
    let name = held
        .objects
        .iter()
        .find(|o| o.kind == ObjectKind::Table && o.name.eq_ignore_ascii_case(&a))
        .expect("the table")
        .name
        .clone();
    let schema = held.info.name.clone();
    let job = h.handle.refresh_objects(
        &schema,
        vec![ObjectRef {
            schema: schema.clone(),
            name,
            kind: ObjectKind::Table,
        }],
        Level::Structure,
    );
    let one = finished(h, job);
    assert!(!one.full && one.objects_read >= 1, "{one:?}");
    refresh(fresh, RefreshMode::Full, Level::Structure);
    assert_eq!(
        own(h, &prefix),
        own(fresh, &prefix),
        "one object equals full"
    );

    // A deeper level asks for a whole read of the schema, not a diff.
    let deeper = refresh(h, RefreshMode::Incremental, Level::Full);
    assert_eq!(deeper.schemas_read, 1, "{deeper:?}");
    refresh(fresh, RefreshMode::Full, Level::Full);
    assert_eq!(own(h, &prefix), own(fresh, &prefix), "a deeper level");

    run(
        h,
        &format!(
            "drop view {tag}_v;\n{};\n{}",
            (d.drop_table)(&a),
            (d.drop_table)(&c)
        ),
    );
    let last = refresh(h, RefreshMode::Incremental, Level::Full);
    assert!(!last.full && last.objects_dropped >= 3, "{last:?}");
    assert!(own(h, &prefix).objects.is_empty());
}

fn sqlite_params(dir: &tempfile::TempDir) -> ConnectParams {
    let mut params = ConnectParams {
        database: dir.path().join("it.db").to_string_lossy().into_owned(),
        ..ConnectParams::default()
    };
    params
        .options
        .insert(opt::SQLITE_CREATE.into(), "true".into());
    params
}

#[test]
fn sqlite_incremental_equals_full() {
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        harness(
            Arc::new(db_sqlite::SqliteDriver),
            sqlite_params(&dir),
            DataSourceOptions::default(),
        )
    };
    let (h, fresh) = (open(), open());
    scenario(
        &h,
        &fresh,
        &Dialect {
            add_column: standard_add,
            drop_table: plain_drop,
        },
    );
}

fn pg_params() -> ConnectParams {
    ConnectParams {
        host: env("PYOXIDE_TEST_PG_HOST", "localhost"),
        port: env("PYOXIDE_TEST_PG_PORT", "55432").parse().expect("port"),
        database: "pyoxide".into(),
        user: "pyoxide".into(),
        password: password(),
        ..ConnectParams::default()
    }
}

#[test]
#[ignore = "needs the postgres container"]
fn postgres_incremental_equals_full() {
    let open = || {
        harness(
            Arc::new(db_postgres::PostgresDriver),
            pg_params(),
            DataSourceOptions::default(),
        )
    };
    let (h, fresh) = (open(), open());
    scenario(
        &h,
        &fresh,
        &Dialect {
            add_column: standard_add,
            drop_table: plain_drop,
        },
    );
}

/// A change in a transaction nobody committed is not seen by a refresh, which
/// reads on a connection of its own; the commit makes the next incremental
/// refresh read it.
#[test]
#[ignore = "needs the postgres container"]
fn postgres_uncommitted_ddl_is_seen_after_the_commit() {
    let h = harness(
        Arc::new(db_postgres::PostgresDriver),
        pg_params(),
        DataSourceOptions::default(),
    );
    let tag = unique("px_rf_tx");
    let prefix = tag.to_lowercase();
    refresh(&h, RefreshMode::Incremental, Level::Structure);

    let session = h.handle.open_session(SessionOptions {
        tx_mode: Some(TxMode::Manual),
        schema: None,
        isolation: None,
    });
    h.run_ok(&session, &format!("create table {tag} (id integer)"));
    let pending = refresh(&h, RefreshMode::Incremental, Level::Structure);
    assert!(!pending.full, "{pending:?}");
    assert!(own(&h, &prefix).objects.is_empty());

    h.commit(&session);
    let done = refresh(&h, RefreshMode::Incremental, Level::Structure);
    assert!(!done.full && done.objects_read >= 1, "{done:?}");
    assert_eq!(own(&h, &prefix).objects.len(), 1);

    run(&h, &format!("drop table {tag}"));
    let gone = refresh(&h, RefreshMode::Incremental, Level::Structure);
    assert!(gone.objects_dropped >= 1, "{gone:?}");
}

fn oracle_params() -> ConnectParams {
    ConnectParams {
        host: env("PYOXIDE_TEST_ORACLE_HOST", "localhost"),
        port: env("PYOXIDE_TEST_ORACLE_PORT", "1521")
            .parse()
            .unwrap_or(1521),
        database: env("PYOXIDE_TEST_ORACLE_SERVICE", "FREEPDB1"),
        user: "pyoxide".into(),
        password: password(),
        ..ConnectParams::default()
    }
}

#[test]
#[ignore = "needs the oracle container"]
fn oracle_incremental_equals_full() {
    let open = || {
        harness(
            Arc::new(db_oracle_thin::ThinDriver),
            oracle_params(),
            DataSourceOptions::default(),
        )
    };
    let (h, fresh) = (open(), open());
    scenario(
        &h,
        &fresh,
        &Dialect {
            add_column: oracle_add,
            drop_table: plain_drop,
        },
    );
}
