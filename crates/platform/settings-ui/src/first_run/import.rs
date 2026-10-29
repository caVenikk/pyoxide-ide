//! The import of a configuration folder: what importers are, the ones of
//! the platform's settings (theme and editor colour scheme, editor font,
//! soft wraps, autosave, file type associations), the snapshot the import
//! keeps, and the balloon that says what came in.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::App;
use settings::{
    AppearanceSettings, EditorFontSettings, FileTypeAssociation, FileTypeSettings, ImportSettings,
    SaveSettings, SoftWrapSettings, SyncSettings, TerminalFontSettings,
};
use settings_import::pycharm::options::{self, ConfigFolder, ReferenceOptions, ThemePlugin};
use settings_import::snapshot::{self, Snapshot};
use themes::ThemeKind;
use themes::catalog::ImportedFile;
use ui::notification::{Notification, NotificationKind};

use crate::{store, theme_switch};

/// What importers read: the folder imported from, the snapshot of its
/// data files and the options Pyoxide imports, read once from it.
#[derive(Debug, Clone)]
pub struct Source {
    /// The folder imported from: its name and version, and its plugins
    /// (theme plugins are read there, not copied).
    pub folder: ConfigFolder,
    /// The data files to read: the import's snapshot (the folder itself
    /// when no snapshot could be kept). Importers read option, keymap and
    /// scheme files here, never in [`ConfigFolder::path`].
    pub snapshot: Snapshot,
    pub options: ReferenceOptions,
    /// Pyoxide's configuration folder (where `settings.toml` is), for the
    /// files an importer writes (`themes/`, `keymaps/`, `interpreters.toml`).
    pub config_dir: Option<PathBuf>,
    /// The import is made for settings that already exist (a configuration
    /// older than the first-run window, [`super::adopt_existing`]): an
    /// importer that honours this sets only what the settings do not hold
    /// yet.
    pub fill_only: bool,
}

/// What an import brings, said in the balloon: parts of a sentence ("the
/// theme Monokai Classic", "2 interpreters") and whole sentences about
/// what could not be imported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub imported: Vec<String>,
    pub problems: Vec<String>,
}

impl Outcome {
    pub fn imported(part: impl Into<String>) -> Self {
        Self {
            imported: vec![part.into()],
            problems: Vec::new(),
        }
    }

    pub fn problem(sentence: impl Into<String>) -> Self {
        Self {
            imported: Vec::new(),
            problems: vec![sentence.into()],
        }
    }

    fn extend(&mut self, other: Outcome) {
        self.imported.extend(other.imported);
        self.problems.extend(other.problems);
    }
}

/// What an importer applies on the UI thread once its files are read
/// and written.
pub type Applied = Box<dyn FnOnce(&mut App) -> Outcome + Send>;

/// An area's part of the import: `work` runs off the UI thread (it may
/// read the reference's files and write Pyoxide's), its result runs on it.
#[derive(Clone, Copy)]
pub struct Importer {
    pub id: &'static str,
    pub work: fn(&Source) -> Applied,
}

/// Nothing to apply.
pub fn nothing() -> Applied {
    Box::new(|_| Outcome::default())
}

pub(super) fn platform_importers() -> Vec<Importer> {
    vec![
        Importer {
            id: "appearance",
            work: appearance,
        },
        Importer {
            id: "editor-font",
            work: editor_font,
        },
        Importer {
            id: "terminal-font",
            work: terminal_font,
        },
        Importer {
            id: "soft-wraps",
            work: soft_wraps,
        },
        Importer {
            id: "autosave",
            work: autosave,
        },
        Importer {
            id: "file-types",
            work: file_types,
        },
    ]
}

// ---- theme and editor colour scheme ---------------------------------------------

/// The reference's default theme (`laf.xml` without a theme).
const DEFAULT_THEME: &str = "Islands Dark";
/// The prefix of an editable copy of a scheme (`Scheme.EDITABLE_COPY_PREFIX`).
const USER_COPY: &str = "_@user_";

/// A theme of the reference as Pyoxide has it.
enum Chosen {
    /// A built-in theme; `approximates` names the reference's theme when it
    /// is not the same.
    Builtin {
        id: &'static str,
        approximates: Option<String>,
    },
    /// Imported from a plugin of the reference.
    Imported(Box<ImportedFile>),
}

