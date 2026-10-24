//! The structure of a document and the targets of Go to Declaration.

use std::ops::Range;

use serde::Serialize;
use text::Point;

use crate::completion::SymbolKind;
use crate::edit::Location;

/// A symbol of a document's structure (the Structure view): a class, a
/// function, a statement, with the symbols declared inside it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentSymbol {
    pub name: String,
    /// More about it, shown after the name (a signature, a statement kind).
    pub detail: Option<String>,
    pub kind: SymbolKind,
    /// The whole symbol, with its body.
    pub range: Range<Point>,
    /// The part that names it (selected when the symbol is chosen).
    pub selection_range: Range<Point>,
    pub children: Vec<DocumentSymbol>,
}

impl DocumentSymbol {
    /// A symbol without detail or children, named by its whole range.
    pub fn new(name: impl Into<String>, kind: SymbolKind, range: Range<Point>) -> Self {
        Self {
            name: name.into(),
            detail: None,
            kind,
            selection_range: range.clone(),
            range,
            children: Vec::new(),
        }
    }
}

/// Something that is not a place in a file, such as a table of a
/// database: the workbench passes it to the handler registered for its
/// scheme (the database module registers `db`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ObjectTarget {
    /// Whose object it is (`db`).
    pub scheme: String,
    /// The object, in the form its handler understands.
    pub id: String,
    /// Its name as lists show it.
    pub title: String,
}

impl ObjectTarget {
    pub fn new(scheme: impl Into<String>, id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            scheme: scheme.into(),
            id: id.into(),
            title: title.into(),
        }
    }
}

/// Where Go to Declaration goes: a place in a file or an object.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefinitionTarget {
    File(Location),
    Object(ObjectTarget),
}
