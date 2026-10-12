//! The Search Everywhere settings that last across sessions: where the
//! popup was left and how large it was, the Files tab's scope, and what the
//! filters leave out.

use std::path::PathBuf;

use gpui::{App, AppContext as _, Global};
use serde::{Deserialize, Serialize};

use crate::popup::PopupPlace;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchEverywhereSettings {
    /// Where the popup was moved and how it was resized.
    pub popup: Option<PopupPlace>,
    /// File types unchecked in the Files tab's filter (by name; "Directory"
    /// for folders).
    pub hidden_file_types: Vec<String>,
    /// Kinds of results unchecked in the All tab's filter.
    pub hidden_providers: Vec<String>,
    /// The Files tab's scope is All Places (else Project Files).
    pub files_all_places: bool,
}

#[derive(Default)]
struct Store {
    settings: SearchEverywhereSettings,
    file: Option<PathBuf>,
}

impl Global for Store {}

/// Where the settings are kept: next to Find in Files' (`PYOXIDE_CONFIG_DIR`,
/// else the user's configuration folder), in `search-everywhere.toml`.
pub fn settings_file() -> Option<PathBuf> {
    crate::find_in_files::settings_file().map(|f| f.with_file_name("search-everywhere.toml"))
}

/// Reads the settings from `file` and saves them there from now on.
pub fn load_settings(cx: &mut App, file: PathBuf) {
    let loaded = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| toml::from_str::<SearchEverywhereSettings>(&text).ok());
    let store = cx.default_global::<Store>();
    if let Some(settings) = loaded {
        store.settings = settings;
    }
    store.file = Some(file);
}

pub fn settings(cx: &App) -> SearchEverywhereSettings {
    cx.try_global::<Store>()
        .map(|s| s.settings.clone())
        .unwrap_or_default()
}

/// Changes the settings and writes them in the background.
pub(super) fn update_settings(cx: &mut App, f: impl FnOnce(&mut SearchEverywhereSettings)) {
    let store = cx.default_global::<Store>();
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
