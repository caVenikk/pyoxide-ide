//! Connection profiles: a TOML file with one table per profile and no
//! secrets (ADR 0004).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use db_core::{Error, Result};
use serde::{Deserialize, Serialize};

pub use db_core::profile::{
    AuthMethod, GuiOptions, PasswordSave, Profile, TlsProfile, expand_home, validate_name,
};

/// Environment variable that overrides the profiles file location.
pub const PROFILES_ENV: &str = "PYOXIDE_DB_PROFILES";

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

#[cfg(test)]
mod tests {
    use super::*;
    use db_core::schema::Scope;
    use db_core::{Backend, Secret, TlsMode, TxMode, opt};

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
        // The IDE's default names (`postgres@localhost`).
        assert!(validate_name("postgres@localhost").is_ok());
    }

    /// The IDE's options round-trip under `[profiles.<name>.gui]`; their
    /// unknown keys are rejected like the profile's; a profile without
    /// them has none.
    #[test]
    fn gui_options() {
        let text = r#"
[profiles.local-pg]
backend = "postgres"
host = "localhost"
database = "pyoxide"
user = "pyoxide"
[profiles.local-pg.gui]
password_save = "never"
tx_mode = "manual"
time_zone = "UTC"
keep_alive_secs = 60
startup_script = "set application_name = 'x'"
[profiles.local-pg.gui.scope]
filter = { list = ["public", "demo"] }
"#;
        let f = ProfilesFile::parse(text).unwrap();
        let gui = f.get("local-pg").unwrap().gui_options();
        assert_eq!(gui.password_save, PasswordSave::Never);
        assert_eq!(gui.tx_mode, Some(TxMode::Manual));
        assert_eq!(gui.keep_alive_secs, Some(60));
        assert_eq!(gui.scope, Some(Scope::list(["public", "demo"])));
        assert_eq!(ProfilesFile::parse(&f.to_toml().unwrap()).unwrap(), f);

        let bad = text.replace("time_zone", "timezone");
        let e = ProfilesFile::parse(&bad).unwrap_err();
        assert!(e.message.contains("unknown field"), "{e}");

        let plain = ProfilesFile::parse(SAMPLE).unwrap();
        assert!(plain.profiles.values().all(|p| p.gui.is_none()));
        assert!(!plain.to_toml().unwrap().contains("gui"));
    }

    #[test]
    fn no_auth_needs_no_password() {
        let mut p = Profile::new(Backend::Postgres);
        assert!(p.needs_password());
        p.gui = Some(GuiOptions {
            auth: AuthMethod::None,
            ..GuiOptions::default()
        });
        assert!(!p.needs_password());
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
