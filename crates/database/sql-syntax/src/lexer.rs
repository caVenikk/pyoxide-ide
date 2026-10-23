use std::ops::Range;

use crate::dialect::{Dialect, LexRules, ParamForms};

/// The character that stands for text outside SQL in a virtual document (a
/// substitution of a host language, ADR 0022). The lexer returns it as a
/// [`TokenKind::Hole`] token of its own, also inside a word, and as part of
/// the text of a string, comment or quoted identifier.
pub const HOLE: char = '\u{E000}';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    /// `-- ...` up to, not including, the line break.
    LineComment,
    /// `/* ... */`, nested on PostgreSQL.
    BlockComment,
    /// A name or a keyword; [`Dialect::keyword`] tells which.
    Word,
    /// `"x"`, and `` `x` `` or `[x]` where the dialect has them.
    QuotedIdent,
    Number,
    /// Every string form: `'..'`, `E'..'`, `N'..'`, `X'..'`, `$tag$..$tag$`,
    /// `q'[..]'`.
    String,
    /// `?`, `$1`, `:name`, `${name}`, `%(name)s` ... see [`ParamForms`].
    Parameter,
    /// A run of operator characters (`=`, `<>`, `||`, `->>`, `@>`), and `::`,
    /// `:=` and a lone `:`.
    Operator,
    Semicolon,
    Comma,
    Dot,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    /// A `/` alone on its line, where the dialect has it as a terminator
    /// (Oracle).
    SlashLine,
    /// [`HOLE`].
    Hole,
    /// A character that is nothing in SQL (a stray `\`, a symbol).
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte offsets into the lexed text, on character boundaries.
    pub start: usize,
    pub end: usize,
    /// A string, quoted identifier or block comment that runs to the end of
    /// the text without being closed.
    pub unterminated: bool,
}

impl Token {
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }

    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    pub fn is_trivia(&self) -> bool {
        self.kind.is_trivia()
    }
}

impl TokenKind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
        )
    }
}

/// Lexes `src` with the default parameter forms of `dialect`. Never fails and
/// never panics: the tokens are adjacent, cover the whole text and fall on
/// character boundaries, so concatenating their text gives `src` back.
pub fn lex(src: &str, dialect: Dialect) -> Vec<Token> {
    lex_with(src, dialect, dialect.default_params())
}

pub fn lex_with(src: &str, dialect: Dialect, params: ParamForms) -> Vec<Token> {
    Lexer::new(src, dialect, params).collect()
}

/// A token with the trivia around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub token: Token,
    /// Whitespace and comments before the token, after the previous token's
    /// trailing trivia.
    pub leading: Range<usize>,
    /// Whitespace and comments after the token on the same line, up to the
    /// line break. The last token takes everything up to the end.
    pub trailing: Range<usize>,
}

/// Attaches the trivia to the significant tokens (the convention of
/// syntax trees: a comment at the end of a line belongs to the token before
/// it, the rest to the token after it). The ranges of `leading`, `token` and
/// `trailing` of all items follow each other and cover the whole text. Empty
/// when there is no significant token.
pub fn attach_trivia(src: &str, tokens: &[Token]) -> Vec<Attached> {
    let mut out: Vec<Attached> = Vec::new();
    let mut prev_end = 0;
    for tok in tokens.iter().filter(|t| !t.is_trivia()) {
        let leading_start = match out.last_mut() {
            None => 0,
            Some(prev) => {
                let cut = first_line_break(src, tokens, prev_end..tok.start);
                prev.trailing = prev_end..cut;
                cut
            }
        };
        out.push(Attached {
            token: *tok,
            leading: leading_start..tok.start,
            trailing: tok.end..tok.end,
        });
        prev_end = tok.end;
    }
    if let Some(last) = out.last_mut() {
        last.trailing = last.token.end..src.len();
    }
    out
}

/// The offset of the first line break in the whitespace of `gap` (the end of
/// `gap` if none); line breaks inside comments do not count.
fn first_line_break(src: &str, tokens: &[Token], gap: Range<usize>) -> usize {
    let first = tokens.partition_point(|t| t.end <= gap.start);
    for t in &tokens[first..] {
        if t.start >= gap.end {
            break;
        }
        if t.kind == TokenKind::Whitespace
            && let Some(i) = src[t.start..t.end].find(['\n', '\r'])
        {
            return t.start + i;
        }
    }
    gap.end
}

