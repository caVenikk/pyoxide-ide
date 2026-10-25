use crate::dialect::Dialect;
use crate::lexer::{Token, TokenKind, lex};

/// What a statement is, by its first words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementKind {
    /// `SELECT`, `VALUES`, `TABLE x`, also in parentheses.
    Select,
    /// `WITH ...`; [`Classification::writes`] says whether the main statement
    /// changes data.
    With,
    /// `INSERT`, `UPDATE`, `DELETE`, `MERGE`.
    Dml,
    /// `CREATE`, `ALTER`, `DROP`, `TRUNCATE`, `RENAME`, `COMMENT`, `GRANT`,
    /// `REVOKE`.
    Ddl,
    /// `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`, `BEGIN` as a transaction
    /// start, `SET TRANSACTION`.
    Tcl,
    /// An Oracle `BEGIN`/`DECLARE` block, a PostgreSQL `DO`.
    AnonymousBlock,
    Other,
}

/// A statement that changes every row because it has no `WHERE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Danger {
    DeleteWithoutWhere,
    UpdateWithoutWhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DdlAction {
    Create,
    Alter,
    Drop,
    Truncate,
    Comment,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Index,
    Sequence,
    Schema,
    Database,
    Function,
    Procedure,
    Trigger,
    Type,
    Package,
    PackageBody,
    Synonym,
    Extension,
    Column,
    Other,
}

/// One part of a name: its text without quotes, and whether it was quoted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub text: String,
    pub quoted: bool,
}

impl Ident {
    /// The catalog name ([`Dialect::fold`]).
    pub fn name(&self, dialect: Dialect) -> String {
        if self.quoted {
            self.text.clone()
        } else {
            dialect.fold(&self.text)
        }
    }
}

/// `schema.table`, as written, part by part.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub parts: Vec<Ident>,
}

impl QualifiedName {
    /// The folded catalog names of the parts.
    pub fn names(&self, dialect: Dialect) -> Vec<String> {
        self.parts.iter().map(|p| p.name(dialect)).collect()
    }
}

/// The objects a DDL statement names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DdlTarget {
    pub action: DdlAction,
    pub object: ObjectKind,
    /// The named objects (several for `DROP TABLE a, b`); none for an index
    /// created without a name.
    pub names: Vec<QualifiedName>,
    /// The table of `CREATE INDEX n ON t` and `CREATE TRIGGER .. ON t`.
    pub on: Option<QualifiedName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub kind: StatementKind,
    /// Changes data: `INSERT`, `UPDATE`, `DELETE`, `MERGE`, a `WITH` whose
    /// main statement is one of them, an anonymous block (which may).
    pub writes: bool,
    pub danger: Option<Danger>,
    pub ddl: Option<DdlTarget>,
}

/// Classifies one statement given as text.
pub fn classify_text(text: &str, dialect: Dialect) -> Classification {
    let sig: Vec<Token> = lex(text, dialect)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .collect();
    classify(text, &sig, dialect)
}

/// Classifies a statement given the significant tokens (no trivia) of it.
pub fn classify(src: &str, sig: &[Token], dialect: Dialect) -> Classification {
    let c = Cursor { src, sig, dialect };
    c.classify()
}

struct Cursor<'a> {
    src: &'a str,
    sig: &'a [Token],
    dialect: Dialect,
}

const MAIN_VERBS: [&str; 6] = ["SELECT", "INSERT", "UPDATE", "DELETE", "MERGE", "VALUES"];

