//! The snapshot of an imported configuration: a copy of the reference
//! IDE's configuration data files that an import keeps in Pyoxide's
//! configuration folder (`imported/<source>-<version>/`), so that a
//! feature built after the import can take its part of the reference's
//! settings later, once (`[import.applied]` of `settings.toml`, ADR 0017).
//!
//! Only data files are copied (PyCharm: `options/*.xml` with the per-OS
//! folders, `keymaps/*.xml`, `colors/*.icls`, `codestyles/*.xml`,
//! `templates/*.xml`), never anything that holds secrets: the password
//! safe's settings (`security.xml`), password stores (`c.kdbx`, `*.pwd`),
//! licences and token files. The snapshot keeps the layout of the
//! configuration folder, so [`Snapshot`] reads a snapshot and a
//! configuration folder alike. Everything here is blocking file system
//! access: run it off the UI thread.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::pycharm::options::{self, ReferenceOptions};
use crate::xml::{self, Element};

/// The folder of the snapshots in Pyoxide's configuration folder.
pub const FOLDER: &str = "imported";

/// The folders of a PyCharm configuration folder that are copied, with the
/// extensions of the files copied from them.
const PYCHARM_PARTS: [(&str, &[&str]); 5] = [
    ("options", &["xml"]),
    ("keymaps", &["xml"]),
    ("colors", &["icls"]),
    ("codestyles", &["xml"]),
    ("templates", &["xml"]),
];

/// Folders inside a part that are copied too (`options/mac`), at most
/// this deep.
const MAX_DEPTH: usize = 2;

/// A larger file is not copied: option files are small.
const MAX_FILE: u64 = 8 * 1024 * 1024;

/// Words in the names of files that hold secrets or licences.
const SECRET_WORDS: [&str; 7] = [
    "security",
    "password",
    "credential",
    "secret",
    "token",
    "license",
    "keychain",
];

/// The extensions of password stores and keys.
const SECRET_EXTENSIONS: [&str; 4] = ["kdbx", "pwd", "key", "keystore"];

/// A file that is never copied (`security.xml`, `c.kdbx`, `github-token.xml`).
pub fn is_secret(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let extension = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    SECRET_EXTENSIONS.contains(&extension) || SECRET_WORDS.iter().any(|w| name.contains(w))
}

/// The snapshot's folder name for `source` `version` (`PyCharm-2026.2`):
/// characters other than letters, digits, `.`, `-` and `_` become `_`.
pub fn folder_name(source: &str, version: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let name = if version.is_empty() {
        clean(source)
    } else {
        format!("{}-{}", clean(source), clean(version))
    };
    match name.trim_start_matches('.') {
        "" => "import".to_owned(),
        rest => rest.to_owned(),
    }
}

/// Copies the data files of the PyCharm configuration folder `folder` to
/// `dest`, replacing a snapshot there. The copy is made next to `dest`
/// and renamed over it, so a failed copy leaves the old snapshot.
pub fn save_pycharm(folder: &Path, dest: &Path) -> io::Result<Snapshot> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("a snapshot needs a parent folder"))?;
    let name = dest
        .file_name()
        .ok_or_else(|| io::Error::other("a snapshot needs a name"))?
        .to_string_lossy();
    fs::create_dir_all(parent)?;
    let partial = parent.join(format!(".{name}.partial"));
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    fs::create_dir_all(&partial)?;
    let copied = PYCHARM_PARTS.iter().try_for_each(|(part, extensions)| {
        copy_part(&folder.join(part), &partial.join(part), extensions, 0)
    });
    if let Err(e) = copied {
        let _ = fs::remove_dir_all(&partial);
        return Err(e);
    }
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::rename(&partial, dest)?;
    Ok(Snapshot::at(dest))
}

/// Copies the data files of `from` (a missing folder has none).
fn copy_part(from: &Path, to: &Path, extensions: &[&str], depth: usize) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(from) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || is_secret(&name) {
            continue;
        }
        // Links are not followed: only the folder's own files are copied.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                copy_part(&path, &to.join(&name), extensions, depth + 1)?;
            }
            continue;
        }
        let data = kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
            && entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE);
        if data {
            fs::create_dir_all(to)?;
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

/// A snapshot (or a configuration folder of the same layout), read by the
/// features that take their settings from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    dir: PathBuf,
}

