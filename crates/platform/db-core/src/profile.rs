//! Connection profiles (ADR 0004): one saved connection without secrets,
//! with the options the IDE applies to its sessions (ADR 0021, the
//! `gui` table). The profiles file itself (TOML) belongs to `db-service`;
//! the types live here so that the database module's crates, which never
//! depend on `db-service`, can read and edit them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::schema::{Level, Scope};
use crate::{
    Backend, ConnectParams, Dialect, Error, Result, Secret, TlsMode, TlsParams, TxMode, opt,
};

/// One saved connection. Passwords are never part of a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub backend: Backend,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Oracle service name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Oracle SID (instead of `service`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    /// Oracle connect string: TNS alias, Easy Connect or descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsn: Option<String>,
    /// PostgreSQL database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// SQLite file (or `:memory:`). `~` expands to the home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// SQLite: create the file when it does not exist.
    #[serde(default, skip_serializing_if = "is_false")]
    pub create: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user: String,
    /// Name of an environment variable holding the password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsProfile>,
    /// Extra backend options (see `db_core::opt`), never secrets.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
    /// What the IDE applies to the data source's sessions and shows of it
    /// (`[profiles.<name>.gui]`, ADR 0021); the CLI ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui: Option<GuiOptions>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsProfile {
    #[serde(default)]
    pub mode: TlsMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_dir: Option<String>,
    /// Name of an environment variable holding the wallet password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_password_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Profile {
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            host: String::new(),
            port: None,
            service: None,
            sid: None,
            dsn: None,
            database: None,
            path: None,
            create: false,
            user: String::new(),
            password_env: None,
            connect_timeout_secs: None,
            query_timeout_secs: None,
            read_only: false,
            tls: None,
            options: BTreeMap::new(),
            gui: None,
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.backend.dialect()
    }

    pub fn port_or_default(&self) -> u16 {
        self.port
            .or(self.backend.default_port())
            .unwrap_or_default()
    }

    pub fn tls_mode(&self) -> TlsMode {
        self.tls.as_ref().map(|t| t.mode).unwrap_or_default()
    }

    /// The database part of the target: service, SID, database or path.
    pub fn database_name(&self) -> Option<&str> {
        match self.dialect() {
            Dialect::Oracle => self.service.as_deref().or(self.sid.as_deref()),
            Dialect::Postgres => self.database.as_deref(),
            Dialect::Sqlite => self.path.as_deref(),
        }
    }

    /// The IDE options, defaults when the profile has none.
    pub fn gui_options(&self) -> GuiOptions {
        self.gui.clone().unwrap_or_default()
    }

    /// Whether a password is expected. SQLite never needs one; Oracle with
    /// no user uses external (OS or Kerberos) authentication.
    pub fn needs_password(&self) -> bool {
        if self
            .gui
            .as_ref()
            .is_some_and(|g| g.auth == AuthMethod::None)
        {
            return false;
        }
        match self.dialect() {
            Dialect::Sqlite => false,
            Dialect::Oracle => !self.user.is_empty(),
            Dialect::Postgres => true,
        }
    }

    /// Short human-readable target, e.g. `db.example.com:1521/ORCLPDB1`.
    pub fn target(&self) -> String {
        if let Some(dsn) = &self.dsn {
            return dsn.clone();
        }
        match self.dialect() {
            Dialect::Sqlite => self.path.clone().unwrap_or_default(),
            _ => format!(
                "{}:{}/{}",
                self.host,
                self.port_or_default(),
                self.database_name().unwrap_or("")
            ),
        }
    }

    /// Check that the fields the backend needs are present and consistent.
    pub fn validate(&self) -> Result<()> {
        let missing = |what: &str| {
            Err(Error::config(format!(
                "{what} is required for backend {}",
                self.backend
            )))
        };
        match self.dialect() {
            Dialect::Oracle => {
                if self.service.is_some() && self.sid.is_some() {
                    return Err(Error::config("set either service or sid, not both"));
                }
                if self.dsn.is_none() {
                    if self.host.is_empty() {
                        return missing("host (or dsn)");
                    }
                    if self.service.is_none() && self.sid.is_none() {
                        return missing("service or sid (or dsn)");
                    }
                }
            }
            Dialect::Postgres => {
                if self.host.is_empty() {
                    return missing("host");
                }
                if self.database.is_none() {
                    return missing("database");
                }
                if self.user.is_empty() {
                    return missing("user");
                }
            }
            Dialect::Sqlite => {
                if self.path.is_none() {
                    return missing("path");
                }
            }
        }
        if self.dsn.is_some() && self.dialect() != Dialect::Oracle {
            return Err(Error::config("dsn is only valid for Oracle backends"));
        }
        Ok(())
    }

    /// Connection parameters with the given secrets.
    pub fn to_params(&self, password: Secret, wallet_password: Secret) -> Result<ConnectParams> {
        self.validate()?;
        let mut options = self.options.clone();
        let database = match self.dialect() {
            Dialect::Oracle => {
                if let Some(sid) = &self.sid {
                    options.insert(opt::ORACLE_SID.into(), "true".into());
                    sid.clone()
                } else {
                    self.service.clone().unwrap_or_default()
                }
            }
            Dialect::Postgres => self.database.clone().unwrap_or_default(),
            Dialect::Sqlite => {
                let path = self.path.as_deref().unwrap_or_default();
                if self.create {
                    options.insert(opt::SQLITE_CREATE.into(), "true".into());
                }
                if path == ":memory:" {
                    path.to_owned()
                } else {
                    expand_home(path).to_string_lossy().into_owned()
                }
            }
        };
        if let Some(dsn) = &self.dsn {
            options.insert(opt::ORACLE_DSN.into(), dsn.clone());
        }
        let tls = self.tls.clone().unwrap_or_default();
        let path = |p: &Option<String>| p.as_deref().map(expand_home);
        Ok(ConnectParams {
            host: self.host.clone(),
            port: self.port_or_default(),
            database,
            user: self.user.clone(),
            password,
            tls: TlsParams {
                mode: tls.mode,
                ca_file: path(&tls.ca_file),
                client_cert: path(&tls.client_cert),
                client_key: path(&tls.client_key),
                wallet_dir: path(&tls.wallet_dir),
                wallet_password,
                server_name: tls.server_name.clone(),
            },
            connect_timeout: Duration::from_secs(self.connect_timeout_secs.unwrap_or(10)),
            read_only: self.read_only,
            options,
        })
    }
}

