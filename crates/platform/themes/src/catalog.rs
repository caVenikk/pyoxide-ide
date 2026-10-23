//! The themes the IDE offers (the reference's `UiThemeProviderListManager`
//! with `ThemeListProviderImpl`): the built-in themes and the themes
//! imported into a folder of the configuration (`themes/<id>.toml`), in the
//! reference's groups, and a theme combined with an editor colour scheme
//! chosen apart from it.
//!
//! Every theme also offers its editor colour scheme under its own name; an
//! imported `.icls` is a scheme only ([`ThemeHeader::scheme_only`]). The
//! scheme part of a theme is every text role and the UI roles that a
//! colour scheme colours (the editor, version control markers,
//! diagnostics, the console; see [`is_scheme_role`]).
//!
//! [`ThemeHeader::scheme_only`]: crate::ThemeHeader::scheme_only

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::builtin;
use crate::import::{
    ImportError, ImportReport, Imported, SourceKind, import_path, import_plugin_theme,
    import_scheme_named,
};
use crate::model::{Theme, ThemeFile, ThemeKind, skipped_warning};
use crate::registry::{Registry, ThemeError};
use crate::roles::UiRole;

/// The group a theme is listed in, in the reference's order: Islands, the
/// other bundled themes, then "Custom" for imported ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThemeGroup {
    /// The Islands themes, the defaults.
    Islands,
    /// The other themes that ship with the IDE.
    Bundled,
    /// Imported themes.
    Custom,
}

impl ThemeGroup {
    /// The title of the group's separator in the Theme combo box
    /// (`combobox.list.custom.section.title`); the other groups are set
    /// apart by a line only.
    pub fn title(self) -> Option<&'static str> {
        match self {
            ThemeGroup::Custom => Some("Custom"),
            ThemeGroup::Islands | ThemeGroup::Bundled => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeGroup::Islands => "islands",
            ThemeGroup::Bundled => "bundled",
            ThemeGroup::Custom => "custom",
        }
    }
}

/// A theme (or a scheme only) the catalog offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub kind: ThemeKind,
    pub group: ThemeGroup,
    /// Offered as an editor colour scheme only.
    pub scheme_only: bool,
    /// The file an imported theme was read from.
    pub path: Option<PathBuf>,
}

/// The themes the IDE offers, by id, in list order.
#[derive(Debug, Clone)]
pub struct Catalog {
    registry: Registry,
    builtin: HashSet<String>,
    entries: Vec<CatalogEntry>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::builtin()
    }
}

/// The scheme part of a theme: the role is coloured by an editor colour
/// scheme.
pub fn is_scheme_role(role: UiRole) -> bool {
    crate::import::is_scheme_role(role)
}

/// `theme` with the editor colour scheme of `scheme`: its text roles and
/// its scheme roles ([`is_scheme_role`]).
pub fn with_scheme(mut theme: Theme, scheme: &Theme) -> Theme {
    for &role in UiRole::ALL {
        if is_scheme_role(role) {
            theme.ui[role] = scheme.ui[role];
        }
    }
    theme.text = scheme.text.clone();
    theme
}

impl Catalog {
    /// The built-in themes.
    pub fn builtin() -> Self {
        let registry = Registry::with_builtin();
        let builtin: HashSet<String> = registry.ids().map(str::to_owned).collect();
        let mut catalog = Self {
            registry,
            builtin,
            entries: Vec::new(),
        };
        let ids: Vec<String> = catalog.builtin.iter().cloned().collect();
        for id in ids {
            if let Some(entry) = catalog.entry_for(&id, None) {
                catalog.entries.push(entry);
            }
        }
        catalog.sort();
        catalog
    }

