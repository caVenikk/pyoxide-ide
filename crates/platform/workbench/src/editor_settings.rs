//! Editor settings that last across sessions (`editor.toml` next to Find
//! in Files' settings): which files open with soft wraps and how wrapped
//! lines look.
//!
//! The reference keeps "Soft-wrap these files" (off by default) with file
//! masks separated by `;` (`*.md; *.txt; *.rst; *.adoc` by default; `*`
//! when it is turned on without masks of its own), "Use original line's
//! indent for wrapped part" (on) with an extra indent (0), and whether the
//! signs at soft wraps show on every line (off: on the caret's line).

use std::path::{Path, PathBuf};

use gpui::{App, Global};
use project_search::FileMask;
use serde::{Deserialize, Serialize};

/// The masks shown while soft wraps are off and none were set.
pub const DEFAULT_SOFT_WRAP_FILES: &str = "*.md; *.txt; *.rst; *.adoc";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoftWrapSettings {
    /// "Soft-wrap these files".
    pub enabled: bool,
    /// The file masks, separated by `;`; `None` for the reference's default.
    pub files: Option<String>,
    /// Continuation rows take the line's indentation.
    pub use_original_indent: bool,
    /// Columns added to that indentation.
    pub indent: usize,
    /// Signs at the soft wraps of every line, not only the caret's.
    pub show_all: bool,
}

impl Default for SoftWrapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            files: None,
            use_original_indent: true,
            indent: 0,
            show_all: false,
        }
    }
}

impl SoftWrapSettings {
    /// Sets up the editor of `path`: soft wraps when the file matches, the
    /// indent of continuation rows and where the signs show.
    pub fn apply(
        &self,
        editor: &mut editor::Editor,
        path: &Path,
        cx: &mut gpui::Context<editor::Editor>,
    ) {
        editor.set_soft_wrap_indent(self.use_original_indent.then_some(self.indent), cx);
        editor.set_show_all_soft_wraps(self.show_all, cx);
        editor.set_soft_wraps(self.applies_to(path), cx);
    }

    /// The masks in effect.
    pub fn masks(&self) -> &str {
        match &self.files {
            Some(files) => files,
            None if self.enabled => "*",
            None => DEFAULT_SOFT_WRAP_FILES,
        }
    }

    /// Whether a file opens with soft wraps.
    pub fn applies_to(&self, path: &Path) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        FileMask::parse(&self.masks().replace(';', ",")).is_ok_and(|mask| mask.matches(name))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub soft_wraps: SoftWrapSettings,
}

#[derive(Default)]
struct Store {
    settings: EditorSettings,
}

impl Global for Store {}

/// Where the settings are kept: next to Find in Files' (`PYOXIDE_CONFIG_DIR`,
/// else the user's configuration folder), in `editor.toml`.
pub fn settings_file() -> Option<PathBuf> {
    crate::find_in_files::settings_file().map(|f| f.with_file_name("editor.toml"))
}

/// Reads the settings from `file` (keeping the defaults when there is none).
pub fn load_settings(cx: &mut App, file: PathBuf) {
    let loaded = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| toml::from_str::<EditorSettings>(&text).ok());
    if let Some(settings) = loaded {
        cx.default_global::<Store>().settings = settings;
    }
}

pub fn settings(cx: &App) -> EditorSettings {
    cx.try_global::<Store>()
        .map(|s| s.settings.clone())
        .unwrap_or_default()
}

/// Replaces the settings (tests and a future settings page).
pub fn set_settings(cx: &mut App, settings: EditorSettings) {
    cx.default_global::<Store>().settings = settings;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_wraps_apply_to_the_masked_files_once_turned_on() {
        let off = SoftWrapSettings::default();
        assert_eq!(off.masks(), DEFAULT_SOFT_WRAP_FILES);
        assert!(!off.applies_to(Path::new("/p/README.md")));
        let on = SoftWrapSettings {
            enabled: true,
            files: Some(DEFAULT_SOFT_WRAP_FILES.into()),
            ..SoftWrapSettings::default()
        };
        assert!(on.applies_to(Path::new("/p/README.md")));
        assert!(on.applies_to(Path::new("/p/notes.TXT")));
        assert!(!on.applies_to(Path::new("/p/main.py")));
        // Turned on without masks of its own: every file.
        let all = SoftWrapSettings {
            enabled: true,
            files: None,
            ..SoftWrapSettings::default()
        };
        assert!(all.applies_to(Path::new("/p/main.py")));
        let parsed: EditorSettings =
            toml::from_str("[soft_wraps]\nenabled = true\nfiles = \"*.py\"\n").unwrap();
        assert!(parsed.soft_wraps.applies_to(Path::new("main.py")));
    }
}
