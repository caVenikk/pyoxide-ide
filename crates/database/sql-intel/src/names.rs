//! Names as the tree has them: the parts of a qualified reference, folded by
//! the dialect, and the names a dialect knows without a model.

use std::ops::Range;

use sql_syntax::{Dialect, SyntaxKind, SyntaxNode, TokenKind};

/// One part of a qualified name (`s`, `t` or `c` of `s.t.c`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    /// The name as written, quotes included; empty for a missing name.
    pub written: String,
    /// The catalog name ([`Dialect::fold`]).
    pub name: String,
    pub range: Range<usize>,
    /// The node of this part (`SchemaRef`, `TableRef`, ...).
    pub kind: SyntaxKind,
    pub quoted: bool,
    /// The name is missing (a syntax error) or stands for a hole.
    pub broken: bool,
}

/// The parts of a name node, the outermost qualifier first:
/// `ColumnRef(TableRef(SchemaRef(s) . t) . c)` gives `s`, `t`, `c`.
pub(crate) fn parts(node: SyntaxNode<'_>, dialect: Dialect) -> Vec<Part> {
    let mut out = Vec::with_capacity(3);
    collect_parts(node, dialect, &mut out);
    out
}

fn collect_parts(node: SyntaxNode<'_>, dialect: Dialect, out: &mut Vec<Part>) {
    let mut own: Option<Part> = None;
    for child in node.children() {
        if let Some(n) = child.as_node() {
            match n.kind() {
                k if k.is_name() && own.is_none() => collect_parts(n, dialect, out),
                SyntaxKind::Missing | SyntaxKind::Hole | SyntaxKind::Error => {
                    own = Some(Part {
                        written: String::new(),
                        name: String::new(),
                        range: n.range(),
                        kind: node.kind(),
                        quoted: false,
                        broken: true,
                    });
                }
                _ => {}
            }
        } else if let Some(t) = child.as_token() {
            match t.kind() {
                TokenKind::Word | TokenKind::QuotedIdent => {
                    let quoted = t.kind() == TokenKind::QuotedIdent;
                    let written = t.text().to_string();
                    match &mut own {
                        // A hole inside a word: `orders_<hole>` is two tokens.
                        Some(p) if p.range.end == t.range().start => {
                            p.broken = true;
                            p.range.end = t.range().end;
                        }
                        // `AS name`: the last word is the name.
                        _ => {
                            own = Some(Part {
                                name: dialect.fold(&written),
                                written,
                                range: t.range(),
                                kind: node.kind(),
                                quoted,
                                broken: false,
                            })
                        }
                    }
                }
                TokenKind::Hole => match &mut own {
                    Some(p) if p.range.end == t.range().start => {
                        p.broken = true;
                        p.range.end = t.range().end;
                    }
                    _ => {
                        own = Some(Part {
                            written: String::new(),
                            name: String::new(),
                            range: t.range(),
                            kind: node.kind(),
                            quoted: false,
                            broken: true,
                        })
                    }
                },
                _ => {}
            }
        }
    }
    if let Some(p) = own {
        out.push(p);
    }
}

/// The node contains a hole anywhere.
pub(crate) fn has_hole(node: SyntaxNode<'_>) -> bool {
    node.preorder().any(|e| match e {
        sql_syntax::SyntaxElement::Node(n) => n.kind() == SyntaxKind::Hole,
        sql_syntax::SyntaxElement::Token(t) => t.kind() == TokenKind::Hole,
    })
}

/// The node contains a syntax error.
pub(crate) fn has_error(node: SyntaxNode<'_>) -> bool {
    node.descendants()
        .any(|n| matches!(n.kind(), SyntaxKind::Error | SyntaxKind::Missing))
}

/// Two catalog names denote the same object in `dialect`.
pub(crate) fn same(dialect: Dialect, a: &str, b: &str) -> bool {
    dialect.names_equal(a, b)
}

/// A key for hashing names: equal for names [`same`] calls equal.
pub(crate) fn key(dialect: Dialect, name: &str) -> String {
    match dialect {
        Dialect::Sqlite => name.to_ascii_lowercase(),
        _ => name.to_string(),
    }
}

/// Names that stand for a value without a table in every dialect (written
/// as columns: `current_date`, `user`).
const NILADIC: &[&str] = &[
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "localtime",
    "localtimestamp",
    "session_user",
    "system_user",
    "user",
];

