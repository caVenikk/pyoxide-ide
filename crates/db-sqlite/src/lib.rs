//! SQLite backend (rusqlite, bundled SQLite).
//!
//! Every connection owns a worker thread that owns the `rusqlite` connection;
//! the async side sends requests over a channel and awaits the replies, so no
//! call blocks a runtime thread. Result sets are read incrementally: the
//! worker keeps the prepared statement open and steps only as many rows as
//! each `next_page` asks for.

mod conn;
mod convert;
mod error;
mod worker;

pub use conn::{OPT_BUSY_TIMEOUT_MS, SqliteConnection, SqliteDriver};
