//! Settings > Debugger > Data Views (`[debugger.data-views]`): how the
//! debugger shows values. The reference keeps them at application level
//! (`XDebuggerDataViewSettings` in `debugger.xml`, `<data-views>`); only
//! the option that the Debug toolbar's More > Debugger Settings > Show
//! Variable Values in Editor toggles is stored here so far, and the page
//! is not built yet.

use serde::{Deserialize, Serialize};

use crate::Section;

/// The debugger's data views (`XDebuggerDataViewSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DataViewSettings {
    /// Show the values of variables in the editor after the lines that
    /// declare them (`show-values-inline`, on by default).
    pub show_values_inline: bool,
}

impl Default for DataViewSettings {
    fn default() -> Self {
        Self {
            show_values_inline: true,
        }
    }
}

impl Section for DataViewSettings {
    const KEY: &'static str = "debugger.data-views";
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Store;

    #[test]
    fn values_are_shown_inline_by_default() {
        let mut store = Store::new();
        store.register::<DataViewSettings>();
        assert!(store.section::<DataViewSettings>().show_values_inline);
        assert_eq!(
            store.get("debugger.data-views.show-values-inline"),
            Some(json!(true))
        );
    }

    #[test]
    fn the_choice_survives_a_restart_and_the_default_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut store = Store::load(file.clone());
        store.register::<DataViewSettings>();
        let off = DataViewSettings {
            show_values_inline: false,
        };
        assert!(store.set_section(&off));
        assert!(!store.set_section(&off));
        store.save_now().unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "version = 1\n\n[debugger.data-views]\nshow-values-inline = false\n"
        );
        // A new start reads it back.
        let mut again = Store::load(file.clone());
        again.register::<DataViewSettings>();
        assert_eq!(again.section::<DataViewSettings>(), off);
        // Turned on again: the key goes, the file reads as the default.
        assert!(again.set_section(&DataViewSettings::default()));
        again.save_now().unwrap();
        assert!(
            !std::fs::read_to_string(&file)
                .unwrap()
                .contains("show-values-inline")
        );
        let mut last = Store::load(file);
        last.register::<DataViewSettings>();
        assert!(last.section::<DataViewSettings>().show_values_inline);
    }
}
