// Routines, triggers, types and packages: CREATE FUNCTION / PROCEDURE (also
// with a body in `$$ .. $$`, parsed as an embedded fragment), CREATE TRIGGER,
// CREATE TYPE [BODY], CREATE PACKAGE [BODY], and PostgreSQL's DO.
use super::block::BlockStart;
use super::{Kw, Parser, Proc};
use crate::dialect::Dialect;
use crate::lexer::{Token, TokenKind, lex};
use crate::tree::{Expected, Fragment, FragmentLanguage, NameKind, StatementTree, SyntaxKind};

impl Parser<'_> {
    /// The `LANGUAGE x` of the statement, lower case.
    fn routine_language(&self) -> Option<String> {
        let mut i = self.pos;
        while i < self.sig.len() {
            let t = &self.toks[self.sig[i] as usize];
            match t.kind {
                TokenKind::Semicolon | TokenKind::SlashLine => return None,
                TokenKind::LParen => match self.close[i] {
                    super::NO_CLOSE => return None,
                    c => {
                        i = c as usize + 1;
                        continue;
                    }
                },
                TokenKind::Word if self.text[t.start..t.end].eq_ignore_ascii_case("language") => {
                    let n = &self.toks[*self.sig.get(i + 1)? as usize];
                    let w = &self.text[n.start..n.end];
                    return match n.kind {
                        TokenKind::Word | TokenKind::QuotedIdent | TokenKind::String => Some(
                            w.trim_matches(|c| c == '\'' || c == '"')
                                .to_ascii_lowercase(),
                        ),
                        _ => None,
                    };
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    // --- Functions and procedures.

    pub(super) fn create_routine(&mut self) {
        use Kw::*;
        self.chain_of(SyntaxKind::FunctionRef);
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.param_list();
        }
        let language = self.routine_language();
        let mut returned = false;
        if self.peek_kw(RETURNS) || (self.peek_kw(RETURN) && self.dialect != Dialect::Postgres) {
            self.returns_clause();
            returned = true;
        }
        loop {
            self.routine_options(returned);
            if self.eof() || self.at_terminator() {
                break;
            }
            if self.peek_kw(AS) || self.peek_kw(IS) {
                if self.nth_kind(1) == Some(TokenKind::String) {
                    self.bump();
                    self.embedded_body(language.as_deref());
                    // `AS 'object file', 'link symbol'`
                    if self.nth_kind(0) == Some(TokenKind::Comma) {
                        self.bump();
                        if self.nth_kind(0) == Some(TokenKind::String) {
                            self.bump();
                        }
                    }
                } else if self.g.plsql
                    && !(self.nth_word(1, "language") || self.nth_word(1, "external"))
                {
                    self.bump();
                    self.block(BlockStart::Body);
                } else {
                    self.opaque_to_end();
                }
            } else if self.peek_kw(BEGIN) || self.peek_kw(DECLARE) {
                self.block(BlockStart::Full);
            } else if self.peek_kw(RETURN) {
                // PostgreSQL 14: `RETURN expr` as the body.
                let saved = self.proc;
                self.proc = Proc::PlPgSql;
                self.proc_statement();
                self.proc = saved;
            } else {
                self.opaque_to_end();
            }
            returned = true;
        }
    }

    /// Whether the current token starts a routine option.
    fn routine_option_at(&self) -> bool {
        use Kw::*;
        match self.nth_kw(0) {
            Some(SET | DEFAULT) => return true,
            Some(RETURNS) => return self.nth_kw(1) == Some(NULL),
            Some(NOT) => {
                return self.nth_word(1, "leakproof") || self.nth_word(1, "deterministic");
            }
            _ => {}
        }
        const OPTIONS: [&str; 24] = [
            "language",
            "immutable",
            "stable",
            "volatile",
            "leakproof",
            "strict",
            "called",
            "security",
            "external",
            "parallel",
            "cost",
            "rows",
            "support",
            "window",
            "transform",
            "deterministic",
            "pipelined",
            "parallel_enable",
            "result_cache",
            "authid",
            "accessible",
            "sharing",
            "definer",
            "invoker",
        ];
        self.nth_kind(0) == Some(TokenKind::Word) && OPTIONS.iter().any(|w| self.nth_word(0, w))
    }

    /// Routine options (`LANGUAGE x`, `IMMUTABLE`, `AUTHID ..`) up to the
    /// body. `returned`: a `RETURN` is the start of the body.
    pub(super) fn routine_options(&mut self, returned: bool) {
        use Kw::*;
        loop {
            if self.eof()
                || self.at_terminator()
                || self.peek_kw(AS)
                || self.peek_kw(IS)
                || self.peek_kw(BEGIN)
                || self.peek_kw(DECLARE)
                || (self.peek_kw(RETURN) && (returned || self.dialect == Dialect::Postgres))
            {
                break;
            }
            if self.routine_option_at() {
                self.start(SyntaxKind::RoutineOption);
                self.bump();
                while !self.eof()
                    && !self.at_terminator()
                    && !self.peek_kw(AS)
                    && !self.peek_kw(IS)
                    && !self.peek_kw(BEGIN)
                    && !self.peek_kw(DECLARE)
                    && !self.peek_kw(RETURN)
                    && !self.routine_option_at()
                {
                    self.bump_group();
                }
                self.finish();
            } else if self.peek_kw(RETURNS) || self.peek_kw(RETURN) {
                self.returns_clause();
            } else {
                self.start(SyntaxKind::Opaque);
                loop {
                    self.bump_group();
                    if self.eof()
                        || self.at_terminator()
                        || self.peek_kw(AS)
                        || self.peek_kw(IS)
                        || self.peek_kw(BEGIN)
                        || self.routine_option_at()
                    {
                        break;
                    }
                }
                self.finish();
            }
        }
    }

    // --- Bodies in strings.

    /// A routine body written as a string. When the language is known and the
    /// quotes need no unescaping, the program inside is parsed as a fragment.
    pub(super) fn embedded_body(&mut self, language: Option<&str>) {
        self.start(SyntaxKind::EmbeddedBody);
        match self.nth(0) {
            Some(&tok) if tok.kind == TokenKind::String => {
                let index = self.sig[self.pos] as usize;
                self.bump();
                let lang = match language {
                    Some("plpgsql") => Some(FragmentLanguage::PlPgSql),
                    Some("sql") => Some(FragmentLanguage::Sql),
                    _ => None,
                };
                if let Some(lang) = lang
                    && self.g.plpgsql
                    && let Some((offset, len)) = self.string_body(&tok)
                {
                    let body = &self.text[tok.start + offset..tok.start + offset + len];
                    let tree = parse_fragment(body, lang, self.dialect);
                    self.b
                        .fragment(Fragment::new(index, tok.start + offset, lang, tree));
                }
            }
            _ => self.missing(),
        }
        self.finish();
    }

    /// The offset and length of the text inside the quotes of a string token,
    /// if the text is the program as written (no escapes to undo).
    fn string_body(&self, tok: &Token) -> Option<(usize, usize)> {
        if tok.unterminated {
            return None;
        }
        let s = self.text.get(tok.start..tok.end)?;
        if let Some(rest) = s.strip_prefix('$') {
            let tag = rest.find('$')? + 2;
            let delim = &s[..tag];
            if s.len() < 2 * tag || !s.ends_with(delim) {
                return None;
            }
            Some((tag, s.len() - 2 * tag))
        } else if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2 {
            let inner = &s[1..s.len() - 1];
            if inner.contains("''") || inner.contains('\\') {
                None
            } else {
                Some((1, inner.len()))
            }
        } else {
            None
        }
    }

    /// `DO [LANGUAGE x] 'code' [LANGUAGE x]`.
    pub(super) fn do_statement(&mut self) {
        self.start(SyntaxKind::DoStatement);
        self.bump();
        let language = self
            .routine_language()
            .unwrap_or_else(|| "plpgsql".to_string());
        let mut body = false;
        loop {
            if self.nth_word(0, "language") {
                self.start(SyntaxKind::RoutineOption);
                self.bump();
                if self.nth_kind(0) != Some(TokenKind::Semicolon) && !self.eof() {
                    self.bump();
                }
                self.finish();
            } else if !body && self.nth_kind(0) == Some(TokenKind::String) {
                self.embedded_body(Some(&language));
                body = true;
            } else {
                break;
            }
        }
        if !body {
            self.note(Expected::Keyword("LANGUAGE"));
            self.missing();
        }
        self.finish();
    }

    // --- Triggers.

    pub(super) fn create_trigger(&mut self) {
        use Kw::*;
        self.opt_if(true);
        self.object_name();
        if self.peek_kw(BEFORE) || self.peek_kw(AFTER) {
            self.bump();
        } else if self.at_kw(INSTEAD) {
            self.bump();
            self.expect_kw(OF);
        } else if self.peek_kw(FOR) || self.nth_word(0, "compound") {
            // Oracle: FOR ... COMPOUND TRIGGER
        } else {
            self.at_kw(BEFORE);
            self.at_kw(AFTER);
            self.missing();
        }
        // Events.
        loop {
            // Oracle also has system events (`LOGON`, `DDL`, `SERVERERROR`).
            let event = matches!(self.nth_kw(0), Some(INSERT | UPDATE | DELETE | TRUNCATE))
                || (self.g.plsql && self.nth_kind(0) == Some(TokenKind::Word) && !self.peek_kw(ON));
            if !event {
                if self.eof() || self.at_terminator() {
                    break;
                }
                self.at_kw(INSERT);
                self.at_kw(UPDATE);
                self.at_kw(DELETE);
                self.missing();
                break;
            }
            self.start(SyntaxKind::TriggerEvent);
            let update = self.peek_kw(UPDATE);
            self.bump();
            if update && self.peek_kw(OF) {
                self.bump();
                loop {
                    self.column_name_ref();
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.finish();
            if !(self.eat_kw(OR) || self.eat(TokenKind::Comma)) {
                break;
            }
        }
        if self.at_kw(ON) {
            self.bump();
            self.table_name();
        } else if !self.eof() && !self.at_terminator() {
            self.missing();
        }
        // Options and the body.
        loop {
            if self.eof() || self.at_terminator() {
                break;
            }
            if self.peek_kw(WHEN) {
                self.start(SyntaxKind::TriggerCondition);
                self.bump();
                self.expr();
                self.finish();
            } else if self.peek_kw(BEGIN) || self.peek_kw(DECLARE) {
                self.block(BlockStart::Full);
            } else if self.nth_word(0, "compound") {
                self.opaque_to_end();
            } else if self.peek_kw(EXECUTE) {
                self.bump();
                if self.peek_kw(FUNCTION) || self.peek_kw(PROCEDURE) {
                    self.bump();
                }
                self.call_in_clause();
            } else if self.nth_word(0, "call") {
                self.bump();
                self.call_in_clause();
            } else if self.nth_word(0, "referencing") || self.peek_kw(FROM) {
                self.start(SyntaxKind::Opaque);
                loop {
                    self.bump_group();
                    if self.eof()
                        || self.at_terminator()
                        || self.peek_kw(FOR)
                        || self.peek_kw(WHEN)
                        || self.peek_kw(EXECUTE)
                        || self.peek_kw(BEGIN)
                        || self.peek_kw(DECLARE)
                        || self.nth_word(0, "call")
                    {
                        break;
                    }
                }
                self.finish();
            } else {
                // FOR EACH ROW | STATEMENT, [NOT] DEFERRABLE, FOLLOWS x, ENABLE, ...
                self.bump_group();
            }
        }
    }

    // --- Types and packages.

    pub(super) fn create_type(&mut self) {
        use Kw::*;
        self.opt_if(true);
        self.chain_of(SyntaxKind::TypeRef);
        while self.nth_word(0, "force") || self.nth_word(0, "oid") || self.nth_word(0, "authid") {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::Word)
                || self.nth_kind(0) == Some(TokenKind::String)
            {
                self.bump();
            }
        }
        if self.peek_kw(AS) || self.peek_kw(IS) {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.type_members();
            } else if self.nth_word(0, "object") {
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.type_members();
                }
            }
        }
        self.opaque_to_end();
    }

    /// `( attribute type, MEMBER FUNCTION .. )`.
    fn type_members(&mut self) {
        self.parened(|p| {
            if p.nth_kind(0) != Some(TokenKind::RParen) {
                loop {
                    if [
                        "member",
                        "static",
                        "constructor",
                        "map",
                        "order",
                        "final",
                        "instantiable",
                        "overriding",
                    ]
                    .iter()
                    .any(|w| p.nth_word(0, w))
                        || p.peek_kw(Kw::NOT)
                    {
                        p.opaque_item();
                    } else if p.is_name_at(0) {
                        p.column_def();
                    } else {
                        p.note(Expected::Name(NameKind::Column));
                        p.recover_item();
                    }
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
            }
            p.rest_of_group();
        });
    }

    pub(super) fn create_type_body(&mut self) {
        use Kw::*;
        self.chain_of(SyntaxKind::TypeRef);
        while self.nth_word(0, "authid") {
            self.bump();
            self.bump();
        }
        if self.peek_kw(AS) || self.peek_kw(IS) {
            self.bump();
            self.member_subprograms();
        } else {
            self.at_kw(AS);
            self.missing();
        }
    }

    /// `MEMBER FUNCTION .. IS .. END; ..` up to the closing `END`.
    fn member_subprograms(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::DeclareSection);
        loop {
            if self.eof() || self.peek_kw(END) {
                break;
            }
            let before = self.pos;
            while [
                "member",
                "static",
                "constructor",
                "map",
                "order",
                "final",
                "instantiable",
                "overriding",
            ]
            .iter()
            .any(|w| self.nth_word(0, w))
                || (self.peek_kw(NOT) && self.nth_word(1, "final"))
            {
                self.bump();
            }
            if self.peek_kw(FUNCTION) || self.peek_kw(PROCEDURE) {
                self.subprogram_decl();
                self.end_statement();
            } else {
                self.error_until(false, None);
                self.eat(TokenKind::Semicolon);
            }
            if self.pos == before {
                self.error_until(false, None);
            }
        }
        self.finish();
        if self.at_kw(END) {
            self.bump();
            self.end_name();
        } else {
            self.missing();
        }
    }

    pub(super) fn create_package(&mut self, body: bool) {
        use Kw::*;
        self.opt_if(true);
        self.object_name();
        while self.nth_word(0, "authid")
            || self.nth_word(0, "accessible")
            || self.nth_word(0, "sharing")
        {
            self.bump_group();
            if !self.peek_kw(IS) && !self.peek_kw(AS) && !self.eof() && !self.at_terminator() {
                self.bump_group();
            }
        }
        if !(self.eat_kw(IS) || self.eat_kw(AS)) {
            self.missing();
            return;
        }
        let saved = self.proc;
        if saved == Proc::No {
            self.proc = if self.dialect == Dialect::Postgres {
                Proc::PlPgSql
            } else {
                Proc::PlSql
            };
        }
        self.declare_section();
        if body && self.at_kw(BEGIN) {
            self.bump();
            self.statement_list();
            if self.peek_kw(EXCEPTION) {
                self.exception_section();
            }
        }
        if self.at_kw(END) {
            self.bump();
            self.end_name();
        } else {
            self.missing();
        }
        self.proc = saved;
    }
}

/// Parses the program inside a string as a unit of its own.
fn parse_fragment(body: &str, language: FragmentLanguage, dialect: Dialect) -> StatementTree {
    let toks = lex(body, dialect);
    let parts = {
        let mut p = Parser::new(body, &toks, dialect, false);
        if language == FragmentLanguage::PlPgSql {
            p.proc = Proc::PlPgSql;
        }
        p.unit();
        p.b.finish_root(body.len())
    };
    parts.into_tree(toks)
}
