//! Names that highlighters use for tokens, resolved to [`TextRole`]s: capture
//! names of tree-sitter highlight queries and the token types and modifiers
//! of LSP semantic tokens. `docs/theme-mapping.md` lists the same tables.

use crate::roles::TextRole;

/// Capture name to role. A capture that is not listed uses the longest listed
/// prefix at a dot boundary, so `keyword.control.return` is a `keyword`.
const CAPTURES: &[(&str, TextRole)] = &[
    ("keyword", TextRole::Keyword),
    ("boolean", TextRole::ConstantBuiltin),
    ("constant", TextRole::Constant),
    ("constant.builtin", TextRole::ConstantBuiltin),
    ("constant.numeric", TextRole::Number),
    ("number", TextRole::Number),
    ("float", TextRole::Number),
    ("string", TextRole::String),
    ("character", TextRole::String),
    ("string.escape", TextRole::StringEscape),
    ("string.documentation", TextRole::Docstring),
    ("string.special.key", TextRole::PropertyKey),
    ("string.special.url", TextRole::Link),
    ("comment", TextRole::Comment),
    ("comment.documentation", TextRole::Docstring),
    ("comment.todo", TextRole::Todo),
    ("comment.note", TextRole::Todo),
    ("comment.warning", TextRole::Todo),
    ("comment.error", TextRole::Todo),
    ("function", TextRole::FunctionDecl),
    ("function.call", TextRole::FunctionCall),
    ("function.macro", TextRole::FunctionCall),
    ("function.method", TextRole::MethodDecl),
    ("function.method.call", TextRole::MethodCall),
    ("function.builtin", TextRole::Builtin),
    ("function.decorator", TextRole::Decorator),
    ("constructor", TextRole::Class),
    ("type", TextRole::Class),
    ("type.builtin", TextRole::Builtin),
    ("type.definition", TextRole::ClassDecl),
    ("type.annotation", TextRole::Annotation),
    ("annotation", TextRole::Annotation),
    ("attribute", TextRole::Decorator),
    ("variable", TextRole::Variable),
    ("variable.parameter", TextRole::Parameter),
    ("variable.builtin", TextRole::SelfParameter),
    ("variable.member", TextRole::Attribute),
    ("parameter", TextRole::Parameter),
    ("property", TextRole::Attribute),
    ("field", TextRole::Attribute),
    ("module", TextRole::Module),
    ("namespace", TextRole::Module),
    ("operator", TextRole::Operator),
    ("punctuation", TextRole::Punctuation),
    ("punctuation.delimiter", TextRole::Punctuation),
    ("punctuation.bracket", TextRole::Bracket),
    ("punctuation.special", TextRole::FstringBrace),
    ("embedded", TextRole::FstringExpression),
    ("tag", TextRole::Tag),
    ("tag.attribute", TextRole::TagAttribute),
    ("tag.delimiter", TextRole::Punctuation),
    ("markup.link", TextRole::Link),
    ("text.uri", TextRole::Link),
];

impl TextRole {
    /// The role for a tree-sitter highlight capture such as `@keyword.control`.
    /// The leading `@` is optional.
    pub fn from_capture(name: &str) -> Option<TextRole> {
        let mut name = name.strip_prefix('@').unwrap_or(name);
        loop {
            if let Some((_, role)) = CAPTURES.iter().find(|(n, _)| *n == name) {
                return Some(*role);
            }
            name = name.rsplit_once('.')?.0;
        }
    }

