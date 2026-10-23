use std::ops::Range;

use crate::dialect::{Dialect, ParamForms};
use crate::lexer::{TokenKind, lex_with};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamKind {
    /// `?`
    Question,
    /// `?1`, `$1`, `:1`
    Numbered(u32),
    /// `:name`, `@name`, `$name`, `${name}`, `%(name)s`
    Named,
    /// `%s`
    Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub range: Range<usize>,
    pub kind: ParamKind,
    /// The name of a named parameter, without its prefix and brackets.
    pub name: Option<String>,
}

/// The parameters of a statement, in order, with the forms in `forms` (the
/// dialect's by default: [`Dialect::default_params`]). Strings, comments and
/// quoted names hide their content; `::int`, `:=`, `->>` and `@>` are
/// operators, not parameters.
pub fn parameters(src: &str, dialect: Dialect, forms: ParamForms) -> Vec<Parameter> {
    lex_with(src, dialect, forms)
        .into_iter()
        .filter(|t| t.kind == TokenKind::Parameter)
        .map(|t| {
            let (kind, name) = describe(t.text(src));
            Parameter {
                range: t.range(),
                kind,
                name,
            }
        })
        .collect()
}

fn describe(text: &str) -> (ParamKind, Option<String>) {
    if text == "?" {
        return (ParamKind::Question, None);
    }
    if text == "%s" {
        return (ParamKind::Format, None);
    }
    if let Some(inner) = text.strip_prefix("%(") {
        let name = inner.strip_suffix(")s").unwrap_or(inner);
        return (ParamKind::Named, Some(name.to_string()));
    }
    if let Some(inner) = text.strip_prefix("${") {
        let name = inner.strip_suffix('}').unwrap_or(inner);
        return (ParamKind::Named, Some(name.to_string()));
    }
    let rest = &text[1..];
    match rest.parse::<u32>() {
        Ok(n) => (ParamKind::Numbered(n), None),
        Err(_) if rest.is_empty() => (ParamKind::Question, None),
        Err(_) => (ParamKind::Named, Some(rest.to_string())),
    }
}
