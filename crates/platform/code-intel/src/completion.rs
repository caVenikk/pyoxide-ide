//! Completion items and the kinds of symbols.

use std::ops::Range;

use serde::Serialize;
use text::Point;

use crate::document::Token;
use crate::edit::TextEdit;
use crate::markup::Markup;
use crate::server::ServerId;

/// What a completion item or a symbol is: the LSP completion item kinds
/// and symbol kinds in one list. The UI picks the icon (the reference's
/// `LspCompletionCustomizer.getIcon`) and shows keywords bold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
    Folder,
    EnumMember,
    Constant,
    Struct,
    Event,
    Operator,
    TypeParameter,
    Namespace,
    Package,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
}

/// How a completion item goes into the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InsertText {
    /// Replaces the identifier typed before the caret (LSP `insertText`,
    /// or the label).
    Text(String),
    /// Replaces a range of the document (LSP `textEdit`): `insert` when
    /// the item is chosen with Enter, `replace` with Tab (the same range
    /// unless the server gives both).
    Edit {
        insert: Range<Point>,
        replace: Range<Point>,
        text: String,
    },
}

impl InsertText {
    pub fn text(&self) -> &str {
        match self {
            InsertText::Text(text) | InsertText::Edit { text, .. } => text,
        }
    }
}

/// One completion item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompletionItem {
    /// The name shown.
    pub label: String,
    pub kind: Option<SymbolKind>,
    /// Grey text right after the name (LSP `labelDetails.detail`, for
    /// example ` (import collections)`).
    pub tail_text: Option<String>,
    /// Grey text at the right edge (LSP `labelDetails.description`, else
    /// `detail`): the type or the module.
    pub type_text: Option<String>,
    /// The server's detail (a signature, a type).
    pub detail: Option<String>,
    /// Text matched against the typed prefix; the label when `None`.
    pub filter_text: Option<String>,
    /// Text the server orders items by; the label when `None`.
    pub sort_text: Option<String>,
    pub insert: InsertText,
    /// The insert text is a snippet (`$1`, `${2:name}` placeholders).
    pub snippet: bool,
    /// Edits elsewhere in the document made with the insertion, in the same
    /// undo step (an import).
    pub additional_edits: Vec<TextEdit>,
    pub documentation: Option<Markup>,
    /// Struck out.
    pub deprecated: bool,
    /// The server asks to select this item first.
    pub preselect: bool,
    /// The server that offered it.
    pub server: Option<ServerId>,
    /// The implementation's data, for [`CodeIntel::resolve_completion`](crate::CodeIntel::resolve_completion).
    #[serde(skip)]
    pub token: Option<Token>,
}

impl CompletionItem {
    /// An item that inserts its label.
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            insert: InsertText::Text(label.clone()),
            label,
            kind: None,
            tail_text: None,
            type_text: None,
            detail: None,
            filter_text: None,
            sort_text: None,
            snippet: false,
            additional_edits: Vec::new(),
            documentation: None,
            deprecated: false,
            preselect: false,
            server: None,
            token: None,
        }
    }

    pub fn with_kind(mut self, kind: SymbolKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// The text matched against what was typed.
    pub fn filter_text(&self) -> &str {
        self.filter_text.as_deref().unwrap_or(&self.label)
    }

    /// The text the server orders items by.
    pub fn sort_text(&self) -> &str {
        self.sort_text.as_deref().unwrap_or(&self.label)
    }
}

/// The answer of a completion request.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Completions {
    pub items: Vec<CompletionItem>,
    /// The server filtered the items by what was typed so far: typing more
    /// asks again ([`CompletionTrigger::Incomplete`]) instead of filtering
    /// the list.
    pub incomplete: bool,
}

/// Why completion is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionTrigger {
    /// Explicitly (Basic Completion), or by typing an identifier.
    Invoked,
    /// By typing one of the server's trigger characters (`.`).
    Character(char),
    /// Typing went on after an incomplete answer.
    Incomplete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_default_to_their_label() {
        let mut item = CompletionItem::new("norm").with_kind(SymbolKind::Method);
        assert_eq!(item.insert.text(), "norm");
        assert_eq!(item.filter_text(), "norm");
        assert_eq!(item.sort_text(), "norm");
        item.filter_text = Some("Norm".into());
        item.sort_text = Some("0001".into());
        assert_eq!((item.filter_text(), item.sort_text()), ("Norm", "0001"));
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["kind"], "method");
        assert_eq!(json["insert"]["text"], "norm");
        let edit = InsertText::Edit {
            insert: Point::new(0, 2)..Point::new(0, 4),
            replace: Point::new(0, 2)..Point::new(0, 6),
            text: "norm()".into(),
        };
        assert_eq!(edit.text(), "norm()");
    }
}
