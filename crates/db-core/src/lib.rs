//! Backend-agnostic database client interface.
//!
//! Drivers (`db-oracle-thin`, `db-oracle-odpi`, `db-postgres`, `db-sqlite`)
//! implement [`Driver`], [`Connection`] and [`Cursor`]; the IDE, the CLI and
//! commands only talk to these traits. This crate has no driver dependencies.

mod connection;
mod error;
pub mod oracle;
pub mod oracle_net;
mod params;
mod render;
pub mod schema;
pub mod timeout;
mod value;

pub use connection::{
    BusyFlag, BusyGuard, Column, Connection, Cursor, Driver, ExecOptions, Outcome, ResultSet, Row,
    VecCursor, collect,
};
pub use error::{Error, ErrorKind, Result};
pub use params::{Backend, ConnectParams, Dialect, Secret, TlsMode, TlsParams, TxMode, opt};
pub use render::render_table;
pub use value::{Value, ValueKind};
