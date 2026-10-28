//! Application settings of Pyoxide IDE (ADR 0017).
//!
//! One [`Store`] holds `<config dir>/pyoxide/settings.toml`: `version = 1`
//! and tables by area (`[system.files]`, `[editor.font]`, `[terminal]`,
//! ...). Areas read their options as typed [`Section`]s with defaults;
//! keys and comments Pyoxide does not know survive, defaults are not
//! written, a file of a newer version is read and never rewritten, and a
//! broken file falls back to the defaults and is not written over until a
//! setting changes. Changes are saved by a [`SavePlan`] that reads the
//! file again and writes only the keys that changed, at once (a temporary
//! file renamed over it), so two windows or a second instance keep each
//! other's changes.
//!
//! The crate knows no UI toolkit; `settings-ui` keeps the store of the
//! running application, its change events and the Settings window.

mod appearance;
mod debugger;
mod documentation;
mod editor;
mod files;
mod import;
mod keymap;
mod store;
mod terminal;

use std::path::PathBuf;

pub use appearance::{AppearanceSettings, themes_dir};
pub use debugger::DataViewSettings;
pub use documentation::{DocFontSize, DocumentationSettings};
pub use editor::{DEFAULT_SOFT_WRAP_FILES, EditorFontSettings, SoftWrapSettings};
pub use files::{SaveSettings, SyncSettings};
pub use import::{FileTypeAssociation, FileTypeSettings, ImportSettings};
pub use keymap::{KeymapSettings, keymaps_dir};
pub use store::{Problem, SavePlan, Section, SettingsError, Store, VERSION, write_atomic};
pub use terminal::{TerminalFontSettings, TerminalSettings};

/// The application's configuration folder: the product's `CONFIG_DIR`
/// environment variable, else its slug in the user's configuration folder
/// (`app_info::config_dir`).
pub fn config_dir() -> Option<PathBuf> {
    app_info::config_dir()
}

/// Where the application settings are kept: `settings.toml` in
/// [`config_dir`].
pub fn settings_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.toml"))
}
