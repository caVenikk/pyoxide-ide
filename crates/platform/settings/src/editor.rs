//! Editor > Font (`[editor.font]`) and the Soft Wraps group of Editor >
//! General (`[editor.soft-wraps]`).

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::Section;

/// The editor font chosen on Editor > Font (the reference's
/// `editor-font.xml`). A key that is not set leaves the theme's value:
/// the bundled themes use JetBrains Mono at 13 with line height 1.2 and no
/// ligatures, an imported colour scheme may bring its own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct EditorFontSettings {
    /// The font family.
    pub family: Option<String>,
    /// The size in points.
    #[serde(serialize_with = "crate::short_float::serialize")]
    pub size: Option<f32>,
    /// The line height, a multiple of the font's own.
    #[serde(serialize_with = "crate::short_float::serialize")]
    pub line_height: Option<f32>,
    /// Ligatures of the font are used.
    pub ligatures: Option<bool>,
    /// The family of the characters the font lacks (`<None>`: the
    /// platform's).
    pub fallback: Option<String>,
}

impl EditorFontSettings {
    /// The sizes allowed (`EditorFontsConstants`: 4 to 40).
    pub const SIZES: RangeInclusive<f32> = 4.0..=40.0;
    /// The line heights allowed (`EditorFontsConstants`: 0.6 to 3).
    pub const LINE_HEIGHTS: RangeInclusive<f32> = 0.6..=3.0;
    /// The family "Restore defaults" sets (`FontPreferences.DEFAULT_FONT_NAME`).
    pub const DEFAULT_FAMILY: &'static str = "JetBrains Mono";
    /// The size "Restore defaults" sets (`FontPreferences.DEFAULT_FONT_SIZE`
    /// of the New UI).
    pub const DEFAULT_SIZE: f32 = 13.0;
    /// The line height "Restore defaults" sets
    /// (`FontPreferences.DEFAULT_LINE_SPACING` of the New UI).
    pub const DEFAULT_LINE_HEIGHT: f32 = 1.2;

    /// A size in the allowed range.
    pub fn clamp_size(size: f32) -> f32 {
        size.clamp(*Self::SIZES.start(), *Self::SIZES.end())
    }

    /// A line height in the allowed range.
    pub fn clamp_line_height(height: f32) -> f32 {
        height.clamp(*Self::LINE_HEIGHTS.start(), *Self::LINE_HEIGHTS.end())
    }
}

impl Section for EditorFontSettings {
    const KEY: &'static str = "editor.font";
}

/// The masks "Soft-wrap these files" shows while none were set.
pub const DEFAULT_SOFT_WRAP_FILES: &str = "*.md; *.txt; *.rst; *.adoc";

/// Which files open with soft wraps and how wrapped lines look
/// (`EditorSettingsExternalizable` in the reference's `editor.xml`):
/// "Soft-wrap these files" (off by default) with file masks separated by
/// `;` ([`DEFAULT_SOFT_WRAP_FILES`] by default; `*` when it is turned on
/// with the masks cleared), "Use the original line's indent for wrapped
/// fragments" (on) with an additional indent (0), and whether the signs at
/// soft wraps show on every line (off: on the caret's line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SoftWrapSettings {
    /// "Soft-wrap these files".
    pub enabled: bool,
    /// The file masks, separated by `;`; `None` for the default.
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
    /// The additional indents allowed (two columns of digits).
    pub const INDENTS: RangeInclusive<usize> = 0..=99;

    /// The masks in effect.
    pub fn masks(&self) -> &str {
        match &self.files {
            Some(files) => files,
            None if self.enabled => "*",
            None => DEFAULT_SOFT_WRAP_FILES,
        }
    }
}

impl Section for SoftWrapSettings {
    const KEY: &'static str = "editor.soft-wraps";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn font_keys_are_written_only_when_set() {
        let mut store = Store::new();
        store.register::<EditorFontSettings>();
        assert_eq!(
            store.section::<EditorFontSettings>(),
            EditorFontSettings::default()
        );
        let chosen = EditorFontSettings {
            family: Some("Menlo".into()),
            size: Some(14.0),
            line_height: Some(1.5),
            ligatures: Some(true),
            fallback: None,
        };
        assert!(store.set_section(&chosen));
        assert_eq!(store.section::<EditorFontSettings>(), chosen);
        assert_eq!(
            store.get("editor.font.family"),
            Some(serde_json::json!("Menlo"))
        );
        assert_eq!(
            store.get("editor.font.line-height"),
            Some(serde_json::json!(1.5))
        );
        assert!(store.set_section(&EditorFontSettings::default()));
        assert_eq!(
            store.get("editor.font.family"),
            Some(serde_json::Value::Null)
        );
        assert_eq!(EditorFontSettings::clamp_size(100.0), 40.0);
        assert_eq!(EditorFontSettings::clamp_line_height(0.1), 0.6);
    }

    #[test]
    fn soft_wraps_round_trip_and_masks() {
        let mut store = Store::new();
        store.register::<SoftWrapSettings>();
        let off = store.section::<SoftWrapSettings>();
        assert_eq!(off, SoftWrapSettings::default());
        assert_eq!(off.masks(), DEFAULT_SOFT_WRAP_FILES);
        let on = SoftWrapSettings {
            enabled: true,
            files: Some("*.py".into()),
            indent: 4,
            ..SoftWrapSettings::default()
        };
        assert!(store.set_section(&on));
        assert_eq!(store.section::<SoftWrapSettings>(), on);
        assert_eq!(
            store.get("editor.soft-wraps.files"),
            Some(serde_json::json!("*.py"))
        );
        // Turned on with the masks cleared: every file.
        let all = SoftWrapSettings {
            enabled: true,
            files: None,
            ..SoftWrapSettings::default()
        };
        assert_eq!(all.masks(), "*");
    }
}
