mod layout;
mod out;
mod style;
mod words;

use std::ops::Range;

use crate::dialect::Dialect;
use crate::script::{TextEdit, parse};

pub use style::{
    AndOr, Case, ClauseElements, CodeStyle, Collapse, CommaPlacement, FunctionCase, SelectItems,
    StyleError,
};

/// Formats a script (or one statement) by `style`. Each statement is laid
/// out from its syntax tree: clauses start lines, lists and conditions break
/// as the style says, keywords and names change case as the style says.
/// Nothing else changes: the tokens are the same, strings, quoted names,
/// comments and `$$` bodies are copied as they are, and a statement with a
/// syntax error, or of a kind the formatter does not lay out yet, is left as
/// written. Formatting the result again gives the same text.
pub fn format(text: &str, dialect: Dialect, style: &CodeStyle) -> String {
    if style.disable_formatting {
        return text.to_string();
    }
    let crlf = text.contains("\r\n");
    let normalized;
    let src = if crlf {
        normalized = text.replace("\r\n", "\n");
        normalized.as_str()
    } else {
        text
    };
    let script = parse(src, dialect);
    let out = layout::run(&script, style);
    if out.is_empty() {
        return text.to_string();
    }
    if crlf { out.replace('\n', "\r\n") } else { out }
}

/// Formats the statements that overlap `range` (whole statements, with the
/// comments after them); `None` when the text would not change.
pub fn format_range(
    text: &str,
    range: Range<usize>,
    dialect: Dialect,
    style: &CodeStyle,
) -> Option<TextEdit> {
    if style.disable_formatting {
        return None;
    }
    let script = parse(text, dialect);
    let mut start = None;
    let mut end = 0;
    for root in script.statements() {
        let r = root.range();
        let touches =
            r.start < range.end.max(range.start + 1) && range.start < r.end.max(r.start + 1);
        if touches {
            start.get_or_insert(r.start);
            end = r.end;
        }
    }
    let start = start?;
    let slice = &text[start..end];
    let formatted = format(slice, dialect, style);
    (formatted != slice).then_some(TextEdit {
        range: start..end,
        text: formatted,
    })
}
