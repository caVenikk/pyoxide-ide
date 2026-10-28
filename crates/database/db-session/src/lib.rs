//! The asynchronous layer of the database module (ADR 0021): everything that
//! talks to a database on behalf of the interface, without ever blocking the
//! caller.
//!
//! - [`DbRuntime`]: a small multi-thread runtime with named threads that the
//!   product starts once. Nothing of it appears in this crate's API: the UI
//!   calls methods that return at once and receives [`Event`]s through an
//!   [`EventSink`].
//! - [`DataSourceHandle`]: one data source, a state machine
//!   ([`DataSourceState`]: disconnected, connecting, connected, failed) over
//!   a `db_core::ConnectionSource`. It owns the console sessions, up to two
//!   data connections (table grids, `COUNT(*)`, export: never in a console's
//!   transaction), short metadata connections for refreshes, keep-alive pings
//!   and the auto-disconnect, which asks instead of dropping pending changes.
//!   Every new connection gets the time zone, the startup script and the
//!   server's read-only mode of [`DataSourceOptions`].
//! - [`Session`]: a console's session, an actor that runs [`Job`]s,
//!   commit, rollback, schema and transaction mode changes in order, with
//!   [`Session::cancel`] out of band. In manual mode every statement is
//!   wrapped in a savepoint, so a failing statement leaves the transaction
//!   usable, and the pending-change count follows the statements'
//!   classification by `sql-syntax`.
//! - Jobs report, per statement, `Started`, then `ResultOpened` (with the
//!   first page) or `Affected`, then `Finished` with execute and fetch time
//!   or `Failed` with the server's error and its position ([`JobEvent`]).
//! - [`ResultPager`]: pages of a table for the data grid on the data
//!   connections (a generated `SELECT` with the user's WHERE and ORDER BY,
//!   offset paging, one row more than the page to know whether more
//!   follow) and `COUNT(*)` on request; every read is cancellable.
//! - [`commands::register`]: the headless `db.session.run` command.
//! - `fake` (feature `test-support`): a scripted connection source for tests
//!   (hangs, lost connections, cancel support, a statement log).
//!
//! No UI toolkit; tokio stays inside.

mod cell;
pub mod commands;
mod data_source;
mod dialect;
mod event;
pub mod export;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod import;
mod job;
mod options;
mod pager;
mod refresh;
mod runtime;
mod session;

pub use cell::{CellQuery, FULL_VALUE_LIMIT, is_keyable, literal};
pub use data_source::DataSourceHandle;
pub use event::{
    Command, DataSourceState, DecisionReason, Event, EventKind, EventSink, JobEvent, JobId,
    JobOutcome, RefreshEvent, ResultId, SessionEvent, SessionId, StatementError, Timing, TxStatus,
};
pub use export::{
    ExportDone, ExportEvent, ExportOptions, ExportOutcome, ExportRequest, ExportSink, ExportSource,
    ExportTarget, Format,
};
pub use import::{
    ColumnMap, Encoding, ImportDone, ImportEvent, ImportFailure, ImportFormat, ImportOutcome,
    ImportRequest, ImportSink,
};
pub use job::{ErrorPolicy, Job, JobStatement, syntax_dialect};
pub use options::{DataSourceOptions, Isolation, SessionOptions, TxPolicy};
pub use pager::{
    Page, PageError, PageSize, PagerEvent, PagerSink, ResultPager, TableQuery, TableRef,
};
pub use refresh::{RefreshMode, RefreshSummary};
pub use runtime::DbRuntime;
pub use session::{Session, SessionStatus};

#[cfg(test)]
mod tests;
