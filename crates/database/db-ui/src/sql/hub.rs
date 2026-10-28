//! What the editor side knows about SQL files: the settings of the open
//! projects (shared and personal), their data sources' dialects, and the
//! answer to "which dialect and scope has this file". The answer is a
//! lookup in memory, so the highlighter and the analysis threads ask it
//! freely; reading and writing the files is blocking and is done off the
//! UI thread by the callers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use sql_syntax::Dialect;
use workbench::FileDecoration;

use super::settings::{self, Origin, Resolved, SqlSettings};
use crate::store::DataSourceStore;

/// Where a setting is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.oxide/project.toml`, meant to be committed.
    Shared,
    /// `.oxide/workspace.toml`, this person only.
    Personal,
}

/// A data source as the SQL glue knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub dialect: Dialect,
    /// The database (service, SID or file) the source points at, for the
    /// chip of the query file toolbar.
    pub database: Option<String>,
}

/// One project's SQL state.
#[derive(Debug, Clone, Default)]
pub struct Project {
    pub shared: SqlSettings,
    pub personal: SqlSettings,
    /// The project's data sources and their dialects, in their order.
    pub sources: Vec<Source>,
}

impl Project {
    pub fn source(&self, name: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.name == name)
    }

    pub fn source_dialect(&self, name: &str) -> Option<Dialect> {
        self.source(name).map(|s| s.dialect)
    }

    fn resolve(&self, relative: Option<&str>) -> Resolved {
        settings::resolve(&self.shared, &self.personal, relative, &|name| {
            self.source_dialect(name)
        })
    }
}

/// The SQL dialect an engine speaks.
pub fn syntax_dialect(dialect: db_core::Dialect) -> Dialect {
    match dialect {
        db_core::Dialect::Oracle => Dialect::Oracle,
        db_core::Dialect::Postgres => Dialect::Postgres,
        db_core::Dialect::Sqlite => Dialect::Sqlite,
    }
}

/// A console file's data source and current schema: a console is bound
/// to its data source by where it lives, not by a mapping of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleBinding {
    pub source: Source,
    pub schema: Option<String>,
}

/// The settings of the open projects.
pub struct Hub {
    store: Arc<dyn DataSourceStore>,
    projects: RwLock<HashMap<PathBuf, Project>>,
    consoles: RwLock<HashMap<PathBuf, ConsoleBinding>>,
    /// Bumped by every change of a dialect or scope, for the holders of an
    /// answer (the highlighters).
    epoch: AtomicU64,
}

impl Hub {
    pub fn new(store: Arc<dyn DataSourceStore>) -> Self {
        Self {
            store,
            projects: RwLock::new(HashMap::new()),
            consoles: RwLock::new(HashMap::new()),
            epoch: AtomicU64::new(1),
        }
    }

