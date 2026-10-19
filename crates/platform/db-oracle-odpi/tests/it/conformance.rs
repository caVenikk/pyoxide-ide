//! The db-testkit conformance suite against the Oracle container, through
//! an installed Oracle Client (see the crate README and
//! docs/testing-databases.md):
//!
//! ```text
//! docker compose up -d oracle
//! cargo test -p db-oracle-odpi --test it conformance:: -- --ignored
//! ```

use std::sync::Arc;
use std::time::Duration;

use db_core::{ConnectParams, Secret};
use db_oracle_odpi::OdpiDriver;
use db_testkit::{Expect, Hooks, Target};

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn target() -> Target {
    let params = ConnectParams {
        host: env_or("PYOXIDE_TEST_ORACLE_HOST", "localhost"),
        port: env_or("PYOXIDE_TEST_ORACLE_PORT", "1521").parse().unwrap(),
        database: env_or("PYOXIDE_TEST_ORACLE_SERVICE", "FREEPDB1"),
        user: "pyoxide".into(),
        password: Secret::new(env_or("PYOXIDE_DB_PASSWORD", "pyoxide")),
        connect_timeout: Duration::from_secs(10),
        ..ConnectParams::default()
    };
    Target::new(Arc::new(OdpiDriver), params, Hooks::oracle())
}

db_testkit::conformance_tests! {
    target: target();
    #[ignore = "needs an Oracle Client and the oracle container"]
    typed_round_trip => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container"]
    paging => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container"]
    busy_rule => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container"]
    early_close => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container (12c+ for DBMS_SESSION.SLEEP)"]
    cancel => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container (12c+ for DBMS_SESSION.SLEEP)"]
    timeout => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container"]
    transactions => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container"]
    read_only => Expect::Run;
    #[ignore = "needs an Oracle Client and the oracle container"]
    error_kinds => Expect::Run;
}
