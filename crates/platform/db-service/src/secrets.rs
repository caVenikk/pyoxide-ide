//! Password lookup for connection profiles: environment, secret store,
//! interactive prompt (ADR 0004). Values only travel as [`Secret`].

use secrets::{EnvLookup, Prompter, Secret, SecretError, SecretStore};
use serde::{Deserialize, Serialize};

use crate::profiles::Profile;

/// Generic environment variable for the connection password.
pub const PASSWORD_ENV: &str = "PYOXIDE_DB_PASSWORD";
/// Generic environment variable for an Oracle wallet password.
pub const WALLET_PASSWORD_ENV: &str = "PYOXIDE_DB_WALLET_PASSWORD";
/// Service under which database secrets are stored in a secret store.
pub const SECRET_SERVICE: &str = "pyoxide-db";

/// Which secret of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    Password,
    WalletPassword,
}

impl SecretKind {
    /// Account name in the credential store.
    pub fn account(self, profile: &str) -> String {
        match self {
            SecretKind::Password => profile.to_owned(),
            SecretKind::WalletPassword => format!("{profile}/wallet"),
        }
    }
}

/// Where a secret came from. Reported by diagnostics; never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", content = "detail", rename_all = "snake_case")]
pub enum SecretSource {
    /// An environment variable (its name).
    Env(String),
    /// The OS credential store (its name).
    Store(String),
    Prompt,
    /// Not needed for this profile.
    NotNeeded,
    Missing,
}

impl std::fmt::Display for SecretSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretSource::Env(name) => write!(f, "environment variable {name}"),
            SecretSource::Store(name) => write!(f, "{name}"),
            SecretSource::Prompt => f.write_str("interactive prompt"),
            SecretSource::NotNeeded => f.write_str("not needed"),
            SecretSource::Missing => f.write_str("missing"),
        }
    }
}

/// Everything needed to find a profile's secrets.
pub struct SecretResolver<'a> {
    pub store: &'a dyn SecretStore,
    pub prompter: Option<&'a dyn Prompter>,
    pub env: &'a EnvLookup,
}

impl SecretResolver<'_> {
    /// Find a secret: profile-specific env var, generic env var, secret
    /// store, prompt. A failing store is skipped (and reported by
    /// diagnostics) so environment and prompt still work on headless machines.
    pub fn resolve(
        &self,
        name: &str,
        profile: &Profile,
        kind: SecretKind,
    ) -> (Secret, SecretSource, Option<SecretError>) {
        let (specific, generic) = match kind {
            SecretKind::Password => {
                if !profile.needs_password() {
                    return (Secret::default(), SecretSource::NotNeeded, None);
                }
                (profile.password_env.clone(), PASSWORD_ENV)
            }
            SecretKind::WalletPassword => {
                let tls = profile.tls.as_ref();
                if tls.and_then(|t| t.wallet_dir.as_ref()).is_none() {
                    return (Secret::default(), SecretSource::NotNeeded, None);
                }
                (
                    tls.and_then(|t| t.wallet_password_env.clone()),
                    WALLET_PASSWORD_ENV,
                )
            }
        };
        for var in specific.iter().map(String::as_str).chain([generic]) {
            if let Some(v) = (self.env)(var) {
                return (Secret::new(v), SecretSource::Env(var.to_owned()), None);
            }
        }
        let mut store_error = None;
        match self.store.get(SECRET_SERVICE, &kind.account(name)) {
            Ok(Some(s)) => return (s, SecretSource::Store(self.store.name().to_owned()), None),
            Ok(None) => {}
            Err(e) => store_error = Some(e),
        }
        if let Some(p) = self.prompter {
            let what = match kind {
                SecretKind::Password => format!("Password for {} ({name}): ", profile.user),
                SecretKind::WalletPassword => format!("Wallet password ({name}): "),
            };
            if let Some(s) = p.secret(&what) {
                return (s, SecretSource::Prompt, store_error);
            }
        }
        // An Oracle wallet without a password (auto-login) is valid.
        (Secret::default(), SecretSource::Missing, store_error)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use db_core::Backend;
    use secrets::MemoryStore;

    fn pg() -> Profile {
        let mut p = Profile::new(Backend::Postgres);
        p.host = "h".into();
        p.database = Some("d".into());
        p.user = "u".into();
        p.password_env = Some("MY_PG_PASS".into());
        p
    }

    fn env(vars: &[(&str, &str)]) -> EnvLookup {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Arc::new(move |k| map.get(k).cloned())
    }

    struct FixedPrompt;
    impl Prompter for FixedPrompt {
        fn secret(&self, _: &str) -> Option<Secret> {
            Some(Secret::new("typed"))
        }
    }

    #[test]
    fn lookup_order() {
        let store = MemoryStore::default();
        let profile = pg();
        let e = env(&[("MY_PG_PASS", "specific"), (PASSWORD_ENV, "generic")]);
        let r = SecretResolver {
            store: &store,
            prompter: None,
            env: &e,
        };
        let (s, src, _) = r.resolve("p", &profile, SecretKind::Password);
        assert_eq!(s.expose(), "specific");
        assert_eq!(src, SecretSource::Env("MY_PG_PASS".into()));

        let e = env(&[(PASSWORD_ENV, "generic")]);
        let r = SecretResolver {
            store: &store,
            prompter: None,
            env: &e,
        };
        assert_eq!(
            r.resolve("p", &profile, SecretKind::Password).0.expose(),
            "generic"
        );

        let e = env(&[]);
        store
            .set(SECRET_SERVICE, "p", &Secret::new("stored"))
            .unwrap();
        let r = SecretResolver {
            store: &store,
            prompter: Some(&FixedPrompt),
            env: &e,
        };
        let (s, src, _) = r.resolve("p", &profile, SecretKind::Password);
        assert_eq!(s.expose(), "stored");
        assert_eq!(src, SecretSource::Store(store.name().into()));

        store.delete(SECRET_SERVICE, "p").unwrap();
        let (s, src, _) = r.resolve("p", &profile, SecretKind::Password);
        assert_eq!((s.expose(), src), ("typed", SecretSource::Prompt));

        let r = SecretResolver {
            store: &store,
            prompter: None,
            env: &e,
        };
        assert_eq!(
            r.resolve("p", &profile, SecretKind::Password).1,
            SecretSource::Missing
        );
    }

    #[test]
    fn sqlite_and_wallet_without_dir_need_nothing() {
        let store = MemoryStore::default();
        let e = env(&[]);
        let r = SecretResolver {
            store: &store,
            prompter: None,
            env: &e,
        };
        let mut lite = Profile::new(Backend::Sqlite);
        lite.path = Some(":memory:".into());
        assert_eq!(
            r.resolve("l", &lite, SecretKind::Password).1,
            SecretSource::NotNeeded
        );
        assert_eq!(
            r.resolve("p", &pg(), SecretKind::WalletPassword).1,
            SecretSource::NotNeeded
        );
    }

    #[test]
    fn wallet_account_names() {
        assert_eq!(SecretKind::Password.account("prod"), "prod");
        assert_eq!(SecretKind::WalletPassword.account("prod"), "prod/wallet");
    }
}
