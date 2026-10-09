//! SQLite backend (rusqlite, bundled SQLite).

use async_trait::async_trait;
use db_core::{Backend, ConnectParams, Connection, Driver, Error, ErrorKind, Result};

pub struct SqliteDriver;

#[async_trait]
impl Driver for SqliteDriver {
    fn backend(&self) -> Backend {
        Backend::Sqlite
    }

    fn availability(&self) -> Result<String> {
        Err(Error::new(ErrorKind::Unavailable, "not implemented yet"))
    }

    async fn connect(&self, _params: &ConnectParams) -> Result<Box<dyn Connection>> {
        Err(Error::new(ErrorKind::Unavailable, "not implemented yet"))
    }
}
