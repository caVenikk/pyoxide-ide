//! Backend-agnostic database client interface.
//!
//! Drivers (`db-oracle-thin`, `db-oracle-odpi`, `db-postgres`, `db-sqlite`)
//! implement [`Driver`], [`Connection`] and [`Cursor`]; the IDE, the CLI and
//! commands only talk to these traits. This crate has no driver dependencies.
//!
//! Shared driver building blocks: [`worker`] (a connection thread for
//! blocking client libraries), [`convert`] (value size limit, exact
//! decimals), [`transaction`] (transaction bookkeeping), [`oracle`] (Oracle
//! error codes and statement text) and [`timeout`].

mod connection;
pub mod convert;
mod error;
pub mod oracle;
pub mod oracle_net;
mod params;
mod render;
pub mod schema;
pub mod timeout;
pub mod transaction;
mod value;
pub mod worker;

pub use connection::{
    BusyFlag, BusyGuard, CleanupGuard, Column, Connection, Cursor, Driver, ExecOptions, Outcome,
    ResultSet, Row, VecCursor, collect,
};
pub use error::{Error, ErrorKind, Result};
pub use params::{Backend, ConnectParams, Dialect, Secret, TlsMode, TlsParams, TxMode, opt};
pub use render::render_table;
pub use value::{Value, ValueKind};