/// The built-in theme for one of the reference's bundled themes, and
/// whether it is that theme (`Islands Dark`) or stands in for it.
fn builtin_theme(id: &str) -> Option<(&'static str, bool)> {
    let known = themes::import::KNOWN_THEMES.iter().find(|t| t.id == id)?;
    let kind = if known.dark {
        ThemeKind::Dark
    } else {
        ThemeKind::Light
    };
    let exact = matches!(id, "Islands Dark" | "Islands Light");
    Some((themes::builtin::default_id_for(kind), exact))
}

/// The reference's bundled editor colour schemes, dark or not.
fn builtin_scheme(name: &str) -> Option<bool> {
    match name {
        "Islands Dark" | "Dark" | "Darcula" | "Darcula Contrast" | "High contrast" => Some(true),
        "Islands Light" | "Light" | "Default" | "IntelliJ Light" | "Classic Light" => Some(false),
        _ => None,
    }
}

fn appearance(source: &Source) -> Applied {
    let Some(dir) = source.config_dir.as_ref().map(|d| d.join("themes")) else {
        return nothing();
    };
    let mut outcome = Outcome::default();
    let wanted = source
        .options
        .theme
        .clone()
        .unwrap_or_else(|| DEFAULT_THEME.to_owned());
    let mut plugins: Option<Vec<ThemePlugin>> = None;
    let mut plugins = || -> Vec<ThemePlugin> {
        plugins
            .get_or_insert_with(|| options::theme_plugins(&source.folder.plugins))
            .clone()
    };
    let theme = match builtin_theme(&wanted) {
        Some((id, exact)) => Some(Chosen::Builtin {
            id,
            approximates: (!exact).then(|| wanted.clone()),
        }),
        None => {
            let found = plugins();
            match options::theme_provider(&found, &wanted) {
                Some((archive, path)) => {
                    let context: Vec<PathBuf> = found.iter().map(|p| p.archive.clone()).collect();
                    match themes::catalog::import_plugin_theme_into(archive, path, &context, &dir) {
                        Ok(file) => Some(Chosen::Imported(Box::new(file))),
                        Err(e) => {
                            outcome
                                .problems
                                .push(format!("The theme could not be read: {e}."));
                            None
                        }
                    }
                }
                None => {
                    outcome.problems.push(
                        "The theme comes from a plugin that is not installed; the default theme is kept."
                            .to_owned(),
                    );
                    None
                }
            }
        }
    };
    // The scheme: the theme's own, a bundled one, a copy of one's own in
    // `colors/`, or one a plugin brings.
    let mut scheme_file = None;
    let mut scheme_builtin = None;
    if let Some(name) = source.options.scheme.clone() {
        let theme_name = match &theme {
            Some(Chosen::Imported(file)) => Some(file.file.theme.name.clone()),
            _ => None,
        };
        let theme_dark = match &theme {
            Some(Chosen::Builtin { id, .. }) => {
                Some(*id == themes::builtin::default_id_for(ThemeKind::Dark))
            }
            _ => None,
        };
        let own = options::scheme_file(source.snapshot.dir(), &name);
        if let Some(file) = own {
            let mut sources = vec![file];
            sources.extend(plugins().into_iter().map(|p| p.archive));
            match themes::catalog::import_scheme_into(&name, &sources, &dir) {
                Ok(file) => scheme_file = Some(file),
                Err(e) => outcome
                    .problems
                    .push(format!("The editor color scheme could not be read: {e}.")),
            }
        } else if let Some(dark) = builtin_scheme(&name) {
            if theme_dark != Some(dark) {
                let kind = if dark {
                    ThemeKind::Dark
                } else {
                    ThemeKind::Light
                };
                scheme_builtin = Some(themes::builtin::default_id_for(kind));
            }
        } else if theme_name.as_deref() != Some(name.as_str())
            && theme_name.as_deref() != name.strip_prefix(USER_COPY)
        {
            let sources: Vec<PathBuf> = plugins().into_iter().map(|p| p.archive).collect();
            match themes::catalog::import_scheme_into(&name, &sources, &dir) {
                Ok(file) => scheme_file = Some(file),
                Err(_) => outcome.problems.push(format!(
                    "The editor color scheme {name} was not found; the theme's own is used."
                )),
            }
        }
    }
    Box::new(move |cx| {
        let mut files = Vec::new();
        let theme_id = match theme {
            Some(Chosen::Builtin { id, approximates }) => {
                if let Some(reference) = approximates {
                    let name = theme_switch::catalog(cx)
                        .get(id)
                        .map(|e| e.name.clone())
                        .unwrap_or_else(|| id.to_owned());
                    let reference = themes::import::KNOWN_THEMES
                        .iter()
                        .find(|k| k.id == reference)
                        .map(|k| k.name.to_owned())
                        .unwrap_or(reference);
                    outcome.problems.push(format!(
                        "{} has no {reference} theme; {name} is used.",
                        app_info::name()
                    ));
                }
                Some(id.to_owned())
            }
            Some(Chosen::Imported(file)) => {
                files.push(*file);
                None
            }
            None => None,
        };
        let scheme_name = scheme_file.as_ref().map(|f| f.file.theme.name.clone());
        let had_scheme_file = scheme_file.is_some();
        files.extend(scheme_file);
        let (themes_added, schemes_added) = theme_switch::add_imported(files, cx);
        let theme_id = theme_id.or_else(|| themes_added.first().cloned());
        let scheme_id = schemes_added
            .first()
            .cloned()
            .filter(|_| had_scheme_file)
            .or(scheme_builtin.map(str::to_owned));
        let Some(theme_id) = theme_id else {
            return outcome;
        };
        let before: AppearanceSettings = store::get(cx);
        match theme_switch::set_theme_and_scheme(&theme_id, scheme_id.as_deref(), cx) {
            Ok(()) => {
                let name = theme_switch::catalog(cx)
                    .get(&theme_id)
                    .map(|e| e.name.clone())
                    .unwrap_or(theme_id);
                let after: AppearanceSettings = store::get(cx);
                if after != before || scheme_name.is_some() {
                    outcome.imported.insert(0, format!("the theme {name}"));
                }
                if let Some(scheme) = scheme_name {
                    outcome
                        .imported
                        .insert(1, format!("the editor color scheme {scheme}"));
                }
            }
            Err(e) => outcome.problems.push(format!("{e}.")),
        }
        outcome
    })
}

