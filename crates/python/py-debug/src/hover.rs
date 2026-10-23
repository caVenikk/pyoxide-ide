//! The expression whose value the debugger shows under the mouse (the
//! reference's `PyDebuggerEvaluator.getExpressionRangeAtOffset` with
//! `filterExpressionRangeForHover`, and `evaluateWithOffset`): a reference
//! to a name, `summary` in `summary["sum"]`, or an attribute up to the
//! hovered name, `a.b` over `b` of `a.b.c`. Nothing for a literal, a
//! keyword, a comment, the name of a `def` or a `class`, a parameter, a
//! keyword argument's name, an imported name, or a function called
//! (hovering a call does not run it).

use std::ops::Range;

use tree_sitter::{Node, Parser};

/// The byte range of the expression at byte `offset` of `text`.
pub fn expression_at(text: &str, offset: usize) -> Option<Range<usize>> {
    if offset >= text.len() {
        return None;
    }
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let leaf = tree
        .root_node()
        .descendant_for_byte_range(offset, offset + 1)?;
    if leaf.kind() != "identifier" {
        return None;
    }
    let parent = leaf.parent()?;
    if !is_reference(leaf, parent) {
        return None;
    }
    // `b` of `a.b` stands for `a.b`.
    let element = match field_of(parent, leaf) {
        Some("attribute") if parent.kind() == "attribute" => parent,
        _ => leaf,
    };
    // A function called is not evaluated on hover.
    if let Some(outer) = element.parent()
        && outer.kind() == "call"
        && field_of(outer, element) == Some("function")
    {
        return None;
    }
    Some(element.byte_range())
}

/// The field of `parent` that holds `child`.
fn field_of(parent: Node, child: Node) -> Option<&'static str> {
    let mut cursor = parent.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        if cursor.node() == child {
            return cursor.field_name();
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

/// An identifier that refers to a value rather than declaring a function,
/// a class, a parameter, a keyword argument or an imported name.
fn is_reference(leaf: Node, parent: Node) -> bool {
    match parent.kind() {
        "function_definition" | "class_definition" => field_of(parent, leaf) != Some("name"),
        "keyword_argument" => field_of(parent, leaf) != Some("name"),
        "parameters"
        | "lambda_parameters"
        | "typed_parameter"
        | "default_parameter"
        | "typed_default_parameter"
        | "list_splat_pattern"
        | "dictionary_splat_pattern" => !in_parameters(parent),
        "dotted_name" | "aliased_import" => !in_import(parent),
        _ => true,
    }
}

fn in_parameters(node: Node) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        match n.kind() {
            "parameters" | "lambda_parameters" => return true,
            "function_definition" | "lambda" | "module" | "block" => return false,
            _ => current = n.parent(),
        }
    }
    false
}

fn in_import(node: Node) -> bool {
    let mut current = Some(node);
    while let Some(n) = current {
        match n.kind() {
            "import_statement" | "import_from_statement" | "future_import_statement" => {
                return true;
            }
            "module" | "block" => return false,
            _ => current = n.parent(),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "import os\n\
from demo_pkg.util import total\n\
\n\
def describe(items: list[int], n=1, *rest) -> dict[str, int]:\n\
    summary = {\"count\": len(items), \"sum\": 0}  # summary\n\
    for value in items:\n\
        summary[\"sum\"] += value\n\
    return summary.copy().keys\n\
\n\
print(describe(items=[1, 2]), os.path.sep, f\"{value}\")\n";

    /// The text of the expression at the `n`-th (from 0) occurrence of
    /// `word`, hovered at its `at`-th char.
    fn at(word: &str, n: usize, at: usize) -> Option<&'static str> {
        let start = TEXT.match_indices(word).nth(n).unwrap().0;
        expression_at(TEXT, start + at).map(|r| &TEXT[r])
    }

    #[test]
    fn a_name_is_evaluated_where_it_is_read_or_assigned() {
        // `summary["sum"]` over `summary`: the name only.
        assert_eq!(at("summary[", 0, 3), Some("summary"));
        assert_eq!(at("summary = ", 0, 0), Some("summary"));
        assert_eq!(at("value", 0, 2), Some("value"));
        assert_eq!(at("items)", 0, 0), Some("items"));
        assert_eq!(at("{value}", 0, 2), Some("value"));
    }

    #[test]
    fn an_attribute_is_evaluated_up_to_the_hovered_name() {
        assert_eq!(at("os.path", 0, 0), Some("os"));
        assert_eq!(at("os.path", 0, 4), Some("os.path"));
        assert_eq!(at("os.path.sep", 0, 9), Some("os.path.sep"));
    }

    #[test]
    fn calls_declarations_literals_and_keywords_are_not() {
        // Called functions and methods: `len`, `describe`, `copy`.
        assert_eq!(at("len", 0, 1), None);
        assert_eq!(at("describe(items=", 0, 2), None);
        assert_eq!(at("copy", 0, 1), None);
        // But what a method is called on is: `summary` of `summary.copy()`.
        assert_eq!(at("summary.copy", 0, 1), Some("summary"));
        // `.keys` is read, not called.
        assert_eq!(at("keys", 0, 1), Some("summary.copy().keys"));
        // The `def`'s name, parameters, a keyword argument's name.
        assert_eq!(at("describe(items:", 0, 1), None);
        assert_eq!(at("items:", 0, 1), None);
        assert_eq!(at("n=1", 0, 0), None);
        assert_eq!(at("rest", 0, 0), None);
        assert_eq!(at("items=[", 0, 0), None);
        // Imports, literals, strings, comments, keywords.
        assert_eq!(at("os\n", 0, 0), None);
        assert_eq!(at("total", 0, 1), None);
        assert_eq!(at("\"count\"", 0, 2), None);
        assert_eq!(at("0}", 0, 0), None);
        assert_eq!(at("# summary", 0, 4), None);
        assert_eq!(at("for", 0, 1), None);
        assert_eq!(expression_at(TEXT, TEXT.len()), None);
    }
}
