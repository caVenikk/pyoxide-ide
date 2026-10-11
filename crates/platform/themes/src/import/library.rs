//! A collection of theme data files (UI themes and colour schemes) that
//! import runs against. Files are added from paths, archives or text; themes
//! find their parents and their colour scheme among the collected files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::color::Rgba;

use super::archive::{self, ArchiveSummary, DataKind};
use super::convert::{Imported, Inputs, convert};
use super::known::{known_chain, known_scheme, known_theme};
use super::report::{Level, SourceKind, Warning, WarningKind};
use super::scheme::{Scheme, SchemeChain};
use super::ui_theme::UiThemeData;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{origin}: {message}")]
    Parse { origin: String, message: String },
    #[error("{origin}: {message}")]
    Archive { origin: String, message: String },
    #[error(
        "{0}: not a theme file (expected .theme.json, .json, .icls, .xml, .jar, .zip or a directory)"
    )]
    Unsupported(String),
    #[error("{0}: no UI theme or colour scheme found")]
    Empty(String),
}

/// A file or entry that was looked at but not used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug)]
struct Stored<T> {
    data: T,
    /// Primary entries are imported; the others only supply parents.
    primary: bool,
}

#[derive(Debug, Default)]
pub struct Library {
    themes: Vec<Stored<UiThemeData>>,
    schemes: Vec<Stored<Scheme>>,
    skipped: Vec<Skipped>,
    archives: Vec<ArchiveSummary>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a UI theme file, a colour scheme file, a plugin archive or a
    /// directory. `primary` entries are imported by [`Library::import_all`];
    /// the others only provide parent themes and parent schemes.
    pub fn add_path(&mut self, path: &Path, primary: bool) -> Result<(), ImportError> {
        let io = |source| ImportError::Io {
            path: path.to_owned(),
            source,
        };
        let meta = std::fs::metadata(path).map_err(io)?;
        let display = path.display().to_string();
        if meta.is_dir() {
            let archives = archive::add_dir(self, path, primary)?;
            self.archives.extend(archives);
            return Ok(());
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".jar") || lower.ends_with(".zip") {
            let summary = archive::add_archive(self, path, primary)?;
            self.archives.push(summary);
            return Ok(());
        }
        let kind = if lower.ends_with(".icls") || lower.ends_with(".xml") {
            DataKind::Scheme
        } else if lower.ends_with(".json") {
            DataKind::UiTheme
        } else {
            return Err(ImportError::Unsupported(display));
        };
        let text = std::fs::read_to_string(path).map_err(io)?;
        match kind {
            DataKind::Scheme => {
                self.add_scheme_xml(&text, &display, &name, primary)?;
            }
            DataKind::UiTheme => {
                let first = self.themes.len();
                self.add_ui_theme_json(&text, &display, &name, primary)?;
                // The scheme file usually sits next to the theme.
                for reference in self.unresolved_scheme_references(first) {
                    if let Some(found) = find_on_disk(path, &reference)
                        && let Some(text) = archive::read_file(&found, self)
                    {
                        let entry = archive::normalize(&reference).unwrap_or(reference);
                        self.add_text(
                            DataKind::Scheme,
                            &text,
                            &found.display().to_string(),
                            &entry,
                            primary,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Adds a `.theme.json` given as text. `entry` is its path inside the
    /// plugin (or just a file name).
    pub fn add_ui_theme_json(
        &mut self,
        text: &str,
        origin: &str,
        entry: &str,
        primary: bool,
    ) -> Result<(), ImportError> {
        let data =
            UiThemeData::parse(text, origin, entry).map_err(|message| ImportError::Parse {
                origin: origin.to_owned(),
                message,
            })?;
        self.themes.push(Stored { data, primary });
        Ok(())
    }

    /// Adds the schemes of an `.icls` or scheme XML given as text. Returns
    /// how many schemes the document holds.
    pub fn add_scheme_xml(
        &mut self,
        text: &str,
        origin: &str,
        entry: &str,
        primary: bool,
    ) -> Result<usize, ImportError> {
        let schemes = Scheme::parse(text, origin, entry).map_err(|message| ImportError::Parse {
            origin: origin.to_owned(),
            message,
        })?;
        let count = schemes.len();
        self.schemes
            .extend(schemes.into_iter().map(|data| Stored { data, primary }));
        Ok(count)
    }

    pub fn themes(&self) -> impl Iterator<Item = &UiThemeData> {
        self.themes.iter().map(|s| &s.data)
    }

    pub fn schemes(&self) -> impl Iterator<Item = &Scheme> {
        self.schemes.iter().map(|s| &s.data)
    }

    /// Archives that were opened, with the entries read from each.
    pub fn archives(&self) -> &[ArchiveSummary] {
        &self.archives
    }

    /// Files and entries that looked like theme data but could not be used.
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    pub(crate) fn theme_count(&self) -> usize {
        self.themes.len()
    }

    pub(crate) fn note_skipped(&mut self, path: &str, reason: &str) {
        self.skipped.push(Skipped {
            path: path.to_owned(),
            reason: reason.to_owned(),
        });
    }

    /// Adds text that was chosen by name; a file that does not parse as the
    /// expected kind (for example a JSON that is not a theme) is noted as
    /// skipped instead of failing the whole run.
    pub(crate) fn add_text(
        &mut self,
        kind: DataKind,
        text: &str,
        origin: &str,
        entry: &str,
        primary: bool,
    ) {
        let result = match kind {
            DataKind::UiTheme => self.add_ui_theme_json(text, origin, entry, primary),
            DataKind::Scheme => self
                .add_scheme_xml(text, origin, entry, primary)
                .map(|_| ()),
        };
        if let Err(e) = result {
            let reason = match e {
                ImportError::Parse { message, .. } => message,
                other => other.to_string(),
            };
            self.note_skipped(origin, &reason);
        }
    }

    /// Scheme references of themes from index `first` on that name a path but
    /// find no scheme in the library.
    pub(crate) fn unresolved_scheme_references(&self, first: usize) -> Vec<String> {
        let mut out = Vec::new();
        for stored in &self.themes[first.min(self.themes.len())..] {
            let theme = &stored.data;
            let Some(reference) = theme.editor_scheme.as_deref() else {
                continue;
            };
            if looks_like_path(reference)
                && self.find_scheme_ref(reference, &theme.origin).is_none()
                && !out.iter().any(|r| r == reference)
            {
                out.push(reference.to_owned());
            }
        }
        out
    }

    // ---- lookups ----------------------------------------------------

    fn find_theme(&self, id: &str, near: &str) -> Option<usize> {
        let known = known_theme(id);
        let stem = |t: &UiThemeData| {
            let file = t.entry_path.rsplit('/').next().unwrap_or(&t.entry_path);
            file.split('.').next().unwrap_or(file).to_ascii_lowercase()
        };
        let candidates: Vec<usize> = (0..self.themes.len())
            .filter(|&i| {
                let t = &self.themes[i].data;
                known.is_some_and(|k| stem(t) == k.file_stem.to_ascii_lowercase())
                    || t.name == id
                    || stem(t) == id.to_ascii_lowercase()
            })
            .collect();
        pick_nearest(&candidates, near, |i| &self.themes[i].data.origin)
    }

    /// Finds a scheme by the `editorScheme` value of a theme: a path inside
    /// the plugin or a scheme name.
    fn find_scheme_ref(&self, reference: &str, near: &str) -> Option<usize> {
        let candidates: Vec<usize> = if looks_like_path(reference) {
            let wanted = archive::normalize(reference)?;
            let base = wanted.rsplit('/').next().unwrap_or(&wanted).to_owned();
            let exact: Vec<usize> = (0..self.schemes.len())
                .filter(|&i| {
                    let path = &self.schemes[i].data.entry_path;
                    *path == wanted || path.ends_with(&format!("/{wanted}"))
                })
                .collect();
            if exact.is_empty() {
                (0..self.schemes.len())
                    .filter(|&i| {
                        let path = &self.schemes[i].data.entry_path;
                        path.rsplit('/').next().unwrap_or(path) == base
                    })
                    .collect()
            } else {
                exact
            }
        } else {
            (0..self.schemes.len())
                .filter(|&i| {
                    let s = &self.schemes[i].data;
                    s.name == reference && !s.additional
                })
                .collect()
        };
        pick_nearest(&candidates, near, |i| &self.schemes[i].data.origin)
    }

    fn find_scheme_by_name(&self, name: &str, near: &str) -> Option<usize> {
        let candidates: Vec<usize> = (0..self.schemes.len())
            .filter(|&i| {
                let s = &self.schemes[i].data;
                s.name == name && !s.additional
            })
            .collect();
        pick_nearest(&candidates, near, |i| &self.schemes[i].data.origin)
    }

    // ---- chains -----------------------------------------------------

    /// The theme and the ancestors that are available (nearest first), notes
    /// about the ones that are not, and the `dark` flag the table knows for
    /// the first missing ancestor. A missing ancestor does not end the walk:
    /// the table of known themes names its parent, which may be available.
    fn ui_chain<'a>(
        &'a self,
        start: &'a UiThemeData,
    ) -> (Vec<&'a UiThemeData>, Vec<Warning>, Option<bool>) {
        let mut layers = vec![start];
        let mut warnings = Vec::new();
        let mut known_dark = None;
        let mut noted: Vec<&'static str> = Vec::new();
        let mut near = start.origin.clone();
        let mut next_id = start.parent.clone();
        while let Some(id) = next_id.take() {
            if let Some(index) = self.find_theme(&id, &near) {
                let parent = &self.themes[index].data;
                if layers.iter().any(|l| std::ptr::eq(*l, parent)) {
                    warnings.push(Warning {
                        level: Level::Warning,
                        kind: WarningKind::InheritanceCycle { id: id.clone() },
                        message: format!(
                            "parentTheme `{id}` leads back to a theme already in the chain"
                        ),
                    });
                    break;
                }
                near = parent.origin.clone();
                next_id = parent.parent.clone();
                layers.push(parent);
                continue;
            }
            let Some(known) = known_theme(&id) else {
                warnings.push(Warning {
                    level: Level::Warning,
                    kind: WarningKind::UnknownParentTheme { id: id.clone() },
                    message: format!(
                        "parentTheme `{id}` is not a known theme id and no available theme matches it"
                    ),
                });
                break;
            };
            known_dark.get_or_insert(known.dark);
            if !noted.contains(&known.id) {
                let chain = known_chain(&id);
                noted.extend(chain.iter().copied());
                let chain: Vec<String> = chain.into_iter().map(str::to_owned).collect();
                warnings.push(Warning {
                    level: Level::Info,
                    kind: WarningKind::ParentThemeUnavailable {
                        id: id.clone(),
                        chain: chain.clone(),
                    },
                    message: format!(
                        "parent theme `{id}` is not among the inputs (known chain: {}); its values are not imported",
                        chain.join(" -> ")
                    ),
                });
            }
            next_id = known.parent.map(str::to_owned);
        }
        (layers, warnings, known_dark)
    }

    /// Additional attributes that languages add to the base scheme `name`.
    fn additional_for<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Scheme> + 'a {
        let name = name.to_owned();
        self.schemes
            .iter()
            .map(|s| &s.data)
            .filter(move |s| s.additional && s.name == name)
    }

    fn scheme_chain<'a>(&'a self, start: &'a Scheme) -> (Vec<&'a Scheme>, Vec<Warning>) {
        let mut layers = vec![start];
        layers.extend(self.additional_for(&start.name));
        let mut warnings = Vec::new();
        let mut current = start;
        while let Some(parent) = current.parent.as_deref() {
            match self.find_scheme_by_name(parent, &current.origin) {
                Some(index) => {
                    let next = &self.schemes[index].data;
                    if layers.iter().any(|l| std::ptr::eq(*l, next)) {
                        warnings.push(Warning {
                            level: Level::Warning,
                            kind: WarningKind::InheritanceCycle {
                                id: parent.to_owned(),
                            },
                            message: format!("parent_scheme `{parent}` leads back to a scheme already in the chain"),
                        });
                        break;
                    }
                    layers.push(next);
                    layers.extend(self.additional_for(&next.name));
                    current = next;
                }
                None => {
                    // Language attributes for a base scheme that is not
                    // among the inputs still apply.
                    layers.extend(self.additional_for(parent));
                    let known = known_scheme(parent).is_some();
                    warnings.push(Warning {
                        level: if known { Level::Info } else { Level::Warning },
                        kind: WarningKind::ParentSchemeUnavailable {
                            name: parent.to_owned(),
                            known,
                        },
                        message: if known {
                            format!(
                                "parent scheme `{parent}` is a base scheme of the IDE and is not among the inputs; values it would supply are filled from the built-in theme"
                            )
                        } else {
                            format!("parent_scheme `{parent}` is not among the inputs")
                        },
                    });
                    break;
                }
            }
        }
        (layers, warnings)
    }

    // ---- import -----------------------------------------------------

    /// Imports a UI theme of this library together with its available
    /// parents and its editor colour scheme.
    pub fn import_ui_theme(&self, theme: &UiThemeData) -> Imported {
        self.import_ui_theme_with_scheme(theme).0
    }

    fn import_ui_theme_with_scheme(&self, theme: &UiThemeData) -> (Imported, Option<usize>) {
        let (ui_layers, mut warnings, known_dark) = self.ui_chain(theme);
        let dark_hint = ui_layers.iter().find_map(|l| l.dark).or(known_dark);
        let mut reference = ui_layers.iter().find_map(|l| {
            l.editor_scheme
                .as_deref()
                .map(|r| (r.to_owned(), l.origin.clone()))
        });
        // A theme that names no scheme gets the IDE's default scheme for its
        // brightness, when that scheme is among the inputs.
        let mut assumed = false;
        if reference.is_none()
            && let Some(dark) = dark_hint
        {
            reference = Some((
                if dark { "Darcula" } else { "Default" }.to_owned(),
                theme.origin.clone(),
            ));
            assumed = true;
        }
        let mut scheme_index = None;
        let mut scheme_layers = Vec::new();
        if let Some((reference, near)) = reference {
            match self.find_scheme_ref(&reference, &near) {
                Some(index) => {
                    scheme_index = Some(index);
                    let (layers, more) = self.scheme_chain(&self.schemes[index].data);
                    scheme_layers = layers;
                    if assumed {
                        warnings.push(Warning {
                            level: Level::Info,
                            kind: WarningKind::SchemeAssumed {
                                name: reference.clone(),
                            },
                            message: format!(
                                "the theme names no editor scheme; the default scheme `{reference}` is used"
                            ),
                        });
                    }
                    warnings.extend(more);
                }
                None if assumed => {}
                None => warnings.push(Warning {
                    level: Level::Warning,
                    kind: WarningKind::SchemeNotFound {
                        reference: reference.clone(),
                    },
                    message: format!("editorScheme `{reference}` is not among the inputs"),
                }),
            }
        }
        let dark = dark_hint
            .or_else(|| scheme_is_dark(&scheme_layers))
            .unwrap_or(true);
        let mut sources: Vec<String> = Vec::new();
        for origin in ui_layers
            .iter()
            .map(|l| &l.origin)
            .chain(scheme_layers.iter().map(|s| &s.origin))
        {
            if !sources.contains(origin) {
                sources.push(origin.clone());
            }
        }
        let imported = convert(Inputs {
            name: theme.name.clone(),
            kind: SourceKind::UiTheme,
            author: ui_layers.iter().find_map(|l| l.author.clone()),
            dark,
            ui_layers,
            scheme_layers,
            sources,
            warnings,
        });
        (imported, scheme_index)
    }

    /// Imports a colour scheme on its own, with its available parents.
    pub fn import_scheme(&self, scheme: &Scheme) -> Imported {
        let (scheme_layers, warnings) = self.scheme_chain(scheme);
        let dark = scheme_is_dark(&scheme_layers).unwrap_or(true);
        let sources =
            scheme_layers
                .iter()
                .map(|s| s.origin.clone())
                .fold(Vec::new(), |mut acc, origin| {
                    if !acc.contains(&origin) {
                        acc.push(origin);
                    }
                    acc
                });
        convert(Inputs {
            name: scheme.name.clone(),
            kind: SourceKind::Scheme,
            author: None,
            dark,
            ui_layers: Vec::new(),
            scheme_layers,
            sources,
            warnings,
        })
    }

    /// Imports every primary entry: each UI theme with the scheme it names,
    /// and each primary scheme that no imported theme used.
    pub fn import_all(&self) -> Vec<Imported> {
        let mut out = Vec::new();
        let mut used_schemes: HashSet<usize> = HashSet::new();
        for stored in self.themes.iter().filter(|t| t.primary) {
            let (imported, scheme) = self.import_ui_theme_with_scheme(&stored.data);
            used_schemes.extend(scheme);
            out.push(imported);
        }
        for (index, stored) in self.schemes.iter().enumerate() {
            if stored.primary && !stored.data.additional && !used_schemes.contains(&index) {
                out.push(self.import_scheme(&stored.data));
            }
        }
        out
    }
}

/// Imports a file, archive or directory. `context` paths are searched for
/// parent themes and schemes but are not imported themselves.
pub fn import_path(path: &Path, context: &[PathBuf]) -> Result<Vec<Imported>, ImportError> {
    let mut library = Library::new();
    for extra in context {
        library.add_path(extra, false)?;
    }
    library.add_path(path, true)?;
    let imported = library.import_all();
    if imported.is_empty() {
        return Err(ImportError::Empty(path.display().to_string()));
    }
    Ok(imported)
}

fn looks_like_path(reference: &str) -> bool {
    let lower = reference.to_ascii_lowercase();
    reference.contains('/')
        || reference.contains('\\')
        || lower.ends_with(".xml")
        || lower.ends_with(".icls")
}

/// Among `candidates` the one that sits in the same archive or folder as
/// `near`, else the first.
fn pick_nearest<'a>(
    candidates: &[usize],
    near: &str,
    origin_of: impl Fn(usize) -> &'a str,
) -> Option<usize> {
    let container = container_of(near);
    candidates
        .iter()
        .copied()
        .find(|&c| container_of(origin_of(c)) == container)
        .or_else(|| candidates.first().copied())
}

/// The archive part of `archive!entry`, else the folder of a file path.
fn container_of(origin: &str) -> &str {
    if let Some((archive, _)) = origin.split_once('!') {
        return archive;
    }
    origin.rfind(['/', '\\']).map_or("", |end| &origin[..end])
}

/// Looks for the scheme file a theme file refers to, beside the theme file or
/// at the reference path below one of its parent folders.
fn find_on_disk(theme_file: &Path, reference: &str) -> Option<PathBuf> {
    let relative = archive::normalize(reference)?;
    if !archive::is_scheme_reference(&relative) {
        return None;
    }
    let base = relative.rsplit('/').next()?.to_owned();
    let dir = theme_file.parent()?;
    let beside = dir.join(&base);
    if beside.is_file() {
        return Some(beside);
    }
    dir.ancestors()
        .map(|a| a.join(&relative))
        .find(|p| p.is_file())
}

/// Whether the scheme paints a dark editor background, from its `TEXT`
/// background or the console background, else from the names of its chain.
fn scheme_is_dark(layers: &[&Scheme]) -> Option<bool> {
    if layers.is_empty() {
        return None;
    }
    let chain = SchemeChain {
        layers: layers.to_vec(),
    };
    let background = chain
        .attribute("TEXT")
        .and_then(|a| a.background)
        .or_else(|| chain.color("CONSOLE_BACKGROUND_KEY").map(str::to_owned))
        .or_else(|| chain.color("GUTTER_BACKGROUND").map(str::to_owned))
        .and_then(|raw| Rgba::parse_scheme(&raw).ok());
    if let Some(color) = background {
        return Some(color.luminance() < 0.18);
    }
    for layer in layers {
        for name in [Some(layer.name.as_str()), layer.parent.as_deref()]
            .into_iter()
            .flatten()
        {
            if let Some(known) = known_scheme(name) {
                return Some(known.dark);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_prefers_the_same_container() {
        let origins = ["a.jar!x", "b.jar!y", "b.jar!z"];
        let pick = |near: &str| pick_nearest(&[0, 1, 2], near, |i| origins[i]);
        assert_eq!(pick("b.jar!theme"), Some(1));
        assert_eq!(pick("c.jar!theme"), Some(0));
        assert_eq!(container_of("/dir/sub/file.json"), "/dir/sub");
        assert_eq!(container_of("file.json"), "");
    }

    #[test]
    fn references_are_paths_or_names() {
        assert!(looks_like_path("/themes/A.xml"));
        assert!(looks_like_path("A.icls"));
        assert!(!looks_like_path("Darcula"));
    }
}
