//! Code actions: quick fixes, refactorings and source actions.

use std::fmt;

use serde::{Serialize, Serializer};

use crate::diagnostic::Diagnostic;
use crate::document::Token;
use crate::edit::WorkspaceEdit;
use crate::server::ServerId;

/// What a code action is (LSP `CodeActionKind`, a dotted hierarchy).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CodeActionKind {
    /// Fixes a problem (`quickfix` and its sub-kinds): shown first.
    QuickFix,
    /// A refactoring: the sub-kind after `refactor.` (`extract`), empty for
    /// `refactor` itself.
    Refactor(String),
    /// A whole-file action: the sub-kind after `source.`
    /// (`organizeImports`, `fixAll.ruff`), empty for `source` itself.
    Source(String),
    /// Any other kind as the server sent it (also an empty kind).
    Other(String),
}

impl CodeActionKind {
    pub fn parse(kind: &str) -> Self {
        let sub = |prefix: &str| {
            kind.strip_prefix(prefix).and_then(|rest| {
                if rest.is_empty() {
                    Some(String::new())
                } else {
                    rest.strip_prefix('.').map(str::to_owned)
                }
            })
        };
        if sub("quickfix").is_some() {
            CodeActionKind::QuickFix
        } else if let Some(sub) = sub("refactor") {
            CodeActionKind::Refactor(sub)
        } else if let Some(sub) = sub("source") {
            CodeActionKind::Source(sub)
        } else {
            CodeActionKind::Other(kind.to_owned())
        }
    }

    /// `source.organizeImports` (Optimize Imports).
    pub fn organize_imports() -> Self {
        CodeActionKind::Source("organizeImports".into())
    }

    pub fn is_quick_fix(&self) -> bool {
        *self == CodeActionKind::QuickFix
    }

    /// An action of this kind is asked for by `filter`: the same kind or
    /// one below it (`source` covers `source.organizeImports.ruff`).
    pub fn matches(&self, filter: &CodeActionKind) -> bool {
        let kind = self.to_string();
        let filter = filter.to_string();
        filter.is_empty()
            || kind == filter
            || kind
                .strip_prefix(&filter)
                .is_some_and(|rest| rest.starts_with('.'))
    }
}

impl fmt::Display for CodeActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prefix, sub) = match self {
            CodeActionKind::QuickFix => return f.write_str("quickfix"),
            CodeActionKind::Refactor(sub) => ("refactor", sub),
            CodeActionKind::Source(sub) => ("source", sub),
            CodeActionKind::Other(kind) => return f.write_str(kind),
        };
        if sub.is_empty() {
            f.write_str(prefix)
        } else {
            write!(f, "{prefix}.{sub}")
        }
    }
}

impl Serialize for CodeActionKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A server command to run (a code action without an edit).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Command {
    pub title: String,
    /// The implementation's data: the server, the command and its
    /// arguments.
    #[serde(skip)]
    pub token: Token,
}

/// A quick fix or another action at a range.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodeAction {
    pub title: String,
    pub kind: CodeActionKind,
    /// The fix the server recommends for its problem.
    pub preferred: bool,
    /// The problems it fixes.
    pub diagnostics: Vec<Diagnostic>,
    /// What it changes; `None` until resolved when the server computes it
    /// on demand, or when it is a command.
    pub edit: Option<WorkspaceEdit>,
    /// Runs after the edit is applied.
    pub command: Option<Command>,
    /// Sub-actions shown behind an arrow (a product nests "Disable for
    /// this line" under a fix).
    pub children: Vec<CodeAction>,
    /// The server that offered it.
    pub server: Option<ServerId>,
    /// The implementation's data, for
    /// [`CodeIntel::resolve_code_action`](crate::CodeIntel::resolve_code_action).
    #[serde(skip)]
    pub token: Option<Token>,
}

impl CodeAction {
    /// An action with a title and a kind and nothing else.
    pub fn new(title: impl Into<String>, kind: CodeActionKind) -> Self {
        Self {
            title: title.into(),
            kind,
            preferred: false,
            diagnostics: Vec::new(),
            edit: None,
            command: None,
            children: Vec::new(),
            server: None,
            token: None,
        }
    }
}

/// Why code actions are asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionTrigger {
    /// Explicitly (Show Context Actions, Optimize Imports).
    #[default]
    Invoked,
    /// Automatically (the light bulb).
    Automatic,
}

/// The context of a code action request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodeActionContext {
    /// Only actions of these kinds (and their sub-kinds); all with `None`.
    pub only: Option<Vec<CodeActionKind>>,
    /// The problems at the range, as received (with their tokens).
    pub diagnostics: Vec<Diagnostic>,
    pub trigger: ActionTrigger,
}

impl CodeActionContext {
    /// Optimize Imports: `source.organizeImports` actions only.
    pub fn organize_imports() -> Self {
        Self {
            only: Some(vec![CodeActionKind::organize_imports()]),
            ..Self::default()
        }
    }

    /// An action of this kind is asked for.
    pub fn wants(&self, kind: &CodeActionKind) -> bool {
        self.only
            .as_ref()
            .is_none_or(|only| only.iter().any(|f| kind.matches(f)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_parse_print_and_match() {
        for (text, kind) in [
            ("quickfix", CodeActionKind::QuickFix),
            ("refactor", CodeActionKind::Refactor(String::new())),
            (
                "refactor.extract",
                CodeActionKind::Refactor("extract".into()),
            ),
            ("source.organizeImports", CodeActionKind::organize_imports()),
            (
                "source.fixAll.ruff",
                CodeActionKind::Source("fixAll.ruff".into()),
            ),
            ("", CodeActionKind::Other(String::new())),
            ("sourcery", CodeActionKind::Other("sourcery".into())),
        ] {
            assert_eq!(CodeActionKind::parse(text), kind, "{text}");
            assert_eq!(kind.to_string(), text);
        }
        assert_eq!(
            CodeActionKind::parse("quickfix.ruff"),
            CodeActionKind::QuickFix
        );

        let ruff = CodeActionKind::parse("source.organizeImports.ruff");
        assert!(ruff.matches(&CodeActionKind::organize_imports()));
        assert!(ruff.matches(&CodeActionKind::Source(String::new())));
        assert!(!ruff.matches(&CodeActionKind::QuickFix));
        assert!(
            !CodeActionKind::Source("organizeImportsX".into())
                .matches(&CodeActionKind::organize_imports())
        );

        let context = CodeActionContext::organize_imports();
        assert!(context.wants(&ruff));
        assert!(!context.wants(&CodeActionKind::QuickFix));
        assert!(CodeActionContext::default().wants(&CodeActionKind::QuickFix));

        let action = CodeAction::new("Remove unused import", CodeActionKind::QuickFix);
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["kind"], "quickfix");
    }
}
