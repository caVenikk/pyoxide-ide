//! The text around a caret: the identifier being typed, the qualifier in
//! front of it and the tokens before it. Completion, quick documentation
//! and parameter info all start from this.

use std::collections::VecDeque;
use std::ops::Range;

use sql_syntax::{Dialect, Script, SyntaxNode, SyntaxToken, TokenKind};

/// How many significant tokens before the identifier are kept.
const WINDOW: usize = 12;

/// One part of a qualifier (`s` and `t` of `s.t.`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualifierPart {
    pub written: String,
    /// The catalog name ([`Dialect::fold`]).
    pub name: String,
    pub range: Range<usize>,
}

/// The caret in the text of a script.
pub(crate) struct Caret<'a> {
    pub offset: usize,
    /// Where the text a completion replaces starts: the identifier typed
    /// before the caret, with its opening quote.
    pub start: usize,
    /// The identifier typed before the caret, without a quote.
    pub prefix: String,
    /// The end of the identifier the caret is in (with its closing quote).
    pub word_end: usize,
    /// The identifier was opened with a quote.
    pub quoted: bool,
    /// The root of the statement (or of the program embedded in a string)
    /// the caret is in.
    pub root: SyntaxNode<'a>,
    /// The significant tokens before the identifier, nearest last.
    pub before: VecDeque<SyntaxToken<'a>>,
}

impl<'a> Caret<'a> {
    /// The caret at `offset`; `None` inside a comment or a string.
    pub fn at(script: &'a Script, offset: usize) -> Option<Caret<'a>> {
        let text = script.text();
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let unit = script.root_at(offset);
        let embedded = unit.fragment_at(offset);
        let root = embedded.as_ref().map_or(unit, |f| f.root);
        if embedded.is_none() {
            // The token the caret is in, and the one that ends at the caret
            // (a line comment ends where its line does).
            let around = [Some(offset), offset.checked_sub(1)];
            for t in around
                .into_iter()
                .flatten()
                .filter_map(|o| script.token_at(o))
            {
                let r = t.range();
                let inside = r.start < offset && (offset < r.end || t.unterminated());
                match t.kind() {
                    TokenKind::LineComment if r.start < offset && offset <= r.end => return None,
                    TokenKind::BlockComment | TokenKind::String if inside => return None,
                    _ => {}
                }
            }
        }
        let mut caret = Caret {
            offset,
            start: offset,
            prefix: String::new(),
            word_end: offset,
            quoted: false,
            root,
            before: VecDeque::with_capacity(WINDOW),
        };
        for t in root.tokens() {
            let r = t.range();
            if r.start >= offset {
                break;
            }
            match t.kind() {
                TokenKind::Word if offset <= r.end => {
                    caret.start = r.start;
                    caret.prefix = text[r.start..offset].to_string();
                    caret.word_end = r.end;
                }
                TokenKind::QuotedIdent if offset < r.end || t.unterminated() => {
                    caret.start = r.start;
                    caret.quoted = true;
                    caret.prefix = text[r.start + 1..offset].to_string();
                    caret.word_end = r.end;
                }
                _ => {
                    if caret.before.len() == WINDOW {
                        caret.before.pop_front();
                    }
                    caret.before.push_back(t);
                }
            }
        }
        Some(caret)
    }

    /// The last significant token before the identifier.
    pub fn previous(&self) -> Option<SyntaxToken<'a>> {
        self.before.back().copied()
    }

    /// The offset a name at the caret is looked up from: the end of the
    /// token before the identifier.
    pub fn anchor(&self) -> usize {
        self.previous().map_or(self.start, |t| t.range().end)
    }

    /// The qualifier in front of the identifier (`s.t.` before the caret),
    /// outermost part first.
    pub fn qualifier(&self, dialect: Dialect) -> Vec<QualifierPart> {
        let mut out = Vec::new();
        let mut i = self.before.len();
        while let Some(dot) = i.checked_sub(1).map(|j| self.before[j]) {
            if dot.kind() != TokenKind::Dot {
                break;
            }
            let Some(name) = i.checked_sub(2).map(|j| self.before[j]) else {
                break;
            };
            if !matches!(name.kind(), TokenKind::Word | TokenKind::QuotedIdent) {
                break;
            }
            let written = name.text().to_string();
            out.push(QualifierPart {
                name: dialect.fold(&written),
                written,
                range: name.range(),
            });
            i -= 2;
        }
        out.reverse();
        out
    }

    /// The word before the identifier is `word` (any case).
    pub fn previous_is(&self, word: &str) -> bool {
        self.previous()
            .is_some_and(|t| t.kind() == TokenKind::Word && t.text().eq_ignore_ascii_case(word))
    }
}
