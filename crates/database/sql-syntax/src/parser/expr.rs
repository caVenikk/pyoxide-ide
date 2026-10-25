// Expressions: a Pratt parser. Binding powers follow PostgreSQL's operator
// precedence (low to high): OR, AND, NOT, IS, comparison, BETWEEN/IN/LIKE,
// other operators, + -, * / %, ^, AT TIME ZONE/COLLATE, unary, postfix
// (`::`, subscripts, `(+)`, field access).
use super::{Kw, Parser, Tail};
use crate::dialect::Dialect;
use crate::lexer::TokenKind;
use crate::tree::{Expected, NameKind, SyntaxKind};

pub(super) const BP_OR: u8 = 1;
pub(super) const BP_AND: u8 = 2;
pub(super) const BP_NOT: u8 = 3;
pub(super) const BP_IS: u8 = 4;
pub(super) const BP_CMP: u8 = 5;
pub(super) const BP_LIKE: u8 = 6;
pub(super) const BP_OTHER: u8 = 7;
pub(super) const BP_ADD: u8 = 8;
pub(super) const BP_MUL: u8 = 9;
pub(super) const BP_EXP: u8 = 10;
pub(super) const BP_AT: u8 = 11;
pub(super) const BP_UNARY: u8 = 12;
pub(super) const BP_POSTFIX: u8 = 13;

#[derive(Debug, Clone, Copy)]
enum Infix {
    Binary(u8),
    Is,
    NullPostfix,
    In,
    Between,
    Like,
    Collate,
    AtTimeZone,
    Cast,
    Subscript,
    OuterJoin,
    Field,
    /// `cursor%FOUND`.
    Attribute,
}

/// Functions whose arguments have keywords of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Special {
    None,
    Extract,
    Trim,
    Position,
    Substring,
    Overlay,
}

const PREFIX_OPS: [&str; 7] = ["-", "+", "~", "@", "|/", "||/", "!!"];

