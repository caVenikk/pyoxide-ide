//! The one-time import of the reference IDE's settings (`[import]`) and
//! what it brings that has no page yet: file name associations
//! (`[file-types]`, kept for Editor > File Types).

use serde::{Deserialize, Serialize};

use crate::Section;

/// What the first start decided about the reference's settings: the name
/// of the configuration folder they were imported from (`PyCharm2026.2`),
/// or [`ImportSettings::SKIPPED`]. Once set, the first-run window does not
/// come again; `settings.import-reference` imports on request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImportSettings {
    pub source: Option<String>,
}

impl ImportSettings {
    /// The import was skipped.
    pub const SKIPPED: &'static str = "skipped";

    /// The settings of `folder` were imported.
    pub fn imported(folder: &str) -> Self {
        Self {
            source: Some(folder.to_owned()),
        }
    }

    /// The import was skipped.
    pub fn skipped() -> Self {
        Self {
            source: Some(Self::SKIPPED.to_owned()),
        }
    }

    /// Imported or skipped: the first start is over.
    pub fn decided(&self) -> bool {
        self.source.is_some()
    }
}

impl Section for ImportSettings {
    const KEY: &'static str = "import";
}

/// A file name pattern of a file type, by the reference's name of the type
/// (`Python`, `TOML`, `HTML`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTypeAssociation {
    /// `*.jinja`, `Pipfile`.
    pub pattern: String,
    #[serde(rename = "type")]
    pub file_type: String,
}

/// File name associations people added to file types or took from them
/// (the reference's `filetypes.xml`): imported once, for the File Types
/// page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FileTypeSettings {
    /// Patterns added to a file type.
    pub associations: Vec<FileTypeAssociation>,
    /// Built-in patterns taken from a file type.
    pub removed: Vec<FileTypeAssociation>,
}

impl Section for FileTypeSettings {
    const KEY: &'static str = "file-types";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn the_marker_and_associations_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        let mut store = Store::load(file.clone());
        store.register::<ImportSettings>();
        store.register::<FileTypeSettings>();
        assert!(!store.section::<ImportSettings>().decided());
        assert!(store.set_section(&ImportSettings::imported("PyCharm2026.2")));
        let types = FileTypeSettings {
            associations: vec![FileTypeAssociation {
                pattern: "*.jinja".into(),
                file_type: "HTML".into(),
            }],
            removed: Vec::new(),
        };
        assert!(store.set_section(&types));
        store.save_now().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("[import]\nsource = \"PyCharm2026.2\""),
            "{text}"
        );
        assert!(text.contains("pattern = \"*.jinja\""), "{text}");
        let mut again = Store::load(file);
        again.register::<ImportSettings>();
        again.register::<FileTypeSettings>();
        assert_eq!(
            again.section::<ImportSettings>().source.as_deref(),
            Some("PyCharm2026.2")
        );
        assert_eq!(again.section::<FileTypeSettings>(), types);
        assert!(ImportSettings::skipped().decided());
    }
}
