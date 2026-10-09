//! The service object: where profiles live, which secret store to use, and
//! how to open a connection for a profile.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use db_core::{ConnectParams, Connection, Error, ErrorKind, Result, Secret};
use secrets::{EnvLookup, Prompter, SecretStore};
use serde::{Deserialize, Serialize};

use crate::profiles::{Profile, ProfilesFile, validate_name};
use crate::secrets::{SecretKind, SecretResolver, SecretSource};

/// What to connect to: a saved profile or an inline one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Profile(String),
    Connection(Box<Profile>),
}

/// Name used for inline (unsaved) connections; also the secret store account
/// they would use.
pub const INLINE_NAME: &str = "inline";

pub struct DbService {
    pub profiles_path: PathBuf,
    pub store: Arc<dyn SecretStore>,
    pub prompter: Option<Arc<dyn Prompter>>,
    pub env: EnvLookup,
}

/// Secrets resolved for a profile, with where they came from.
pub struct ResolvedSecrets {
    pub password: Secret,
    pub password_source: SecretSource,
    pub wallet_password: Secret,
    pub wallet_source: SecretSource,
    /// The secret store failed (and was skipped).
    pub store_error: Option<String>,
}

/// An open connection plus what was used to open it.
pub struct Opened {
    pub conn: Box<dyn Connection>,
    pub name: String,
    pub profile: Profile,
    pub password_source: SecretSource,
    pub connect_ms: f64,
}

pub fn millis(since: Instant) -> f64 {
    (since.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

impl DbService {
    pub fn new(
        profiles_path: PathBuf,
        store: Arc<dyn SecretStore>,
        prompter: Option<Arc<dyn Prompter>>,
        env: EnvLookup,
    ) -> Self {
        Self {
            profiles_path,
            store,
            prompter,
            env,
        }
    }

    pub fn profiles(&self) -> Result<ProfilesFile> {
        ProfilesFile::load(&self.profiles_path)
    }

    pub fn save_profile(&self, name: &str, profile: Profile) -> Result<()> {
        validate_name(name)?;
        profile.validate()?;
        let mut file = self.profiles()?;
        file.profiles.insert(name.to_owned(), profile);
        file.save(&self.profiles_path)
    }

    pub fn remove_profile(&self, name: &str) -> Result<bool> {
        let mut file = self.profiles()?;
        let removed = file.profiles.remove(name).is_some();
        if removed {
            file.save(&self.profiles_path)?;
        }
        Ok(removed)
    }

    /// Name and profile for a target.
    pub fn resolve_target(&self, target: &Target) -> Result<(String, Profile)> {
        match target {
            Target::Profile(name) => Ok((name.clone(), self.profiles()?.get(name)?.clone())),
            Target::Connection(p) => {
                p.validate()?;
                Ok((INLINE_NAME.to_owned(), (**p).clone()))
            }
        }
    }

    fn resolver(&self) -> SecretResolver<'_> {
        SecretResolver {
            store: self.store.as_ref(),
            prompter: self.prompter.as_deref(),
            env: &self.env,
        }
    }

    /// Look up the password and wallet password. May block (OS store,
    /// prompt): call off the UI thread.
    pub fn secrets_for(&self, name: &str, profile: &Profile) -> ResolvedSecrets {
        let r = self.resolver();
        let (password, password_source, e1) = r.resolve(name, profile, SecretKind::Password);
        let (wallet_password, wallet_source, e2) =
            r.resolve(name, profile, SecretKind::WalletPassword);
        ResolvedSecrets {
            password,
            password_source,
            wallet_password,
            wallet_source,
            store_error: e1.or(e2).map(|e| e.to_string()),
        }
    }

    /// Connection parameters for a profile, failing early when a needed
    /// password is missing.
    pub fn params_for(
        &self,
        name: &str,
        profile: &Profile,
    ) -> Result<(ConnectParams, SecretSource)> {
        let s = self.secrets_for(name, profile);
        if s.password_source == SecretSource::Missing {
            let mut msg = format!(
                "no password for profile {name:?}: set {} or the profile's password_env, or run `pyoxide-db secret set {name}`",
                crate::secrets::PASSWORD_ENV
            );
            if let Some(e) = s.store_error {
                msg.push_str(&format!(" ({e})"));
            }
            return Err(Error::new(ErrorKind::Config, msg));
        }
        let params = profile.to_params(s.password, s.wallet_password)?;
        Ok((params, s.password_source))
    }

    pub async fn open(&self, target: &Target) -> Result<Opened> {
        let (name, profile) = self.resolve_target(target)?;
        let (params, password_source) = self.params_for(&name, &profile)?;
        let driver = crate::driver(profile.backend);
        let started = Instant::now();
        let conn = driver.connect(&params).await?;
        Ok(Opened {
            conn,
            name,
            profile,
            password_source,
            connect_ms: millis(started),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_core::Backend;
    use secrets::MemoryStore;

    fn service(dir: &std::path::Path) -> DbService {
        DbService::new(
            dir.join("db-profiles.toml"),
            Arc::new(MemoryStore::default()),
            None,
            Arc::new(|_| None),
        )
    }

    #[test]
    fn save_resolve_remove() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path());
        let mut p = Profile::new(Backend::Sqlite);
        p.path = Some(":memory:".into());
        svc.save_profile("scratch", p.clone()).unwrap();
        let (name, got) = svc
            .resolve_target(&Target::Profile("scratch".into()))
            .unwrap();
        assert_eq!((name.as_str(), &got), ("scratch", &p));
        assert!(svc.remove_profile("scratch").unwrap());
        assert!(
            svc.resolve_target(&Target::Profile("scratch".into()))
                .is_err()
        );
    }

    #[test]
    fn missing_password_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path());
        let mut p = Profile::new(Backend::Postgres);
        p.host = "localhost".into();
        p.database = Some("d".into());
        p.user = "u".into();
        let e = svc.params_for("pg", &p).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Config);
        assert!(e.message.contains("secret set pg"));
    }

    #[test]
    fn target_json_shape() {
        let t: Target = serde_json::from_str(r#"{"profile": "prod"}"#).unwrap();
        assert_eq!(t, Target::Profile("prod".into()));
        let t: Target =
            serde_json::from_str(r#"{"connection": {"backend": "sqlite", "path": ":memory:"}}"#)
                .unwrap();
        assert!(matches!(t, Target::Connection(_)));
    }
}
