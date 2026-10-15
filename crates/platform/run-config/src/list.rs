//! The run configurations of a project and what goes with them: the
//! selected one, the custom order, the recently used temporary ones, the
//! pinned ones, the start history and the last executor of each, as the
//! reference IDE's `RunManagerImpl` keeps them.

use std::cmp::Ordering;

use crate::model::{ConfigKey, RunConfiguration, Storage, Types};
use crate::natural;

/// How many temporary configurations are kept
/// (`temporary.configurations.limit`).
pub const TEMPORARY_LIMIT: usize = 5;

/// The configurations of a project.
#[derive(Debug, Clone, PartialEq)]
pub struct RunConfigurations {
    configs: Vec<RunConfiguration>,
    /// The selected configuration; `None` is Current File.
    pub selected: Option<ConfigKey>,
    /// The custom order (`<list>`); empty sorts by name.
    pub order: Vec<ConfigKey>,
    /// Temporary configurations, most recently used first.
    pub recent_temporary: Vec<ConfigKey>,
    pub pinned: Vec<ConfigKey>,
    /// Configurations started, most recent first.
    pub history: Vec<ConfigKey>,
    /// The executor used last per configuration (`Run`, `Debug`).
    pub executors: Vec<(ConfigKey, String)>,
    /// How many temporary configurations are kept.
    pub temporary_limit: usize,
    /// The run widget's dropdown shows every configuration below the
    /// recent ones ("All Configurations", `allConfigurationsExpanded`).
    pub all_expanded: bool,
    /// The per-user configurations of the reference were imported on this
    /// machine (the personal import marker).
    pub personal_imported: bool,
    /// Keys of the personal table this version does not know.
    pub extra: toml::Table,
}

impl Default for RunConfigurations {
    fn default() -> Self {
        Self {
            configs: Vec::new(),
            selected: None,
            order: Vec::new(),
            recent_temporary: Vec::new(),
            pinned: Vec::new(),
            history: Vec::new(),
            executors: Vec::new(),
            temporary_limit: TEMPORARY_LIMIT,
            all_expanded: false,
            personal_imported: false,
            extra: toml::Table::new(),
        }
    }
}

impl RunConfigurations {
    /// The configurations in the order they were added or loaded.
    pub fn all(&self) -> &[RunConfiguration] {
        &self.configs
    }

