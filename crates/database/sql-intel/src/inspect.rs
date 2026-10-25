//! The inspections of the first set (ADR 0022) over one statement, with
//! their quick fixes as text edits.

use std::ops::Range;

use sql_syntax::{Expected, SyntaxKind as K, SyntaxNode, TokenKind};

use crate::profile::Inspection;
use crate::reference::{Reference, Resolution, Suggestion, Target};
use crate::resolve::StatementOut;

/// A replacement of a byte range of the document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FixEdit {
    pub range: Range<usize>,
    pub text: String,
}

/// A quick fix of a problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<FixEdit>,
}

impl Fix {
    fn new(title: impl Into<String>, edits: Vec<FixEdit>) -> Self {
        Fix {
            title: title.into(),
            edits,
        }
    }

    fn one(title: impl Into<String>, range: Range<usize>, text: impl Into<String>) -> Self {
        Fix::new(
            title,
            vec![FixEdit {
                range,
                text: text.into(),
            }],
        )
    }
}

/// A problem an inspection found. Ranges are byte offsets in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub inspection: Inspection,
    pub range: Range<usize>,
    pub message: String,
    /// Greyed like unused code.
    pub unnecessary: bool,
    /// The inspection's own fixes, best first. *Suppress for statement* and
    /// *Disable inspection* are added by the services.
    pub fixes: Vec<Fix>,
}

impl Problem {
    pub(crate) fn shift(&mut self, delta: isize) {
        self.range = shift_range(&self.range, delta);
        for f in &mut self.fixes {
            for e in &mut f.edits {
                e.range = shift_range(&e.range, delta);
            }
        }
    }
}

pub(crate) fn shift_range(r: &Range<usize>, delta: isize) -> Range<usize> {
    let s = (r.start as isize + delta).max(0) as usize;
    let e = (r.end as isize + delta).max(0) as usize;
    s..e
}

