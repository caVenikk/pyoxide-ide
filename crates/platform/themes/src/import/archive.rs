//! Reading theme data out of plugin archives and directories.
//!
//! Only data files are ever opened: UI theme JSON and colour scheme XML. The
//! decision is made from the entry name alone, before the entry is touched, so
//! class files and anything else in a plugin are never read.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::Serialize;
use zip::ZipArchive;

use super::library::{ImportError, Library};

/// Largest data file that is read (theme files are well below 1 MiB).
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

/// How deep a directory is searched.
const MAX_DEPTH: usize = 12;

/// How many folders below the root an archive may sit to be opened.
const MAX_ARCHIVE_DEPTH: usize = 2;

/// What kind of data an entry name stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataKind {
    /// `*.theme.json`, or a JSON file under a `themes` directory.
    UiTheme,
    /// `*.icls`, or an XML file under a `themes` directory.
    Scheme,
}

/// Normalizes an entry path to `/` separators without leading `./` or `/`.
/// Returns `None` for paths that try to leave the root.
pub(crate) fn normalize(path: &str) -> Option<String> {
    let replaced = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Decides from the name alone whether an entry may be opened.
pub(crate) fn classify(path: &str) -> Option<DataKind> {
    let path = normalize(path)?;
    let lower = path.to_ascii_lowercase();
    let mut components: Vec<&str> = lower.split('/').collect();
    let file = components.pop()?;
    if components.first() == Some(&"meta-inf") {
        return None;
    }
    let in_themes = components.contains(&"themes");
    let in_metadata = components.contains(&"metadata");
    if file.ends_with(".thememetadata.json") {
        return None;
    }
    if file.ends_with(".theme.json") {
        return Some(DataKind::UiTheme);
    }
    if file.ends_with(".json") && in_themes && !in_metadata {
        return Some(DataKind::UiTheme);
    }
    if file.ends_with(".icls") {
        return Some(DataKind::Scheme);
    }
    if file.ends_with(".xml") && (in_themes || file == "defaultcolorschemesmanager.xml") {
        return Some(DataKind::Scheme);
    }
    None
}

/// True when `path` may be opened as the colour scheme a UI theme refers to.
pub(crate) fn is_scheme_reference(path: &str) -> bool {
    let Some(path) = normalize(path) else {
        return false;
    };
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".xml") || lower.ends_with(".icls")
}

/// What was done with one archive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveSummary {
    pub path: String,
    /// Entries in the archive (files and directories).
    pub entries: usize,
    /// Names of the entries that were opened.
    pub read: Vec<String>,
    /// Class files that were left untouched.
    pub ignored_classes: usize,
    /// Other entries that were left untouched.
    pub ignored_other: usize,
}