    pub fn len(&self) -> usize {
        self.configs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    pub fn get(&self, key: &ConfigKey) -> Option<&RunConfiguration> {
        self.configs
            .iter()
            .find(|c| c.type_id == key.type_id && c.name == key.name)
    }

    pub fn get_mut(&mut self, key: &ConfigKey) -> Option<&mut RunConfiguration> {
        self.configs
            .iter_mut()
            .find(|c| c.type_id == key.type_id && c.name == key.name)
    }

    pub fn contains(&self, key: &ConfigKey) -> bool {
        self.get(key).is_some()
    }

    /// Configurations by name, whatever their type.
    pub fn find_by_name<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a RunConfiguration> {
        self.configs.iter().filter(move |c| c.name == name)
    }

    /// `name`, or `name (1)`, `name (2)`, … when the type has it already
    /// (`RunManager.suggestUniqueName`).
    pub fn unique_name(&self, type_id: &str, name: &str) -> String {
        let taken = |n: &str| {
            self.configs
                .iter()
                .any(|c| c.type_id == type_id && c.name == n)
        };
        if !taken(name) {
            return name.to_owned();
        }
        let base = base_name(name);
        (1..)
            .map(|i| format!("{base} ({i})"))
            .find(|n| !taken(n))
            .expect("a free name")
    }

    /// Adds a configuration, renaming it when its type has the name
    /// already. Returns its key.
    pub fn add(&mut self, mut config: RunConfiguration) -> ConfigKey {
        config.name = self.unique_name(&config.type_id, &config.name);
        let key = config.key();
        self.configs.push(config);
        key
    }

    /// Adds a temporary configuration (a context run) and drops the
    /// oldest temporary ones that are neither recently used nor pinned
    /// above the limit (`checkRecentsLimit`). Returns its key and the
    /// keys removed.
    pub fn add_temporary(&mut self, mut config: RunConfiguration) -> (ConfigKey, Vec<ConfigKey>) {
        config.temporary = true;
        config.storage = Storage::Personal;
        let key = self.add(config);
        self.use_temporary(&key);
        let removed = self.check_temporary_limit();
        (key, removed)
    }

    /// A temporary configuration was used: first in the recent list,
    /// which keeps the limit.
    pub fn use_temporary(&mut self, key: &ConfigKey) {
        if !self.get(key).is_some_and(|c| c.temporary) {
            return;
        }
        self.recent_temporary.retain(|k| k != key);
        self.recent_temporary.insert(0, key.clone());
        self.recent_temporary.truncate(self.temporary_limit);
    }

    /// Removes temporary configurations above the limit: unpinned ones
    /// that are not in the recent list, in the order they were added.
    pub fn check_temporary_limit(&mut self) -> Vec<ConfigKey> {
        self.recent_temporary.truncate(self.temporary_limit);
        let counted = self
            .configs
            .iter()
            .filter(|c| c.temporary && !self.pinned.contains(&c.key()))
            .count();
        let mut excess = counted.saturating_sub(self.temporary_limit);
        let mut removed = Vec::new();
        for c in &self.configs {
            if excess == 0 {
                break;
            }
            let key = c.key();
            if c.temporary && !self.recent_temporary.contains(&key) && !self.pinned.contains(&key) {
                removed.push(key);
                excess -= 1;
            }
        }
        for key in &removed {
            self.remove(key);
        }
        removed
    }

    /// Removes a configuration and every reference to it.
    pub fn remove(&mut self, key: &ConfigKey) -> Option<RunConfiguration> {
        let index = self
            .configs
            .iter()
            .position(|c| c.type_id == key.type_id && c.name == key.name)?;
        let removed = self.configs.remove(index);
        if self.selected.as_ref() == Some(key) {
            self.selected = None;
        }
        for list in [
            &mut self.order,
            &mut self.recent_temporary,
            &mut self.pinned,
            &mut self.history,
        ] {
            list.retain(|k| k != key);
        }
        self.executors.retain(|(k, _)| k != key);
        Some(removed)
    }

    /// Renames a configuration (references to it follow). Fails when its
    /// type has the name already or the name is empty.
    pub fn rename(&mut self, key: &ConfigKey, name: &str) -> Result<ConfigKey, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Configuration name shouldn't be empty".to_owned());
        }
        if name == key.name {
            return Ok(key.clone());
        }
        let new = ConfigKey::new(&key.type_id, name);
        if self.contains(&new) {
            return Err(format!("A configuration named '{name}' already exists"));
        }
        let Some(config) = self.get_mut(key) else {
            return Err(format!("No run configuration '{}'", key.name));
        };
        config.name = name.to_owned();
        config.name_is_generated = false;
        self.rekey(key, &new);
        Ok(new)
    }

    /// Points every reference to `old` at `new`.
    fn rekey(&mut self, old: &ConfigKey, new: &ConfigKey) {
        if self.selected.as_ref() == Some(old) {
            self.selected = Some(new.clone());
        }
        for list in [
            &mut self.order,
            &mut self.recent_temporary,
            &mut self.pinned,
            &mut self.history,
        ] {
            for k in list.iter_mut().filter(|k| *k == old) {
                *k = new.clone();
            }
        }
        for (k, _) in self.executors.iter_mut().filter(|(k, _)| k == old) {
            *k = new.clone();
        }
    }

    /// Replaces a configuration by an edited copy (its name may differ;
    /// references follow).
    pub fn replace(&mut self, key: &ConfigKey, config: RunConfiguration) -> bool {
        let Some(index) = self
            .configs
            .iter()
            .position(|c| c.type_id == key.type_id && c.name == key.name)
        else {
            return false;
        };
        let new = config.key();
        self.configs[index] = config;
        if new != *key {
            self.rekey(key, &new);
        }
        true
    }

    /// Makes a temporary configuration permanent (`makeStable`).
    pub fn make_permanent(&mut self, key: &ConfigKey) -> bool {
        let Some(config) = self.get_mut(key) else {
            return false;
        };
        if !config.temporary {
            return false;
        }
        config.temporary = false;
        self.recent_temporary.retain(|k| k != key);
        true
    }

    /// Selects a configuration (`None`: Current File). An unknown key
    /// selects nothing and returns false.
    pub fn select(&mut self, key: Option<ConfigKey>) -> bool {
        match key {
            Some(key) if !self.contains(&key) => false,
            key => {
                self.selected = key;
                true
            }
        }
    }

    /// The selected configuration.
    pub fn selected_config(&self) -> Option<&RunConfiguration> {
        self.selected.as_ref().and_then(|k| self.get(k))
    }

    /// When nothing is selected after loading, the first configuration in
    /// list order is (`selectAnyConfiguration`).
    pub fn select_any(&mut self, types: &Types) {
        if self.selected_config().is_none() {
            self.selected = self.sorted(types).first().map(|c| c.key());
        }
    }

    /// Pins or unpins a configuration; returns whether it is pinned.
    pub fn toggle_pin(&mut self, key: &ConfigKey) -> bool {
        if let Some(i) = self.pinned.iter().position(|k| k == key) {
            self.pinned.remove(i);
            false
        } else {
            self.pinned.push(key.clone());
            true
        }
    }

    /// A configuration was started with `executor` (`Run`): first in the
    /// history, its executor remembered.
    pub fn started(&mut self, key: &ConfigKey, executor: &str) {
        self.history.retain(|k| k != key);
        self.history.insert(0, key.clone());
        let keep = (self.pinned.len() + 2 * RECENT_COUNT).max(5);
        self.history.truncate(keep);
        match self.executors.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = executor.to_owned(),
            None => self.executors.push((key.clone(), executor.to_owned())),
        }
        self.use_temporary(key);
    }

    /// The executor used last for a configuration.
    pub fn executor(&self, key: &ConfigKey) -> Option<&str> {
        self.executors
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, e)| e.as_str())
    }

    /// The configurations in the reference's list order
    /// (`compareByTypeAndFolderAndCustomComparator`): by type name (unknown
    /// types last), then by folder (natural order, or the custom order;
    /// configurations without a folder last), permanent before temporary,
    /// then by name in natural order, or by the custom order.
    pub fn sorted<'a>(&'a self, types: &Types) -> Vec<&'a RunConfiguration> {
        let index = |c: &RunConfiguration| -> i64 {
            let key = c.key();
            self.order
                .iter()
                .position(|k| *k == key)
                .map_or(-1, |i| i as i64)
        };
        let folders: Vec<Option<&str>> = if self.order.is_empty() {
            let mut names: Vec<&str> = Vec::new();
            for c in &self.configs {
                if let Some(f) = c.folder.as_deref()
                    && !names.contains(&f)
                {
                    names.push(f);
                }
            }
            names.sort_by(|a, b| natural::compare(a, b));
            names.into_iter().map(Some).chain([None]).collect()
        } else {
            let mut with_order: Vec<(&str, i64)> = self
                .configs
                .iter()
                .filter_map(|c| c.folder.as_deref().map(|f| (f, index(c))))
                .collect();
            with_order.sort_by_key(|(_, i)| *i);
            let mut names: Vec<Option<&str>> = Vec::new();
            for (f, _) in with_order {
                if !names.contains(&Some(f)) {
                    names.push(Some(f));
                }
            }
            names.push(None);
            names
        };
        let folder_index =
            |c: &RunConfiguration| folders.iter().position(|f| *f == c.folder.as_deref());
        let mut list: Vec<&RunConfiguration> = self.configs.iter().collect();
        list.sort_by(|a, b| {
            let by_type = compare_types(types, a, b);
            if by_type != Ordering::Equal {
                return by_type;
            }
            if a.folder != b.folder {
                let by_folder = folder_index(a).cmp(&folder_index(b));
                if by_folder != Ordering::Equal {
                    return by_folder;
                }
            }
            match (a.temporary, b.temporary) {
                (false, true) => return Ordering::Less,
                (true, false) => return Ordering::Greater,
                _ => {}
            }
            if self.order.is_empty() {
                natural::compare(&a.name, &b.name)
            } else {
                let (i, j) = (index(a), index(b));
                if i == -1 && j == -1 {
                    a.name.cmp(&b.name)
                } else {
                    i.cmp(&j)
                }
            }
        });
        list
    }
}

