// The error-tolerant recursive-descent parser of ADR 0022: one parser for
// every dialect, switched by `Grammar` flags. It never fails: what it does
// not expect becomes an `Error` node (with the expected set) or a `Missing`
// node, what it does not know becomes an `Opaque` node, and it resumes at
// the next clause keyword, closing parenthesis or statement end.
use crate::dialect::{Dialect, Grammar};
use crate::lexer::{Token, TokenKind};
use crate::tree::{Builder, Expected, NameKind, StatementTree, SyntaxKind};

mod dml;
mod expr;
mod kw;
mod query;

pub(crate) use kw::Kw;

/// Nesting deeper than this (parentheses, subqueries, unary operators) is
/// kept as an `Opaque` node instead of recursing further.
const MAX_DEPTH: u32 = 96;

/// What kind of procedural code is being parsed: it decides what `INTO`,
/// `:=` and the statements mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Proc {
    No,
    /// PL/SQL (Oracle, and generic SQL).
    PlSql,
    /// PL/pgSQL.
    PlPgSql,
}

/// `Parser::close` of a `(` that is never closed, and of other tokens.
const NO_CLOSE: u32 = u32::MAX;

/// Parses one unit (`text`, its `tokens` relative to it).
pub(crate) fn parse_unit(text: &str, tokens: Vec<Token>, dialect: Dialect) -> StatementTree {
    let parts = {
        let mut p = Parser::new(text, &tokens, dialect, false);
        p.unit();
        p.b.finish_root(text.len())
    };
    parts.into_tree(tokens)
}

/// What may stand at `offset` (relative to the unit): the unit is parsed up
/// to the offset, and the expected set is taken where the parser first fails
/// or ends there. A word that touches the offset is the prefix being typed and
/// is left out.
pub(crate) fn expected_at(
    text: &str,
    tokens: &[Token],
    dialect: Dialect,
    offset: usize,
) -> Vec<Expected> {
    let mut cut = tokens.partition_point(|t| t.end <= offset);
    // The significant token before the cut, if it touches the offset and is a
    // word being typed, goes too.
    if let Some(last) = tokens[..cut].iter().rposition(|t| !t.is_trivia()) {
        let t = &tokens[last];
        if t.end == offset
            && matches!(
                t.kind,
                TokenKind::Word | TokenKind::QuotedIdent | TokenKind::Hole
            )
        {
            cut = last;
        }
    }
    // Trivia before the offset belongs to the prefix.
    let mut p = Parser::new(text, &tokens[..cut], dialect, true);
    p.unit();
    p.probe_result.unwrap_or_default()
}