/// Oracle's pseudo-columns and value functions written without
/// parentheses.
const ORACLE_PSEUDO: &[&str] = &[
    "connect_by_iscycle",
    "connect_by_isleaf",
    "currval",
    "dbtimezone",
    "level",
    "nextval",
    "ora_rowscn",
    "rowid",
    "rownum",
    "sessiontimezone",
    "sysdate",
    "systimestamp",
    "uid",
];

/// A name written where a column may stand that the dialect knows by
/// itself.
pub(crate) fn is_builtin_value(dialect: Dialect, written: &str) -> bool {
    let lower = written.to_ascii_lowercase();
    NILADIC.contains(&lower.as_str())
        || (dialect == Dialect::Oracle && ORACLE_PSEUDO.contains(&lower.as_str()))
}

/// Columns every table has without listing them: PostgreSQL's system
/// columns, SQLite's row id, Oracle's `ROWID`.
pub(crate) fn is_system_column(dialect: Dialect, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let list: &[&str] = match dialect {
        Dialect::Postgres => &["ctid", "xmin", "xmax", "cmin", "cmax", "tableoid"],
        Dialect::Sqlite => &["rowid", "oid", "_rowid_"],
        Dialect::Oracle => &["rowid", "ora_rowscn"],
        Dialect::Generic => &[],
    };
    list.contains(&lower.as_str())
}

/// Tables and views of the database's own catalog that a model usually
/// does not contain: unknown, not unresolved, unless the model has the
/// catalog schema.
pub(crate) fn is_catalog_relation(dialect: Dialect, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match dialect {
        Dialect::Postgres => lower.starts_with("pg_"),
        Dialect::Oracle => {
            ["all_", "user_", "dba_", "cdb_", "v$", "gv$", "v_$", "gv_$"]
                .iter()
                .any(|p| lower.starts_with(p))
                || matches!(lower.as_str(), "dictionary" | "dict" | "tab" | "cat")
        }
        Dialect::Sqlite => false,
        Dialect::Generic => false,
    }
}

/// Tables a dialect has in every database: Oracle's `DUAL`, SQLite's
/// schema tables.
pub(crate) fn is_builtin_relation(dialect: Dialect, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match dialect {
        Dialect::Oracle => lower == "dual",
        Dialect::Sqlite => matches!(
            lower.as_str(),
            "sqlite_master"
                | "sqlite_schema"
                | "sqlite_temp_master"
                | "sqlite_temp_schema"
                | "sqlite_sequence"
                | "sqlite_stat1"
        ),
        _ => false,
    }
}

/// Schemas a dialect always has, whose objects a model usually lacks.
pub(crate) fn is_catalog_schema(dialect: Dialect, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match dialect {
        Dialect::Postgres => matches!(lower.as_str(), "pg_catalog" | "information_schema"),
        Dialect::Oracle => matches!(lower.as_str(), "sys" | "system"),
        Dialect::Sqlite => matches!(lower.as_str(), "main" | "temp"),
        Dialect::Generic => false,
    }
}

/// The edit distance of two names (Levenshtein on chars, ASCII letters
/// compared ignoring case), capped: `None` beyond `max`.
pub(crate) fn distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().map(|c| c.to_ascii_lowercase()).collect();
    let b: Vec<char> = b.chars().map(|c| c.to_ascii_lowercase()).collect();
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let d = prev[b.len()];
    (d <= max).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(distance("custmers", "customers", 2), Some(1));
        assert_eq!(distance("ORDERS", "orders", 2), Some(0));
        assert_eq!(distance("abc", "xyz", 2), None);
        assert_eq!(distance("a", "abcd", 2), None);
    }

    #[test]
    fn parts_of_names() {
        let s = sql_syntax::parse("select s.t.c, \"X\".y from s.t", Dialect::Postgres);
        let refs: Vec<Vec<String>> = s
            .statements()
            .flat_map(|r| r.descendants().collect::<Vec<_>>())
            .filter(|n| n.kind() == SyntaxKind::ColumnRef)
            .map(|n| {
                parts(n, Dialect::Postgres)
                    .into_iter()
                    .map(|p| p.name)
                    .collect()
            })
            .collect();
        assert_eq!(refs, [vec!["s", "t", "c"], vec!["X", "y"]]);
    }
}
