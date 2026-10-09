//! Secret storage shared by every part of the IDE that needs credentials
//! (database connections today; VCS and package indexes later).
//!
//! Backends mirror the usual IDE choices: the OS credential store (default),
//! a KeePass database file ([`keepass`]), or memory only (forget on exit).
//! Secret values only travel as [`Secret`]: redacted in `Debug`, wiped on drop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zeroize::Zeroize;

mod fsutil;
pub mod keepass;
#[cfg(test)]
mod test_kdbx3;

pub use keepass::{KeePassKey, KeePassOptions, KeePassStore};

/// A secret string (password, passphrase, token). Never printed, never
/// serialized, wiped from memory on drop.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "<empty>"
        } else {
            "<redacted>"
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretErrorKind {
    /// The store cannot be used here (no keychain service, missing file).
    Unavailable,
    /// The store is locked: wrong or missing master password.
    Locked,
    /// Reading or writing failed.
    Io,
    /// The store's data is malformed or unsupported.
    Format,
}

/// Failure of a secret store. The message never contains secret values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{store}: {message}")]
pub struct SecretError {
    pub kind: SecretErrorKind,
    pub store: String,
    pub message: String,
}

impl SecretError {
    pub fn new(kind: SecretErrorKind, store: &str, message: impl Into<String>) -> Self {
        Self {
            kind,
            store: store.to_owned(),
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, SecretError>;

/// A place secrets are kept between runs. Keys are a `service` (namespace,
/// e.g. `pyoxide-db`) and an `account` (e.g. a profile name).
pub trait SecretStore: Send + Sync {
    /// Human-readable name, e.g. "OS credential store".
    fn name(&self) -> &str;
    fn get(&self, service: &str, account: &str) -> Result<Option<Secret>>;
    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<()>;
    /// `true` if something was deleted.
    fn delete(&self, service: &str, account: &str) -> Result<bool>;
}

/// Asks the person at the keyboard. Provided by the front-end (terminal,
/// dialog); absent in non-interactive runs.
pub trait Prompter: Send + Sync {
    fn secret(&self, prompt: &str) -> Option<Secret>;
}

/// The OS credential store: macOS Keychain, Windows Credential Manager,
/// Secret Service on Linux. Calls block; run them off the UI thread.
pub struct KeyringStore;

const KEYRING: &str = "OS credential store";

fn keyring_err(e: keyring::Error) -> SecretError {
    let kind = match e {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            SecretErrorKind::Unavailable
        }
        keyring::Error::BadEncoding(_) => SecretErrorKind::Format,
        _ => SecretErrorKind::Io,
    };
    SecretError::new(kind, KEYRING, e.to_string())
}

impl SecretStore for KeyringStore {
    fn name(&self) -> &str {
        KEYRING
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Secret>> {
        let entry = keyring::Entry::new(service, account).map_err(keyring_err)?;
        match entry.get_password() {
            Ok(p) => Ok(Some(Secret::new(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<()> {
        keyring::Entry::new(service, account)
            .and_then(|e| e.set_password(secret.expose()))
            .map_err(keyring_err)
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool> {
        let entry = keyring::Entry::new(service, account).map_err(keyring_err)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(keyring_err(e)),
        }
    }
}

/// Memory only: secrets are forgotten when the process exits.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<(String, String), Secret>>);

impl SecretStore for MemoryStore {
    fn name(&self) -> &str {
        "memory (not saved)"
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Secret>> {
        let map = self.0.lock().expect("poisoned");
        Ok(map.get(&(service.to_owned(), account.to_owned())).cloned())
    }

    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<()> {
        let mut map = self.0.lock().expect("poisoned");
        map.insert((service.to_owned(), account.to_owned()), secret.clone());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool> {
        let mut map = self.0.lock().expect("poisoned");
        Ok(map
            .remove(&(service.to_owned(), account.to_owned()))
            .is_some())
    }
}

/// Reads environment variables; replaceable in tests.
pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The process environment; empty values count as unset.
pub fn process_env() -> EnvLookup {
    Arc::new(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        assert_eq!(format!("{:?}", Secret::new("hunter2")), "<redacted>");
        assert_eq!(format!("{:?}", Secret::default()), "<empty>");
    }

    #[test]
    fn memory_store() {
        let s = MemoryStore::default();
        assert!(s.get("svc", "a").unwrap().is_none());
        s.set("svc", "a", &Secret::new("x")).unwrap();
        assert_eq!(s.get("svc", "a").unwrap().unwrap().expose(), "x");
        assert!(s.get("other", "a").unwrap().is_none());
        assert!(s.delete("svc", "a").unwrap());
        assert!(!s.delete("svc", "a").unwrap());
    }

    /// Touches the real OS store: `cargo test -p secrets -- --ignored`.
    #[test]
    #[ignore]
    fn keyring_round_trip() {
        let s = KeyringStore;
        let account = format!("test-{}", std::process::id());
        s.set("pyoxide-test", &account, &Secret::new("x")).unwrap();
        assert_eq!(
            s.get("pyoxide-test", &account).unwrap().unwrap().expose(),
            "x"
        );
        assert!(s.delete("pyoxide-test", &account).unwrap());
    }
}
