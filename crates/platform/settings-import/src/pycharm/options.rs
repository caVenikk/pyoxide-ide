//! The reference IDE's application settings, read once for the first-run
//! import (ADR 0017): its configuration folders, newest first, and the
//! options Pyoxide has a place for in their data files.
//!
//! - `options/laf.xml`: the theme (`LafManager`, `<laf themeId>`);
//! - `options/colors.scheme.xml`: the editor colour scheme
//!   (`EditorColorsManagerImpl`, `<global_color_scheme name>`), a scheme of
//!   its own in `colors/*.icls`;
//! - `options/<os>/keymap.xml` (`options/keymap.xml` before per-OS
//!   storage): the keymap (`KeymapManager`, `<active_keymap name>`), a
//!   custom one in `keymaps/*.xml`;
//! - `options/editor-font.xml`: the editor font (`DefaultFont`);
//! - `options/terminal-font.xml`: the terminal font (`TerminalFontOptions`,
//!   the keys of the editor font and `COLUMN_SPACING`);
//! - `options/editor.xml`: soft wraps (`EditorSettings`);
//! - `options/ui.lnf.xml`: the UI options Pyoxide honours (`UISettings`);
//! - `options/ide.general.xml`: autosave and synchronisation
//!   (`GeneralSettings`);
//! - `options/filetypes.xml`: file name associations (`FileTypeManager`);
//! - `META-INF/plugin.xml` of the plugins in the plugin folder: which
//!   archive holds a theme `laf.xml` names by id (`<themeProvider id path>`)
//!   and which bring colour schemes (`<bundledColorScheme path>`). Only
//!   that entry is read here; the theme importer opens the theme and scheme
//!   files it names, nothing else of a plugin.
//!
//! The reference's files are only read, never written. Everything here is
//! blocking file system access: run it off the UI thread.

use std::fs::File;
use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::xml::{self, Element};

/// The products whose configuration folders are offered, longest prefix
/// first (`IDEData`: `PyCharmCE` before `PyCharm`).
const PRODUCTS: [&str; 2] = ["PyCharmCE", "PyCharm"];

/// A folder unused for this long is offered under Other Options only
/// (`ConfigImportHelper.isConfigOld`: 180 days).
pub const OLD_AFTER: Duration = Duration::from_secs(180 * 24 * 60 * 60);

/// The option files whose dates tell when a folder was last used
/// (`JbImportServiceImpl.DEFAULT_SETTINGS_FILES`).
const DATED_FILES: [&str; 2] = ["ide.general.xml", "other.xml"];

/// The largest descriptor that is read from a plugin archive.
const MAX_DESCRIPTOR: u64 = 4 * 1024 * 1024;

/// A configuration folder of the reference IDE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFolder {
    pub path: PathBuf,
    /// The folder's name (`PyCharm2026.2`).
    pub name: String,
    /// The product part of the name (`PyCharm`, `PyCharmCE`).
    pub product: String,
    /// The version part (`2026.2`).
    pub version: String,
    /// The newest of its dated option files: when it was used last.
    pub last_used: SystemTime,
    /// Where its plugins are.
    pub plugins: PathBuf,
}

impl ConfigFolder {
    /// The folder as the first-run window names it: the reference's
    /// product names do not appear in the UI; a folder of another name is
    /// named by its name.
    pub fn label(&self) -> String {
        if self.version.is_empty() {
            self.name.clone()
        } else if self.product.ends_with("CE") {
            format!("Reference IDE {} Community", self.version)
        } else {
            format!("Reference IDE {}", self.version)
        }
    }

    /// The folder's name has the default form (`PyCharm2026.2`, not a
    /// copy such as `PyCharm2026.2-backup`).
    fn default_name(&self) -> bool {
        is_version(&self.version)
    }
}

/// The configuration folders found: the newest version of each product
/// that was used in the last [`OLD_AFTER`] (the main choices), and the
/// others; both by last use, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub main: Vec<ConfigFolder>,
    pub other: Vec<ConfigFolder>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.main.is_empty() && self.other.is_empty()
    }

    /// Every folder, the main ones first.
    pub fn all(&self) -> impl Iterator<Item = &ConfigFolder> {
        self.main.iter().chain(&self.other)
    }
}

