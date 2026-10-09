//! Database engine service: picks the driver for a backend. Profiles,
//! secrets, diagnostics and the `db.*` commands live here as well.

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
