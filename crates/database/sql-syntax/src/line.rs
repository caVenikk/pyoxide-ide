use std::sync::Mutex;

use crate::dialect::{Dialect, ParamForms};
use crate::lexer::{Token, TokenKind, lex_with};

/// Lexing line by line, for an editor that colours the lines it paints and
/// carries a small state from one line to the next (ADR 0022, layer 1).
///
/// The state says which construct is open at the start of a line: a block
/// comment (with its depth where comments nest), a string, a quoted
/// identifier, a `$tag$` body or an Oracle `q'[` string. A line that starts
/// inside one is lexed with the construct's opening put before it, so the
/// tokens are exactly those the lexer finds in the whole text.
#[derive(Debug)]
pub struct LineLexer {
    dialect: Dialect,
    params: ParamForms,
    /// The `$tag$`s that were open at the end of a line, by their number in
    /// the state.
    tags: Mutex<Vec<Box<str>>>,
}

const NORMAL: u32 = 0;
const BLOCK_COMMENT: u32 = 1;
const STRING: u32 = 2;
const ESCAPE_STRING: u32 = 3;
const QUOTED_IDENT: u32 = 4;
const DOLLAR: u32 = 5;
const Q_QUOTE: u32 = 6;
const KIND_BITS: u32 = 4;
const KIND_MASK: u32 = (1 << KIND_BITS) - 1;

/// More distinct tags than this in one lexer fall back to a plain string.
const MAX_TAGS: usize = 1 << 16;

impl LineLexer {
    pub fn new(dialect: Dialect) -> Self {
        Self::with_params(dialect, dialect.default_params())
    }