    /// Changes whenever a file's dialect or scope may have changed.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<PathBuf, Project>> {
        self.projects.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<PathBuf, Project>> {
        self.projects
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Binds a console file to its data source (and the schema its session
    /// is in); every open editor of it is coloured again.
    pub fn bind_console(&self, file: &Path, binding: ConsoleBinding) {
        let changed = {
            let mut consoles = self
                .consoles
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            consoles.insert(file.to_owned(), binding.clone()) != Some(binding)
        };
        if changed {
            self.bump();
        }
    }

    /// The data source a console file is bound to.
    pub fn console_binding(&self, file: &Path) -> Option<ConsoleBinding> {
        self.consoles
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(file)
            .cloned()
    }

    pub fn is_loaded(&self, root: &Path) -> bool {
        self.read().contains_key(root)
    }

    /// Reads a project's settings and data sources (blocking). A file that
    /// cannot be read leaves its part empty: the editor still colours.
    pub fn read_project(&self, root: &Path) -> Project {
        let shared = project::store::load_table(root, settings::TABLE)
            .ok()
            .flatten()
            .map(|t| SqlSettings::from_table(&t))
            .unwrap_or_default();
        let personal = project::workspace::load_table(root, settings::TABLE)
            .ok()
            .flatten()
            .map(|t| SqlSettings::from_table(&t))
            .unwrap_or_default();
        Project {
            shared,
            personal,
            sources: self.read_sources(root),
        }
    }

    fn read_sources(&self, root: &Path) -> Vec<Source> {
        self.store
            .load(root)
            .unwrap_or_default()
            .into_iter()
            .map(|s| Source {
                dialect: syntax_dialect(s.profile.dialect()),
                database: s.profile.database_name().map(str::to_owned),
                name: s.name,
            })
            .collect()
    }

    /// Installs what [`Hub::read_project`] read.
    pub fn install(&self, root: &Path, project: Project) {
        self.write().insert(root.to_owned(), project);
        self.bump();
    }

    /// The project's data sources changed (the Data Sources dialog saved).
    pub fn set_sources(&self, root: &Path, sources: Vec<Source>) {
        if let Some(p) = self.write().get_mut(root) {
            p.sources = sources;
        }
        self.bump();
    }

    /// The data sources of the project, read from the store (blocking).
    pub fn load_sources(&self, root: &Path) -> Vec<Source> {
        self.read_sources(root)
    }

    /// The names of the project's data sources.
    pub fn source_names(&self, root: &Path) -> Vec<String> {
        self.read()
            .get(root)
            .map(|p| p.sources.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The project a file belongs to: the loaded root closest above it.
    pub fn root_of(&self, file: &Path) -> Option<PathBuf> {
        self.read()
            .keys()
            .filter(|r| file.starts_with(r))
            .max_by_key(|r| r.components().count())
            .cloned()
    }

    /// The dialect and scope of a file, from memory.
    pub fn resolve(&self, file: &Path) -> Resolved {
        if let Some(console) = self.console_binding(file) {
            return Resolved {
                dialect: console.source.dialect,
                dialect_origin: Origin::DataSource(console.source.name.clone()),
                data_source: Some(console.source.name),
                schema: console.schema,
                scope_origin: Origin::Console,
            };
        }
        let projects = self.read();
        let Some(root) = projects
            .keys()
            .filter(|r| file.starts_with(r))
            .max_by_key(|r| r.components().count())
        else {
            return Resolved::default();
        };
        let relative = settings::relative(root, file);
        projects[root].resolve(relative.as_deref())
    }

    /// A data source of the project the file belongs to, from memory.
    pub fn source_of(&self, file: &Path, name: &str) -> Option<Source> {
        if let Some(console) = self.console_binding(file) {
            return Some(console.source).filter(|s| s.name == name);
        }
        let projects = self.read();
        let root = projects
            .keys()
            .filter(|r| file.starts_with(r))
            .max_by_key(|r| r.components().count())?;
        projects[root].source(name).cloned()
    }

    /// The icon and hint of a file in the editor, project tree and
    /// toolbar title: the icon of the data source the file is attached to
    /// and its name (nothing for a file that is not attached).
    pub fn decoration(&self, file: &Path) -> FileDecoration {
        let resolved = self.resolve(file);
        let Some(name) = resolved.data_source else {
            return FileDecoration::default();
        };
        let icon = match self.source_of(file, &name).map(|s| s.dialect) {
            Some(Dialect::Postgres) => crate::icons::POSTGRES,
            Some(Dialect::Oracle) => crate::icons::ORACLE,
            Some(Dialect::Sqlite) => crate::icons::SQLITE,
            _ => crate::icons::DATA_SOURCE,
        };
        let console = self.console_binding(file).is_some();
        FileDecoration {
            icon: Some(icon),
            name: console.then(|| "console".to_owned()),
            autosave: console,
            breadcrumb: console.then(|| {
                vec![
                    "Database Consoles".to_owned(),
                    name.clone(),
                    format!("console [{name}]"),
                ]
            }),
            hint: Some(name),
        }
    }

    pub fn dialect(&self, file: &Path) -> Dialect {
        self.resolve(file).dialect
    }

    /// A copy of a project's state.
    pub fn project(&self, root: &Path) -> Option<Project> {
        self.read().get(root).cloned()
    }

    /// Changes one of the project's two settings sets and writes it
    /// (blocking), then publishes the change. The state is left as it was
    /// when the write fails.
    pub fn update(
        &self,
        root: &Path,
        kind: Kind,
        change: impl FnOnce(&mut SqlSettings),
    ) -> Result<(), String> {
        let mut project = self.project(root).unwrap_or_default();
        {
            let set = match kind {
                Kind::Shared => &mut project.shared,
                Kind::Personal => &mut project.personal,
            };
            change(set);
        }
        match kind {
            Kind::Shared => write_shared(root, &project.shared)?,
            Kind::Personal => write_personal(root, &project.personal)?,
        }
        self.install(root, project);
        Ok(())
    }
}

fn write_shared(root: &Path, settings: &SqlSettings) -> Result<(), String> {
    let text = |v: &Option<String>| v.as_ref().map(|s| toml::Value::String(s.clone()));
    let mappings = settings.to_table().and_then(|mut t| t.remove("mappings"));
    project::store::save_values(
        root,
        settings::TABLE,
        &[
            ("dialect", text(&settings.dialect)),
            ("data-source", text(&settings.data_source)),
            ("schema", text(&settings.schema)),
            ("mappings", mappings),
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn write_personal(root: &Path, settings: &SqlSettings) -> Result<(), String> {
    let table = settings
        .to_table()
        .and_then(|t| toml::to_string(&t).ok())
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .map(|doc| doc.as_table().clone());
    project::workspace::save_table(root, settings::TABLE, table)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// What a dialect or a scope answers when asked "why": for the widget's
/// tooltip.
pub fn origin_text(origin: &Origin) -> String {
    match origin {
        Origin::Mapping { path, personal } => {
            let scope = if *personal { "personal" } else { "shared" };
            format!("mapped at {path} ({scope})")
        }
        Origin::Project => "the project default".to_owned(),
        Origin::Console => "the console's data source".to_owned(),
        Origin::DataSource(name) => format!("the data source {name}"),
        Origin::None => "not configured".to_owned(),
    }
}
