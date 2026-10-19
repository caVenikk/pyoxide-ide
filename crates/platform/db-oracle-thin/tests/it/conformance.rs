//! The db-testkit conformance suite against the Oracle container (see
//! docs/testing-databases.md): `docker compose up -d oracle`, then
//! `cargo test -p db-oracle-thin --test it conformance:: -- --ignored`.
//!
//! The thin driver cannot cancel a statement (oracle-rs has no break), so
//! the cancel case is skipped; a timeout drops the call and reports the
//! connection lost.

use crate::common;

use std::sync::Arc;

use db_oracle_thin::ThinDriver;
use db_testkit::{Expect, Hooks, Target};

fn target() -> Target {
    Target::new(Arc::new(ThinDriver), common::params(), Hooks::oracle())
}

db_testkit::conformance_tests! {
    target: target();
    #[ignore = "needs the oracle container"]
    typed_round_trip => Expect::Run;
    #[ignore = "needs the oracle container"]
    paging => Expect::Run;
    #[ignore = "needs the oracle container"]
    busy_rule => Expect::Run;
    #[ignore = "needs the oracle container"]
    early_close => Expect::Run;
    #[ignore = "needs the oracle container"]
    cancel => Expect::Skip;
    #[ignore = "needs the oracle container"]
    timeout => Expect::Run;
    #[ignore = "needs the oracle container"]
    transactions => Expect::Run;
    #[ignore = "needs the oracle container"]
    read_only => Expect::Run;
    #[ignore = "needs the oracle container"]
    error_kinds => Expect::Run;
}
