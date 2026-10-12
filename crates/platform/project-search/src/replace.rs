//! Replace in Files: the replacements of a search in a text, and replacing
//! in a file on disk that keeps its encoding and line separator.

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use project::LoadError;
use search::{Matcher, ReplaceError};
use text::Point;

/// One replacement: a char range of the text and what replaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub range: Range<usize>,
    pub text: String,
}

/// Why a file was not changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileReplaceError {
    /// The file cannot be written, or not without changing it elsewhere
    /// (bytes that do not decode).
    ReadOnly,
    /// The replacement template does not fit the query.
    Template(String),
    /// Reading or writing failed.
    Io(String),
}

impl std::fmt::Display for FileReplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileReplaceError::ReadOnly => f.write_str("the file is read-only"),
            FileReplaceError::Template(e) => write!(f, "malformed replacement: {e}"),
            FileReplaceError::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for FileReplaceError {}

/// The replacements of the matches of `matcher` in `text` (`\n` line
/// breaks), as char ranges in text order. With `only`, just the matches
/// that start at these points (line and char column).
pub fn replacements(
    text: &str,
    matcher: &Matcher,
    template: &str,
    only: Option<&HashSet<Point>>,
) -> Result<Vec<Replacement>, ReplaceError> {
    let mut out = Vec::new();
    // Line, its start and the char offset of the last match, counted
    // forwards.
    let (mut line, mut line_start, mut pos, mut chars) = (0usize, 0usize, 0usize, 0usize);
    for m in matcher.find_iter(text) {
        let between = &text[pos..m.start];
        chars += between.chars().count();
        for (i, b) in between.bytes().enumerate() {
            if b == b'\n' {
                line += 1;
                line_start = pos + i + 1;
            }
        }
        pos = m.start;
        if let Some(only) = only {
            let column = text[line_start..m.start].chars().count();
            if !only.contains(&Point::new(line, column)) {
                continue;
            }
        }
        let len = text[m.clone()].chars().count();
        out.push(Replacement {
            range: chars..chars + len,
            text: matcher.replacement(text, m, template)?,
        });
    }
    Ok(out)
}

/// The text with the replacements made (char ranges in text order).
pub fn apply(text: &str, replacements: &[Replacement]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().map(|(i, _)| i).chain([text.len()]);
    let mut byte_at = {
        let mut seen = 0usize;
        let mut last = 0usize;
        move |char_offset: usize| {
            while seen <= char_offset {
                last = chars.next().unwrap_or(text.len());
                seen += 1;
            }
            last
        }
    };
    let mut copied = 0usize;
    for r in replacements {
        let start = byte_at(r.range.start);
        let end = byte_at(r.range.end);
        out.push_str(&text[copied..start]);
        out.push_str(&r.text);
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// Replaces in a file that is not open: read and decoded as the editor
/// would, replaced, and written back in its encoding with its line
/// separator. Returns the number of replacements (0: the file is left
/// alone).
pub fn replace_in_file(
    path: &Path,
    matcher: &Matcher,
    template: &str,
    only: Option<&HashSet<Point>>,
) -> Result<usize, FileReplaceError> {
    let loaded = project::read_document(path).map_err(|e| match e {
        LoadError::Io(e) => FileReplaceError::Io(e),
        other => FileReplaceError::Io(other.to_string()),
    })?;
    let decoded = loaded.decoded;
    let edits = replacements(&decoded.text, matcher, template, only)
        .map_err(|e| FileReplaceError::Template(e.to_string()))?;
    if edits.is_empty() {
        return Ok(0);
    }
    if loaded.read_only || decoded.lossy {
        return Err(FileReplaceError::ReadOnly);
    }
    let text = apply(&decoded.text, &edits);
    let bytes = text::encode(&text, decoded.line_ending, decoded.encoding);
    project::write_document(path, &bytes).map_err(|e| FileReplaceError::Io(e.to_string()))?;
    Ok(edits.len())
}

#[cfg(test)]
mod tests {
    use search::{Query, SearchOptions};

    use super::*;

    fn matcher(text: &str, regex: bool) -> Matcher {
        let options = SearchOptions {
            regex,
            ..SearchOptions::default()
        };
        Query::new(text, options).compile().unwrap()
    }

    #[test]
    fn replacements_in_text_and_only_some() {
        let text = "ä foo\nfoo foo\n";
        let m = matcher("foo", false);
        let all = replacements(text, &m, "bar", None).unwrap();
        assert_eq!(
            all.iter().map(|r| r.range.clone()).collect::<Vec<_>>(),
            vec![2..5, 6..9, 10..13]
        );
        assert_eq!(apply(text, &all), "ä bar\nbar bar\n");
        let only = HashSet::from([Point::new(1, 4)]);
        let some = replacements(text, &m, "x", Some(&only)).unwrap();
        assert_eq!(apply(text, &some), "ä foo\nfoo x\n");
        // Groups of a regular expression.
        let m = matcher(r"f(o+)", true);
        let r = replacements("foo fooo", &m, "<$1>", None).unwrap();
        assert_eq!(apply("foo fooo", &r), "<oo> <ooo>");
    }

    #[test]
    fn files_keep_their_encoding_and_line_separator() {
        let dir = tempfile::tempdir().unwrap();
        let crlf = dir.path().join("crlf.txt");
        std::fs::write(&crlf, b"one foo\r\ntwo foo\r\n").unwrap();
        let utf16 = dir.path().join("utf16.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "café foo\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&utf16, &bytes).unwrap();
        let m = matcher("foo", false);
        assert_eq!(replace_in_file(&crlf, &m, "bar", None), Ok(2));
        assert_eq!(std::fs::read(&crlf).unwrap(), b"one bar\r\ntwo bar\r\n");
        assert_eq!(replace_in_file(&utf16, &m, "bär", None), Ok(1));
        let back = text::decode(&std::fs::read(&utf16).unwrap()).unwrap();
        assert_eq!(back.text, "café bär\n");
        assert_eq!(back.encoding, text::Encoding::Utf16Le);
        // Nothing to replace: the file is not written.
        assert_eq!(replace_in_file(&crlf, &m, "bar", None), Ok(0));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_files_are_reported() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ro.txt");
        std::fs::write(&path, "foo\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        let m = matcher("foo", false);
        assert_eq!(
            replace_in_file(&path, &m, "bar", None),
            Err(FileReplaceError::ReadOnly)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "foo\n");
    }
}
