//! Keymap (`[keymap]`): the keymap in use (ADR 0018).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Section;

/// The keymap in use (the reference's `keymap.xml`): a predefined keymap
/// ("macOS", "Windows", "VSCode", ...) or one in [`keymaps_dir`] by its
/// name. `None` is the platform's default keymap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct KeymapSettings {
    pub name: Option<String>,
}

impl Section for KeymapSettings {
    const KEY: &'static str = "keymap";
}

/// The folder of custom (imported) keymaps, `keymaps/` in the
/// configuration folder (one `<name>.toml` per keymap, ADR 0018).
pub fn keymaps_dir() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("keymaps"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn the_default_is_not_written() {
        let mut store = Store::new();
        store.register::<KeymapSettings>();
        assert_eq!(store.section::<KeymapSettings>(), KeymapSettings::default());
        let chosen = KeymapSettings {
            name: Some("VSCode".into()),
        };
        assert!(store.set_section(&chosen));
        assert_eq!(store.get("keymap.name"), Some(serde_json::json!("VSCode")));
        assert!(store.set_section(&KeymapSettings::default()));
        assert_eq!(store.get("keymap.name"), Some(serde_json::Value::Null));
    }
}
