//! Protocol values to the `code-intel` model. Lenient: an item of an
//! unknown shape is skipped, not the whole answer.

use std::ops::Range;
use std::path::{Path, PathBuf};

use code_intel::{FileEdits, Point, TextEdit, WorkspaceEdit};
use lsp_types::Position;
use serde_json::Value;

use crate::encoding::{PositionEncoding, Text};
use crate::uri;

/// An LSP position object.
pub(crate) fn position(value: &Value) -> Option<Position> {
    let line = value.get("line")?.as_u64()?;
    let character = value.get("character")?.as_u64()?;
    Some(Position::new(
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(character).unwrap_or(u32::MAX),
    ))
}

/// An LSP range object in a text.
pub(crate) fn range(
    value: &Value,
    text: &Text,
    encoding: PositionEncoding,
) -> Option<Range<Point>> {
    let start = text.point(position(value.get("start")?)?, encoding);
    let end = text.point(position(value.get("end")?)?, encoding);
    Some(if end < start { end..start } else { start..end })
}

/// LSP text edits (`TextEdit`, `AnnotatedTextEdit`) in a text.
pub(crate) fn text_edits(value: &Value, text: &Text, encoding: PositionEncoding) -> Vec<TextEdit> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|edit| {
            let range = range(edit.get("range")?, text, encoding)?;
            let new_text = edit.get("newText")?.as_str()?;
            Some(TextEdit::new(range, new_text))
        })
        .collect()
}

/// An LSP `WorkspaceEdit` (`changes` or `documentChanges`); `text_of` gives
/// the text of each file the positions refer to. Resource operations
/// (create, rename, delete) are not supported: an edit with them fails
/// as a whole, as `failureHandling: abort` asks.
pub(crate) fn workspace_edit(
    value: &Value,
    encoding: PositionEncoding,
    text_of: &dyn Fn(&Path) -> Result<Text, String>,
) -> Result<WorkspaceEdit, String> {
    let mut files: Vec<(PathBuf, &Value)> = Vec::new();
    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            if let Some(kind) = change.get("kind").and_then(Value::as_str) {
                return Err(format!("file operations ({kind}) are not supported"));
            }
            let uri = change
                .get("textDocument")
                .and_then(|d| d.get("uri"))
                .and_then(Value::as_str)
                .ok_or("a document change without a document")?;
            let path = uri::str_to_path(uri).ok_or_else(|| format!("not a file: {uri}"))?;
            files.push((path, change.get("edits").unwrap_or(&Value::Null)));
        }
    } else if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let path = uri::str_to_path(uri).ok_or_else(|| format!("not a file: {uri}"))?;
            files.push((path, edits));
        }
    }
    let mut edit = WorkspaceEdit::default();
    for (path, edits) in files {
        let text = text_of(&path)?;
        let edits = text_edits(edits, &text, encoding);
        match edit.files.iter_mut().find(|f| f.path == path) {
            Some(file) => file.edits.extend(edits),
            None => edit.files.push(FileEdits { path, edits }),
        }
    }
    Ok(edit)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use text::Rope;

    use super::*;

    fn texts(path: &Path) -> Result<Text, String> {
        match path.file_name().and_then(|n| n.to_str()) {
            Some("a.py") => Ok(Text::new(Rope::from_str("import os\nx = \"🦀\"\n"))),
            Some("b.py") => Ok(Text::new(Rope::from_str("pass\n"))),
            _ => Err(format!("cannot read {}", path.display())),
        }
    }

    fn file(name: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\p\{name}"))
        } else {
            PathBuf::from(format!("/p/{name}"))
        }
    }

    fn uri_of(name: &str) -> String {
        uri::from_path(&file(name)).to_string()
    }

    #[test]
    fn changes_and_document_changes() {
        let edit = json!({
            "changes": {
                uri_of("a.py"): [
                    {"range": {"start": {"line": 1, "character": 5}, "end": {"line": 1, "character": 7}}, "newText": "x"},
                    {"range": {"start": {"line": 0}}, "newText": "skipped"},
                ],
            },
        });
        let converted = workspace_edit(&edit, PositionEncoding::Utf16, &texts).unwrap();
        assert_eq!(converted.files.len(), 1);
        assert_eq!(converted.files[0].path, file("a.py"));
        // UTF-16 columns 5..7 are the crab, one char.
        assert_eq!(
            converted.files[0].edits,
            [TextEdit::new(Point::new(1, 5)..Point::new(1, 6), "x")]
        );

        let edit = json!({
            "documentChanges": [
                {"textDocument": {"uri": uri_of("b.py"), "version": 3}, "edits": [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}}, "newText": "...", "annotationId": "x"},
                ]},
                {"textDocument": {"uri": uri_of("a.py"), "version": null}, "edits": []},
            ],
            "changes": {uri_of("a.py"): []},
        });
        let converted = workspace_edit(&edit, PositionEncoding::Utf8, &texts).unwrap();
        assert_eq!(converted.files.len(), 2);
        assert_eq!(converted.files[0].path, file("b.py"));
        assert_eq!(
            converted.files[0].edits,
            [TextEdit::new(Point::new(0, 0)..Point::new(0, 4), "...")]
        );
    }

    #[test]
    fn unsupported_edits_fail() {
        let create = json!({"documentChanges": [{"kind": "create", "uri": uri_of("c.py")}]});
        assert!(
            workspace_edit(&create, PositionEncoding::Utf16, &texts)
                .unwrap_err()
                .contains("create")
        );
        let unreadable = json!({"changes": {uri_of("missing.py"): []}});
        assert!(workspace_edit(&unreadable, PositionEncoding::Utf16, &texts).is_err());
        let other = json!({"changes": {"untitled:1": []}});
        assert!(workspace_edit(&other, PositionEncoding::Utf16, &texts).is_err());
        assert_eq!(
            workspace_edit(&json!(null), PositionEncoding::Utf16, &texts).unwrap(),
            WorkspaceEdit::default()
        );
    }
}
