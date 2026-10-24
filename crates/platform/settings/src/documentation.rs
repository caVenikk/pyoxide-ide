//! The options of quick documentation (`[editor.quick-documentation]`)
//! that the documentation popup's more menu toggles: Show Documentation
//! Popup First, Show on Mouse Move and the size of its text (Adjust Font
//! Size). The reference keeps them apart (`documentation.skip.popup` and
//! `quick.doc.font.size.v3` in `PropertiesComponent`,
//! `isShowQuickDocOnMouseOverElement` in `EditorSettingsExternalizable`);
//! here they share one table.

use serde::{Deserialize, Serialize};

use crate::Section;

/// The quick documentation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DocumentationSettings {
    /// Quick Documentation shows a popup first; off, it opens the
    /// Documentation tool window at once (`DocumentationManager.skipPopup`
    /// inverted; on by default).
    pub show_popup_first: bool,
    /// The popup over the text under the mouse shows the documentation of
    /// the symbol there (`isShowQuickDocOnMouseOverElement`, on by
    /// default); off, it shows the problems there only.
    pub show_on_mouse_move: bool,
    /// The size of documentation text (`DocumentationFontSize`).
    pub font_size: DocFontSize,
}

impl Default for DocumentationSettings {
    fn default() -> Self {
        Self {
            show_popup_first: true,
            show_on_mouse_move: true,
            font_size: DocFontSize::default(),
        }
    }
}

impl Section for DocumentationSettings {
    const KEY: &'static str = "editor.quick-documentation";
}

/// The sizes of documentation text that the Adjust Font Size slider steps
/// through (`com.intellij.openapi.options.FontSize`); `Small` by default
/// (`getDefaultFontSize`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocFontSize {
    XxSmall,
    XSmall,
    #[default]
    Small,
    Medium,
    Large,
    XLarge,
    XxLarge,
}

impl DocFontSize {
    /// The sizes from the smallest.
    pub const ALL: [DocFontSize; 7] = [
        DocFontSize::XxSmall,
        DocFontSize::XSmall,
        DocFontSize::Small,
        DocFontSize::Medium,
        DocFontSize::Large,
        DocFontSize::XLarge,
        DocFontSize::XxLarge,
    ];

    /// The size in points at the default UI font size (`FontSize.getSize`:
    /// 10, 12, 13, 14, 16, 18, 24).
    pub fn points(self) -> f32 {
        match self {
            DocFontSize::XxSmall => 10.0,
            DocFontSize::XSmall => 12.0,
            DocFontSize::Small => 13.0,
            DocFontSize::Medium => 14.0,
            DocFontSize::Large => 16.0,
            DocFontSize::XLarge => 18.0,
            DocFontSize::XxLarge => 24.0,
        }
    }

    /// How much larger documentation text is than at the default size:
    /// the UI font size scales it (the reference scales the quick
    /// documentation font with the UI font).
    pub fn scale(self) -> f32 {
        self.points() / DocFontSize::Small.points()
    }

    /// Its place among [`Self::ALL`] (the slider's position).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(2)
    }

    /// The size at a place of [`Self::ALL`], the nearest end beyond them.
    pub fn at(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Store;

    #[test]
    fn defaults_and_sizes() {
        let mut store = Store::new();
        store.register::<DocumentationSettings>();
        let settings = store.section::<DocumentationSettings>();
        assert!(settings.show_popup_first);
        assert!(settings.show_on_mouse_move);
        assert_eq!(settings.font_size, DocFontSize::Small);
        assert_eq!(
            store.get("editor.quick-documentation.font-size"),
            Some(json!("small"))
        );
        assert_eq!(DocFontSize::Small.scale(), 1.0);
        assert_eq!(DocFontSize::XxLarge.points(), 24.0);
        assert_eq!(DocFontSize::Large.index(), 4);
        assert_eq!(DocFontSize::at(0), DocFontSize::XxSmall);
        assert_eq!(DocFontSize::at(99), DocFontSize::XxLarge);
    }

    #[test]
    fn the_choices_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut store = Store::load(file.clone());
        store.register::<DocumentationSettings>();
        let changed = DocumentationSettings {
            show_popup_first: false,
            show_on_mouse_move: false,
            font_size: DocFontSize::XLarge,
        };
        assert!(store.set_section(&changed));
        store.save_now().unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "version = 1\n\n[editor.quick-documentation]\nfont-size = \"x-large\"\nshow-on-mouse-move = false\nshow-popup-first = false\n"
        );
        let mut again = Store::load(file);
        again.register::<DocumentationSettings>();
        assert_eq!(again.section::<DocumentationSettings>(), changed);
    }
}
