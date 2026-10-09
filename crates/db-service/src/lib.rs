//! Database engine service: connection profiles and their secrets, driver
//! dispatch, diagnostics and the `db.*` commands. The CLI and the IDE call
//! the same functions.

pub mod profiles;
pub mod secrets;

use std::sync::Arc;

use db_core::{Backend, Driver};

/// The driver for a backend.
pub fn driver(backend: Backend) -> Arc<dyn Driver> {
    match backend {
        Backend::OracleThin => Arc::new(db_oracle_thin::ThinDriver),
        Backend::OracleOdpi => Arc::new(db_oracle_odpi::OdpiDriver),
        Backend::Postgres => Arc::new(db_postgres::PostgresDriver),
        Backend::Sqlite => Arc::new(db_sqlite::SqliteDriver),
    }
}
