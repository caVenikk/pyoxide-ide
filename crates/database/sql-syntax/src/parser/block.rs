// Procedural code: PL/SQL blocks and PL/pgSQL bodies (declarations, control
// flow, cursors, exception handlers), and SQLite trigger bodies, which are
// blocks of plain statements. A construct that is not known is kept as an
// `Opaque` statement; a missing `END IF` or `;` is a `Missing` node at the
// place where the block would have to go on, so an error stays in its block.
use super::{Kw, Parser, Proc, Tail};
use crate::dialect::Dialect;
use crate::lexer::TokenKind;
use crate::tree::{Expected, NameKind, SyntaxKind};

/// Where a block starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockStart {
    /// `[<<label>>] [DECLARE declarations] BEGIN`.
    Full,
    /// The declarations after `IS` / `AS` of a routine.
    Body,
}

impl Parser<'_> {
    /// The procedural code a block at the current position is written in.
    fn default_proc(&self) -> Proc {
        if self.dialect == Dialect::Postgres {
            Proc::PlPgSql
        } else {
            Proc::PlSql
        }
    }

    /// A block starts at the current token: a label, `DECLARE` or `BEGIN`.
    pub(super) fn block_at(&self) -> bool {
        self.peek_kw(Kw::DECLARE) || self.peek_kw(Kw::BEGIN) || self.nth_op(0, "<<")
    }

    /// A top-level unit of procedural code: a block or a statement.
    pub(super) fn proc_unit_statement(&mut self) {
        if self.block_at() {
            self.block(BlockStart::Full);
        } else {
            self.proc_statement();
        }
    }

    pub(super) fn block(&mut self, start: BlockStart) {
        use Kw::*;
        if !self.enter() {
            return;
        }
        let saved = self.proc;
        if saved == Proc::No {
            self.proc = self.default_proc();
        }
        self.start(SyntaxKind::Block);
        if start == BlockStart::Full {
            if self.nth_op(0, "<<") {
                self.label();
            }
            if self.at_kw(DECLARE) {
                self.bump();
                self.declare_section();
            }
        } else {
            self.declare_section();
        }
        if self.at_kw(BEGIN) {
            self.bump();
            if self.nth_word(0, "atomic") {
                self.bump();
            }
        } else {
            self.missing();
        }
        self.statement_list();
        if self.peek_kw(EXCEPTION) {
            self.exception_section();
        }
        if self.at_kw(END) {
            self.bump();
            self.end_name();
        } else {
            self.missing();
        }
        self.finish();
        self.proc = saved;
        self.leave();
    }

    /// `<<name>>`.
    fn label(&mut self) {
        self.start(SyntaxKind::Label);
        self.bump();
        self.label_name();
        if self.nth_op(0, ">>") {
            self.bump();
        } else {
            self.note(Expected::Punct(">>"));
            self.missing();
        }
        self.finish();
    }

    /// The name of a label: any word, also one that is a keyword elsewhere
    /// (`outer`, `inner`).
    fn label_name(&mut self) {
        self.start(SyntaxKind::DefName);
        if matches!(
            self.nth_kind(0),
            Some(TokenKind::Word | TokenKind::QuotedIdent)
        ) {
            self.bump();
        } else {
            self.note(Expected::Name(NameKind::Object));
            self.missing();
        }
        self.finish();
    }

    /// A reference to a label after `EXIT` or `CONTINUE`.
    fn label_ref(&mut self) {
        self.start(SyntaxKind::ObjectRef);
        self.bump();
        self.finish();
    }

    /// The name after the `END` of a block or routine.
    pub(super) fn end_name(&mut self) {
        let word = self.nth_kind(0) == Some(TokenKind::Word)
            && !matches!(
                self.nth_kw(0),
                Some(Kw::IF | Kw::LOOP | Kw::CASE | Kw::WHEN | Kw::ELSE)
            );
        if word || self.nth_kind(0) == Some(TokenKind::QuotedIdent) {
            self.bump();
        }
    }

    /// The `;` that ends a statement. A statement with something left over
    /// before it takes the rest as an error; a `;` that is not there is
    /// missing.
    pub(super) fn end_statement(&mut self) {
        if self.at(TokenKind::Semicolon) {
            self.bump();
        } else if self.at_stmt_boundary() {
            self.missing();
        } else {
            self.error_until(false, None);
            self.eat(TokenKind::Semicolon);
        }
    }

    /// Where a list of statements ends.
    fn at_stmt_boundary(&self) -> bool {
        use Kw::*;
        self.eof()
            || self.nth_kind(0) == Some(TokenKind::SlashLine)
            || matches!(
                self.nth_kw(0),
                Some(END | ELSE | ELSIF | ELSEIF | WHEN | EXCEPTION)
            )
    }

    pub(super) fn statement_list(&mut self) {
        self.start(SyntaxKind::StatementList);
        loop {
            if self.at_stmt_boundary() {
                break;
            }
            if self.at_terminator() {
                // An empty statement.
                self.bump();
                continue;
            }
            self.proc_statement();
        }
        self.finish();
    }

    // --- Declarations.

    /// Declarations up to `BEGIN` or `END`.
    pub(super) fn declare_section(&mut self) {
        use Kw::*;
        if self.peek_kw(BEGIN) || self.peek_kw(END) || self.eof() {
            return;
        }
        self.start(SyntaxKind::DeclareSection);
        loop {
            if self.eof()
                || self.peek_kw(BEGIN)
                || self.peek_kw(END)
                || self.nth_kind(0) == Some(TokenKind::SlashLine)
            {
                break;
            }
            let before = self.pos;
            self.declaration();
            if self.pos == before {
                self.error_until(false, None);
            }
        }
        self.finish();
    }

    fn declaration(&mut self) {
        use Kw::*;
        if self.at_terminator() {
            self.bump();
            return;
        }
        match self.nth_kw(0) {
            Some(CURSOR) => {
                self.cursor_decl();
                self.end_statement();
            }
            Some(TYPE) => {
                self.type_decl();
                self.end_statement();
            }
            Some(PRAGMA) => {
                self.start(SyntaxKind::Pragma);
                self.bump();
                self.opaque_to_semicolon();
                self.finish();
                self.end_statement();
            }
            Some(PROCEDURE | FUNCTION) => {
                self.subprogram_decl();
                self.end_statement();
            }
            _ if self.nth_word(0, "subtype") => {
                self.type_decl();
                self.end_statement();
            }
            _ if self.is_name_at(0) && self.nth_kw(1) == Some(EXCEPTION) => {
                self.start(SyntaxKind::ExceptionDecl);
                self.def_name(NameKind::Object);
                self.bump();
                self.finish();
                self.end_statement();
            }
            _ if self.is_name_at(0) => {
                self.var_decl();
                self.end_statement();
            }
            _ => {
                self.note(Expected::Name(NameKind::Column));
                self.error_until(false, None);
                self.eat(TokenKind::Semicolon);
            }
        }
    }

    /// Everything up to the `;` that ends the statement, as one `Opaque` node.
    fn opaque_to_semicolon(&mut self) {
        if self.eof() || self.at_terminator() {
            return;
        }
        self.start(SyntaxKind::Opaque);
        while !self.eof() && !self.at_terminator() {
            self.bump_group();
        }
        self.finish();
    }

    fn var_decl(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::VarDecl);
        self.def_name(NameKind::Column);
        if self.nth_word(0, "alias") {
            self.opaque_to_semicolon();
        } else if self.peek_kw(CURSOR) || self.nth_word(0, "scroll") || self.peek_kw(NO) {
            self.cursor_rest();
        } else {
            self.eat_kw(CONSTANT);
            if self.is_name_at(0) {
                self.type_ref();
            } else {
                self.note(Expected::Name(NameKind::Type));
                self.missing();
            }
            if self.nth_word(0, "collate") || self.peek_kw(COLLATE) {
                self.bump();
                self.bump();
            }
            if self.peek_kw(NOT) && self.nth_kw(1) == Some(NULL) {
                self.bump();
                self.bump();
            }
            if self.nth_op(0, ":=") || self.nth_op(0, "=") || self.peek_kw(DEFAULT) {
                self.bump();
                self.expr();
            }
            self.opaque_to_semicolon();
        }
        self.finish();
    }

    /// `CURSOR name [(params)] [RETURN type] IS|FOR query`.
    fn cursor_decl(&mut self) {
        self.start(SyntaxKind::CursorDecl);
        self.bump();
        self.def_name(NameKind::Object);
        self.cursor_rest();
        self.finish();
    }

    /// What follows the name of a cursor.
    fn cursor_rest(&mut self) {
        use Kw::*;
        while self.nth_word(0, "scroll") || self.peek_kw(NO) || self.peek_kw(CURSOR) {
            self.bump();
        }
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.param_list();
        }
        if self.peek_kw(RETURN) {
            self.returns_clause();
        }
        if self.peek_kw(IS) || self.peek_kw(FOR) {
            self.bump();
            if self.query_starts(0) || self.nth_kind(0) == Some(TokenKind::LParen) {
                self.query();
            } else {
                self.at_kw(SELECT);
                self.missing();
            }
        }
    }

    /// `TYPE name IS ..`, `SUBTYPE name IS ..`: the rest is not parsed.
    fn type_decl(&mut self) {
        self.start(SyntaxKind::TypeDecl);
        self.bump();
        self.def_name(NameKind::Type);
        self.opaque_to_semicolon();
        self.finish();
    }

    /// A procedure or function declared or defined in a block or package.
    pub(super) fn subprogram_decl(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::SubprogramDecl);
        self.bump();
        self.chain_of(SyntaxKind::FunctionRef);
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.param_list();
        }
        let mut returned = false;
        if self.peek_kw(RETURN) || self.peek_kw(RETURNS) {
            self.returns_clause();
            returned = true;
        }
        self.routine_options(returned);
        if self.peek_kw(IS) || self.peek_kw(AS) {
            if self.nth_word(1, "language") || self.nth_word(1, "external") {
                self.opaque_to_semicolon();
            } else {
                self.bump();
                self.block(BlockStart::Body);
            }
        }
        self.finish();
    }

    // --- Statements.

    pub(super) fn proc_statement(&mut self) {
        let before = self.pos;
        if self.enter() {
            self.proc_statement_inner();
            self.leave();
        } else {
            self.opaque_to_semicolon();
            self.eat(TokenKind::Semicolon);
        }
        if self.pos == before && !self.eof() {
            self.error_until(false, None);
        }
    }

    fn proc_statement_inner(&mut self) {
        use Kw::*;
        if self.nth_op(0, "<<") {
            self.label();
            return;
        }
        if self.nth_kind(0) == Some(TokenKind::Hole) {
            self.hole();
            return;
        }
        match self.nth_kw(0) {
            Some(DECLARE | BEGIN) => {
                self.block(BlockStart::Full);
                self.end_statement();
            }
            Some(IF) => {
                self.if_stmt();
                self.end_statement();
            }
            Some(CASE) => {
                self.case_stmt();
                self.end_statement();
            }
            Some(LOOP) => {
                self.start(SyntaxKind::LoopStmt);
                self.bump();
                self.statement_list();
                self.end_of(LOOP, "END LOOP");
                self.finish();
                self.end_statement();
            }
            Some(WHILE) => {
                self.start(SyntaxKind::WhileStmt);
                self.bump();
                self.expr();
                self.loop_body();
                self.finish();
                self.end_statement();
            }
            Some(FOR) => {
                self.for_stmt();
                self.end_statement();
            }
            Some(FOREACH) => {
                self.foreach_stmt();
                self.end_statement();
            }
            Some(FORALL) => {
                self.forall_stmt();
                self.end_statement();
            }
            Some(EXIT | CONTINUE) => {
                self.start(SyntaxKind::ExitStmt);
                self.bump();
                if matches!(
                    self.nth_kind(0),
                    Some(TokenKind::Word | TokenKind::QuotedIdent)
                ) && !self.peek_kw(WHEN)
                {
                    self.label_ref();
                }
                if self.peek_kw(WHEN) {
                    self.bump();
                    self.expr();
                }
                self.finish();
                self.end_statement();
            }
            Some(RETURN) => {
                self.return_stmt();
                self.end_statement();
            }
            Some(NULL) if self.nth_kind(1) == Some(TokenKind::Semicolon) => {
                self.start(SyntaxKind::NullStmt);
                self.bump();
                self.finish();
                self.end_statement();
            }
            Some(RAISE) => {
                self.raise_stmt();
                self.end_statement();
            }
            Some(OPEN) => {
                self.open_stmt();
                self.end_statement();
            }
            Some(FETCH) => {
                self.fetch_stmt();
                self.end_statement();
            }
            Some(CLOSE) => {
                self.start(SyntaxKind::CloseStmt);
                self.bump();
                self.object_name();
                self.finish();
                self.end_statement();
            }
            Some(EXECUTE) => {
                self.execute_stmt();
                self.end_statement();
            }
            Some(PERFORM) => {
                let cp = self.checkpoint();
                self.select();
                self.query_tail();
                self.start_at(cp, SyntaxKind::Query);
                self.finish();
                self.end_statement();
            }
            Some(SELECT | VALUES | WITH) => {
                self.query_statement();
                self.end_statement();
            }
            Some(GOTO) => {
                self.opaque_to_semicolon();
                self.end_statement();
            }
            _ if self.dml_at() => {
                self.dml(None);
                self.end_statement();
            }
            _ if self.ddl_at() && !self.peek_kw(DO) => {
                self.ddl_statement();
                self.end_statement();
            }
            _ if self.nth_word(0, "call") && self.is_name_at(1) => {
                self.start(SyntaxKind::CallStmt);
                self.bump();
                self.expr_bp(super::expr::BP_POSTFIX);
                self.opaque_to_semicolon();
                self.finish();
                self.end_statement();
            }
            _ if self.assignment_ahead() => {
                self.assign_stmt();
                self.end_statement();
            }
            _ if self.is_name_at(0) && !self.known_statement_word() => {
                self.start(SyntaxKind::CallStmt);
                self.expr_bp(super::expr::BP_POSTFIX);
                self.opaque_to_semicolon();
                self.finish();
                self.end_statement();
            }
            _ => {
                // COMMIT, ROLLBACK, GET DIAGNOSTICS, ASSERT, PIPE ROW, ...
                self.opaque_to_semicolon();
                self.end_statement();
            }
        }
    }

    /// A word that starts a statement the parser keeps opaque.
    fn known_statement_word(&self) -> bool {
        const WORDS: [&str; 12] = [
            "commit",
            "rollback",
            "savepoint",
            "get",
            "assert",
            "pipe",
            "lock",
            "set",
            "reset",
            "notify",
            "move",
            "release",
        ];
        WORDS.iter().any(|w| self.nth_word(0, w))
    }

    /// `target := value` (or `target = value` in PL/pgSQL) is next: a name
    /// with subscripts, fields and call arguments, then the operator.
    fn assignment_ahead(&self) -> bool {
        let mut i = self.pos;
        let mut first = true;
        // Two names in a row are not a target (`GET DIAGNOSTICS x = ..`).
        let mut after_name = false;
        while i < self.sig.len() {
            let t = &self.toks[self.sig[i] as usize];
            let name_like = matches!(t.kind, TokenKind::Word | TokenKind::QuotedIdent);
            if name_like && after_name {
                return false;
            }
            after_name = name_like;
            match t.kind {
                TokenKind::Operator => {
                    let text = &self.text[t.start..t.end];
                    if text == ":=" || (text == "=" && self.proc == Proc::PlPgSql) {
                        return !first;
                    }
                    if text != ":" && text != "%" {
                        return false;
                    }
                    i += 1;
                }
                TokenKind::Word
                | TokenKind::QuotedIdent
                | TokenKind::Parameter
                | TokenKind::Dot => {
                    i += 1;
                }
                TokenKind::LParen => match self.close[i] {
                    super::NO_CLOSE => return false,
                    c => i = c as usize + 1,
                },
                TokenKind::LBracket => {
                    let mut depth = 0u32;
                    while i < self.sig.len() {
                        match self.toks[self.sig[i] as usize].kind {
                            TokenKind::LBracket => depth += 1,
                            TokenKind::RBracket => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            TokenKind::Semicolon | TokenKind::SlashLine => return false,
                            _ => {}
                        }
                        i += 1;
                    }
                    i += 1;
                }
                _ => return false,
            }
            first = false;
        }
        false
    }

    fn assign_stmt(&mut self) {
        self.start(SyntaxKind::AssignStmt);
        self.expr_bp(super::expr::BP_POSTFIX);
        if self.nth_op(0, ":=") || self.nth_op(0, "=") {
            self.bump();
            self.expr();
        } else {
            self.note(Expected::Punct(":="));
            self.missing();
        }
        self.finish();
    }

    /// `END <kw> [label]`, or a `Missing` node when the construct is not
    /// closed (the `END` then belongs to a block around it).
    fn end_of(&mut self, kw: Kw, text: &'static str) {
        if self.peek_kw(Kw::END) && self.nth_kw(1) == Some(kw) {
            self.note(Expected::Keyword(text));
            self.bump();
            self.bump();
            if kw == Kw::LOOP {
                self.end_name();
            }
        } else {
            self.note(Expected::Keyword(text));
            self.missing();
        }
    }

    fn if_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::IfStmt);
        self.bump();
        self.expr();
        self.expect_kw(THEN);
        self.statement_list();
        while self.peek_kw(ELSIF) || self.peek_kw(ELSEIF) {
            self.start(SyntaxKind::ElsifClause);
            self.bump();
            self.expr();
            self.expect_kw(THEN);
            self.statement_list();
            self.finish();
        }
        if self.peek_kw(ELSE) {
            self.start(SyntaxKind::ElseClause);
            self.bump();
            self.statement_list();
            self.finish();
        }
        self.end_of(IF, "END IF");
        self.finish();
    }

    fn case_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::CaseStmt);
        self.bump();
        if !self.peek_kw(WHEN) {
            self.expr();
        }
        let mut whens = 0;
        while self.peek_kw(WHEN) {
            whens += 1;
            self.start(SyntaxKind::WhenClause);
            self.bump();
            self.expr_list();
            self.expect_kw(THEN);
            self.statement_list();
            self.finish();
        }
        if whens == 0 {
            self.at_kw(WHEN);
            self.missing();
        }
        if self.peek_kw(ELSE) {
            self.start(SyntaxKind::ElseClause);
            self.bump();
            self.statement_list();
            self.finish();
        }
        self.end_of(CASE, "END CASE");
        self.finish();
    }

    /// `LOOP statements END LOOP [label]`.
    fn loop_body(&mut self) {
        self.expect_kw(Kw::LOOP);
        self.statement_list();
        self.end_of(Kw::LOOP, "END LOOP");
    }

    fn for_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::ForStmt);
        self.bump();
        self.def_name(NameKind::Column);
        while self.eat(TokenKind::Comma) {
            self.def_name(NameKind::Column);
        }
        self.expect_kw(IN);
        self.eat_kw(REVERSE);
        if self.query_starts(0) {
            self.query();
        } else if self.peek_kw(EXECUTE) {
            self.execute_stmt();
        } else {
            self.expr();
            if self.range_dots_at(0) {
                self.bump();
                self.bump();
                self.expr();
            }
            if self.nth_word(0, "by") || self.peek_kw(BY) {
                self.bump();
                self.expr();
            }
        }
        self.loop_body();
        self.finish();
    }

    /// PL/pgSQL: `FOREACH x [SLICE n] IN ARRAY expr LOOP .. END LOOP`.
    fn foreach_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::ForStmt);
        self.bump();
        self.def_name(NameKind::Column);
        if self.nth_word(0, "slice") {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::Number) {
                self.bump();
            }
        }
        self.expect_kw(IN);
        if self.peek_kw(ARRAY) {
            self.bump();
        }
        self.expr();
        self.loop_body();
        self.finish();
    }

    /// Oracle: `FORALL i IN a..b [SAVE EXCEPTIONS] dml`.
    fn forall_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::ForallStmt);
        self.bump();
        self.def_name(NameKind::Column);
        self.expect_kw(IN);
        if self.nth_word(0, "indices") || self.nth_word(0, "values") || self.peek_kw(VALUES) {
            self.bump();
            self.expect_kw(OF);
            self.expr();
        } else {
            self.expr();
            if self.range_dots_at(0) {
                self.bump();
                self.bump();
                self.expr();
            }
        }
        if self.nth_word(0, "save") {
            self.bump();
            self.bump();
        }
        if self.dml_at() {
            self.dml(None);
        } else {
            self.missing();
        }
        self.finish();
    }

    fn return_stmt(&mut self) {
        self.start(SyntaxKind::ReturnStmt);
        self.bump();
        if self.nth_word(0, "next") && self.nth_kind(1) != Some(TokenKind::Semicolon) {
            self.bump();
            self.expr();
        } else if self.nth_word(0, "query") {
            self.bump();
            if self.peek_kw(Kw::EXECUTE) {
                self.execute_stmt();
            } else if self.query_starts(0) {
                self.query();
            } else {
                self.at_kw(Kw::SELECT);
                self.missing();
            }
        } else if !self.at_terminator() && !self.at_stmt_boundary() {
            self.expr();
        } else if self.probe && self.eof() {
            self.note(Expected::Expression);
        }
        self.finish();
    }

    fn raise_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::RaiseStmt);
        self.bump();
        const LEVELS: [&str; 7] = [
            "exception",
            "notice",
            "warning",
            "info",
            "log",
            "debug",
            "error",
        ];
        if self.peek_kw(EXCEPTION) || LEVELS.iter().any(|w| self.nth_word(0, w)) {
            self.bump();
        }
        if self.nth_kind(0) == Some(TokenKind::String) {
            self.expr();
            while self.eat(TokenKind::Comma) {
                self.expr();
            }
        } else if self.nth_word(0, "sqlstate") {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::String) {
                self.bump();
            }
        } else if self.is_name_at(0) && !self.nth_word(0, "using") {
            self.object_name();
        }
        if self.nth_word(0, "using") || self.peek_kw(USING) {
            self.bump();
            loop {
                if self.nth_kind(0) == Some(TokenKind::Word) {
                    self.bump();
                } else {
                    self.missing();
                    break;
                }
                if self.nth_op(0, "=") {
                    self.bump();
                    self.expr();
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.finish();
    }

    fn open_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::OpenStmt);
        self.bump();
        self.object_name();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.tuple();
        }
        while self.nth_word(0, "scroll") || self.peek_kw(NO) {
            self.bump();
        }
        if self.peek_kw(FOR) {
            self.bump();
            if self.query_starts(0) {
                self.query();
            } else if self.peek_kw(EXECUTE) {
                self.bump();
                self.expr();
            } else {
                self.expr();
            }
            if self.peek_kw(USING) {
                self.using_args();
            }
        }
        self.finish();
    }

    /// `USING [IN|OUT|IN OUT] expr, ..`.
    fn using_args(&mut self) {
        self.bump();
        loop {
            while self.peek_kw(Kw::IN) || self.nth_word(0, "out") {
                self.bump();
            }
            self.expr();
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
    }

    fn fetch_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::FetchStmt);
        self.bump();
        const DIRECTIONS: [&str; 8] = [
            "next", "prior", "first", "last", "absolute", "relative", "forward", "backward",
        ];
        if (DIRECTIONS.iter().any(|w| self.nth_word(0, w))
            || self.peek_kw(NEXT)
            || self.peek_kw(FIRST)
            || self.peek_kw(LAST))
            && !self.at_into()
        {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::Number) {
                self.bump();
            }
            if self.peek_kw(FROM) || self.peek_kw(IN) {
                self.bump();
            }
        } else if self.peek_kw(FROM) || self.peek_kw(IN) {
            self.bump();
        }
        self.object_name();
        if self.at_into() {
            self.targets_clause();
        }
        if self.nth_word(0, "limit") || self.peek_kw(LIMIT) {
            self.bump();
            self.expr();
        }
        self.finish();
    }

    fn execute_stmt(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::ExecuteStmt);
        self.bump();
        self.eat_kw(IMMEDIATE);
        self.expr();
        loop {
            if self.at_into() {
                self.targets_clause();
            } else if self.peek_kw(USING) {
                self.using_args();
            } else if self.peek_kw(RETURNING) || self.peek_kw(RETURN) {
                self.bump();
                self.targets_clause();
            } else {
                break;
            }
        }
        self.finish();
    }

    // --- Exceptions.

    pub(super) fn exception_section(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::ExceptionSection);
        self.bump();
        while self.peek_kw(WHEN) {
            self.start(SyntaxKind::ExceptionHandler);
            self.bump();
            loop {
                if self.nth_word(0, "sqlstate") {
                    self.bump();
                    if self.nth_kind(0) == Some(TokenKind::String) {
                        self.bump();
                    }
                } else {
                    self.object_name();
                }
                if !self.eat_kw(OR) {
                    break;
                }
            }
            self.expect_kw(THEN);
            self.statement_list();
            self.finish();
        }
        self.finish();
    }

    // --- Parameters and return types.

    /// `( [mode] name type [DEFAULT value], .. )`.
    pub(super) fn param_list(&mut self) {
        self.start(SyntaxKind::ParamList);
        self.parened(|p| {
            if p.nth_kind(0) != Some(TokenKind::RParen) {
                loop {
                    p.routine_param();
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
            }
            p.rest_of_group();
        });
        self.finish();
    }

    fn is_param_mode(&self, n: usize) -> bool {
        self.nth_kw(n) == Some(Kw::IN)
            || ["out", "inout", "variadic", "nocopy"]
                .iter()
                .any(|w| self.nth_word(n, w))
    }

    fn routine_param(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::RoutineParam);
        // PostgreSQL: the mode comes first; Oracle: after the name.
        while self.is_param_mode(0) && self.nth_kind(1) != Some(TokenKind::Comma) {
            self.bump();
        }
        let named = self.is_name_at(0)
            && match self.nth_kind(1) {
                Some(TokenKind::Word) => self.nth_kw(1) != Some(DEFAULT),
                Some(TokenKind::QuotedIdent) => true,
                _ => false,
            };
        if named {
            self.def_name(NameKind::Column);
            while self.is_param_mode(0) {
                self.bump();
            }
        }
        if self.is_name_at(0) {
            self.type_ref();
        } else {
            self.note(Expected::Name(NameKind::Type));
            self.recover_item();
        }
        if self.peek_kw(DEFAULT) || self.nth_op(0, ":=") || self.nth_op(0, "=") {
            self.bump();
            self.expr();
        }
        self.finish();
    }

    /// `RETURNS type | SETOF type | TABLE (..)` or Oracle's `RETURN type`.
    pub(super) fn returns_clause(&mut self) {
        self.start(SyntaxKind::ReturnsClause);
        self.bump();
        if self.peek_kw(Kw::TABLE) && self.nth_kind(1) == Some(TokenKind::LParen) {
            self.bump();
            self.column_def_list();
        } else {
            if self.nth_word(0, "setof") {
                self.bump();
            }
            if self.is_name_at(0) {
                self.type_ref();
            } else {
                self.note(Expected::Name(NameKind::Type));
                self.missing();
            }
        }
        self.finish();
    }

    /// The call of a routine from a trigger: `name(args)`.
    pub(super) fn call_in_clause(&mut self) {
        match self.chain_ahead() {
            Some((_, Tail::Call)) => self.expr_bp(super::expr::BP_POSTFIX),
            _ => self.object_name(),
        }
    }
}
