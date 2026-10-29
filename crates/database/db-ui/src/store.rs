//! What the database module needs of the product to keep data sources and
//! to try them (ADR 0021): the module never depends on `db-service` or a
//! driver, so the product hands it a [`DataSourceStore`] made from its
//! service. Profiles are the ADR 0004 ones (`db_core::Profile`), kept per
//! project; passwords only travel as [`Secret`] and are kept by the store
//! (the OS credential store, memory until restart, or nowhere).
//!
//! Reads and writes of files and of the credential store block: the module
//! calls them off the UI thread. Connecting is a future the module drops to
//! cancel it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use std::sync::Arc;

use db_core::schema::SchemaInfo;
use db_core::{ConnectionSource, Profile, Secret};
use db_model::ModelCache;

/// A future of the store, run by the module's executor (dropping it
/// cancels the work).
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A data source: its name (the profile's key, also the credential
/// store's account) and its profile.
#[derive(Debug, Clone, PartialEq)]
pub struct DataSource {
    pub name: String,
    pub profile: Profile,
}

/// What happens to a data source's saved password.
#[derive(Debug, Clone)]
pub enum PasswordChange {
    /// As it is.
    Keep,
    /// This password, kept as the profile's `password_save` says.
    Set(Secret),
    /// None kept any more.
    Clear,
}

/// The data sources of a project after the Data Sources dialog: the
/// complete list in its order, the renames and removals (their secrets
/// move or go), the passwords to change by (new) name.
#[derive(Debug, Clone, Default)]
pub struct SaveRequest {
    pub sources: Vec<DataSource>,
    pub renamed: Vec<(String, String)>,
    pub removed: Vec<String>,
    pub passwords: Vec<(String, PasswordChange)>,
}

/// A successful connection test.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerInfo {
    /// The database product (`PostgreSQL`, `Oracle`, `SQLite`).
    pub product: String,
    /// Its version as the server reports it.
    pub version: String,
    /// The client side: the driver and its version.
    pub driver: String,
    pub connect_ms: f64,
    pub ping_ms: f64,
}

/// A failed connection test or schema read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Failure {
    /// The server's or the driver's message, never holding a secret.
    pub message: String,
    /// The server's code (SQLSTATE, `ORA-…`).
    pub code: Option<String>,
    /// A plain-language cause and the next step (`db-service` hints).
    pub hint: Option<String>,
    pub next: Option<String>,
    /// The credentials were refused: the dialog offers to change them.
    pub auth: bool,
}

/// What an import of a PyCharm project's data sources did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportSummary {
    /// Data sources added or replaced, by their new names.
    pub imported: Vec<String>,
    /// Data sources left out, with the reason.
    pub skipped: Vec<(String, String)>,
}

/// The product's side of the database module.
pub trait DataSourceStore: Send + Sync {
    /// The project's data sources, in their order (none when the project
    /// has none yet).
    fn load(&self, project: &Path) -> Result<Vec<DataSource>, String>;

    /// Writes the project's data sources and their passwords.
    fn save(&self, project: &Path, request: SaveRequest) -> Result<(), String>;

    /// A password is kept for the data source (the dialog shows
    /// `<hidden>`).
    fn has_password(&self, project: &Path, name: &str) -> bool;

    /// Connecting to `source` would need the person to type a password: it
    /// needs one and none is to be had without asking (the environment, a
    /// kept one, one held for this run). The lookup never prompts, but it
    /// may read the credential store, so the module calls it off the UI
    /// thread. What code intelligence asks before it connects on its own.
    fn needs_prompt(&self, project: &Path, source: &DataSource) -> bool {
        source.profile.needs_password() && !self.has_password(project, &source.name)
    }

    /// Holds a password the person typed for the data source `name` (memory
    /// only, never written anywhere), so that the data source's connections
    /// open with it. For "Until restart" it stays for the run; for "Never"
    /// the caller releases it once the connect it served is done. Does not
    /// block.
    fn hold_password(&self, _project: &Path, _name: &str, _password: Secret) {}

    /// Forgets the password [`Self::hold_password`] holds for `name` (the
    /// server refused it, or it was typed for one connect of a data source
    /// that saves "Never").
    fn release_password(&self, _project: &Path, _name: &str) {}

