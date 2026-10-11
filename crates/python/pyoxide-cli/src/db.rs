//! The database service behind the `db.*` commands, set up the way
//! `pyoxide-db` does it: profiles file, the secret store chosen in the
//! secret settings, a terminal prompt for passwords.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use db_service::DbService;
use db_service::profiles;
use secrets::config::{StoreKind, config_path};
use secrets::{
    EnvLookup, KeyringStore, LazyStore, Prompter, Secret, SecretError, SecretErrorKind, SecretStore,
    SecretsConfig,
};

/// Asks on the terminal without echo; `None` when stdin is not a terminal.
struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn secret(&self, prompt: &str) -> Option<Secret> {
        if !std::io::stdin().is_terminal() {
            return None;
        }
        rpassword::prompt_password(prompt).ok().map(Secret::new)
    }
}

pub fn service(profiles_path: Option<PathBuf>) -> DbService {
    let env = secrets::process_env();
    let prompter: Arc<dyn Prompter> = Arc::new(TerminalPrompter);
    DbService::new(
        profiles_path.unwrap_or_else(profiles::default_path),
        configured_store(Some(prompter.clone()), env.clone()),
        Some(prompter),
        env,
    )
}

/// The store chosen in the secret settings, opened on first use.
fn configured_store(prompter: Option<Arc<dyn Prompter>>, env: EnvLookup) -> Arc<dyn SecretStore> {
    let path = config_path(&env);
    let config = match SecretsConfig::load(&path) {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            return Arc::new(LazyStore::new("secret settings", move || {
                Err(SecretError::new(SecretErrorKind::Format, "secret settings", msg))
            }));
        }
    };
    let name = match config.store {
        StoreKind::Keychain => KeyringStore.name().to_owned(),
        StoreKind::Memory => "memory (not saved)".to_owned(),
        StoreKind::KeePass => format!(
            "KeePass database {}",
            config.keepass.as_ref().map(|k| k.path.as_str()).unwrap_or("?")
        ),
    };
    Arc::new(LazyStore::new(name, move || {
        secrets::open_store(&config, prompter.as_deref(), &env)
    }))
}
