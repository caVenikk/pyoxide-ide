//! Appearance & Behavior > Appearance (`[appearance]`): the theme and the
//! editor colour scheme.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Section;

/// The theme and the editor colour scheme (the reference's `laf.xml` and
/// `colors.scheme.xml`). Ids are those of the `themes` catalog: a built-in
/// theme or one imported into [`themes_dir`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppearanceSettings {
    /// The theme; `None` is the default theme (Islands Dark).
    pub theme: Option<String>,
    /// The editor colour scheme when it is not the theme's own; choosing
    /// another theme goes back to that theme's scheme.
    pub editor_scheme: Option<String>,
}

impl Section for AppearanceSettings {
    const KEY: &'static str = "appearance";
}

/// The folder of imported themes, `themes/` in the configuration folder
/// (one `<id>.toml` per theme, ADR 0017).
pub fn themes_dir() -> Option<PathBuf> {
    crate::config_dir().map(|d| d.join("themes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn defaults_are_not_written_and_ids_round_trip() {
        let mut store = Store::new();
        store.register::<AppearanceSettings>();
        assert_eq!(
            store.section::<AppearanceSettings>(),
            AppearanceSettings::default()
        );
        let chosen = AppearanceSettings {
            theme: Some("islands-light".into()),
            editor_scheme: None,
        };
        assert!(store.set_section(&chosen));
        assert_eq!(store.section::<AppearanceSettings>(), chosen);
        assert_eq!(
            store.get("appearance.theme"),
            Some(serde_json::json!("islands-light"))
        );
        // Back to the default: the key goes away.
        assert!(store.set_section(&AppearanceSettings::default()));
        assert_eq!(store.get("appearance.theme"), Some(serde_json::Value::Null));
    }
}
