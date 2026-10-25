use std::time::Duration;

use db_core::TxMode;
use db_core::timeout::CANCEL_GRACE;
use serde::{Deserialize, Serialize};

/// Session options of a data source (the `gui` table of a profile, ADR 0021).
/// They apply to every new connection of any purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataSourceOptions {
    /// Run in every new connection before anything else: statements split by
    /// the dialect's rules.
    pub startup_script: Option<String>,
    /// Session time zone (`Europe/Berlin`, `+02:00`); PostgreSQL and Oracle.
    pub time_zone: Option<String>,
    /// Put every connection into the server's read-only mode where the
    /// engine has one (PostgreSQL: read-only transactions, SQLite:
    /// `query_only`; Oracle: the driver refuses writes). A write then fails
    /// with the server's error.
    pub read_only: bool,
    /// Transaction mode of new console sessions.
    pub default_tx_mode: TxMode,
    /// Send a ping on a connection idle this long.
    pub keep_alive: Option<Duration>,
    /// Close every connection after this long without activity. With pending
    /// changes in a session the user is asked instead
    /// ([`DecisionReason::IdleDisconnect`](crate::DecisionReason::IdleDisconnect)).
    pub auto_disconnect: Option<Duration>,
    /// All consoles share one session.
    pub single_session: bool,
    /// Rows in the first page of a result and in every fetch.
    pub page_size: usize,
    /// Rows per network round trip.
    pub fetch_size: usize,
    /// Longer values come back as `Value::Truncated`.
    pub max_value_bytes: usize,
    /// Time limit of one statement; `None`: no limit.
    pub statement_timeout: Option<Duration>,
    /// After a cancel request, how long a statement may keep running before
    /// the session abandons the connection and opens a new one.
    pub cancel_grace: Duration,
    /// Connections for table grids, `COUNT(*)` and export.
    pub data_connections: usize,
}

impl Default for DataSourceOptions {
    fn default() -> Self {
        Self {
            startup_script: None,
            time_zone: None,
            read_only: false,
            default_tx_mode: TxMode::Auto,
            keep_alive: None,
            auto_disconnect: None,
            single_session: false,
            page_size: 500,
            fetch_size: 500,
            max_value_bytes: 1 << 20,
            statement_timeout: None,
            cancel_grace: CANCEL_GRACE,
            data_connections: 2,
        }
    }
}

/// Options of one console session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionOptions {
    /// Transaction mode; the data source's default when `None`.
    pub tx_mode: Option<TxMode>,
    /// Schema to switch to after connecting (PostgreSQL: the search path,
    /// Oracle: the current schema).
    pub schema: Option<String>,
}

/// What to do with pending changes when an action would end a transaction
/// (close, disconnect, switch to auto-commit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxPolicy {
    /// Do nothing and ask
    /// ([`EventKind::DecisionNeeded`](crate::EventKind::DecisionNeeded)) when
    /// changes are pending.
    #[default]
    Ask,
    Commit,
    Rollback,
}