/// Profile names double as keychain accounts: keep them simple.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'));
    if ok {
        Ok(())
    } else {
        Err(Error::config(format!(
            "invalid profile name {name:?}: use letters, digits, '-', '_', '.' or '@' (max 64)"
        )))
    }
}

/// Expand a leading `~` to the home directory.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~")
        && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest.trim_start_matches(['/', '\\']));
    }
    PathBuf::from(path)
}

/// How long a password typed into the IDE is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PasswordSave {
    /// In the configured secret store, across restarts.
    #[default]
    Forever,
    /// In memory until the IDE quits.
    UntilRestart,
    /// Not kept: asked on every connect.
    Never,
}

impl PasswordSave {
    pub const ALL: [PasswordSave; 3] = [
        PasswordSave::Forever,
        PasswordSave::UntilRestart,
        PasswordSave::Never,
    ];

    pub fn id(self) -> &'static str {
        match self {
            PasswordSave::Forever => "forever",
            PasswordSave::UntilRestart => "until-restart",
            PasswordSave::Never => "never",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// How a data source authenticates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    /// A user name and a password.
    #[default]
    UserPassword,
    /// Nothing (a trusting server, a file database).
    None,
}

impl AuthMethod {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Session and display options of a data source that the IDE keeps with
/// its profile (`[profiles.<name>.gui]`, ADR 0021). Unknown keys are
/// rejected, as everywhere in the profiles file; every key is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiOptions {
    /// Where a typed password is kept.
    #[serde(default, skip_serializing_if = "PasswordSave::is_default")]
    pub password_save: PasswordSave,
    #[serde(default, skip_serializing_if = "AuthMethod::is_default")]
    pub auth: AuthMethod,
    /// The connection is edited as a URL only.
    #[serde(default, skip_serializing_if = "is_false")]
    pub url_only: bool,
    /// A note shown with the data source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// The data source's colour (`blue`, `green`, `orange`, `rose`,
    /// `violet`, `yellow`, `gray`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Transaction mode of new consoles (auto when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_mode: Option<TxMode>,
    /// Session time zone (an IANA name or an offset); the server's when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// Run a keep-alive query after this many idle seconds (off when
    /// absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive_secs: Option<u64>,
    /// Close an idle connection without an open transaction after this
    /// many seconds (off when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_disconnect_secs: Option<u64>,
    /// Statements run on every new connection before the first user
    /// statement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_script: Option<String>,
    /// All consoles share one session instead of a session each.
    #[serde(default, skip_serializing_if = "is_false")]
    pub single_session: bool,
    /// Do not refresh the metadata after DDL run in a console.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_auto_sync: bool,
    /// How much a refresh reads per schema (chosen from the object count
    /// when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    /// Refresh the metadata every this many minutes (off when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_minutes: Option<u64>,
    /// What a refresh reads (the current schema when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
}

impl GuiOptions {
    /// Every option at its default (the table is left out of the file).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Interval of the keep-alive query and of auto-disconnect shown while
/// they are off (seconds).
pub const DEFAULT_KEEP_ALIVE_SECS: u64 = 60;
pub const DEFAULT_AUTO_DISCONNECT_SECS: u64 = 300;
