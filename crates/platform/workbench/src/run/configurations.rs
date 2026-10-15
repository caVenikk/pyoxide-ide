//! The run configurations of the open project (ADR 0012): loaded, and the
//! reference IDE's ones imported once, off the UI thread when the project
//! opens; selection and starts kept in the personal project state, written
//! in the background one save at a time.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AppContext as _, Context, EventEmitter};
use run_config::{ConfigKey, Importer, RunConfiguration, RunConfigurations, TypeInfo, Types};
use serde_json::{Value, json};

use super::RunHook;

/// What the configurations tell the workbench.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunConfigsEvent {
    /// A notification ("Imported 3 run configurations").
    Notice(String),
}

/// The run configurations of the open project.
pub struct RunConfigs {
    root: Option<PathBuf>,
    list: RunConfigurations,
    types: Types,
    importers: Arc<Vec<Arc<dyn Importer>>>,
    /// The project's configurations are being loaded (and imported).
    loading: bool,
    /// Counts project opens: a late load of a previous project is dropped.
    generation: u64,
    /// A write runs, and another one is due after it.
    saving: bool,
    save_again: bool,
    /// The dropdown shows every configuration below the recent ones
    /// ("All Configurations").
    pub show_all: bool,
    /// Files that could not be read.
    errors: Vec<String>,
}

impl EventEmitter<RunConfigsEvent> for RunConfigs {}

impl RunConfigs {
    /// The configurations of the product's types and the platform's (Shell
    /// Script); no project yet.
    pub fn new(provider: Option<&RunHook>) -> Self {
        let mut types = vec![run_config::shell::TYPE];
        let mut importers: Vec<Arc<dyn Importer>> =
            vec![Arc::new(run_config::shell::ShellScriptImporter)];
        if let Some(provider) = provider {
            types.extend(provider.0.configuration_types());
            importers.extend(provider.0.importers());
        }
        Self {
            root: None,
            list: RunConfigurations::default(),
            types: Types::new(types),
            importers: Arc::new(importers),
            loading: false,
            generation: 0,
            saving: false,
            save_again: false,
            show_all: false,
            errors: Vec::new(),
        }
    }

    pub fn list(&self) -> &RunConfigurations {
        &self.list
    }

    pub fn types(&self) -> &Types {
        &self.types
    }

    pub fn type_info(&self, config: &RunConfiguration) -> Option<&TypeInfo> {
        self.types.get(&config.type_id)
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Loads the configurations of the project at `root`, importing the
    /// reference's ones once, in the background.
    pub fn open(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.root = Some(root.clone());
        self.list = RunConfigurations::default();
        self.errors.clear();
        self.loading = true;
        self.show_all = false;
        let importers = self.importers.clone();
        let types = self.types.clone();
        let task = cx.background_spawn(async move {
            let refs: Vec<&dyn Importer> = importers.iter().map(|i| i.as_ref()).collect();
            run_config::import::open(&root, &refs, &types)
        });
        cx.spawn(async move |this, cx| {
            let opened = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match opened {
                    Ok(opened) => {
                        this.list = opened.list;
                        this.errors = opened.errors;
                        if let Some(report) = opened.report {
                            cx.emit(RunConfigsEvent::Notice(report.message()));
                        }
                    }
                    Err(e) => {
                        let message = format!("Cannot load run configurations: {e}");
                        this.errors = vec![message.clone()];
                        cx.emit(RunConfigsEvent::Notice(message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Imports the reference's configurations again, adding the ones that
    /// do not exist yet.
    pub fn import_again(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            cx.emit(RunConfigsEvent::Notice("No project is open".to_owned()));
            return;
        };
        let generation = self.generation;
        let mut list = self.list.clone();
        let importers = self.importers.clone();
        let task = cx.background_spawn(async move {
            let refs: Vec<&dyn Importer> = importers.iter().map(|i| i.as_ref()).collect();
            run_config::import::import_again(&root, &mut list, &refs).map(|r| (r, list))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let message = match result {
                    Ok((report, list)) => {
                        this.list = list;
                        if report.imported == 0 {
                            "No new run configurations to import".to_owned()
                        } else {
                            report.message()
                        }
                    }
                    Err(e) => format!("Cannot import run configurations: {e}"),
                };
                cx.emit(RunConfigsEvent::Notice(message));
                cx.notify();
            });
        })
        .detach();
    }

    /// A configuration by name, and by type when two types have the name.
    pub fn find(&self, name: &str, type_id: Option<&str>) -> Result<ConfigKey, String> {
        let found: Vec<&RunConfiguration> = self
            .list
            .find_by_name(name)
            .filter(|c| type_id.is_none_or(|t| t == c.type_id))
            .collect();
        match found.as_slice() {
            [one] => Ok(one.key()),
            [] => Err(format!("No run configuration '{name}'")),
            _ => Err(format!(
                "Several run configurations are named '{name}': give its type"
            )),
        }
    }

    /// Selects a configuration (`None`: Current File) and remembers it.
    pub fn select(&mut self, key: Option<ConfigKey>, cx: &mut Context<Self>) -> bool {
        if !self.list.select(key) {
            return false;
        }
        self.save(cx);
        cx.notify();
        true
    }

    /// A configuration was started: first in the history, its executor
    /// remembered.
    pub fn started(&mut self, key: &ConfigKey, cx: &mut Context<Self>) {
        self.list.started(key, "Run");
        self.save(cx);
        cx.notify();
    }

    /// Writes the configurations in the background, one write at a time.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.loading {
            return;
        }
        if self.saving {
            self.save_again = true;
            return;
        }
        self.saving = true;
        let generation = self.generation;
        let mut list = self.list.clone();
        let task = cx.background_spawn(async move {
            let result = run_config::store::save(&root, &mut list);
            (result, list)
        });
        cx.spawn(async move |this, cx| {
            let (result, saved) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                if this.generation != generation {
                    return;
                }
                // New shared configurations got their file names.
                for config in saved.all() {
                    if let Some(own) = this.list.get_mut(&config.key())
                        && own.file.is_none()
                    {
                        own.file = config.file.clone();
                    }
                }
                if let Err(e) = result {
                    cx.emit(RunConfigsEvent::Notice(format!(
                        "Cannot save run configurations: {e}"
                    )));
                }
                if std::mem::take(&mut this.save_again) {
                    this.save(cx);
                }
            });
        })
        .detach();
    }

    /// The configurations for `run.state`: in list order, with what the
    /// run widget shows.
    pub fn state_json(&self) -> Value {
        let list = &self.list;
        let key_json = |k: &ConfigKey| json!({"type": k.type_id, "name": k.name});
        json!({
            "loading": self.loading,
            "selected": list.selected.as_ref().map(key_json),
            "configurations": list
                .sorted(&self.types)
                .iter()
                .map(|c| {
                    let key = c.key();
                    json!({
                        "name": c.name,
                        "type": c.type_id,
                        "type_name": self.types.name_of(c),
                        "folder": c.folder,
                        "storage": if c.is_shared() { "shared" } else { "personal" },
                        "temporary": c.temporary,
                        "supported": !c.is_unsupported(),
                        "pinned": list.pinned.contains(&key),
                        "executor": list.executor(&key),
                    })
                })
                .collect::<Vec<_>>(),
            "order": list.order.iter().map(key_json).collect::<Vec<_>>(),
            "history": list.history.iter().map(key_json).collect::<Vec<_>>(),
            "show_all": self.show_all,
            "errors": self.errors,
        })
    }
}
