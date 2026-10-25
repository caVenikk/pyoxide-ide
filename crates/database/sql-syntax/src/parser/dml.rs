// Data modification: INSERT (also Oracle's multi-table form), UPDATE, DELETE
// and MERGE, with RETURNING, ON CONFLICT and the SET clause.
use super::{Kw, Parser, Proc, Tail};
use crate::lexer::TokenKind;
use crate::tree::{Expected, NameKind, SyntaxKind};

impl Parser<'_> {
    /// A data-modifying statement begins at the current token.
    pub(super) fn dml_at(&self) -> bool {
        match self.nth_kw(0) {
            Some(Kw::INSERT | Kw::UPDATE | Kw::DELETE) => true,
            Some(Kw::MERGE) => self.g.merge,
            Some(Kw::REPLACE) => self.g.conflict_resolution && self.nth_kw(1) == Some(Kw::INTO),
            _ => false,
        }
    }

    /// An `INSERT`, `UPDATE`, `DELETE` or `MERGE`; `with` is the checkpoint of
    /// a `WITH` clause before it, which becomes its first child.
    pub(super) fn dml(&mut self, with: Option<usize>) {
        if !self.enter() {
            return;
        }
        let cp = with.unwrap_or_else(|| self.checkpoint());
        let kind = match self.nth_kw(0) {
            Some(Kw::INSERT | Kw::REPLACE) => {
                self.insert_body();
                SyntaxKind::Insert
            }
            Some(Kw::UPDATE) => {
                self.update_body();
                SyntaxKind::Update
            }
            Some(Kw::DELETE) => {
                self.delete_body();
                SyntaxKind::Delete
            }
            _ => {
                self.merge_body();
                SyntaxKind::Merge
            }
        };
        self.start_at(cp, kind);
        self.finish();
        self.leave();
    }

    fn insert_body(&mut self) {
        use Kw::*;
        self.bump();
        if self.g.multi_insert && (self.peek_kw(ALL) || self.peek_kw(FIRST)) {
            self.multi_insert();
            return;
        }
        if self.g.conflict_resolution && self.at_kw(OR) {
            self.conflict_resolution();
        }
        self.expect_kw(INTO);
        self.dml_target();
        self.insert_source();
        while self.g.on_conflict && self.peek_kw(ON) && self.nth_kw(1) == Some(CONFLICT) {
            self.on_conflict();
        }
        self.dml_tail();
    }

    /// `[(columns)] VALUES (..) | query | DEFAULT VALUES`.
    fn insert_source(&mut self) {
        use Kw::*;
        if self.nth_kind(0) == Some(TokenKind::LParen) && !self.paren_query_ahead() {
            self.column_list();
        }
        if self.nth_word(0, "overriding") {
            self.start(SyntaxKind::Opaque);
            while !self.at_recovery() && !self.peek_kw(VALUES) && !self.peek_kw(SELECT) {
                self.bump();
            }
            self.finish();
        }
        if self.at_kw(VALUES) {
            self.insert_values();
        } else if self.at_kw(DEFAULT) {
            self.bump();
            self.expect_kw(VALUES);
        } else if self.query_starts(0)
            || self.nth_kind(0) == Some(TokenKind::LParen)
            || (self.g.table_statement && self.peek_kw(TABLE))
        {
            self.query();
        } else {
            self.at_kw(SELECT);
            self.missing();
        }
    }

    /// `VALUES (..), ..`; in PL/SQL also `VALUES record`.
    fn insert_values(&mut self) {
        if self.nth_kind(1) != Some(TokenKind::LParen) && self.g.plsql && self.nth_kind(1).is_some()
        {
            self.start(SyntaxKind::Values);
            self.bump();
            self.expr();
            self.finish();
        } else {
            self.values();
        }
    }

    /// The `(` at the current token starts a query (`(SELECT ..)`), possibly
    /// behind more parentheses.
    fn paren_query_ahead(&self) -> bool {
        let mut k = 0;
        while self.nth_kind(k) == Some(TokenKind::LParen) {
            k += 1;
        }
        self.query_starts(k)
    }

    /// `ON CONFLICT [target] DO NOTHING | DO UPDATE SET ..`.
    fn on_conflict(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::OnConflictClause);
        self.bump();
        self.bump();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.start(SyntaxKind::ConflictTarget);
            self.parened(|p| {
                loop {
                    p.index_element();
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
                p.rest_of_group();
            });
            if self.peek_kw(WHERE) {
                self.where_clause();
            }
            self.finish();
        } else if self.peek_kw(ON) && self.nth_word(1, "constraint") {
            self.start(SyntaxKind::ConflictTarget);
            self.bump();
            self.bump();
            self.object_name();
            self.finish();
        }
        self.expect_kw(DO);
        if self.at_kw(NOTHING) {
            self.bump();
        } else if self.at_kw(UPDATE) {
            self.bump();
            self.set_clause();
            if self.peek_kw(WHERE) {
                self.where_clause();
            }
        } else {
            self.missing();
        }
        self.finish();
    }

    /// An indexed column or expression with its operator class and order.
    pub(super) fn index_element(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::IndexElement);
        self.expr();
        if self.is_name_at(0) && !matches!(self.nth_kw(0), Some(ASC | DESC | NULLS)) {
            // An operator class.
            self.name_part();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.bump_group();
            }
        }
        if !self.eat_kw(ASC) {
            self.eat_kw(DESC);
        }
        if self.eat_kw(NULLS) {
            self.expect_any_kw(&[FIRST, LAST]);
        }
        self.finish();
    }

    /// SQLite's `OR REPLACE | IGNORE | ABORT | FAIL | ROLLBACK`.
    fn conflict_resolution(&mut self) {
        self.start(SyntaxKind::ConflictResolution);
        self.bump();
        if self.nth_kind(0) == Some(TokenKind::Word) && !self.at_recovery() {
            self.bump();
        } else {
            self.missing();
        }
        self.finish();
    }

    /// Oracle: `INSERT ALL|FIRST [WHEN c THEN] INTO t .. [ELSE INTO ..] query`.
    fn multi_insert(&mut self) {
        use Kw::*;
        self.bump();
        loop {
            if self.at_kw(WHEN) {
                self.start(SyntaxKind::WhenClause);
                self.bump();
                self.expr();
                self.expect_kw(THEN);
                while self.peek_kw(INTO) {
                    self.insert_into();
                }
                self.finish();
            } else if self.at_kw(ELSE) {
                self.start(SyntaxKind::ElseClause);
                self.bump();
                while self.peek_kw(INTO) {
                    self.insert_into();
                }
                self.finish();
            } else if self.at_kw(INTO) {
                self.insert_into();
            } else {
                break;
            }
        }
        if self.query_starts(0) || self.nth_kind(0) == Some(TokenKind::LParen) {
            self.query();
        } else {
            self.at_kw(SELECT);
            self.missing();
        }
    }

    fn insert_into(&mut self) {
        self.start(SyntaxKind::InsertInto);
        self.bump();
        self.dml_target();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_list();
        }
        if self.peek_kw(Kw::VALUES) {
            self.insert_values();
        }
        self.finish();
    }

    fn update_body(&mut self) {
        use Kw::*;
        self.bump();
        if self.g.conflict_resolution && self.at_kw(OR) {
            self.conflict_resolution();
        }
        self.dml_target();
        if self.at_kw(SET) {
            self.set_clause();
        } else {
            self.missing();
        }
        if self.g.update_from && self.at_kw(FROM) {
            self.source_clause();
        }
        self.dml_tail();
    }

    fn delete_body(&mut self) {
        use Kw::*;
        self.bump();
        if !self.eat_kw(FROM) && !self.g.plsql {
            self.missing();
        }
        self.dml_target();
        if self.g.delete_using && self.at_kw(USING) {
            self.start(SyntaxKind::UsingSource);
            self.bump();
            loop {
                self.source_item();
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            self.finish();
        }
        self.dml_tail();
    }

    /// `WHERE`, `RETURNING`, and SQLite's `ORDER BY` / `LIMIT`.
    fn dml_tail(&mut self) {
        use Kw::*;
        if self.at_kw(WHERE) {
            self.where_clause();
        }
        if self.g.returning && (self.at_kw(RETURNING) || (self.g.plsql && self.at_kw(RETURN))) {
            self.returning();
        }
        if self.g.limit && (self.peek_kw(ORDER) || self.peek_kw(LIMIT) || self.peek_kw(OFFSET)) {
            self.query_tail();
        }
        if self.g.plsql && self.nth_word(0, "log") && self.nth_word(1, "errors") {
            self.start(SyntaxKind::Opaque);
            while !self.eof() && !self.at_terminator() {
                self.bump_group();
            }
            self.finish();
        }
    }

    fn merge_body(&mut self) {
        use Kw::*;
        self.bump();
        self.expect_kw(INTO);
        self.dml_target();
        self.start(SyntaxKind::UsingSource);
        if self.at_kw(USING) {
            self.bump();
            self.table_primary();
        } else {
            self.missing();
        }
        self.finish();
        if self.at_kw(ON) {
            self.start(SyntaxKind::OnClause);
            self.bump();
            self.expr();
            self.finish();
        } else {
            self.missing();
        }
        let mut clauses = 0;
        while self.at_kw(WHEN) {
            clauses += 1;
            self.merge_clause();
        }
        if clauses == 0 {
            self.missing();
        }
        if self.g.returning && self.at_kw(RETURNING) {
            self.returning();
        }
    }

    fn merge_clause(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::MergeClause);
        self.bump();
        self.eat_kw(NOT);
        self.expect_kw(MATCHED);
        if self.peek_kw(BY) && (self.nth_word(1, "source") || self.nth_word(1, "target")) {
            self.bump();
            self.bump();
        }
        if self.at_kw(AND) {
            self.bump();
            self.expr();
        }
        self.expect_kw(THEN);
        if self.at_kw(UPDATE) {
            self.start(SyntaxKind::MergeUpdate);
            self.bump();
            self.set_clause();
            if self.peek_kw(WHERE) {
                self.where_clause();
            }
            if self.peek_kw(DELETE) && self.nth_kw(1) == Some(WHERE) {
                self.bump();
                self.where_clause();
            }
            self.finish();
        } else if self.at_kw(DELETE) {
            self.start(SyntaxKind::MergeDelete);
            self.bump();
            self.finish();
        } else if self.at_kw(INSERT) {
            self.start(SyntaxKind::MergeInsert);
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.column_list();
            }
            if self.nth_word(0, "overriding") {
                self.start(SyntaxKind::Opaque);
                while !self.at_recovery() && !self.peek_kw(VALUES) && !self.peek_kw(DEFAULT) {
                    self.bump();
                }
                self.finish();
            }
            if self.at_kw(VALUES) {
                self.values();
            } else if self.eat_kw(DEFAULT) {
                self.expect_kw(VALUES);
            } else {
                self.missing();
            }
            if self.peek_kw(WHERE) {
                self.where_clause();
            }
            self.finish();
        } else if self.at_kw(DO) {
            self.bump();
            self.expect_kw(NOTHING);
        } else {
            self.missing();
        }
        self.finish();
    }

    /// The table a statement changes, or a subquery: `[ONLY] name [*] [alias]`.
    pub(super) fn dml_target(&mut self) {
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.table_primary();
            return;
        }
        if self.chain_ahead().is_none() && !self.peek_kw(Kw::ONLY) {
            self.start(SyntaxKind::TableSource);
            self.note(Expected::Name(NameKind::Table));
            self.missing_or_error();
            self.finish();
            return;
        }
        self.start(SyntaxKind::TableSource);
        if self.g.table_statement && self.peek_kw(Kw::ONLY) && self.is_name_at(1) {
            self.bump();
        }
        self.table_name();
        if self.nth_op(0, "*") {
            self.bump();
        }
        self.opaque_suffixes();
        self.opt_alias(false);
        self.opaque_suffixes();
        self.finish();
    }

    /// `SET target = value, ..`.
    pub(super) fn set_clause(&mut self) {
        self.start(SyntaxKind::SetClause);
        if self.at_kw(Kw::SET) {
            self.bump();
        } else {
            self.missing();
        }
        loop {
            self.assignment();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    fn assignment(&mut self) {
        self.start(SyntaxKind::Assignment);
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_list();
        } else {
            self.column_target();
        }
        // `a[1] = ..`
        self.bump_brackets();
        if self.at_op("=") {
            self.bump();
            if self.at_kw(Kw::DEFAULT) {
                self.bump();
            } else {
                self.expr();
            }
        } else {
            self.missing();
        }
        self.finish();
    }

    /// A column name of a `SET` clause, possibly qualified.
    fn column_target(&mut self) {
        match self.chain_ahead() {
            Some((parts, tail)) if tail != Tail::Call => {
                let missing = matches!(tail, Tail::Missing | Tail::Star);
                self.chain(SyntaxKind::ColumnRef, parts, missing, false);
            }
            _ => {
                self.note(Expected::Name(NameKind::Column));
                self.recover_item();
            }
        }
    }

    /// A `Missing` node where the list item would start, an error region
    /// otherwise (up to the next `,` or recovery point).
    pub(super) fn recover_item(&mut self) {
        if self.at_recovery() || self.nth_kind(0) == Some(TokenKind::Comma) {
            self.missing();
        } else {
            self.error_until(true, None);
        }
    }

    /// `(a, b)`: column names.
    pub(super) fn column_list(&mut self) {
        self.start(SyntaxKind::ColumnList);
        self.parened(|p| {
            if p.nth_kind(0) != Some(TokenKind::RParen) {
                loop {
                    p.column_name_ref();
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
            } else {
                p.note(Expected::Name(NameKind::Column));
                p.missing();
            }
            p.rest_of_group();
        });
        self.finish();
    }

    pub(super) fn column_name_ref(&mut self) {
        match self.chain_ahead() {
            Some((parts, tail)) if tail != Tail::Call => {
                let missing = matches!(tail, Tail::Missing | Tail::Star);
                self.chain(SyntaxKind::ColumnRef, parts, missing, false);
            }
            _ => {
                self.note(Expected::Name(NameKind::Column));
                self.recover_item();
            }
        }
    }

    /// `WHERE condition` or `WHERE CURRENT OF cursor`.
    pub(super) fn where_clause(&mut self) {
        self.start(SyntaxKind::WhereClause);
        self.bump();
        if self.peek_kw(Kw::CURRENT) && self.nth_kw(1) == Some(Kw::OF) {
            self.start(SyntaxKind::CurrentOf);
            self.bump();
            self.bump();
            self.object_name();
            self.finish();
        } else {
            self.expr();
        }
        self.finish();
    }

    /// `RETURNING items [INTO targets]`.
    fn returning(&mut self) {
        self.start(SyntaxKind::ReturningClause);
        self.bump();
        loop {
            self.select_item();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        if (self.g.plsql || self.in_procedure()) && self.at_into() {
            self.into_clause();
        }
        self.finish();
    }

    /// `[BULK COLLECT] INTO a, b` where the targets are variables.
    pub(super) fn into_clause(&mut self) {
        self.start(SyntaxKind::IntoClause);
        if self.peek_kw(Kw::BULK) {
            self.bump();
            self.bump();
        }
        self.expect_kw(Kw::INTO);
        if self.nth_word(0, "strict") {
            self.bump();
        }
        loop {
            if self.expr_start_here() {
                self.expr_bp(super::expr::BP_POSTFIX);
            } else {
                self.note(Expected::Name(NameKind::Column));
                self.recover_item();
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        self.finish();
    }

    /// A named object (cursor, index, trigger, ...) as an `ObjectRef`.
    pub(super) fn object_name(&mut self) {
        match self.chain_ahead() {
            Some((parts, tail)) if tail != Tail::Call => {
                let missing = matches!(tail, Tail::Missing | Tail::Star);
                self.chain(SyntaxKind::ObjectRef, parts, missing, false);
            }
            _ => {
                self.note(Expected::Name(NameKind::Object));
                self.recover_item();
            }
        }
    }

    pub(super) fn in_procedure(&self) -> bool {
        self.proc != Proc::No
    }
}
