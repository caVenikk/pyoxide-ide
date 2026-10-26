//! IDE-wide secret settings: which store keeps passwords and, for a
//! KeePass database, where its master password comes from.
//!
//! File: `$PYOXIDE_SECRETS_CONFIG`, else `<config dir>/pyoxide/secrets.toml`.
//!
//! ```toml
//! store = "keepass"              # keychain (default) | keepass | memory
//! [keepass]
//! path = "~/secrets/pyoxide.kdbx"
//! master_password = "keychain"   # keychain (default) | env | prompt
//! key_file = "~/secrets/pyoxide.key"   # optional
//! create = true                  # create the database if it is missing
//! ```
//!
//! The file never holds secret values. The master password is kept in the
//! OS credential store (service [`KEEPASS_MASTER_SERVICE`], account = the
//! database path), read from [`KEEPASS_MASTER_ENV`], or typed at a prompt.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fsutil::{atomic_write, expand_home};
use crate::keepass::{KeePassKey, KeePassOptions, KeePassStore};
use crate::{
    EnvLookup, KeyringStore, MemoryStore, Prompter, Result, Secret, SecretError, SecretErrorKind,
    SecretStore,
};

/// Environment variable that overrides the settings file location.
pub const CONFIG_ENV: &str = "PYOXIDE_SECRETS_CONFIG";
/// Environment variable with the KeePass master password (`master_password = "env"`).
pub const KEEPASS_MASTER_ENV: &str = "PYOXIDE_KEEPASS_PASSWORD";
/// OS credential store service that keeps KeePass master passwords.
pub const KEEPASS_MASTER_SERVICE: &str = "pyoxide-keepass";

const SETTINGS: &str = "secret settings";

/// Where secrets are kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoreKind {
    /// The OS credential store (macOS Keychain, Windows Credential Manager,
    /// Secret Service).
    #[default]
    #[serde(rename = "keychain")]
    Keychain,
    /// A KeePass database file.
    #[serde(rename = "keepass")]
    KeePass,
    /// Memory only: forgotten when the process exits.
    #[serde(rename = "memory")]
    Memory,
}

/// Where the KeePass master password comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MasterPasswordSource {
    /// The OS credential store, service [`KEEPASS_MASTER_SERVICE`]. Falls
    /// back to the prompt when it has no entry.
    #[default]
    Keychain,
    /// The environment variable [`KEEPASS_MASTER_ENV`].
    Env,
    /// Ask every time the store is opened.
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeePassConfig {
    /// The `.kdbx` file. `~` expands to the home directory.
    pub path: String,
    #[serde(default)]
    pub master_password: MasterPasswordSource,
    /// Optional key file, as in KeePass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<String>,
    /// Create an empty database when the file does not exist.
    #[serde(default, skip_serializing_if = "is_false")]
    pub create: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl KeePassConfig {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            master_password: MasterPasswordSource::default(),
            key_file: None,
            create: false,
        }
    }

    /// The database file with `~` expanded.
    pub fn db_path(&self) -> PathBuf {
        expand_home(&self.path)
    }

    pub fn key_file_path(&self) -> Option<PathBuf> {
        self.key_file.as_deref().map(expand_home)
    }

    /// Account under which the master password is kept in the OS store
    /// (service [`KEEPASS_MASTER_SERVICE`]).
    pub fn master_password_account(&self) -> String {
        self.db_path().to_string_lossy().into_owned()
    }
}

/// The whole settings file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretsConfig {
    #[serde(default)]
    pub store: StoreKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepass: Option<KeePassConfig>,
}

fn settings_err(kind: SecretErrorKind, message: impl Into<String>) -> SecretError {
    SecretError::new(kind, SETTINGS, message)
}

