use crate::keywords;

/// The SQL dialects of the text layer. A dialect is data (lexical rules,
/// keyword tables, folding and quoting of identifiers), not behaviour spread
/// around the code. `Generic` accepts the union of the common syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Dialect {
    #[default]
    Generic,
    Postgres,
    Oracle,
    Sqlite,
}

/// Whether a keyword may be used as an unquoted name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeywordClass {
    /// Cannot be a table or column name without quotes.
    Reserved,
    /// Has a meaning in some places of the grammar, still a valid name.
    NonReserved,
}

/// Which parameter forms the lexer recognises. Defaults per dialect are in
/// [`Dialect::default_params`]; a caller may change them (the forms are
/// configurable per file in the reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ParamForms {
    /// `?`
    pub question: bool,
    /// `?1`, only together with `question`
    pub question_numbered: bool,
    /// `$1`
    pub dollar_numbered: bool,
    /// `$name`
    pub dollar_named: bool,
    /// `${name}`
    pub brace_named: bool,
    /// `:name`
    pub colon_named: bool,
    /// `:1`
    pub colon_numbered: bool,
    /// `@name`
    pub at_named: bool,
    /// `%(name)s`
    pub pyformat: bool,
    /// `%s`
    pub percent_s: bool,
}

impl ParamForms {
    /// No parameters at all.
    pub const NONE: ParamForms = ParamForms {
        question: false,
        question_numbered: false,
        dollar_numbered: false,
        dollar_named: false,
        brace_named: false,
        colon_named: false,
        colon_numbered: false,
        at_named: false,
        pyformat: false,
        percent_s: false,
    };

    /// The forms the Python database drivers use (`%s`, `%(name)s`), for
    /// injected Python strings.
    pub const PYTHON_DRIVERS: ParamForms = ParamForms {
        pyformat: true,
        percent_s: true,
        ..ParamForms::NONE
    };
}

/// Lexical rules of a dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexRules {
    /// `/* /* */ */` nests (PostgreSQL).
    pub nested_comments: bool,
    /// `$tag$ ... $tag$` strings.
    pub dollar_quotes: bool,
    /// `E'..'` strings, where `\` escapes the next character.
    pub escape_strings: bool,
    /// `X'..'` and `B'..'` strings.
    pub bit_strings: bool,
    /// `U&'..'` strings.
    pub unicode_strings: bool,
    /// Oracle alternative quoting `q'[..]'` and `nq'{..}'`.
    pub q_quotes: bool,
    /// `[name]` quotes an identifier.
    pub bracket_idents: bool,
    /// `` `name` `` quotes an identifier.
    pub backtick_idents: bool,
    /// A line with only `/` ends a statement or a PL/SQL block.
    pub slash_terminator: bool,
    /// `#` may continue an identifier (Oracle).
    pub hash_in_words: bool,
}

