//! The structure of a script (the Structure view): its statements with
//! their kind and name, and the members of tables, routines and packages.

use std::ops::Range;

use code_intel::SymbolKind;
use sql_syntax::{Dialect, Script, SyntaxKind as K, SyntaxNode};

use crate::names::parts;

/// A symbol of the structure; ranges are byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Symbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub range: Range<usize>,
    pub selection: Range<usize>,
    pub children: Vec<Symbol>,
}

/// How much of the tables a query reads its detail shows.
const MAX_DETAIL: usize = 60;

const NAME_NODES: [K; 5] = [
    K::TableRef,
    K::FunctionRef,
    K::ObjectRef,
    K::SchemaRef,
    K::DefName,
];

pub(crate) fn symbols(script: &Script) -> Vec<Symbol> {
    let dialect = script.dialect();
    script
        .statements()
        .filter_map(|root| root.child_nodes().next())
        .filter_map(|stmt| statement(script, dialect, stmt))
        .collect()
}

fn name_node<'a>(node: SyntaxNode<'a>) -> Option<SyntaxNode<'a>> {
    node.child_nodes().find(|n| NAME_NODES.contains(&n.kind()))
}

/// The name a node gives, as written, and where its last part is.
fn written(
    script: &Script,
    dialect: Dialect,
    name: SyntaxNode<'_>,
) -> Option<(String, Range<usize>)> {
    let ps = parts(name, dialect);
    let last = ps.last().filter(|p| !p.broken)?;
    let text = script.text().get(name.range())?.trim().to_string();
    Some((text, last.range.clone()))
}

fn symbol(
    name: String,
    detail: Option<&str>,
    kind: SymbolKind,
    node: SyntaxNode<'_>,
    selection: Range<usize>,
) -> Symbol {
    Symbol {
        name,
        detail: detail.map(str::to_string),
        kind,
        range: node.range(),
        selection,
        children: Vec::new(),
    }
}

fn statement(script: &Script, dialect: Dialect, stmt: SyntaxNode<'_>) -> Option<Symbol> {
    let detail_kind = match stmt.kind() {
        K::CreateTable => Some(("table", SymbolKind::Struct)),
        K::CreateView => Some(("view", SymbolKind::Interface)),
        K::CreateMaterializedView => Some(("materialized view", SymbolKind::Interface)),
        K::CreateFunction => Some(("function", SymbolKind::Function)),
        K::CreateProcedure => Some(("procedure", SymbolKind::Method)),
        K::CreateTrigger => Some(("trigger", SymbolKind::Event)),
        K::CreatePackage => Some(("package", SymbolKind::Package)),
        K::CreatePackageBody => Some(("package body", SymbolKind::Package)),
        K::CreateType => Some(("type", SymbolKind::Class)),
        K::CreateSequence => Some(("sequence", SymbolKind::Number)),
        K::CreateSchema => Some(("schema", SymbolKind::Namespace)),
        K::CreateIndex => Some(("index", SymbolKind::Key)),
        _ => None,
    };
    if let Some((detail, kind)) = detail_kind {
        let (name, selection) = written(script, dialect, name_node(stmt)?)?;
        let mut s = symbol(name, Some(detail), kind, stmt, selection);
        s.children = match stmt.kind() {
            K::CreateTable => columns(script, dialect, stmt),
            K::CreateFunction | K::CreateProcedure | K::CreatePackage | K::CreatePackageBody => {
                members(script, dialect, stmt)
            }
            _ => Vec::new(),
        };
        return Some(s);
    }
    match stmt.kind() {
        K::Query | K::Select | K::SetOperation | K::ParenQuery | K::Values | K::TableQuery => {
            let selection = first_word(stmt)?;
            let mut s = symbol("select".into(), None, SymbolKind::Text, stmt, selection);
            s.detail = tables_read(script, stmt);
            s.children = ctes(script, dialect, stmt);
            Some(s)
        }
        K::Insert | K::Update | K::Delete | K::Merge => {
            let target = stmt
                .descendants()
                .find(|n| n.kind() == K::TableRef)
                .and_then(|t| written(script, dialect, t));
            let verb = match stmt.kind() {
                K::Insert => "insert into",
                K::Update => "update",
                K::Delete => "delete from",
                _ => "merge into",
            };
            let (name, selection) = match target {
                Some((t, r)) => (format!("{verb} {t}"), r),
                None => (verb.to_string(), first_word(stmt)?),
            };
            Some(symbol(name, None, SymbolKind::Event, stmt, selection))
        }
        K::AlterTable | K::AlterObject | K::DropStatement | K::Truncate | K::CommentStatement => {
            let selection = first_word(stmt)?;
            let end = name_node(stmt)
                .or_else(|| stmt.child_nodes().find(|n| n.kind() == K::TableRef))
                .map_or(selection.end, |n| n.range().end);
            let text = script.text().get(stmt.range().start..end)?;
            let name = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let sel = name_node(stmt)
                .or_else(|| stmt.child_nodes().find(|n| n.kind() == K::TableRef))
                .and_then(|n| written(script, dialect, n))
                .map_or(selection, |(_, r)| r);
            Some(symbol(name, None, SymbolKind::Text, stmt, sel))
        }
        K::Block | K::DoStatement => {
            let selection = first_word(stmt)?;
            let mut s = symbol(
                "anonymous block".into(),
                None,
                SymbolKind::Module,
                stmt,
                selection,
            );
            s.children = members(script, dialect, stmt);
            Some(s)
        }
        _ => None,
    }
}

