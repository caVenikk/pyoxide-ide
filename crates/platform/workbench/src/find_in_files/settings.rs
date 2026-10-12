//! The Find in Files settings that last across sessions.

use std::path::PathBuf;

use gpui::{App, AppContext as _, Global};
use search::SearchOptions;
use serde::{Deserialize, Serialize};

use super::RECENT_MASKS;
use crate::popup::PopupPlace;

/// The Find in Files settings that last across sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FindInFilesSettings {
    /// The mask in use while the File mask box is checked.
    pub file_mask: Option<String>,
    /// Masks used before, newest first.
    pub recent_masks: Vec<String>,
    pub case_sensitive: bool,
    pub whole_words: bool,
    pub regex: bool,
    /// The popup stays open when it loses the focus.
    pub pinned: bool,
    /// Where the popup was moved and how it was resized.
    pub popup: Option<PopupPlace>,
    /// Share of the results list in the split with the preview.
    pub splitter: Option<f32>,
    /// The Directory scope searches subfolders too.
    pub recursive: bool,
    /// Folders searched before, newest first.
    pub recent_directories: Vec<String>,
    /// The scope chosen last on the Scope tab.
    pub search_scope: Option<String>,
}

impl Default for FindInFilesSettings {
    fn default() -> Self {
        Self {
            file_mask: None,
            recent_masks: Vec::new(),
            case_sensitive: false,
            whole_words: false,
            regex: false,
            pinned: false,
            popup: None,
            splitter: None,
            recursive: true,
            recent_directories: Vec::new(),
            search_scope: None,
        }
    }
}

impl FindInFilesSettings {
    /// Settings before any were saved: the product's masks to offer.
    pub fn with_masks(masks: &[&str]) -> Self {
        Self {
            recent_masks: masks.iter().map(|m| (*m).to_owned()).collect(),
            ..Self::default()
        }
    }

    pub(super) fn options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            regex: self.regex,
        }
    }

    /// The mask shown in the field: the one in use, else the most recent.
    pub(super) fn shown_mask(&self) -> String {
        self.file_mask
            .clone()
            .or_else(|| self.recent_masks.first().cloned())
            .unwrap_or_default()
    }

    pub(super) fn remember_mask(&mut self, mask: &str) {
        remember(&mut self.recent_masks, mask);
    }

    pub(super) fn remember_directory(&mut self, dir: &str) {
        remember(&mut self.recent_directories, dir);
    }
}

/// Puts `item` first in a list of recent items.
fn remember(list: &mut Vec<String>, item: &str) {
    if item.is_empty() {
        return;
    }
    list.retain(|m| m != item);
    list.insert(0, item.to_owned());
    list.truncate(RECENT_MASKS);
}

/// The settings and the file they are kept in (none in tests).
#[derive(Default)]
struct SettingsStore {
    settings: FindInFilesSettings,
    file: Option<PathBuf>,
}

impl Global for SettingsStore {}

/// Registers the commands and the default settings (the product's masks).
pub fn init(cx: &mut App, masks: &[&str]) {
    ui::command::register(cx, super::COMMANDS);
    ui::command::register(cx, super::POPUP_COMMANDS);
    cx.set_global(SettingsStore {
        settings: FindInFilesSettings::with_masks(masks),
        file: None,
    });
}

/// Where the settings are kept: `PYOXIDE_CONFIG_DIR`, else the
/// configuration folder of the user, in `pyoxide/find.toml`.
pub fn settings_file() -> Option<PathBuf> {
    let dir = match std::env::var_os("PYOXIDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => dirs::config_dir()?.join("pyoxide"),
    };
    Some(dir.join("find.toml"))
}

/// Reads the settings from `file` (keeping the defaults when there is none)
/// and saves them there from now on.
pub fn load_settings(cx: &mut App, file: PathBuf) {
    let loaded = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| toml::from_str::<FindInFilesSettings>(&text).ok());
    let store = cx.default_global::<SettingsStore>();
    if let Some(settings) = loaded {
        store.settings = settings;
    }
    store.file = Some(file);
}

/// The current settings.
pub fn settings(cx: &App) -> FindInFilesSettings {
    cx.try_global::<SettingsStore>()
        .map(|s| s.settings.clone())
        .unwrap_or_default()
}

/// Changes the settings and writes them in the background.
pub(super) fn update_settings(cx: &mut App, f: impl FnOnce(&mut FindInFilesSettings)) {
    let store = cx.default_global::<SettingsStore>();
    let before = store.settings.clone();
    f(&mut store.settings);
    if store.settings == before {
        return;
    }
    let Some(file) = store.file.clone() else {
        return;
    };
    let Ok(text) = toml::to_string(&store.settings) else {
        return;
    };
    cx.background_spawn(async move {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&file, text) {
            eprintln!("cannot save {}: {e}", file.display());
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_are_remembered_newest_first() {
        let mut s = FindInFilesSettings::with_masks(&["*.py", "*.xml"]);
        assert_eq!(s.shown_mask(), "*.py");
        s.remember_mask("*.txt");
        s.remember_mask("*.py");
        assert_eq!(s.recent_masks, ["*.py", "*.txt", "*.xml"]);
        s.file_mask = Some("*.txt".into());
        assert_eq!(s.shown_mask(), "*.txt");
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<FindInFilesSettings>(&text).unwrap(), s);
        // A file from an older version: what is missing takes its default.
        let old: FindInFilesSettings = toml::from_str("pinned = true").unwrap();
        assert!(old.pinned && old.recent_masks.is_empty() && old.recursive);
    }
}
