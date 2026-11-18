//! Filter and presentation choices in File Structure.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Section;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FileStructureSettings {
    pub groups: BTreeMap<String, bool>,
    pub sort_alphabetically: bool,
    pub narrow_down: bool,
    /// Offset from the centered position and size in logical pixels.
    pub placement: Option<[i32; 4]>,
}

impl Default for FileStructureSettings {
    fn default() -> Self {
        Self {
            groups: BTreeMap::new(),
            sort_alphabetically: false,
            narrow_down: true,
            placement: None,
        }
    }
}

impl Section for FileStructureSettings {
    const KEY: &'static str = "editor.file-structure";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn structure_choices_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut store = Store::load(path.clone());
        store.register::<FileStructureSettings>();
        let chosen = FileStructureSettings {
            groups: BTreeMap::from([
                ("DDL statements".into(), false),
                ("Select statements".into(), true),
            ]),
            sort_alphabetically: true,
            narrow_down: false,
            placement: Some([12, 8, 560, 420]),
        };
        assert!(store.set_section(&chosen));
        store.save_now().unwrap();
        let mut again = Store::load(path);
        again.register::<FileStructureSettings>();
        assert_eq!(again.section::<FileStructureSettings>(), chosen);
    }
}