/// The folder holding the reference IDE's configuration folders:
/// `JetBrains` in the platform's configuration folder (`~/Library/Application
/// Support` on macOS, `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` or
/// `~/.config` on Linux).
pub fn config_root() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("JetBrains"))
}

/// The plugin folder of a configuration folder named `name` under `root`
/// (`PathManager.getDefaultPluginPathFor`): `plugins` inside it on macOS
/// and Windows, `$XDG_DATA_HOME/JetBrains/<name>` on Linux; a `plugins`
/// folder inside it wins everywhere (a copied configuration).
pub fn plugins_dir(root: &Path, name: &str) -> PathBuf {
    let inside = root.join(name).join("plugins");
    if cfg!(any(target_os = "macos", windows)) || inside.is_dir() {
        return inside;
    }
    match dirs::data_dir() {
        Some(data) => data.join("JetBrains").join(name),
        None => root.join(name).join("plugins"),
    }
}

/// The configuration folders under `root` (see [`Found`]); `now` decides
/// which are old.
pub fn find_config_folders(root: &Path, now: SystemTime) -> Found {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Found::default();
    };
    let mut folders: Vec<ConfigFolder> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            config_folder(root, &e.path(), &name)
        })
        .collect();
    folders.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(b.name.cmp(&a.name)));
    let recent = |f: &ConfigFolder| {
        now.duration_since(f.last_used)
            .map(|age| age < OLD_AFTER)
            .unwrap_or(true)
    };
    // The newest version of each product among the recent folders.
    let mut newest: Vec<(&str, &str)> = Vec::new();
    for f in folders.iter().filter(|f| recent(f) && f.default_name()) {
        match newest.iter_mut().find(|(p, _)| *p == f.product) {
            Some((_, v)) if version_key(&f.version) > version_key(v) => *v = &f.version,
            Some(_) => {}
            None => newest.push((&f.product, &f.version)),
        }
    }
    let newest: Vec<(String, String)> = newest
        .into_iter()
        .map(|(p, v)| (p.to_owned(), v.to_owned()))
        .collect();
    let (main, other) = folders.into_iter().partition(|f| {
        f.default_name()
            && newest
                .iter()
                .any(|(p, v)| *p == f.product && *v == f.version)
    });
    Found { main, other }
}

/// A folder of a known product with a version and dated option files.
fn config_folder(root: &Path, path: &Path, name: &str) -> Option<ConfigFolder> {
    let product = PRODUCTS.iter().find(|p| name.starts_with(**p))?;
    let version = &name[product.len()..];
    if !version.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let options = path.join("options");
    let last_used = DATED_FILES
        .iter()
        .filter_map(|f| std::fs::metadata(options.join(f)).ok()?.modified().ok())
        .max()?;
    Some(ConfigFolder {
        path: path.to_owned(),
        name: name.to_owned(),
        product: (*product).to_owned(),
        version: version.to_owned(),
        last_used,
        plugins: plugins_dir(root, name),
    })
}

/// `2026.2`: four digits from 2000, a dot, one digit (`IDE_NAME_PATTERN`).
fn is_version(version: &str) -> bool {
    let b = version.as_bytes();
    b.len() == 6
        && b.starts_with(b"20")
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'.'
        && b[5].is_ascii_digit()
}

/// A version's numbers, for comparing (`2025.10` after `2025.9`).
fn version_key(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// A folder chosen with "Config or Installation Directory": a
/// configuration folder has an `options` folder.
pub fn custom_folder(path: &Path) -> Option<ConfigFolder> {
    if !path.join("options").is_dir() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().into_owned();
    let root = path.parent().unwrap_or(path);
    let (product, version) = match PRODUCTS.iter().find(|p| name.starts_with(**p)) {
        Some(p) => ((*p).to_owned(), name[p.len()..].to_owned()),
        None => (String::new(), String::new()),
    };
    let last_used = DATED_FILES
        .iter()
        .filter_map(|f| {
            std::fs::metadata(path.join("options").join(f))
                .ok()?
                .modified()
                .ok()
        })
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let plugins = if path.join("plugins").is_dir() {
        path.join("plugins")
    } else {
        plugins_dir(root, &name)
    };
    Some(ConfigFolder {
        path: path.to_owned(),
        name,
        product,
        version,
        last_used,
        plugins,
    })
}

// ---- the option files ---------------------------------------------------------

/// The editor font (`AppEditorFontOptions`); a key the file lacks is the
/// reference's default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FontOptions {
    pub family: Option<String>,
    /// `FONT_SIZE_2D`, else `FONT_SIZE`.
    pub size: Option<f32>,
    pub line_spacing: Option<f32>,
    pub ligatures: Option<bool>,
    /// `SECONDARY_FONT_FAMILY`: the family of the characters the font
    /// lacks.
    pub fallback: Option<String>,
}

