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
use db_core::{
    Connection, ConnectionSource, DataSourceId, Dialect, Error, ErrorKind, PasswordSave, Profile,
    Purpose, Secret,
};
use db_model::ModelCache;
use db_service::DbService;
use db_service::profiles::ProfilesFile;
use db_service::secrets::{SecretKind, password_env, secret_service};
use db_ui::store::{
    BoxFuture, DataSource, DataSourceStore, Failure, ImportSummary, PasswordChange, SaveRequest,
    ServerInfo,
};
use secrets::{EnvLookup, MemoryStore, SecretStore};

/// Where a project keeps its data sources.
pub const PROFILES_FILE: &str = "db-profiles.toml";

/// The profiles file of the project at `root`.
pub fn profiles_path(root: &Path) -> PathBuf {
    root.join(brand::PROJECT_DIR).join(PROFILES_FILE)
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
            .thread_name(format!("{}-db", brand::SLUG))
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

/// The folder of a project's models in the user cache: the project
/// folder's name and a hash of its path (data source names repeat between
/// projects).
fn project_key(project: &Path) -> String {
    // FNV-1a: stable between runs and platforms, unlike `DefaultHasher`.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project.to_string_lossy().bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    let name: String = project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!("{name}-{hash:016x}")
}

/// A secret store that finds a password in memory first ("Until
/// restart"), then, for a data source that saves "Forever", in the
/// configured store. For any other data source the configured store is
/// never asked: the OS may prompt for the login password on every access.
struct Layered {
    memory: Arc<MemoryStore>,
    store: Arc<dyn SecretStore>,
    use_store: bool,
}

impl SecretStore for Layered {
    fn name(&self) -> &str {
        if self.use_store {
            self.store.name()
        } else {
            self.memory.name()
        }
    }

    fn get(&self, service: &str, account: &str) -> secrets::Result<Option<Secret>> {
        match self.memory.get(service, account)? {
            Some(s) => Ok(Some(s)),
            None if self.use_store => self.store.get(service, account),
            None => Ok(None),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &Secret) -> secrets::Result<()> {
        if self.use_store {
            self.store.set(service, account, secret)
        } else {
            self.memory.set(service, account, secret)
        }
    }

    fn delete(&self, service: &str, account: &str) -> secrets::Result<bool> {
        let a = self.memory.delete(service, account)?;
        let b = self.use_store && self.store.delete(service, account)?;
        Ok(a || b)
    }
}

/// Whether a data source's passwords may be in the configured store.
fn keeps_in_store(save: PasswordSave) -> bool {
    save == PasswordSave::Forever
}

/// What makes a [`DbService`]: the secret stores and the environment.
#[derive(Clone)]
struct Parts {
    memory: Arc<MemoryStore>,
    store: Arc<dyn SecretStore>,
    env: EnvLookup,
}

impl Parts {
    fn service(&self, project: &Path, use_store: bool) -> DbService {
        DbService::new(
            profiles_path(project),
            Arc::new(Layered {
                memory: self.memory.clone(),
                store: self.store.clone(),
                use_store,
            }),
            None,
            self.env.clone(),
        )
    }
}

/// The connections of a project's data sources (the tree's sessions): each
/// opens with a service that asks the configured store only when that data
/// source saves its password "Forever".
struct ProjectConnections {
    parts: Parts,
    project: PathBuf,
}

#[async_trait::async_trait]
impl ConnectionSource for ProjectConnections {
    async fn open(
        &self,
        data_source: &DataSourceId,
        purpose: Purpose,
    ) -> db_core::Result<Arc<dyn Connection>> {
        let use_store = ProfilesFile::load(&profiles_path(&self.project))
            .ok()
            .and_then(|f| {
                f.profiles
                    .get(data_source.as_str())
                    .map(|p| keeps_in_store(p.gui_options().password_save))
            })
            .unwrap_or(false);
        let service = self.parts.service(&self.project, use_store);
        ConnectionSource::open(&service, data_source, purpose).await
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
    /// The store with the secret store of the secret settings (the shared
    /// one: the OS keychain is read once per secret and run).
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

    /// The service for one data source: it asks the configured store only
    /// when the data source saves its password "Forever".
    fn service(&self, project: &Path, profile: &Profile) -> DbService {
        self.service_with(project, keeps_in_store(profile.gui_options().password_save))
    }

    fn service_with(&self, project: &Path, use_store: bool) -> DbService {
        self.parts().service(project, use_store)
    }

    fn parts(&self) -> Parts {
        Parts {
            memory: self.memory.clone(),
            store: self.store.clone(),
            env: self.env.clone(),
        }
    }

    /// Moves the secrets of a renamed data source; the configured store is
    /// touched only when the data source saved "Forever" before.
    fn move_secret(&self, from: &str, to: &str, in_store: bool) -> Result<(), String> {
        for kind in [SecretKind::Password, SecretKind::WalletPassword] {
            let (from, to) = (kind.account(from), kind.account(to));
            let mut stores = vec![self.memory.clone() as Arc<dyn SecretStore>];
            if in_store {
                stores.push(self.store.clone());
            }
            for store in stores {
                if let Some(secret) = store.get(&secret_service(), &from).map_err(text)? {
                    store.set(&secret_service(), &to, &secret).map_err(text)?;
                    store.delete(&secret_service(), &from).map_err(text)?;
                }
            }
        }
        Ok(())
    }

    /// Deletes the secrets of a data source; the configured store is
    /// touched only when `in_store`.
    fn delete_secrets(&self, name: &str, in_store: bool) -> Result<(), String> {
        for kind in [SecretKind::Password, SecretKind::WalletPassword] {
            let account = kind.account(name);
            self.memory
                .delete(&secret_service(), &account)
                .map_err(text)?;
            if in_store {
                self.store
                    .delete(&secret_service(), &account)
                    .map_err(text)?;
            }
        }
        Ok(())
    }

    /// Applies one data source's password change. `before` is how the data
    /// source saved before (none when it is new), `now` how it saves now;
    /// the configured store is touched only when one of them is "Forever".
    fn change_password(
        &self,
        name: &str,
        before: Option<PasswordSave>,
        now: PasswordSave,
        change: &PasswordChange,
    ) -> Result<(), String> {
        let account = SecretKind::Password.account(name);
        let was_stored = before.is_some_and(keeps_in_store);
        let memory = &self.memory;
        let store = &self.store;
        match change {
            PasswordChange::Set(secret) => match now {
                PasswordSave::Forever => {
                    store
                        .set(&secret_service(), &account, secret)
                        .map_err(text)?;
                    memory.delete(&secret_service(), &account).map_err(text)?;
                }
                PasswordSave::UntilRestart => {
                    memory
                        .set(&secret_service(), &account, secret)
                        .map_err(text)?;
                    if was_stored {
                        store.delete(&secret_service(), &account).map_err(text)?;
                    }
                }
                PasswordSave::Never => {
                    memory.delete(&secret_service(), &account).map_err(text)?;
                    if was_stored {
                        store.delete(&secret_service(), &account).map_err(text)?;
                    }
                }
            },
            PasswordChange::Clear => {
                memory.delete(&secret_service(), &account).map_err(text)?;
                if was_stored || keeps_in_store(now) {
                    store.delete(&secret_service(), &account).map_err(text)?;
                }
            }
            // The password stays; it follows the data source when the way
            // it is saved changed.
            PasswordChange::Keep => match now {
                PasswordSave::Forever => {
                    if !was_stored
                        && let Some(secret) =
                            memory.get(&secret_service(), &account).map_err(text)?
                    {
                        store
                            .set(&secret_service(), &account, &secret)
                            .map_err(text)?;
                        memory.delete(&secret_service(), &account).map_err(text)?;
                    }
                }
                PasswordSave::UntilRestart => {
                    if was_stored
                        && let Some(secret) =
                            store.get(&secret_service(), &account).map_err(text)?
                    {
                        memory
                            .set(&secret_service(), &account, &secret)
                            .map_err(text)?;
                        store.delete(&secret_service(), &account).map_err(text)?;
                    }
                }
                PasswordSave::Never => {
                    memory.delete(&secret_service(), &account).map_err(text)?;
                    if was_stored {
                        store.delete(&secret_service(), &account).map_err(text)?;
                    }
                }
            },
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
        // How each data source saved its password before this save, by the
        // name it has now (a renamed one by its old name).
        let before = ProfilesFile::load(&profiles_path(project)).unwrap_or_default();
        let saved_as = |name: &str| {
            let old = request
                .renamed
                .iter()
                .find(|(_, new)| new == name)
                .map_or(name, |(old, _)| old.as_str());
            before
                .profiles
                .get(old)
                .map(|p| p.gui_options().password_save)
        };
        let mut file = ProfilesFile::default();
        for source in &request.sources {
            db_service::profiles::validate_name(&source.name).map_err(|e| e.message)?;
            file.profiles
                .insert(source.name.clone(), source.profile.clone());
        }
        file.save(&profiles_path(project)).map_err(|e| e.message)?;
        for (old, new) in &request.renamed {
            let in_store = saved_as(new).is_some_and(keeps_in_store);
            self.move_secret(old, new, in_store)?;
        }
        for name in &request.removed {
            let in_store = before
                .profiles
                .get(name)
                .is_some_and(|p| keeps_in_store(p.gui_options().password_save));
            self.delete_secrets(name, in_store)?;
        }
        for (name, change) in &request.passwords {
            let now = request
                .sources
                .iter()
                .find(|s| &s.name == name)
                .map(|s| s.profile.gui_options().password_save)
                .unwrap_or_default();
            self.change_password(name, saved_as(name), now, change)?;
        }
        Ok(())
    }

    fn has_password(&self, project: &Path, name: &str) -> bool {
        let account = SecretKind::Password.account(name);
        if matches!(self.memory.get(&secret_service(), &account), Ok(Some(_))) {
            return true;
        }
        // Only a data source that saves "Forever" can have its password in
        // the configured store; for any other the OS is not asked.
        let forever = ProfilesFile::load(&profiles_path(project))
            .ok()
            .and_then(|f| f.profiles.get(name).map(|p| p.gui_options().password_save))
            .is_some_and(keeps_in_store);
        forever && matches!(self.store.get(&secret_service(), &account), Ok(Some(_)))
    }

    fn needs_prompt(&self, project: &Path, source: &DataSource) -> bool {
        let profile = &source.profile;
        if !profile.needs_password() {
            return false;
        }
        // The environment is looked up before any store, as when connecting.
        let in_env = profile
            .password_env
            .iter()
            .map(String::as_str)
            .chain([password_env().as_str()])
            .any(|var| (self.env)(var).is_some());
        !in_env && !self.has_password(project, &source.name)
    }

    fn hold_password(&self, _: &Path, name: &str, password: Secret) {
        // A failing memory store is a store that holds nothing: the
        // connection then asks again.
        let _ = self.memory.set(
            &secret_service(),
            &SecretKind::Password.account(name),
            &password,
        );
    }

    fn release_password(&self, _: &Path, name: &str) {
        let _ = self
            .memory
            .delete(&secret_service(), &SecretKind::Password.account(name));
    }

    fn test(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<ServerInfo, Failure>> {
        let svc = self.service(project, &source.profile);
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
        let svc = self.service(project, &source.profile);
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
        let svc = self.service(project, &source.profile);
        spawn(async move {
            let dialect = source.profile.dialect();
            db_service::ops::profile_databases(&svc, &source.name, &source.profile, password)
                .await
                .map_err(|e| failure(dialect, e))
        })
    }

    fn connections(&self, project: &Path) -> Arc<dyn ConnectionSource> {
        Arc::new(ProjectConnections {
            parts: self.parts(),
            project: project.to_owned(),
        })
    }

    fn model_cache(&self, project: &Path) -> Option<ModelCache> {
        let user = ModelCache::user()?;
        Some(ModelCache::new(user.root().join(project_key(project))))
    }

    fn import_pycharm(&self, project: &Path) -> Result<ImportSummary, String> {
        // The import moves no passwords (`no_passwords`): the store stays
        // closed.
        let svc = self.service_with(project, false);
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        os.get(&secret_service(), name)
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
        let svc = store.service(root, &pg(PasswordSave::UntilRestart));
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
        let save = |mode, change| {
            let request = SaveRequest {
                sources: vec![DataSource {
                    name: "local".into(),
                    profile: pg(mode),
                }],
                passwords: vec![("local".into(), change)],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
        };
        save(
            PasswordSave::Forever,
            PasswordChange::Set(Secret::new("old")),
        );
        assert_eq!(os_password(&os, "local").as_deref(), Some("old"));
        save(
            PasswordSave::Never,
            PasswordChange::Set(Secret::new("typed")),
        );
        assert!(!store.has_password(root, "local"));
        assert_eq!(os_password(&os, "local"), None);
    }

    /// Changing how a data source saves moves its password with it: from
    /// the store to memory and back.
    #[test]
    fn a_changed_save_mode_moves_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (store, os) = store();
        let save = |mode, change| {
            let request = SaveRequest {
                sources: vec![DataSource {
                    name: "local".into(),
                    profile: pg(mode),
                }],
                passwords: vec![("local".into(), change)],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
        };
        save(
            PasswordSave::Forever,
            PasswordChange::Set(Secret::new("pw")),
        );
        save(PasswordSave::UntilRestart, PasswordChange::Keep);
        assert_eq!(os_password(&os, "local"), None);
        assert!(store.has_password(root, "local"));
        save(PasswordSave::Forever, PasswordChange::Keep);
        assert_eq!(os_password(&os, "local").as_deref(), Some("pw"));
        assert!(store.has_password(root, "local"));
    }

    /// A secret store that counts how often it is asked.
    #[derive(Clone, Default)]
    struct Counting {
        inner: Arc<MemoryStore>,
        calls: Arc<AtomicUsize>,
        gets: Arc<AtomicUsize>,
    }

    impl Counting {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn gets(&self) -> usize {
            self.gets.load(Ordering::SeqCst)
        }
    }

    impl SecretStore for Counting {
        fn name(&self) -> &str {
            self.inner.name()
        }

        fn get(&self, service: &str, account: &str) -> secrets::Result<Option<Secret>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.inner.get(service, account)
        }

        fn set(&self, service: &str, account: &str, secret: &Secret) -> secrets::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.set(service, account, secret)
        }

        fn delete(&self, service: &str, account: &str) -> secrets::Result<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.delete(service, account)
        }

        fn get_any_account(&self, service: &str) -> secrets::Result<Option<Secret>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.get_any_account(service)
        }
    }

    /// A data source nobody listens at: connecting fails at once, after the
    /// password was looked up.
    fn unreachable_source(name: &str, save: PasswordSave) -> DataSource {
        let mut profile = pg(save);
        profile.host = "127.0.0.1".into();
        profile.port = Some(1);
        DataSource {
            name: name.into(),
            profile,
        }
    }

    /// Opens the dialog (kept passwords), saves, tests, lists schemas and
    /// databases of the data source, with and without a typed password.
    fn use_data_source(store: &ProjectStore, root: &Path, source: &DataSource) {
        store.has_password(root, &source.name);
        for password in [None, Some(Secret::new("typed"))] {
            let _ = futures_block_on(store.test(root, source.clone(), password.clone()));
            let _ = futures_block_on(store.schemas(root, source.clone(), password.clone()));
            let _ = futures_block_on(store.databases(root, source.clone(), password));
        }
    }

    /// The OS store is asked for a data source that saves "Forever" only:
    /// "Until restart" and "Never" make no call to it, whatever the
    /// dialog does (a keychain prompt otherwise).
    #[test]
    fn until_restart_and_never_data_sources_never_ask_the_os_store() {
        for mode in [PasswordSave::UntilRestart, PasswordSave::Never] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let os = Counting::default();
            let store = ProjectStore::new(Arc::new(os.clone()), Arc::new(|_| None));
            let source = unreachable_source("local", mode);
            let request = SaveRequest {
                sources: vec![source.clone()],
                passwords: vec![("local".into(), PasswordChange::Set(Secret::new("pw")))],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
            use_data_source(&store, root, &source);
            // Saved again and edited, renamed, removed.
            let renamed = unreachable_source("renamed", mode);
            let request = SaveRequest {
                sources: vec![renamed.clone()],
                renamed: vec![("local".into(), "renamed".into())],
                passwords: vec![("renamed".into(), PasswordChange::Keep)],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
            let request = SaveRequest {
                sources: vec![renamed.clone()],
                passwords: vec![("renamed".into(), PasswordChange::Clear)],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
            let request = SaveRequest {
                removed: vec!["renamed".into()],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
            assert_eq!(os.calls(), 0, "{mode:?}");

            // After a restart (nothing in memory) it is the same.
            let fresh = ProjectStore::new(Arc::new(os.clone()), Arc::new(|_| None));
            let request = SaveRequest {
                sources: vec![source.clone()],
                passwords: vec![("local".into(), PasswordChange::Keep)],
                ..SaveRequest::default()
            };
            fresh.save(root, request).unwrap();
            assert!(!fresh.has_password(root, "local"));
            use_data_source(&fresh, root, &source);
            assert_eq!(os.calls(), 0, "{mode:?} after a restart");
        }
    }

    /// A data source that saves "Forever" reads the OS store once while
    /// the IDE runs: the dialog, a test and a connect share the read.
    #[test]
    fn a_forever_data_source_reads_the_os_store_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let os = Counting::default();
        os.inner
            .set(&secret_service(), "local", &Secret::new("pw"))
            .unwrap();
        let cached = Arc::new(secrets::CachedStore::new(Arc::new(os.clone())));
        let store = ProjectStore::new(cached, Arc::new(|_| None));
        let source = unreachable_source("local", PasswordSave::Forever);
        let request = SaveRequest {
            sources: vec![source.clone()],
            passwords: vec![("local".into(), PasswordChange::Keep)],
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        assert_eq!(os.calls(), 0, "saving writes and does not read");
        assert!(store.has_password(root, "local"));
        use_data_source(&store, root, &source);
        assert_eq!(os.gets(), 1);
        assert_eq!(os.calls(), 1);
    }

    /// The tree's connections follow the same rule: the OS store is asked
    /// for a data source that saves "Forever" only, once.
    #[test]
    fn the_trees_connections_ask_the_os_store_only_for_forever() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let os = Counting::default();
        os.inner
            .set(&secret_service(), "forever", &Secret::new("pw"))
            .unwrap();
        let cached = Arc::new(secrets::CachedStore::new(Arc::new(os.clone())));
        let store = ProjectStore::new(cached, Arc::new(|_| None));
        let sources = [
            unreachable_source("forever", PasswordSave::Forever),
            unreachable_source("restart", PasswordSave::UntilRestart),
            unreachable_source("never", PasswordSave::Never),
        ];
        let request = SaveRequest {
            sources: sources.to_vec(),
            ..SaveRequest::default()
        };
        store.save(root, request).unwrap();
        let connections = store.connections(root);
        let open = |name: &str| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(connections.open(&DataSourceId::new(name), Purpose::Meta))
        };
        for name in ["restart", "never", "unknown"] {
            assert!(open(name).is_err());
        }
        assert_eq!(os.calls(), 0);
        for _ in 0..3 {
            assert!(open("forever").is_err());
        }
        assert_eq!(os.gets(), 1);
        assert_eq!(os.calls(), 1);
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

    /// Code intelligence connects on its own only when no password has to
    /// be typed: the environment, a kept one or one held for this run is
    /// enough, and a lookup never reaches the OS store for a data source
    /// that does not save "Forever".
    #[test]
    fn a_prompt_is_needed_only_without_a_password_to_be_had_silently() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let os = Counting::default();
        let store = ProjectStore::new(Arc::new(os.clone()), Arc::new(|_| None));
        let save = |name: &str, profile: Profile| {
            let source = DataSource {
                name: name.into(),
                profile,
            };
            let mut sources = store.load(root).unwrap();
            sources.push(source.clone());
            let request = SaveRequest {
                sources,
                passwords: vec![(name.into(), PasswordChange::Keep)],
                ..SaveRequest::default()
            };
            store.save(root, request).unwrap();
            source
        };

        let never = save("never", pg(PasswordSave::Never));
        let until = save("until", pg(PasswordSave::UntilRestart));
        let forever = save("forever", pg(PasswordSave::Forever));
        let file = save("file", Profile::new(Backend::Sqlite));
        assert!(store.needs_prompt(root, &never));
        assert!(store.needs_prompt(root, &until));
        assert!(!store.needs_prompt(root, &file));
        // Nothing is kept for the Forever data source yet.
        assert!(store.needs_prompt(root, &forever));

        // One held serves the connect it was typed for, for any data source.
        store.hold_password(root, "never", Secret::new("typed"));
        store.hold_password(root, "until", Secret::new("typed"));
        assert!(!store.needs_prompt(root, &never));
        assert!(!store.needs_prompt(root, &until));

        // "Never" lets it go once that connect is done, so the next asks
        // again; "Until restart" keeps it for the run.
        store.release_password(root, "never");
        assert!(store.needs_prompt(root, &never));
        assert!(!store.needs_prompt(root, &until));
        assert_eq!(os.calls(), 0);

        // A kept one is, for the Forever data source.
        os.inner
            .set(&secret_service(), "forever", &Secret::new("kept"))
            .unwrap();
        assert!(!store.needs_prompt(root, &forever));

        // So is the environment, the generic variable or the profile's own.
        let var = password_env();
        let env: secrets::EnvLookup =
            Arc::new(move |name| (name == var).then(|| "from-env".to_owned()));
        let with_env = ProjectStore::new(Arc::new(Counting::default()), env);
        assert!(!with_env.needs_prompt(root, &until));
        let mut own = pg(PasswordSave::Never);
        own.password_env = Some("MY_OWN_PASSWORD".into());
        let source = DataSource {
            name: "own".into(),
            profile: own,
        };
        let env: secrets::EnvLookup =
            Arc::new(|name| (name == "MY_OWN_PASSWORD").then(|| "x".to_owned()));
        let with_own = ProjectStore::new(Arc::new(Counting::default()), env);
        assert!(!with_own.needs_prompt(root, &source));
        assert!(store.needs_prompt(root, &source));
    }

    /// A "Never" or "Until restart" data source does not read the OS store
    /// when asked whether it needs a prompt.
    #[test]
    fn asking_for_a_prompt_does_not_touch_the_os_store_of_other_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let os = Counting::default();
        let store = ProjectStore::new(Arc::new(os.clone()), Arc::new(|_| None));
        for (name, mode) in [
            ("never", PasswordSave::Never),
            ("until", PasswordSave::UntilRestart),
        ] {
            let source = DataSource {
                name: name.into(),
                profile: pg(mode),
            };
            store
                .save(
                    root,
                    SaveRequest {
                        sources: vec![source.clone()],
                        ..SaveRequest::default()
                    },
                )
                .unwrap();
            assert!(store.needs_prompt(root, &source));
        }
        assert_eq!(os.calls(), 0);
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
