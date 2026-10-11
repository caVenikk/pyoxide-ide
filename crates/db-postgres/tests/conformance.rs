//! The db-testkit conformance suite against the PostgreSQL container of
//! `docker-compose.yml` (see docs/testing-databases.md):
//!
//! ```bash
//! docker compose up -d postgres
//! cargo test -p db-postgres --test conformance -- --ignored
//! ```

use std::sync::Arc;

use db_core::{ConnectParams, Secret};
use db_postgres::PostgresDriver;
use db_testkit::{Expect, Hooks, Target};

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn target() -> Target {
    let params = ConnectParams {
        host: env_or("PYOXIDE_TEST_PG_HOST", "localhost"),
        port: env_or("PYOXIDE_TEST_PG_PORT", "55432")
            .parse()
            .expect("port"),
        database: "pyoxide".into(),
        user: "pyoxide".into(),
        password: Secret::new(env_or("PYOXIDE_DB_PASSWORD", "pyoxide")),
        ..ConnectParams::default()
    };
    Target::new(Arc::new(PostgresDriver), params, Hooks::postgres())
}

db_testkit::conformance_tests! {
    target: target();
    #[ignore = "needs the postgres container"]
    typed_round_trip => Expect::Run;
    #[ignore = "needs the postgres container"]
    paging => Expect::Run;
    #[ignore = "needs the postgres container"]
    busy_rule => Expect::Run;
    #[ignore = "needs the postgres container"]
    early_close => Expect::Run;
    #[ignore = "needs the postgres container"]
    cancel => Expect::Run;
    #[ignore = "needs the postgres container"]
    timeout => Expect::Run;
    #[ignore = "needs the postgres container"]
    transactions => Expect::Run;
    #[ignore = "needs the postgres container"]
    read_only => Expect::Run;
    #[ignore = "needs the postgres container"]
    error_kinds => Expect::Run;
}