impl FontOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The terminal font (`TerminalFontSettingsService`): the keys of the editor
/// font and the column spacing. A file that never held a choice has none
/// of the keys, which is [`TerminalFontOptions::is_empty`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerminalFontOptions {
    /// `FONT_FAMILY`, `FONT_SIZE_2D` (else `FONT_SIZE`), `LINE_SPACING`
    /// and `SECONDARY_FONT_FAMILY`; the ligatures key is not used.
    pub font: FontOptions,
    /// `COLUMN_SPACING`.
    pub column_spacing: Option<f32>,
}

impl TerminalFontOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Soft wraps (`EditorSettingsExternalizable`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoftWrapOptions {
    /// `USE_SOFT_WRAPS` names the main editor: "Soft-wrap these files".
    pub enabled: bool,
    /// `SOFT_WRAP_FILE_MASKS`; `None` for the default.
    pub masks: Option<String>,
    /// `USE_CUSTOM_SOFT_WRAP_INDENT`: wrapped fragments take the line's
    /// indent (on by default).
    pub use_original_indent: Option<bool>,
    /// `CUSTOM_SOFT_WRAP_INDENT`.
    pub indent: Option<usize>,
    /// `IS_ALL_SOFTWRAPS_SHOWN`.
    pub show_all: Option<bool>,
}

/// Autosave and synchronisation (`GeneralSettings`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GeneralOptions {
    /// `autoSaveFiles`: save when the IDE window is deactivated.
    pub save_on_deactivation: Option<bool>,
    /// `autoSaveIfInactive`: save after the IDE has been idle.
    pub save_on_idle: Option<bool>,
    /// `inactiveTimeout`, in seconds.
    pub idle_seconds: Option<u32>,
    /// `autoSyncFiles`: look at the files when the window gets the focus.
    pub sync_on_activation: Option<bool>,
}

impl GeneralOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The UI options (`UISettings`) Pyoxide honours.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiOptions {
    /// `SHOW_PREVIEW_IN_SEARCH_EVERYWHERE`.
    pub search_everywhere_preview: Option<bool>,
}

/// A file name pattern of a file type (`<mapping>` of `filetypes.xml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTypeMapping {
    /// `*.ext` for an extension mapping, else the pattern as written.
    pub pattern: String,
    /// The reference's name of the file type (`Python`, `TOML`).
    pub file_type: String,
}

/// What the reference's option files say, for the parts Pyoxide imports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReferenceOptions {
    /// The theme's id (`Islands Dark`, a plugin theme's UUID); `None`: the
    /// reference's default theme.
    pub theme: Option<String>,
    /// The editor colour scheme's name; `None`: the theme's own.
    pub scheme: Option<String>,
    /// The keymap's name; `None`: the platform's default.
    pub keymap: Option<String>,
    pub font: FontOptions,
    /// `terminal-font.xml`; empty without the file.
    pub terminal_font: TerminalFontOptions,
    /// `None` when `editor.xml` has no soft-wrap options.
    pub soft_wraps: Option<SoftWrapOptions>,
    pub general: GeneralOptions,
    pub ui: UiOptions,
    /// Patterns added to file types.
    pub file_types: Vec<FileTypeMapping>,
    /// Built-in patterns taken away from file types (`<removed_mapping>`).
    pub removed_file_types: Vec<FileTypeMapping>,
    /// Files that exist but could not be read, with the reason.
    pub problems: Vec<String>,
}

/// The font keys of an `AppFontOptions` component.
fn font_options(c: &Element) -> FontOptions {
    FontOptions {
        family: option(c, "FONT_FAMILY").map(str::to_owned),
        size: positive(option(c, "FONT_SIZE_2D").or_else(|| option(c, "FONT_SIZE"))),
        line_spacing: positive(option(c, "LINE_SPACING")),
        ligatures: option(c, "USE_LIGATURES").and_then(boolean),
        fallback: option(c, "SECONDARY_FONT_FAMILY").map(str::to_owned),
    }
}