/// Runs the inspections over a parse unit; `out` is its resolution.
pub(crate) fn inspect(root: SyntaxNode<'_>, out: &StatementOut) -> Vec<Problem> {
    let mut problems = Vec::new();
    syntax(root, &mut problems);
    for r in &out.refs {
        reference(r, &mut problems);
    }
    for (range, columns, values) in &out.arity {
        problems.push(Problem {
            inspection: Inspection::InsertValues,
            range: range.clone(),
            message: format!(
                "{columns} target {} but {values} {}",
                plural(*columns, "column", "columns"),
                plural(*values, "value", "values"),
            ),
            unnecessary: false,
            fixes: Vec::new(),
        });
    }
    tree(root, out, &mut problems);
    problems.sort_by_key(|p| p.range.start);
    problems
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

// ---- syntax ------------------------------------------------------------------

fn syntax(root: SyntaxNode<'_>, out: &mut Vec<Problem>) {
    let mut last_missing: Option<Range<usize>> = None;
    for n in root.descendants() {
        match n.kind() {
            K::Missing => {
                let range = n.range();
                if last_missing.as_ref() == Some(&range) {
                    continue;
                }
                last_missing = Some(range.clone());
                let expected = n.expected();
                let items: Vec<String> = expected.iter().take(5).map(|e| e.to_string()).collect();
                let message = if items.is_empty() {
                    "Syntax error".to_string()
                } else {
                    format!("Expected: {}", items.join(", "))
                };
                let mut fixes = Vec::new();
                if expected.contains(&Expected::Punct(")")) && unbalanced(root, range.start) {
                    fixes.push(Fix::one("Add missing ')'", range.clone(), ")"));
                }
                out.push(Problem {
                    inspection: Inspection::Syntax,
                    range,
                    message,
                    unnecessary: false,
                    fixes,
                });
            }
            K::Error => {
                let range = n.tokens().next().map_or(n.range(), |t| t.range());
                let text = n.tokens().next().map_or("", |t| t.text());
                out.push(Problem {
                    inspection: Inspection::Syntax,
                    message: format!("Unexpected '{text}'"),
                    range,
                    unnecessary: false,
                    fixes: Vec::new(),
                });
            }
            K::EmbeddedBody => {
                if let Some(f) = n.fragment() {
                    syntax(f.root, out);
                }
            }
            _ => {}
        }
    }
    for e in root.preorder() {
        let Some(t) = e.as_token() else { continue };
        if !t.unterminated() {
            continue;
        }
        let what = match t.kind() {
            TokenKind::String => "string",
            TokenKind::QuotedIdent => "quoted identifier",
            TokenKind::BlockComment => "comment",
            _ => continue,
        };
        let start = t.range().start;
        let first = t.text().chars().next().map_or(1, char::len_utf8);
        out.push(Problem {
            inspection: Inspection::Syntax,
            range: start..start + first,
            message: format!("Unterminated {what}"),
            unnecessary: false,
            fixes: Vec::new(),
        });
    }
}

/// More `(` than `)` before `at` in the unit.
fn unbalanced(root: SyntaxNode<'_>, at: usize) -> bool {
    let mut depth: i32 = 0;
    for t in root.tokens() {
        if t.range().start >= at {
            break;
        }
        match t.kind() {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

// ---- references --------------------------------------------------------------

fn reference(r: &Reference, out: &mut Vec<Problem>) {
    match &r.resolution {
        Resolution::Unresolved => {
            let mut fixes = Vec::new();
            for s in &r.suggestions {
                match s {
                    Suggestion::Rename(name) => {
                        fixes.push(Fix::one(
                            format!("Change to '{name}'"),
                            r.range.clone(),
                            name.clone(),
                        ));
                    }
                    Suggestion::Qualify(schema) => {
                        fixes.push(Fix::one(
                            format!("Qualify with schema '{schema}'"),
                            r.range.start..r.range.start,
                            format!("{schema}."),
                        ));
                    }
                    Suggestion::UseAlias(alias) => {
                        fixes.push(Fix::one(
                            format!("Use alias '{alias}'"),
                            r.range.clone(),
                            alias.clone(),
                        ));
                    }
                }
            }
            out.push(Problem {
                inspection: Inspection::Resolve,
                range: r.range.clone(),
                message: format!("Cannot resolve {} '{}'", r.kind.noun(), r.written),
                unnecessary: false,
                fixes,
            });
        }
        Resolution::Ambiguous(targets) => {
            let qualifiers: Vec<&str> = targets
                .iter()
                .filter_map(|t| match t {
                    Target::SourceColumn { qualifier, .. } if !qualifier.is_empty() => {
                        Some(qualifier.as_str())
                    }
                    _ => None,
                })
                .collect();
            let listed = match qualifiers.as_slice() {
                [] => String::new(),
                [one] => format!("'{one}'"),
                [init @ .., last] => {
                    let init: Vec<String> = init.iter().map(|q| format!("'{q}'")).collect();
                    format!("{} and '{last}'", init.join(", "))
                }
            };
            let message = if listed.is_empty() {
                format!("Ambiguous column '{}'", r.written)
            } else {
                format!("Ambiguous column '{}': it is in {listed}", r.written)
            };
            let fixes = qualifiers
                .iter()
                .map(|q| {
                    Fix::one(
                        format!("Qualify with '{q}'"),
                        r.range.start..r.range.start,
                        format!("{q}."),
                    )
                })
                .collect();
            out.push(Problem {
                inspection: Inspection::AmbiguousColumn,
                range: r.range.clone(),
                message,
                unnecessary: false,
                fixes,
            });
        }
        Resolution::Resolved(_) | Resolution::Unknown => {}
    }
}

// ---- the tree ----------------------------------------------------------------

fn tree(root: SyntaxNode<'_>, out: &StatementOut, problems: &mut Vec<Problem>) {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            K::Update | K::Delete => without_where(n, problems),
            K::BinaryExpr => null_comparison(n, problems),
            K::WithClause => unused_ctes(n, out, problems),
            K::Error | K::Opaque => continue,
            _ => {}
        }
        stack.extend(n.child_nodes());
        if n.kind() == K::EmbeddedBody
            && let Some(f) = n.fragment()
        {
            stack.push(f.root);
        }
    }
}

fn without_where(node: SyntaxNode<'_>, out: &mut Vec<Problem>) {
    if node.child(K::WhereClause).is_some() || names_error(node) {
        return;
    }
    let Some(target) = node
        .child_nodes()
        .find(|n| matches!(n.kind(), K::TableSource | K::SubquerySource))
    else {
        return;
    };
    let start = node
        .child_tokens()
        .next()
        .map_or(node.range().start, |t| t.range().start);
    let (verb, effect) = if node.kind() == K::Delete {
        ("DELETE", "deletes")
    } else {
        ("UPDATE", "changes")
    };
    let fix = match node.child(K::ReturningClause) {
        Some(r) => Fix::one(
            "Add WHERE clause",
            r.range().start..r.range().start,
            "where ",
        ),
        None => Fix::one(
            "Add WHERE clause",
            node.range().end..node.range().end,
            " where ",
        ),
    };
    out.push(Problem {
        inspection: Inspection::WithoutWhere,
        range: start..target.range().end,
        message: format!("{verb} without WHERE {effect} every row of the table"),
        unnecessary: false,
        fixes: vec![fix],
    });
}

/// The statement has a syntax error that may have swallowed its `WHERE`.
fn names_error(node: SyntaxNode<'_>) -> bool {
    node.child_nodes()
        .any(|n| matches!(n.kind(), K::Error | K::Missing))
}

fn is_null(node: SyntaxNode<'_>) -> bool {
    node.kind() == K::Literal && node.text().eq_ignore_ascii_case("null")
}

fn null_comparison(node: SyntaxNode<'_>, out: &mut Vec<Problem>) {
    let Some(op) = node
        .child_tokens()
        .find(|t| matches!(t.text(), "=" | "<>" | "!=" | "^="))
    else {
        return;
    };
    let mut operands = node.child_nodes();
    let (Some(lhs), Some(rhs)) = (operands.next(), operands.next()) else {
        return;
    };
    let negated = op.text() != "=";
    let (null, other, null_right) = if is_null(rhs) {
        (rhs, lhs, true)
    } else if is_null(lhs) {
        (lhs, rhs, false)
    } else {
        return;
    };
    if is_null(other) {
        return;
    }
    let upper = null.text().starts_with('N');
    let replacement = match (negated, upper) {
        (false, false) => "is null",
        (false, true) => "IS NULL",
        (true, false) => "is not null",
        (true, true) => "IS NOT NULL",
    };
    let shown = if negated { "IS NOT NULL" } else { "IS NULL" };
    let (range, fix) = if null_right {
        let range = op.range().start..null.range().end;
        (
            range.clone(),
            Fix::one(format!("Replace with {shown}"), range, replacement),
        )
    } else {
        (
            null.range().start..op.range().end,
            Fix::one(
                format!("Replace with {shown}"),
                node.range(),
                format!("{} {replacement}", other.text()),
            ),
        )
    };
    out.push(Problem {
        inspection: Inspection::NullComparison,
        range,
        message: format!("Comparison with NULL is never true; use {shown}"),
        unnecessary: false,
        fixes: vec![fix],
    });
}

fn unused_ctes(with: SyntaxNode<'_>, out: &StatementOut, problems: &mut Vec<Problem>) {
    let ctes: Vec<SyntaxNode<'_>> = with.child_nodes().filter(|n| n.kind() == K::Cte).collect();
    for (i, cte) in ctes.iter().enumerate() {
        let Some(def) = cte.child(K::CteDef) else {
            continue;
        };
        let range = def.range();
        if range.is_empty() || out.used_ctes.contains(&range.start) {
            continue;
        }
        if crate::names::has_hole(with.parent().unwrap_or(with)) {
            continue;
        }
        let removal = if ctes.len() == 1 {
            // The whole WITH clause, up to what follows it.
            let end = next_sibling_start(with).unwrap_or(with.range().end);
            with.range().start..end
        } else if i + 1 < ctes.len() {
            cte.range().start..ctes[i + 1].range().start
        } else {
            ctes[i - 1].range().end..cte.range().end
        };
        problems.push(Problem {
            inspection: Inspection::UnusedCte,
            range,
            message: format!("CTE '{}' is never used", def.text()),
            unnecessary: true,
            fixes: vec![Fix::one("Remove CTE", removal, "")],
        });
    }
}

/// The start of the node or token after `node` among its parent's
/// significant children.
fn next_sibling_start(node: SyntaxNode<'_>) -> Option<usize> {
    let parent = node.parent()?;
    let end = node.range().end;
    parent
        .children()
        .filter(|e| e.as_token().is_none_or(|t| !t.is_trivia()))
        .map(|e| e.range().start)
        .find(|&s| s >= end && s > node.range().start)
}
