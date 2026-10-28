//! SQLite introspection over sqlite_schema and the table-valued PRAGMA functions, written from the public SQLite documentation.

mod load;

use std::collections::HashMap;

use async_trait::async_trait;

use super::change::{self, SchemaStamps, Stamper, Units};
use super::{
    ChangeToken, ColumnInfo, ConstraintInfo, ConstraintKind, Delta, GeneratedColumn, IndexInfo,
    Introspector, Level, ObjectInfo, ObjectKind, SchemaInfo, SchemaSnapshot, Scope, Snapshot,
    fetch_all, wants,
};
use crate::{Connection, Dialect, Error, Result};

pub struct SqliteIntrospector<'a> {
    conn: &'a dyn Connection,
}

impl<'a> SqliteIntrospector<'a> {
    pub fn new(conn: &'a dyn Connection) -> Self {
        Self { conn }
    }
}

const MAIN: &str = "main";

const DATABASES: &str = "SELECT name FROM pragma_database_list ORDER BY seq";

const DATABASE_BY_NAME: &str =
    "SELECT name FROM pragma_database_list WHERE name = ?1 COLLATE NOCASE";

/// Pragma functions take the table first and the schema second; both are bound.
const TABLE_XINFO: &str = "\
SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden \
FROM pragma_table_xinfo(?1, ?2) ORDER BY cid";

const INDEX_LIST: &str = "\
SELECT name, \"unique\", origin FROM pragma_index_list(?1, ?2) ORDER BY seq";

const INDEX_XINFO: &str = "\
SELECT seqno, cid, name FROM pragma_index_xinfo(?1, ?2) WHERE key = 1 ORDER BY seqno";

const FOREIGN_KEYS: &str = "\
SELECT id, seq, \"table\", \"from\", \"to\", on_update, on_delete \
FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq";

/// `SQLite` quotes identifiers with double quotes, doubling embedded quotes.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// One row of `pragma_table_xinfo`.
struct RawColumn {
    cid: i64,
    name: String,
    declared: String,
    not_null: bool,
    default: Option<String>,
    /// 1-based position in the primary key, 0 when not part of it.
    pk: i64,
    /// 1 for a hidden column of a virtual table.
    hidden: i64,
}

impl SqliteIntrospector<'_> {
    /// The stored name of a schema (`main`, `temp` or an attached database).
    /// Schema names cannot be bound, so they are checked against the list of
    /// databases before they are quoted into a statement.
    async fn schema_name(&self, schema: &str) -> Result<String> {
        let rows = fetch_all(
            self.conn,
            "database_by_name",
            DATABASE_BY_NAME,
            &[schema.into()],
        )
        .await?;
        if let Some(row) = rows.first() {
            return row.text(0);
        }
        // The temp schema is not listed until something is stored in it.
        if schema.eq_ignore_ascii_case("temp") {
            return Ok("temp".to_owned());
        }
        Err(Error::query(format!("unknown schema '{schema}'")))
    }

    async fn raw_columns(&self, schema: &str, table: &str) -> Result<Vec<RawColumn>> {
        let rows = fetch_all(
            self.conn,
            "table_xinfo",
            TABLE_XINFO,
            &[table.into(), schema.into()],
        )
        .await?;
        rows.iter()
            .map(|r| {
                Ok(RawColumn {
                    cid: r.int(0)?.unwrap_or(0),
                    name: r.text(1)?,
                    declared: r.text(2)?,
                    not_null: r.int(3)?.unwrap_or(0) != 0,
                    default: r.opt_text(4)?,
                    pk: r.int(5)?.unwrap_or(0),
                    hidden: r.int(6)?.unwrap_or(0),
                })
            })
            .collect()
    }

    /// The `sql` column of `sqlite_schema` for one object.
    async fn object_sql(&self, schema: &str, kind: &str, name: &str) -> Result<Option<String>> {
        let sql = format!(
            "SELECT sql FROM {}.sqlite_schema WHERE type = ?1 AND name = ?2 COLLATE NOCASE",
            quote_ident(schema)
        );
        let rows = fetch_all(self.conn, "source_sql", &sql, &[kind.into(), name.into()]).await?;
        Ok(rows.first().map(|r| r.opt_text(0)).transpose()?.flatten())
    }

    /// Key columns of an index, in order. Expressions come from the
    /// `CREATE INDEX` text; the pragma only says that there is one.
    async fn index_columns(&self, schema: &str, index: &str) -> Result<Vec<String>> {
        let rows = fetch_all(
            self.conn,
            "index_xinfo",
            INDEX_XINFO,
            &[index.into(), schema.into()],
        )
        .await?;
        let mut expressions: Option<Vec<String>> = None;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Some(name) = row.opt_text(2)? {
                out.push(name);
                continue;
            }
            let cid = row.int(1)?.unwrap_or(-2);
            if cid == -1 {
                out.push("rowid".to_owned());
                continue;
            }
            if expressions.is_none() {
                let sql = self.object_sql(schema, "index", index).await?;
                expressions = Some(sql.map(|s| sql_text::index_items(&s)).unwrap_or_default());
            }
            let seqno = usize::try_from(row.int(0)?.unwrap_or(0)).unwrap_or(0);
            let item = expressions.as_ref().and_then(|e| e.get(seqno)).cloned();
            out.push(item.unwrap_or_else(|| "<expression>".to_owned()));
        }
        Ok(out)
    }
}

