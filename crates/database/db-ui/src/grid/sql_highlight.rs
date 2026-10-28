//! SQL colours of the WHERE and ORDER BY fields: keywords, strings, numbers
//! and comments of the engine's dialect, from the text layer's lexer.

use editor::highlight::{Highlighter, Span};
use sql_syntax::{Dialect, TokenKind};
use text::{AppliedEdit, Buffer};
use themes::TextRole;

/// Highlights the one line of a filter field.
pub(super) struct SqlHighlighter {
    dialect: Dialect,
    spans: Vec<Span>,
}

impl SqlHighlighter {
    pub(super) fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            spans: Vec::new(),
        }
    }

    fn lex(&mut self, text: &str) {
        self.spans.clear();
        let mut chars = 0;
        let mut at = 0;
        for token in sql_syntax::lex(text, self.dialect) {
            chars += text[at..token.start].chars().count();
            let len = text[token.start..token.end].chars().count();
            at = token.end;
            let role = match token.kind {
                TokenKind::Word
                    if self
                        .dialect
                        .keyword(&text[token.start..token.end])
                        .is_some() =>
                {
                    Some(TextRole::Keyword)
                }
                TokenKind::String => Some(TextRole::String),
                TokenKind::Number => Some(TextRole::Number),
                TokenKind::LineComment | TokenKind::BlockComment => Some(TextRole::Comment),
                _ => None,
            };
            if let Some(role) = role {
                self.spans.push((chars..chars + len, role));
            }
            chars += len;
        }
    }
}

impl Highlighter for SqlHighlighter {
    fn reset(&mut self, buffer: &Buffer) {
        self.lex(&buffer.text());
    }

    fn edited(&mut self, buffer: &Buffer, _: &[AppliedEdit]) {
        self.lex(&buffer.text());
    }

    fn line_spans(&self, line: usize, out: &mut Vec<Span>) {
        if line == 0 {
            out.extend(self.spans.iter().cloned());
        }
    }
}
