use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use secrets::Secret;

/// A concrete driver implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Backend {
    /// Oracle over the pure-Rust thin driver.
    #[serde(rename = "oracle-thin")]
    OracleThin,
    /// Oracle through an installed Oracle Client (ODPI-C).
    #[serde(rename = "oracle-odpi")]
    OracleOdpi,
    #[serde(rename = "postgres")]
    Postgres,
    #[serde(rename = "sqlite")]
    Sqlite,
}

impl Backend {
    pub const ALL: [Backend; 4] = [
        Backend::OracleThin,
        Backend::OracleOdpi,
        Backend::Postgres,
        Backend::Sqlite,
    ];

    /// Stable id used in profiles, the CLI and logs.
    pub fn id(self) -> &'static str {
        match self {
            Backend::OracleThin => "oracle-thin",
            Backend::OracleOdpi => "oracle-odpi",
            Backend::Postgres => "postgres",
            Backend::Sqlite => "sqlite",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == id)
    }

    pub fn dialect(self) -> Dialect {
        match self {
            Backend::OracleThin | Backend::OracleOdpi => Dialect::Oracle,
            Backend::Postgres => Dialect::Postgres,
            Backend::Sqlite => Dialect::Sqlite,
        }
    }

    /// Default TCP port, `None` for file-based backends.
    pub fn default_port(self) -> Option<u16> {
        match self.dialect() {
            Dialect::Oracle => Some(1521),
            Dialect::Postgres => Some(5432),
            Dialect::Sqlite => None,
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// SQL dialect, shared by all drivers of one database engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    Oracle,
    Postgres,
    Sqlite,
}

/// How strictly to use TLS. Names follow PostgreSQL's `sslmode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    /// Plain TCP.
    #[default]
    Disable,
    /// TLS if the server offers it, otherwise plain (PostgreSQL only).
    Prefer,
    /// TLS without certificate verification.
    Require,
    /// TLS, the certificate chain must be trusted.
    VerifyCa,
    /// TLS, trusted chain and the host name must match the certificate.
    VerifyFull,
}

impl TlsMode {
    pub fn is_enabled(self) -> bool {
        self != TlsMode::Disable
    }
}

/// TLS material. Paths only; key and wallet passwords are [`Secret`]s.
#[derive(Debug, Clone, Default)]
pub struct TlsParams {
    pub mode: TlsMode,
    /// PEM file with trusted CA certificates (in addition to the OS store).
    pub ca_file: Option<PathBuf>,
    /// PEM client certificate and key for mutual TLS.
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    /// Oracle wallet directory (`ewallet.pem` for the thin driver,
    /// `cwallet.sso` for the Oracle Client).
    pub wallet_dir: Option<PathBuf>,
    pub wallet_password: Secret,
    /// Expected certificate DN or host name when it differs from `host`.
    pub server_name: Option<String>,
}

/// Option keys for [`ConnectParams::options`]. Values are never secrets.
pub mod opt {
    /// Oracle: `database` is a SID, not a service name (`"true"`).
    pub const ORACLE_SID: &str = "oracle.sid";
    /// Oracle Client only: a full connect string (TNS alias, Easy Connect
    /// Plus or descriptor), used instead of host, port and database.
    pub const ORACLE_DSN: &str = "oracle.dsn";
    /// PostgreSQL `application_name`.
    pub const APPLICATION_NAME: &str = "application_name";
    /// Oracle Client only: directory with the Oracle Client libraries.
    pub const ORACLE_LIB_DIR: &str = "oracle.lib_dir";
    /// Oracle Client only: directory with `tnsnames.ora` and `sqlnet.ora`.
    pub const ORACLE_CONFIG_DIR: &str = "oracle.config_dir";
    /// Oracle thin driver only: the session data unit to request, in bytes
    /// (512..=65535; default 65535). Mainly for testing servers that
    /// negotiate a small SDU.
    pub const ORACLE_SDU: &str = "oracle.sdu";
    /// SQLite: create the file if it does not exist (`"true"`).
    pub const SQLITE_CREATE: &str = "sqlite.create";
    /// SQLite: how long to wait for a lock held by another connection, in
    /// milliseconds (default 5000).
    pub const SQLITE_BUSY_TIMEOUT_MS: &str = "sqlite.busy_timeout_ms";
}

/// Everything needed to open a connection.
#[derive(Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    /// Oracle service name (or SID, see [`opt::ORACLE_SID`]), PostgreSQL
    /// database name, SQLite file path or `:memory:`.
    pub database: String,
    pub user: String,
    pub password: Secret,
    pub tls: TlsParams,
    pub connect_timeout: Duration,
    /// Open the session read-only where the backend supports it.
    pub read_only: bool,
    /// Backend-specific settings, see [`opt`].
    pub options: BTreeMap<String, String>,
}

impl Default for ConnectParams {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 0,
            database: String::new(),
            user: String::new(),
            password: Secret::default(),
            tls: TlsParams::default(),
            connect_timeout: Duration::from_secs(10),
            read_only: false,
            options: BTreeMap::new(),
        }
    }
}

impl ConnectParams {
    pub fn option(&self, key: &str) -> Option<&str> {
        self.options.get(key).map(String::as_str)
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.option(key), Some("true" | "1" | "yes"))
    }
}

// Never print the password, even in debug output.
impl std::fmt::Debug for ConnectParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("tls", &self.tls)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_only", &self.read_only)
            .field("options", &self.options)
            .finish()
    }
}

/// Transaction handling of a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxMode {
    /// Every statement commits on success.
    #[default]
    Auto,
    /// Changes stay pending until `commit` or `rollback`.
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_password() {
        let mut p = ConnectParams {
            password: Secret::new("hunter2"),
            ..Default::default()
        };
        p.tls.wallet_password = Secret::new("walletpw");
        let s = format!("{p:?}");
        assert!(!s.contains("hunter2"));
        assert!(!s.contains("walletpw"));
        assert!(s.contains("<redacted>"));
    }

    #[test]
    fn backend_ids_round_trip() {
        for b in Backend::ALL {
            assert_eq!(Backend::from_id(b.id()), Some(b));
        }
        assert_eq!(Backend::from_id("mysql"), None);
    }
}