#[async_trait]
impl Introspector for SqliteIntrospector<'_> {
    async fn current_schema(&self) -> Result<String> {
        Ok(MAIN.to_owned())
    }

    async fn schemas(&self) -> Result<Vec<SchemaInfo>> {
        let rows = fetch_all(self.conn, "databases", DATABASES, &[]).await?;
        let mut out: Vec<SchemaInfo> = rows
            .iter()
            .map(|r| {
                let name = r.text(0)?;
                Ok(SchemaInfo {
                    is_current: name == MAIN,
                    is_system: false,
                    name,
                    ..SchemaInfo::default()
                })
            })
            .collect::<Result<_>>()?;
        if !out.iter().any(|s| s.name == MAIN) {
            out.insert(
                0,
                SchemaInfo {
                    name: MAIN.to_owned(),
                    is_system: false,
                    is_current: true,
                    ..SchemaInfo::default()
                },
            );
        }
        Ok(out)
    }

    async fn objects(&self, schema: &str, kinds: &[ObjectKind]) -> Result<Vec<ObjectInfo>> {
        let schema = self.schema_name(schema).await?;
        // The internal tables (sqlite_sequence, sqlite_stat1, ...) are
        // hidden; automatic indexes are kept and point at their table.
        let sql = format!(
            "SELECT type, name, tbl_name FROM {}.sqlite_schema \
             WHERE type IN ('table', 'view', 'index', 'trigger') \
             AND NOT (type = 'table' AND name LIKE 'sqlite\\_%' ESCAPE '\\') \
             ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 1 WHEN 'index' THEN 2 ELSE 3 END, \
             name",
            quote_ident(&schema)
        );
        let rows = fetch_all(self.conn, "objects", &sql, &[]).await?;
        let mut out = Vec::new();
        for row in &rows {
            let (kind, has_parent) = match row.text(0)?.as_str() {
                "table" => (ObjectKind::Table, false),
                "view" => (ObjectKind::View, false),
                "index" => (ObjectKind::Index, true),
                "trigger" => (ObjectKind::Trigger, true),
                _ => continue,
            };
            if !wants(kinds, kind) {
                continue;
            }
            out.push(ObjectInfo {
                schema: schema.clone(),
                name: row.text(1)?,
                kind,
                parent: if has_parent { Some(row.text(2)?) } else { None },
                ..ObjectInfo::default()
            });
        }
        Ok(out)
    }

    async fn columns(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let schema = self.schema_name(schema).await?;
        let raw = self.raw_columns(&schema, table).await?;
        let pk: Vec<&RawColumn> = raw.iter().filter(|c| c.pk > 0).collect();
        // An INTEGER PRIMARY KEY of a rowid table is the rowid itself: it is
        // assigned automatically and can never be NULL.
        let mut rowid_alias = None;
        if let [only] = pk.as_slice()
            && only.declared.eq_ignore_ascii_case("INTEGER")
        {
            let sql = self.object_sql(&schema, "table", table).await?;
            let parsed = sql.map(|s| sql_text::parse_table(&s)).unwrap_or_default();
            if !parsed.without_rowid {
                rowid_alias = Some(only.cid);
            }
        }
        Ok(column_infos(&raw, rowid_alias, None))
    }

    async fn indexes(&self, schema: &str, table: &str) -> Result<Vec<IndexInfo>> {
        let schema = self.schema_name(schema).await?;
        let list = fetch_all(
            self.conn,
            "index_list",
            INDEX_LIST,
            &[table.into(), schema.as_str().into()],
        )
        .await?;
        let mut out = Vec::with_capacity(list.len());
        for row in &list {
            let name = row.text(0)?;
            out.push(IndexInfo {
                unique: row.int(1)?.unwrap_or(0) != 0,
                columns: self.index_columns(&schema, &name).await?,
                method: None,
                name,
                ..IndexInfo::default()
            });
        }
        Ok(out)
    }

    async fn constraints(&self, schema: &str, table: &str) -> Result<Vec<ConstraintInfo>> {
        let schema = self.schema_name(schema).await?;
        let raw = self.raw_columns(&schema, table).await?;
        let parsed = self
            .object_sql(&schema, "table", table)
            .await?
            .map(|s| sql_text::parse_table(&s))
            .unwrap_or_default();

        let list = fetch_all(
            self.conn,
            "index_list",
            INDEX_LIST,
            &[table.into(), schema.as_str().into()],
        )
        .await?;
        let mut unique = Vec::new();
        for row in &list {
            if row.text(2)? != "u" {
                continue;
            }
            let index = row.text(0)?;
            let columns = self.index_columns(&schema, &index).await?;
            unique.push((index, columns));
        }

        let fks = fetch_all(
            self.conn,
            "foreign_keys",
            FOREIGN_KEYS,
            &[table.into(), schema.as_str().into()],
        )
        .await?;
        let columns = fks
            .iter()
            .map(|row| {
                Ok((
                    row.int(0)?.unwrap_or(0),
                    row.text(2)?,
                    row.text(3)?,
                    row.opt_text(4)?,
                    row.opt_text(5)?,
                    row.opt_text(6)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let groups = foreign_key_groups(columns.into_iter());
        // `REFERENCES parent` without columns means parent's primary key.
        let mut parent_pks = HashMap::new();
        for group in &groups {
            if group.to.iter().all(Option::is_some) || parent_pks.contains_key(&group.table) {
                continue;
            }
            let parent = self.raw_columns(&schema, &group.table).await?;
            parent_pks.insert(group.table.clone(), primary_key(&parent));
        }
        Ok(build_constraints(
            &TableFacts {
                schema: &schema,
                table,
                raw: &raw,
                parsed: &parsed,
                unique: &unique,
                groups: &groups,
                parent_pks: &parent_pks,
            },
            false,
        ))
    }

    async fn source(&self, schema: &str, name: &str, kind: ObjectKind) -> Result<Option<String>> {
        let stored = match kind {
            ObjectKind::Table => "table",
            ObjectKind::View => "view",
            ObjectKind::Index => "index",
            ObjectKind::Trigger => "trigger",
            _ => return Ok(None),
        };
        let schema = self.schema_name(schema).await?;
        self.object_sql(&schema, stored, name).await
    }

    async fn load(&self, scope: &Scope, level: Level) -> Result<Snapshot> {
        change::load(self, scope, level).await
    }

    async fn changes(&self, since: &ChangeToken, scope: &Scope) -> Result<Delta> {
        change::changes(self, since, scope).await
    }
}

#[async_trait]
impl Stamper for SqliteIntrospector<'_> {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn schema_infos(&self) -> Result<Vec<SchemaInfo>> {
        self.schemas().await
    }

    async fn stamps(&self, info: &SchemaInfo, level: Level) -> Result<SchemaStamps> {
        load::stamps(self.conn, info, level).await
    }

    async fn stamped_read(
        &self,
        info: SchemaInfo,
        level: Level,
    ) -> Result<(SchemaStamps, SchemaSnapshot)> {
        load::stamped_read(self.conn, info, level).await
    }

    async fn read(
        &self,
        info: SchemaInfo,
        level: Level,
        units: Option<&Units>,
    ) -> Result<SchemaSnapshot> {
        load::read(self.conn, info, level, units).await
    }
}

/// A foreign key as `pragma_foreign_key_list` reports it, one row per column.
struct ForeignKey {
    id: i64,
    table: String,
    from: Vec<String>,
    to: Vec<Option<String>>,
    on_delete: Option<String>,
    on_update: Option<String>,
}

/// Group the rows of `pragma_foreign_key_list` (id, referenced table, from,
/// to, on_update, on_delete; ordered by id and column) into keys.
fn foreign_key_groups(
    rows: impl Iterator<
        Item = (
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >,
) -> Vec<ForeignKey> {
    let mut groups: Vec<ForeignKey> = Vec::new();
    for (id, table, from, to, on_update, on_delete) in rows {
        if groups.last().is_none_or(|g| g.id != id) {
            groups.push(ForeignKey {
                id,
                table,
                from: Vec::new(),
                to: Vec::new(),
                on_delete,
                on_update,
            });
        }
        let group = groups.last_mut().expect("a group was just pushed");
        group.from.push(from);
        group.to.push(to);
    }
    groups
}

/// The primary key columns of a table, in key order.
fn primary_key(raw: &[RawColumn]) -> Vec<String> {
    let mut pk: Vec<&RawColumn> = raw.iter().filter(|c| c.pk > 0).collect();
    pk.sort_by_key(|c| c.pk);
    pk.into_iter().map(|c| c.name.clone()).collect()
}

/// What SQLite tells about one table, gathered by the per-object reads and
/// by the bulk load alike.
struct TableFacts<'a> {
    schema: &'a str,
    table: &'a str,
    raw: &'a [RawColumn],
    parsed: &'a sql_text::Table,
    /// Indexes that back a `UNIQUE` constraint: name and key columns.
    unique: &'a [(String, Vec<String>)],
    groups: &'a [ForeignKey],
    parent_pks: &'a HashMap<String, Vec<String>>,
}

/// Constraints in the order the per-object read returns them: primary key,
/// unique, foreign keys, checks. SQLite does not store constraint names, so
/// they come from the `CREATE TABLE` text, or are made up (`<table>_pk`).
/// `extended` adds the optional fields.
fn build_constraints(f: &TableFacts<'_>, extended: bool) -> Vec<ConstraintInfo> {
    let TableFacts {
        schema,
        table,
        raw,
        parsed,
        ..
    } = *f;
    let named = |kind: ConstraintKind, columns: &[String]| -> Option<String> {
        parsed
            .constraints
            .iter()
            .find(|c| c.kind == kind && same_names(&c.columns, columns))
            .and_then(|c| c.name.clone())
    };
    let mut out = Vec::new();

    let columns = primary_key(raw);
    if !columns.is_empty() {
        out.push(ConstraintInfo {
            name: named(ConstraintKind::PrimaryKey, &columns)
                .unwrap_or_else(|| format!("{table}_pk")),
            kind: ConstraintKind::PrimaryKey,
            columns,
            ..ConstraintInfo::default()
        });
    }

    for (index, columns) in f.unique {
        out.push(ConstraintInfo {
            name: named(ConstraintKind::Unique, columns).unwrap_or_else(|| index.clone()),
            kind: ConstraintKind::Unique,
            columns: columns.clone(),
            ..ConstraintInfo::default()
        });
    }

    // SQLite numbers foreign keys in reverse order of declaration.
    let mut groups: Vec<&ForeignKey> = f.groups.iter().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.id));
    for group in groups {
        let ref_columns = if group.to.iter().all(Option::is_some) {
            group.to.iter().flatten().cloned().collect()
        } else {
            f.parent_pks.get(&group.table).cloned().unwrap_or_default()
        };
        let name = parsed
            .constraints
            .iter()
            .find(|c| {
                c.kind == ConstraintKind::ForeignKey
                    && same_names(&c.columns, &group.from)
                    && c.ref_table
                        .as_deref()
                        .is_some_and(|t| t.eq_ignore_ascii_case(&group.table))
            })
            .and_then(|c| c.name.clone())
            .unwrap_or_else(|| format!("{table}_fk{}", group.id));
        out.push(ConstraintInfo {
            name,
            kind: ConstraintKind::ForeignKey,
            columns: group.from.clone(),
            ref_schema: Some(schema.to_owned()),
            ref_table: Some(group.table.clone()),
            ref_columns,
            on_delete: group.on_delete.clone(),
            on_update: if extended {
                group.on_update.clone()
            } else {
                None
            },
            ..ConstraintInfo::default()
        });
    }

    let checks = parsed
        .constraints
        .iter()
        .filter(|c| c.kind == ConstraintKind::Check);
    for (n, c) in checks.enumerate() {
        let position = n + 1;
        out.push(ConstraintInfo {
            name: c
                .name
                .clone()
                .unwrap_or_else(|| format!("{table}_check{position}")),
            kind: ConstraintKind::Check,
            check: c.check.clone(),
            ..ConstraintInfo::default()
        });
    }
    out
}