    /// The built-in themes and the `*.toml` files of `dir` (imported
    /// themes). A file that cannot be read or resolved, or that takes the
    /// id of a built-in theme, is left out and named in the messages
    /// returned. A file with keys that name no role of this version (saved
    /// by a newer one) loads without them and has one message listing them.
    /// A missing folder holds no themes. Reads files: call it at start-up or
    /// off the UI thread.
    pub fn load(dir: &Path) -> (Self, Vec<String>) {
        let mut catalog = Self::builtin();
        let mut problems = Vec::new();
        let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(read) => read
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                problems.push(format!("cannot read {}: {e}", dir.display()));
                Vec::new()
            }
        };
        paths.sort();
        for path in paths {
            let origin = path.display().to_string();
            let file = std::fs::read_to_string(&path)
                .map_err(|source| ThemeError::Io {
                    path: path.clone(),
                    source,
                })
                .and_then(|text| ThemeFile::parse_toml(&text, &origin))
                .map(|(file, skipped)| {
                    if !skipped.is_empty() {
                        problems.push(skipped_warning(&origin, &skipped));
                    }
                    file
                });
            match file.and_then(|f| catalog.add(f, Some(path.clone()))) {
                Ok(_) => {}
                Err(e) => problems.push(format!("{origin}: {e}")),
            }
        }
        (catalog, problems)
    }

    /// Adds (or replaces) an imported theme file; its `extends` refers to
    /// themes of the catalog. Returns its id.
    pub fn add(&mut self, file: ThemeFile, path: Option<PathBuf>) -> Result<String, ThemeError> {
        let id = file.theme.id.clone();
        if self.builtin.contains(&id) {
            return Err(ThemeError::Parse {
                origin: id,
                message: "takes the id of a built-in theme".into(),
            });
        }
        // It must resolve before it is offered.
        self.registry.resolve_file(&file)?;
        self.registry.insert(file)?;
        self.entries.retain(|e| e.id != id);
        if let Some(entry) = self.entry_for(&id, path) {
            self.entries.push(entry);
        }
        self.sort();
        Ok(id)
    }

    fn entry_for(&self, id: &str, path: Option<PathBuf>) -> Option<CatalogEntry> {
        let file = self.registry.get(id)?;
        let kind = self.registry.resolve(id).ok()?.kind;
        let builtin = self.builtin.contains(id);
        let group = if !builtin {
            ThemeGroup::Custom
        } else if id.starts_with("islands-") {
            ThemeGroup::Islands
        } else {
            ThemeGroup::Bundled
        };
        Some(CatalogEntry {
            id: id.to_owned(),
            name: file.theme.name.clone(),
            kind,
            group,
            scheme_only: file.theme.scheme_only,
            path,
        })
    }

    /// Groups in their order, names within a group (`sortBy { it.name }`).
    fn sort(&mut self) {
        self.entries
            .sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
    }

    /// Every entry, themes and schemes, in list order.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// The UI themes, in list order.
    pub fn themes(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter().filter(|e| !e.scheme_only)
    }

    /// The editor colour schemes: every theme's own and the schemes
    /// imported alone, in list order.
    pub fn schemes(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter()
    }

    pub fn get(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// A UI theme of the catalog (not a scheme only).
    pub fn is_theme(&self, id: &str) -> bool {
        self.get(id).is_some_and(|e| !e.scheme_only)
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        self.builtin.contains(id)
    }

    /// A theme of the catalog, resolved.
    pub fn resolve(&self, id: &str) -> Result<Theme, ThemeError> {
        self.registry.resolve(id)
    }

    /// The theme `theme` with the editor colour scheme `scheme` (its own
    /// when `None` or the same id).
    pub fn compose(&self, theme: &str, scheme: Option<&str>) -> Result<Theme, ThemeError> {
        let resolved = self.registry.resolve(theme)?;
        match scheme.filter(|s| *s != theme) {
            Some(scheme) => Ok(with_scheme(resolved, &self.registry.resolve(scheme)?)),
            None => Ok(resolved),
        }
    }
}

/// A theme or scheme written by [`import_into`].
#[derive(Debug, Clone)]
pub struct ImportedFile {
    pub file: ThemeFile,
    pub path: PathBuf,
    pub report: ImportReport,
}

/// Imports a UI theme, a colour scheme, a plugin archive or a folder of
/// them ([`import_path`]) and writes each theme found as `<dir>/<id>.toml`,
/// replacing an earlier import of the same id. A scheme imported alone is
/// marked as a scheme only; an id of a built-in theme gets `-imported`
/// appended. Reads and writes files: call it off the UI thread.
pub fn import_into(path: &Path, dir: &Path) -> Result<Vec<ImportedFile>, ImportError> {
    write_imported(import_path(path, &[])?, dir)
}

