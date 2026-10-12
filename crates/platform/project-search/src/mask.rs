//! File masks of Find in Files: `*.py, !test_*.py`.

use regex::{Regex, RegexBuilder};

/// Which file names a search looks at.
///
/// The reference IDE's syntax: masks separated by commas, `*` for any run
/// of characters, `?` for one character, a leading `!` to leave matching
/// names out; names compare without regard to case. Only the file name
/// counts, not its folder. Without any mask that is not excluded, every
/// name matches (`!*.min.js` alone means "all but minified scripts").
#[derive(Debug, Clone)]
pub struct FileMask {
    text: String,
    include: Option<Regex>,
    exclude: Option<Regex>,
}

/// A mask that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskError(pub String);

impl std::fmt::Display for MaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid file mask: {}", self.0)
    }
}

impl std::error::Error for MaskError {}

impl FileMask {
    pub fn parse(text: &str) -> Result<Self, MaskError> {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for mask in text.split(',').map(str::trim).filter(|m| !m.is_empty()) {
            match mask.strip_prefix('!') {
                Some(rest) => exclude.push(wildcard(rest.trim())),
                None => include.push(wildcard(mask)),
            }
        }
        let compile = |parts: Vec<String>| -> Result<Option<Regex>, MaskError> {
            if parts.is_empty() {
                return Ok(None);
            }
            RegexBuilder::new(&format!("^(?:{})$", parts.join("|")))
                .case_insensitive(true)
                .build()
                .map(Some)
                .map_err(|e| MaskError(e.to_string()))
        };
        Ok(Self {
            text: text.to_owned(),
            include: compile(include)?,
            exclude: compile(exclude)?,
        })
    }

    /// The mask as typed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether a file of this name is searched.
    pub fn matches(&self, file_name: &str) -> bool {
        self.include.as_ref().is_none_or(|r| r.is_match(file_name))
            && self.exclude.as_ref().is_none_or(|r| !r.is_match(file_name))
    }
}

/// A wildcard (`*`, `?`) as a regular expression.
fn wildcard(mask: &str) -> String {
    let mut out = String::new();
    for c in mask.chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c => out.push_str(&regex::escape(c.encode_utf8(&mut [0; 4]))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_include_exclude_and_ignore_case() {
        let m = FileMask::parse("*.py, *.PYI ,!test_*.py").unwrap();
        assert!(m.matches("app.py"));
        assert!(m.matches("stubs.pyi"));
        assert!(m.matches("APP.PY"));
        assert!(!m.matches("test_app.py"));
        assert!(!m.matches("app.pyc"));
        assert!(!m.matches("readme.md"));
        // `?` is one character; other characters are literal.
        let m = FileMask::parse("a?.txt").unwrap();
        assert!(m.matches("ab.txt"));
        assert!(!m.matches("abc.txt"));
        assert!(!m.matches("ab_txt"));
        // Only exclusions: everything else.
        let m = FileMask::parse("!*.min.js").unwrap();
        assert!(m.matches("app.js"));
        assert!(!m.matches("app.min.js"));
        // Empty: everything.
        assert!(FileMask::parse(" , ").unwrap().matches("x"));
        assert_eq!(FileMask::parse("*.py").unwrap().text(), "*.py");
    }
}