impl Snapshot {
    /// The snapshot in `dir`.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn exists(&self) -> bool {
        self.dir.is_dir()
    }

    /// A file by its path in the snapshot (`options/terminal.xml`), when it
    /// is there; a path that leaves the snapshot finds nothing.
    pub fn file(&self, path: &str) -> Option<PathBuf> {
        let relative = Path::new(path);
        let inside = relative.components().next().is_some()
            && relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        let file = self.dir.join(relative);
        (inside && file.is_file()).then_some(file)
    }

    /// The `<component name>` of an option file (`terminal.xml`), from the
    /// per-OS folder (`options/mac`) first.
    pub fn component(&self, file: &str, name: &str) -> Option<Element> {
        let per_os = format!("options/{}/{file}", options::os_folder());
        [per_os, format!("options/{file}")]
            .iter()
            .filter_map(|path| self.file(path))
            .find_map(|path| {
                let text = fs::read_to_string(path).ok()?;
                let root = xml::parse(&text).ok()?;
                options::component(&root, name).cloned()
            })
    }

    /// The value of `<option name value>` of a component, as written (an
    /// empty value included).
    pub fn option(&self, file: &str, component: &str, name: &str) -> Option<String> {
        let component = self.component(file, component)?;
        options::raw_option(&component, name).map(str::to_owned)
    }

    /// The options Pyoxide imports ([`options::read_options`]).
    pub fn options(&self) -> ReferenceOptions {
        options::read_options(&self.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn reference(dir: &Path) {
        write(
            &dir.join("options/terminal.xml"),
            r#"<application><component name="TerminalOptionsProvider">
  <option name="myShellPath" value="/bin/zsh" />
  <option name="myTabName" value="" />
</component></application>"#,
        );
        write(
            &dir.join(format!("options/{}/keymap.xml", options::os_folder())),
            r#"<application><component name="KeymapManager"><active_keymap name="Mine" /></component></application>"#,
        );
        write(&dir.join("keymaps/Mine.xml"), r#"<keymap name="Mine" />"#);
        write(&dir.join("colors/Mine.icls"), r#"<scheme name="Mine" />"#);
        write(&dir.join("codestyles/Default.xml"), "<code_scheme/>");
        // Secrets, licences and what is not a data file stay behind.
        write(&dir.join("options/security.xml"), "<application/>");
        write(&dir.join("options/github-token.xml"), "<application/>");
        write(&dir.join("options/trace_license_storage.xml"), "<x/>");
        write(&dir.join("options/notes.txt"), "text");
        write(&dir.join("c.kdbx"), "secret");
        write(&dir.join("c.pwd"), "secret");
        write(&dir.join("pycharm.key"), "licence");
        write(&dir.join("user.web.token"), "token");
        write(&dir.join("plugins/a/lib/a.jar"), "jar");
        write(&dir.join("keymaps/.hidden.xml"), "<keymap/>");
    }

    fn files(dir: &Path) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let relative = path.strip_prefix(root).unwrap();
                    let parts: Vec<String> = relative
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    out.push(parts.join("/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn the_snapshot_holds_data_files_and_no_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("PyCharm2026.2");
        reference(&folder);
        let dest = tmp.path().join("config/imported/PyCharm-2026.2");
        let snapshot = save_pycharm(&folder, &dest).unwrap();
        assert_eq!(snapshot.dir(), dest);
        let os = options::os_folder();
        let mut expected = vec![
            "codestyles/Default.xml".to_owned(),
            "colors/Mine.icls".to_owned(),
            "keymaps/Mine.xml".to_owned(),
            format!("options/{os}/keymap.xml"),
            "options/terminal.xml".to_owned(),
        ];
        expected.sort();
        assert_eq!(files(&dest), expected);
        assert_eq!(snapshot.options().keymap.as_deref(), Some("Mine"));
        assert_eq!(
            options::keymap_file(snapshot.dir(), "Mine"),
            Some(dest.join("keymaps/Mine.xml"))
        );
        // Saving again replaces the snapshot.
        fs::remove_file(folder.join("colors/Mine.icls")).unwrap();
        save_pycharm(&folder, &dest).unwrap();
        assert!(!dest.join("colors/Mine.icls").exists());
        assert!(
            !tmp.path()
                .join("config/imported/.PyCharm-2026.2.partial")
                .exists()
        );
    }

    #[test]
    fn values_are_read_from_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        reference(tmp.path());
        let snapshot = Snapshot::at(tmp.path());
        assert_eq!(
            snapshot
                .option("terminal.xml", "TerminalOptionsProvider", "myShellPath")
                .as_deref(),
            Some("/bin/zsh")
        );
        assert_eq!(
            snapshot
                .option("terminal.xml", "TerminalOptionsProvider", "myTabName")
                .as_deref(),
            Some("")
        );
        assert_eq!(
            snapshot.option("terminal.xml", "TerminalOptionsProvider", "missing"),
            None
        );
        assert_eq!(snapshot.option("missing.xml", "X", "y"), None);
        // The per-OS folder.
        let keymap = snapshot.component("keymap.xml", "KeymapManager").unwrap();
        assert_eq!(
            keymap.child("active_keymap").unwrap().attr("name"),
            Some("Mine")
        );
        assert!(snapshot.file("options/terminal.xml").is_some());
        assert_eq!(snapshot.file("../outside.xml"), None);
        assert_eq!(snapshot.file(""), None);
        assert!(!Snapshot::at(tmp.path().join("gone")).exists());
    }

    #[test]
    fn names_of_snapshots_and_secrets() {
        assert_eq!(folder_name("PyCharm", "2026.2"), "PyCharm-2026.2");
        assert_eq!(folder_name("my config", ""), "my_config");
        assert_eq!(folder_name("../x", "1"), "_x-1");
        assert_eq!(folder_name("", ""), "import");
        for name in [
            "security.xml",
            "c.kdbx",
            "c.pwd",
            "pdb.key",
            "github-token.xml",
            "Credentials.xml",
            "trace_license_storage.xml",
        ] {
            assert!(is_secret(name), "{name}");
        }
        for name in ["editor.xml", "keymap.xml", "terminal.xml", "laf.xml"] {
            assert!(!is_secret(name), "{name}");
        }
    }
}