/// Adds the theme data of a `.jar` or `.zip` file to the library.
pub(crate) fn add_archive(
    library: &mut Library,
    path: &Path,
    primary: bool,
) -> Result<ArchiveSummary, ImportError> {
    let display = path.display().to_string();
    let file = File::open(path).map_err(|source| ImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut zip = ZipArchive::new(BufReader::new(file)).map_err(|e| ImportError::Archive {
        origin: display.clone(),
        message: e.to_string(),
    })?;
    let mut summary = ArchiveSummary {
        path: display.clone(),
        entries: zip.len(),
        ..ArchiveSummary::default()
    };

    let mut wanted: Vec<(usize, String, DataKind)> = Vec::new();
    for index in 0..zip.len() {
        let Some(Ok(name)) = zip.name_for_index(index) else {
            summary.ignored_other += 1;
            continue;
        };
        let name = name.into_owned();
        if name.ends_with('/') {
            continue;
        }
        match classify(&name) {
            Some(kind) => wanted.push((index, name, kind)),
            None if name.to_ascii_lowercase().ends_with(".class") => summary.ignored_classes += 1,
            None => summary.ignored_other += 1,
        }
    }

    let first_theme = library.theme_count();
    for (index, name, kind) in wanted {
        let Some(text) = read_entry(&mut zip, index, &name, &display, library) else {
            continue;
        };
        summary.read.push(name.clone());
        let origin = format!("{display}!{name}");
        let entry = normalize(&name).unwrap_or(name);
        library.add_text(kind, &text, &origin, &entry, primary);
    }

    // Colour schemes that the themes of this archive point at by path.
    let references = library.unresolved_scheme_references(first_theme);
    for reference in references {
        let Some(entry) = normalize(&reference) else {
            continue;
        };
        if !is_scheme_reference(&entry) {
            continue;
        }
        let Some(index) = zip.index_for_name(&entry) else {
            continue;
        };
        let Some(text) = read_entry(&mut zip, index, &entry, &display, library) else {
            continue;
        };
        summary.read.push(entry.clone());
        let origin = format!("{display}!{entry}");
        library.add_text(DataKind::Scheme, &text, &origin, &entry, primary);
    }
    Ok(summary)
}

fn read_entry<R: Read + std::io::Seek>(
    zip: &mut ZipArchive<R>,
    index: usize,
    name: &str,
    archive: &str,
    library: &mut Library,
) -> Option<String> {
    let mut skip = |reason: &str| {
        library.note_skipped(&format!("{archive}!{name}"), reason);
        None
    };
    let entry = match zip.by_index(index) {
        Ok(entry) => entry,
        Err(e) => return skip(&e.to_string()),
    };
    if entry.size() > MAX_ENTRY_BYTES {
        return skip("larger than 8 MiB");
    }
    let mut bytes = Vec::new();
    if entry
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return skip("cannot be decompressed");
    }
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return skip("larger than 8 MiB");
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Adds the theme data found under a directory (and in archives directly
/// inside it or below it, to the same depth limit).
pub(crate) fn add_dir(
    library: &mut Library,
    root: &Path,
    primary: bool,
) -> Result<Vec<ArchiveSummary>, ImportError> {
    let mut archives = Vec::new();
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    walk(root, root, 0, &mut files)?;
    files.sort();
    let first_theme = library.theme_count();
    for (path, relative) in files {
        let lower = relative.to_ascii_lowercase();
        if lower.ends_with(".jar") || lower.ends_with(".zip") {
            // Archives next to the files or one folder down (a plugin's `lib`).
            if relative.matches('/').count() <= MAX_ARCHIVE_DEPTH {
                match add_archive(library, &path, primary) {
                    Ok(summary) => archives.push(summary),
                    Err(e) => library.note_skipped(&path.display().to_string(), &e.to_string()),
                }
            }
            continue;
        }
        let Some(kind) = classify(&relative) else {
            continue;
        };
        let Some(text) = read_file(&path, library) else {
            continue;
        };
        library.add_text(kind, &text, &path.display().to_string(), &relative, primary);
    }
    // Schemes referenced by path but stored outside the usual folders.
    for reference in library.unresolved_scheme_references(first_theme) {
        let Some(entry) = normalize(&reference) else {
            continue;
        };
        if !is_scheme_reference(&entry) {
            continue;
        }
        let path = root.join(&entry);
        if let Some(text) = read_file(&path, library) {
            library.add_text(
                DataKind::Scheme,
                &text,
                &path.display().to_string(),
                &entry,
                primary,
            );
        }
    }
    Ok(archives)
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<(), ImportError> {
    let entries = std::fs::read_dir(dir).map_err(|source| ImportError::Io {
        path: dir.to_owned(),
        source,
    })?;
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // Symbolic links are not followed: no loops, no escape from the root.
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, depth + 1, out)?;
            }
        } else if kind.is_file()
            && let Ok(relative) = path.strip_prefix(root)
        {
            let relative = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((path.clone(), relative));
        }
    }
    Ok(())
}

/// Reads one data file with the same size limit as archive entries.
pub(crate) fn read_file(path: &Path, library: &mut Library) -> Option<String> {
    let display = path.display().to_string();
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) => {
            library.note_skipped(&display, &e.to_string());
            return None;
        }
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        library.note_skipped(&display, "cannot be read");
        return None;
    }
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        library.note_skipped(&display, "larger than 8 MiB");
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_files_are_recognised_by_name() {
        use DataKind::*;
        let cases = [
            ("themes/Sample.theme.json", Some(UiTheme)),
            ("Sample.theme.json", Some(UiTheme)),
            ("themes/sub/other.json", Some(UiTheme)),
            ("themes/Sample.xml", Some(Scheme)),
            ("themes/expUI/expUI_dark.theme.json", Some(UiTheme)),
            ("colors/user.icls", Some(Scheme)),
            ("DefaultColorSchemesManager.xml", Some(Scheme)),
            ("themes/metadata/IntelliJPlatform.themeMetadata.json", None),
            ("themes/metadata/other.json", None),
            ("META-INF/plugin.xml", None),
            ("META-INF/themes/x.theme.json", None),
            ("com/example/Theme.class", None),
            ("themes/Evil.class", None),
            ("themes/icons/logo.svg", None),
            ("config/settings.xml", None),
            ("lib/inner.jar", None),
            ("../themes/x.theme.json", None),
            ("", None),
        ];
        for (path, expected) in cases {
            assert_eq!(classify(path), expected, "{path}");
        }
    }

    #[test]
    fn paths_are_normalized() {
        assert_eq!(normalize("/themes/A.xml").as_deref(), Some("themes/A.xml"));
        assert_eq!(normalize("./a\\b/c").as_deref(), Some("a/b/c"));
        assert_eq!(normalize("a/../b"), None);
        assert_eq!(normalize("/"), None);
        assert!(is_scheme_reference("/themes/A.xml"));
        assert!(!is_scheme_reference("/themes/A.class"));
        assert!(!is_scheme_reference("Dark"));
    }
}
