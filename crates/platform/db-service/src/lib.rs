//! Database engine service: connection profiles and their secrets, driver
//! dispatch, diagnostics and the `db.*` commands. The CLI and the IDE call
//! the same functions.

pub mod diagnose;
pub mod hints;
pub mod import;
pub mod ops;
pub mod profiles;
pub mod redact;
pub mod secrets;
pub mod service;
mod tls_probe;

use std::sync::Arc;

use db_core::{Backend, Driver};

pub use service::{DbService, Opened, Target};

/// Client-side availability for a profile: like `Driver::availability`, but
/// the Oracle Client backend honours the profile's library directory.
pub fn availability_for(profile: &profiles::Profile) -> db_core::Result<String> {
    if profile.backend == Backend::OracleOdpi {
        let dir = |k: &str| profile.options.get(k).map(|d| profiles::expand_home(d));
        let lib = dir(db_core::opt::ORACLE_LIB_DIR);
        let cfg = dir(db_core::opt::ORACLE_CONFIG_DIR);
        if lib.is_some() || cfg.is_some() {
            return db_oracle_odpi::init_client(lib.as_deref(), cfg.as_deref());
        }
    }
    driver(profile.backend).availability()
}

/// The driver for a backend.
pub fn driver(backend: Backend) -> Arc<dyn Driver> {
    match backend {
        Backend::OracleThin => Arc::new(db_oracle_thin::ThinDriver),
        Backend::OracleOdpi => Arc::new(db_oracle_odpi::OdpiDriver),
        Backend::Postgres => Arc::new(db_postgres::PostgresDriver),
        Backend::Sqlite => Arc::new(db_sqlite::SqliteDriver),
    }
}
