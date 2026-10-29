//! Parameter info: the call the caret is in, the signatures of the routine
//! it names (the metadata model first, then the dialect's built-in
//! functions) and the parameter the caret is at.

use std::ops::Range;

use code_intel::{Markup, ParameterInfo, Signature, SignatureHelp};
use db_core::schema::{ArgumentMode, ObjectDetail};
use db_model::{Group, NameMatch, NodeKind};
use sql_syntax::{Dialect, Script, SyntaxToken, TokenKind};

use crate::builtins::{self, FnKind};
use crate::context::SqlContext;
use crate::cursor::{Caret, QualifierPart};

/// The call the caret is in.
#[derive(Debug)]
pub(crate) struct Call {
    /// The name of the routine, qualifier first.
    pub name: Vec<QualifierPart>,
    /// The argument the caret is in, counted from 0.
    pub index: usize,
    /// The name of a named argument (`name => value`) the caret is in.
    pub named: Option<String>,
}

struct Open<'a> {
    paren: SyntaxToken<'a>,
    before: Vec<SyntaxToken<'a>>,
    commas: usize,
    /// The first tokens of the current argument.
    argument: Vec<SyntaxToken<'a>>,
}

/// The innermost call around `offset`.
pub(crate) fn call_at(script: &Script, dialect: Dialect, offset: usize) -> Option<Call> {
    let caret = Caret::at(script, offset)?;
    let mut stack: Vec<Open<'_>> = Vec::new();
    let mut prev: Vec<SyntaxToken<'_>> = Vec::new();
    let tokens = caret
        .root
        .tokens()
        .take_while(|t| t.range().start < caret.offset);
    for t in tokens {
        match t.kind() {
            TokenKind::LParen => {
                let n = prev.len();
                stack.push(Open {
                    paren: t,
                    before: prev[n.saturating_sub(6)..].to_vec(),
                    commas: 0,
                    argument: Vec::new(),
                });
            }
            TokenKind::RParen => {
                stack.pop();
            }
            TokenKind::Comma => {
                if let Some(o) = stack.last_mut() {
                    o.commas += 1;
                    o.argument.clear();
                }
            }
            _ => {
                if let Some(o) = stack.last_mut()
                    && o.argument.len() < 2
                {
                    o.argument.push(t);
                }
            }
        }
        prev.push(t);
    }
    let open = stack.pop()?;
    let _ = open.paren;
    // The name: words joined by dots right before the parenthesis.
    let mut name: Vec<QualifierPart> = Vec::new();
    let mut i = open.before.len();
    while let Some(w) = i.checked_sub(1).map(|j| open.before[j]) {
        if !matches!(w.kind(), TokenKind::Word | TokenKind::QuotedIdent) {
            break;
        }
        let written = w.text().to_string();
        name.push(QualifierPart {
            name: dialect.fold(&written),
            written,
            range: w.range(),
        });
        i -= 1;
        match i.checked_sub(1).map(|j| open.before[j]) {
            Some(d) if d.kind() == TokenKind::Dot => i -= 1,
            _ => break,
        }
    }
    name.reverse();
    if name.is_empty() {
        return None;
    }
    let named = match open.argument.as_slice() {
        [w, op] if w.kind() == TokenKind::Word && matches!(op.text(), "=>" | ":=") => {
            Some(dialect.fold(w.text()))
        }
        _ => None,
    };
    Some(Call {
        name,
        index: open.commas,
        named,
    })
}

/// A label of `name(` + parameters + `)`, with the byte range of each
/// parameter in it.
fn label(name: &str, params: &[String], tail: &str) -> (String, Vec<Range<usize>>) {
    let mut s = format!("{name}(");
    let mut ranges = Vec::new();
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let start = s.len();
        s.push_str(p);
        ranges.push(start..s.len());
    }
    s.push(')');
    s.push_str(tail);
    (s, ranges)
}

