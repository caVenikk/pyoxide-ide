//! The one-time import of the reference IDE's settings (`[import]`) and
//! what it brings that has no page yet: file name associations
//! (`[file-types]`, kept for Editor > File Types).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::Section;

/// What the first start decided about the reference's settings: where
/// they were imported from, or [`ImportSettings::SKIPPED`]. Once set, the
/// first-run window does not come again; `settings.import-reference`
/// imports on request.
///
/// An import keeps a snapshot of the configuration it read (its data
/// files, in the configuration folder) and the features that took their
/// part of it ([`Self::applied`]): a feature that comes later takes its
/// part from the snapshot once, and the values changed after that are
/// never overwritten.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImportSettings {
    /// The application the settings came from (`PyCharm`, `PyCharmCE`, the
    /// folder's name for a folder of another name), or
    /// [`Self::SKIPPED`].
    pub source: Option<String>,
    /// Its version (`2026.2`).
    pub version: Option<String>,
    /// When the settings were imported (RFC 3339, UTC).
    pub time: Option<String>,
    /// The snapshot's folder, relative to the configuration folder
    /// (`imported/PyCharm-2026.2`).
    pub snapshot: Option<String>,
    /// The features that took their part of the import (by importer id).
    pub applied: BTreeMap<String, bool>,
}

impl ImportSettings {
    /// The import was skipped.
    pub const SKIPPED: &'static str = "skipped";

    /// The settings of `source` `version` were imported at `time`.
    pub fn imported(source: &str, version: &str, time: SystemTime) -> Self {
        Self {
            source: Some(source.to_owned()),
            version: (!version.is_empty()).then(|| version.to_owned()),
            time: Some(rfc3339(time)),
            ..Self::default()
        }
    }

    /// The import was skipped.
    pub fn skipped() -> Self {
        Self {
            source: Some(Self::SKIPPED.to_owned()),
            ..Self::default()
        }
    }

    /// With the snapshot at `path` (relative to the configuration folder).
    pub fn with_snapshot(mut self, path: &str) -> Self {
        self.snapshot = Some(path.to_owned());
        self
    }

    /// Imported or skipped: the first start is over.
    pub fn decided(&self) -> bool {
        self.source.is_some()
    }

    /// The settings were imported, not skipped.
    pub fn imported_from(&self) -> Option<&str> {
        self.source.as_deref().filter(|s| *s != Self::SKIPPED)
    }

    /// The snapshot's folder in the configuration folder `config_dir`; a
    /// path that leaves it is refused.
    pub fn snapshot_dir(&self, config_dir: &Path) -> Option<PathBuf> {
        self.imported_from()?;
        let relative = Path::new(self.snapshot.as_deref()?);
        let inside = relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        (inside && relative.components().next().is_some()).then(|| config_dir.join(relative))
    }

    /// The feature `id` took its part of the import.
    pub fn is_applied(&self, id: &str) -> bool {
        self.applied.get(id).copied().unwrap_or(false)
    }

    /// Records that the feature `id` took its part.
    pub fn mark_applied(&mut self, id: &str) {
        self.applied.insert(id.to_owned(), true);
    }
}

impl Section for ImportSettings {
    const KEY: &'static str = "import";
}

/// `2026-10-17T15:10:24Z`.
fn rfc3339(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The date of a day since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
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
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_792_249_824);
        let mut record = ImportSettings::imported("PyCharm", "2026.2", time)
            .with_snapshot("imported/PyCharm-2026.2");
        record.mark_applied("appearance");
        record.mark_applied("editor-font");
        assert!(store.set_section(&record));
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
            text.contains(
                "[import]\nsnapshot = \"imported/PyCharm-2026.2\"\nsource = \"PyCharm\"\n\
                 time = \"2026-10-17T15:10:24Z\"\nversion = \"2026.2\"\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("[import.applied]\nappearance = true\neditor-font = true\n"),
            "{text}"
        );
        assert!(text.contains("pattern = \"*.jinja\""), "{text}");
        let mut again = Store::load(file);
        again.register::<ImportSettings>();
        again.register::<FileTypeSettings>();
        let read = again.section::<ImportSettings>();
        assert_eq!(read, record);
        assert!(read.is_applied("appearance"));
        assert!(!read.is_applied("terminal"));
        assert_eq!(
            read.snapshot_dir(Path::new("/c")),
            Some(PathBuf::from("/c/imported/PyCharm-2026.2"))
        );
        assert_eq!(again.section::<FileTypeSettings>(), types);
        assert!(ImportSettings::skipped().decided());
        assert_eq!(ImportSettings::skipped().imported_from(), None);
    }

    #[test]
    fn a_snapshot_outside_the_configuration_folder_is_refused() {
        let at = |path: &str| {
            ImportSettings::imported("PyCharm", "2026.2", UNIX_EPOCH)
                .with_snapshot(path)
                .snapshot_dir(Path::new("/c"))
        };
        assert_eq!(at("../elsewhere"), None);
        assert_eq!(at("/abs"), None);
        assert_eq!(at(""), None);
        assert!(at("imported/PyCharm-2026.2").is_some());
        let skipped = ImportSettings::skipped().with_snapshot("imported/x");
        assert_eq!(skipped.snapshot_dir(Path::new("/c")), None);
    }

    #[test]
    fn times_read_as_rfc_3339() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let leap = UNIX_EPOCH + std::time::Duration::from_secs(951_825_600);
        assert_eq!(rfc3339(leap), "2000-02-29T12:00:00Z");
    }
}
