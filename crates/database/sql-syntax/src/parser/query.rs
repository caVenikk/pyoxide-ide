// Queries: WITH, set operations, SELECT with its clauses, FROM items and
// joins, VALUES, ORDER BY, LIMIT / OFFSET / FETCH, locking clauses.
use super::{Kw, Parser, Tail};
use crate::dialect::Dialect;
use crate::lexer::TokenKind;
use crate::tree::{Expected, NameKind, SyntaxKind};

/// The clauses of a select after the select list, in their order.
const SELECT_CLAUSES: usize = 7;

impl Parser<'_> {
    /// A query begins at the `n`th token.
    pub(super) fn query_starts(&self, n: usize) -> bool {
        matches!(self.nth_kw(n), Some(Kw::SELECT | Kw::WITH | Kw::VALUES))
    }

    /// A query statement; `WITH` followed by something else than a query
    /// (data-modifying statements) is kept as an `Opaque` node around the
    /// parsed `WITH` clause.
    pub(super) fn query_statement(&mut self) {
        if !self.peek_kw(Kw::WITH) {
            self.query();
            return;
        }
        let cp = self.checkpoint();
        self.with_clause();
        if self.query_body_starts() {
            self.query_rest(cp);
        } else if self.dml_at() {
            self.dml(Some(cp));
        } else {
            let end = self.statement_end();
            self.start_at(cp, SyntaxKind::Opaque);
            while self.pos < end {
                self.bump();
            }
            self.finish();
        }
    }

    fn query_body_starts(&self) -> bool {
        matches!(self.nth_kw(0), Some(Kw::SELECT | Kw::VALUES))
            || self.nth_kind(0) == Some(TokenKind::LParen)
            || (self.peek_kw(Kw::TABLE) && self.g.table_statement)
    }

    pub(super) fn query(&mut self) {
        if !self.enter() {
            return;
        }
        let cp = self.checkpoint();
        if self.at_kw(Kw::WITH) {
            self.with_clause();
        }
        self.query_rest(cp);
        self.leave();
    }

    fn query_rest(&mut self, cp: usize) {
        self.set_expr(0);
        self.query_tail();
        self.start_at(cp, SyntaxKind::Query);
        self.finish();
    }

    fn with_clause(&mut self) {
        self.start(SyntaxKind::WithClause);
        self.bump();
        self.eat_kw(Kw::RECURSIVE);
        loop {
            self.cte();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    fn cte(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::Cte);
        if self.is_name_at(0) {
            self.start(SyntaxKind::CteDef);
            self.name_part();
            self.finish();
        } else {
            self.note(Expected::Name(NameKind::Cte));
            self.missing();
        }
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_alias_list();
        }
        self.expect_kw(AS);
        if self.eat_kw(NOT) {
            self.expect_kw(MATERIALIZED);
        } else {
            self.eat_kw(MATERIALIZED);
        }
        self.parened(|p| {
            if p.query_starts(0) || p.nth_kind(0) == Some(TokenKind::LParen) {
                p.query();
            } else if p.dml_at() {
                // A data-modifying statement (PostgreSQL).
                p.dml(None);
            } else if p.nth_kind(0) != Some(TokenKind::RParen) {
                p.opaque_until(false);
            } else {
                p.at_kw(SELECT);
                p.missing();
            }
        });
        if self.peek_kw(SEARCH) || self.peek_kw(CYCLE) {
            self.opaque_until(true);
        }
        self.finish();
    }

    /// Query bodies combined by set operations; `INTERSECT` binds tighter.
    fn set_expr(&mut self, min: u8) {
        let cp = self.checkpoint();
        self.query_primary();
        loop {
            let bp = if self.at_kw(Kw::UNION)
                || self.at_kw(Kw::EXCEPT)
                || (self.g.minus && self.at_kw(Kw::MINUS))
            {
                1
            } else if self.at_kw(Kw::INTERSECT) {
                2
            } else {
                break;
            };
            if bp < min {
                break;
            }
            self.bump();
            if !self.eat_kw(Kw::ALL) {
                self.eat_kw(Kw::DISTINCT);
            }
            self.set_expr(bp + 1);
            self.start_at(cp, SyntaxKind::SetOperation);
            self.finish();
        }
    }

    fn query_primary(&mut self) {
        if self.at_kw(Kw::SELECT) {
            self.select();
        } else if self.at_kw(Kw::VALUES) {
            self.values();
        } else if self.g.table_statement && self.at_kw(Kw::TABLE) {
            self.start(SyntaxKind::TableQuery);
            self.bump();
            self.table_name();
            self.finish();
        } else if self.at(TokenKind::LParen) {
            self.start(SyntaxKind::ParenQuery);
            self.parened(|p| p.query());
            self.finish();
        } else {
            self.missing_or_error();
        }
    }

    pub(super) fn values(&mut self) {
        self.start(SyntaxKind::Values);
        self.bump();
        loop {
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.tuple();
            } else {
                self.at(TokenKind::LParen);
                self.missing_or_error();
                break;
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    fn select(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::Select);
        self.bump();
        if self.at_kw(DISTINCT) {
            self.start(SyntaxKind::DistinctClause);
            self.bump();
            if self.g.distinct_on && self.at_kw(ON) {
                self.bump();
                self.parened(|p| p.expr_list());
            }
            self.finish();
        } else if !self.eat_kw(ALL) && self.g.select_unique {
            self.eat_kw(UNIQUE);
        }
        self.select_list();
        let mut stage = 0;
        loop {
            match self.select_clause_at(stage) {
                Some(i) if i >= stage => {
                    self.select_clause(i);
                    stage = i + 1;
                }
                Some(i) => {
                    // A clause out of its place: parsed, inside an error.
                    let set = self.snapshot();
                    self.start(SyntaxKind::Error);
                    self.select_clause(i);
                    self.b.finish(Some(set));
                }
                None => {
                    if self.at_select_end() {
                        break;
                    }
                    if self.dialect == Dialect::Generic && self.nth_kind(0) == Some(TokenKind::Word)
                    {
                        self.opaque_until(false);
                    } else {
                        self.error_until(false, None);
                    }
                }
            }
        }
        self.finish();
    }

    /// The select clause at the current token; clauses from `stage` on are
    /// added to the expected set.
    fn select_clause_at(&mut self, stage: usize) -> Option<usize> {
        use Kw::*;
        let g = self.g;
        for i in 0..SELECT_CLAUSES {
            let hit = match i {
                0 => {
                    if self.in_procedure() && self.at_into() {
                        true
                    } else {
                        self.check_kw(INTO, i >= stage)
                    }
                }
                1 => self.check_kw(FROM, i >= stage),
                2 => self.check_kw(WHERE, i >= stage),
                3 => {
                    g.connect_by
                        && ((self.check_kw(START, i >= stage) && self.nth_kw(1) == Some(WITH))
                            || self.check_kw(CONNECT, i >= stage))
                }
                4 => self.check_kw(GROUP, i >= stage),
                5 => self.check_kw(HAVING, i >= stage),
                _ => self.check_kw(WINDOW, i >= stage),
            };
            if hit {
                return Some(i);
            }
        }
        None
    }

    fn check_kw(&mut self, kw: Kw, note: bool) -> bool {
        if note {
            self.at_kw(kw)
        } else {
            self.peek_kw(kw)
        }
    }

    fn select_clause(&mut self, i: usize) {
        use Kw::*;
        match i {
            0 if self.in_procedure() => self.into_clause(),
            0 => {
                self.start(SyntaxKind::IntoClause);
                self.bump();
                if self.nth_word(0, "temp")
                    || self.nth_word(0, "temporary")
                    || self.nth_word(0, "unlogged")
                {
                    self.bump();
                }
                self.eat_kw(TABLE);
                loop {
                    self.table_name();
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.finish();
            }
            1 => self.source_clause(),
            2 => {
                self.start(SyntaxKind::WhereClause);
                self.bump();
                self.expr();
                self.finish();
            }
            3 => {
                self.start(SyntaxKind::HierarchicalClause);
                for _ in 0..2 {
                    if self.at_kw(START) && self.nth_kw(1) == Some(WITH) {
                        self.bump();
                        self.bump();
                        self.expr();
                    } else if self.at_kw(CONNECT) {
                        self.bump();
                        self.expect_kw(BY);
                        self.eat_kw(NOCYCLE);
                        self.expr();
                    } else {
                        break;
                    }
                }
                self.finish();
            }
            4 => {
                self.start(SyntaxKind::GroupByClause);
                self.bump();
                self.expect_kw(BY);
                if !self.eat_kw(ALL) {
                    self.eat_kw(DISTINCT);
                }
                loop {
                    self.group_item();
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.finish();
            }
            5 => {
                self.start(SyntaxKind::HavingClause);
                self.bump();
                self.expr();
                self.finish();
            }
            _ => {
                self.start(SyntaxKind::WindowClause);
                self.bump();
                loop {
                    self.start(SyntaxKind::WindowDecl);
                    if self.is_name_at(0) {
                        self.start(SyntaxKind::WindowDef);
                        self.name_part();
                        self.finish();
                    } else {
                        self.note(Expected::Name(NameKind::Window));
                        self.missing();
                    }
                    self.expect_kw(AS);
                    self.window_spec();
                    self.finish();
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.finish();
            }
        }
    }

    /// What ends a select: the end of the query or of the parentheses
    /// around it, a set operation, or a clause of the whole query.
    fn at_select_end(&self) -> bool {
        use Kw::*;
        let g = self.g;
        match self.nth_kind(0) {
            None | Some(TokenKind::Semicolon | TokenKind::SlashLine) => return true,
            Some(TokenKind::RParen) => return self.parens > 0,
            _ => {}
        }
        if g.plsql && self.nth_word(0, "log") && self.nth_word(1, "errors") {
            return true;
        }
        match self.nth_kw(0) {
            Some(ORDER | UNION | INTERSECT | EXCEPT) => true,
            Some(RETURNING) => g.returning,
            Some(ON) => g.on_conflict && self.nth_kw(1) == Some(CONFLICT),
            Some(MINUS) => g.minus,
            Some(LIMIT) => g.limit,
            Some(OFFSET) => g.limit || g.fetch_first,
            Some(FETCH) => g.fetch_first,
            Some(FOR) => matches!(self.nth_kw(1), Some(UPDATE | SHARE | NO | KEY)),
            _ => false,
        }
    }

    fn select_list(&mut self) {
        self.start(SyntaxKind::SelectList);
        let empty = self.g.empty_select_list
            && (self.at_recovery() || self.at_select_end() || self.at_into());
        if empty {
            self.at_op("*");
            self.note(Expected::Expression);
        } else {
            loop {
                self.select_item();
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                if self.at_recovery() || self.at_select_end() || self.at_into() {
                    break;
                }
                self.error_until(true, None);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.finish();
    }

    pub(super) fn select_item(&mut self) {
        self.start(SyntaxKind::SelectItem);
        if self.at_op("*") {
            self.start(SyntaxKind::Star);
            self.bump();
            self.finish();
        } else {
            self.expr();
            self.opt_alias(false);
        }
        self.finish();
    }

    /// `[AS] alias`, and with `columns` a column list `(a, b)` after it.
    pub(super) fn opt_alias(&mut self, columns: bool) {
        use Kw::*;
        let flashback = self.peek_kw(AS)
            && self.nth_kw(1) == Some(OF)
            && (self.nth_word(2, "timestamp") || self.nth_word(2, "scn"));
        if flashback {
            return;
        }
        let mut aliased = false;
        if self.at_kw(AS) {
            self.start(SyntaxKind::AliasDef);
            self.bump();
            if self.is_name_at(0) || self.nth_kind(0) == Some(TokenKind::String) {
                self.name_or_string();
            } else {
                self.note(Expected::Name(NameKind::Alias));
                self.missing();
            }
            self.finish();
            aliased = true;
        } else {
            if self.probe {
                self.note(Expected::Name(NameKind::Alias));
            }
            if self.alias_ok(0) {
                self.start(SyntaxKind::AliasDef);
                self.name_part();
                self.finish();
                aliased = true;
            }
        }
        if columns && aliased && self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_alias_list();
        }
    }

    fn name_or_string(&mut self) {
        if self.nth_kind(0) == Some(TokenKind::String) {
            self.bump();
        } else {
            self.name_part();
        }
    }

    /// `(a, b)`: column aliases, each with an optional type (column
    /// definitions of a function in `FROM`).
    fn column_alias_list(&mut self) {
        self.start(SyntaxKind::ColumnAliasList);
        self.parened(|p| {
            loop {
                p.start(SyntaxKind::AliasDef);
                if p.is_name_at(0) {
                    p.name_part();
                } else {
                    p.note(Expected::Name(NameKind::Alias));
                    p.missing();
                }
                p.finish();
                if p.is_name_at(0) {
                    p.type_ref();
                }
                if !p.eat(TokenKind::Comma) {
                    break;
                }
            }
        });
        self.finish();
    }

    /// A table name as a `TableRef` (qualified, possibly with a missing last
    /// part), or a `Missing` node expecting one.
    pub(super) fn table_name(&mut self) {
        match self.chain_ahead() {
            Some((parts, tail)) => {
                let missing = matches!(tail, Tail::Missing | Tail::Star);
                self.chain(SyntaxKind::TableRef, parts, missing, false);
            }
            None => {
                self.note(Expected::Name(NameKind::Table));
                self.missing_or_error();
            }
        }
    }

    pub(super) fn source_clause(&mut self) {
        self.start(SyntaxKind::FromClause);
        self.bump();
        loop {
            self.source_item();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    /// A source with the joins after it.
    pub(super) fn source_item(&mut self) {
        use Kw::*;
        let cp = self.checkpoint();
        self.table_primary();
        loop {
            let g = self.g;
            let natural = self.at_kw(NATURAL);
            let join = natural
                || self.at_kw(JOIN)
                || self.at_kw(INNER)
                || self.at_kw(LEFT)
                || self.at_kw(RIGHT)
                || self.at_kw(FULL)
                || self.at_kw(CROSS)
                || (g.apply_joins && self.peek_kw(OUTER) && self.nth_kw(1) == Some(APPLY));
            if !join {
                break;
            }
            let mut condition = !natural;
            self.eat_kw(NATURAL);
            if self.eat_kw(CROSS) {
                condition = false;
                if !(g.apply_joins && self.eat_kw(APPLY)) {
                    self.expect_kw(JOIN);
                }
            } else if self.peek_kw(OUTER) && self.nth_kw(1) == Some(APPLY) {
                condition = false;
                self.bump();
                self.bump();
            } else {
                if !self.eat_kw(INNER)
                    && (self.eat_kw(LEFT) || self.eat_kw(RIGHT) || self.eat_kw(FULL))
                {
                    self.eat_kw(OUTER);
                }
                self.expect_kw(JOIN);
            }
            self.table_primary();
            if condition {
                if self.at_kw(ON) {
                    self.start(SyntaxKind::OnClause);
                    self.bump();
                    self.expr();
                    self.finish();
                } else if self.at_kw(USING) {
                    self.start(SyntaxKind::UsingClause);
                    self.bump();
                    self.parened(|p| {
                        loop {
                            if p.is_name_at(0) {
                                p.start(SyntaxKind::ColumnRef);
                                p.name_part();
                                p.finish();
                            } else {
                                p.note(Expected::Name(NameKind::Column));
                                p.missing();
                            }
                            if !p.eat(TokenKind::Comma) {
                                break;
                            }
                        }
                    });
                    self.opt_alias(false);
                    self.finish();
                } else if g.join_requires_condition {
                    self.missing();
                }
            }
            self.start_at(cp, SyntaxKind::Join);
            self.finish();
        }
    }

    /// A table, a subquery, a function or a parenthesised join, with its
    /// alias.
    pub(super) fn table_primary(&mut self) {
        use Kw::*;
        if !self.enter() {
            return;
        }
        let lateral = self.g.lateral && self.at_kw(LATERAL);
        let n = usize::from(lateral);
        if self.nth_kind(n) == Some(TokenKind::LParen) {
            // A query inside, possibly behind more parentheses?
            let mut k = n;
            while self.nth_kind(k) == Some(TokenKind::LParen) {
                k += 1;
            }
            let kind = if self.query_starts(k) {
                SyntaxKind::SubquerySource
            } else {
                SyntaxKind::ParenSource
            };
            self.start(kind);
            if lateral {
                self.bump();
            }
            self.bump();
            self.parens += 1;
            if kind == SyntaxKind::SubquerySource {
                self.query();
            } else {
                self.source_item();
            }
            self.parens -= 1;
            self.expect(TokenKind::RParen);
            self.source_suffixes();
            self.finish();
        } else if lateral {
            let call = matches!(self.lookahead_chain(1), Some((_, Tail::Call)));
            self.start(if call {
                SyntaxKind::FunctionSource
            } else {
                SyntaxKind::TableSource
            });
            self.bump();
            self.source_body();
            self.finish();
        } else if self.g.table_statement && self.peek_kw(ONLY) && self.is_name_at(1) {
            self.start(SyntaxKind::TableSource);
            self.bump();
            self.source_body();
            self.finish();
        } else if let Some((_, tail)) = self.chain_ahead() {
            self.start(if tail == Tail::Call {
                SyntaxKind::FunctionSource
            } else {
                SyntaxKind::TableSource
            });
            self.source_body();
            self.finish();
        } else {
            self.note(Expected::Name(NameKind::Table));
            self.at(TokenKind::LParen);
            self.missing_or_error();
        }
        self.leave();
    }

    /// `chain_ahead` from the `n`th token.
    fn lookahead_chain(&mut self, n: usize) -> Option<(usize, Tail)> {
        self.pos += n;
        let r = self.chain_ahead();
        self.pos -= n;
        r
    }

    /// The name or call of a source, then its suffixes.
    fn source_body(&mut self) {
        match self.chain_ahead() {
            Some((parts, Tail::Call)) => {
                self.start(SyntaxKind::CallExpr);
                self.chain(SyntaxKind::FunctionRef, parts, false, false);
                self.call_suffix_in_from();
                self.finish();
                if self.peek_kw(Kw::WITH) && self.nth_kw(1) == Some(Kw::ORDINALITY) {
                    self.bump();
                    self.bump();
                }
            }
            Some((parts, tail)) => {
                let missing = matches!(tail, Tail::Missing | Tail::Star);
                self.chain(SyntaxKind::TableRef, parts, missing, false);
            }
            None => {
                self.note(Expected::Name(NameKind::Table));
                self.missing();
            }
        }
        self.source_suffixes();
    }

    fn call_suffix_in_from(&mut self) {
        // The argument list of a table function; no window suffixes here.
        self.start(SyntaxKind::ArgList);
        self.bump();
        self.parens += 1;
        if self.nth_kind(0) != Some(TokenKind::RParen) {
            loop {
                if self.query_starts(0) {
                    self.query();
                } else if matches!(
                    self.nth_kind(0),
                    Some(TokenKind::Word | TokenKind::QuotedIdent)
                ) && (self.nth_op(1, "=>") || self.nth_op(1, ":="))
                {
                    self.start(SyntaxKind::NamedArg);
                    self.bump();
                    self.bump();
                    self.expr();
                    self.finish();
                } else {
                    self.expr();
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.rest_of_group();
        self.parens -= 1;
        self.expect(TokenKind::RParen);
        self.finish();
    }

    /// Alias, `PIVOT`/`UNPIVOT` and the clauses the parser keeps opaque
    /// (`TABLESAMPLE`, `SAMPLE`, `PARTITION (..)`, `AS OF ..`, `INDEXED BY`).
    fn source_suffixes(&mut self) {
        self.opaque_suffixes();
        self.opt_alias(true);
        if self.g.pivot {
            if self.peek_kw(Kw::PIVOT) {
                self.pivot();
                self.opt_alias(false);
            } else if self.peek_kw(Kw::UNPIVOT) {
                self.start(SyntaxKind::UnpivotClause);
                self.bump();
                if self.nth_word(0, "include") || self.nth_word(0, "exclude") {
                    self.bump();
                    self.expect_kw(Kw::NULLS);
                }
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.start(SyntaxKind::Opaque);
                    self.bump_group();
                    self.finish();
                } else {
                    self.at(TokenKind::LParen);
                    self.missing();
                }
                self.finish();
                self.opt_alias(false);
            }
        }
        self.opaque_suffixes();
    }

    pub(super) fn opaque_suffixes(&mut self) {
        use Kw::*;
        loop {
            let paren_next = self.nth_kind(1) == Some(TokenKind::LParen);
            let opaque = match self.nth_kw(0) {
                Some(TABLESAMPLE | MATCH_RECOGNIZE) => true,
                Some(SAMPLE) => paren_next || self.nth_word(1, "block"),
                Some(PARTITION) => paren_next || self.nth_kw(1) == Some(FOR),
                Some(AS) => {
                    self.nth_kw(1) == Some(OF)
                        && (self.nth_word(2, "timestamp") || self.nth_word(2, "scn"))
                }
                Some(INDEXED) => self.nth_kw(1) == Some(BY),
                Some(NOT) => self.nth_kw(1) == Some(INDEXED),
                _ => self.nth_word(0, "versions") && self.nth_kw(1) == Some(BETWEEN),
            };
            if !opaque {
                break;
            }
            self.start(SyntaxKind::Opaque);
            match self.nth_kw(0) {
                Some(TABLESAMPLE) => {
                    // TABLESAMPLE method (args) [REPEATABLE (seed)]
                    self.bump();
                    if self.nth_kind(0) == Some(TokenKind::Word) {
                        self.bump();
                    }
                    self.bump_paren();
                    if self.nth_word(0, "repeatable") {
                        self.bump();
                        self.bump_paren();
                    }
                }
                Some(SAMPLE) => {
                    // SAMPLE [BLOCK] (percent [, size]) [SEED (seed)]
                    self.bump();
                    if self.nth_word(0, "block") {
                        self.bump();
                    }
                    self.bump_paren();
                    if self.nth_word(0, "seed") {
                        self.bump();
                        self.bump_paren();
                    }
                }
                Some(PARTITION) => {
                    // PARTITION (name) | PARTITION FOR (keys)
                    self.bump();
                    if self.peek_kw(FOR) {
                        self.bump();
                    }
                    self.bump_paren();
                }
                Some(AS) => {
                    // AS OF TIMESTAMP|SCN expr
                    self.bump();
                    self.bump();
                    self.bump();
                    self.opaque_operand();
                }
                Some(INDEXED) => {
                    // INDEXED BY name
                    self.bump();
                    self.bump();
                    if self.is_name_at(0) {
                        self.bump();
                    }
                }
                Some(NOT) => {
                    // NOT INDEXED
                    self.bump();
                    self.bump();
                }
                Some(MATCH_RECOGNIZE) => {
                    self.bump();
                    self.bump_paren();
                }
                _ => {
                    // VERSIONS BETWEEN {SCN|TIMESTAMP} a AND b
                    self.bump();
                    self.bump();
                    if self.nth_word(0, "scn") || self.nth_word(0, "timestamp") {
                        self.bump();
                    }
                    self.opaque_operand();
                    if self.peek_kw(AND) {
                        self.bump();
                        self.opaque_operand();
                    }
                }
            }
            self.finish();
        }
    }

    /// A parenthesised group, if one is next.
    fn bump_paren(&mut self) {
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.bump_group();
        }
    }

    /// An operand inside an opaque clause: a group or a token, with more
    /// joined by operators (`systimestamp - interval '1' day`).
    fn opaque_operand(&mut self) {
        loop {
            if self.at_recovery() || self.nth_kind(0) == Some(TokenKind::Comma) {
                return;
            }
            self.bump_group();
            // `INTERVAL '1' DAY`: a literal with its unit.
            while matches!(self.nth_kind(0), Some(TokenKind::String))
                || ["day", "hour", "minute", "second", "month", "year"]
                    .iter()
                    .any(|u| self.nth_word(0, u))
            {
                self.bump();
            }
            if self.nth_kind(0) != Some(TokenKind::Operator) {
                return;
            }
            self.bump();
        }
    }

    /// `PIVOT [XML] (agg [AS a], .. FOR col IN (value [AS a], ..))`.
    fn pivot(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::PivotClause);
        self.bump();
        if self.nth_word(0, "xml") {
            self.bump();
        }
        self.parened(|p| {
            loop {
                p.start(SyntaxKind::SelectItem);
                p.expr();
                p.opt_alias(false);
                p.finish();
                if !p.eat(TokenKind::Comma) {
                    break;
                }
            }
            p.expect_kw(FOR);
            if p.nth_kind(0) == Some(TokenKind::LParen) {
                p.tuple();
            } else {
                p.expr_bp(super::expr::BP_LIKE + 1);
            }
            p.expect_kw(IN);
            p.parened(|p| {
                if p.query_starts(0) {
                    p.query();
                } else if p.at_kw(ANY) {
                    p.bump();
                    while p.eat(TokenKind::Comma) {
                        p.at_kw(ANY);
                        p.bump();
                    }
                } else {
                    loop {
                        p.start(SyntaxKind::SelectItem);
                        p.expr();
                        p.opt_alias(false);
                        p.finish();
                        if !p.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                }
            });
        });
        self.finish();
    }

    /// A `GROUP BY` item: an expression, `()`, `GROUPING SETS (..)`,
    /// `ROLLUP (..)` or `CUBE (..)`.
    fn group_item(&mut self) {
        if !self.enter() {
            return;
        }
        let grouping_sets = self.peek_kw(Kw::GROUPING) && self.nth_kw(1) == Some(Kw::SETS);
        let rollup = (self.nth_word(0, "rollup") || self.nth_word(0, "cube"))
            && self.nth_kind(1) == Some(TokenKind::LParen);
        if grouping_sets || rollup {
            self.start(SyntaxKind::GroupingSets);
            self.bump();
            if grouping_sets {
                self.bump();
            }
            self.parened(|p| {
                if p.nth_kind(0) != Some(TokenKind::RParen) {
                    loop {
                        p.group_item();
                        if !p.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                }
            });
            self.finish();
        } else {
            self.expr();
        }
        self.leave();
    }

    pub(super) fn order_by(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::OrderByClause);
        self.bump();
        if self.g.connect_by {
            self.eat_kw(SIBLINGS);
        }
        self.expect_kw(BY);
        loop {
            self.start(SyntaxKind::OrderItem);
            self.expr();
            if !self.eat_kw(ASC) && !self.eat_kw(DESC) && self.at_kw(USING) {
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::Operator) {
                    self.bump();
                } else {
                    self.missing();
                }
            }
            if self.eat_kw(NULLS) {
                self.expect_any_kw(&[FIRST, LAST]);
            }
            self.finish();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    /// `ORDER BY`, `LIMIT`, `OFFSET`, `FETCH` and locking clauses of a query.
    pub(super) fn query_tail(&mut self) {
        use Kw::*;
        let g = self.g;
        if self.at_kw(ORDER) {
            self.order_by();
        }
        for _ in 0..3 {
            if g.limit && self.at_kw(LIMIT) {
                self.start(SyntaxKind::LimitClause);
                self.bump();
                if !self.eat_kw(ALL) {
                    self.expr();
                }
                if self.eat(TokenKind::Comma) {
                    self.expr();
                }
                self.finish();
            } else if (g.limit || g.fetch_first) && self.at_kw(OFFSET) {
                self.start(SyntaxKind::OffsetClause);
                self.bump();
                self.expr();
                if !self.eat_kw(ROWS) {
                    self.eat_kw(ROW);
                }
                self.finish();
            } else if g.fetch_first && self.at_kw(FETCH) {
                self.start(SyntaxKind::FetchClause);
                self.bump();
                self.expect_any_kw(&[FIRST, NEXT]);
                if !self.peek_kw(ROW) && !self.peek_kw(ROWS) {
                    self.expr();
                }
                self.eat_kw(PERCENT);
                self.expect_any_kw(&[ROWS, ROW]);
                if !self.eat_kw(ONLY) {
                    if self.at_kw(WITH) {
                        self.bump();
                        self.expect_kw(TIES);
                    } else {
                        self.missing();
                    }
                }
                self.finish();
            } else {
                break;
            }
        }
        while self.at_kw(FOR) && matches!(self.nth_kw(1), Some(UPDATE | SHARE | NO | KEY)) {
            self.start(SyntaxKind::LockingClause);
            self.bump();
            while !self.at_recovery() && !self.peek_kw(FOR) {
                self.bump_group();
            }
            self.finish();
        }
    }
}