    /// The role for an LSP semantic token. `self` and `cls` arrive as
    /// `parameter`; telling them apart needs the token text, so the caller
    /// maps those to [`TextRole::SelfParameter`] itself. The `deprecated`
    /// modifier is an overlay: apply [`TextRole::DiagnosticDeprecated`] on top
    /// of the role returned here.
    pub fn from_semantic_token(token_type: &str, modifiers: &[&str]) -> Option<TextRole> {
        let has = |m: &str| modifiers.contains(&m);
        let declaration = has("declaration") || has("definition");
        let role = match token_type {
            "namespace" => TextRole::Module,
            "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => {
                if declaration {
                    TextRole::ClassDecl
                } else {
                    TextRole::Class
                }
            }
            "parameter" => TextRole::Parameter,
            "variable" => {
                if has("readonly") {
                    TextRole::Constant
                } else {
                    TextRole::Variable
                }
            }
            "property" | "event" => TextRole::Attribute,
            "enumMember" => TextRole::Constant,
            "function" => {
                if declaration {
                    TextRole::FunctionDecl
                } else {
                    TextRole::FunctionCall
                }
            }
            "method" => {
                if declaration {
                    TextRole::MethodDecl
                } else {
                    TextRole::MethodCall
                }
            }
            "macro" => TextRole::FunctionCall,
            "keyword" | "modifier" => TextRole::Keyword,
            "comment" => {
                if has("documentation") {
                    TextRole::Docstring
                } else {
                    TextRole::Comment
                }
            }
            "string" | "regexp" => TextRole::String,
            "number" => TextRole::Number,
            "operator" => TextRole::Operator,
            "decorator" => TextRole::Decorator,
            _ => return None,
        };
        let library = has("defaultLibrary")
            && matches!(
                token_type,
                "function" | "method" | "class" | "type" | "variable" | "struct" | "enum"
            );
        Some(if library { TextRole::Builtin } else { role })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_fall_back_to_their_prefix() {
        assert_eq!(TextRole::from_capture("@keyword"), Some(TextRole::Keyword));
        assert_eq!(
            TextRole::from_capture("keyword.control.return"),
            Some(TextRole::Keyword)
        );
        assert_eq!(
            TextRole::from_capture("@function.method.call"),
            Some(TextRole::MethodCall)
        );
        assert_eq!(
            TextRole::from_capture("function.method.special"),
            Some(TextRole::MethodDecl)
        );
        assert_eq!(
            TextRole::from_capture("string.special.key"),
            Some(TextRole::PropertyKey)
        );
        assert_eq!(
            TextRole::from_capture("string.regexp"),
            Some(TextRole::String)
        );
        assert_eq!(
            TextRole::from_capture("number.float"),
            Some(TextRole::Number)
        );
        assert_eq!(TextRole::from_capture("unheard.of"), None);
        assert_eq!(TextRole::from_capture(""), None);
    }

    #[test]
    fn python_captures_resolve_to_the_python_tokens() {
        for (capture, role) in [
            ("variable.parameter", TextRole::Parameter),
            ("variable.builtin", TextRole::SelfParameter),
            ("function.builtin", TextRole::Builtin),
            ("type.builtin", TextRole::Builtin),
            ("constant.builtin", TextRole::ConstantBuiltin),
            ("attribute", TextRole::Decorator),
            ("punctuation.special", TextRole::FstringBrace),
            ("string.escape", TextRole::StringEscape),
            ("comment.documentation", TextRole::Docstring),
        ] {
            assert_eq!(TextRole::from_capture(capture), Some(role), "{capture}");
        }
    }

    #[test]
    fn semantic_tokens_use_declaration_and_library_modifiers() {
        let role = TextRole::from_semantic_token;
        assert_eq!(
            role("function", &["declaration"]),
            Some(TextRole::FunctionDecl)
        );
        assert_eq!(role("function", &[]), Some(TextRole::FunctionCall));
        assert_eq!(role("method", &["definition"]), Some(TextRole::MethodDecl));
        assert_eq!(role("class", &["declaration"]), Some(TextRole::ClassDecl));
        assert_eq!(role("class", &[]), Some(TextRole::Class));
        assert_eq!(
            role("function", &["defaultLibrary"]),
            Some(TextRole::Builtin)
        );
        assert_eq!(role("class", &["defaultLibrary"]), Some(TextRole::Builtin));
        assert_eq!(role("variable", &["readonly"]), Some(TextRole::Constant));
        assert_eq!(
            role("comment", &["documentation"]),
            Some(TextRole::Docstring)
        );
        assert_eq!(role("decorator", &[]), Some(TextRole::Decorator));
        assert_eq!(role("namespace", &[]), Some(TextRole::Module));
        assert_eq!(role("keyword", &[]), Some(TextRole::Keyword));
        assert_eq!(role("nonsense", &[]), None);
    }
}
