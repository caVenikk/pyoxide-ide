//! The type server's adjustments: its completion items as Python's
//! ([`crate::completion`]), and the severities of its problems as the
//! reference's own type engine reports them.
//!
//! In the reference the type server stands in for its own engine, which
//! reports names and imports that do not resolve as errors (red) and its
//! type checks as warnings (`bad: int = "text"` has a yellow wave). The
//! type servers report all of these as errors, so a loosely typed file
//! shows hundreds of red problems where the reference shows a few
//! warnings. Errors stay errors for syntax errors (no rule, or ty's
//! `invalid-syntax`), names defined nowhere and imports that do not
//! resolve; every other error of the type server is a warning. Warnings,
//! weak warnings and hints stay as they are. (The reference's own
//! inspection of unresolved references is a warning that highlights
//! unknown names like errors; its type checker and its check of calls of
//! things that cannot be called were not in the sources at hand, so they
//! follow the rule above.)

use code_intel::{CompletionItem, Diagnostic, DocumentId, Severity};
use lsp_client::ServerCustomizer;

use crate::completion::present;
use crate::settings::Tool;

/// ty's rules whose errors stay errors.
pub const TY_ERRORS: [&str; 3] = [
    "invalid-syntax",
    "unresolved-reference",
    "unresolved-import",
];
/// basedpyright's rules whose errors stay errors (its syntax errors have
/// no rule).
pub const BASEDPYRIGHT_ERRORS: [&str; 2] = ["reportUndefinedVariable", "reportMissingImports"];

/// The type server's [`ServerCustomizer`].
#[derive(Debug, Clone, Copy)]
pub struct TypeServerCustomizer {
    tool: Tool,
}

impl TypeServerCustomizer {
    pub fn new(tool: Tool) -> Self {
        Self { tool }
    }
}

impl ServerCustomizer for TypeServerCustomizer {
    fn diagnostic(&self, diagnostic: &mut Diagnostic) {
        diagnostic.severity = severity(self.tool, diagnostic.code.as_deref(), diagnostic.severity);
    }

    fn completion_items(&self, _: &DocumentId, items: &mut Vec<CompletionItem>) {
        for item in items.iter_mut() {
            present(item);
        }
    }
}

/// The severity of a type server's problem with the rule `code`: an error
/// stays an error for syntax, unresolved names and imports, else it is a
/// warning; other severities stay.
pub fn severity(tool: Tool, code: Option<&str>, severity: Severity) -> Severity {
    if severity != Severity::Error {
        return severity;
    }
    let stays = match (tool, code) {
        (_, None) => true,
        (Tool::Ty, Some(code)) => TY_ERRORS.contains(&code),
        (Tool::Basedpyright, Some(code)) => BASEDPYRIGHT_ERRORS.contains(&code),
        (Tool::Ruff, Some(_)) => true,
    };
    if stays {
        Severity::Error
    } else {
        Severity::Warning
    }
}

#[cfg(test)]
mod tests {
    use code_intel::Point;

    use super::*;

    /// A problem as the client reports it, before the customizer.
    fn problem(code: Option<&str>, severity: Severity, message: &str) -> Diagnostic {
        let mut d = Diagnostic::new(Point::new(0, 0)..Point::new(0, 1), severity, message);
        d.code = code.map(str::to_owned);
        d
    }

    fn after(tool: Tool, mut d: Diagnostic) -> Severity {
        TypeServerCustomizer::new(tool).diagnostic(&mut d);
        d.severity
    }

    #[test]
    fn ty_problems() {
        use Severity::*;
        // What ty 0.0.87 reports on a sample, all as errors.
        let cases = [
            (
                "unresolved-import",
                "Cannot resolve imported module `missing_module`",
                Error,
            ),
            (
                "unresolved-import",
                "Module `os` has no member `not_there`",
                Error,
            ),
            (
                "invalid-assignment",
                "Object of type `Literal[\"text\"]` is not assignable to `int`",
                Warning,
            ),
            (
                "invalid-argument-type",
                "Argument to function `greet` is incorrect: Expected `str`, found `Literal[1]`",
                Warning,
            ),
            (
                "unresolved-reference",
                "Name `undefined_name` used when not defined",
                Error,
            ),
            (
                "call-non-callable",
                "Object of type `Literal[3]` is not callable",
                Warning,
            ),
            (
                "unresolved-attribute",
                "Object of type `Literal[\"text\"]` has no attribute `nope`",
                Warning,
            ),
            ("invalid-syntax", "unexpected EOF while parsing", Error),
        ];
        for (code, message, want) in cases {
            assert_eq!(
                after(Tool::Ty, problem(Some(code), Error, message)),
                want,
                "{code}"
            );
        }
        // Other severities stay.
        assert_eq!(
            after(
                Tool::Ty,
                problem(Some("possibly-unresolved-reference"), Warning, "")
            ),
            Warning
        );
        assert_eq!(after(Tool::Ty, problem(None, Information, "")), Information);
        assert_eq!(
            after(Tool::Ty, problem(None, Error, "a syntax error")),
            Error
        );
    }

    #[test]
    fn basedpyright_problems() {
        use Severity::*;
        // What basedpyright 1.40.2 reports in standard mode.
        let cases = [
            (
                Some("reportMissingImports"),
                "Import \"missing_module\" could not be resolved",
                Error,
            ),
            (
                Some("reportAttributeAccessIssue"),
                "\"not_there\" is unknown import symbol",
                Warning,
            ),
            (
                Some("reportAssignmentType"),
                "Type \"Literal['text']\" is not assignable to declared type \"int\"",
                Warning,
            ),
            (
                Some("reportArgumentType"),
                "Argument of type \"Literal[1]\" cannot be assigned to parameter \"name\"",
                Warning,
            ),
            (
                Some("reportUndefinedVariable"),
                "\"undefined_name\" is not defined",
                Error,
            ),
            (
                Some("reportCallIssue"),
                "Object of type \"Literal[3]\" is not callable",
                Warning,
            ),
            (None, "\"[\" was not closed", Error),
            (
                None,
                "Statements must be separated by newlines or semicolons",
                Error,
            ),
        ];
        for (code, message, want) in cases {
            assert_eq!(
                after(Tool::Basedpyright, problem(code, Error, message)),
                want,
                "{message}"
            );
        }
        assert_eq!(
            after(
                Tool::Basedpyright,
                problem(Some("reportUnusedImport"), Information, "")
            ),
            Information
        );
        // ruff's own severities stay.
        assert_eq!(severity(Tool::Ruff, Some("F821"), Error), Error);
    }
}
