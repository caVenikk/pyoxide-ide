//! Opening a project's run configurations: what is stored, and the
//! reference IDE's configurations imported once per storage class (ADR
//! 0012). The shared ones are marked as imported in the shared settings,
//! so a teammate who clones the project does not import them again; the
//! per-user ones are marked in the personal state, so each person's are
//! imported once on their machine. A configuration deleted after the
//! import does not come back.

use std::collections::HashMap;
use std::path::Path;

use crate::idea::{self, IdeaImport, Importer};
use crate::list::RunConfigurations;
use crate::model::{ConfigKey, Types};
use crate::store::{self, StoreError};

/// What an import added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub imported: usize,
    /// Configurations of types Pyoxide does not support, kept read-only.
    pub read_only: usize,
    /// Configurations renamed because their type had the name already:
    /// the name in the reference and the new one.
    pub renamed: Vec<(String, String)>,
}

impl Report {
    /// The notification: "Imported N run configurations (M kept
    /// read-only)", and the renamed ones.
    pub fn message(&self) -> String {
        let plural = if self.imported == 1 { "" } else { "s" };
        let mut text = format!("Imported {} run configuration{plural}", self.imported);
        if self.read_only > 0 {
            text.push_str(&format!(" ({} kept read-only)", self.read_only));
        }
        if !self.renamed.is_empty() {
            let renamed: Vec<String> = self
                .renamed
                .iter()
                .map(|(from, to)| format!("'{from}' as '{to}'"))
                .collect();
            text.push_str(&format!(". Renamed: {}", renamed.join(", ")));
        }
        text
    }
}

/// Adds the reference's configurations to `list`: the shared ones first
/// (they win a name, as they replace a per-user one with the same unique
/// id in the reference), then the per-user ones, then the selection, the
/// custom order, the recent temporary list and the executors where the
/// list has none. With `only_missing` a configuration whose type has its
/// name already is skipped (an import on request); otherwise it is
/// renamed.
pub fn merge(list: &mut RunConfigurations, import: IdeaImport, only_missing: bool) -> Report {
    let mut report = Report::default();
    let mut ids: HashMap<String, ConfigKey> = HashMap::new();
    let workspace = import.workspace;
    let configurations = import.shared.into_iter().chain(
        workspace
            .iter()
            .flat_map(|w| w.configurations.iter().cloned()),
    );
    for imported in configurations {
        let key = imported.config.key();
        if only_missing && list.contains(&key) {
            ids.entry(imported.unique_id).or_insert(key);
            continue;
        }
        let read_only = imported.config.is_unsupported();
        let original = imported.config.name.clone();
        let added = list.add(imported.config);
        if added.name != original {
            report.renamed.push((original, added.name.clone()));
        }
        report.imported += 1;
        report.read_only += usize::from(read_only);
        ids.entry(imported.unique_id).or_insert(added);
    }
    if let Some(w) = workspace {
        let map = |ids_: &[String]| -> Vec<ConfigKey> {
            ids_.iter().filter_map(|id| ids.get(id).cloned()).collect()
        };
        if list.selected.is_none() {
            list.selected = w.selected.as_ref().and_then(|id| ids.get(id).cloned());
        }
        if list.order.is_empty() {
            list.order = map(&w.order);
        }
        if list.recent_temporary.is_empty() {
            list.recent_temporary = map(&w.recent_temporary);
        }
        for (id, executor) in &w.executors {
            if let Some(key) = ids.get(id)
                && list.executor(key).is_none()
            {
                list.executors.push((key.clone(), executor.clone()));
            }
        }
    }
    report
}

/// A project's configurations as opened.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opened {
    pub list: RunConfigurations,
    /// What the one-time import added, when it ran.
    pub report: Option<Report>,
    /// Files that could not be read.
    pub errors: Vec<String>,
}

/// Loads the configurations of the project at `root`, importing the
/// reference's ones that were not imported yet, and selects the first one
/// when none is. Blocking: run it off the UI thread. `.idea` is only read.
pub fn open(root: &Path, importers: &[&dyn Importer], types: &Types) -> Result<Opened, StoreError> {
    let stored = store::load(root)?;
    let mut list = stored.list;
    let need_shared = !stored.shared_imported;
    let need_personal = !list.personal_imported;
    let mut report = None;
    if need_shared || need_personal {
        let import = idea::read(root, importers, need_shared, need_personal);
        let has_shared = !import.shared.is_empty();
        let has_personal = import
            .workspace
            .as_ref()
            .is_some_and(|w| !w.configurations.is_empty());
        if has_shared || has_personal {
            let r = merge(&mut list, import, false);
            if has_personal {
                list.personal_imported = true;
            }
            store::save(root, &mut list)?;
            if has_shared {
                store::mark_shared_imported(root)?;
            }
            report = Some(r);
        }
    }
    list.select_any(types);
    Ok(Opened {
        list,
        report,
        errors: stored.errors,
    })
}

/// Imports the reference's configurations again, on request: only those
/// whose type and name are not there yet are added. Saves the list and
/// marks both imports done.
pub fn import_again(
    root: &Path,
    list: &mut RunConfigurations,
    importers: &[&dyn Importer],
) -> Result<Report, StoreError> {
    let import = idea::read(root, importers, true, true);
    let has_shared = !import.shared.is_empty();
    let report = merge(list, import, true);
    list.personal_imported = true;
    store::save(root, list)?;
    if has_shared {
        store::mark_shared_imported(root)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let mut r = Report {
            imported: 1,
            ..Report::default()
        };
        assert_eq!(r.message(), "Imported 1 run configuration");
        r.imported = 4;
        r.read_only = 2;
        r.renamed = vec![("api".into(), "api (1)".into())];
        assert_eq!(
            r.message(),
            "Imported 4 run configurations (2 kept read-only). Renamed: 'api' as 'api (1)'"
        );
    }
}