impl Cursor<'_> {
    fn text(&self, i: usize) -> &str {
        self.sig.get(i).map_or("", |t| t.text(self.src))
    }

    fn kind(&self, i: usize) -> Option<TokenKind> {
        self.sig.get(i).map(|t| t.kind)
    }

    /// Is the token `i` the word `w` (case-insensitive)?
    fn is(&self, i: usize, w: &str) -> bool {
        self.kind(i) == Some(TokenKind::Word) && self.text(i).eq_ignore_ascii_case(w)
    }

    fn is_any(&self, i: usize, words: &[&str]) -> bool {
        words.iter().any(|w| self.is(i, w))
    }

    fn classify(&self) -> Classification {
        let mut kind = StatementKind::Other;
        let mut writes = false;
        let mut verb_at = None;
        let mut i = 0;
        // `(select ..) union ..`
        while self.kind(i) == Some(TokenKind::LParen) {
            i += 1;
        }
        // `<<label>> begin`
        if self.dialect == Dialect::Oracle && self.text(i) == "<<" {
            while i < self.sig.len() && self.text(i) != ">>" {
                i += 1;
            }
            i += 1;
        }
        let d = self.dialect;
        if self.kind(i) == Some(TokenKind::Word) {
            let w = self.text(i).to_ascii_uppercase();
            match w.as_str() {
                "SELECT" | "VALUES" | "TABLE" => kind = StatementKind::Select,
                "WITH" => {
                    kind = StatementKind::With;
                    verb_at = self.main_verb(i + 1);
                    writes = verb_at
                        .is_some_and(|v| self.is_any(v, &["INSERT", "UPDATE", "DELETE", "MERGE"]));
                }
                "INSERT" | "UPDATE" | "DELETE" | "MERGE" => {
                    kind = StatementKind::Dml;
                    writes = true;
                    verb_at = Some(i);
                }
                "REPLACE"
                    if matches!(d, Dialect::Sqlite | Dialect::Generic)
                        && self.is(i + 1, "INTO") =>
                {
                    kind = StatementKind::Dml;
                    writes = true;
                }
                "CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "RENAME" | "COMMENT" | "GRANT"
                | "REVOKE" => {
                    kind = StatementKind::Ddl;
                }
                "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" | "ABORT" => {
                    kind = StatementKind::Tcl
                }
                "START" if self.is(i + 1, "TRANSACTION") => kind = StatementKind::Tcl,
                "PREPARE" if self.is(i + 1, "TRANSACTION") => kind = StatementKind::Tcl,
                "END" if d != Dialect::Oracle => kind = StatementKind::Tcl,
                "SET" if self.is_any(i + 1, &["TRANSACTION", "CONSTRAINT", "CONSTRAINTS"]) => {
                    kind = StatementKind::Tcl
                }
                "BEGIN" if d == Dialect::Oracle => kind = StatementKind::AnonymousBlock,
                "BEGIN" if self.is(i + 1, "NOT") && self.is(i + 2, "ATOMIC") => {
                    kind = StatementKind::AnonymousBlock
                }
                "BEGIN" => kind = StatementKind::Tcl,
                "DECLARE" if d == Dialect::Oracle => kind = StatementKind::AnonymousBlock,
                "DO" if matches!(d, Dialect::Postgres | Dialect::Generic) => {
                    kind = StatementKind::AnonymousBlock
                }
                _ => {}
            }
        }
        if kind == StatementKind::AnonymousBlock {
            writes = true;
        }
        let danger = verb_at.and_then(|v| self.danger(v));
        let ddl = (kind == StatementKind::Ddl)
            .then(|| self.ddl_target(i))
            .flatten();
        Classification {
            kind,
            writes,
            danger,
            ddl,
        }
    }

    /// The main statement of a `WITH`: the first statement verb outside
    /// parentheses after the common table expressions.
    fn main_verb(&self, from: usize) -> Option<usize> {
        let mut depth = 0usize;
        for i in from..self.sig.len() {
            match self.kind(i)? {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth = depth.saturating_sub(1),
                TokenKind::Word if depth == 0 && self.is_any(i, &MAIN_VERBS) => return Some(i),
                _ => {}
            }
        }
        None
    }

    /// `DELETE`/`UPDATE` at `verb` with no `WHERE` outside parentheses.
    fn danger(&self, verb: usize) -> Option<Danger> {
        let which = if self.is(verb, "DELETE") {
            Danger::DeleteWithoutWhere
        } else if self.is(verb, "UPDATE") {
            Danger::UpdateWithoutWhere
        } else {
            return None;
        };
        let mut depth = 0usize;
        for i in verb + 1..self.sig.len() {
            match self.kind(i)? {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth = depth.saturating_sub(1),
                TokenKind::Word if depth == 0 && self.is(i, "WHERE") => return None,
                _ => {}
            }
        }
        Some(which)
    }

    fn ident(&self, i: usize) -> Option<Ident> {
        let t = self.sig.get(i)?;
        match t.kind {
            TokenKind::Word => Some(Ident {
                text: t.text(self.src).to_string(),
                quoted: false,
            }),
            TokenKind::QuotedIdent => {
                let written = t.text(self.src);
                Some(Ident {
                    text: self
                        .dialect
                        .unquote(written)
                        .unwrap_or_else(|| written.to_string()),
                    quoted: true,
                })
            }
            _ => None,
        }
    }

    /// `a.b.c` at `*i`; advances `*i` past it.
    fn name(&self, i: &mut usize) -> Option<QualifiedName> {
        let mut parts = vec![self.ident(*i)?];
        *i += 1;
        while self.kind(*i) == Some(TokenKind::Dot) {
            match self.ident(*i + 1) {
                Some(p) => {
                    parts.push(p);
                    *i += 2;
                }
                None => break,
            }
        }
        Some(QualifiedName { parts })
    }

    fn ddl_target(&self, at: usize) -> Option<DdlTarget> {
        let action = match self.text(at).to_ascii_uppercase().as_str() {
            "CREATE" => DdlAction::Create,
            "ALTER" => DdlAction::Alter,
            "DROP" => DdlAction::Drop,
            "TRUNCATE" => DdlAction::Truncate,
            "COMMENT" => DdlAction::Comment,
            "RENAME" => DdlAction::Rename,
            _ => return None,
        };
        let mut i = at + 1;
        let object = match action {
            DdlAction::Truncate => {
                if self.is(i, "TABLE") {
                    i += 1;
                }
                ObjectKind::Table
            }
            DdlAction::Rename => ObjectKind::Other,
            DdlAction::Comment => {
                if !self.is(i, "ON") {
                    return None;
                }
                i += 1;
                self.object_kind(&mut i, false)?
            }
            _ => self.object_kind(&mut i, true)?,
        };
        if object == ObjectKind::Type && self.is(i, "BODY") {
            i += 1;
        }
        while self.is(i, "CONCURRENTLY") {
            i += 1;
        }
        if self.is(i, "IF") {
            // IF [NOT] EXISTS
            i += 1;
            if self.is(i, "NOT") {
                i += 1;
            }
            if self.is(i, "EXISTS") {
                i += 1;
            }
        }
        if object == ObjectKind::Schema && self.is(i, "AUTHORIZATION") {
            // `CREATE SCHEMA AUTHORIZATION role`: the schema is named after the role.
            i += 1;
        }
        while self.is_any(i, &["CONCURRENTLY", "ONLY"]) {
            i += 1;
        }
        let mut names = Vec::new();
        let mut on = None;
        if !(object == ObjectKind::Index && self.is(i, "ON")) {
            match self.name(&mut i) {
                Some(n) => names.push(n),
                None if action == DdlAction::Create && object == ObjectKind::Index => {}
                None => return None,
            }
            // `DROP TABLE a, b`, `TRUNCATE a, b`
            while matches!(action, DdlAction::Drop | DdlAction::Truncate)
                && self.kind(i) == Some(TokenKind::Comma)
            {
                i += 1;
                if self.is(i, "ONLY") {
                    i += 1;
                }
                match self.name(&mut i) {
                    Some(n) => names.push(n),
                    None => break,
                }
            }
        }
        if action == DdlAction::Create && matches!(object, ObjectKind::Index | ObjectKind::Trigger)
        {
            let mut depth = 0usize;
            for j in i..self.sig.len() {
                match self.kind(j) {
                    Some(TokenKind::LParen) => depth += 1,
                    Some(TokenKind::RParen) => depth = depth.saturating_sub(1),
                    Some(TokenKind::Word) if depth == 0 && self.is(j, "ON") => {
                        let mut k = j + 1;
                        if self.is(k, "ONLY") {
                            k += 1;
                        }
                        on = self.name(&mut k);
                        break;
                    }
                    _ => {}
                }
            }
        }
        if names.is_empty() && on.is_none() {
            return None;
        }
        Some(DdlTarget {
            action,
            object,
            names,
            on,
        })
    }

    /// Skips the modifiers and reads the object keyword at `*i`.
    fn object_kind(&self, i: &mut usize, modifiers: bool) -> Option<ObjectKind> {
        const MODIFIERS: [&str; 17] = [
            "OR",
            "REPLACE",
            "TEMP",
            "TEMPORARY",
            "GLOBAL",
            "LOCAL",
            "UNLOGGED",
            "UNIQUE",
            "BITMAP",
            "EDITIONABLE",
            "NONEDITIONABLE",
            "FORCE",
            "NOFORCE",
            "PUBLIC",
            "CONSTRAINT",
            "VIRTUAL",
            "RECURSIVE",
        ];
        let mut materialized = false;
        for _ in 0..8 {
            let w = self.text(*i).to_ascii_uppercase();
            if self.kind(*i) != Some(TokenKind::Word) {
                return None;
            }
            let kind = match w.as_str() {
                "TABLE" => Some(ObjectKind::Table),
                "VIEW" if materialized => Some(ObjectKind::MaterializedView),
                "VIEW" => Some(ObjectKind::View),
                "INDEX" => Some(ObjectKind::Index),
                "SEQUENCE" => Some(ObjectKind::Sequence),
                "SCHEMA" => Some(ObjectKind::Schema),
                "DATABASE" => Some(ObjectKind::Database),
                "FUNCTION" => Some(ObjectKind::Function),
                "PROCEDURE" => Some(ObjectKind::Procedure),
                "TRIGGER" => Some(ObjectKind::Trigger),
                "TYPE" => Some(ObjectKind::Type),
                "PACKAGE" if self.is(*i + 1, "BODY") => {
                    *i += 1;
                    Some(ObjectKind::PackageBody)
                }
                "PACKAGE" => Some(ObjectKind::Package),
                "SYNONYM" => Some(ObjectKind::Synonym),
                "EXTENSION" => Some(ObjectKind::Extension),
                "COLUMN" => Some(ObjectKind::Column),
                _ => None,
            };
            if let Some(k) = kind {
                *i += 1;
                return Some(k);
            }
            if w == "MATERIALIZED" {
                materialized = true;
            } else if !(modifiers && MODIFIERS.contains(&w.as_str())) {
                return None;
            }
            *i += 1;
        }
        None
    }
}
