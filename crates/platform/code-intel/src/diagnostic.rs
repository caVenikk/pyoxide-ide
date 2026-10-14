//! Problems a service reports in a document.

use std::ops::Range;

use serde::Serialize;
use text::Point;

use crate::document::Token;
use crate::server::ServerId;

/// How bad a problem is, ordered from the most severe. The reference maps
/// LSP severities to its own levels (`LspDiagnosticsCustomizer`): Error to
/// error, Warning to warning, Information to weak warning, Hint to
/// information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// A red wave; counted, first for Next Highlighted Error.
    Error,
    /// A yellow wave; counted.
    Warning,
    /// A wave in the weak warning colour; counted.
    WeakWarning,
    /// No mark in the text and not counted (shown in tooltips only).
    Information,
}

impl Severity {
    /// Counted in the inspection widget and visited by Next Highlighted
    /// Error.
    pub fn is_problem(self) -> bool {
        self != Severity::Information
    }
}

/// A problem in a document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    pub range: Range<Point>,
    pub severity: Severity,
    /// The message; the first line is its summary.
    pub message: String,
    /// Who reports it (`Ruff`, `ty`).
    pub source: Option<String>,
    /// The rule (`F401`, or its name once a product renamed it).
    pub code: Option<String>,
    /// Documentation of the rule.
    pub code_href: Option<String>,
    /// The code is unused: greyed like unused code (LSP tag Unnecessary).
    pub unnecessary: bool,
    /// The code is deprecated: struck out (LSP tag Deprecated).
    pub deprecated: bool,
    /// The server that reported it.
    pub server: Option<ServerId>,
    /// The implementation's data, sent back with code action requests.
    #[serde(skip)]
    pub token: Option<Token>,
}

impl Diagnostic {
    /// A diagnostic with a message and nothing else.
    pub fn new(range: Range<Point>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            range,
            severity,
            message: message.into(),
            source: None,
            code: None,
            code_href: None,
            unnecessary: false,
            deprecated: false,
            server: None,
            token: None,
        }
    }

    /// The text of the inspection widget and the Problems view, as the
    /// reference shows it: the first line of the message, two spaces, then
    /// the source and the code (`` `os` imported but unused  Ruff(F401) ``).
    pub fn summary(&self) -> String {
        let first = self.message.lines().next().unwrap_or_default();
        match (&self.source, &self.code) {
            (Some(source), Some(code)) => format!("{first}  {source}({code})"),
            (Some(source), None) => format!("{first}  {source}"),
            (None, Some(code)) => format!("{first}  ({code})"),
            (None, None) => first.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_and_severities() {
        let mut d = Diagnostic::new(
            Point::new(0, 7)..Point::new(0, 9),
            Severity::Warning,
            "`os` imported but unused\nmore",
        );
        assert_eq!(d.summary(), "`os` imported but unused");
        d.source = Some("Ruff".into());
        assert_eq!(d.summary(), "`os` imported but unused  Ruff");
        d.code = Some("unused-import".into());
        assert_eq!(d.summary(), "`os` imported but unused  Ruff(unused-import)");
        assert!(Severity::Error < Severity::Warning);
        assert!(Severity::WeakWarning.is_problem());
        assert!(!Severity::Information.is_problem());
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["severity"], "warning");
        assert_eq!(json["range"]["start"]["column"], 7);
        assert!(json.get("token").is_none());
    }
}