/// Grammar flags of a dialect: one parser serves every dialect and a flag
/// turns a construct on. `Generic` has the union of the constructs and is
/// lenient where the engines differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Grammar {
    /// `LATERAL` before a subquery or a function in `FROM`.
    pub lateral: bool,
    /// `SELECT DISTINCT ON (..)`.
    pub distinct_on: bool,
    /// `ILIKE`.
    pub ilike: bool,
    /// `LIMIT n [OFFSET m]`, `LIMIT m, n`.
    pub limit: bool,
    /// `OFFSET n ROWS FETCH FIRST m ROWS ONLY`.
    pub fetch_first: bool,
    /// `START WITH .. CONNECT BY [NOCYCLE] ..`, `PRIOR`, `CONNECT_BY_ROOT`.
    pub connect_by: bool,
    /// The outer join mark `(+)`.
    pub outer_join_mark: bool,
    /// `PIVOT` and `UNPIVOT` after a table.
    pub pivot: bool,
    /// `MINUS` as a set operation.
    pub minus: bool,
    /// `expr::type`.
    pub double_colon_cast: bool,
    /// `ARRAY[..]`, `ARRAY(subquery)` and subscripts `a[1]`, `a[1:2]`.
    pub arrays: bool,
    /// `GLOB`, `REGEXP` and `MATCH` operators.
    pub sqlite_operators: bool,
    /// `ISNULL`, `NOTNULL` and `NOT NULL` after an expression.
    pub null_postfix: bool,
    /// `SELECT UNIQUE`.
    pub select_unique: bool,
    /// A join other than `CROSS` and `NATURAL` needs `ON` or `USING`.
    pub join_requires_condition: bool,
    /// `SELECT` with no select list (`SELECT FROM t`).
    pub empty_select_list: bool,
    /// `CROSS APPLY` and `OUTER APPLY`.
    pub apply_joins: bool,
    /// `TABLE name` as a query, and `ONLY name` in `FROM`.
    pub table_statement: bool,
    /// `FILTER (WHERE ..)` after an aggregate.
    pub filter_clause: bool,
    /// `WITHIN GROUP (ORDER BY ..)` after an aggregate.
    pub within_group: bool,
    /// `INSERT .. ON CONFLICT ..`.
    pub on_conflict: bool,
    /// `RETURNING` after `INSERT`, `UPDATE`, `DELETE` and `MERGE`.
    pub returning: bool,
    /// The `MERGE` statement.
    pub merge: bool,
    /// Oracle's multi-table `INSERT ALL|FIRST`.
    pub multi_insert: bool,
    /// SQLite's `INSERT OR REPLACE`, `UPDATE OR IGNORE`, `REPLACE INTO`.
    pub conflict_resolution: bool,
    /// `UPDATE .. FROM sources`.
    pub update_from: bool,
    /// `DELETE .. USING sources`.
    pub delete_using: bool,
    /// PL/SQL: anonymous blocks, `CREATE PROCEDURE .. IS ..`, packages,
    /// `%TYPE`, cursor attributes.
    pub plsql: bool,
    /// PL/pgSQL: `DO` blocks and `$$` function bodies, `CREATE FUNCTION ..
    /// AS $$ .. $$ LANGUAGE ..`.
    pub plpgsql: bool,
    /// `PRAGMA`, and trigger bodies of SQLite.
    pub pragma: bool,
}

const GENERIC: LexRules = LexRules {
    nested_comments: false,
    dollar_quotes: true,
    escape_strings: true,
    bit_strings: true,
    unicode_strings: true,
    q_quotes: false,
    bracket_idents: true,
    backtick_idents: true,
    slash_terminator: false,
    hash_in_words: false,
};

const POSTGRES: LexRules = LexRules {
    nested_comments: true,
    dollar_quotes: true,
    escape_strings: true,
    bit_strings: true,
    unicode_strings: true,
    q_quotes: false,
    bracket_idents: false,
    backtick_idents: false,
    slash_terminator: false,
    hash_in_words: false,
};

const ORACLE: LexRules = LexRules {
    nested_comments: false,
    dollar_quotes: false,
    escape_strings: false,
    bit_strings: false,
    unicode_strings: false,
    q_quotes: true,
    bracket_idents: false,
    backtick_idents: false,
    slash_terminator: true,
    hash_in_words: true,
};

const SQLITE: LexRules = LexRules {
    nested_comments: false,
    dollar_quotes: false,
    escape_strings: false,
    bit_strings: true,
    unicode_strings: false,
    q_quotes: false,
    bracket_idents: true,
    backtick_idents: true,
    slash_terminator: false,
    hash_in_words: false,
};

impl Dialect {
    pub const ALL: [Dialect; 4] = [
        Dialect::Generic,
        Dialect::Postgres,
        Dialect::Oracle,
        Dialect::Sqlite,
    ];