pub(crate) fn signature_help(
    script: &Script,
    ctx: &SqlContext,
    offset: usize,
) -> Option<SignatureHelp> {
    let dialect = ctx.dialect;
    let call = call_at(script, dialect, offset)?;
    let last = call.name.last()?;
    let mode = if dialect == Dialect::Sqlite {
        NameMatch::IgnoreCase
    } else {
        NameMatch::Exact
    };
    let mut signatures: Vec<(Signature, Vec<Option<String>>, bool)> = Vec::new();
    if let Some(scope) = &ctx.scope {
        let model = &scope.model;
        let schemas: Vec<&db_model::SchemaModel> = if call.name.len() >= 2 {
            model
                .schema(&call.name[call.name.len() - 2].name, mode)
                .into_iter()
                .collect()
        } else {
            scope
                .search_path(dialect)
                .iter()
                .filter_map(|n| model.schema(n, mode))
                .collect()
        };
        for s in schemas {
            for r in s.lookup(&last.name, mode, &[NodeKind::Function, NodeKind::Procedure]) {
                let args: Vec<_> = r
                    .group(Group::Arguments)
                    .nodes()
                    .filter_map(|a| a.argument().map(|i| (a, i)))
                    .filter(|(_, i)| i.mode != ArgumentMode::Table)
                    .collect();
                let params: Vec<String> = args
                    .iter()
                    .map(|(a, i)| {
                        let name = if a.name().starts_with('$') {
                            String::new()
                        } else {
                            format!("{} ", a.name())
                        };
                        let default = if i.has_default { " = default" } else { "" };
                        format!("{name}{}{default}", i.data_type)
                    })
                    .collect();
                let names: Vec<Option<String>> = args
                    .iter()
                    .map(|(a, _)| (!a.name().starts_with('$')).then(|| a.name().to_string()))
                    .collect();
                let ret = match r.object().and_then(|o| o.detail.as_ref()) {
                    Some(ObjectDetail::Routine(d)) => d.return_type.clone(),
                    _ => None,
                };
                let tail = ret.map(|t| format!(" returns {t}")).unwrap_or_default();
                let (text, ranges) = label(r.name(), &params, &tail);
                signatures.push((
                    Signature {
                        label: text,
                        documentation: r
                            .comment()
                            .filter(|c| !c.trim().is_empty())
                            .map(|c| Markup::plain(c.trim())),
                        parameters: ranges
                            .into_iter()
                            .map(|label| ParameterInfo {
                                label,
                                documentation: None,
                            })
                            .collect(),
                        active_parameter: None,
                    },
                    names,
                    false,
                ));
            }
        }
    }
    if signatures.is_empty() && call.name.len() == 1 {
        for f in builtins::function(dialect, &last.written) {
            if f.kind == FnKind::Special {
                continue;
            }
            let params = f.parameters();
            let texts: Vec<String> = params
                .iter()
                .map(|p| {
                    if p.optional {
                        format!("[{}]", p.name)
                    } else {
                        p.name.clone()
                    }
                })
                .collect();
            let tail = if f.returns.is_empty() {
                String::new()
            } else {
                format!(" -> {}", f.returns)
            };
            let (text, ranges) = label(f.name, &texts, &tail);
            let variadic = params.last().is_some_and(|p| p.name == "...");
            signatures.push((
                Signature {
                    label: text,
                    documentation: Some(Markup::plain(f.doc)),
                    parameters: ranges
                        .into_iter()
                        .map(|label| ParameterInfo {
                            label,
                            documentation: None,
                        })
                        .collect(),
                    active_parameter: None,
                },
                params.iter().map(|p| Some(p.name.clone())).collect(),
                variadic,
            ));
        }
    }
    if signatures.is_empty() {
        return None;
    }
    // The parameter of each overload; the overload that has one for the
    // argument (and the named argument) is shown first.
    let mut best = 0;
    let mut found = false;
    for (i, (sig, names, variadic)) in signatures.iter_mut().enumerate() {
        let by_name = call.named.as_ref().and_then(|n| {
            names.iter().position(|p| {
                p.as_deref()
                    .is_some_and(|p| dialect.names_equal(&dialect.fold(p), n))
            })
        });
        let index = match (&call.named, by_name) {
            (Some(_), Some(i)) => Some(i),
            (Some(_), None) => None,
            (None, _) if call.index < sig.parameters.len() => Some(call.index),
            (None, _) if *variadic => Some(sig.parameters.len().saturating_sub(1)),
            _ => None,
        };
        sig.active_parameter = index;
        if index.is_some() && !found {
            found = true;
            best = i;
        }
    }
    Some(SignatureHelp {
        signatures: signatures.into_iter().map(|(s, _, _)| s).collect(),
        active_signature: best,
        active_parameter: call.named.is_none().then_some(call.index),
    })
}
