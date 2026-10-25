//! The database module (ADR 0021, 0022) in the product: the store its
//! views keep data sources with, made from `db-service`. A project's data
//! sources are the ADR 0004 profiles in `.oxide/db-profiles.toml` (the
//! format of the `pyoxide-db` command line, which reads the same file with
//! `--profiles`); passwords live in the configured secret store ("Save:
//! Forever"), in memory until the IDE quits ("Until restart") or nowhere
//! ("Never"), never in a file.
//!
//! Connecting runs on a small runtime of its own; dropping the module's
//! future aborts it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use db_core::schema::SchemaInfo;
use db_core::{Dialect, Error, ErrorKind, PasswordSave, Secret};
use db_service::DbService;
use db_service::profiles::ProfilesFile;
use db_service::secrets::{SECRET_SERVICE, SecretKind};
use db_ui::store::{
    BoxFuture, DataSource, DataSourceStore, Failure, ImportSummary, PasswordChange, SaveRequest,
    ServerInfo,
};
use secrets::{EnvLookup, MemoryStore, SecretStore};

/// Where a project keeps its data sources.
pub const PROFILES_FILE: &str = "db-profiles.toml";

/// The profiles file of the project at `root`.
pub fn profiles_path(root: &Path) -> PathBuf {
    root.join(".oxide").join(PROFILES_FILE)
}

/// The database module of the product.
pub fn module() -> workbench::Module {
    db_ui::module(db_ui::DbUiConfig {
        store: Arc::new(ProjectStore::configured()),
    })
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pyoxide-db")
            .enable_all()
            .build()
            .expect("the database runtime starts")
    })
}

/// Runs `work` on the runtime; dropping the future aborts it.
fn spawn<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static) -> BoxFuture<T> {
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let handle = runtime().spawn(work);
    let guard = AbortOnDrop(handle.abort_handle());
    Box::pin(async move {
        let _guard = guard;
        handle.await.expect("database work does not panic")
    })
}

/// A secret store that finds a password in memory first ("Until
/// restart"), then in the configured store.
struct Layered {
    memory: Arc<MemoryStore>,
    store: Arc<dyn SecretStore>,
}

impl SecretStore for Layered {
    fn name(&self) -> &str {
        self.store.name()
    }

    fn get(&self, service: &str, account: &str) -> secrets::Result<Option<Secret>> {
        match self.memory.get(service, account)? {
            Some(s) => Ok(Some(s)),
            None => self.store.get(service, account),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &Secret) -> secrets::Result<()> {
        self.store.set(service, account, secret)
    }

    fn delete(&self, service: &str, account: &str) -> secrets::Result<bool> {
        let a = self.memory.delete(service, account)?;
        let b = self.store.delete(service, account)?;
        Ok(a || b)
    }
}

/// The product's store: data sources per project, the configured secret
/// store and the passwords kept until restart.
pub struct ProjectStore {
    store: Arc<dyn SecretStore>,
    memory: Arc<MemoryStore>,
    env: EnvLookup,
}

impl ProjectStore {
    /// The store with the secret store of the secret settings.
    pub fn configured() -> Self {
        let env = secrets::process_env();
        Self::new(secrets::config::configured_store(None, env.clone()), env)
    }

    pub fn new(store: Arc<dyn SecretStore>, env: EnvLookup) -> Self {
        Self {
            store,
            memory: Arc::new(MemoryStore::default()),
            env,
        }
    }

    fn service(&self, project: &Path) -> DbService {
        DbService::new(
            profiles_path(project),
            Arc::new(Layered {
                memory: self.memory.clone(),
                store: self.store.clone(),
            }),
            None,
            self.env.clone(),
        )
    }