    pub fn with_params(dialect: Dialect, params: ParamForms) -> Self {
        Self {
            dialect,
            params,
            tags: Mutex::new(Vec::new()),
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The state at the start of a text.
    pub fn initial_state(&self) -> u32 {
        NORMAL
    }

    /// Lexes `line` (without its line break), which starts in `state`, and
    /// appends its tokens to `out` (byte offsets into `line`, adjacent, not
    /// empty). Returns the state at the end of the line.
    pub fn lex_line(&self, line: &str, state: u32, out: &mut Vec<Token>) -> u32 {
        let prefix = self.reopen(state);
        let skip = prefix.len();
        let joined;
        let text: &str = if skip == 0 {
            line
        } else {
            joined = format!("{prefix}{line}");
            &joined
        };
        let mut next = NORMAL;
        for token in lex_with(text, self.dialect, self.params) {
            if token.unterminated {
                next = self.state_of(&token, token.text(text));
            }
            if token.end <= skip {
                continue;
            }
            out.push(Token {
                start: token.start.saturating_sub(skip),
                end: token.end - skip,
                ..token
            });
        }
        next
    }

    /// The text that opens the construct a state is inside.
    fn reopen(&self, state: u32) -> String {
        let param = state >> KIND_BITS;
        match state & KIND_MASK {
            BLOCK_COMMENT => "/*".repeat(param.max(1) as usize),
            STRING => "'".to_string(),
            ESCAPE_STRING => "E'".to_string(),
            QUOTED_IDENT => char::from_u32(param).unwrap_or('"').to_string(),
            DOLLAR => self
                .tags
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(param as usize)
                .map(|t| t.to_string())
                .unwrap_or_else(|| "$$".to_string()),
            Q_QUOTE => format!("q'{}", char::from_u32(param).unwrap_or('[')),
            _ => String::new(),
        }
    }

    /// The state after an unterminated token whose text is `text`.
    fn state_of(&self, token: &Token, text: &str) -> u32 {
        match token.kind {
            TokenKind::BlockComment => {
                let mut depth = 0u32;
                let bytes = text.as_bytes();
                let nested = self.dialect.rules().nested_comments;
                let mut i = 0;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'/' && bytes[i + 1] == b'*' && (nested || depth == 0) {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes[i + 1] == b'/' && depth > 0 {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                BLOCK_COMMENT | (depth.clamp(1, 255) << KIND_BITS)
            }
            TokenKind::QuotedIdent => {
                let open = text.chars().next().unwrap_or('"');
                QUOTED_IDENT | ((open as u32) << KIND_BITS)
            }
            TokenKind::String => self.string_state(text),
            _ => NORMAL,
        }
    }

    fn string_state(&self, text: &str) -> u32 {
        let bytes = text.as_bytes();
        if bytes.first() == Some(&b'$') {
            let Some(end) = text[1..].find('$') else {
                return STRING;
            };
            let tag = &text[..end + 2];
            let mut tags = self.tags.lock().unwrap_or_else(|e| e.into_inner());
            let index = match tags.iter().position(|t| &**t == tag) {
                Some(i) => i,
                None if tags.len() < MAX_TAGS => {
                    tags.push(tag.into());
                    tags.len() - 1
                }
                None => return STRING,
            };
            return DOLLAR | ((index as u32) << KIND_BITS);
        }
        let skip = usize::from(matches!(bytes.first(), Some(b'n' | b'N')));
        if self.dialect.rules().q_quotes
            && matches!(bytes.get(skip), Some(b'q' | b'Q'))
            && bytes.get(skip + 1) == Some(&b'\'')
            && let Some(&open) = bytes.get(skip + 2)
        {
            return Q_QUOTE | (u32::from(open) << KIND_BITS);
        }
        if self.dialect.rules().escape_strings
            && matches!(bytes.first(), Some(b'e' | b'E'))
            && bytes.get(1) == Some(&b'\'')
        {
            return ESCAPE_STRING;
        }
        STRING
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    /// The tokens of every line, joined with the line breaks put back, as
    /// `(kind, text)`.
    fn by_line(dialect: Dialect, text: &str) -> Vec<(TokenKind, String)> {
        let lexer = LineLexer::new(dialect);
        let mut state = lexer.initial_state();
        let mut out = Vec::new();
        for line in text.split('\n') {
            let mut tokens = Vec::new();
            state = lexer.lex_line(line, state, &mut tokens);
            let mut at = 0;
            for t in tokens {
                assert_eq!(t.start, at, "adjacent tokens in {line:?}");
                assert!(t.end > t.start);
                at = t.end;
                out.push((t.kind, line[t.range()].to_string()));
            }
            assert_eq!(at, line.len(), "{line:?} is covered");
        }
        out
    }

    /// Whole-text tokens of the kinds the highlighter draws, with line
    /// breaks inside a token split at them.
    fn whole(dialect: Dialect, text: &str) -> Vec<(TokenKind, String)> {
        let mut out = Vec::new();
        for t in lex(text, dialect) {
            if t.kind == TokenKind::Whitespace {
                continue;
            }
            for piece in t.text(text).split('\n') {
                if !piece.is_empty() {
                    out.push((t.kind, piece.to_string()));
                }
            }
        }
        out
    }

    fn significant(tokens: Vec<(TokenKind, String)>) -> Vec<(TokenKind, String)> {
        tokens
            .into_iter()
            .filter(|(k, _)| *k != TokenKind::Whitespace)
            .collect()
    }
    fn same_as_whole(dialect: Dialect, text: &str) {
        assert_eq!(
            significant(by_line(dialect, text)),
            whole(dialect, text),
            "{text:?}"
        );
    }

    #[test]
    fn block_comments_run_over_lines() {
        same_as_whole(Dialect::Generic, "select /* a\nb\nc */ 1\n/* d */ x");
    }

    #[test]
    fn postgres_comments_nest() {
        same_as_whole(Dialect::Postgres, "/* a /* b\n*/ c\n*/ select 1");
        same_as_whole(Dialect::Postgres, "/* a /* b */\nstill */ d");
    }

    #[test]
    fn strings_run_over_lines() {
        same_as_whole(Dialect::Generic, "select 'a\nb''\nc' || 'd'");
        same_as_whole(Dialect::Generic, "select 'x\n'' y\n'");
    }

    #[test]
    fn escape_strings_keep_their_backslashes() {
        same_as_whole(Dialect::Postgres, "select E'a\\'\nb' , 'c\\'\nd'");
    }

    #[test]
    fn dollar_quoted_bodies_run_over_lines() {
        same_as_whole(
            Dialect::Postgres,
            "create function f() as $body$\nselect '$$';\n$$ still $body$ language sql;",
        );
        same_as_whole(Dialect::Postgres, "select $$a\nb$$, $x$c\nd$x$");
    }

    #[test]
    fn oracle_q_quotes_run_over_lines() {
        same_as_whole(Dialect::Oracle, "select q'[a\n'b]' from dual");
    }

    #[test]
    fn quoted_identifiers_run_over_lines() {
        same_as_whole(Dialect::Generic, "select \"a\nb\" from t");
    }

    #[test]
    fn a_line_of_a_comment_is_one_token() {
        let lexer = LineLexer::new(Dialect::Generic);
        let mut tokens = Vec::new();
        let first = lexer.lex_line("/* a", 0, &mut tokens);
        assert_ne!(first, 0, "still inside");
        tokens.clear();
        let end = lexer.lex_line("still comment", first, &mut tokens);
        assert_eq!(end, first);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::BlockComment);
        assert_eq!(tokens[0].range(), 0..13);
    }

    #[test]
    fn an_empty_line_inside_a_construct_keeps_the_state() {
        let lexer = LineLexer::new(Dialect::Postgres);
        let mut tokens = Vec::new();
        let state = lexer.lex_line("select $t$", 0, &mut tokens);
        tokens.clear();
        let same = lexer.lex_line("", state, &mut tokens);
        assert_eq!(same, state);
        assert!(tokens.is_empty());
        let end = lexer.lex_line("$t$ x", state, &mut tokens);
        assert_eq!(end, 0);
    }

    #[test]
    fn arbitrary_text_is_covered_without_panics() {
        for dialect in [
            Dialect::Generic,
            Dialect::Postgres,
            Dialect::Oracle,
            Dialect::Sqlite,
        ] {
            by_line(
                dialect,
                "é/*\u{e000}'\n\"x\n$a$\nq'<\n[a]\n`b\n\\\n/\n/* */ */ 'é\nÿ",
            );
        }
    }
}
