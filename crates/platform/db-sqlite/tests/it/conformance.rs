//! The db-testkit conformance suite against SQLite, one database file per
//! case.

use std::sync::Arc;

use db_core::{ConnectParams, opt};
use db_sqlite::SqliteDriver;
use db_testkit::{Expect, Hooks, Target};

fn target() -> Target {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut params = ConnectParams {
        database: dir
            .path()
            .join("conformance.db")
            .to_string_lossy()
            .into_owned(),
        ..ConnectParams::default()
    };
    params
        .options
        .insert(opt::SQLITE_CREATE.into(), "true".into());
    Target::new(Arc::new(SqliteDriver), params, Hooks::sqlite()).keep(dir)
}

db_testkit::conformance_tests! {
    target: target();
    typed_round_trip => Expect::Run;
    paging => Expect::Run;
    busy_rule => Expect::Run;
    early_close => Expect::Run;
    cancel => Expect::Run;
    timeout => Expect::Run;
    transactions => Expect::Run;
    read_only => Expect::Run;
    error_kinds => Expect::Run;
    bulk_load => Expect::Run;
    load_query_count => Expect::Run;
    savepoint_round_trip => Expect::Run;
    cursor_closed_by_next_statement => Expect::Run;
    cancel_while_paging => Expect::Run;
}
