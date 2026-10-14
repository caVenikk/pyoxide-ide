//! Problems shown in an editor: the editor's own marks, so that it knows
//! no language service. Whoever has diagnostics (the code intelligence
//! controller) turns them into [`DiagnosticMark`]s.

use std::ops::Range;

use gpui::SharedString;

/// How a mark is drawn, from the most severe (the reference's highlight
/// levels: error, warning, weak warning, information).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MarkSeverity {
    /// A wave in the error colour; counted in the inspection widget.
    Error,
    /// A wave in the warning colour; counted.
    Warning,
    /// A wave in the weak warning colour; counted.
    WeakWarning,
    /// No mark in the text and not counted; its messages show in the
    /// tooltip of the range.
    Information,
}

/// One message of a mark (marks of several services at one range carry
/// all their messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkMessage {
    pub text: SharedString,
    /// Who reports it (`Ruff`), shown grey after the text.
    pub source: Option<SharedString>,
    /// The rule (`unused-import`), shown in brackets after the source.
    pub code: Option<SharedString>,
    /// Documentation of the rule: the code is a link to it.
    pub href: Option<SharedString>,
}

/// A problem in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticMark {
    /// Char range in the text (may be empty: a mark at a position).
    pub range: Range<usize>,
    pub severity: MarkSeverity,
    /// Greyed like unused code.
    pub unnecessary: bool,
    /// Struck out.
    pub deprecated: bool,
    pub messages: Vec<MarkMessage>,
}
