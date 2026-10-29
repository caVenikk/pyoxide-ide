//! Editor settings of the application store that the main windows follow
//! (ADR 0017): which files open with soft wraps and how wrapped lines look
//! (`[editor.soft-wraps]`, the Soft Wraps group of Editor > General). A
//! change reaches the open editors at once, as the reference's
//! `refreshAllEditors` after Apply.

use std::path::Path;

use gpui::{App, Context};
use project_search::FileMask;

pub use settings::{DEFAULT_SOFT_WRAP_FILES, SoftWrapSettings};

use crate::workbench::Workbench;

/// The soft wrap settings of the store.
pub fn soft_wraps(cx: &App) -> SoftWrapSettings {
    settings_ui::store::get(cx)
}

/// Stores the soft wrap settings; the open editors follow.
pub fn set_soft_wraps(cx: &mut App, settings: &SoftWrapSettings) {
    settings_ui::store::set(cx, settings);
}

/// Whether a file opens with soft wraps.
pub fn applies_to(settings: &SoftWrapSettings, path: &Path) -> bool {
    if !settings.enabled {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // Masks cleared: no files ("No files").
    let masks = settings.masks().replace(';', ",");
    if masks.split(',').all(|m| m.trim().is_empty()) {
        return false;
    }
    FileMask::parse(&masks).is_ok_and(|mask| mask.matches(name))
}

/// Sets up the editor of `path`: soft wraps when the file matches, the
/// indent of continuation rows and where the signs show.
pub fn apply(
    settings: &SoftWrapSettings,
    editor: &mut editor::Editor,
    path: &Path,
    cx: &mut Context<editor::Editor>,
) {
    editor.set_soft_wrap_indent(settings.use_original_indent.then_some(settings.indent), cx);
    editor.set_show_all_soft_wraps(settings.show_all, cx);
    editor.set_soft_wraps(applies_to(settings, path), cx);
}

/// The editors of every main window take the soft wrap settings again
/// (after a change of the store).
pub(crate) fn follow(cx: &mut App) {
    let settings = soft_wraps(cx);
    for window in cx.windows() {
        let Some(handle) = window.downcast::<Workbench>() else {
            continue;
        };
        let _ = handle.update(cx, |w, _, cx| {
            for path in w.tab_paths() {
                for editor in w.views_of(&path) {
                    editor.update(cx, |e, cx| apply(&settings, e, &path, cx));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_wraps_apply_to_the_masked_files_once_turned_on() {
        let off = SoftWrapSettings::default();
        assert!(!applies_to(&off, Path::new("/p/README.md")));
        let on = SoftWrapSettings {
            enabled: true,
            files: Some(DEFAULT_SOFT_WRAP_FILES.into()),
            ..SoftWrapSettings::default()
        };
        assert!(applies_to(&on, Path::new("/p/README.md")));
        assert!(applies_to(&on, Path::new("/p/notes.TXT")));
        assert!(!applies_to(&on, Path::new("/p/main.py")));
        // Turned on with the masks cleared: every file.
        let all = SoftWrapSettings {
            enabled: true,
            files: None,
            ..SoftWrapSettings::default()
        };
        assert!(applies_to(&all, Path::new("/p/main.py")));
        let py = SoftWrapSettings {
            enabled: true,
            files: Some("*.py".into()),
            ..SoftWrapSettings::default()
        };
        assert!(applies_to(&py, Path::new("main.py")));
        let none = SoftWrapSettings {
            enabled: true,
            files: Some(String::new()),
            ..SoftWrapSettings::default()
        };
        assert!(!applies_to(&none, Path::new("README.md")));
    }
}
