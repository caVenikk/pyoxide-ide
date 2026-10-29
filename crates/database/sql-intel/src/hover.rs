//! Quick documentation (hover): what the name or keyword at a position is.

use std::ops::Range;

use db_model::{Group, Model};
use sql_syntax::{Dialect, Script, SyntaxKind as K, SyntaxToken, TokenKind};

use crate::analysis::DocumentAnalysis;
use crate::builtins;
use crate::context::SqlContext;
use crate::docs;
use crate::reference::{RefKind, Resolution, Target};

/// The documentation (Markdown) and the range of the word it describes.
pub(crate) struct Documentation {
    pub text: String,
    pub range: Range<usize>,
}

fn word_at(script: &Script, offset: usize) -> Option<SyntaxToken<'_>> {
    let root = script.root_at(offset);
    let root = root.fragment_at(offset).map_or(root, |f| f.root);
    let mut found = None;
    for t in root.tokens() {
        let r = t.range();
        if r.start > offset {
            break;
        }
        if !matches!(t.kind(), TokenKind::Word | TokenKind::QuotedIdent) {
            continue;
        }
        if r.start <= offset && offset < r.end {
            return Some(t);
        }
        if r.end == offset {
            found = Some(t);
        }
    }
    found
}

pub(crate) fn hover(
    script: &Script,
    ctx: &SqlContext,
    analysis: &DocumentAnalysis,
    offset: usize,
) -> Option<Documentation> {
    let dialect = ctx.dialect;
    let token = word_at(script, offset)?;
    let range = token.range();
    let model = ctx.scope.as_ref().map(|s| &*s.model);
    let reference = analysis.reference_at(range.start);
    if let Some(r) = reference.filter(|r| r.range == range || r.range.start == range.start) {
        let text = match &r.resolution {
            Resolution::Resolved(Target::Object(id)) => {
                model.and_then(|m| docs::node_doc(m, *id, dialect))
            }
            Resolution::Resolved(Target::SourceColumn {
                column: Some(id), ..
            }) => model.and_then(|m| docs::node_doc(m, *id, dialect)),
            Resolution::Resolved(Target::SourceColumn { qualifier, .. }) => Some(format!(
                "```sql\ncolumn {} of {qualifier}\n```\n",
                token.text()
            )),
            Resolution::Resolved(Target::Local(def)) => {
                local_doc(script, analysis, model, dialect, def.clone(), token.text())
            }
            Resolution::Resolved(Target::ScriptObject { schema, name }) => {
                let q = match schema {
                    Some(s) => format!("{s}.{name}"),
                    None => name.clone(),
                };
                Some(format!("```sql\n{q}\n```\n\nCreated in this script.\n"))
            }
            _ => None,
        };
        if let Some(text) = text {
            return Some(Documentation { text, range });
        }
        if r.kind != RefKind::Routine {
            if let Some(text) = builtin_or_keyword(script, dialect, &token) {
                return Some(Documentation { text, range });
            }
            return None;
        }
    }
    let text = builtin_or_keyword(script, dialect, &token)?;
    Some(Documentation { text, range })
}

/// A built-in function (when the word is called) or a keyword.
fn builtin_or_keyword(
    script: &Script,
    dialect: Dialect,
    token: &SyntaxToken<'_>,
) -> Option<String> {
    let text = token.text();
    let rest = script.text().get(token.range().end..).unwrap_or("");
    let called = rest.trim_start().starts_with('(');
    let overloads = builtins::function(dialect, text);
    let special_ok = overloads
        .iter()
        .any(|f| f.kind == builtins::FnKind::Special);
    if (called || special_ok)
        && let Some(f) = overloads.first()
    {
        return Some(docs::function_doc(f, &overloads));
    }
    if token.kind() == TokenKind::Word {
        if dialect.keyword(text).is_some()
            && let Some(doc) = docs::keyword_text(text)
        {
            return Some(doc);
        }
        if let Some(t) = builtins::data_type(dialect, text) {
            // A type name, not a column called `text`.
            let in_type = token
                .parent()
                .is_some_and(|p| matches!(p.kind(), K::TypeRef | K::CastExpr));
            if in_type {
                return Some(docs::type_doc(t));
            }
        }
    }
    None
}

/// An alias or a CTE: what it stands for.
fn local_doc(
    script: &Script,
    analysis: &DocumentAnalysis,
    model: Option<&Model>,
    dialect: Dialect,
    def: Range<usize>,
    written: &str,
) -> Option<String> {
    let node = script.node_at(def.start)?;
    let def_node = std::iter::once(node)
        .chain(node.ancestors())
        .find(|n| matches!(n.kind(), K::AliasDef | K::CteDef))?;
    match def_node.kind() {
        K::CteDef => {
            let cte = def_node.parent()?;
            let mut columns = Vec::new();
            if let Some(list) = cte.child(K::ColumnAliasList) {
                for c in list.tokens() {
                    if c.kind() == TokenKind::Word || c.kind() == TokenKind::QuotedIdent {
                        columns.push(c.text().to_string());
                    }
                }
            } else if let Some(q) = cte.child(K::Query) {
                columns = select_names(q);
            }
            Some(docs::cte_doc(written, &columns))
        }
        _ => {
            let source = def_node.parent()?;
            let table = source.child(K::TableRef);
            let (shown, columns) = match table {
                Some(t) => {
                    let name = t.range();
                    let shown = script.text().get(name.clone()).map(str::to_string);
                    let mut cols = Vec::new();
                    let object = analysis
                        .reference_at(last_word_start(script, &name))
                        .and_then(|r| match r.resolution {
                            Resolution::Resolved(Target::Object(id)) => Some(id),
                            _ => None,
                        });
                    if let (Some(m), Some(id)) = (model, object)
                        && let Some(node) = m.node(id)
                    {
                        for c in node.group(Group::Columns).nodes() {
                            cols.push((
                                c.name().to_string(),
                                c.column().map(|i| i.data_type.clone()),
                            ));
                        }
                    }
                    (shown, cols)
                }
                None => (Some("a subquery".to_string()), Vec::new()),
            };
            let _ = dialect;
            Some(docs::alias_doc(written, shown, &columns))
        }
    }
}

fn last_word_start(script: &Script, name: &Range<usize>) -> usize {
    let text = script.text().get(name.clone()).unwrap_or("");
    let tail = text.rsplit('.').next().unwrap_or(text);
    name.end - tail.trim_end().len()
}

/// The output names of the first select of a query.
fn select_names(q: sql_syntax::SyntaxNode<'_>) -> Vec<String> {
    let Some(select) = q.descendants().find(|n| n.kind() == K::Select) else {
        return Vec::new();
    };
    let Some(list) = select.child(K::SelectList) else {
        return Vec::new();
    };
    list.child_nodes()
        .filter(|n| n.kind() == K::SelectItem)
        .filter_map(|item| {
            if let Some(a) = item.child(K::AliasDef) {
                return a.tokens().last().map(|t| t.text().to_string());
            }
            item.child(K::ColumnRef)
                .and_then(|c| c.tokens().last())
                .map(|t| t.text().to_string())
        })
        .collect()
}