fn first_word(node: SyntaxNode<'_>) -> Option<Range<usize>> {
    node.tokens().next().map(|t| t.range())
}

fn columns(script: &Script, dialect: Dialect, table: SyntaxNode<'_>) -> Vec<Symbol> {
    table
        .child_nodes()
        .filter(|c| c.kind() == K::ColumnDef)
        .filter_map(|c| {
            let (name, selection) = written(script, dialect, c.child(K::DefName)?)?;
            let ty = c
                .child(K::TypeRef)
                .and_then(|t| script.text().get(t.range()))
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
            let mut s = symbol(name, ty.as_deref(), SymbolKind::Field, c, selection);
            s.range = c.range();
            Some(s)
        })
        .collect()
}

/// Subprograms, cursors, types, exceptions and variables a routine, a
/// package or a block declares.
fn members(script: &Script, dialect: Dialect, node: SyntaxNode<'_>) -> Vec<Symbol> {
    let mut out = Vec::new();
    for c in node.child_nodes() {
        match c.kind() {
            K::DeclareSection => out.extend(members(script, dialect, c)),
            K::Block => out.extend(members(script, dialect, c)),
            K::SubprogramDecl => {
                let Some((name, selection)) =
                    name_node(c).and_then(|n| written(script, dialect, n))
                else {
                    continue;
                };
                let function = c
                    .child_tokens()
                    .next()
                    .is_some_and(|t| t.text().eq_ignore_ascii_case("function"));
                let (detail, kind) = if function {
                    ("function", SymbolKind::Function)
                } else {
                    ("procedure", SymbolKind::Method)
                };
                let mut s = symbol(name, Some(detail), kind, c, selection);
                s.children = members(script, dialect, c);
                out.push(s);
            }
            K::CursorDecl | K::VarDecl | K::TypeDecl | K::ExceptionDecl => {
                let Some((name, selection)) = c
                    .child(K::DefName)
                    .and_then(|n| written(script, dialect, n))
                else {
                    continue;
                };
                let (detail, kind) = match c.kind() {
                    K::CursorDecl => ("cursor", SymbolKind::Variable),
                    K::TypeDecl => ("type", SymbolKind::Class),
                    K::ExceptionDecl => ("exception", SymbolKind::Event),
                    _ => ("variable", SymbolKind::Variable),
                };
                out.push(symbol(name, Some(detail), kind, c, selection));
            }
            _ => {}
        }
    }
    out
}

fn ctes(script: &Script, dialect: Dialect, query: SyntaxNode<'_>) -> Vec<Symbol> {
    let Some(with) = query.child(K::WithClause) else {
        return Vec::new();
    };
    with.child_nodes()
        .filter(|c| c.kind() == K::Cte)
        .filter_map(|c| {
            let (name, selection) = written(script, dialect, c.child(K::CteDef)?)?;
            Some(symbol(name, Some("CTE"), SymbolKind::Struct, c, selection))
        })
        .collect()
}

/// `customers c, orders`: the tables of the outer select's `FROM`.
fn tables_read(script: &Script, query: SyntaxNode<'_>) -> Option<String> {
    let select = query
        .child_nodes()
        .find(|n| n.kind() == K::Select)
        .or_else(|| (query.kind() == K::Select).then_some(query))?;
    let from = select.child(K::FromClause)?;
    let names: Vec<String> = from
        .descendants()
        .filter(|n| n.kind() == K::TableRef)
        .filter_map(|t| script.text().get(t.range()))
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return None;
    }
    let mut text = format!("from {}", names.join(", "));
    if text.chars().count() > MAX_DETAIL {
        text = text.chars().take(MAX_DETAIL - 1).collect::<String>() + "\u{2026}";
    }
    Some(text)
}
