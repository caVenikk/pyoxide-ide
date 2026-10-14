//! Locations and edits.

use std::ops::Range;
use std::path::PathBuf;

use serde::Serialize;
use text::Point;

/// A range in a file (a declaration, a usage). The file need not be open;
/// positions are in its text as the service saw it (on disk, unless it is
/// an open document).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Location {
    pub path: PathBuf,
    pub range: Range<Point>,
}

/// A replacement of a range with text. The edits of one list do not
/// overlap and are all in the text before any of them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TextEdit {
    pub range: Range<Point>,
    pub text: String,
}

impl TextEdit {
    pub fn new(range: Range<Point>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }

    pub fn insert(at: Point, text: impl Into<String>) -> Self {
        Self::new(at..at, text)
    }
}

/// The edits of one file in a [`WorkspaceEdit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEdits {
    pub path: PathBuf,
    pub edits: Vec<TextEdit>,
}

/// Edits of several files (a code action, a rename), applied together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkspaceEdit {
    pub files: Vec<FileEdits>,
}

impl WorkspaceEdit {
    pub fn is_empty(&self) -> bool {
        self.files.iter().all(|f| f.edits.is_empty())
    }

    /// The edits of one file.
    pub fn edits_for(&self, path: &std::path::Path) -> impl Iterator<Item = &TextEdit> {
        self.files
            .iter()
            .filter(move |f| f.path == path)
            .flat_map(|f| &f.edits)
    }
}

/// How the document is indented, for formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FormatOptions {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            tab_size: 4,
            insert_spaces: true,
        }
    }
}

/// How a symbol occurs at a [`DocumentHighlight`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HighlightKind {
    Text,
    Read,
    Write,
}

/// An occurrence of a symbol in a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocumentHighlight {
    pub range: Range<Point>,
    pub kind: HighlightKind,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn workspace_edits_by_file() {
        let edit = WorkspaceEdit {
            files: vec![
                FileEdits {
                    path: "/p/a.py".into(),
                    edits: vec![TextEdit::insert(Point::new(0, 0), "import os\n")],
                },
                FileEdits {
                    path: "/p/pyproject.toml".into(),
                    edits: Vec::new(),
                },
            ],
        };
        assert!(!edit.is_empty());
        assert_eq!(edit.edits_for(Path::new("/p/a.py")).count(), 1);
        assert_eq!(edit.edits_for(Path::new("/p/b.py")).count(), 0);
        assert!(WorkspaceEdit::default().is_empty());
        assert_eq!(FormatOptions::default().tab_size, 4);
    }
}