/// A finite number above zero.
fn positive(value: Option<&str>) -> Option<f32> {
    value
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// The options of the configuration folder `dir`.
pub fn read_options(dir: &Path) -> ReferenceOptions {
    let options = dir.join("options");
    let mut out = ReferenceOptions::default();
    let mut read = |name: &str| -> Option<Element> {
        let file = options.join(name);
        let text = std::fs::read_to_string(&file).ok()?;
        match xml::parse(&text) {
            Ok(root) => Some(root),
            Err(e) => {
                out.problems.push(format!("{}: {e}", file.display()));
                None
            }
        }
    };
    let laf = read("laf.xml");
    let scheme = read("colors.scheme.xml");
    let keymap = read(&format!("{}/keymap.xml", os_folder())).or_else(|| read("keymap.xml"));
    let font = read("editor-font.xml");
    let terminal_font = read("terminal-font.xml");
    let editor = read("editor.xml");
    let ui = read("ui.lnf.xml");
    let general = read("ide.general.xml");
    let file_types = read("filetypes.xml");
    if let Some(root) = laf {
        out.theme = theme_id(&root);
    }
    if let Some(root) = scheme {
        out.scheme = component(&root, "EditorColorsManagerImpl")
            .and_then(|c| c.child("global_color_scheme"))
            .and_then(|e| e.attr("name"))
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
    }
    if let Some(root) = keymap {
        out.keymap = component(&root, "KeymapManager")
            .and_then(|c| c.child("active_keymap"))
            .and_then(|e| e.attr("name"))
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
    }
    if let Some(root) = font
        && let Some(c) = component(&root, "DefaultFont")
    {
        out.font = font_options(c);
    }
    if let Some(root) = terminal_font
        && let Some(c) = component(&root, "TerminalFontOptions")
    {
        out.terminal_font = TerminalFontOptions {
            font: FontOptions {
                ligatures: None,
                ..font_options(c)
            },
            column_spacing: positive(option(c, "COLUMN_SPACING")),
        };
    }
    if let Some(root) = editor
        && let Some(c) = component(&root, "EditorSettings")
    {
        let keys = [
            "USE_SOFT_WRAPS",
            "SOFT_WRAP_FILE_MASKS",
            "USE_CUSTOM_SOFT_WRAP_INDENT",
            "CUSTOM_SOFT_WRAP_INDENT",
            "IS_ALL_SOFTWRAPS_SHOWN",
        ];
        if keys.iter().any(|k| raw_option(c, k).is_some()) {
            out.soft_wraps = Some(SoftWrapOptions {
                enabled: raw_option(c, "USE_SOFT_WRAPS")
                    .is_some_and(|v| v.split(':').any(|p| p == "MAIN_EDITOR")),
                masks: option(c, "SOFT_WRAP_FILE_MASKS").map(str::to_owned),
                use_original_indent: option(c, "USE_CUSTOM_SOFT_WRAP_INDENT").and_then(boolean),
                indent: option(c, "CUSTOM_SOFT_WRAP_INDENT").and_then(|v| v.parse().ok()),
                show_all: option(c, "IS_ALL_SOFTWRAPS_SHOWN").and_then(boolean),
            });
        }
    }
    if let Some(root) = ui
        && let Some(c) = component(&root, "UISettings")
    {
        out.ui.search_everywhere_preview =
            option(c, "SHOW_PREVIEW_IN_SEARCH_EVERYWHERE").and_then(boolean);
    }
    if let Some(root) = general
        && let Some(c) = component(&root, "GeneralSettings")
    {
        out.general = GeneralOptions {
            save_on_deactivation: option(c, "autoSaveFiles").and_then(boolean),
            save_on_idle: option(c, "autoSaveIfInactive").and_then(boolean),
            idle_seconds: option(c, "inactiveTimeout").and_then(|v| v.parse().ok()),
            sync_on_activation: option(c, "autoSyncFiles").and_then(boolean),
        };
    }
    if let Some(root) = file_types
        && let Some(map) = component(&root, "FileTypeManager").and_then(|c| c.child("extensionMap"))
    {
        let mapping = |e: &Element| {
            let file_type = e.attr("type").filter(|t| !t.is_empty())?.to_owned();
            let pattern = match (e.attr("ext"), e.attr("pattern")) {
                (Some(ext), _) if !ext.is_empty() => format!("*.{ext}"),
                (_, Some(p)) if !p.is_empty() => p.to_owned(),
                _ => return None,
            };
            Some(FileTypeMapping { pattern, file_type })
        };
        out.file_types = map.children_named("mapping").filter_map(mapping).collect();
        out.removed_file_types = map
            .children_named("removed_mapping")
            .filter_map(mapping)
            .collect();
    }
    out
}

/// The per-OS folder of `options` (`getPerOsSettingsStorageFolderName`).
pub(crate) fn os_folder() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else {
        "unix"
    }
}

