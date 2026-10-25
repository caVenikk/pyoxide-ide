// Data definition: CREATE (table, view, materialized view, index, sequence,
// schema), ALTER TABLE, DROP, TRUNCATE, COMMENT ON, SQLite's PRAGMA. Routines,
// triggers, types and packages are in `routine.rs`. What the parser does not
// know inside a statement it keeps as `Opaque`, never as an error.
use super::{Kw, Parser, Tail};
use crate::lexer::TokenKind;
use crate::tree::{Expected, NameKind, SyntaxKind};

/// What a `CREATE` statement creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Create {
    Table,
    View,
    MaterializedView,
    Index,
    Sequence,
    Schema,
    Type,
    TypeBody,
    Trigger,
    Function,
    Procedure,
    Package,
    PackageBody,
}

impl Parser<'_> {
    /// A DDL statement (or `PRAGMA`, `DO`) begins at the current token.
    pub(super) fn ddl_at(&self) -> bool {
        match self.nth_kw(0) {
            Some(Kw::CREATE | Kw::ALTER | Kw::DROP | Kw::TRUNCATE) => true,
            Some(Kw::COMMENT) => self.nth_kw(1) == Some(Kw::ON),
            Some(Kw::PRAGMA) => self.g.pragma,
            Some(Kw::DO) => {
                self.g.plpgsql
                    && (self.nth_kind(1) == Some(TokenKind::String) || self.nth_word(1, "language"))
            }
            _ => false,
        }
    }

    pub(super) fn ddl_statement(&mut self) {
        match self.nth_kw(0) {
            Some(Kw::CREATE) => self.create_statement(),
            Some(Kw::ALTER) => self.alter_statement(),
            Some(Kw::DROP) => self.drop_statement(),
            Some(Kw::TRUNCATE) => self.truncate_statement(),
            Some(Kw::COMMENT) => self.comment_statement(),
            Some(Kw::PRAGMA) => self.pragma_statement(),
            _ => self.do_statement(),
        }
        self.opaque_to_end();
    }

    /// What is left of the statement, as one `Opaque` node.
    pub(super) fn opaque_to_end(&mut self) {
        if self.eof() || self.at_terminator() {
            return;
        }
        self.start(SyntaxKind::Opaque);
        while !self.eof() && !self.at_terminator() {
            self.bump_group();
        }
        self.finish();
    }

    /// The tokens up to `stop`, a `,` or a `)` that closes the item, a
    /// terminator, as one `Opaque` node (at least one token).
    pub(super) fn opaque_item(&mut self) {
        if self.eof() || self.at_terminator() {
            return;
        }
        self.start(SyntaxKind::Opaque);
        loop {
            self.bump_group();
            if self.eof()
                || self.at_terminator()
                || matches!(self.nth_kind(0), Some(TokenKind::Comma | TokenKind::RParen))
            {
                break;
            }
        }
        self.finish();
    }

    // --- CREATE.

    /// What the `CREATE` at the current token creates, and the index of the
    /// token of the object keyword (the others before it are modifiers).
    fn create_kind(&self) -> Option<(Create, usize)> {
        let mut materialized = false;
        for i in 1..10 {
            if self.nth_kind(i) != Some(TokenKind::Word) {
                return None;
            }
            let w = self.nth_text(i).to_ascii_uppercase();
            let kind = match w.as_str() {
                "OR" | "REPLACE" | "GLOBAL" | "LOCAL" | "TEMP" | "TEMPORARY" | "UNLOGGED"
                | "UNIQUE" | "BITMAP" | "RECURSIVE" | "EDITIONABLE" | "NONEDITIONABLE"
                | "FORCE" | "NOFORCE" | "VIRTUAL" | "CONSTRAINT" | "NO" | "NOT" => continue,
                "MATERIALIZED" => {
                    materialized = true;
                    continue;
                }
                "TABLE" => Create::Table,
                "VIEW" if materialized => Create::MaterializedView,
                "VIEW" => Create::View,
                "INDEX" => Create::Index,
                "SEQUENCE" => Create::Sequence,
                "SCHEMA" => Create::Schema,
                "TYPE" if self.nth_word(i + 1, "body") => Create::TypeBody,
                "TYPE" => Create::Type,
                "TRIGGER" => Create::Trigger,
                "FUNCTION" => Create::Function,
                "PROCEDURE" => Create::Procedure,
                "PACKAGE" if self.nth_word(i + 1, "body") => Create::PackageBody,
                "PACKAGE" => Create::Package,
                _ => return None,
            };
            return Some((kind, i));
        }
        None
    }

    fn create_statement(&mut self) {
        let Some((kind, at)) = self.create_kind() else {
            self.opaque_statement();
            return;
        };
        let node = match kind {
            Create::Table => SyntaxKind::CreateTable,
            Create::View => SyntaxKind::CreateView,
            Create::MaterializedView => SyntaxKind::CreateMaterializedView,
            Create::Index => SyntaxKind::CreateIndex,
            Create::Sequence => SyntaxKind::CreateSequence,
            Create::Schema => SyntaxKind::CreateSchema,
            Create::Type => SyntaxKind::CreateType,
            Create::TypeBody => SyntaxKind::CreateTypeBody,
            Create::Trigger => SyntaxKind::CreateTrigger,
            Create::Function => SyntaxKind::CreateFunction,
            Create::Procedure => SyntaxKind::CreateProcedure,
            Create::Package => SyntaxKind::CreatePackage,
            Create::PackageBody => SyntaxKind::CreatePackageBody,
        };
        self.start(node);
        for _ in 0..=at {
            self.bump();
        }
        if matches!(kind, Create::TypeBody | Create::PackageBody) {
            self.bump();
        }
        match kind {
            Create::Table => self.create_table(),
            Create::View | Create::MaterializedView => self.create_view(),
            Create::Index => self.create_index(),
            Create::Sequence => {
                self.opt_if(true);
                self.object_name();
                self.opaque_to_end();
            }
            Create::Schema => {
                self.opt_if(true);
                // `AUTHORIZATION role` alone names the schema after the role.
                if self.peek_word("authorization") {
                    self.bump();
                }
                self.chain_of(SyntaxKind::SchemaRef);
                self.opaque_to_end();
            }
            Create::Type => self.create_type(),
            Create::TypeBody => self.create_type_body(),
            Create::Trigger => self.create_trigger(),
            Create::Function | Create::Procedure => self.create_routine(),
            Create::Package => self.create_package(false),
            Create::PackageBody => self.create_package(true),
        }
        self.finish();
    }

    pub(super) fn peek_word(&self, w: &str) -> bool {
        self.nth_word(0, w)
    }

    /// `IF [NOT] EXISTS`.
    pub(super) fn opt_if(&mut self, not: bool) {
        use Kw::*;
        let hit = self.peek_kw(IF)
            && if not {
                self.nth_kw(1) == Some(NOT) && self.nth_kw(2) == Some(EXISTS)
            } else {
                self.nth_kw(1) == Some(EXISTS)
            };
        if hit {
            for _ in 0..(if not { 3 } else { 2 }) {
                self.bump();
            }
        }
    }

    /// A dotted name as a node of `outer` (a definition or a reference).
    pub(super) fn chain_of(&mut self, outer: SyntaxKind) {
        match self.chain_ahead() {
            Some((parts, tail)) => {
                let missing = matches!(tail, Tail::Missing | Tail::Star);
                self.chain(outer, parts, missing, false);
            }
            None => {
                let name = match outer {
                    SyntaxKind::FunctionRef => NameKind::Function,
                    SyntaxKind::TypeRef => NameKind::Type,
                    SyntaxKind::SchemaRef => NameKind::Schema,
                    SyntaxKind::TableRef => NameKind::Table,
                    SyntaxKind::ColumnRef => NameKind::Column,
                    _ => NameKind::Object,
                };
                self.note(Expected::Name(name));
                self.recover_item();
            }
        }
    }

    /// A name that the statement defines.
    pub(super) fn def_name(&mut self, kind: NameKind) {
        self.start(SyntaxKind::DefName);
        if self.is_name_at(0) {
            self.name_part();
        } else {
            self.note(Expected::Name(kind));
            self.recover_item();
        }
        self.finish();
    }

    fn create_table(&mut self) {
        use Kw::*;
        self.opt_if(true);
        self.table_name();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.table_elements();
        }
        // Options, then `AS query`; what is not known is kept opaque.
        loop {
            if self.eof() || self.at_terminator() {
                break;
            }
            if self.peek_kw(AS) && self.query_follows_or_ends(1) {
                self.bump();
                self.query();
                continue;
            }
            self.start(SyntaxKind::Opaque);
            loop {
                self.bump_group();
                if self.eof()
                    || self.at_terminator()
                    || (self.peek_kw(AS) && self.query_follows_or_ends(1))
                    || self.at_no_data()
                {
                    break;
                }
            }
            self.finish();
        }
    }

    /// `WITH [NO] DATA`.
    fn at_no_data(&self) -> bool {
        self.peek_kw(Kw::WITH) && (self.nth_word(1, "data") || self.nth_word(1, "no"))
    }

    /// A query starts at the `n`th token.
    fn query_follows(&self, n: usize) -> bool {
        let mut k = n;
        while self.nth_kind(k) == Some(TokenKind::LParen) {
            k += 1;
        }
        self.query_starts(k) || (self.g.table_statement && self.nth_kw(n) == Some(Kw::TABLE))
    }

    /// A query starts at the `n`th token, or the statement ends there (the
    /// query is missing).
    fn query_follows_or_ends(&self, n: usize) -> bool {
        self.query_follows(n)
            || self.nth(n).is_none()
            || matches!(
                self.nth_kind(n),
                Some(TokenKind::Semicolon | TokenKind::SlashLine)
            )
    }

    /// `( element, .. )` of a table.
    fn table_elements(&mut self) {
        self.parened(|p| {
            if p.nth_kind(0) == Some(TokenKind::RParen) {
                p.note(Expected::Name(NameKind::Column));
                p.missing();
            } else {
                loop {
                    p.table_element();
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
            }
            p.rest_of_group();
        });
    }

    /// `( column definition, .. )`.
    pub(super) fn column_def_list(&mut self) {
        self.parened(|p| {
            if p.nth_kind(0) != Some(TokenKind::RParen) {
                loop {
                    p.column_def();
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
            }
            p.rest_of_group();
        });
    }

    fn table_element(&mut self) {
        if self.table_constraint_starts() {
            self.table_constraint();
        } else if self.peek_kw(Kw::LIKE) && self.is_name_at(1) {
            self.start(SyntaxKind::Opaque);
            self.bump();
            self.table_name();
            while !self.at_recovery() && self.nth_kind(0) != Some(TokenKind::Comma) {
                self.bump_group();
            }
            self.finish();
        } else if self.is_name_at(0) {
            self.column_def();
        } else {
            self.note(Expected::Name(NameKind::Column));
            self.recover_item();
        }
    }

    fn table_constraint_starts(&self) -> bool {
        use Kw::*;
        match self.nth_kw(0) {
            Some(CONSTRAINT) => true,
            Some(CHECK) => self.nth_kind(1) == Some(TokenKind::LParen),
            Some(PRIMARY) => self.nth_kw(1) == Some(KEY),
            Some(FOREIGN) => self.nth_kw(1) == Some(KEY),
            Some(UNIQUE) => {
                matches!(self.nth_kind(1), Some(TokenKind::LParen)) || self.nth_word(1, "key")
            }
            Some(EXCLUDE) => {
                self.nth_kind(1) == Some(TokenKind::LParen) || self.nth_word(1, "using")
            }
            _ => false,
        }
    }

    /// The current token starts a column constraint.
    fn column_constraint_starts(&self) -> bool {
        use Kw::*;
        matches!(
            self.nth_kw(0),
            Some(
                CONSTRAINT
                    | NOT
                    | NULL
                    | DEFAULT
                    | PRIMARY
                    | UNIQUE
                    | CHECK
                    | REFERENCES
                    | GENERATED
                    | COLLATE
            )
        ) || (self.peek_kw(AS) && self.nth_kind(1) == Some(TokenKind::LParen))
            || self.nth_word(0, "autoincrement")
    }

    pub(super) fn column_def(&mut self) {
        self.start(SyntaxKind::ColumnDef);
        self.def_name(NameKind::Column);
        if self.is_name_at(0) && !self.column_constraint_starts() {
            self.type_ref();
        }
        loop {
            if self.column_constraint_starts() {
                self.column_constraint();
            } else if self.eof()
                || self.at_terminator()
                || matches!(self.nth_kind(0), Some(TokenKind::Comma | TokenKind::RParen))
            {
                break;
            } else {
                // Options the parser does not know (`VISIBLE`, `ENCRYPT`, ...).
                self.start(SyntaxKind::Opaque);
                loop {
                    self.bump_group();
                    if self.eof()
                        || self.at_terminator()
                        || self.column_constraint_starts()
                        || matches!(self.nth_kind(0), Some(TokenKind::Comma | TokenKind::RParen))
                    {
                        break;
                    }
                }
                self.finish();
            }
        }
        self.finish();
    }

    fn column_constraint(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::ColumnConstraint);
        if self.at_kw(CONSTRAINT) {
            self.bump();
            self.def_name(NameKind::Object);
        }
        if self.at_kw(NOT) {
            self.bump();
            self.expect_kw(NULL);
        } else if self.at_kw(NULL) {
            self.bump();
        } else if self.at_kw(PRIMARY) {
            self.bump();
            self.expect_kw(KEY);
            if self.peek_kw(ASC) || self.peek_kw(DESC) {
                self.bump();
            }
            if self.peek_kw(ON) && self.nth_kw(1) == Some(CONFLICT) {
                self.bump();
                self.bump();
                self.bump();
            }
            if self.nth_word(0, "autoincrement") {
                self.bump();
            }
        } else if self.at_kw(UNIQUE) {
            self.bump();
            if self.peek_kw(NULLS) {
                self.bump();
                self.eat_kw(NOT);
                self.bump();
            }
        } else if self.at_kw(CHECK) {
            self.bump();
            self.parened(|p| p.expr());
            if self.peek_kw(NO) && self.nth_word(1, "inherit") {
                self.bump();
                self.bump();
            }
        } else if self.at_kw(REFERENCES) {
            self.references_clause();
        } else if self.at_kw(DEFAULT) {
            self.bump();
            if self.peek_kw(ON) && self.nth_kw(1) == Some(NULL) {
                self.bump();
                self.bump();
            }
            self.default_expr();
        } else if self.at_kw(GENERATED) {
            self.generated();
        } else if self.at_kw(COLLATE) {
            self.bump();
            if matches!(
                self.nth_kind(0),
                Some(TokenKind::Word | TokenKind::QuotedIdent | TokenKind::String)
            ) {
                self.bump();
            }
        } else if self.at_kw(AS) {
            self.generated_expr();
        } else if self.nth_word(0, "autoincrement") {
            self.bump();
        } else {
            self.missing();
        }
        self.constraint_options();
        self.finish();
    }

    /// A default value: an expression that ends before a following constraint.
    fn default_expr(&mut self) {
        let saved = self.stop_not_null;
        self.stop_not_null = true;
        self.expr();
        self.stop_not_null = saved;
    }

    /// `GENERATED {ALWAYS | BY DEFAULT [ON NULL]} AS IDENTITY [(..)]` or
    /// `GENERATED ALWAYS AS (expr) [STORED | VIRTUAL]`.
    fn generated(&mut self) {
        self.bump();
        while !self.eof() && !self.at_terminator() && !self.peek_kw(Kw::AS) {
            if matches!(
                self.nth_kind(0),
                Some(TokenKind::Comma | TokenKind::RParen | TokenKind::LParen)
            ) {
                return;
            }
            self.bump();
        }
        if self.peek_kw(Kw::AS) {
            if self.nth_word(1, "identity") {
                self.bump();
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.bump_group();
                }
            } else {
                self.generated_expr();
            }
        }
    }

    /// `AS (expr) [STORED | VIRTUAL]`.
    fn generated_expr(&mut self) {
        self.bump();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.parened(|p| p.expr());
        } else {
            self.missing();
        }
        if self.nth_word(0, "stored") || self.nth_word(0, "virtual") {
            self.bump();
        }
    }

    /// `REFERENCES table [(columns)] [options]`.
    fn references_clause(&mut self) {
        self.start(SyntaxKind::ReferencesClause);
        self.bump();
        self.table_name();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_list();
        }
        self.constraint_options();
        self.finish();
    }

    /// `ON DELETE action`, `MATCH FULL`, `[NOT] DEFERRABLE`, `NOT VALID`,
    /// `ENABLE`, `USING INDEX ..` and the like after a constraint.
    fn constraint_options(&mut self) {
        use Kw::*;
        loop {
            if self.peek_kw(ON)
                && (self.nth_kw(1) == Some(DELETE) || self.nth_kw(1) == Some(UPDATE))
            {
                self.bump();
                self.bump();
                if self.peek_kw(SET) {
                    self.bump();
                    self.bump();
                    if self.nth_kind(0) == Some(TokenKind::LParen) {
                        self.bump_group();
                    }
                } else if self.peek_kw(NO) {
                    self.bump();
                    self.bump();
                } else if self.nth_kind(0) == Some(TokenKind::Word) {
                    self.bump();
                }
            } else if self.peek_kw(ON) && self.nth_kw(1) == Some(CONFLICT) {
                self.bump();
                self.bump();
                self.bump();
            } else if self.peek_kw(NOT)
                && (self.nth_word(1, "deferrable")
                    || self.nth_word(1, "valid")
                    || self.nth_word(1, "enforced"))
                || (self.peek_kw(NO) && self.nth_word(1, "inherit"))
            {
                self.bump();
                self.bump();
            } else if self.peek_kw(USING) && self.nth_word(1, "index") {
                self.bump();
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.bump_group();
                } else {
                    while self.nth_word(0, "tablespace") && self.is_name_at(1) {
                        self.bump();
                        self.bump();
                    }
                }
            } else if (self.peek_kw(INCLUDE) || self.peek_kw(WITH))
                && self.nth_kind(1) == Some(TokenKind::LParen)
            {
                self.bump();
                self.bump_group();
            } else if self.nth_word(0, "deferrable") {
                self.bump();
            } else if self.nth_word(0, "match") || self.nth_word(0, "initially") {
                self.bump();
                if self.nth_kind(0) == Some(TokenKind::Word) && !self.at_recovery() {
                    self.bump();
                }
            } else if [
                "enable",
                "disable",
                "validate",
                "novalidate",
                "rely",
                "norely",
            ]
            .iter()
            .any(|w| self.nth_word(0, w))
            {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn table_constraint(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::TableConstraint);
        if self.at_kw(CONSTRAINT) {
            self.bump();
            self.def_name(NameKind::Object);
        }
        if self.at_kw(PRIMARY) {
            self.bump();
            self.expect_kw(KEY);
            self.constraint_columns();
        } else if self.at_kw(UNIQUE) {
            self.bump();
            if self.nth_word(0, "key") {
                self.bump();
            }
            if self.peek_kw(NULLS) {
                self.bump();
                self.eat_kw(NOT);
                self.bump();
            }
            self.constraint_columns();
        } else if self.at_kw(FOREIGN) {
            self.bump();
            self.expect_kw(KEY);
            self.constraint_columns();
            if self.at_kw(REFERENCES) {
                self.references_clause();
            } else {
                self.missing();
            }
        } else if self.at_kw(CHECK) {
            self.bump();
            self.parened(|p| p.expr());
        } else if self.at_kw(EXCLUDE) {
            self.bump();
            while !self.eof()
                && !self.at_terminator()
                && !matches!(self.nth_kind(0), Some(TokenKind::Comma | TokenKind::RParen))
            {
                self.bump_group();
            }
        } else {
            self.missing();
        }
        self.constraint_options();
        self.finish();
    }

    fn constraint_columns(&mut self) {
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_list();
        } else {
            self.at(TokenKind::LParen);
            self.missing();
        }
    }

    fn create_view(&mut self) {
        use Kw::*;
        self.opt_if(true);
        self.table_name();
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.column_list();
        }
        loop {
            if self.eof() || self.at_terminator() {
                break;
            }
            if self.peek_kw(AS) && self.query_follows_or_ends(1) {
                self.bump();
                self.query();
                continue;
            }
            self.start(SyntaxKind::Opaque);
            loop {
                self.bump_group();
                if self.eof()
                    || self.at_terminator()
                    || (self.peek_kw(AS) && self.query_follows_or_ends(1))
                {
                    break;
                }
            }
            self.finish();
        }
    }

    fn create_index(&mut self) {
        use Kw::*;
        if self.nth_word(0, "concurrently") {
            self.bump();
        }
        self.opt_if(true);
        if !self.peek_kw(ON) {
            self.object_name();
        }
        self.expect_kw(ON);
        if self.peek_kw(ONLY) {
            self.bump();
        }
        self.table_name();
        if self.peek_kw(USING) && self.nth_kind(1) == Some(TokenKind::Word) {
            self.bump();
            self.bump();
        }
        if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.parened(|p| {
                loop {
                    p.index_element();
                    if !p.eat(TokenKind::Comma) {
                        break;
                    }
                }
                p.rest_of_group();
            });
        } else {
            self.at(TokenKind::LParen);
            self.missing();
        }
        loop {
            if self.eof() || self.at_terminator() {
                break;
            }
            if self.peek_kw(WHERE) {
                self.where_clause();
            } else if self.peek_kw(INCLUDE) && self.nth_kind(1) == Some(TokenKind::LParen) {
                self.bump();
                self.column_list();
            } else {
                self.start(SyntaxKind::Opaque);
                loop {
                    self.bump_group();
                    if self.eof()
                        || self.at_terminator()
                        || self.peek_kw(WHERE)
                        || (self.peek_kw(INCLUDE) && self.nth_kind(1) == Some(TokenKind::LParen))
                    {
                        break;
                    }
                }
                self.finish();
            }
        }
    }

    // --- ALTER.

    fn alter_statement(&mut self) {
        use Kw::*;
        let table = self.nth_kw(1) == Some(TABLE);
        let known = table
            || matches!(
                self.nth_kw(1),
                Some(INDEX | SEQUENCE | TYPE | FUNCTION | PROCEDURE | TRIGGER | PACKAGE)
            )
            || self.nth_word(1, "view")
            || self.nth_word(1, "schema")
            || (self.nth_word(1, "materialized") && self.nth_word(2, "view"));
        if !known {
            self.opaque_statement();
            return;
        }
        self.start(if table {
            SyntaxKind::AlterTable
        } else {
            SyntaxKind::AlterObject
        });
        self.bump();
        self.bump();
        if self.nth_word(0, "view") {
            self.bump();
        }
        if self.nth_word(0, "body") {
            self.bump();
        }
        self.opt_if(false);
        if table {
            if self.peek_kw(ONLY) {
                self.bump();
            }
            self.table_name();
            if self.nth_op(0, "*") {
                self.bump();
            }
            loop {
                if self.at_terminator() {
                    break;
                }
                self.alter_action();
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        } else {
            let view = self.prev_text().eq_ignore_ascii_case("view");
            let kind = if view {
                SyntaxKind::TableRef
            } else if self.prev_text().eq_ignore_ascii_case("function")
                || self.prev_text().eq_ignore_ascii_case("procedure")
            {
                SyntaxKind::FunctionRef
            } else if self.prev_text().eq_ignore_ascii_case("type") {
                SyntaxKind::TypeRef
            } else if self.prev_text().eq_ignore_ascii_case("schema") {
                SyntaxKind::SchemaRef
            } else {
                SyntaxKind::ObjectRef
            };
            self.chain_of(kind);
            self.opaque_to_end();
        }
        self.finish();
    }

    fn alter_action(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::AlterAction);
        if self.at_kw(ADD) {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                // Oracle: ADD (column or constraint, ..)
                self.parened(|p| {
                    loop {
                        p.table_element();
                        if !p.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    p.rest_of_group();
                });
            } else {
                if self.at_kw(COLUMN) {
                    self.bump();
                }
                self.opt_if(true);
                if self.table_constraint_starts() {
                    self.table_constraint();
                } else if self.is_name_at(0) {
                    self.column_def();
                } else {
                    self.note(Expected::Name(NameKind::Column));
                    self.recover_item();
                }
            }
        } else if self.at_kw(DROP) {
            self.bump();
            if self.at_kw(CONSTRAINT) {
                self.bump();
                self.opt_if(false);
                self.object_name();
            } else if self.peek_kw(PRIMARY) {
                self.bump();
                self.expect_kw(KEY);
            } else if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.column_list();
            } else {
                if self.at_kw(COLUMN) {
                    self.bump();
                }
                self.opt_if(false);
                self.column_name_ref();
            }
            while self.nth_kind(0) == Some(TokenKind::Word)
                && (self.peek_kw(CASCADE)
                    || self.nth_word(0, "restrict")
                    || self.nth_word(0, "constraints"))
            {
                self.bump();
            }
        } else if self.at_kw(ALTER) {
            self.bump();
            if self.at_kw(COLUMN) {
                self.bump();
            }
            self.column_name_ref();
            self.alter_column_rest();
        } else if self.nth_word(0, "modify") {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::LParen) {
                self.column_def_list();
            } else {
                if self.at_kw(COLUMN) {
                    self.bump();
                }
                self.column_def();
            }
        } else if self.nth_word(0, "rename") {
            self.bump();
            if self.at_kw(TO) {
                self.bump();
                self.def_name(NameKind::Table);
            } else if self.at_kw(CONSTRAINT) {
                self.bump();
                self.object_name();
                self.expect_kw(TO);
                self.def_name(NameKind::Object);
            } else {
                if self.at_kw(COLUMN) {
                    self.bump();
                }
                self.column_name_ref();
                self.expect_kw(TO);
                self.def_name(NameKind::Column);
            }
        } else if self.eof() {
            self.missing();
        } else {
            self.opaque_item();
        }
        self.finish();
    }

    /// What follows `ALTER [COLUMN] name`.
    fn alter_column_rest(&mut self) {
        use Kw::*;
        loop {
            if self.eof()
                || self.at_terminator()
                || matches!(self.nth_kind(0), Some(TokenKind::Comma | TokenKind::RParen))
            {
                break;
            }
            if self.peek_kw(SET) && self.nth_kw(1) == Some(DEFAULT) {
                self.bump();
                self.bump();
                self.default_expr();
            } else if self.nth_word(0, "type")
                || self.peek_kw(TYPE)
                || (self.peek_kw(SET) && self.nth_word(1, "data") && self.peek_kw_at(2, TYPE))
            {
                if self.peek_kw(SET) {
                    self.bump();
                    self.bump();
                }
                self.bump();
                self.type_ref();
                if self.peek_kw(USING) {
                    self.bump();
                    self.expr();
                }
                if self.peek_kw(COLLATE) {
                    self.bump();
                    self.bump();
                }
            } else {
                self.bump_group();
            }
        }
    }

    fn peek_kw_at(&self, n: usize, kw: Kw) -> bool {
        self.nth_kw(n) == Some(kw)
    }

    // --- DROP, TRUNCATE, COMMENT, PRAGMA, DO.

    fn drop_statement(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::DropStatement);
        self.bump();
        let mut mods = 0;
        while mods < 3
            && self.nth_kind(0) == Some(TokenKind::Word)
            && [
                "public",
                "temporary",
                "temp",
                "materialized",
                "global",
                "unique",
            ]
            .iter()
            .any(|w| self.nth_word(0, w))
        {
            self.bump();
            mods += 1;
        }
        let kind = self.nth_text(0).to_ascii_lowercase();
        if self.nth_kind(0) == Some(TokenKind::Word) {
            self.bump();
        } else {
            self.missing();
        }
        if matches!(kind.as_str(), "type" | "package") && self.nth_word(0, "body") {
            self.bump();
        }
        if self.nth_word(0, "concurrently") {
            self.bump();
        }
        self.opt_if(false);
        let node = match kind.as_str() {
            "table" | "view" => SyntaxKind::TableRef,
            "function" | "procedure" | "routine" | "aggregate" => SyntaxKind::FunctionRef,
            "type" | "domain" => SyntaxKind::TypeRef,
            "schema" => SyntaxKind::SchemaRef,
            _ => SyntaxKind::ObjectRef,
        };
        let routine = node == SyntaxKind::FunctionRef;
        loop {
            self.chain_of(node);
            if routine && self.nth_kind(0) == Some(TokenKind::LParen) {
                self.bump_group();
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        if self.peek_kw(ON) {
            self.bump();
            self.table_name();
        }
        while self.nth_kind(0) == Some(TokenKind::Word) && !self.at_terminator() {
            self.bump();
        }
        self.finish();
    }

    fn truncate_statement(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::Truncate);
        self.bump();
        self.eat_kw(TABLE);
        loop {
            if self.peek_kw(ONLY) {
                self.bump();
            }
            self.table_name();
            if self.nth_op(0, "*") {
                self.bump();
            }
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        while self.nth_kind(0) == Some(TokenKind::Word) && !self.at_terminator() {
            self.bump();
        }
        self.finish();
    }

    fn comment_statement(&mut self) {
        use Kw::*;
        self.start(SyntaxKind::CommentStatement);
        self.bump();
        self.bump();
        let kind = self.nth_text(0).to_ascii_lowercase();
        if self.nth_kind(0) == Some(TokenKind::Word) {
            self.bump();
        }
        if kind == "materialized" && self.nth_word(0, "view") {
            self.bump();
        }
        match kind.as_str() {
            "column" => self.chain_of(SyntaxKind::ColumnRef),
            "table" | "view" | "materialized" => self.table_name(),
            "function" | "procedure" | "routine" | "aggregate" => {
                self.chain_of(SyntaxKind::FunctionRef);
                if self.nth_kind(0) == Some(TokenKind::LParen) {
                    self.bump_group();
                }
            }
            "type" | "domain" => self.chain_of(SyntaxKind::TypeRef),
            "schema" => self.chain_of(SyntaxKind::SchemaRef),
            _ => self.chain_of(SyntaxKind::ObjectRef),
        }
        // `COMMENT ON CONSTRAINT c ON t IS ..`, `TRIGGER t ON tbl`.
        while !self.eof() && !self.at_terminator() && !self.peek_kw(IS) {
            self.bump_group();
        }
        if self.at_kw(IS) {
            self.bump();
            if self.nth_kind(0) == Some(TokenKind::String) || self.peek_kw(NULL) {
                self.bump();
            } else {
                self.missing();
            }
        } else {
            self.missing();
        }
        self.finish();
    }

    fn pragma_statement(&mut self) {
        self.start(SyntaxKind::Pragma);
        self.bump();
        self.chain_of(SyntaxKind::ObjectRef);
        if self.nth_op(0, "=") {
            self.bump();
            if !self.eof() && !self.at_terminator() {
                self.bump();
            }
        } else if self.nth_kind(0) == Some(TokenKind::LParen) {
            self.bump_group();
        }
        self.finish();
    }
}
