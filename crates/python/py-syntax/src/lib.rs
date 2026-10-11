//! Python syntax for Pyoxide IDE: the `tree-sitter-python` grammar (MIT,
//! see NOTICE) with this crate's own highlight query
//! (`queries/highlights.scm`), whose captures map to the text roles of the
//! themes crate.

use std::sync::{Arc, OnceLock};

use themes::TextRole;

/// The highlight query.
pub const HIGHLIGHTS: &str = include_str!("../queries/highlights.scm");

/// Capture names of the query that the shared capture table does not know.
const EXTRA_CAPTURES: &[(&str, TextRole)] = &[
    ("keyword.argument", TextRole::KeywordArgument),
    ("variable.self", TextRole::SelfParameter),
    ("variable.dunder", TextRole::Dunder),
    ("function.dunder", TextRole::Dunder),
    ("string.escape.invalid", TextRole::StringEscapeInvalid),
    ("comment.doc.tag", TextRole::DocstringTag),
];

/// The Python language (compiled once, shared).
pub fn language() -> Arc<syntax::Language> {
    static LANGUAGE: OnceLock<Arc<syntax::Language>> = OnceLock::new();
    LANGUAGE
        .get_or_init(|| {
            Arc::new(
                syntax::Language::new(
                    "Python",
                    tree_sitter_python::LANGUAGE.into(),
                    HIGHLIGHTS,
                    EXTRA_CAPTURES,
                )
                .expect("the Python highlight query compiles"),
            )
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use themes::TextRole::*;

    use super::*;

    const SAMPLE: &str = r#"import os
from dataclasses import dataclass

MAX_SIZE = 10


@dataclass
class Greeting(Base):
    """Says hello."""

    def __init__(self, name: str, count=3):
        # TODO: check the name
        self.name = name
        print(len(name), f"hi {name}\n", 0x1F, 2.5, True, None)
        os.path.join(name, sep="/")

    @property
    def size(cls) -> int:
        return cls.MAX
"#;

    /// The role painted on the first occurrence of `needle` on `line`.
    fn role_at(
        spans: &[Vec<syntax::Span>],
        text: &str,
        line: usize,
        needle: &str,
    ) -> Option<TextRole> {
        let line_text = text.lines().nth(line).unwrap();
        let col = line_text[..line_text.find(needle).unwrap()].chars().count();
        spans[line]
            .iter()
            .find(|(r, _)| r.contains(&col))
            .map(|(_, role)| *role)
    }

    #[test]
    fn query_compiles_and_every_capture_has_a_role() {
        let lang = language();
        assert!(
            lang.unresolved_captures().is_empty(),
            "{:?}",
            lang.unresolved_captures()
        );
    }

    #[test]
    fn highlights_python_tokens() {
        let lang = language();
        let rope = text::Rope::from_str(SAMPLE);
        let tree = syntax::parse(&lang, &rope, None, &AtomicBool::new(false)).unwrap();
        let spans = syntax::highlight(&lang, &tree, &rope, 0..rope.len_lines());
        let at = |line, needle| role_at(&spans, SAMPLE, line, needle);
        assert_eq!(at(0, "import"), Some(Keyword));
        // Upper-case names are not special by default.
        assert_eq!(at(3, "MAX_SIZE"), None);
        assert_eq!(at(3, "10"), Some(Number));
        assert_eq!(at(6, "dataclass"), Some(Decorator));
        assert_eq!(at(7, "class"), Some(Keyword));
        assert_eq!(at(7, "Greeting"), Some(ClassDecl));
        assert_eq!(at(8, "\"\"\"Says"), Some(Docstring));
        assert_eq!(at(10, "__init__"), Some(Dunder));
        assert_eq!(at(10, "self"), Some(SelfParameter));
        assert_eq!(at(11, "# TODO"), Some(Todo));
        assert_eq!(at(13, "print"), Some(Builtin));
        assert_eq!(at(13, "len"), Some(Builtin));
        assert_eq!(at(13, "f\"hi"), Some(String));
        assert_eq!(at(13, "\\n"), Some(StringEscape));
        assert_eq!(at(13, "0x1F"), Some(Number));
        assert_eq!(at(13, "2.5"), Some(Number));
        assert_eq!(at(13, "True"), Some(ConstantBuiltin));
        assert_eq!(at(13, "None"), Some(ConstantBuiltin));
        assert_eq!(at(14, "join"), Some(MethodCall));
        assert_eq!(at(14, "sep"), Some(KeywordArgument));
        assert_eq!(at(16, "property"), Some(Decorator));
        assert_eq!(at(17, "size"), Some(MethodDecl));
        assert_eq!(at(17, "cls"), Some(SelfParameter));
        // Plain names and attributes stay in the editor text colour.
        assert_eq!(at(12, "name ="), None);
        assert_eq!(at(14, "path"), None);
    }

    #[test]
    fn reparses_incrementally_after_edits() {
        let lang = language();
        let mut buffer = text::Buffer::new("x = 1\n");
        let mut tree = syntax::parse(&lang, buffer.rope(), None, &AtomicBool::new(false)).unwrap();
        let mut sel = text::SelectionSet::caret(0);
        let applied = buffer.transact(
            &mut sel,
            vec![text::Edit::insert(0, "def f():\n    ")],
            text::EditKind::Other,
            text::CaretPlacement::AfterEachEdit,
        );
        for e in &applied {
            tree.edit(&syntax::input_edit(e));
        }
        let tree =
            syntax::parse(&lang, buffer.rope(), Some(&tree), &AtomicBool::new(false)).unwrap();
        let spans = syntax::highlight(&lang, &tree, buffer.rope(), 0..2);
        assert_eq!(spans[0][0], (0..3, Keyword));
        assert_eq!(spans[0][1], (4..5, FunctionDecl));
        // A cancelled parse returns nothing (the flag is checked while
        // parsing, so the input has to be long enough).
        let big = text::Rope::from_str(&"def f(x):\n    return x + 1\n".repeat(20_000));
        assert!(syntax::parse(&lang, &big, None, &AtomicBool::new(true)).is_none());
    }
}