/// `<laf themeId>` of `LafManager`; Darcula of an IDE before 2023.3 by
/// its class name. `JetBrainsLightTheme` became Islands Light
/// (`getThemeNameWithReplacingDeprecated`).
fn theme_id(root: &Element) -> Option<String> {
    let laf = component(root, "LafManager")?.child("laf")?;
    match laf.attr("themeId").filter(|t| !t.is_empty()) {
        Some("JetBrainsLightTheme") => Some("Islands Light".to_owned()),
        Some(id) => Some(id.to_owned()),
        None if laf.attr("class-name") == Some("com.intellij.ide.ui.laf.darcula.DarculaLaf") => {
            Some("Darcula".to_owned())
        }
        None => None,
    }
}

/// The `<component name>` of an `<application>` file.
pub(crate) fn component<'a>(root: &'a Element, name: &str) -> Option<&'a Element> {
    if root.name == "component" && root.attr("name") == Some(name) {
        return Some(root);
    }
    root.children_named("component")
        .find(|c| c.attr("name") == Some(name))
}

/// `<option name value>` as written, empty included.
pub(crate) fn raw_option<'a>(component: &'a Element, name: &str) -> Option<&'a str> {
    component
        .children_named("option")
        .find(|o| o.attr("name") == Some(name))
        .and_then(|o| o.attr("value"))
}

/// `<option name value>` with a value.
pub(crate) fn option<'a>(component: &'a Element, name: &str) -> Option<&'a str> {
    raw_option(component, name).filter(|v| !v.is_empty())
}

fn boolean(v: &str) -> Option<bool> {
    match v {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

// ---- themes, schemes and keymaps of their own ---------------------------------

/// A plugin archive that brings themes or editor colour schemes, from its
/// descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemePlugin {
    pub archive: PathBuf,
    /// `<themeProvider id path>`: the theme's id and its file's path in the
    /// archive, without a leading `/` (`themes/monokai-classic.json`).
    pub themes: Vec<(String, String)>,
    /// `<bundledColorScheme path>`: scheme files of the archive.
    pub schemes: Vec<String>,
}

/// The plugin archives of a plugin folder: `<plugin>/lib/*.jar` and
/// single-file plugins (`*.jar`, `*.zip`) at the top.
pub fn plugin_archives(plugins: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(plugins) else {
        return Vec::new();
    };
    let is_archive = |p: &Path| {
        p.is_file()
            && p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jar") || e.eq_ignore_ascii_case("zip"))
    };
    let mut out = Vec::new();
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
    paths.sort();
    for path in paths {
        if is_archive(&path) {
            out.push(path);
        } else if path.is_dir()
            && let Ok(lib) = std::fs::read_dir(path.join("lib"))
        {
            let mut jars: Vec<PathBuf> = lib
                .filter_map(|e| Some(e.ok()?.path()))
                .filter(|p| is_archive(p))
                .collect();
            jars.sort();
            out.extend(jars);
        }
    }
    out
}

/// The archives of a plugin folder whose descriptors declare themes or
/// colour schemes. Only `META-INF/plugin.xml` of each archive is read.
pub fn theme_plugins(plugins: &Path) -> Vec<ThemePlugin> {
    plugin_archives(plugins)
        .into_iter()
        .filter_map(|archive| {
            let descriptor = read_descriptor(&archive)?;
            let root = xml::parse(&descriptor).ok()?;
            let mut plugin = ThemePlugin {
                archive,
                themes: Vec::new(),
                schemes: Vec::new(),
            };
            collect_providers(&root, &mut plugin);
            (!plugin.themes.is_empty() || !plugin.schemes.is_empty()).then_some(plugin)
        })
        .collect()
}

/// The archive and the file of the theme `id` among `plugins`.
pub fn theme_provider<'a>(plugins: &'a [ThemePlugin], id: &str) -> Option<(&'a Path, &'a str)> {
    plugins.iter().find_map(|p| {
        p.themes
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, path)| (p.archive.as_path(), path.as_str()))
    })
}