/// How many configurations the run widget's Recent section shows
/// (`max.recent.run.configurations`).
pub const RECENT_COUNT: usize = 5;

/// `name (3)` without the number (`RunManager.extractBaseName`).
fn base_name(name: &str) -> &str {
    let Some(inner) = name.strip_suffix(')') else {
        return name;
    };
    let Some(open) = inner.rfind('(') else {
        return name;
    };
    let digits = &inner[open + 1..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return name;
    }
    inner[..open].trim_end()
}

/// Types by name, unknown types last (`compareTypesForUi`).
fn compare_types(types: &Types, a: &RunConfiguration, b: &RunConfiguration) -> Ordering {
    if a.type_id == b.type_id {
        return Ordering::Equal;
    }
    match (types.is_unknown(a), types.is_unknown(b)) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => natural::compare(types.name_of(a), types.name_of(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Reference, TypeInfo};

    fn types() -> Types {
        Types::new([
            TypeInfo {
                id: "python",
                name: "Python",
                icon: "python/file",
                reference_id: "PythonConfigurationType",
                allow_parallel: false,
            },
            TypeInfo {
                id: "shell-script",
                name: "Shell Script",
                icon: "fileTypes/shell",
                reference_id: "ShConfigurationType",
                allow_parallel: false,
            },
        ])
    }

    fn config(type_id: &str, name: &str) -> RunConfiguration {
        RunConfiguration::new(type_id, name, Storage::Personal)
    }

    fn names(list: &RunConfigurations) -> Vec<String> {
        list.sorted(&types())
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    #[test]
    fn unique_names_take_a_number() {
        let mut list = RunConfigurations::default();
        assert_eq!(list.add(config("python", "api")).name, "api");
        assert_eq!(list.add(config("python", "api")).name, "api (1)");
        assert_eq!(list.add(config("python", "api (1)")).name, "api (2)");
        // Another type may have the name.
        assert_eq!(list.add(config("shell-script", "api")).name, "api");
        assert_eq!(base_name("x (12)"), "x");
        assert_eq!(base_name("x (a)"), "x (a)");
        assert_eq!(base_name("plain"), "plain");
    }

    #[test]
    fn order_by_type_folder_temporary_and_name() {
        let mut list = RunConfigurations::default();
        let mut unknown = config("idea:Weird", "zzz");
        unknown.reference = Some(Reference {
            type_id: "Weird".into(),
            factory: None,
            type_name: None,
            xml: String::new(),
        });
        list.add(unknown);
        list.add(config("shell-script", "build"));
        let mut tests = config("idea:tests", "tests");
        tests.reference = Some(Reference {
            type_id: "tests".into(),
            factory: None,
            type_name: Some("Python tests".into()),
            xml: String::new(),
        });
        list.add(tests);
        list.add(config("python", "worker10"));
        let mut temp = config("python", "aaa-temporary");
        temp.temporary = true;
        list.add(temp);
        list.add(config("python", "worker2"));
        let mut in_folder = config("python", "zeta");
        in_folder.folder = Some("Servers".into());
        list.add(in_folder);
        let mut in_folder = config("python", "alpha");
        in_folder.folder = Some("Servers".into());
        list.add(in_folder);
        assert_eq!(
            names(&list),
            [
                "alpha",
                "zeta",
                "worker2",
                "worker10",
                "aaa-temporary",
                "tests",
                "build",
                "zzz"
            ]
        );
        // A custom order applies within type and folder; configurations
        // it does not name come first.
        list.order = vec![
            ConfigKey::new("python", "worker10"),
            ConfigKey::new("python", "worker2"),
            ConfigKey::new("python", "zeta"),
            ConfigKey::new("python", "alpha"),
        ];
        assert_eq!(
            names(&list),
            [
                "zeta",
                "alpha",
                "worker10",
                "worker2",
                "aaa-temporary",
                "tests",
                "build",
                "zzz"
            ]
        );
    }

    #[test]
    fn temporary_limit_keeps_recent_and_pinned() {
        let mut list = RunConfigurations::default();
        let mut removed_all = Vec::new();
        for i in 0..5 {
            let (_, removed) = list.add_temporary(config("python", &format!("t{i}")));
            assert!(removed.is_empty());
        }
        list.toggle_pin(&ConfigKey::new("python", "t0"));
        // The pinned one does not count; nothing goes yet.
        let (_, removed) = list.add_temporary(config("python", "t5"));
        assert!(removed.is_empty());
        // A seventh: the oldest unpinned one not recently used goes. The
        // recent list holds the last five: t2..t6, so t1 goes.
        let (_, removed) = list.add_temporary(config("python", "t6"));
        removed_all.extend(removed);
        assert_eq!(removed_all, [ConfigKey::new("python", "t1")]);
        assert!(list.contains(&ConfigKey::new("python", "t0")));
        assert!(!list.contains(&ConfigKey::new("python", "t1")));
        // Saving makes one permanent: it no longer counts.
        assert!(list.make_permanent(&ConfigKey::new("python", "t2")));
        assert!(!list.get(&ConfigKey::new("python", "t2")).unwrap().temporary);
        let (_, removed) = list.add_temporary(config("python", "t7"));
        assert!(removed.is_empty(), "{removed:?}");
    }

    #[test]
    fn selection_history_and_removal() {
        let mut list = RunConfigurations::default();
        let a = list.add(config("python", "b"));
        let b = list.add(config("python", "a"));
        assert!(!list.select(Some(ConfigKey::new("python", "missing"))));
        list.select_any(&types());
        assert_eq!(list.selected, Some(b.clone()));
        list.started(&a, "Run");
        list.started(&b, "Debug");
        list.started(&a, "Run");
        assert_eq!(list.history, [a.clone(), b.clone()]);
        assert_eq!(list.executor(&b), Some("Debug"));
        list.toggle_pin(&b);
        list.remove(&b);
        assert_eq!(list.selected, None);
        assert_eq!(list.history, std::slice::from_ref(&a));
        assert!(list.pinned.is_empty());
        assert_eq!(list.executor(&b), None);
        assert!(list.select(None));
    }
}