    /// A stable identifier for settings files.
    pub fn id(self) -> &'static str {
        match self {
            Dialect::Generic => "generic",
            Dialect::Postgres => "postgres",
            Dialect::Oracle => "oracle",
            Dialect::Sqlite => "sqlite",
        }
    }

    pub fn from_id(id: &str) -> Option<Dialect> {
        Dialect::ALL
            .into_iter()
            .find(|d| d.id().eq_ignore_ascii_case(id))
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Dialect::Generic => "SQL",
            Dialect::Postgres => "PostgreSQL",
            Dialect::Oracle => "Oracle",
            Dialect::Sqlite => "SQLite",
        }
    }

    pub fn rules(self) -> &'static LexRules {
        match self {
            Dialect::Generic => &GENERIC,
            Dialect::Postgres => &POSTGRES,
            Dialect::Oracle => &ORACLE,
            Dialect::Sqlite => &SQLITE,
        }
    }

    /// The grammar flags of the parser for this dialect.
    pub fn grammar(self) -> Grammar {
        let all = Grammar {
            lateral: true,
            distinct_on: true,
            ilike: true,
            limit: true,
            fetch_first: true,
            connect_by: true,
            outer_join_mark: true,
            pivot: true,
            minus: true,
            double_colon_cast: true,
            arrays: true,
            sqlite_operators: true,
            null_postfix: true,
            select_unique: true,
            join_requires_condition: false,
            empty_select_list: true,
            apply_joins: true,
            table_statement: true,
            filter_clause: true,
            within_group: true,
            on_conflict: true,
            returning: true,
            merge: true,
            multi_insert: true,
            conflict_resolution: true,
            update_from: true,
            delete_using: true,
            plsql: true,
            plpgsql: true,
            pragma: true,
        };
        let none = Grammar {
            lateral: false,
            distinct_on: false,
            ilike: false,
            limit: false,
            fetch_first: false,
            connect_by: false,
            outer_join_mark: false,
            pivot: false,
            minus: false,
            double_colon_cast: false,
            arrays: false,
            sqlite_operators: false,
            null_postfix: false,
            select_unique: false,
            join_requires_condition: false,
            empty_select_list: false,
            apply_joins: false,
            table_statement: false,
            filter_clause: false,
            within_group: false,
            on_conflict: false,
            returning: false,
            merge: false,
            multi_insert: false,
            conflict_resolution: false,
            update_from: false,
            delete_using: false,
            plsql: false,
            plpgsql: false,
            pragma: false,
        };
        match self {
            Dialect::Generic => all,
            Dialect::Postgres => Grammar {
                lateral: true,
                distinct_on: true,
                ilike: true,
                limit: true,
                fetch_first: true,
                double_colon_cast: true,
                arrays: true,
                null_postfix: true,
                join_requires_condition: true,
                empty_select_list: true,
                table_statement: true,
                filter_clause: true,
                within_group: true,
                on_conflict: true,
                returning: true,
                merge: true,
                update_from: true,
                delete_using: true,
                plpgsql: true,
                ..none
            },
            Dialect::Oracle => Grammar {
                lateral: true,
                fetch_first: true,
                connect_by: true,
                outer_join_mark: true,
                pivot: true,
                minus: true,
                select_unique: true,
                join_requires_condition: true,
                apply_joins: true,
                within_group: true,
                returning: true,
                merge: true,
                multi_insert: true,
                plsql: true,
                ..none
            },
            Dialect::Sqlite => Grammar {
                limit: true,
                sqlite_operators: true,
                null_postfix: true,
                filter_clause: true,
                on_conflict: true,
                returning: true,
                conflict_resolution: true,
                update_from: true,
                pragma: true,
                ..none
            },
        }
    }

    /// The parameter forms a console of this dialect understands by default.
    pub fn default_params(self) -> ParamForms {
        let base = ParamForms {
            question: true,
            brace_named: true,
            ..ParamForms::NONE
        };
        match self {
            Dialect::Generic => ParamForms {
                question_numbered: false,
                dollar_numbered: true,
                colon_named: true,
                ..base
            },
            Dialect::Postgres => ParamForms {
                dollar_numbered: true,
                colon_named: true,
                ..base
            },
            Dialect::Oracle => ParamForms {
                colon_named: true,
                colon_numbered: true,
                ..base
            },
            Dialect::Sqlite => ParamForms {
                question_numbered: true,
                colon_named: true,
                at_named: true,
                dollar_named: true,
                ..base
            },
        }
    }

    /// The class of a keyword, if `word` is one. Case is ignored. `Generic`
    /// is the union of the dialects: reserved in any of them is reserved.
    pub fn keyword(self, word: &str) -> Option<KeywordClass> {
        match self {
            Dialect::Generic => {
                let all = [Dialect::Postgres, Dialect::Oracle, Dialect::Sqlite];
                let mut found = None;
                for d in all {
                    match d.keyword(word) {
                        Some(KeywordClass::Reserved) => return Some(KeywordClass::Reserved),
                        Some(c) => found = Some(c),
                        None => {}
                    }
                }
                found
            }
            Dialect::Postgres => keywords::lookup(
                word,
                keywords::POSTGRES_RESERVED,
                keywords::POSTGRES_NON_RESERVED,
            ),
            Dialect::Oracle => keywords::lookup(
                word,
                keywords::ORACLE_RESERVED,
                keywords::ORACLE_NON_RESERVED,
            ),
            Dialect::Sqlite => keywords::lookup(
                word,
                keywords::SQLITE_RESERVED,
                keywords::SQLITE_NON_RESERVED,
            ),
        }
    }

    pub fn is_reserved(self, word: &str) -> bool {
        self.keyword(word) == Some(KeywordClass::Reserved)
    }

    /// The catalog name that a name written in SQL stands for. A quoted name
    /// (`"x"`, `` `x` `` or `[x]` where the dialect has them) is kept as
    /// written inside the quotes; an unquoted one folds: PostgreSQL to lower
    /// case, Oracle to upper case, SQLite and Generic to lower case (ASCII;
    /// SQLite compares names case-insensitively, see [`Dialect::names_equal`]).
    pub fn fold(self, written: &str) -> String {
        if let Some(inner) = self.unquote(written) {
            return inner;
        }
        match self {
            Dialect::Oracle => written.to_uppercase(),
            Dialect::Postgres | Dialect::Sqlite | Dialect::Generic => written.to_ascii_lowercase(),
        }
    }

    /// The text inside the quotes if `written` is a quoted identifier of this
    /// dialect, with doubled quote characters collapsed. An unterminated quote
    /// reads to the end.
    pub fn unquote(self, written: &str) -> Option<String> {
        let rules = self.rules();
        let first = written.chars().next()?;
        let (close, doubled) = match first {
            '"' => ('"', true),
            '`' if rules.backtick_idents => ('`', true),
            '[' if rules.bracket_idents => (']', false),
            _ => return None,
        };
        let body = &written[1..];
        let body = body.strip_suffix(close).unwrap_or(body);
        Some(if doubled {
            let pair: String = [close, close].iter().collect();
            body.replace(&pair, &close.to_string())
        } else {
            body.to_string()
        })
    }

    /// Two catalog names (results of [`Dialect::fold`]) denote the same
    /// object. SQLite compares case-insensitively (ASCII), the others exactly.
    pub fn names_equal(self, a: &str, b: &str) -> bool {
        match self {
            Dialect::Sqlite => a.eq_ignore_ascii_case(b),
            _ => a == b,
        }
    }

    /// A catalog name as it is written in SQL: bare when it folds back to the
    /// same name and is not reserved, otherwise in double quotes.
    pub fn quote(self, name: &str) -> String {
        if self.is_plain(name) {
            name.to_string()
        } else {
            self.quote_always(name)
        }
    }

    pub fn quote_always(self, name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    fn is_plain(self, name: &str) -> bool {
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        type Test = fn(char) -> bool;
        let (start_ok, rest_ok): (Test, Test) = match self {
            Dialect::Oracle => (
                |c| c.is_ascii_uppercase(),
                |c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '$' | '#'),
            ),
            Dialect::Postgres => (
                |c| c.is_ascii_lowercase() || c == '_',
                |c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '$'),
            ),
            // Names are compared case-insensitively: any case is plain.
            Dialect::Sqlite => (
                |c| c.is_ascii_alphabetic() || c == '_',
                |c| c.is_ascii_alphanumeric() || c == '_',
            ),
            Dialect::Generic => (
                |c| c.is_ascii_lowercase() || c == '_',
                |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_',
            ),
        };
        start_ok(first) && chars.all(rest_ok) && !self.is_reserved(name)
    }
}