// ---- editor ------------------------------------------------------------------------

fn editor_font(source: &Source) -> Applied {
    let font = source.options.font.clone();
    let fill = source.fill_only;
    if font.is_empty() {
        return nothing();
    }
    Box::new(move |cx| {
        let before: EditorFontSettings = store::get(cx);
        let mut settings = before.clone();
        put(&mut settings.family, font.family, fill);
        put(
            &mut settings.size,
            font.size.map(EditorFontSettings::clamp_size),
            fill,
        );
        put(
            &mut settings.line_height,
            font.line_spacing.map(EditorFontSettings::clamp_line_height),
            fill,
        );
        put(&mut settings.ligatures, font.ligatures, fill);
        put(&mut settings.fallback, font.fallback, fill);
        if fill && settings == before {
            return Outcome::default();
        }
        store::set(cx, &settings);
        let family = settings
            .family
            .clone()
            .unwrap_or_else(|| EditorFontSettings::DEFAULT_FAMILY.to_owned());
        let size = settings.size.unwrap_or(EditorFontSettings::DEFAULT_SIZE);
        let mut parts = vec![family, format_number(size)];
        if settings.ligatures == Some(true) {
            parts.push("ligatures".to_owned());
        }
        Outcome::imported(format!("the editor font ({})", parts.join(", ")))
    })
}

/// Sets `slot` to the imported `value`; with `fill_only` only a slot that
/// holds nothing.
fn put<T>(slot: &mut Option<T>, value: Option<T>, fill_only: bool) {
    if value.is_some() && (!fill_only || slot.is_none()) {
        *slot = value;
    }
}

/// `options/terminal-font.xml`: a user who set a font for the terminal (a
/// Nerd Font for the prompt) keeps it.
fn terminal_font(source: &Source) -> Applied {
    let options = source.options.terminal_font.clone();
    let fill = source.fill_only;
    if options.is_empty() {
        return nothing();
    }
    Box::new(move |cx| {
        let font = options.font;
        let before: TerminalFontSettings = store::get(cx);
        let mut settings = before.clone();
        put(&mut settings.family, font.family, fill);
        put(
            &mut settings.size,
            font.size.map(TerminalFontSettings::clamp_size),
            fill,
        );
        put(
            &mut settings.line_spacing,
            font.line_spacing.map(TerminalFontSettings::clamp_spacing),
            fill,
        );
        put(
            &mut settings.column_spacing,
            options
                .column_spacing
                .map(TerminalFontSettings::clamp_spacing),
            fill,
        );
        put(&mut settings.fallback, font.fallback, fill);
        if fill && settings == before {
            return Outcome::default();
        }
        store::set(cx, &settings);
        let mut parts = Vec::new();
        parts.extend(settings.family.clone());
        parts.extend(settings.size.map(format_number));
        Outcome::imported(format!("the terminal font ({})", parts.join(", ")))
    })
}