impl SecretsConfig {
    pub fn parse(text: &str) -> Result<Self> {
        // `toml` errors quote the offending line, which could hold a
        // mistyped password: report the line number only.
        toml::from_str(text).map_err(|e| {
            let line = e
                .span()
                .map(|r| text[..r.start.min(text.len())].lines().count().max(1));
            let at = line.map(|l| format!(", line {l}")).unwrap_or_default();
            settings_err(
                SecretErrorKind::Format,
                format!("secrets.toml{at}: {}", e.message()),
            )
        })
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|e| settings_err(SecretErrorKind::Format, e.to_string()))
    }

    /// Load from `path`; a missing file means the defaults.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(settings_err(
                SecretErrorKind::Io,
                format!("cannot read {}: {e}", path.display()),
            )),
        }
    }

    /// Write atomically, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<()> {
        let header = "# Pyoxide secret settings. No passwords here.\n\n";
        let text = format!("{header}{}", self.to_toml()?);
        atomic_write(path, text.as_bytes(), false).map_err(|e| {
            settings_err(
                SecretErrorKind::Io,
                format!("cannot write {}: {e}", path.display()),
            )
        })
    }
}

/// Settings file location: `$PYOXIDE_SECRETS_CONFIG`, else
/// `<config dir>/pyoxide/secrets.toml`.
pub fn config_path(env: &EnvLookup) -> PathBuf {
    if let Some(p) = env(CONFIG_ENV) {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pyoxide")
        .join("secrets.toml")
}

/// Open the configured store. KeePass needs its master password: see
/// [`MasterPasswordSource`]. Blocks (keychain access, key derivation).
pub fn open_store(
    config: &SecretsConfig,
    prompter: Option<&dyn Prompter>,
    env: &EnvLookup,
) -> Result<Arc<dyn SecretStore>> {
    open_store_with(config, prompter, env, &KeyringStore)
}

/// [`open_store`] with the OS credential store passed in (tests use a
/// memory store instead of the real keychain).
pub fn open_store_with(
    config: &SecretsConfig,
    prompter: Option<&dyn Prompter>,
    env: &EnvLookup,
    os_store: &dyn SecretStore,
) -> Result<Arc<dyn SecretStore>> {
    match config.store {
        // Cached: each secret is read from the OS store once per run, since
        // every read may ask the person for the login password.
        StoreKind::Keychain => Ok(Arc::new(crate::CachedStore::new(Arc::new(KeyringStore)))),
        StoreKind::Memory => Ok(Arc::new(MemoryStore::default())),
        StoreKind::KeePass => {
            let kp = config.keepass.as_ref().ok_or_else(|| {
                settings_err(
                    SecretErrorKind::Unavailable,
                    "store = \"keepass\" needs a [keepass] section with a path",
                )
            })?;
            let master = keepass_master_password(kp, prompter, env, os_store)?;
            let mut key = KeePassKey::new(master);
            if let Some(file) = kp.key_file_path() {
                key = key.with_key_file(file);
            }
            let options = KeePassOptions {
                create: kp.create,
                ..KeePassOptions::default()
            };
            Ok(Arc::new(KeePassStore::open(kp.db_path(), key, options)?))
        }
    }
}

/// The store chosen in the settings file, opened on first use (see
/// [`crate::LazyStore`]), so commands that never touch a secret never unlock
/// a KeePass database. A broken settings file surfaces on first use.
pub fn configured_store(
    prompter: Option<Arc<dyn Prompter>>,
    env: EnvLookup,
) -> Arc<dyn SecretStore> {
    let config = match SecretsConfig::load(&config_path(&env)) {
        Ok(c) => c,
        Err(e) => {
            return Arc::new(crate::LazyStore::new(SETTINGS, move || Err(e)));
        }
    };
    let name = match config.store {
        StoreKind::Keychain => KeyringStore.name().to_owned(),
        StoreKind::Memory => MemoryStore::default().name().to_owned(),
        StoreKind::KeePass => format!(
            "KeePass database {}",
            config
                .keepass
                .as_ref()
                .map(|k| k.path.as_str())
                .unwrap_or("?")
        ),
    };
    Arc::new(crate::LazyStore::new(name, move || {
        open_store(&config, prompter.as_deref(), &env)
    }))
}

/// Find the KeePass master password as configured.
pub fn keepass_master_password(
    kp: &KeePassConfig,
    prompter: Option<&dyn Prompter>,
    env: &EnvLookup,
    os_store: &dyn SecretStore,
) -> Result<Secret> {
    let prompt = |why: &str| -> Result<Secret> {
        let p = prompter.ok_or_else(|| {
            settings_err(
                SecretErrorKind::Locked,
                format!(
                    "KeePass master password for {}: {why}",
                    kp.db_path().display()
                ),
            )
        })?;
        p.secret(&format!(
            "KeePass master password ({}): ",
            kp.db_path().display()
        ))
        .ok_or_else(|| settings_err(SecretErrorKind::Locked, "no master password entered"))
    };
    match kp.master_password {
        MasterPasswordSource::Env => match env(KEEPASS_MASTER_ENV) {
            Some(v) => Ok(Secret::new(v)),
            None => Err(settings_err(
                SecretErrorKind::Locked,
                format!("set {KEEPASS_MASTER_ENV} to the KeePass master password"),
            )),
        },
        MasterPasswordSource::Prompt => prompt("no interactive prompt available"),
        MasterPasswordSource::Keychain => {
            match os_store.get(KEEPASS_MASTER_SERVICE, &kp.master_password_account()) {
                Ok(Some(s)) => Ok(s),
                Ok(None) => prompt(&format!(
                    "not found in {} (service {KEEPASS_MASTER_SERVICE})",
                    os_store.name()
                )),
                Err(e) if prompter.is_some() => prompt(&e.to_string()),
                Err(e) => Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::keepass::tests::fast_create;

    fn env(vars: &[(&str, &str)]) -> EnvLookup {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Arc::new(move |k| map.get(k).cloned())
    }

    struct Typed(&'static str);
    impl Prompter for Typed {
        fn secret(&self, _: &str) -> Option<Secret> {
            Some(Secret::new(self.0))
        }
    }

    const SAMPLE: &str = r#"
store = "keepass"
[keepass]
path = "/srv/secrets/pyoxide.kdbx"
master_password = "env"
key_file = "~/pyoxide.key"
create = true
"#;

    #[test]
    fn parses_and_round_trips() {
        let c = SecretsConfig::parse(SAMPLE).unwrap();
        assert_eq!(c.store, StoreKind::KeePass);
        let kp = c.keepass.as_ref().unwrap();
        assert_eq!(kp.master_password, MasterPasswordSource::Env);
        assert_eq!(kp.db_path(), PathBuf::from("/srv/secrets/pyoxide.kdbx"));
        assert_eq!(
            kp.key_file_path().unwrap(),
            dirs::home_dir().unwrap().join("pyoxide.key")
        );
        assert!(kp.create);
        assert_eq!(SecretsConfig::parse(&c.to_toml().unwrap()).unwrap(), c);
    }

    #[test]
    fn defaults() {
        let c = SecretsConfig::parse("").unwrap();
        assert_eq!(c.store, StoreKind::Keychain);
        assert!(c.keepass.is_none());
        let c = SecretsConfig::parse("store = \"memory\"").unwrap();
        assert_eq!(c.store, StoreKind::Memory);
        let kp: SecretsConfig = SecretsConfig::parse("[keepass]\npath = \"x.kdbx\"").unwrap();
        assert_eq!(
            kp.keepass.unwrap().master_password,
            MasterPasswordSource::Keychain
        );
    }

    #[test]
    fn rejects_unknown_keys_without_echoing_them() {
        let e = SecretsConfig::parse("store = \"keepass\"\nmaster = \"hunter2\"\n").unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Format);
        assert!(!e.to_string().contains("hunter2"));
        let e = SecretsConfig::parse("store = \"vault\"").unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Format);
    }

    #[test]
    fn load_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pyoxide").join("secrets.toml");
        assert_eq!(
            SecretsConfig::load(&path).unwrap(),
            SecretsConfig::default()
        );
        let c = SecretsConfig::parse(SAMPLE).unwrap();
        c.save(&path).unwrap();
        assert_eq!(SecretsConfig::load(&path).unwrap(), c);
    }

    #[test]
    fn config_path_env_override() {
        let e = env(&[(CONFIG_ENV, "/tmp/x/secrets.toml")]);
        assert_eq!(config_path(&e), PathBuf::from("/tmp/x/secrets.toml"));
        assert!(config_path(&env(&[])).ends_with("pyoxide/secrets.toml"));
    }

    #[test]
    fn opens_simple_stores() {
        let os = MemoryStore::default();
        let e = env(&[]);
        let memory = SecretsConfig {
            store: StoreKind::Memory,
            keepass: None,
        };
        let s = open_store_with(&memory, None, &e, &os).unwrap();
        assert_eq!(s.name(), "memory (not saved)");
        let s = open_store_with(&SecretsConfig::default(), None, &e, &os).unwrap();
        assert_eq!(s.name(), "OS credential store");
        let broken = SecretsConfig {
            store: StoreKind::KeePass,
            keepass: None,
        };
        let err = open_store_with(&broken, None, &e, &os).err().unwrap();
        assert_eq!(err.kind, SecretErrorKind::Unavailable);
    }

    fn keepass_config(dir: &Path, source: MasterPasswordSource) -> SecretsConfig {
        SecretsConfig {
            store: StoreKind::KeePass,
            keepass: Some(KeePassConfig {
                path: dir.join("s.kdbx").to_string_lossy().into_owned(),
                master_password: source,
                key_file: None,
                create: false,
            }),
        }
    }

    #[test]
    fn master_password_sources() {
        let dir = tempfile::tempdir().unwrap();
        // A database created with the fast KDF; open_store then reads it.
        let path = dir.path().join("s.kdbx");
        let seed =
            KeePassStore::open(&path, KeePassKey::new(Secret::new("m")), fast_create()).unwrap();
        seed.set("svc", "acc", &Secret::new("value")).unwrap();
        let os = MemoryStore::default();

        // env
        let c = keepass_config(dir.path(), MasterPasswordSource::Env);
        let err = open_store_with(&c, None, &env(&[]), &os).err().unwrap();
        assert_eq!(err.kind, SecretErrorKind::Locked);
        let s = open_store_with(&c, None, &env(&[(KEEPASS_MASTER_ENV, "m")]), &os).unwrap();
        assert_eq!(s.get("svc", "acc").unwrap().unwrap().expose(), "value");
        let err = open_store_with(&c, None, &env(&[(KEEPASS_MASTER_ENV, "bad")]), &os)
            .err()
            .unwrap();
        assert_eq!(err.kind, SecretErrorKind::Locked);

        // prompt
        let c = keepass_config(dir.path(), MasterPasswordSource::Prompt);
        let err = open_store_with(&c, None, &env(&[]), &os).err().unwrap();
        assert_eq!(err.kind, SecretErrorKind::Locked);
        let s = open_store_with(&c, Some(&Typed("m")), &env(&[]), &os).unwrap();
        assert_eq!(s.name(), crate::keepass::STORE_NAME);

        // keychain, falling back to the prompt
        let c = keepass_config(dir.path(), MasterPasswordSource::Keychain);
        let err = open_store_with(&c, None, &env(&[]), &os).err().unwrap();
        assert_eq!(err.kind, SecretErrorKind::Locked);
        assert!(open_store_with(&c, Some(&Typed("m")), &env(&[]), &os).is_ok());
        let account = c.keepass.as_ref().unwrap().master_password_account();
        os.set(KEEPASS_MASTER_SERVICE, &account, &Secret::new("m"))
            .unwrap();
        let s = open_store_with(&c, None, &env(&[]), &os).unwrap();
        assert_eq!(s.get("svc", "acc").unwrap().unwrap().expose(), "value");
    }
}
