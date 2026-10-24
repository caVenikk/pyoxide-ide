//! The documentation of a symbol that is a whole file, such as a Python
//! module, as the reference shows it (`PsiElementDocumentationTarget`,
//! `DocumentationManager.generateFileDoc`, IntelliJ Community, see
//! notices/code-intel-file-documentation.md): the product's header
//! (`Module contextlib`), the documentation the service sent, then the
//! file's facts in grey labels: the scope when the file has a file colour
//! (`Non-Project Files`), the size, the type and the modification and
//! creation times in the system's format; at the bottom a link to the
//! external documentation (`` `contextlib` on docs.python.org ``).
//!
//! The product says when a hover is about a file ([`FileDoc`], through
//! `CodeIntelProvider::file_doc`); the facts are read here, off the UI
//! thread ([`FileInfo::read`]).

use std::path::Path;
use std::time::SystemTime;

use serde_json::{Value as Json, json};

/// What the product says about a symbol that is a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDoc {
    /// The kind of the symbol as the header names it (`Module`,
    /// `Package`).
    pub kind: String,
    /// Its name, bold in the header and quoted in the link (`contextlib`).
    pub name: String,
    /// The file type's name (`Python`, `PythonStub`).
    pub file_type: String,
    /// Where its documentation is on the web.
    pub external_url: Option<String>,
}

/// The scope a file is in, when the scope has a file colour (the
/// reference's default file colours: `Non-Project Files` in yellow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileScope {
    /// Outside the project's content: outside its folder or in an
    /// excluded folder (`NonProjectFilesScope`).
    NonProject,
}

impl FileScope {
    /// The name of the scope (`NonProjectFilesScope.NAME`).
    pub fn name(self) -> &'static str {
        match self {
            FileScope::NonProject => "Non-Project Files",
        }
    }

    /// The scope of a file in the project at `root`; `None` for a file of
    /// the project's content (which has no file colour by default).
    /// Blocking: reads the project's exclusions.
    pub fn of(path: &Path, root: Option<&Path>) -> Option<Self> {
        let Some(root) = root.filter(|root| path.starts_with(root)) else {
            return Some(FileScope::NonProject);
        };
        let filter = project::Filter::for_root(root);
        filter
            .is_excluded(path, false)
            .then_some(FileScope::NonProject)
    }
}

/// The facts of a file as the documentation shows them, formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub scope: Option<FileScope>,
    /// `9,74 kB`.
    pub size: String,
    pub file_type: String,
    /// The times in the system's short date and time format.
    pub modified: Option<String>,
    pub created: Option<String>,
}

impl FileInfo {
    /// The facts of a file (blocking: reads its metadata and the
    /// project's exclusions); `None` when its metadata cannot be read, as
    /// the reference then shows none.
    pub fn read(path: &Path, root: Option<&Path>, file_type: &str) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let time = |t: std::io::Result<SystemTime>| t.ok().map(ui::locale::date_time);
        Some(Self {
            scope: FileScope::of(path, root),
            size: ui::locale::file_size(metadata.len()),
            file_type: file_type.to_owned(),
            modified: time(metadata.modified()),
            created: time(metadata.created()),
        })
    }

    /// The rows: a grey label and the value (the reference's labels,
    /// `CodeInsightBundle`).
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let Some(scope) = self.scope {
            rows.push(("Scope:", scope.name().to_owned()));
        }
        rows.push(("Size:", self.size.clone()));
        rows.push(("Type:", self.file_type.clone()));
        if let Some(modified) = &self.modified {
            rows.push(("Modified:", modified.clone()));
        }
        if let Some(created) = &self.created {
            rows.push(("Created:", created.clone()));
        }
        rows
    }

    pub fn to_json(&self) -> Json {
        json!({
            "scope": self.scope.map(FileScope::name),
            "size": self.size,
            "type": self.file_type,
            "modified": self.modified,
            "created": self.created,
        })
    }
}

/// The link to the external documentation at the bottom of the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLink {
    pub url: String,
    /// `` `contextlib` on docs.python.org ``.
    pub text: String,
}

impl ExternalLink {
    /// The link to `url` for the symbol `title` as the reference words it
    /// (`DocumentationManager.getLink`: `` `{0}` on {1} `` with the URL's
    /// host; without a host `` External documentation for `{0}` ``).
    pub fn new(title: &str, url: &str) -> Self {
        let text = match host(url) {
            Some(host) => format!("`{title}` on {host}"),
            None => format!("External documentation for `{title}`"),
        };
        Self {
            url: url.to_owned(),
            text,
        }
    }
}

/// The host of a URL (`docs.python.org` of
/// `https://docs.python.org/3.13/library/contextlib.html`).
pub fn host(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_the_host() {
        let link = ExternalLink::new(
            "contextlib",
            "https://docs.python.org/3.13/library/contextlib.html",
        );
        assert_eq!(link.text, "`contextlib` on docs.python.org");
        assert_eq!(
            host("http://user@example.com:8080/x?y"),
            Some("example.com")
        );
        assert_eq!(host("docs"), None);
        assert_eq!(
            ExternalLink::new("x", "nowhere").text,
            "External documentation for `x`"
        );
    }

    #[test]
    fn files_outside_the_project_are_non_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let app = root.join("app.py");
        std::fs::write(&app, "x = 1\n").unwrap();
        let outside = dir.path().join("contextlib.pyi");
        std::fs::write(&outside, "\"\"\"Doc.\"\"\"\n").unwrap();

        assert_eq!(FileScope::of(&app, Some(&root)), None);
        assert_eq!(
            FileScope::of(&outside, Some(&root)),
            Some(FileScope::NonProject)
        );
        assert_eq!(FileScope::of(&outside, None), Some(FileScope::NonProject));

        let info = FileInfo::read(&outside, Some(&root), "PythonStub").unwrap();
        let rows: Vec<&str> = info.rows().iter().map(|(label, _)| *label).collect();
        assert_eq!(rows[..3], ["Scope:", "Size:", "Type:"]);
        assert_eq!(info.rows()[1].1, "11 B");
        assert_eq!(info.rows()[2].1, "PythonStub");
        assert!(info.modified.is_some());
        assert!(FileInfo::read(dir.path(), None, "x").is_none());
        assert!(FileInfo::read(&dir.path().join("missing"), None, "x").is_none());
    }
}