/// `14`, `14.5`.
fn format_number(n: f32) -> String {
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

fn soft_wraps(source: &Source) -> Applied {
    let Some(wraps) = source.options.soft_wraps.clone() else {
        return nothing();
    };
    Box::new(move |cx| {
        let mut settings: SoftWrapSettings = store::get(cx);
        settings.enabled = wraps.enabled;
        settings.files = wraps.masks;
        if let Some(original) = wraps.use_original_indent {
            settings.use_original_indent = original;
        }
        if let Some(indent) = wraps.indent {
            settings.indent = indent.min(*SoftWrapSettings::INDENTS.end());
        }
        if let Some(all) = wraps.show_all {
            settings.show_all = all;
        }
        store::set(cx, &settings);
        let part = if settings.enabled {
            format!("soft wraps (on for {})", settings.masks())
        } else {
            "soft wraps (off)".to_owned()
        };
        Outcome::imported(part)
    })
}

// ---- system ------------------------------------------------------------------------

fn autosave(source: &Source) -> Applied {
    let general = source.options.general;
    if general.is_empty() {
        return nothing();
    }
    Box::new(move |cx| {
        let mut save: SaveSettings = store::get(cx);
        if let Some(on) = general.save_on_deactivation {
            save.save_on_deactivation = on;
        }
        if let Some(on) = general.save_on_idle {
            save.save_on_idle = on;
        }
        if let Some(seconds) = general.idle_seconds {
            save.idle_seconds = seconds.clamp(
                *SaveSettings::IDLE_SECONDS.start(),
                *SaveSettings::IDLE_SECONDS.end(),
            );
        }
        store::set(cx, &save);
        if let Some(on) = general.sync_on_activation {
            let sync = SyncSettings {
                sync_on_activation: on,
                ..store::get(cx)
            };
            store::set(cx, &sync);
        }
        Outcome::imported("autosave and file synchronization")
    })
}

fn file_types(source: &Source) -> Applied {
    let convert = |list: &[options::FileTypeMapping]| -> Vec<FileTypeAssociation> {
        list.iter()
            .map(|m| FileTypeAssociation {
                pattern: m.pattern.clone(),
                file_type: m.file_type.clone(),
            })
            .collect()
    };
    let settings = FileTypeSettings {
        associations: convert(&source.options.file_types),
        removed: convert(&source.options.removed_file_types),
    };
    if settings == FileTypeSettings::default() {
        return nothing();
    }
    Box::new(move |cx| {
        let count = settings.associations.len() + settings.removed.len();
        store::set(cx, &settings);
        Outcome::imported(if count == 1 {
            "1 file type association".to_owned()
        } else {
            format!("{count} file type associations")
        })
    })
}

// ---- the balloon -------------------------------------------------------------------

/// The balloon of a finished import.
pub(super) fn notification(folder: &ConfigFolder, outcome: &Outcome) -> Notification {
    let mut text = if outcome.imported.is_empty() {
        format!(
            "{} has no settings {} imports; the defaults are kept.",
            folder.label(),
            app_info::name()
        )
    } else {
        format!(
            "From {}: {}.",
            folder.label(),
            join_parts(&outcome.imported)
        )
    };
    for problem in &outcome.problems {
        text.push(' ');
        text.push_str(problem);
    }
    let kind = if outcome.problems.is_empty() {
        NotificationKind::Information
    } else {
        NotificationKind::Warning
    };
    Notification::new(kind, "Settings imported", text)
}

/// `a`, `a and b`, `a, b and c`.
fn join_parts(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The source of an import, with its snapshot.
pub(super) struct Prepared {
    pub source: Source,
    /// The snapshot's path in the configuration folder; `None` when none
    /// was kept.
    pub snapshot: Option<String>,
    /// Why no snapshot was kept, a sentence of the balloon.
    pub problem: Option<String>,
}

/// Keeps the snapshot of `folder` in the configuration folder and reads
/// the options from it (off the UI thread). Without a configuration
/// folder, or when the copy fails, the folder itself is read.
pub(super) fn prepare(folder: ConfigFolder, config_dir: Option<PathBuf>) -> Prepared {
    let name = snapshot::folder_name(&source_name(&folder), &folder.version);
    let relative = format!("{}/{name}", snapshot::FOLDER);
    let saved = config_dir
        .as_ref()
        .map(|dir| snapshot::save_pycharm(&folder.path, &dir.join(snapshot::FOLDER).join(&name)));
    let (snapshot, kept, problem) = match saved {
        Some(Ok(snapshot)) => (snapshot, Some(relative), None),
        Some(Err(e)) => {
            eprintln!("import: the snapshot could not be kept: {e}");
            (
                Snapshot::at(&folder.path),
                None,
                Some(format!(
                    "The settings could not be kept for features that come later: {e}."
                )),
            )
        }
        None => (Snapshot::at(&folder.path), None, None),
    };
    let options = snapshot.options();
    Prepared {
        source: Source {
            folder,
            snapshot,
            options,
            config_dir,
            fill_only: false,
        },
        snapshot: kept,
        problem,
    }
}

/// The application a folder belongs to as `[import]` records it
/// (`PyCharm`; the folder's name for a folder of another name).
fn source_name(folder: &ConfigFolder) -> String {
    if folder.product.is_empty() {
        folder.name.clone()
    } else {
        folder.product.clone()
    }
}

/// The record of a finished import: where it came from, its snapshot and
/// the importers that ran.
pub(super) fn record(
    folder: &ConfigFolder,
    snapshot: Option<&str>,
    ran: &[&str],
) -> ImportSettings {
    let mut record =
        ImportSettings::imported(&source_name(folder), &folder.version, SystemTime::now());
    if let Some(path) = snapshot {
        record = record.with_snapshot(path);
    }
    for id in ran {
        record.mark_applied(id);
    }
    record
}

/// The source of a pick-up: the snapshot of the import `record` names,
/// read again (off the UI thread).
pub(super) fn recorded_source(
    record: &ImportSettings,
    dir: PathBuf,
    config_dir: PathBuf,
) -> Source {
    let product = record.source.clone().unwrap_or_default();
    let version = record.version.clone().unwrap_or_default();
    let folder = ConfigFolder {
        path: dir.clone(),
        name: format!("{product}{version}"),
        product,
        version,
        last_used: SystemTime::UNIX_EPOCH,
        plugins: dir.join("plugins"),
    };
    let snapshot = Snapshot::at(dir);
    let options = snapshot.options();
    Source {
        folder,
        snapshot,
        options,
        config_dir: Some(config_dir),
        fill_only: false,
    }
}

/// Pyoxide's configuration folder: the folder of `settings.toml`.
pub(super) fn config_dir(cx: &App) -> Option<PathBuf> {
    store::file(cx).and_then(|f| f.parent().map(Path::to_path_buf))
}

/// Runs the outcomes of the importers, in order.
pub(super) fn apply_all(applied: Vec<Applied>, cx: &mut App) -> Outcome {
    let mut all = Outcome::default();
    for apply in applied {
        let outcome = apply(cx);
        for problem in &outcome.problems {
            eprintln!("import: {problem}");
        }
        all.extend(outcome);
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_join_as_a_sentence() {
        let parts = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(join_parts(&parts(&["a"])), "a");
        assert_eq!(join_parts(&parts(&["a", "b"])), "a and b");
        assert_eq!(join_parts(&parts(&["a", "b", "c"])), "a, b and c");
        assert_eq!(format_number(14.0), "14");
        assert_eq!(format_number(14.5), "14.5");
    }

    #[test]
    fn bundled_themes_map_to_islands() {
        assert_eq!(builtin_theme("Islands Dark"), Some(("islands-dark", true)));
        assert_eq!(builtin_theme("Darcula"), Some(("islands-dark", false)));
        assert_eq!(
            builtin_theme("ExperimentalLight"),
            Some(("islands-light", false))
        );
        assert_eq!(builtin_theme("1bf66303-7318"), None);
        assert_eq!(builtin_scheme("Darcula"), Some(true));
        assert_eq!(builtin_scheme("Monokai Classic"), None);
    }
}
