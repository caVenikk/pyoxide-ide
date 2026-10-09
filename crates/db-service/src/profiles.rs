//! Connection profiles: a TOML file with one table per profile and no
//! secrets (ADR 0004).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use db_core::{Backend, ConnectParams, Dialect, Error, Result, Secret, TlsMode, TlsParams, opt};
use serde::{Deserialize, Serialize};

/// Environment variable that overrides the profiles file location.
pub const PROFILES_ENV: &str = "PYOXIDE_DB_PROFILES";

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

    /// Whether a password is expected. SQLite never needs one; Oracle with
    /// no user uses external (OS or Kerberos) authentication.
    pub fn needs_password(&self) -> bool {
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
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(Error::config(format!(
            "invalid profile name {name:?}: use letters, digits, '-', '_' or '.' (max 64)"
        )))
    }
}

/// The whole profiles file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilesFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl ProfilesFile {
    pub fn parse(text: &str) -> Result<Self> {
        // The error text of `toml` quotes the offending source line, which
        // could hold a mistyped secret: report the line number only.
        let file: Self = toml::from_str(text).map_err(|e| {
            let line = e
                .span()
                .map(|r| text[..r.start.min(text.len())].lines().count().max(1));
            let at = line.map(|l| format!(", line {l}")).unwrap_or_default();
            Error::config(format!("profiles file{at}: {}", e.message()))
        })?;
        for name in file.profiles.keys() {
            validate_name(name)?;
        }
        Ok(file)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::internal(e.to_string()))
    }

    /// Load from `path`; a missing file is an empty set of profiles.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::config(format!(
                "cannot read {}: {e}",
                path.display()
            ))),
        }
    }

    /// Write atomically (temp file + rename), creating parent directories.
    pub fn save(&self, path: &Path) -> Result<()> {
        let io = |e: std::io::Error| Error::config(format!("cannot write {}: {e}", path.display()));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = path.with_extension("toml.tmp");
        let header = "# Pyoxide database connection profiles. No passwords here:\n\
                      # they live in the OS credential store or in environment variables.\n\n";
        std::fs::write(&tmp, format!("{header}{}", self.to_toml()?)).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    pub fn get(&self, name: &str) -> Result<&Profile> {
        self.profiles
            .get(name)
            .ok_or_else(|| Error::config(format!("no profile named {name:?}")))
    }
}

/// Default profiles file: `$PYOXIDE_DB_PROFILES`, else
/// `<config dir>/pyoxide/db-profiles.toml`.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os(PROFILES_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pyoxide")
        .join("db-profiles.toml")
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[profiles.reporting]
backend = "oracle-thin"
host = "db.example.com"
service = "ORCLPDB1"
user = "app_reader"
read_only = true
[profiles.reporting.tls]
mode = "verify-full"
wallet_dir = "/opt/wallet"

[profiles.local-pg]
backend = "postgres"
host = "localhost"
port = 55432
database = "pyoxide"
user = "pyoxide"
password_env = "PG_PASS"

[profiles.scratch]
backend = "sqlite"
path = ":memory:"
"#;

    #[test]
    fn parses_and_round_trips() {
        let f = ProfilesFile::parse(SAMPLE).unwrap();
        assert_eq!(f.profiles.len(), 3);
        let r = f.get("reporting").unwrap();
        assert_eq!(r.backend, Backend::OracleThin);
        assert_eq!(r.port_or_default(), 1521);
        assert_eq!(r.tls_mode(), TlsMode::VerifyFull);
        assert_eq!(r.target(), "db.example.com:1521/ORCLPDB1");
        let again = ProfilesFile::parse(&f.to_toml().unwrap()).unwrap();
        assert_eq!(again, f);
    }

    #[test]
    fn rejects_unknown_keys_and_passwords() {
        let bad = "[profiles.x]\nbackend = \"postgres\"\npassword = \"hunter2\"\n";
        let e = ProfilesFile::parse(bad).unwrap_err();
        assert!(e.message.contains("unknown field"), "{e}");
        assert!(!e.message.contains("hunter2"));
    }

    #[test]
    fn validates_required_fields() {
        let mut p = Profile::new(Backend::Postgres);
        p.host = "h".into();
        p.user = "u".into();
        assert!(p.validate().is_err());
        p.database = Some("d".into());
        assert!(p.validate().is_ok());

        let mut o = Profile::new(Backend::OracleOdpi);
        assert!(o.validate().is_err());
        o.dsn = Some("PRODDB".into());
        assert!(o.validate().is_ok());
        o.service = Some("a".into());
        o.sid = Some("b".into());
        assert!(o.validate().is_err());
    }

    #[test]
    fn builds_params() {
        let f = ProfilesFile::parse(SAMPLE).unwrap();
        let p = f
            .get("reporting")
            .unwrap()
            .to_params(Secret::new("pw"), Secret::default())
            .unwrap();
        assert_eq!(p.database, "ORCLPDB1");
        assert_eq!(p.port, 1521);
        assert!(p.read_only);
        assert_eq!(p.tls.wallet_dir.as_deref(), Some(Path::new("/opt/wallet")));

        let mut sid = Profile::new(Backend::OracleThin);
        sid.host = "h".into();
        sid.sid = Some("ORCL".into());
        let p = sid.to_params(Secret::default(), Secret::default()).unwrap();
        assert!(p.flag(opt::ORACLE_SID));
        assert_eq!(p.database, "ORCL");
    }

    #[test]
    fn profile_names() {
        assert!(validate_name("work-oracle_19.prod").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a b").is_err());
        assert!(validate_name("x/y").is_err());
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("db-profiles.toml");
        assert!(ProfilesFile::load(&path).unwrap().profiles.is_empty());
        let f = ProfilesFile::parse(SAMPLE).unwrap();
        f.save(&path).unwrap();
        assert_eq!(ProfilesFile::load(&path).unwrap(), f);
    }

    #[test]
    fn expands_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_home("~/x.db"), home.join("x.db"));
        assert_eq!(expand_home("/abs/x.db"), PathBuf::from("/abs/x.db"));
        assert_eq!(expand_home("~user/x"), PathBuf::from("~user/x"));
    }
}