impl Parser<'_> {
    pub(super) fn expr(&mut self) {
        self.expr_bp(0);
    }

    /// Expressions separated by commas.
    pub(super) fn expr_list(&mut self) {
        loop {
            self.expr();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
    }

    pub(super) fn expr_bp(&mut self, min: u8) {
        if !self.enter() {
            return;
        }
        let cp = self.checkpoint();
        self.prefix();
        while let Some((op, lbp)) = self.infix() {
            if lbp < min {
                break;
            }
            self.infix_rest(op, cp);
        }
        self.leave();
    }

    /// The `n`th token is a PL/SQL attribute name (`FOUND`, `ROWCOUNT`, ..).
    fn attribute_at(&self, n: usize) -> bool {
        [
            "found",
            "notfound",
            "isopen",
            "rowcount",
            "bulk_rowcount",
            "bulk_exceptions",
        ]
        .iter()
        .any(|w| self.nth_word(n, w))
    }

    /// `:name` of a PL/SQL trigger (`:new`, `:old`) or a bind at `n`.
    fn colon_name_at(&self, n: usize) -> bool {
        self.g.plsql
            && self.nth_op(n, ":")
            && self.nth_kind(n + 1) == Some(TokenKind::Word)
            && self
                .nth(n)
                .zip(self.nth(n + 1))
                .is_some_and(|(a, b)| a.end == b.start)
    }

    fn infix(&mut self) -> Option<(Infix, u8)> {
        use Kw::*;
        let g = self.g;
        let kind = self.nth_kind(0);
        match kind {
            Some(TokenKind::Operator) => {
                let op = self.nth_text(0);
                let bp = match op {
                    "%" if g.plsql && self.attribute_at(1) => {
                        return Some((Infix::Attribute, BP_POSTFIX));
                    }
                    "::" if g.double_colon_cast => return Some((Infix::Cast, BP_POSTFIX)),
                    "=" | "<>" | "!=" | "<" | ">" | "<=" | ">=" | "==" => BP_CMP,
                    "+" | "-" => BP_ADD,
                    "*" | "/" | "%" => BP_MUL,
                    "^" => BP_EXP,
                    "=>" | ":=" | ":" => return None,
                    _ => BP_OTHER,
                };
                return Some((Infix::Binary(bp), bp));
            }
            Some(TokenKind::Parameter)
                if matches!(self.dialect, Dialect::Postgres | Dialect::Generic)
                    && self.nth_text(0) == "?" =>
            {
                return Some((Infix::Binary(BP_OTHER), BP_OTHER));
            }
            Some(TokenKind::LBracket) if g.arrays => {
                return Some((Infix::Subscript, BP_POSTFIX));
            }
            Some(TokenKind::LParen) if self.outer_join_mark_at(0) => {
                return Some((Infix::OuterJoin, BP_POSTFIX));
            }
            Some(TokenKind::Dot) if self.range_dots_at(0) => return None,
            Some(TokenKind::Dot) => return Some((Infix::Field, BP_POSTFIX)),
            _ => {}
        }
        // Keyword operators: in the expected set for completion only, a
        // syntax error does not list every operator that could continue.
        if self.at_op_kw(OR) {
            return Some((Infix::Binary(BP_OR), BP_OR));
        }
        if self.at_op_kw(AND) {
            return Some((Infix::Binary(BP_AND), BP_AND));
        }
        if self.at_op_kw(IS) {
            return Some((Infix::Is, BP_IS));
        }
        if self.at_op_kw(NOT) {
            return match self.nth_kw(1) {
                Some(IN) => Some((Infix::In, BP_LIKE)),
                Some(BETWEEN) => Some((Infix::Between, BP_LIKE)),
                Some(LIKE | SIMILAR) => Some((Infix::Like, BP_LIKE)),
                Some(ILIKE) if g.ilike => Some((Infix::Like, BP_LIKE)),
                Some(GLOB | REGEXP | MATCH) if g.sqlite_operators => Some((Infix::Like, BP_LIKE)),
                Some(NULL) if g.null_postfix && !self.stop_not_null => {
                    Some((Infix::NullPostfix, BP_IS))
                }
                _ => None,
            };
        }
        if self.at_op_kw(IN) {
            return Some((Infix::In, BP_LIKE));
        }
        if self.at_op_kw(BETWEEN) {
            return Some((Infix::Between, BP_LIKE));
        }
        if self.at_op_kw(LIKE) || (g.ilike && self.at_op_kw(ILIKE)) {
            return Some((Infix::Like, BP_LIKE));
        }
        if self.peek_kw(SIMILAR) && self.nth_kw(1) == Some(TO) {
            return Some((Infix::Like, BP_LIKE));
        }
        if g.sqlite_operators && matches!(self.nth_kw(0), Some(GLOB | REGEXP | MATCH)) {
            return Some((Infix::Like, BP_LIKE));
        }
        if g.null_postfix && matches!(self.nth_kw(0), Some(ISNULL | NOTNULL)) {
            return Some((Infix::NullPostfix, BP_IS));
        }
        if self.peek_kw(COLLATE) {
            return Some((Infix::Collate, BP_AT));
        }
        if self.peek_kw(AT) && (self.nth_kw(1) == Some(TIME) || self.nth_kw(1) == Some(LOCAL)) {
            return Some((Infix::AtTimeZone, BP_AT));
        }
        None
    }

    fn infix_rest(&mut self, op: Infix, cp: usize) {
        use Kw::*;
        let kind = match op {
            Infix::Binary(bp) => {
                self.bump();
                self.expr_bp(bp + 1);
                SyntaxKind::BinaryExpr
            }
            Infix::Is => {
                self.bump();
                self.eat_kw(NOT);
                if self.at_kw(NULL) || self.at_kw(TRUE) || self.at_kw(FALSE) || self.at_kw(UNKNOWN)
                {
                    self.bump();
                } else if self.at_kw(DISTINCT) {
                    self.bump();
                    self.expect_kw(FROM);
                    self.expr_bp(BP_IS + 1);
                } else if ["document", "normalized", "json"]
                    .iter()
                    .any(|w| self.nth_word(0, w))
                {
                    self.bump();
                } else {
                    self.expr_bp(BP_IS + 1);
                }
                SyntaxKind::IsExpr
            }
            Infix::NullPostfix => {
                if self.peek_kw(NOT) {
                    self.bump();
                }
                self.bump();
                SyntaxKind::IsExpr
            }
            Infix::In => {
                self.eat_kw(NOT);
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    if self.query_starts(1) {
                        self.subquery(SyntaxKind::SubqueryExpr);
                    } else {
                        self.tuple();
                    }
                } else if let Some((parts, tail)) = self.chain_ahead() {
                    // `x IN table` (SQLite).
                    self.chain(SyntaxKind::TableRef, parts, tail == Tail::Missing, false);
                } else {
                    self.at(TokenKind::LParen);
                    self.missing_or_error();
                }
                SyntaxKind::InExpr
            }
            Infix::Between => {
                self.eat_kw(NOT);
                self.bump();
                if !self.eat_kw(SYMMETRIC) {
                    self.eat_kw(ASYMMETRIC);
                }
                self.expr_bp(BP_LIKE + 1);
                self.expect_kw(AND);
                self.expr_bp(BP_LIKE + 1);
                SyntaxKind::BetweenExpr
            }
            Infix::Like => {
                self.eat_kw(NOT);
                let similar = self.peek_kw(SIMILAR);
                self.bump();
                if similar {
                    self.expect_kw(TO);
                }
                self.expr_bp(BP_LIKE + 1);
                if self.eat_kw(ESCAPE) {
                    self.expr_bp(BP_LIKE + 1);
                }
                SyntaxKind::LikeExpr
            }
            Infix::Collate => {
                self.bump();
                if matches!(
                    self.nth_kind(0),
                    Some(TokenKind::Word | TokenKind::QuotedIdent | TokenKind::String)
                ) {
                    self.bump();
                } else {
                    self.missing();
                }
                SyntaxKind::CollateExpr
            }
            Infix::AtTimeZone => {
                self.bump();
                if !self.eat_kw(LOCAL) {
                    self.expect_kw(TIME);
                    self.expect_kw(ZONE);
                    self.expr_bp(BP_AT + 1);
                }
                SyntaxKind::AtTimeZone
            }
            Infix::Cast => {
                self.bump();
                self.type_ref();
                SyntaxKind::CastExpr
            }
            Infix::Subscript => {
                self.bump();
                if !self.nth_op(0, ":") && self.nth_kind(0) != Some(TokenKind::RBracket) {
                    self.expr();
                }
                if self.nth_op(0, ":") {
                    self.bump();
                    if self.nth_kind(0) != Some(TokenKind::RBracket) {
                        self.expr();
                    }
                }
                self.expect(TokenKind::RBracket);
                SyntaxKind::SubscriptExpr
            }
            Infix::OuterJoin => {
                self.bump();
                self.bump();
                self.bump();
                SyntaxKind::OuterJoinMark
            }
            Infix::Attribute => {
                self.bump();
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.bump_group();
                }
                SyntaxKind::AttributeExpr
            }
            Infix::Field => {
                self.bump();
                if self.is_name_at(0) {
                    self.name_part();
                } else if !self.at_op("*") {
                    self.note(Expected::Name(NameKind::Column));
                    self.missing();
                } else {
                    self.bump();
                }
                SyntaxKind::FieldExpr
            }
        };
        self.start_at(cp, kind);
        self.finish();
    }

    fn prefix(&mut self) {
        let cp = self.checkpoint();
        let g = self.g;
        if self.at_op_kw(Kw::NOT) {
            self.bump();
            self.expr_bp(BP_NOT);
        } else if (self.nth_kind(0) == Some(TokenKind::Operator)
            && PREFIX_OPS.contains(&self.nth_text(0)))
            || (g.connect_by
                && matches!(self.nth_kw(0), Some(Kw::PRIOR | Kw::CONNECT_BY_ROOT))
                && self.expr_start_at(1))
        {
            self.bump();
            self.expr_bp(BP_UNARY);
        } else {
            self.primary();
            return;
        }
        self.start_at(cp, SyntaxKind::UnaryExpr);
        self.finish();
    }

    /// Something that can start an expression is at `n` (no expected set).
    fn expr_start_at(&self, n: usize) -> bool {
        use Kw::*;
        let g = self.g;
        match self.nth_kind(n) {
            Some(
                TokenKind::Number
                | TokenKind::String
                | TokenKind::Parameter
                | TokenKind::LParen
                | TokenKind::QuotedIdent
                | TokenKind::Hole,
            ) => true,
            Some(TokenKind::Operator) => {
                let op = self.nth_text(n);
                op == "*" || PREFIX_OPS.contains(&op) || self.colon_name_at(n)
            }
            Some(TokenKind::Word) => {
                let paren = self.nth_kind(n + 1) == Some(TokenKind::LParen);
                match self.nth_kw(n) {
                    Some(NULL | TRUE | FALSE | CASE | NOT) => true,
                    Some(EXISTS) => paren,
                    Some(ARRAY) => {
                        g.arrays
                            && matches!(
                                self.nth_kind(n + 1),
                                Some(TokenKind::LBracket | TokenKind::LParen)
                            )
                    }
                    Some(LEFT | RIGHT | ALL | ANY | SOME) => paren,
                    Some(PRIOR | CONNECT_BY_ROOT) if g.connect_by => true,
                    _ => !self.is_hard(n),
                }
            }
            _ => false,
        }
    }

    /// Something that can start an expression is next (no expected set).
    pub(super) fn expr_start_here(&self) -> bool {
        self.expr_start_at(0)
    }

    fn at_expr_start(&mut self) -> bool {
        self.note(Expected::Expression);
        self.expr_start_at(0)
    }

    /// No expression where one is needed: a `Missing` node before anything
    /// that can follow an expression, else the stray token as an error.
    fn expr_missing(&mut self) {
        let follows = self.at_recovery()
            || self.is_hard(0)
            || matches!(
                self.nth_kind(0),
                Some(
                    TokenKind::Comma
                        | TokenKind::Operator
                        | TokenKind::RParen
                        | TokenKind::RBracket
                        | TokenKind::Dot
                )
            );
        if follows {
            self.missing();
        } else if self.nth_kind(0) == Some(TokenKind::Hole) {
            self.hole();
        } else {
            let set = self.snapshot();
            self.start(SyntaxKind::Error);
            self.bump_group();
            self.b.finish(Some(set));
        }
    }

    fn primary(&mut self) {
        if !self.at_expr_start() {
            self.expr_missing();
            return;
        }
        match self.nth_kind(0) {
            Some(TokenKind::Number | TokenKind::String) => {
                self.start(SyntaxKind::Literal);
                self.bump();
                self.finish();
            }
            Some(TokenKind::Parameter) => {
                self.start(SyntaxKind::Param);
                self.bump();
                self.finish();
            }
            Some(TokenKind::LParen) => self.paren_expr(),
            Some(TokenKind::Operator) if self.colon_name_at(0) => {
                self.start(SyntaxKind::Param);
                self.bump();
                self.bump();
                self.finish();
            }
            Some(TokenKind::Operator) => {
                // `*` where an expression stands.
                self.start(SyntaxKind::Star);
                self.bump();
                self.finish();
            }
            _ => self.word_primary(),
        }
    }

    fn word_primary(&mut self) {
        use Kw::*;
        let paren = self.nth_kind(1) == Some(TokenKind::LParen);
        match self.nth_kw(0) {
            Some(NULL | TRUE | FALSE | DEFAULT) if !paren => {
                self.start(SyntaxKind::Literal);
                self.bump();
                self.finish();
            }
            Some(CASE) => self.case_expr(),
            Some(CAST) if paren => self.cast_expr(),
            Some(EXISTS) if paren => {
                self.start(SyntaxKind::ExistsExpr);
                self.bump();
                self.subquery(SyntaxKind::SubqueryExpr);
                self.finish();
            }
            Some(ARRAY) if self.g.arrays => {
                self.start(SyntaxKind::ArrayExpr);
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LBracket) {
                    self.bracket_items();
                } else {
                    self.parened(|p| p.query());
                }
                self.finish();
            }
            Some(ROW) if paren => {
                self.start(SyntaxKind::TupleExpr);
                self.bump();
                self.parened(|p| {
                    if p.nth_kind(0) != Some(TokenKind::RParen) {
                        p.expr_list();
                    }
                });
                self.finish();
            }
            Some(INTERVAL) if self.nth_kind(1) == Some(TokenKind::String) => {
                self.start(SyntaxKind::Literal);
                self.bump();
                self.bump();
                self.interval_fields();
                self.finish();
            }
            _ if self.nth_word(0, "try_cast") && paren => self.cast_expr(),
            _ => self.name_expr(),
        }
    }

    /// A column, a qualified star, a call, a typed literal or a hole.
    fn name_expr(&mut self) {
        let Some((parts, tail)) = self.chain_ahead() else {
            // `LEFT(..)`, `ANY(..)`: a hard keyword used as a function.
            if self.nth_kind(0) == Some(TokenKind::Word)
                && self.nth_kind(1) == Some(TokenKind::LParen)
            {
                self.start(SyntaxKind::CallExpr);
                self.start(SyntaxKind::FunctionRef);
                self.bump();
                self.finish();
                self.call_rest(Special::None);
                self.finish();
            } else {
                self.expr_missing();
            }
            return;
        };
        match tail {
            Tail::Call => {
                let special = if parts == 1 {
                    match self.nth_kw(0) {
                        Some(Kw::EXTRACT) => Special::Extract,
                        Some(Kw::TRIM) => Special::Trim,
                        Some(Kw::POSITION) => Special::Position,
                        Some(Kw::SUBSTRING) => Special::Substring,
                        Some(Kw::OVERLAY) => Special::Overlay,
                        _ => Special::None,
                    }
                } else {
                    Special::None
                };
                self.start(SyntaxKind::CallExpr);
                self.chain(SyntaxKind::FunctionRef, parts, false, false);
                self.call_rest(special);
                self.finish();
            }
            Tail::Star => {
                self.start(SyntaxKind::Star);
                self.chain(SyntaxKind::TableRef, parts, false, false);
                self.bump(); // `.`
                self.bump(); // `*`
                self.finish();
            }
            Tail::Missing => self.chain(SyntaxKind::ColumnRef, parts, true, false),
            Tail::Name => {
                if parts == 1 && self.nth_kind(0) == Some(TokenKind::Hole) {
                    self.hole();
                } else if parts == 1
                    && self.nth_kind(0) == Some(TokenKind::Word)
                    && self.nth_kind(1) == Some(TokenKind::String)
                {
                    // `DATE '2024-01-01'`, `TIMESTAMP '..'`.
                    self.start(SyntaxKind::Literal);
                    self.bump();
                    self.bump();
                    self.finish();
                } else {
                    self.chain(SyntaxKind::ColumnRef, parts, false, false);
                }
            }
        }
    }

    fn paren_expr(&mut self) {
        if self.query_starts(1) {
            self.subquery(SyntaxKind::SubqueryExpr);
            return;
        }
        let cp = self.checkpoint();
        self.bump();
        self.parens += 1;
        let kind = if self.at(TokenKind::RParen) {
            SyntaxKind::TupleExpr
        } else {
            self.expr();
            if self.nth_kind(0) == Some(TokenKind::Comma) {
                while self.eat(TokenKind::Comma) {
                    self.expr();
                }
                SyntaxKind::TupleExpr
            } else {
                SyntaxKind::ParenExpr
            }
        };
        self.parens -= 1;
        self.expect(TokenKind::RParen);
        self.start_at(cp, kind);
        self.finish();
    }

    /// `( query )` as a node of `kind`.
    pub(super) fn subquery(&mut self, kind: SyntaxKind) {
        self.start(kind);
        self.parened(|p| p.query());
        self.finish();
    }

    /// `( expr, .. )` as a `TupleExpr`.
    pub(super) fn tuple(&mut self) {
        self.start(SyntaxKind::TupleExpr);
        self.parened(|p| {
            if p.nth_kind(0) != Some(TokenKind::RParen) {
                p.expr_list();
            }
        });
        self.finish();
    }

    fn bracket_items(&mut self) {
        self.bump(); // `[`
        if self.nth_kind(0) != Some(TokenKind::RBracket) {
            loop {
                if self.nth_kind(0) == Some(TokenKind::LBracket) {
                    if self.enter() {
                        self.start(SyntaxKind::ArrayExpr);
                        self.bracket_items();
                        self.finish();
                        self.leave();
                    }
                } else {
                    self.expr();
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBracket);
    }

    fn case_expr(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::CaseExpr);
        self.bump();
        if !self.at_kw(WHEN) {
            self.expr();
        }
        let mut whens = 0;
        while self.at_kw(WHEN) {
            whens += 1;
            self.start(SyntaxKind::WhenClause);
            self.bump();
            self.expr();
            self.expect_kw(THEN);
            self.expr();
            self.finish();
        }
        if whens == 0 {
            self.missing();
        }
        if self.at_kw(ELSE) {
            self.start(SyntaxKind::ElseClause);
            self.bump();
            self.expr();
            self.finish();
        }
        self.expect_kw(END);
        self.finish();
    }

    fn cast_expr(&mut self) {
        self.start(SyntaxKind::CastExpr);
        self.bump();
        self.parened(|p| {
            p.expr();
            p.expect_kw(Kw::AS);
            p.type_ref();
        });
        self.finish();
    }

    /// `YEAR [(p)] [TO MONTH [(p)]]` after `INTERVAL '..'`.
    fn interval_fields(&mut self) {
        const UNITS: [&str; 6] = ["year", "month", "day", "hour", "minute", "second"];
        if UNITS.iter().any(|u| self.nth_word(0, u)) {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.bump_group();
            }
            if self.peek_kw(Kw::TO) && UNITS.iter().any(|u| self.nth_word(1, u)) {
                self.bump();
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.bump_group();
                }
            }
        }
    }

    /// A type name: `int`, `varchar(10)`, `double precision`,
    /// `timestamp(3) with time zone`, `numeric[]`, `s.my_type`.
    pub(super) fn type_ref(&mut self) {
        use Kw::*;
        let Some((parts, tail)) = self.chain_ahead() else {
            self.note(Expected::Name(NameKind::Type));
            self.missing_or_error();
            return;
        };
        let missing = tail == Tail::Missing;
        self.chain(SyntaxKind::TypeRef, parts, missing, true);
        if !missing
            && self.g.plsql
            && self.nth_op(0, "%")
            && (self.nth_word(1, "type") || self.nth_word(1, "rowtype"))
        {
            self.bump();
            self.bump();
        } else if !missing {
            let base = self.prev_text().to_ascii_lowercase();
            match base.as_str() {
                "double" if self.nth_word(0, "precision") => self.bump(),
                "character" | "char" | "nchar" | "bit" if self.nth_word(0, "varying") => {
                    self.bump()
                }
                "national" if self.nth_word(0, "character") || self.nth_word(0, "char") => {
                    self.bump();
                    if self.nth_word(0, "varying") {
                        self.bump();
                    }
                }
                "long" if self.nth_word(0, "raw") => self.bump(),
                _ => {}
            }
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.bump_group();
            }
            match base.as_str() {
                "timestamp" | "time"
                    if matches!(self.nth_kw(0), Some(WITH | WITHOUT))
                        && matches!(self.nth_kw(1), Some(TIME | LOCAL)) =>
                {
                    self.bump();
                    self.eat_kw(LOCAL);
                    self.expect_kw(TIME);
                    self.expect_kw(ZONE);
                }
                "interval" => self.interval_fields(),
                _ => {}
            }
            while self.nth_kind(0) == Some(TokenKind::LBracket) {
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::Number) {
                    self.bump();
                }
                self.expect(TokenKind::RBracket);
            }
        }
        self.finish();
    }

    /// The argument list of a call and what may follow it: `WITHIN GROUP`,
    /// `FILTER`, `RESPECT|IGNORE NULLS`, `KEEP (..)`, `OVER`.
    fn call_rest(&mut self, special: Special) {
        use Kw::*;
        self.arg_list(special);
        if self.g.within_group && self.peek_kw(WITHIN) && self.nth_kw(1) == Some(GROUP) {
            self.start(SyntaxKind::WithinGroup);
            self.bump();
            self.bump();
            self.parened(|p| {
                if p.at_kw(ORDER) {
                    p.order_by();
                } else {
                    p.missing();
                }
            });
            self.finish();
        }
        if self.peek_kw(KEEP) && self.nth_kind(1) == Some(TokenKind::LParen) {
            self.start(SyntaxKind::Opaque);
            self.bump();
            self.bump_group();
            self.finish();
        }
        if self.g.filter_clause
            && self.peek_kw(FILTER)
            && self.nth_kind(1) == Some(TokenKind::LParen)
        {
            self.start(SyntaxKind::FilterClause);
            self.bump();
            self.parened(|p| {
                p.expect_kw(WHERE);
                p.expr();
            });
            self.finish();
        }
        if matches!(self.nth_kw(0), Some(RESPECT | IGNORE)) && self.nth_kw(1) == Some(NULLS) {
            self.bump();
            self.bump();
        }
        if self.at_kw(OVER) {
            self.start(SyntaxKind::OverClause);
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.window_spec();
            } else if self.is_name_at(0) {
                self.start(SyntaxKind::WindowRef);
                self.name_part();
                self.finish();
            } else {
                self.note(Expected::Name(NameKind::Window));
                self.at(TokenKind::LParen);
                self.missing();
            }
            self.finish();
        }
    }

    fn arg_list(&mut self, special: Special) {
        use Kw::*;
        self.start(SyntaxKind::ArgList);
        self.bump(); // `(`
        self.parens += 1;
        match special {
            Special::Extract => {
                if matches!(
                    self.nth_kind(0),
                    Some(TokenKind::Word | TokenKind::String | TokenKind::QuotedIdent)
                ) && self.nth_kw(1) == Some(FROM)
                {
                    self.bump();
                    self.bump();
                    self.expr();
                } else {
                    self.general_args();
                }
            }
            Special::Trim => {
                if matches!(self.nth_kw(0), Some(BOTH | LEADING | TRAILING)) {
                    self.bump();
                }
                if !self.peek_kw(FROM) && self.nth_kind(0) != Some(TokenKind::RParen) {
                    self.expr();
                }
                if self.eat_kw(FROM) {
                    self.expr();
                }
                while self.eat(TokenKind::Comma) {
                    self.expr();
                }
            }
            Special::Position => {
                self.expr_bp(BP_LIKE + 1);
                if self.eat_kw(IN) {
                    self.expr_bp(BP_LIKE + 1);
                }
                while self.eat(TokenKind::Comma) {
                    self.expr();
                }
            }
            Special::Substring => {
                self.expr();
                if self.eat_kw(FROM) {
                    self.expr();
                    if self.eat_kw(FOR) {
                        self.expr();
                    }
                } else if self.eat_kw(FOR) {
                    self.expr();
                    if self.eat_kw(FROM) {
                        self.expr();
                    }
                }
                while self.eat(TokenKind::Comma) {
                    self.expr();
                }
            }
            Special::Overlay => {
                self.expr();
                if self.eat_kw(PLACING) {
                    self.expr();
                    self.expect_kw(FROM);
                    self.expr();
                    if self.eat_kw(FOR) {
                        self.expr();
                    }
                } else {
                    while self.eat(TokenKind::Comma) {
                        self.expr();
                    }
                }
            }
            Special::None => self.general_args(),
        }
        self.rest_of_group();
        self.parens -= 1;
        self.expect(TokenKind::RParen);
        self.finish();
    }

    fn general_args(&mut self) {
        use Kw::*;
        if self.at(TokenKind::RParen) {
            return;
        }
        if self.at_op("*") {
            self.bump();
            return;
        }
        if !self.eat_kw(DISTINCT) {
            self.eat_kw(ALL);
        }
        loop {
            self.arg();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        if self.at_kw(ORDER) {
            self.order_by();
        }
    }

    fn arg(&mut self) {
        if matches!(
            self.nth_kind(0),
            Some(TokenKind::Word | TokenKind::QuotedIdent)
        ) && (self.nth_op(1, "=>") || self.nth_op(1, ":="))
        {
            self.start(SyntaxKind::NamedArg);
            self.bump();
            self.bump();
            self.expr();
            self.finish();
        } else if self.query_starts(0) {
            self.query();
        } else {
            self.expr();
        }
    }

    /// `( [name] [PARTITION BY ..] [ORDER BY ..] [frame] )`.
    pub(super) fn window_spec(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::WindowSpec);
        self.parened(|p| {
            if p.is_name_at(0)
                && !matches!(p.nth_kw(0), Some(PARTITION | ORDER | ROWS | RANGE | GROUPS))
            {
                p.start(SyntaxKind::WindowRef);
                p.name_part();
                p.finish();
            }
            if p.at_kw(PARTITION) {
                p.start(SyntaxKind::PartitionBy);
                p.bump();
                p.expect_kw(BY);
                p.expr_list();
                p.finish();
            }
            if p.at_kw(ORDER) {
                p.order_by();
            }
            if p.at_kw(ROWS) || p.at_kw(RANGE) || p.at_kw(GROUPS) {
                p.frame();
            }
        });
        self.finish();
    }

    fn frame(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::FrameClause);
        self.bump();
        if self.eat_kw(BETWEEN) {
            self.frame_bound();
            self.expect_kw(AND);
            self.frame_bound();
        } else {
            self.frame_bound();
        }
        if self.eat_kw(EXCLUDE) {
            if self.eat_kw(CURRENT) {
                self.expect_kw(ROW);
            } else if self.eat_kw(NO) {
                self.expect_kw(OTHERS);
            } else {
                self.expect_any_kw(&[GROUP, TIES]);
            }
        }
        self.finish();
    }

    fn frame_bound(&mut self) {
        use Kw::*;
        if self.eat_kw(UNBOUNDED) {
            self.expect_any_kw(&[PRECEDING, FOLLOWING]);
        } else if self.eat_kw(CURRENT) {
            self.expect_kw(ROW);
        } else {
            self.expr_bp(BP_AND + 1);
            self.expect_any_kw(&[PRECEDING, FOLLOWING]);
        }
    }
}