/// Columns from the rows of `pragma_table_xinfo`; hidden columns of virtual
/// tables are left out. `generated` (the generated columns of the `CREATE
/// TABLE` text) adds the optional fields.
fn column_infos(
    raw: &[RawColumn],
    rowid_alias: Option<i64>,
    generated: Option<&[sql_text::Generated]>,
) -> Vec<ColumnInfo> {
    raw.iter()
        .filter(|c| c.hidden != 1)
        .map(|c| {
            let alias = rowid_alias == Some(c.cid);
            ColumnInfo {
                name: c.name.clone(),
                position: u32::try_from(c.cid + 1).unwrap_or(0),
                data_type: c.declared.clone(),
                nullable: !c.not_null && !alias,
                default: c.default.clone(),
                auto_increment: alias,
                generated: generated.and_then(|all| {
                    // 2: generated and virtual, 3: generated and stored.
                    (c.hidden >= 2).then(|| GeneratedColumn {
                        expression: all
                            .iter()
                            .find(|g| g.column.eq_ignore_ascii_case(&c.name))
                            .map(|g| g.expression.clone()),
                        stored: c.hidden == 3,
                    })
                }),
                ..ColumnInfo::default()
            }
        })
        .collect()
}

fn same_names(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Reading the text of `CREATE TABLE` and `CREATE INDEX` statements. SQLite
/// does not expose constraint names, `CHECK` conditions or index expressions
/// any other way. This is a best-effort reader of the documented syntax
/// (<https://www.sqlite.org/lang_createtable.html>), not a full parser.
mod sql_text {
    use super::ConstraintKind;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Kind {
        /// An unquoted word: keyword or name.
        Word(String),
        /// A quoted identifier, unquoted here.
        Ident(String),
        /// A string literal or a number.
        Literal,
        Punct(char),
    }

    #[derive(Debug, Clone)]
    struct Token {
        kind: Kind,
        start: usize,
        end: usize,
    }

    impl Token {
        fn is_word(&self, keyword: &str) -> bool {
            matches!(&self.kind, Kind::Word(w) if w.eq_ignore_ascii_case(keyword))
        }

        fn is_punct(&self, c: char) -> bool {
            self.kind == Kind::Punct(c)
        }

        /// The identifier this token spells, quoted or not.
        fn name(&self) -> Option<&str> {
            match &self.kind {
                Kind::Word(s) | Kind::Ident(s) => Some(s),
                _ => None,
            }
        }
    }

    fn tokenize(sql: &str) -> Vec<Token> {
        let chars: Vec<(usize, char)> = sql.char_indices().collect();
        let end_of = |i: usize| chars.get(i).map_or(sql.len(), |(p, _)| *p);
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let (start, c) = chars[i];
            if c.is_whitespace() {
                i += 1;
            } else if c == '-' && chars.get(i + 1).is_some_and(|(_, n)| *n == '-') {
                while i < chars.len() && chars[i].1 != '\n' {
                    i += 1;
                }
            } else if c == '/' && chars.get(i + 1).is_some_and(|(_, n)| *n == '*') {
                i += 2;
                while i < chars.len()
                    && !(chars[i].1 == '*' && chars.get(i + 1).is_some_and(|(_, n)| *n == '/'))
                {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            } else if c == '\'' || c == '"' || c == '`' {
                // Quote characters are doubled to include them in the text.
                let mut text = String::new();
                i += 1;
                while i < chars.len() {
                    if chars[i].1 == c {
                        if chars.get(i + 1).is_some_and(|(_, n)| *n == c) {
                            text.push(c);
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    text.push(chars[i].1);
                    i += 1;
                }
                i += 1;
                let kind = if c == '\'' {
                    Kind::Literal
                } else {
                    Kind::Ident(text)
                };
                out.push(Token {
                    kind,
                    start,
                    end: end_of(i),
                });
            } else if c == '[' {
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i].1 != ']' {
                    text.push(chars[i].1);
                    i += 1;
                }
                i += 1;
                out.push(Token {
                    kind: Kind::Ident(text),
                    start,
                    end: end_of(i),
                });
            } else if c.is_ascii_digit() {
                while i < chars.len()
                    && (chars[i].1.is_ascii_alphanumeric() || matches!(chars[i].1, '.' | '_'))
                {
                    i += 1;
                }
                out.push(Token {
                    kind: Kind::Literal,
                    start,
                    end: end_of(i),
                });
            } else if c.is_alphabetic() || c == '_' || !c.is_ascii() {
                i += 1;
                while i < chars.len()
                    && (chars[i].1.is_alphanumeric()
                        || matches!(chars[i].1, '_' | '$')
                        || !chars[i].1.is_ascii())
                {
                    i += 1;
                }
                let end = end_of(i);
                out.push(Token {
                    kind: Kind::Word(sql[start..end].to_owned()),
                    start,
                    end,
                });
            } else {
                i += 1;
                out.push(Token {
                    kind: Kind::Punct(c),
                    start,
                    end: end_of(i),
                });
            }
        }
        out
    }

    /// The index of the `)` that closes the `(` at `open`.
    fn closing(tokens: &[Token], open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (i, t) in tokens.iter().enumerate().skip(open) {
            if t.is_punct('(') {
                depth += 1;
            } else if t.is_punct(')') {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Split at commas that are not inside parentheses.
    fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut from = 0;
        for (i, t) in tokens.iter().enumerate() {
            if t.is_punct('(') {
                depth += 1;
            } else if t.is_punct(')') {
                depth = depth.saturating_sub(1);
            } else if t.is_punct(',') && depth == 0 {
                parts.push(&tokens[from..i]);
                from = i + 1;
            }
        }
        parts.push(&tokens[from..]);
        parts.retain(|p| !p.is_empty());
        parts
    }

    /// Names in a parenthesized column list, ignoring `COLLATE`/`ASC`/`DESC`.
    fn name_list(tokens: &[Token]) -> Vec<String> {
        split_commas(tokens)
            .iter()
            .filter_map(|part| part.first().and_then(Token::name).map(str::to_owned))
            .collect()
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct Constraint {
        pub name: Option<String>,
        pub kind: ConstraintKind,
        pub columns: Vec<String>,
        pub ref_table: Option<String>,
        pub check: Option<String>,
    }

    /// A generated column as the `CREATE TABLE` text spells it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct Generated {
        pub column: String,
        pub expression: String,
        pub stored: bool,
    }

    #[derive(Debug, Default)]
    pub(super) struct Table {
        pub constraints: Vec<Constraint>,
        pub generated: Vec<Generated>,
        pub without_rowid: bool,
    }

    /// Constraints and table options of a `CREATE TABLE` statement; empty for
    /// anything else (views, virtual tables).
    pub(super) fn parse_table(sql: &str) -> Table {
        let tokens = tokenize(sql);
        let mut table = Table::default();
        let Some(open) = tokens.iter().position(|t| t.is_punct('(')) else {
            return table;
        };
        let header = &tokens[..open];
        if !header.first().is_some_and(|t| t.is_word("CREATE"))
            || header.iter().any(|t| t.is_word("VIRTUAL"))
            || !header.iter().any(|t| t.is_word("TABLE"))
        {
            return table;
        }
        let Some(close) = closing(&tokens, open) else {
            return table;
        };
        for item in split_commas(&tokens[open + 1..close]) {
            let first = &item[0];
            let table_level = ["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"]
                .iter()
                .any(|k| first.is_word(k));
            if table_level {
                table_constraint(sql, item, &mut table.constraints);
            } else {
                column_constraints(sql, item, &mut table.constraints, &mut table.generated);
            }
        }
        let options = &tokens[close + 1..];
        table.without_rowid = options
            .windows(2)
            .any(|w| w[0].is_word("WITHOUT") && w[1].is_word("ROWID"));
        table
    }

    /// `[CONSTRAINT name] PRIMARY KEY (...) | UNIQUE (...) | CHECK (...) |
    /// FOREIGN KEY (...) REFERENCES ...`
    fn table_constraint(sql: &str, item: &[Token], out: &mut Vec<Constraint>) {
        let mut i = 0;
        let mut name = None;
        if item[0].is_word("CONSTRAINT") {
            name = item.get(1).and_then(Token::name).map(str::to_owned);
            i = 2;
        }
        let Some(head) = item.get(i) else { return };
        let group = |from: usize| -> Option<(usize, usize)> {
            let open = from + item[from..].iter().position(|t| t.is_punct('('))?;
            Some((open, closing(item, open)?))
        };
        if head.is_word("PRIMARY") || head.is_word("UNIQUE") {
            let kind = if head.is_word("PRIMARY") {
                ConstraintKind::PrimaryKey
            } else {
                ConstraintKind::Unique
            };
            if let Some((open, close)) = group(i) {
                out.push(Constraint {
                    name,
                    kind,
                    columns: name_list(&item[open + 1..close]),
                    ref_table: None,
                    check: None,
                });
            }
        } else if head.is_word("CHECK") {
            if let Some((open, close)) = group(i) {
                out.push(check(sql, name, &item[open], &item[close]));
            }
        } else if head.is_word("FOREIGN")
            && let Some((open, close)) = group(i)
        {
            let columns = name_list(&item[open + 1..close]);
            let (ref_table, _) = references(&item[close + 1..]);
            out.push(Constraint {
                name,
                kind: ConstraintKind::ForeignKey,
                columns,
                ref_table,
                check: None,
            });
        }
    }

    fn check(sql: &str, name: Option<String>, open: &Token, close: &Token) -> Constraint {
        Constraint {
            name,
            kind: ConstraintKind::Check,
            columns: Vec::new(),
            ref_table: None,
            check: Some(sql[open.end..close.start].trim().to_owned()),
        }
    }

    /// The table (and optional column list) after `REFERENCES`.
    fn references(tokens: &[Token]) -> (Option<String>, Vec<String>) {
        let Some(at) = tokens.iter().position(|t| t.is_word("REFERENCES")) else {
            return (None, Vec::new());
        };
        let table = tokens.get(at + 1).and_then(Token::name).map(str::to_owned);
        let columns = match tokens.get(at + 2) {
            Some(t) if t.is_punct('(') => closing(tokens, at + 2)
                .map(|close| name_list(&tokens[at + 3..close]))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        (table, columns)
    }

    /// Column-level `PRIMARY KEY`, `UNIQUE`, `CHECK` and `REFERENCES`.
    fn column_constraints(
        sql: &str,
        item: &[Token],
        out: &mut Vec<Constraint>,
        generated: &mut Vec<Generated>,
    ) {
        let Some(column) = item[0].name().map(str::to_owned) else {
            return;
        };
        let mut pending: Option<String> = None;
        let mut i = 1;
        while i < item.len() {
            let t = &item[i];
            if t.is_word("AS")
                && item.get(i + 1).is_some_and(|n| n.is_punct('('))
                && let Some(close) = closing(item, i + 1)
            {
                // `[GENERATED ALWAYS] AS (expr) [STORED | VIRTUAL]`.
                let stored = item.get(close + 1).is_some_and(|n| n.is_word("STORED"));
                generated.push(Generated {
                    column: column.clone(),
                    expression: sql[item[i + 1].end..item[close].start].trim().to_owned(),
                    stored,
                });
                pending = None;
                i = close + 1;
                continue;
            }
            if t.is_punct('(') {
                // Type sizes, DEFAULT (...), AS (...): nothing to read inside.
                i = closing(item, i).map_or(item.len(), |c| c + 1);
                continue;
            }
            if t.is_word("CONSTRAINT") {
                pending = item.get(i + 1).and_then(Token::name).map(str::to_owned);
                i += 2;
                continue;
            }
            if t.is_word("PRIMARY") || t.is_word("UNIQUE") {
                out.push(Constraint {
                    name: pending.take(),
                    kind: if t.is_word("PRIMARY") {
                        ConstraintKind::PrimaryKey
                    } else {
                        ConstraintKind::Unique
                    },
                    columns: vec![column.clone()],
                    ref_table: None,
                    check: None,
                });
            } else if t.is_word("CHECK") && item.get(i + 1).is_some_and(|n| n.is_punct('(')) {
                if let Some(close) = closing(item, i + 1) {
                    out.push(check(sql, pending.take(), &item[i + 1], &item[close]));
                    i = close + 1;
                    continue;
                }
            } else if t.is_word("REFERENCES") {
                let (ref_table, _) = references(&item[i..]);
                out.push(Constraint {
                    name: pending.take(),
                    kind: ConstraintKind::ForeignKey,
                    columns: vec![column.clone()],
                    ref_table,
                    check: None,
                });
            } else {
                // NOT NULL, DEFAULT, COLLATE, GENERATED ...: a name given
                // before them belongs to them, not to a later keyword.
                pending = None;
            }
            i += 1;
        }
    }

    /// `CREATE VIRTUAL TABLE ...`.
    pub(super) fn is_virtual_table(sql: &str) -> bool {
        let tokens = tokenize(sql);
        tokens.first().is_some_and(|t| t.is_word("CREATE"))
            && tokens.get(1).is_some_and(|t| t.is_word("VIRTUAL"))
    }

    /// The condition of a partial index: the text after the `WHERE` that
    /// follows the column list of `CREATE INDEX`.
    pub(super) fn index_predicate(sql: &str) -> Option<String> {
        let tokens = tokenize(sql);
        let on = tokens.iter().position(|t| t.is_word("ON"))?;
        let open = on + tokens[on..].iter().position(|t| t.is_punct('('))?;
        let close = closing(&tokens, open)?;
        let at = close
            + 1
            + tokens[close + 1..]
                .iter()
                .position(|t| t.is_word("WHERE"))?;
        let text = sql[tokens[at].end..].trim().trim_end_matches(';').trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// What `CREATE TRIGGER` says about when the trigger fires.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub(super) struct Trigger {
        pub timing: Option<String>,
        pub events: Vec<String>,
        pub condition: Option<String>,
    }

    /// `CREATE [TEMP] TRIGGER [IF NOT EXISTS] name [BEFORE | AFTER | INSTEAD
    /// OF] DELETE | INSERT | UPDATE [OF columns] ON table [FOR EACH ROW]
    /// [WHEN expr] BEGIN ... END`. A trigger without a time is a `BEFORE`
    /// trigger.
    pub(super) fn parse_trigger(sql: &str) -> Trigger {
        let tokens = tokenize(sql);
        let mut out = Trigger::default();
        let Some(at) = tokens.iter().position(|t| t.is_word("TRIGGER")) else {
            return out;
        };
        let mut i = at + 1;
        if tokens.get(i).is_some_and(|t| t.is_word("IF")) {
            i += 3; // IF NOT EXISTS
        }
        // Skip the (possibly schema-qualified) name.
        i += 1;
        if tokens.get(i).is_some_and(|t| t.is_punct('.')) {
            i += 2;
        }
        let mut timing = "BEFORE";
        match tokens.get(i) {
            Some(t) if t.is_word("BEFORE") => i += 1,
            Some(t) if t.is_word("AFTER") => {
                timing = "AFTER";
                i += 1;
            }
            Some(t) if t.is_word("INSTEAD") => {
                timing = "INSTEAD OF";
                i += 2;
            }
            _ => {}
        }
        out.timing = Some(timing.to_owned());
        let i = i.min(tokens.len());
        if let Some(t) = tokens.get(i)
            && let Kind::Word(w) = &t.kind
        {
            out.events.push(w.to_ascii_uppercase());
        }
        // A `WHEN` in the body (`CASE WHEN`) is not the trigger's condition.
        let begin = tokens[i..]
            .iter()
            .position(|t| t.is_word("BEGIN"))
            .map_or(tokens.len(), |p| i + p);
        if let Some(when) = tokens[i..begin].iter().position(|t| t.is_word("WHEN")) {
            let from = tokens[i + when].end;
            let to = tokens.get(begin).map_or(sql.len(), |t| t.start);
            let text = sql[from..to].trim();
            if !text.is_empty() {
                out.condition = Some(text.to_owned());
            }
        }
        out
    }

    /// The comma-separated items between the parentheses of `CREATE INDEX`,
    /// without `ASC`, `DESC` and `COLLATE` suffixes.
    pub(super) fn index_items(sql: &str) -> Vec<String> {
        let tokens = tokenize(sql);
        let Some(on) = tokens.iter().position(|t| t.is_word("ON")) else {
            return Vec::new();
        };
        let Some(open) = tokens[on..]
            .iter()
            .position(|t| t.is_punct('('))
            .map(|p| p + on)
        else {
            return Vec::new();
        };
        let Some(close) = closing(&tokens, open) else {
            return Vec::new();
        };
        split_commas(&tokens[open + 1..close])
            .into_iter()
            .map(|mut part| {
                loop {
                    match part {
                        [rest @ .., t] if t.is_word("ASC") || t.is_word("DESC") => part = rest,
                        [rest @ .., c, _] if c.is_word("COLLATE") => part = rest,
                        _ => break,
                    }
                }
                match (part.first(), part.last()) {
                    (Some(a), Some(b)) => sql[a.start..b.end].to_owned(),
                    _ => String::new(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::sql_text::{index_items, parse_table};
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote_ident("main"), "\"main\"");
        assert_eq!(quote_ident("we\"ird name"), "\"we\"\"ird name\"");
    }

    #[test]
    fn reads_table_and_column_constraints() {
        let t = parse_table(
            "CREATE TABLE \"My Table\" (\n\
               id INTEGER CONSTRAINT pk_my PRIMARY KEY AUTOINCREMENT,\n\
               email TEXT NOT NULL CONSTRAINT uq_email UNIQUE,\n\
               \"Parent Id\" INTEGER REFERENCES parent(id) ON DELETE CASCADE,\n\
               qty INTEGER DEFAULT (1 + 2) CHECK (qty > 0), -- trailing note\n\
               note TEXT DEFAULT 'CHECK (x)',\n\
               a INTEGER, b INTEGER,\n\
               CONSTRAINT ab_unique UNIQUE (a, b),\n\
               CONSTRAINT fk_ab FOREIGN KEY (a, b) REFERENCES other (x, y),\n\
               CONSTRAINT big CHECK (qty < (100 * 2) AND note <> ')')\n\
             ) WITHOUT ROWID",
        );
        assert!(t.without_rowid);
        let by = |kind: ConstraintKind| -> Vec<_> {
            t.constraints.iter().filter(|c| c.kind == kind).collect()
        };
        let pks = by(ConstraintKind::PrimaryKey);
        assert_eq!(pks.len(), 1);
        assert_eq!(pks[0].name.as_deref(), Some("pk_my"));
        assert_eq!(pks[0].columns, ["id"]);

        let uniques = by(ConstraintKind::Unique);
        assert_eq!(uniques.len(), 2);
        assert_eq!(uniques[0].name.as_deref(), Some("uq_email"));
        assert_eq!(uniques[0].columns, ["email"]);
        assert_eq!(uniques[1].name.as_deref(), Some("ab_unique"));
        assert_eq!(uniques[1].columns, ["a", "b"]);

        let fks = by(ConstraintKind::ForeignKey);
        assert_eq!(fks.len(), 2);
        assert_eq!(fks[0].name, None);
        assert_eq!(fks[0].columns, ["Parent Id"]);
        assert_eq!(fks[0].ref_table.as_deref(), Some("parent"));
        assert_eq!(fks[1].name.as_deref(), Some("fk_ab"));
        assert_eq!(fks[1].columns, ["a", "b"]);
        assert_eq!(fks[1].ref_table.as_deref(), Some("other"));

        let checks = by(ConstraintKind::Check);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].name, None);
        assert_eq!(checks[0].check.as_deref(), Some("qty > 0"));
        assert_eq!(checks[1].name.as_deref(), Some("big"));
        assert_eq!(
            checks[1].check.as_deref(),
            Some("qty < (100 * 2) AND note <> ')'")
        );
    }

    #[test]
    fn a_name_belongs_to_the_next_constraint_only() {
        let t = parse_table(
            "CREATE TABLE t (a TEXT CONSTRAINT nn NOT NULL UNIQUE, b INT CHECK (b > 1))",
        );
        assert_eq!(t.constraints.len(), 2);
        assert_eq!(t.constraints[0].kind, ConstraintKind::Unique);
        assert_eq!(t.constraints[0].name, None);
        assert_eq!(t.constraints[1].name, None);
        assert!(!t.without_rowid);
    }

    #[test]
    fn ignores_what_is_not_a_plain_table() {
        assert!(
            parse_table("CREATE VIEW v AS SELECT 1")
                .constraints
                .is_empty()
        );
        assert!(
            parse_table("CREATE VIRTUAL TABLE f USING fts5(a, b)")
                .constraints
                .is_empty()
        );
        assert!(parse_table("").constraints.is_empty());
        // Truncated text does not panic.
        let _ = parse_table("CREATE TABLE t (a CHECK (");
    }

    #[test]
    fn index_expressions() {
        assert_eq!(
            index_items(
                "CREATE UNIQUE INDEX \"My Idx\" ON \"My Table\" (lower(name) COLLATE NOCASE, \
                 \"Mixed Case\" DESC, a + b, price)"
            ),
            ["lower(name)", "\"Mixed Case\"", "a + b", "price"]
        );
        assert!(index_items("CREATE TABLE t(a)").is_empty());
    }
}