    fn move_secret(&self, from: &str, to: &str) -> Result<(), String> {
        for kind in [SecretKind::Password, SecretKind::WalletPassword] {
            let (from, to) = (kind.account(from), kind.account(to));
            for store in [
                self.memory.clone() as Arc<dyn SecretStore>,
                self.store.clone(),
            ] {
                if let Some(secret) = store.get(SECRET_SERVICE, &from).map_err(text)? {
                    store.set(SECRET_SERVICE, &to, &secret).map_err(text)?;
                    store.delete(SECRET_SERVICE, &from).map_err(text)?;
                }
            }
        }
        Ok(())
    }

    fn delete_secrets(&self, name: &str) -> Result<(), String> {
        for kind in [SecretKind::Password, SecretKind::WalletPassword] {
            let account = kind.account(name);
            self.memory.delete(SECRET_SERVICE, &account).map_err(text)?;
            self.store.delete(SECRET_SERVICE, &account).map_err(text)?;
        }
        Ok(())
    }
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The database product a backend talks to.
fn product(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres => "PostgreSQL",
        Dialect::Oracle => "Oracle",
        Dialect::Sqlite => "SQLite",
    }
}

/// The version number in a server's version text: `18.6` of PostgreSQL's
/// `PostgreSQL 18.6 on aarch64-…`, `23.0.0.0.0` of Oracle's `… Release
/// 23.0.0.0.0 - …`, SQLite's as it is.
fn short_version(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let release = words
        .iter()
        .position(|w| w.eq_ignore_ascii_case("release"))
        .and_then(|i| words.get(i + 1));
    release
        .or_else(|| {
            words
                .iter()
                .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(|w| w.trim_end_matches([',', ';']).to_owned())
        .unwrap_or_else(|| text.to_owned())
}

fn failure(dialect: Dialect, e: Error) -> Failure {
    let hint = db_service::hints::for_error(dialect, &e);
    Failure {
        auth: e.kind == ErrorKind::Auth,
        message: e.message,
        code: e.code,
        hint: hint.as_ref().map(|h| h.cause.clone()),
        next: hint.map(|h| h.next),
    }
}

impl DataSourceStore for ProjectStore {
    fn load(&self, project: &Path) -> Result<Vec<DataSource>, String> {
        let file = ProfilesFile::load(&profiles_path(project)).map_err(|e| e.message)?;
        Ok(file
            .profiles
            .into_iter()
            .map(|(name, profile)| DataSource { name, profile })
            .collect())
    }

    fn save(&self, project: &Path, request: SaveRequest) -> Result<(), String> {
        let mut file = ProfilesFile::default();
        for source in &request.sources {
            db_service::profiles::validate_name(&source.name).map_err(|e| e.message)?;
            file.profiles
                .insert(source.name.clone(), source.profile.clone());
        }
        file.save(&profiles_path(project)).map_err(|e| e.message)?;
        for (old, new) in &request.renamed {
            self.move_secret(old, new)?;
        }
        for name in &request.removed {
            self.delete_secrets(name)?;
        }
        for (name, change) in &request.passwords {
            let account = SecretKind::Password.account(name);
            let save = request
                .sources
                .iter()
                .find(|s| &s.name == name)
                .map(|s| s.profile.gui_options().password_save)
                .unwrap_or_default();
            match change {
                PasswordChange::Keep if save == PasswordSave::Never => {
                    self.memory.delete(SECRET_SERVICE, &account).map_err(text)?;
                    self.store.delete(SECRET_SERVICE, &account).map_err(text)?;
                }
                PasswordChange::Keep => {}
                PasswordChange::Set(secret) => match save {
                    PasswordSave::Forever => {
                        self.store
                            .set(SECRET_SERVICE, &account, secret)
                            .map_err(text)?;
                        self.memory.delete(SECRET_SERVICE, &account).map_err(text)?;
                    }
                    PasswordSave::UntilRestart => {
                        self.memory
                            .set(SECRET_SERVICE, &account, secret)
                            .map_err(text)?;
                        self.store.delete(SECRET_SERVICE, &account).map_err(text)?;
                    }
                    PasswordSave::Never => {
                        self.memory.delete(SECRET_SERVICE, &account).map_err(text)?;
                        self.store.delete(SECRET_SERVICE, &account).map_err(text)?;
                    }
                },
                PasswordChange::Clear => {
                    self.memory.delete(SECRET_SERVICE, &account).map_err(text)?;
                    self.store.delete(SECRET_SERVICE, &account).map_err(text)?;
                }
            }
        }
        Ok(())
    }

    fn has_password(&self, _: &Path, name: &str) -> bool {
        let account = SecretKind::Password.account(name);
        matches!(self.memory.get(SECRET_SERVICE, &account), Ok(Some(_)))
            || matches!(self.store.get(SECRET_SERVICE, &account), Ok(Some(_)))
    }

    fn test(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<ServerInfo, Failure>> {
        let svc = self.service(project);
        spawn(async move {
            let dialect = source.profile.dialect();
            db_service::ops::test_profile(&svc, &source.name, &source.profile, password)
                .await
                .map(|r| ServerInfo {
                    product: product(dialect).to_owned(),
                    version: short_version(&r.server_version),
                    driver: r.client,
                    connect_ms: r.connect_ms,
                    ping_ms: r.ping_ms,
                })
                .map_err(|e| failure(dialect, e))
        })
    }

    fn schemas(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<Vec<SchemaInfo>, Failure>> {
        let svc = self.service(project);
        spawn(async move {
            let dialect = source.profile.dialect();
            db_service::ops::profile_schemas(&svc, &source.name, &source.profile, password)
                .await
                .map_err(|e| failure(dialect, e))
        })
    }

    fn databases(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<Vec<String>, Failure>> {
        let svc = self.service(project);
        spawn(async move {
            let dialect = source.profile.dialect();
            db_service::ops::profile_databases(&svc, &source.name, &source.profile, password)
                .await
                .map_err(|e| failure(dialect, e))
        })
    }

    fn import_pycharm(&self, project: &Path) -> Result<ImportSummary, String> {
        let svc = self.service(project);
        let args = db_service::import::PycharmImportArgs {
            project: project.to_owned(),
            no_passwords: true,
            ..Default::default()
        };
        let report = db_service::import::import_pycharm(&svc, &args).map_err(|e| e.message)?;
        let mut summary = ImportSummary::default();
        for entry in report.entries {
            match entry.status {
                db_service::import::EntryStatus::Created
                | db_service::import::EntryStatus::Replaced => summary.imported.push(entry.profile),
                _ => summary.skipped.push((
                    entry.data_source,
                    if entry.notes.is_empty() {
                        "exists already".to_owned()
                    } else {
                        entry.notes.join("; ")
                    },
                )),
            }
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_core::{Backend, GuiOptions, Profile};

    fn store() -> (ProjectStore, Arc<MemoryStore>) {
        let os = Arc::new(MemoryStore::default());
        (ProjectStore::new(os.clone(), Arc::new(|_| None)), os)
    }

    fn pg(save: PasswordSave) -> Profile {
        let mut p = Profile::new(Backend::Postgres);
        p.host = "localhost".into();
        p.database = Some("pyoxide".into());
        p.user = "pyoxide".into();
        p.gui = Some(GuiOptions {
            password_save: save,
            ..GuiOptions::default()
        });
        p
    }

    fn os_password(os: &MemoryStore, name: &str) -> Option<String> {
        os.get(SECRET_SERVICE, name)
            .unwrap()
            .map(|s| s.expose().to_owned())
    }

    /// Profiles go to `.oxide/db-profiles.toml` without the password; the
    /// password goes where its mode says; renames move it and removals
    /// delete it.
    #[test]
    fn passwords_never_reach_the_file_and_follow_the_data_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (store, os) = store();
        let secret = "s3cret-value";
        let request = SaveRequest {
            sources: vec![DataSource {
                name: "local".into(),
                profile: pg(PasswordSave::Forever),
            }],
            passwords: vec![("local".into(), PasswordChange::Set(Secret::new(secret)))],
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        let text = std::fs::read_to_string(profiles_path(root)).unwrap();
        assert!(text.contains("[profiles.local]"), "{text}");
        assert!(!text.contains(secret), "{text}");
        assert_eq!(os_password(&os, "local").as_deref(), Some(secret));
        assert!(store.has_password(root, "local"));
        assert_eq!(store.load(root).unwrap()[0].name, "local");

        // Renamed: the password moves.
        let request = SaveRequest {
            sources: vec![DataSource {
                name: "renamed".into(),
                profile: pg(PasswordSave::Forever),
            }],
            renamed: vec![("local".into(), "renamed".into())],
            passwords: vec![("renamed".into(), PasswordChange::Keep)],
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        assert_eq!(os_password(&os, "local"), None);
        assert_eq!(os_password(&os, "renamed").as_deref(), Some(secret));

        // Until restart: in memory only, found by the service.
        let request = SaveRequest {
            sources: vec![DataSource {
                name: "renamed".into(),
                profile: pg(PasswordSave::UntilRestart),
            }],
            passwords: vec![("renamed".into(), PasswordChange::Set(Secret::new("other")))],
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        assert_eq!(os_password(&os, "renamed"), None);
        assert!(store.has_password(root, "renamed"));
        let svc = store.service(root);
        let found = svc.secrets_for("renamed", &pg(PasswordSave::UntilRestart));
        assert_eq!(found.password.expose(), "other");

        // Removed: its secrets go.
        let request = SaveRequest {
            removed: vec!["renamed".into()],
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        assert!(!store.has_password(root, "renamed"));
        assert!(store.load(root).unwrap().is_empty());
        let text = std::fs::read_to_string(profiles_path(root)).unwrap();
        assert!(!text.contains("other") && !text.contains(secret));
    }

    /// "Never": a typed password is not kept, and one kept before goes.
    #[test]
    fn never_keeps_no_password() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (store, os) = store();
        os.set(SECRET_SERVICE, "local", &Secret::new("old"))
            .unwrap();
        let request = SaveRequest {
            sources: vec![DataSource {
                name: "local".into(),
                profile: pg(PasswordSave::Never),
            }],
            passwords: vec![("local".into(), PasswordChange::Set(Secret::new("typed")))],
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        assert!(!store.has_password(root, "local"));
    }

    /// A test of a SQLite file that does not exist reports it and creates
    /// nothing; one that exists connects.
    #[test]
    fn sqlite_tests_report_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (store, _) = store();
        let missing = root.join("missing.db");
        let mut p = Profile::new(Backend::Sqlite);
        p.path = Some(missing.display().to_string());
        let source = DataSource {
            name: "file".into(),
            profile: p.clone(),
        };
        let result = futures_block_on(store.test(root, source, None));
        assert!(result.is_err(), "{result:?}");
        assert!(!missing.exists());

        let present = root.join("present.db");
        std::fs::write(&present, b"").unwrap();
        p.path = Some(present.display().to_string());
        let source = DataSource {
            name: "file".into(),
            profile: p,
        };
        let info = futures_block_on(store.test(root, source, None)).unwrap();
        assert_eq!(info.product, "SQLite");
    }

    #[test]
    fn version_numbers() {
        assert_eq!(
            short_version("PostgreSQL 18.6 on aarch64-unknown-linux-musl, compiled by gcc"),
            "18.6"
        );
        assert_eq!(
            short_version("Oracle Database 23ai Free Release 23.0.0.0.0 - Develop, Learn"),
            "23.0.0.0.0"
        );
        assert_eq!(short_version("3.46.1"), "3.46.1");
        assert_eq!(short_version("unknown"), "unknown");
    }

    fn futures_block_on<T>(f: BoxFuture<T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }
}