/// The lexer as an iterator.
pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    dialect: Dialect,
    rules: &'static LexRules,
    params: ParamForms,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, dialect: Dialect, params: ParamForms) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            dialect,
            rules: dialect.rules(),
            params,
        }
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos + off).copied()
    }

    fn peek_at(&self, p: usize) -> Option<u8> {
        self.bytes.get(p).copied()
    }

    fn char_at(&self, p: usize) -> Option<char> {
        self.src.get(p..).and_then(|s| s.chars().next())
    }

    fn eat_while(&mut self, f: impl Fn(u8) -> bool) {
        while self.pos < self.bytes.len() && f(self.bytes[self.pos]) {
            self.pos += 1;
        }
    }

    fn eat_chars(&mut self, f: impl Fn(char) -> bool) {
        while let Some(c) = self.char_at(self.pos) {
            if !f(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    /// Advances past the closing `close` (to the end if there is none) and
    /// says whether it was found. `doubled`: a doubled `close` is part of the
    /// text; `backslash`: `\` escapes the next character.
    fn eat_quoted(&mut self, close: u8, doubled: bool, backslash: bool) -> bool {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            if backslash && b == b'\\' {
                if let Some(c) = self.char_at(self.pos) {
                    self.pos += c.len_utf8();
                }
            } else if b == close {
                if doubled && self.peek(0) == Some(close) {
                    self.pos += 1;
                } else {
                    return true;
                }
            }
        }
        false
    }

    fn eat_block_comment(&mut self) -> bool {
        self.pos += 2;
        let mut depth = 1usize;
        while self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'*' && self.peek(1) == Some(b'/') {
                self.pos += 2;
                depth -= 1;
                if depth == 0 {
                    return true;
                }
            } else if self.rules.nested_comments
                && self.bytes[self.pos] == b'/'
                && self.peek(1) == Some(b'*')
            {
                self.pos += 2;
                depth += 1;
            } else {
                self.pos += 1;
            }
        }
        false
    }

    /// The length of a `$tag$` opening at the current position.
    fn dollar_tag_len(&self) -> Option<usize> {
        let rest = &self.src[self.pos..];
        let mut chars = rest.char_indices();
        chars.next(); // the first `$`
        let mut first = true;
        for (i, c) in chars {
            if c == '$' {
                return Some(i + 1);
            }
            let ok = if first {
                c.is_alphabetic() || c == '_'
            } else {
                c.is_alphanumeric() || c == '_'
            };
            if !ok {
                return None;
            }
            first = false;
        }
        None
    }

    /// The length of an Oracle `q'<d>..<d>'` literal (`nq'..'` too) at the
    /// current position, to the end of the text when it is not closed.
    fn q_quote_len(&self) -> Option<(usize, bool)> {
        let rest = &self.bytes[self.pos..];
        let skip = usize::from(matches!(rest.first(), Some(b'n' | b'N')));
        if !matches!(rest.get(skip), Some(b'q' | b'Q')) || rest.get(skip + 1) != Some(&b'\'') {
            return None;
        }
        let open = *rest.get(skip + 2)?;
        if open.is_ascii_whitespace() || open == b'\'' || open >= 0x80 {
            return None;
        }
        let close = match open {
            b'[' => b']',
            b'{' => b'}',
            b'(' => b')',
            b'<' => b'>',
            c => c,
        };
        let body = skip + 3;
        Some(
            rest[body..]
                .windows(2)
                .position(|w| w[0] == close && w[1] == b'\'')
                .map_or((rest.len(), false), |i| (body + i + 2, true)),
        )
    }

    /// The length of the string prefix (`E`, `N`, `X`, `B`, `U&`) at the
    /// current position when a quote follows, and whether `\` escapes.
    fn string_prefix(&self) -> Option<(usize, bool)> {
        let b = self.bytes[self.pos];
        let next = self.peek(1);
        let r = self.rules;
        match b {
            b'e' | b'E' if r.escape_strings && next == Some(b'\'') => Some((1, true)),
            b'n' | b'N' if next == Some(b'\'') => Some((1, false)),
            b'x' | b'X' | b'b' | b'B' if r.bit_strings && next == Some(b'\'') => Some((1, false)),
            b'u' | b'U'
                if r.unicode_strings && next == Some(b'&') && self.peek(2) == Some(b'\'') =>
            {
                Some((2, false))
            }
            _ => None,
        }
    }

    fn is_slash_line(&self) -> bool {
        let before_ok = self.bytes[..self.pos]
            .iter()
            .rev()
            .take_while(|&&b| b != b'\n')
            .all(|&b| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c));
        let after_ok = self.bytes[self.pos + 1..]
            .iter()
            .take_while(|&&b| b != b'\n')
            .all(|&b| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c));
        before_ok && after_ok
    }

    fn is_name_start(c: char) -> bool {
        c.is_alphabetic() || c == '_'
    }

    fn is_name_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// The byte length of the name starting at `p` (0 if none starts there).
    fn name_len(&self, p: usize) -> usize {
        let Some(rest) = self.src.get(p..) else {
            return 0;
        };
        let mut chars = rest.char_indices();
        match chars.next() {
            Some((_, c)) if Self::is_name_start(c) => {}
            _ => return 0,
        }
        for (i, c) in chars {
            if !Self::is_name_char(c) {
                return i;
            }
        }
        rest.len()
    }

    fn digits_len(&self, p: usize) -> usize {
        self.bytes
            .get(p..)
            .map_or(0, |r| r.iter().take_while(|b| b.is_ascii_digit()).count())
    }

    /// The length of a parameter that starts at byte `p`, if one does.
    fn param_at(&self, p: usize) -> Option<usize> {
        let f = &self.params;
        let next = self.peek_at(p + 1);
        match self.bytes[p] {
            b'?' if f.question => {
                if self.dialect == Dialect::Postgres && matches!(next, Some(b'|' | b'&')) {
                    return None;
                }
                let n = self.digits_len(p + 1);
                Some(if f.question_numbered { 1 + n } else { 1 })
            }
            b'$' => {
                let n = self.digits_len(p + 1);
                if f.dollar_numbered && n > 0 {
                    return Some(1 + n);
                }
                if f.brace_named && next == Some(b'{') {
                    let inner = self.src[p + 2..]
                        .char_indices()
                        .find(|&(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '-')));
                    return match inner {
                        Some((i, '}')) if i > 0 => Some(2 + i + 1),
                        _ => None,
                    };
                }
                let name = self.name_len(p + 1);
                (f.dollar_named && name > 0).then_some(1 + name)
            }
            b':' => {
                if next == Some(b':') {
                    return None;
                }
                let n = self.digits_len(p + 1);
                if f.colon_numbered && n > 0 {
                    return Some(1 + n);
                }
                let name = self.name_len(p + 1);
                if !f.colon_named || name == 0 {
                    return None;
                }
                // `:new.x` and `:old.x` of an Oracle trigger are not binds.
                if self.dialect == Dialect::Oracle && self.peek_at(p + 1 + name) == Some(b'.') {
                    let word = &self.src[p + 1..p + 1 + name];
                    if ["new", "old", "parent"]
                        .iter()
                        .any(|w| word.eq_ignore_ascii_case(w))
                    {
                        return None;
                    }
                }
                Some(1 + name)
            }
            b'@' if f.at_named => {
                let name = self.name_len(p + 1);
                (name > 0).then_some(1 + name)
            }
            b'%' if f.pyformat && next == Some(b'(') => {
                let name = self.src[p + 2..]
                    .find(|c: char| !(Self::is_name_char(c) || c == '.'))
                    .filter(|&n| n > 0 && self.peek_at(p + 2 + n) == Some(b')'))?;
                (self.peek_at(p + 2 + name + 1) == Some(b's')).then_some(2 + name + 2)
            }
            b'%' if f.percent_s && next == Some(b's') => {
                let after = self.char_at(p + 2);
                (!after.is_some_and(Self::is_name_char)).then_some(2)
            }
            _ => None,
        }
    }

    fn eat_number(&mut self) {
        // `0x1F`
        if self.peek(0) == Some(b'0')
            && matches!(self.peek(1), Some(b'x' | b'X'))
            && self.peek(2).is_some_and(|b| b.is_ascii_hexdigit())
        {
            self.pos += 2;
            self.eat_while(|b| b.is_ascii_hexdigit());
            return;
        }
        let digits = |lx: &mut Lexer| {
            while let Some(b) = lx.peek(0) {
                if b.is_ascii_digit()
                    || (b == b'_' && lx.peek(1).is_some_and(|n| n.is_ascii_digit()))
                {
                    lx.pos += 1;
                } else {
                    break;
                }
            }
        };
        digits(self);
        if self.peek(0) == Some(b'.') && self.peek(1) != Some(b'.') {
            self.pos += 1;
            digits(self);
        }
        if matches!(self.peek(0), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(self.peek(1), Some(b'+' | b'-')));
            if self.peek(1 + sign).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1 + sign;
                digits(self);
            }
        }
    }

    fn is_op_byte(b: u8) -> bool {
        matches!(
            b,
            b'+' | b'-'
                | b'*'
                | b'/'
                | b'<'
                | b'>'
                | b'='
                | b'~'
                | b'!'
                | b'@'
                | b'#'
                | b'%'
                | b'^'
                | b'&'
                | b'|'
                | b'?'
        )
    }

    fn eat_operator(&mut self) {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if !Self::is_op_byte(b)
                || (b == b'-' && self.peek(1) == Some(b'-'))
                || (b == b'/' && self.peek(1) == Some(b'*'))
                || self.param_at(self.pos).is_some()
            {
                break;
            }
            self.pos += 1;
        }
        // A run does not end in `+` or `-` unless it has another special
        // character (`a=-1` is `=` and `-`; `||-` stays).
        let special = |b: &u8| {
            matches!(
                b,
                b'~' | b'!' | b'@' | b'#' | b'%' | b'^' | b'&' | b'|' | b'?'
            )
        };
        while self.pos - start > 1
            && matches!(self.bytes[self.pos - 1], b'+' | b'-')
            && !self.bytes[start..self.pos].iter().any(special)
        {
            self.pos -= 1;
        }
    }

    fn scan(&mut self, c: char, unterminated: &mut bool) -> TokenKind {
        let b = self.bytes[self.pos];
        let next = self.peek(1);
        if c == HOLE {
            self.pos += c.len_utf8();
            return TokenKind::Hole;
        }
        if c.is_whitespace() {
            self.eat_chars(|c| c.is_whitespace());
            return TokenKind::Whitespace;
        }
        match b {
            b'-' if next == Some(b'-') => {
                self.eat_while(|b| b != b'\n' && b != b'\r');
                TokenKind::LineComment
            }
            b'/' if next == Some(b'*') => {
                *unterminated = !self.eat_block_comment();
                TokenKind::BlockComment
            }
            b'/' if self.rules.slash_terminator && self.is_slash_line() => {
                self.pos += 1;
                TokenKind::SlashLine
            }
            b'\'' => {
                self.pos += 1;
                *unterminated = !self.eat_quoted(b'\'', true, false);
                TokenKind::String
            }
            b'q' | b'Q' | b'n' | b'N' if self.rules.q_quotes && self.q_quote_len().is_some() => {
                let (len, closed) = self.q_quote_len().unwrap_or((1, true));
                self.pos += len;
                *unterminated = !closed;
                TokenKind::String
            }
            b'e' | b'E' | b'n' | b'N' | b'x' | b'X' | b'b' | b'B' | b'u' | b'U'
                if self.string_prefix().is_some() =>
            {
                let (prefix, backslash) = self.string_prefix().unwrap_or((1, false));
                self.pos += prefix + 1;
                *unterminated = !self.eat_quoted(b'\'', true, backslash);
                TokenKind::String
            }
            b'"' => {
                self.pos += 1;
                *unterminated = !self.eat_quoted(b'"', true, false);
                TokenKind::QuotedIdent
            }
            b'`' if self.rules.backtick_idents => {
                self.pos += 1;
                *unterminated = !self.eat_quoted(b'`', true, false);
                TokenKind::QuotedIdent
            }
            b'[' if self.rules.bracket_idents && !self.follows_operand() => {
                self.pos += 1;
                *unterminated = !self.eat_quoted(b']', false, false);
                TokenKind::QuotedIdent
            }
            b'$' => {
                if self.rules.dollar_quotes
                    && let Some(len) = self.dollar_tag_len()
                {
                    let tag = &self.bytes[self.pos..self.pos + len];
                    self.pos += len;
                    let rest = &self.bytes[self.pos..];
                    match rest.windows(len).position(|w| w == tag) {
                        Some(i) => self.pos += i + len,
                        None => {
                            self.pos = self.bytes.len();
                            *unterminated = true;
                        }
                    }
                    TokenKind::String
                } else if let Some(len) = self.param_at(self.pos) {
                    self.pos += len;
                    TokenKind::Parameter
                } else {
                    self.pos += 1;
                    TokenKind::Unknown
                }
            }
            b'0'..=b'9' => {
                self.eat_number();
                TokenKind::Number
            }
            b'.' => {
                let prev = self.pos.checked_sub(1).map(|p| self.bytes[p]);
                let after_operand = prev.is_some_and(|p| {
                    p == b'.'
                        || p == b')'
                        || p == b']'
                        || p == b'"'
                        || p >= 0x80
                        || p.is_ascii_alphanumeric()
                        || p == b'_'
                });
                if next.is_some_and(|n| n.is_ascii_digit()) && !after_operand {
                    self.eat_number();
                    TokenKind::Number
                } else {
                    self.pos += 1;
                    TokenKind::Dot
                }
            }
            b';' => {
                self.pos += 1;
                TokenKind::Semicolon
            }
            b',' => {
                self.pos += 1;
                TokenKind::Comma
            }
            b'(' => {
                self.pos += 1;
                TokenKind::LParen
            }
            b')' => {
                self.pos += 1;
                TokenKind::RParen
            }
            b'[' => {
                self.pos += 1;
                TokenKind::LBracket
            }
            b']' => {
                self.pos += 1;
                TokenKind::RBracket
            }
            b'{' => {
                self.pos += 1;
                TokenKind::LBrace
            }
            b'}' => {
                self.pos += 1;
                TokenKind::RBrace
            }
            b':' => {
                if let Some(len) = self.param_at(self.pos) {
                    self.pos += len;
                    TokenKind::Parameter
                } else if next == Some(b':') || next == Some(b'=') {
                    self.pos += 2;
                    TokenKind::Operator
                } else {
                    self.pos += 1;
                    TokenKind::Operator
                }
            }
            b'?' | b'@' | b'%' if self.param_at(self.pos).is_some() => {
                self.pos += self.param_at(self.pos).unwrap_or(1);
                TokenKind::Parameter
            }
            _ if Self::is_op_byte(b) => {
                self.eat_operator();
                TokenKind::Operator
            }
            _ if Self::is_name_start(c) => {
                let hash = self.rules.hash_in_words;
                self.eat_chars(|c| Self::is_name_char(c) || c == '$' || (hash && c == '#'));
                TokenKind::Word
            }
            _ => {
                self.pos += c.len_utf8();
                TokenKind::Unknown
            }
        }
    }

    /// `arr[1]`: a `[` right after a name or `)` is a subscript, not a quoted
    /// identifier.
    fn follows_operand(&self) -> bool {
        self.pos > 0
            && (matches!(self.bytes[self.pos - 1], b')' | b']' | b'"')
                || self.bytes[self.pos - 1].is_ascii_alphanumeric()
                || self.bytes[self.pos - 1] == b'_')
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        let c = self.char_at(self.pos)?;
        let start = self.pos;
        let mut unterminated = false;
        let kind = self.scan(c, &mut unterminated);
        debug_assert!(self.pos > start && self.src.is_char_boundary(self.pos));
        Some(Token {
            kind,
            start,
            end: self.pos,
            unterminated,
        })
    }
}