/// Imports the UI theme at `entry` of a plugin archive
/// ([`import_plugin_theme`]) and writes it as `<dir>/<id>.toml`, as
/// [`import_into`] does. Reads and writes files: call it off the UI thread.
pub fn import_plugin_theme_into(
    archive: &Path,
    entry: &str,
    context: &[PathBuf],
    dir: &Path,
) -> Result<ImportedFile, ImportError> {
    let imported = import_plugin_theme(archive, entry, context)?;
    let mut files = write_imported(vec![imported], dir)?;
    Ok(files.remove(0))
}

/// Imports the editor colour scheme named `name` from `sources`
/// ([`import_scheme_named`]) and writes it as a scheme only, as
/// [`import_into`] does. Reads and writes files: call it off the UI thread.
pub fn import_scheme_into(
    name: &str,
    sources: &[PathBuf],
    dir: &Path,
) -> Result<ImportedFile, ImportError> {
    let imported = import_scheme_named(name, sources)?;
    let mut files = write_imported(vec![imported], dir)?;
    Ok(files.remove(0))
}

/// Writes each import as `<dir>/<id>.toml` (see [`import_into`]).
fn write_imported(imported: Vec<Imported>, dir: &Path) -> Result<Vec<ImportedFile>, ImportError> {
    let builtin: HashSet<String> = builtin::files().into_iter().map(|f| f.theme.id).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let io = |path: &Path, source| ImportError::Io {
        path: path.to_owned(),
        source,
    };
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let mut out = Vec::new();
    for item in imported {
        let mut file = item.theme.to_file();
        file.theme.scheme_only = item.report.kind == SourceKind::Scheme;
        // The theme it was imported on supplies roles added by later versions.
        file.theme.base = Some(item.report.base.clone());
        let mut id = file.theme.id.clone();
        if builtin.contains(&id) {
            id = format!("{id}-imported");
        }
        let base = id.clone();
        let mut n = 2;
        while !taken.insert(id.clone()) {
            id = format!("{base}-{n}");
            n += 1;
        }
        file.theme.id = id.clone();
        let target = dir.join(format!("{id}.toml"));
        let temp = dir.join(format!(".{id}.toml.tmp"));
        std::fs::write(&temp, file.to_toml()).map_err(|e| io(&temp, e))?;
        std::fs::rename(&temp, &target).map_err(|e| io(&target, e))?;
        out.push(ImportedFile {
            file,
            path: target,
            report: item.report,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roles::TextRole;

    fn ids(catalog: &Catalog) -> Vec<&str> {
        catalog.themes().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn builtin_themes_are_grouped_islands_first() {
        let catalog = Catalog::builtin();
        assert_eq!(
            ids(&catalog),
            [
                "islands-dark",
                "islands-light",
                "pyoxide-dark",
                "pyoxide-light"
            ]
        );
        let groups: Vec<ThemeGroup> = catalog.themes().map(|e| e.group).collect();
        assert_eq!(
            groups,
            [
                ThemeGroup::Islands,
                ThemeGroup::Islands,
                ThemeGroup::Bundled,
                ThemeGroup::Bundled
            ]
        );
        assert_eq!(catalog.get("islands-light").unwrap().kind, ThemeKind::Light);
        assert_eq!(ThemeGroup::Custom.title(), Some("Custom"));
        assert_eq!(ThemeGroup::Islands.title(), None);
    }

    fn custom(id: &str, name: &str, scheme_only: bool) -> ThemeFile {
        let mut file = builtin::theme(ThemeKind::Dark).to_file();
        file.theme.id = id.into();
        file.theme.name = name.into();
        file.theme.scheme_only = scheme_only;
        file
    }

    #[test]
    fn imported_themes_come_last_by_name_and_schemes_only_as_schemes() {
        let mut catalog = Catalog::builtin();
        catalog.add(custom("zeta", "Zeta", false), None).unwrap();
        catalog.add(custom("alpha", "Alpha", false), None).unwrap();
        catalog.add(custom("sky", "Sky", true), None).unwrap();
        assert_eq!(
            ids(&catalog),
            [
                "islands-dark",
                "islands-light",
                "pyoxide-dark",
                "pyoxide-light",
                "alpha",
                "zeta"
            ]
        );
        assert!(catalog.schemes().any(|e| e.id == "sky"));
        assert!(!catalog.is_theme("sky"));
        assert!(catalog.is_theme("alpha"));
        // A built-in id is refused.
        assert!(
            catalog
                .add(custom("islands-dark", "X", false), None)
                .is_err()
        );
        // Adding again replaces.
        catalog
            .add(custom("alpha", "Alpha 2", false), None)
            .unwrap();
        assert_eq!(catalog.get("alpha").unwrap().name, "Alpha 2");
        assert_eq!(catalog.themes().count(), 6);
    }

    #[test]
    fn a_scheme_replaces_the_editor_part_only() {
        let catalog = Catalog::builtin();
        let dark = catalog.resolve("islands-dark").unwrap();
        let light = catalog.resolve("islands-light").unwrap();
        let mixed = catalog
            .compose("islands-dark", Some("islands-light"))
            .unwrap();
        assert_eq!(mixed.id, "islands-dark");
        assert_eq!(mixed.ui[UiRole::Surface], dark.ui[UiRole::Surface]);
        assert_eq!(
            mixed.ui[UiRole::EditorBackground],
            light.ui[UiRole::EditorBackground]
        );
        assert_eq!(mixed.ui[UiRole::GitAdded], light.ui[UiRole::GitAdded]);
        assert_eq!(mixed.text[TextRole::Keyword], light.text[TextRole::Keyword]);
        assert_eq!(catalog.compose("islands-dark", None).unwrap(), dark);
        assert_eq!(
            catalog
                .compose("islands-dark", Some("islands-dark"))
                .unwrap(),
            dark
        );
        assert!(is_scheme_role(UiRole::Caret));
        assert!(!is_scheme_role(UiRole::Toolbar));
    }

    #[test]
    fn load_reads_the_folder_and_reports_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mine.toml"),
            custom("mine", "Mine", false).to_toml(),
        )
        .unwrap();
        std::fs::write(dir.path().join("broken.toml"), "[theme\n").unwrap();
        std::fs::write(
            dir.path().join("clash.toml"),
            custom("pyoxide-dark", "Clash", false).to_toml(),
        )
        .unwrap();
        let (catalog, problems) = Catalog::load(dir.path());
        assert!(catalog.is_theme("mine"));
        assert_eq!(
            catalog.get("mine").unwrap().path.as_deref(),
            Some(dir.path().join("mine.toml").as_path())
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(catalog.get("pyoxide-dark").unwrap().name, "Pyoxide Dark");
        // No folder: the built-in themes.
        let (catalog, problems) = Catalog::load(&dir.path().join("none"));
        assert_eq!(catalog.themes().count(), 4);
        assert!(problems.is_empty());
    }

    fn fixture(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(path)
    }

    #[test]
    fn import_writes_theme_files_that_load_again() {
        let dir = tempfile::tempdir().unwrap();
        let themes = dir.path().join("themes");
        let files = import_into(&fixture("plugin"), &themes).unwrap();
        let theme = files.iter().find(|f| !f.file.theme.scheme_only).unwrap();
        assert!(theme.path.starts_with(&themes));
        assert!(theme.path.is_file());
        let scheme = import_into(&fixture("schemes/Skyline.icls"), &themes).unwrap();
        assert_eq!(scheme.len(), 1);
        assert!(scheme[0].file.theme.scheme_only);
        let (catalog, problems) = Catalog::load(&themes);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(catalog.is_theme(&theme.file.theme.id));
        assert_eq!(
            catalog.get(&theme.file.theme.id).unwrap().group,
            ThemeGroup::Custom
        );
        let sky = &scheme[0].file.theme.id;
        assert!(catalog.get(sky).is_some_and(|e| e.scheme_only));
        // Importing again replaces the file of the same id.
        let again = import_into(&fixture("schemes/Skyline.icls"), &themes).unwrap();
        assert_eq!(&again[0].file.theme.id, sky);
        // Not a theme: an error, nothing written.
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        assert!(import_into(&text, &themes).is_err());
        let count = std::fs::read_dir(&themes).unwrap().count();
        assert_eq!(count, files.len() + 1);
    }
}
