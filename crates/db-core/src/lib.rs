//! Backend-agnostic database client interface.
//!
//! Drivers (`db-oracle-thin`, `db-oracle-odpi`, ...) implement [`Driver`] and
//! [`Connection`]; the IDE and the CLI only talk to these traits.

pub mod introspection;
mod render;

use async_trait::async_trait;

pub use render::render_table;

/// Errors surfaced by any backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connect failed: {0}")]
    Connect(String),
    #[error("query failed: {0}")]
    Query(String),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Connection parameters (service-name style, as used by Oracle).
#[derive(Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub service: String,
    pub user: String,
    pub password: String,
}

// Never print the password, even in debug output.
impl std::fmt::Debug for ConnectParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("service", &self.service)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

/// A cell rendered to text. Typed values come later, with the data grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Null,
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    /// Affected rows for DML; 0 for queries.
    pub rows_affected: u64,
    /// True if more rows exist than were fetched (row limit reached).
    pub truncated: bool,
}

#[async_trait]
pub trait Connection: Send + Sync {
    /// Short backend id, e.g. `oracle-thin`.
    fn backend(&self) -> &'static str;

    /// Run one statement, fetching at most `max_rows` rows.
    async fn query(&self, sql: &str, max_rows: usize) -> Result<ResultSet>;

    async fn close(&self) -> Result<()>;
}

#[async_trait]
pub trait Driver {
    async fn connect(params: &ConnectParams) -> Result<Box<dyn Connection>>;
}