    /// Connects to `source` as edited, with the password typed (`Some`)
    /// or the kept one (`None`), and disconnects; writes nothing.
    fn test(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<ServerInfo, Failure>>;

    /// The schemas of `source` (for its introspection scope), connecting
    /// as [`Self::test`] does.
    fn schemas(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<Vec<SchemaInfo>, Failure>>;

    /// The databases the server of `source` offers, for the Database
    /// field's list (empty for an engine with one database), connecting
    /// as [`Self::test`] does.
    fn databases(
        &self,
        project: &Path,
        source: DataSource,
        password: Option<Secret>,
    ) -> BoxFuture<Result<Vec<String>, Failure>>;

    /// Imports the data sources of the PyCharm project at `project`
    /// (`.idea/dataSources.xml`), without their passwords.
    fn import_pycharm(&self, project: &Path) -> Result<ImportSummary, String>;

    /// Connections to the project's data sources, by name (the
    /// `DataSourceId` of a data source is its name), with their kept
    /// passwords: what the module's data source handles (`db-session`)
    /// open their connections with.
    fn connections(&self, project: &Path) -> Arc<dyn ConnectionSource>;

    /// Where the metadata models of the project's data sources are kept
    /// between runs (ADR 0021: the user cache folder, never the project);
    /// `None` keeps them in memory only.
    fn model_cache(&self, project: &Path) -> Option<ModelCache>;

    /// The folder of the query consoles, one folder per data source in it
    /// (the application's configuration folder when `None`).
    fn console_folder(&self) -> Option<PathBuf> {
        None
    }
}

pub mod fake {
    //! A store in memory for tests: data sources by project, passwords by
    //! name, a test that succeeds with one password.

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use db_core::schema::SchemaInfo;
    use db_core::{Backend, Connection, DataSourceId, Profile, Purpose, Secret};

    use super::*;

    #[derive(Default)]
    struct State {
        sources: HashMap<PathBuf, Vec<DataSource>>,
        passwords: HashMap<String, Secret>,
        saves: usize,
        tests: usize,
        cache: Option<PathBuf>,
        consoles: Option<PathBuf>,
        connections: Option<Arc<dyn ConnectionSource>>,
    }

    /// A server that is not there: every connection is refused, and
    /// counted.
    #[derive(Default)]
    pub struct NoServer {
        opened: AtomicUsize,
    }

    impl NoServer {
        /// The message of the refused connections.
        pub const MESSAGE: &str = "connection refused (localhost:55432)";

        /// How many connections were asked for.
        pub fn opened(&self) -> usize {
            self.opened.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl ConnectionSource for NoServer {
        async fn open(&self, _: &DataSourceId, _: Purpose) -> db_core::Result<Arc<dyn Connection>> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            Err(db_core::Error::new(
                db_core::ErrorKind::Network,
                Self::MESSAGE,
            ))
        }
    }

    /// The fake store. [`FakeStore::password`] is the one the fake server
    /// takes; a test with another fails as refused credentials.
    pub struct FakeStore {
        state: Mutex<State>,
        accepted: String,
        hang: bool,
        cancelled: Arc<AtomicUsize>,
    }

    /// Counts a connection given up before it finished.
    struct Cancelled(Arc<AtomicUsize>);

    impl Drop for Cancelled {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl FakeStore {
        pub fn new(accepted: &str) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::default(),
                accepted: accepted.to_owned(),
                hang: false,
                cancelled: Arc::default(),
            })
        }

        /// A store whose connections never finish (a server that does not
        /// answer): they end only when cancelled.
        pub fn hanging() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::default(),
                accepted: String::new(),
                hang: true,
                cancelled: Arc::default(),
            })
        }

        /// Keeps the models of the data sources in `dir` (in memory only
        /// until this is called).
        pub fn set_model_cache(&self, dir: &Path) {
            self.state.lock().unwrap().cache = Some(dir.to_owned());
        }

        /// Keeps the query consoles in `dir`.
        pub fn set_console_folder(&self, dir: &Path) {
            self.state.lock().unwrap().consoles = Some(dir.to_owned());
        }

        /// The data source handles connect through `source` (a [`NoServer`]
        /// until this is called).
        pub fn set_connections(&self, source: Arc<dyn ConnectionSource>) {
            self.state.lock().unwrap().connections = Some(source);
        }

        /// How many hanging connections were cancelled.
        pub fn cancelled(&self) -> usize {
            self.cancelled.load(Ordering::SeqCst)
        }

        /// Puts a data source into a project as if it had been saved.
        pub fn add(&self, project: &Path, name: &str, profile: Profile) {
            self.state
                .lock()
                .unwrap()
                .sources
                .entry(project.to_owned())
                .or_default()
                .push(DataSource {
                    name: name.to_owned(),
                    profile,
                });
        }

        pub fn sources(&self, project: &Path) -> Vec<DataSource> {
            let state = self.state.lock().unwrap();
            state.sources.get(project).cloned().unwrap_or_default()
        }

        /// The kept password of `name`, exposed (tests only).
        pub fn password(&self, name: &str) -> Option<String> {
            let state = self.state.lock().unwrap();
            state.passwords.get(name).map(|s| s.expose().to_owned())
        }

        pub fn saves(&self) -> usize {
            self.state.lock().unwrap().saves
        }

        pub fn tests(&self) -> usize {
            self.state.lock().unwrap().tests
        }

        fn connect(
            &self,
            source: &DataSource,
            password: Option<Secret>,
        ) -> BoxFuture<Result<(), Failure>> {
            let kept = {
                let mut state = self.state.lock().unwrap();
                state.tests += 1;
                state.passwords.get(&source.name).cloned()
            };
            let needs = source.profile.needs_password();
            let given = password.or(kept).unwrap_or_default();
            let ok = !needs || given.expose() == self.accepted;
            let missing = source.profile.backend == Backend::Sqlite
                && source.profile.path.as_deref().is_none_or(|p| p.is_empty());
            if self.hang {
                let guard = Cancelled(self.cancelled.clone());
                return Box::pin(async move {
                    std::future::pending::<()>().await;
                    drop(guard);
                    Ok(())
                });
            }
            Box::pin(async move {
                if missing {
                    return Err(Failure {
                        message: "path is required for backend sqlite".into(),
                        ..Failure::default()
                    });
                }
                if ok {
                    Ok(())
                } else {
                    Err(Failure {
                        message: "password authentication failed".into(),
                        code: Some("28P01".into()),
                        hint: Some("the server refused the user name or password".into()),
                        next: Some("check the user and the password".into()),
                        auth: true,
                    })
                }
            })
        }
    }

    impl DataSourceStore for FakeStore {
        fn load(&self, project: &Path) -> Result<Vec<DataSource>, String> {
            Ok(self.sources(project))
        }

        fn save(&self, project: &Path, request: SaveRequest) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state.saves += 1;
            for (old, new) in &request.renamed {
                if let Some(p) = state.passwords.remove(old) {
                    state.passwords.insert(new.clone(), p);
                }
            }
            for name in &request.removed {
                state.passwords.remove(name);
            }
            for (name, change) in request.passwords {
                match change {
                    PasswordChange::Keep => {}
                    PasswordChange::Set(secret) => {
                        let never =
                            request
                                .sources
                                .iter()
                                .find(|s| s.name == name)
                                .is_some_and(|s| {
                                    s.profile.gui_options().password_save
                                        == db_core::PasswordSave::Never
                                });
                        if never {
                            state.passwords.remove(&name);
                        } else {
                            state.passwords.insert(name, secret);
                        }
                    }
                    PasswordChange::Clear => {
                        state.passwords.remove(&name);
                    }
                }
            }
            state.sources.insert(project.to_owned(), request.sources);
            Ok(())
        }

        fn has_password(&self, _: &Path, name: &str) -> bool {
            self.state.lock().unwrap().passwords.contains_key(name)
        }

        fn hold_password(&self, _: &Path, name: &str, password: Secret) {
            self.state
                .lock()
                .unwrap()
                .passwords
                .insert(name.to_owned(), password);
        }

        fn release_password(&self, _: &Path, name: &str) {
            self.state.lock().unwrap().passwords.remove(name);
        }

        fn test(
            &self,
            _: &Path,
            source: DataSource,
            password: Option<Secret>,
        ) -> BoxFuture<Result<ServerInfo, Failure>> {
            let connect = self.connect(&source, password);
            Box::pin(async move {
                connect.await?;
                Ok(ServerInfo {
                    product: "PostgreSQL".into(),
                    version: "18.6".into(),
                    driver: "fake driver 1.0".into(),
                    connect_ms: 3.0,
                    ping_ms: 1.0,
                })
            })
        }

        fn databases(
            &self,
            _: &Path,
            source: DataSource,
            password: Option<Secret>,
        ) -> BoxFuture<Result<Vec<String>, Failure>> {
            let connect = self.connect(&source, password);
            Box::pin(async move {
                connect.await?;
                Ok(vec!["postgres".into(), "pyoxide".into()])
            })
        }

        fn schemas(
            &self,
            _: &Path,
            source: DataSource,
            password: Option<Secret>,
        ) -> BoxFuture<Result<Vec<SchemaInfo>, Failure>> {
            let connect = self.connect(&source, password);
            Box::pin(async move {
                connect.await?;
                let schema = |name: &str, is_system, is_current| SchemaInfo {
                    name: name.into(),
                    is_system,
                    is_current,
                    ..SchemaInfo::default()
                };
                Ok(vec![
                    schema("demo", false, false),
                    schema("information_schema", true, false),
                    schema("pg_catalog", true, false),
                    schema("public", false, true),
                ])
            })
        }

        fn import_pycharm(&self, project: &Path) -> Result<ImportSummary, String> {
            let mut profile = Profile::new(Backend::Postgres);
            profile.host = "localhost".into();
            profile.database = Some("imported".into());
            profile.user = "app".into();
            self.add(project, "imported", profile);
            Ok(ImportSummary {
                imported: vec!["imported".into()],
                skipped: Vec::new(),
            })
        }

        fn connections(&self, _: &Path) -> Arc<dyn ConnectionSource> {
            let mut state = self.state.lock().unwrap();
            state
                .connections
                .get_or_insert_with(|| Arc::new(NoServer::default()))
                .clone()
        }

        fn model_cache(&self, _: &Path) -> Option<ModelCache> {
            let state = self.state.lock().unwrap();
            state.cache.as_ref().map(ModelCache::new)
        }

        fn console_folder(&self) -> Option<PathBuf> {
            self.state.lock().unwrap().consoles.clone()
        }
    }
}