/// `META-INF/plugin.xml` of an archive.
fn read_descriptor(archive: &Path) -> Option<String> {
    let file = File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).ok()?;
    let entry = zip.by_name("META-INF/plugin.xml").ok()?;
    if entry.size() > MAX_DESCRIPTOR {
        return None;
    }
    let mut text = String::new();
    entry.take(MAX_DESCRIPTOR).read_to_string(&mut text).ok()?;
    Some(text)
}

fn collect_providers(e: &Element, out: &mut ThemePlugin) {
    let path = |e: &Element| e.attr("path").map(|p| p.trim_start_matches('/').to_owned());
    match e.name.as_str() {
        "themeProvider" => {
            if let (Some(id), Some(path)) = (e.attr("id"), path(e)) {
                out.themes.push((id.to_owned(), path));
            }
        }
        "bundledColorScheme" => out.schemes.extend(path(e)),
        _ => {}
    }
    for child in &e.children {
        collect_providers(child, out);
    }
}

/// The scheme file of the configuration folder `dir` named `name`
/// (`colors/*.icls`, `<scheme name>`).
pub fn scheme_file(dir: &Path, name: &str) -> Option<PathBuf> {
    named_file(&dir.join("colors"), "icls", "scheme", name)
}

/// The custom keymap file of the configuration folder `dir` named `name`
/// (`keymaps/*.xml`, `<keymap name>`).
pub fn keymap_file(dir: &Path, name: &str) -> Option<PathBuf> {
    named_file(&dir.join("keymaps"), "xml", "keymap", name)
}