pub(crate) struct Parser<'a> {
    text: &'a str,
    toks: &'a [Token],
    /// Indices into `toks` of the significant tokens.
    sig: Vec<u32>,
    /// The keyword of each significant token.
    kws: Vec<Option<Kw>>,
    /// For a `(`: the index (into `sig`) of its `)`.
    close: Vec<u32>,
    pos: usize,
    dialect: Dialect,
    g: Grammar,
    b: Builder<'a>,
    /// What was tried at the current position since the last token.
    expected: Vec<Expected>,
    /// Open parentheses: a `)` ends an error region only inside one.
    parens: u32,
    depth: u32,
    /// Parsing a prefix for `expected_at`: the set is captured at the end.
    probe: bool,
    probe_result: Option<Vec<Expected>>,
    /// Inside procedural code.
    proc: Proc,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, toks: &'a [Token], dialect: Dialect, probe: bool) -> Self {
        let mut sig = Vec::with_capacity(toks.len() / 2 + 1);
        let mut kws = Vec::with_capacity(toks.len() / 2 + 1);
        let mut close = Vec::with_capacity(toks.len() / 2 + 1);
        let mut open: Vec<usize> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            if !t.is_trivia() {
                let n = sig.len();
                sig.push(i as u32);
                kws.push(if t.kind == TokenKind::Word {
                    text.get(t.start..t.end).and_then(Kw::lookup)
                } else {
                    None
                });
                close.push(NO_CLOSE);
                match t.kind {
                    TokenKind::LParen => open.push(n),
                    TokenKind::RParen => {
                        if let Some(o) = open.pop() {
                            close[o] = n as u32;
                        }
                    }
                    _ => {}
                }
            }
        }
        Parser {
            text,
            toks,
            sig,
            kws,
            close,
            pos: 0,
            dialect,
            g: dialect.grammar(),
            b: Builder::new(toks),
            expected: Vec::new(),
            parens: 0,
            depth: 0,
            probe,
            probe_result: None,
            proc: Proc::No,
        }
    }

    // --- Looking at tokens.

    fn nth(&self, n: usize) -> Option<&'a Token> {
        let toks = self.toks;
        self.sig.get(self.pos + n).map(|&i| &toks[i as usize])
    }

    fn nth_kind(&self, n: usize) -> Option<TokenKind> {
        self.nth(n).map(|t| t.kind)
    }

    fn nth_kw(&self, n: usize) -> Option<Kw> {
        self.kws.get(self.pos + n).copied().flatten()
    }

    fn nth_text(&self, n: usize) -> &'a str {
        let text = self.text;
        self.nth(n)
            .and_then(|t| text.get(t.start..t.end))
            .unwrap_or("")
    }

    /// The `n`th token is the word `w` (any case), keyword or not.
    fn nth_word(&self, n: usize, w: &str) -> bool {
        self.nth_kind(n) == Some(TokenKind::Word) && self.nth_text(n).eq_ignore_ascii_case(w)
    }

    fn nth_op(&self, n: usize, op: &str) -> bool {
        self.nth_kind(n) == Some(TokenKind::Operator) && self.nth_text(n) == op
    }

    fn prev_text(&self) -> &'a str {
        let text = self.text;
        self.pos
            .checked_sub(1)
            .and_then(|p| self.sig.get(p))
            .map(|&i| &self.toks[i as usize])
            .and_then(|t| text.get(t.start..t.end))
            .unwrap_or("")
    }

    /// `INTO`, or `BULK COLLECT INTO`, is next.
    fn at_into(&self) -> bool {
        self.peek_kw(Kw::INTO)
            || (self.peek_kw(Kw::BULK)
                && self.nth_kw(1) == Some(Kw::COLLECT)
                && self.nth_kw(2) == Some(Kw::INTO))
    }

    /// Skips `[..]` groups (subscripts), nested.
    fn bump_brackets(&mut self) {
        while self.nth_kind(0) == Some(TokenKind::LBracket) {
            let mut depth = 0u32;
            while !self.eof() && !self.at_terminator() {
                match self.nth_kind(0) {
                    Some(TokenKind::LBracket) => depth += 1,
                    Some(TokenKind::RBracket) => depth = depth.saturating_sub(1),
                    _ => {}
                }
                self.bump();
                if depth == 0 {
                    break;
                }
            }
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.sig.len()
    }

    fn at_terminator(&self) -> bool {
        matches!(
            self.nth_kind(0),
            Some(TokenKind::Semicolon | TokenKind::SlashLine)
        )
    }

    /// The index into `toks` of the next significant token.
    fn next_tok(&self) -> usize {
        self.sig
            .get(self.pos)
            .map_or(self.toks.len(), |&i| i as usize)
    }

    // --- Expected sets.

    fn note(&mut self, e: Expected) {
        if self.expected.len() < 512 {
            self.expected.push(e);
        }
    }

    fn at_kw(&mut self, kw: Kw) -> bool {
        self.note(Expected::Keyword(kw.text()));
        self.nth_kw(0) == Some(kw)
    }

    /// `at_kw` for what may optionally continue a construct (operators,
    /// aliases): it joins the expected set only for completion.
    fn at_op_kw(&mut self, kw: Kw) -> bool {
        if self.probe {
            self.at_kw(kw)
        } else {
            self.peek_kw(kw)
        }
    }

    fn peek_kw(&self, kw: Kw) -> bool {
        self.nth_kw(0) == Some(kw)
    }

    fn at(&mut self, kind: TokenKind) -> bool {
        if let Some(p) = punct(kind) {
            self.note(Expected::Punct(p));
        }
        self.nth_kind(0) == Some(kind)
    }

    fn at_op(&mut self, op: &'static str) -> bool {
        self.note(Expected::Punct(op));
        self.nth_op(0, op)
    }

    fn snapshot(&self) -> Box<[Expected]> {
        let mut v = self.expected.clone();
        v.sort_unstable();
        v.dedup();
        v.into_boxed_slice()
    }

    fn capture(&mut self, set: &[Expected]) {
        if self.probe && self.eof() && self.probe_result.is_none() {
            self.probe_result = Some(set.to_vec());
        }
    }

    // --- Building.

    fn start(&mut self, kind: SyntaxKind) {
        let next = self.next_tok();
        self.b.start(kind, next);
    }

    fn checkpoint(&mut self) -> usize {
        let next = self.next_tok();
        self.b.checkpoint(next)
    }

    fn start_at(&mut self, cp: usize, kind: SyntaxKind) {
        self.b.start_at(cp, kind);
    }

    fn finish(&mut self) {
        self.b.finish(None);
    }

    fn bump(&mut self) {
        if let Some(&i) = self.sig.get(self.pos) {
            self.b.token(i as usize);
            self.pos += 1;
            self.expected.clear();
        }
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        let hit = self.at(kind);
        if hit {
            self.bump();
        }
        hit
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        let hit = self.at_kw(kw);
        if hit {
            self.bump();
        }
        hit
    }

    /// One of `kws`, or a `Missing` node expecting all of them.
    fn expect_any_kw(&mut self, kws: &[Kw]) -> bool {
        for &k in kws {
            if self.at_kw(k) {
                self.bump();
                return true;
            }
        }
        self.missing();
        false
    }

    fn expect_kw(&mut self, kw: Kw) -> bool {
        self.expect_any_kw(&[kw])
    }

    /// The token `kind`. A closing token that is not there skips an error
    /// region up to it; anything else is just recorded as missing.
    fn expect(&mut self, kind: TokenKind) -> bool {
        if self.eat(kind) {
            return true;
        }
        let closing = matches!(kind, TokenKind::RParen | TokenKind::RBracket);
        if !closing || self.at_recovery() {
            self.missing();
            return false;
        }
        self.error_until(false, Some(kind));
        self.eat(kind)
    }

    /// A `Missing` node with what was expected here. What is tried after it
    /// at the same position belongs to what follows the missing part.
    fn missing(&mut self) {
        let set = self.snapshot();
        self.capture(&set);
        self.start(SyntaxKind::Missing);
        self.b.finish(Some(set));
        self.expected.clear();
    }

    /// A `Missing` node at a recovery point, an error region elsewhere.
    fn missing_or_error(&mut self) {
        if self.at_recovery() {
            self.missing();
        } else {
            self.error_until(false, None);
        }
    }

    /// An `Error` node with the tokens up to the next recovery point (at
    /// least one), a `,` when `comma`, or `stop`. Parenthesised groups are
    /// skipped whole. A hole is never an error: it becomes a `Hole` node.
    fn error_until(&mut self, comma: bool, stop: Option<TokenKind>) {
        if self.eof() {
            self.missing();
            return;
        }
        if self.nth_kind(0) == Some(TokenKind::Hole) {
            self.hole();
            return;
        }
        let set = self.snapshot();
        self.start(SyntaxKind::Error);
        loop {
            self.bump_group();
            if self.eof()
                || self.at_recovery()
                || (comma && self.nth_kind(0) == Some(TokenKind::Comma))
                || stop.is_some_and(|k| self.nth_kind(0) == Some(k))
            {
                break;
            }
        }
        self.b.finish(Some(set));
    }

    /// An `Opaque` node with the tokens up to the next recovery point (at
    /// least one), or a `,` when `comma`.
    fn opaque_until(&mut self, comma: bool) {
        if self.eof() {
            return;
        }
        self.start(SyntaxKind::Opaque);
        loop {
            self.bump_group();
            if self.eof()
                || self.at_recovery()
                || (comma && self.nth_kind(0) == Some(TokenKind::Comma))
            {
                break;
            }
        }
        self.finish();
    }

    /// Whatever is left inside a parenthesised list before its `)`: when the
    /// `)` comes later in the statement, the tokens up to it are an error
    /// (an `Opaque` node in generic SQL, where unknown syntax is not an
    /// error); when it never comes, nothing is taken and the `)` is missing.
    fn rest_of_group(&mut self) {
        if self.nth_kind(0) == Some(TokenKind::RParen) || self.eof() || self.at_terminator() {
            return;
        }
        if !self.close_paren_ahead() {
            if !self.at_recovery() {
                self.error_until(false, Some(TokenKind::RParen));
            }
            return;
        }
        let generic = self.dialect == Dialect::Generic;
        let set = self.snapshot();
        self.start(if generic {
            SyntaxKind::Opaque
        } else {
            SyntaxKind::Error
        });
        while !self.eof() && !self.at_terminator() && self.nth_kind(0) != Some(TokenKind::RParen) {
            self.bump_group();
        }
        self.b.finish((!generic).then_some(set));
    }

    /// The `)` that closes the current group comes before the end of the
    /// statement.
    fn close_paren_ahead(&self) -> bool {
        let mut i = self.pos;
        while i < self.sig.len() {
            match self.toks[self.sig[i] as usize].kind {
                TokenKind::LParen => match self.close[i] {
                    // A group that never closes: nothing after it closes ours.
                    NO_CLOSE => return false,
                    c => i = c as usize + 1,
                },
                TokenKind::RParen => return true,
                TokenKind::Semicolon | TokenKind::SlashLine => return false,
                _ => i += 1,
            }
        }
        false
    }

    /// A token, or a whole parenthesised group when it is closed.
    fn bump_group(&mut self) {
        let end = match self.close.get(self.pos) {
            Some(&c) if c != NO_CLOSE => c as usize,
            _ => self.pos,
        };
        while self.pos <= end && !self.eof() {
            self.bump();
        }
    }

    /// A hole and the words glued to it.
    fn hole(&mut self) {
        let len = self.part_len(0).max(1);
        self.start(SyntaxKind::Hole);
        for _ in 0..len {
            self.bump();
        }
        self.finish();
    }

    /// Where an error region ends: the end, a terminator, a `)` inside
    /// parentheses, a clause keyword.
    fn at_recovery(&self) -> bool {
        match self.nth_kind(0) {
            None | Some(TokenKind::Semicolon | TokenKind::SlashLine) => true,
            Some(TokenKind::RParen) => self.parens > 0,
            Some(TokenKind::Word) => self.is_clause_kw(0),
            _ => false,
        }
    }

    fn is_clause_kw(&self, n: usize) -> bool {
        use Kw::*;
        let Some(kw) = self.nth_kw(n) else {
            return false;
        };
        let g = self.g;
        match kw {
            FROM | WHERE | GROUP | HAVING | ORDER | UNION | INTERSECT | EXCEPT | WINDOW | JOIN
            | INNER | CROSS | NATURAL | ON | USING | THEN | WHEN | ELSE | END => true,
            RETURNING => g.returning,
            LIMIT => g.limit,
            OFFSET => g.limit || g.fetch_first,
            FETCH => g.fetch_first,
            MINUS => g.minus,
            CONNECT => g.connect_by,
            START => g.connect_by && self.nth_kw(n + 1) == Some(WITH),
            LEFT | RIGHT | FULL => matches!(self.nth_kw(n + 1), Some(JOIN | OUTER)),
            FOR => matches!(self.nth_kw(n + 1), Some(UPDATE | SHARE | NO | KEY)),
            _ => false,
        }
    }

    /// A keyword that is never a bare name: it starts a clause, joins or
    /// combines expressions, or is a literal.
    fn is_hard(&self, n: usize) -> bool {
        use Kw::*;
        if self.nth_kind(n) != Some(TokenKind::Word) {
            return false;
        }
        let Some(kw) = self.nth_kw(n) else {
            return false;
        };
        let g = self.g;
        match kw {
            ALL | AND | AS | BETWEEN | CASE | CROSS | DISTINCT | ELSE | END | EXCEPT | EXISTS
            | FALSE | FOR | FROM | FULL | GROUP | HAVING | IN | INNER | INTERSECT | INTO | IS
            | JOIN | LEFT | LIKE | NATURAL | NOT | NULL | ON | OR | ORDER | OUTER | RIGHT
            | SELECT | THEN | TRUE | UNION | USING | WHEN | WHERE | WINDOW | WITH => true,
            LIMIT => g.limit,
            OFFSET => g.limit || g.fetch_first,
            FETCH => g.fetch_first,
            ILIKE => g.ilike,
            MINUS => g.minus,
            CONNECT => g.connect_by,
            START => g.connect_by && self.nth_kw(n + 1) == Some(WITH),
            ISNULL | NOTNULL => g.null_postfix,
            _ => false,
        }
    }

    /// The `n`th token can be an alias without `AS`.
    fn alias_ok(&self, n: usize) -> bool {
        use Kw::*;
        match self.nth_kind(n) {
            Some(TokenKind::QuotedIdent) => return true,
            Some(TokenKind::Word) => {}
            _ => return false,
        }
        if self.is_hard(n) {
            return false;
        }
        let g = self.g;
        if g.plsql && self.nth_word(n, "log") && self.nth_word(n + 1, "errors") {
            return false;
        }
        if g.on_conflict && self.nth_word(n, "overriding") {
            return false;
        }
        if let Some(kw) = self.nth_kw(n) {
            let paren_next = self.nth_kind(n + 1) == Some(TokenKind::LParen);
            let excluded = match kw {
                GLOB | REGEXP | MATCH => g.sqlite_operators,
                PIVOT | UNPIVOT => g.pivot,
                ILIKE | SIMILAR | COLLATE | ESCAPE | OVER | FILTER | WITHIN | KEEP | LATERAL
                | TABLESAMPLE | INDEXED | MODEL | MATCH_RECOGNIZE | APPLY | SET | VALUES
                | RETURNING | RETURN | DEFAULT => true,
                BULK => self.nth_kw(n + 1) == Some(COLLECT),
                SAMPLE | PARTITION => paren_next || self.nth_word(n + 1, "block"),
                _ => false,
            };
            if excluded {
                return false;
            }
        }
        self.dialect == Dialect::Generic || !self.dialect.is_reserved(self.nth_text(n))
    }

    /// The number of tokens of the name part at `n`: a word that is not a
    /// hard keyword, a quoted identifier, or a hole with the words glued to
    /// it. 0 when no name starts there.
    fn part_len(&self, n: usize) -> usize {
        let Some(first) = self.nth(n) else {
            return 0;
        };
        match first.kind {
            TokenKind::QuotedIdent => 1,
            TokenKind::Word if self.is_hard(n) => 0,
            TokenKind::Word | TokenKind::Hole => {
                let mut len = 1;
                let mut prev = first;
                let mut hole = first.kind == TokenKind::Hole;
                while let Some(next) = self.nth(n + len) {
                    let glued = next.start == prev.end
                        && matches!(
                            next.kind,
                            TokenKind::Word | TokenKind::Hole | TokenKind::Number
                        )
                        && (hole || next.kind == TokenKind::Hole);
                    if !glued {
                        break;
                    }
                    hole |= next.kind == TokenKind::Hole;
                    prev = next;
                    len += 1;
                }
                len
            }
            _ => 0,
        }
    }

    fn is_name_at(&self, n: usize) -> bool {
        self.part_len(n) > 0
    }

    /// A name part: one token, or a `Hole` node with the words glued to it.
    fn name_part(&mut self) {
        let len = self.part_len(0);
        if len == 1 && self.nth_kind(0) != Some(TokenKind::Hole) {
            self.bump();
        } else if len > 0 {
            self.hole();
        }
    }

    /// The dotted name at the current token: its number of parts, what
    /// follows the last part, and whether a `(` follows (a call).
    fn chain_ahead(&self) -> Option<(usize, Tail)> {
        let mut i = 0;
        let mut parts = 0;
        loop {
            let len = self.part_len(i);
            if len == 0 {
                break;
            }
            i += len;
            parts += 1;
            if self.nth_kind(i) == Some(TokenKind::Dot) {
                if self.part_len(i + 1) > 0 {
                    i += 1;
                    continue;
                }
                let tail = if self.nth_op(i + 1, "*") {
                    Tail::Star
                } else {
                    Tail::Missing
                };
                return Some((parts, tail));
            }
            break;
        }
        if parts == 0 {
            return None;
        }
        let call = self.nth_kind(i) == Some(TokenKind::LParen) && !self.outer_join_mark_at(i);
        Some((parts, if call { Tail::Call } else { Tail::Name }))
    }

    fn outer_join_mark_at(&self, n: usize) -> bool {
        self.g.outer_join_mark
            && self.nth_kind(n) == Some(TokenKind::LParen)
            && self.nth_op(n + 1, "+")
            && self.nth_kind(n + 2) == Some(TokenKind::RParen)
    }

    /// Builds a dotted name of `parts` parts (and a missing last part when
    /// `missing`) as nested nodes: `outer` around the qualifiers, each
    /// qualifier typed by its depth. With `keep_open` the outer node is left
    /// open for the caller.
    fn chain(&mut self, outer: SyntaxKind, parts: usize, missing: bool, keep_open: bool) {
        let slots = parts + usize::from(missing);
        let mut kind = outer;
        for _ in 0..slots {
            self.start(kind);
            kind = qualifier(kind);
        }
        for s in 0..slots {
            if s > 0 {
                self.bump(); // `.`
            }
            if s < parts {
                self.name_part();
            } else {
                let name = match outer {
                    SyntaxKind::TableRef => NameKind::Table,
                    SyntaxKind::FunctionRef => NameKind::Function,
                    SyntaxKind::TypeRef => NameKind::Type,
                    SyntaxKind::ObjectRef => NameKind::Object,
                    SyntaxKind::SchemaRef => NameKind::Schema,
                    _ => NameKind::Column,
                };
                self.note(Expected::Name(name));
                if name == NameKind::Column {
                    self.note(Expected::Punct("*"));
                }
                self.missing();
            }
            if !(keep_open && s + 1 == slots) {
                self.finish();
            }
        }
    }

    /// Enters a nesting level; past the limit the construct is kept as an
    /// `Opaque` node and the caller must not recurse.
    fn enter(&mut self) -> bool {
        if self.depth >= MAX_DEPTH {
            if !self.at_recovery() && self.nth_kind(0) != Some(TokenKind::Comma) {
                self.opaque_until(true);
            }
            return false;
        }
        self.depth += 1;
        true
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// `( f )` with the `(` recorded as missing when it is not there.
    fn parened(&mut self, f: impl FnOnce(&mut Self)) {
        let open = self.expect(TokenKind::LParen);
        if open {
            self.parens += 1;
        }
        f(self);
        if open {
            self.parens -= 1;
            self.expect(TokenKind::RParen);
        }
    }

    // --- Units and statements.

    fn unit(&mut self) {
        loop {
            if self.eof() {
                break;
            }
            if self.at_terminator() {
                self.bump();
                continue;
            }
            let before = self.pos;
            self.statement();
            if self.pos == before {
                // Never stall: the token becomes an error.
                let set = self.snapshot();
                self.start(SyntaxKind::Error);
                self.bump();
                self.b.finish(Some(set));
            }
        }
        if self.probe && self.probe_result.is_none() {
            let after_terminator = self.pos == 0
                || self
                    .pos
                    .checked_sub(1)
                    .and_then(|p| self.sig.get(p))
                    .is_some_and(|&i| {
                        matches!(
                            self.toks[i as usize].kind,
                            TokenKind::Semicolon | TokenKind::SlashLine
                        )
                    });
            if after_terminator {
                self.expected.clear();
                self.note_statement_start();
            }
            let set = self.snapshot();
            self.probe_result = Some(set.to_vec());
        }
    }

    fn note_statement_start(&mut self) {
        for k in STATEMENT_STARTS {
            self.note(Expected::Keyword(k));
        }
        match self.dialect {
            Dialect::Postgres => {
                self.note(Expected::Keyword("TABLE"));
                self.note(Expected::Keyword("DO"));
            }
            Dialect::Oracle => {
                self.note(Expected::Keyword("DECLARE"));
            }
            Dialect::Sqlite => self.note(Expected::Keyword("PRAGMA")),
            Dialect::Generic => {}
        }
        self.note(Expected::Punct("("));
    }

    /// The index after the last significant token that is not a terminator.
    fn statement_end(&self) -> usize {
        let mut end = self.sig.len();
        while end > self.pos {
            let k = self.toks[self.sig[end - 1] as usize].kind;
            if !matches!(k, TokenKind::Semicolon | TokenKind::SlashLine) {
                break;
            }
            end -= 1;
        }
        end
    }

    fn statement(&mut self) {
        use Kw::*;
        let query = match self.nth_kw(0) {
            Some(SELECT | VALUES | WITH) => true,
            Some(TABLE) => self.g.table_statement && self.is_name_at(1),
            _ => self.nth_kind(0) == Some(TokenKind::LParen),
        };
        if query {
            self.query_statement();
        } else if self.dml_at() {
            self.dml(None);
        } else if self.nth_kind(0) == Some(TokenKind::Hole)
            && self.part_len(0) + self.pos >= self.statement_end()
        {
            self.hole();
        } else {
            self.opaque_statement();
            return;
        }
        // Whatever is left before the terminator.
        if !self.eof() && !self.at_terminator() {
            if self.nth_kind(0) == Some(TokenKind::Hole) {
                self.hole();
                return;
            }
            let set = self.snapshot();
            self.start(SyntaxKind::Error);
            while !self.eof() && !self.at_terminator() {
                self.bump();
            }
            self.b.finish(Some(set));
        }
    }

    fn opaque_statement(&mut self) {
        let end = self.statement_end();
        self.start(SyntaxKind::Opaque);
        while self.pos < end {
            self.bump();
        }
        self.finish();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tail {
    /// Nothing special after the last part.
    Name,
    /// `.*` after the last part.
    Star,
    /// A `.` with no name after it.
    Missing,
    /// A `(` after the last part.
    Call,
}

/// The kind of the qualifier of a name of `kind`.
fn qualifier(kind: SyntaxKind) -> SyntaxKind {
    match kind {
        SyntaxKind::ColumnRef => SyntaxKind::TableRef,
        SyntaxKind::TableRef
        | SyntaxKind::FunctionRef
        | SyntaxKind::TypeRef
        | SyntaxKind::ObjectRef => SyntaxKind::SchemaRef,
        _ => SyntaxKind::CatalogRef,
    }
}

fn punct(kind: TokenKind) -> Option<&'static str> {
    Some(match kind {
        TokenKind::Comma => ",",
        TokenKind::Dot => ".",
        TokenKind::LParen => "(",
        TokenKind::RParen => ")",
        TokenKind::LBracket => "[",
        TokenKind::RBracket => "]",
        TokenKind::Semicolon => ";",
        _ => return None,
    })
}

const STATEMENT_STARTS: [&str; 16] = [
    "SELECT", "WITH", "VALUES", "INSERT", "UPDATE", "DELETE", "MERGE", "CREATE", "ALTER", "DROP",
    "TRUNCATE", "EXPLAIN", "COMMIT", "ROLLBACK", "GRANT", "REVOKE",
];
