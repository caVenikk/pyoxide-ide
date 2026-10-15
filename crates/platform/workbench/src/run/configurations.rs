//! The run configurations of the open project (ADR 0012): loaded, and the
//! reference IDE's ones imported once, off the UI thread when the project
//! opens; selection and starts kept in the personal project state, written
//! in the background one save at a time.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext as _, Context, EventEmitter};
use run_config::{
    ConfigKey, Importer, RunConfiguration, RunConfigurations, TypeEditor, TypeInfo, Types,
};
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
    /// The forms of the types (the Run/Debug Configurations dialog).
    editors: Vec<Arc<dyn TypeEditor>>,
    provider: Option<RunHook>,
    /// Files of shared configurations that were deleted, removed with the
    /// next save.
    removed_files: Vec<String>,
    /// The project's configurations are being loaded (and imported).
    loading: bool,
    /// Counts project opens: a late load of a previous project is dropped.
    generation: u64,
    /// A write runs, and another one is due after it.
    saving: bool,
    save_again: bool,
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
        let mut editors: Vec<Arc<dyn TypeEditor>> =
            vec![Arc::new(run_config::form::ShellScriptEditor)];
        if let Some(provider) = provider {
            types.extend(provider.0.configuration_types());
            importers.extend(provider.0.importers());
            editors.extend(provider.0.editors());
        }
        Self {
            root: None,
            list: RunConfigurations::default(),
            types: Types::new(types),
            importers: Arc::new(importers),
            editors,
            provider: provider.cloned(),
            removed_files: Vec::new(),
            loading: false,
            generation: 0,
            saving: false,
            save_again: false,
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

    /// The forms of the types.
    pub fn editors(&self) -> &[Arc<dyn TypeEditor>] {
        &self.editors
    }

    /// The form of a type.
    pub fn editor(&self, type_id: &str) -> Option<Arc<dyn TypeEditor>> {
        self.editors
            .iter()
            .find(|e| e.type_id() == type_id)
            .cloned()
    }

    /// The types a new configuration can have, by name.
    pub fn creatable_types(&self) -> Vec<TypeInfo> {
        let mut types: Vec<TypeInfo> = self
            .types
            .all()
            .iter()
            .filter(|t| self.editor(t.id).is_some())
            .copied()
            .collect();
        types.sort_by(|a, b| run_config::natural::compare(a.name, b.name));
        types
    }

    /// What the product reads from an unsupported configuration.
    pub fn unsupported_details(&self, config: &RunConfiguration) -> Vec<(String, String)> {
        self.provider
            .as_ref()
            .map(|p| p.0.unsupported_details(config))
            .unwrap_or_default()
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The dropdown shows every configuration ("All Configurations").
    pub fn show_all(&self) -> bool {
        self.list.all_expanded
    }

    pub fn toggle_show_all(&mut self, cx: &mut Context<Self>) {
        self.list.all_expanded = !self.list.all_expanded;
        self.save(cx);
        cx.notify();
    }

    /// Deletes a configuration (its file too, when it is shared).
    pub fn delete(&mut self, key: &ConfigKey, cx: &mut Context<Self>) -> bool {
        let Some(removed) = self.list.remove(key) else {
            return false;
        };
        if removed.is_shared()
            && let Some(file) = removed.file
        {
            self.removed_files.push(file);
        }
        self.save(cx);
        cx.notify();
        true
    }

    /// Pins or unpins a configuration in the dropdown.
    pub fn toggle_pin(&mut self, key: &ConfigKey, cx: &mut Context<Self>) -> bool {
        if !self.list.contains(key) {
            return false;
        }
        self.list.toggle_pin(key);
        self.save(cx);
        cx.notify();
        true
    }

    /// Makes a temporary configuration permanent ("Save Configuration").
    pub fn save_temporary(&mut self, key: &ConfigKey, cx: &mut Context<Self>) -> bool {
        if !self.list.make_permanent(key) {
            return false;
        }
        self.save(cx);
        cx.notify();
        true
    }

    /// The configuration that runs `file` from its context: an existing
    /// one that runs the same, else a new temporary one (the oldest
    /// temporary ones above the limit go). It becomes the selected one
    /// (`select.run.configuration.from.context`). `None` when the product
    /// has none for the file.
    pub fn context_configuration(
        &mut self,
        file: &Path,
        cx: &mut Context<Self>,
    ) -> Option<ConfigKey> {
        let root = self.root.clone()?;
        let provider = self.provider.clone()?;
        let context = provider.0.context_configuration(file, &root)?;
        let existing = self
            .list
            .all()
            .iter()
            .find(|c| provider.0.same_context(c, &context))
            .map(RunConfiguration::key);
        let key = match existing {
            Some(key) => key,
            None => {
                let (key, removed) = self.list.add_temporary(context);
                drop(removed);
                key
            }
        };
        self.list.select(Some(key.clone()));
        self.save(cx);
        cx.notify();
        Some(key)
    }

    /// Takes the configurations as the dialog left them: shared files of
    /// configurations that are gone or no longer shared are removed.
    pub fn apply_edits(&mut self, list: RunConfigurations, cx: &mut Context<Self>) {
        let used: Vec<&str> = list
            .all()
            .iter()
            .filter(|c| c.is_shared())
            .filter_map(|c| c.file.as_deref())
            .collect();
        for c in self.list.all() {
            if c.is_shared()
                && let Some(file) = &c.file
                && !used.contains(&file.as_str())
            {
                self.removed_files.push(file.clone());
            }
        }
        self.list = list;
        self.save(cx);
        cx.notify();
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
        self.removed_files.clear();
        self.loading = true;
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
    pub fn save(&mut self, cx: &mut Context<Self>) {
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
        let removed = std::mem::take(&mut self.removed_files);
        let task = cx.background_spawn(async move {
            // A file that a shared configuration took again is kept.
            for file in removed {
                let taken = list
                    .all()
                    .iter()
                    .any(|c| c.is_shared() && c.file.as_deref() == Some(file.as_str()));
                if !taken {
                    let _ = run_config::store::delete_shared(&root, &file);
                }
            }
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
            "show_all": list.all_expanded,
            "pinned": list.pinned.iter().map(key_json).collect::<Vec<_>>(),
            "recent_temporary": list.recent_temporary.iter().map(key_json).collect::<Vec<_>>(),
            "errors": self.errors,
        })
    }
}