/// The file of `folder` with extension `ext` whose root element `root`
/// has the attribute `name`.
fn named_file(folder: &Path, ext: &str, root: &str, name: &str) -> Option<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
        .collect();
    paths.sort();
    paths.into_iter().find(|path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| xml::parse(&text).ok())
            .is_some_and(|e| e.name == root && e.attr("name") == Some(name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn folder(root: &Path, name: &str, age_days: u64) -> PathBuf {
        let dir = root.join(name);
        let file = dir.join("options/other.xml");
        write(&file, "<application/>");
        let when = SystemTime::now() - Duration::from_secs(age_days * 24 * 60 * 60);
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(when)
            .unwrap();
        dir
    }

    #[test]
    fn newest_recent_folder_of_each_product_is_a_main_choice() {
        let root = tempfile::tempdir().unwrap();
        folder(root.path(), "PyCharm2026.2", 1);
        folder(root.path(), "PyCharm2026.1", 0);
        folder(root.path(), "PyCharm2025.3", 400);
        folder(root.path(), "PyCharmCE2025.1", 200);
        folder(root.path(), "PyCharm2026.2-backup", 0);
        folder(root.path(), "GoLand2026.2", 0);
        std::fs::create_dir_all(root.path().join("PyCharm2024.1")).unwrap();
        let found = find_config_folders(root.path(), SystemTime::now());
        let names = |v: &[ConfigFolder]| v.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&found.main), ["PyCharm2026.2"]);
        assert_eq!(
            names(&found.other),
            [
                "PyCharm2026.2-backup",
                "PyCharm2026.1",
                "PyCharmCE2025.1",
                "PyCharm2025.3"
            ]
        );
        assert_eq!(found.main[0].label(), "Reference IDE 2026.2");
        assert_eq!(found.other[2].product, "PyCharmCE");
        assert_eq!(found.other[2].label(), "Reference IDE 2025.1 Community");
    }

    #[test]
    fn a_missing_root_finds_nothing() {
        let found = find_config_folders(Path::new("/no/such/folder"), SystemTime::now());
        assert!(found.is_empty());
    }

    #[test]
    fn reads_the_options_pyoxide_imports() {
        let dir = tempfile::tempdir().unwrap();
        let o = dir.path().join("options");
        write(
            &o.join("laf.xml"),
            r#"<application><component name="LafManager"><laf themeId="1bf66303-7318" /></component></application>"#,
        );
        write(
            &o.join("colors.scheme.xml"),
            r#"<application><component name="EditorColorsManagerImpl"><global_color_scheme name="Monokai Classic" /></component></application>"#,
        );
        write(
            &o.join(format!("{}/keymap.xml", os_folder())),
            r#"<application><component name="KeymapManager"><active_keymap name="macOS copy" /></component></application>"#,
        );
        write(
            &o.join("editor-font.xml"),
            r#"<application><component name="DefaultFont">
                <option name="VERSION" value="1" />
                <option name="FONT_SIZE" value="14" />
                <option name="FONT_SIZE_2D" value="14.5" />
                <option name="LINE_SPACING" value="1.3" />
                <option name="FONT_FAMILY" value="Fira Code" />
                <option name="USE_LIGATURES" value="true" />
            </component></application>"#,
        );
        write(
            &o.join("terminal-font.xml"),
            r#"<application><component name="TerminalFontOptions">
                <option name="FONT_SIZE" value="12" />
                <option name="FONT_SIZE_2D" value="12.5" />
                <option name="FONT_FAMILY" value="JetBrainsMono Nerd Font" />
                <option name="LINE_SPACING" value="1.1" />
                <option name="SECONDARY_FONT_FAMILY" value="Monospaced" />
                <option name="COLUMN_SPACING" value="0.9" />
            </component></application>"#,
        );
        write(
            &o.join("editor.xml"),
            r#"<application><component name="EditorSettings">
                <option name="USE_SOFT_WRAPS" value="MAIN_EDITOR:CONSOLE" />
                <option name="SOFT_WRAP_FILE_MASKS" value="*.md; *.py" />
                <option name="CUSTOM_SOFT_WRAP_INDENT" value="4" />
            </component></application>"#,
        );
        write(
            &o.join("ui.lnf.xml"),
            r#"<application><component name="UISettings"><option name="SHOW_PREVIEW_IN_SEARCH_EVERYWHERE" value="true" /></component></application>"#,
        );
        write(
            &o.join("ide.general.xml"),
            r#"<application><component name="GeneralSettings">
                <option name="autoSaveIfInactive" value="true" />
                <option name="inactiveTimeout" value="30" />
                <option name="autoSyncFiles" value="false" />
            </component></application>"#,
        );
        write(
            &o.join("filetypes.xml"),
            r#"<application><component name="FileTypeManager" version="19">
                <extensionMap>
                  <mapping ext="jinja" type="HTML" />
                  <mapping pattern="Pipfile" type="TOML" />
                  <removed_mapping ext="csv" approved="true" type="CSV/TSV" />
                </extensionMap>
            </component></application>"#,
        );
        let options = read_options(dir.path());
        assert_eq!(options.theme.as_deref(), Some("1bf66303-7318"));
        assert_eq!(options.scheme.as_deref(), Some("Monokai Classic"));
        assert_eq!(options.keymap.as_deref(), Some("macOS copy"));
        assert_eq!(
            options.font,
            FontOptions {
                family: Some("Fira Code".into()),
                size: Some(14.5),
                line_spacing: Some(1.3),
                ligatures: Some(true),
                fallback: None,
            }
        );
        assert_eq!(
            options.terminal_font,
            TerminalFontOptions {
                font: FontOptions {
                    family: Some("JetBrainsMono Nerd Font".into()),
                    size: Some(12.5),
                    line_spacing: Some(1.1),
                    ligatures: None,
                    fallback: Some("Monospaced".into()),
                },
                column_spacing: Some(0.9),
            }
        );
        assert_eq!(
            options.soft_wraps,
            Some(SoftWrapOptions {
                enabled: true,
                masks: Some("*.md; *.py".into()),
                use_original_indent: None,
                indent: Some(4),
                show_all: None,
            })
        );
        assert_eq!(options.ui.search_everywhere_preview, Some(true));
        assert_eq!(
            options.general,
            GeneralOptions {
                save_on_deactivation: None,
                save_on_idle: Some(true),
                idle_seconds: Some(30),
                sync_on_activation: Some(false),
            }
        );
        let patterns: Vec<_> = options
            .file_types
            .iter()
            .map(|m| (m.pattern.as_str(), m.file_type.as_str()))
            .collect();
        assert_eq!(patterns, [("*.jinja", "HTML"), ("Pipfile", "TOML")]);
        assert_eq!(options.removed_file_types[0].pattern, "*.csv");
        assert!(options.problems.is_empty());
    }

    #[test]
    fn empty_soft_wraps_are_off_and_missing_files_are_defaults() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("options/editor.xml"),
            r#"<application><component name="EditorSettings"><option name="USE_SOFT_WRAPS" value="" /></component></application>"#,
        );
        write(&dir.path().join("options/laf.xml"), "<application><broken");
        let options = read_options(dir.path());
        assert_eq!(
            options.soft_wraps,
            Some(SoftWrapOptions {
                enabled: false,
                ..SoftWrapOptions::default()
            })
        );
        assert_eq!(options.theme, None);
        assert!(options.font.is_empty());
        assert_eq!(options.problems.len(), 1, "{:?}", options.problems);
    }

    #[test]
    fn old_theme_ids_are_mapped() {
        let laf = |inner: &str| {
            xml::parse(&format!(
                r#"<application><component name="LafManager">{inner}</component></application>"#
            ))
            .unwrap()
        };
        assert_eq!(
            theme_id(&laf(r#"<laf themeId="JetBrainsLightTheme"/>"#)).as_deref(),
            Some("Islands Light")
        );
        assert_eq!(
            theme_id(&laf(
                r#"<laf class-name="com.intellij.ide.ui.laf.darcula.DarculaLaf"/>"#
            ))
            .as_deref(),
            Some("Darcula")
        );
        assert_eq!(theme_id(&laf("")), None);
    }

    fn jar(path: &Path, entries: &[(&str, &str)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, text) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn finds_the_plugin_that_provides_a_theme() {
        let plugins = tempfile::tempdir().unwrap();
        let p = plugins.path();
        jar(
            &p.join("other/lib/other.jar"),
            &[(
                "META-INF/plugin.xml",
                "<idea-plugin><id>other</id></idea-plugin>",
            )],
        );
        jar(
            &p.join("monokai/lib/monokai.jar"),
            &[
                (
                    "META-INF/plugin.xml",
                    r#"<idea-plugin><extensions defaultExtensionNs="com.intellij">
                        <themeProvider id="aaa" path="/themes/a.json"/>
                        <themeProvider id="bbb" path="/themes/b.json"/>
                    </extensions></idea-plugin>"#,
                ),
                ("themes/b.json", "{}"),
            ],
        );
        jar(
            &p.join("single.jar"),
            &[(
                "META-INF/plugin.xml",
                r#"<idea-plugin><extensions><themeProvider id="ccc" path="/c.theme.json"/><bundledColorScheme path="/colors/Night.xml"/></extensions></idea-plugin>"#,
            )],
        );
        let plugins = theme_plugins(p);
        let archives: Vec<&Path> = plugins.iter().map(|t| t.archive.as_path()).collect();
        assert_eq!(
            archives,
            [p.join("monokai/lib/monokai.jar"), p.join("single.jar")]
        );
        assert_eq!(plugins[1].schemes, ["colors/Night.xml"]);
        let (archive, path) = theme_provider(&plugins, "bbb").unwrap();
        assert_eq!(archive, p.join("monokai/lib/monokai.jar"));
        assert_eq!(path, "themes/b.json");
        assert_eq!(theme_provider(&plugins, "ccc").unwrap().1, "c.theme.json");
        assert_eq!(theme_provider(&plugins, "zzz"), None);
    }

    #[test]
    fn finds_schemes_and_keymaps_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("colors/_@user_Darcula.icls"),
            r#"<scheme name="_@user_Darcula" version="142" parent_scheme="Darcula"></scheme>"#,
        );
        write(
            &dir.path().join("keymaps/macOS copy.xml"),
            r#"<keymap version="1" name="macOS copy" parent="Mac OS X 10.5+"></keymap>"#,
        );
        assert_eq!(
            scheme_file(dir.path(), "_@user_Darcula"),
            Some(dir.path().join("colors/_@user_Darcula.icls"))
        );
        assert_eq!(scheme_file(dir.path(), "Other"), None);
        assert_eq!(
            keymap_file(dir.path(), "macOS copy"),
            Some(dir.path().join("keymaps/macOS copy.xml"))
        );
    }

    #[test]
    fn a_custom_folder_needs_options() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(custom_folder(dir.path()), None);
        let config = folder(dir.path(), "PyCharm2025.2", 3);
        let found = custom_folder(&config).unwrap();
        assert_eq!(found.version, "2025.2");
        assert_eq!(found.name, "PyCharm2025.2");
    }
}
