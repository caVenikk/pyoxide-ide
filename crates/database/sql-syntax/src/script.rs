use std::ops::Range;
use std::sync::Arc;

use crate::classify::DdlTarget;
use crate::dialect::Dialect;
use crate::lexer::{Lexer, Token, TokenKind, lex};
use crate::parser;
use crate::split::statement_spans;
use crate::tree::{Expected, FragmentLanguage, StatementTree, SyntaxKind, SyntaxNode, SyntaxToken};

/// How far after the start of a unit an edit must be to leave the unit before
/// it alone: the lexer looks at most this many bytes past a token's end.
const LOOKAHEAD: usize = 4;

/// The syntax trees of a script: one [`StatementTree`] per parse unit. A unit
/// is a statement with its terminator and the trivia up to the next
/// statement (the first unit also takes the trivia before it), so the units
/// follow each other and cover the text. Units are split by the lexer and the
/// statement splitter ([`crate::split`]), never by searching for `;`.
///
/// A `Script` is an immutable snapshot that is cheap to clone (the trees are
/// shared); [`Script::reparse`] makes the next snapshot after an edit,
/// re-lexing and re-parsing only the statements the edit touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    text: String,
    dialect: Dialect,
    units: Vec<Unit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Unit {
    start: usize,
    tree: Arc<StatementTree>,
}

/// A change of a script's text: `range` (byte offsets, clamped and moved
/// back to character boundaries) is replaced by `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxErrorKind {
    /// Something is missing; the range is empty at the end of the previous
    /// token.
    Missing,
    /// The token at the range was not expected.
    Unexpected,
}

/// A syntax error from an `Error` or `Missing` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub kind: SyntaxErrorKind,
    pub range: Range<usize>,
    /// What would have been valid, sorted.
    pub expected: Vec<Expected>,
}

impl SyntaxError {
    /// `Expected: <up to five items>` or `Unexpected '<token>'`.
    pub fn message(&self, src: &str) -> String {
        match self.kind {
            SyntaxErrorKind::Missing => {
                let items: Vec<String> = self
                    .expected
                    .iter()
                    .take(5)
                    .map(|e| e.to_string())
                    .collect();
                if items.is_empty() {
                    "Syntax error".to_string()
                } else {
                    format!("Expected: {}", items.join(", "))
                }
            }
            SyntaxErrorKind::Unexpected => {
                format!("Unexpected '{}'", src.get(self.range.clone()).unwrap_or(""))
            }
        }
    }
}

/// The errors of a tree and of the programs embedded in its strings.
fn collect_errors(root: SyntaxNode<'_>, out: &mut Vec<SyntaxError>) {
    for n in root.descendants() {
        match n.kind() {
            SyntaxKind::Missing => {
                // Only the first of several missing parts at one position:
                // the others follow from it.
                let repeated = out
                    .last()
                    .is_some_and(|e| e.kind == SyntaxErrorKind::Missing && e.range == n.range());
                if !repeated {
                    out.push(SyntaxError {
                        kind: SyntaxErrorKind::Missing,
                        range: n.range(),
                        expected: n.expected().to_vec(),
                    });
                }
            }
            SyntaxKind::Error => out.push(SyntaxError {
                kind: SyntaxErrorKind::Unexpected,
                range: n.tokens().next().map_or(n.range(), |t| t.range()),
                expected: n.expected().to_vec(),
            }),
            SyntaxKind::EmbeddedBody => {
                if let Some(f) = n.fragment() {
                    collect_errors(f.root, out);
                }
            }
            _ => {}
        }
    }
}

/// Parses a script.
pub fn parse(text: &str, dialect: Dialect) -> Script {
    Script::parse(text, dialect)
}

/// The script after `edit`; equal to parsing the edited text from scratch.
pub fn reparse(script: &Script, edit: &TextEdit) -> Script {
    script.reparse(edit)
}

fn floor_boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Parses the units of `text` that start at `starts` and end at `end`;
/// `tokens` are the tokens of that part of the text, with offsets into it.
fn units_of(
    text: &str,
    tokens: &[Token],
    starts: &[usize],
    end: usize,
    dialect: Dialect,
) -> Vec<Unit> {
    let mut out = Vec::with_capacity(starts.len());
    for (i, &s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).copied().unwrap_or(end);
        let first = tokens.partition_point(|t| t.start < s);
        let last = tokens.partition_point(|t| t.start < e);
        let rel: Vec<Token> = tokens[first..last]
            .iter()
            .map(|t| Token {
                start: t.start - s,
                end: t.end - s,
                ..*t
            })
            .collect();
        out.push(Unit {
            start: s,
            tree: Arc::new(parser::parse_unit(&text[s..e], rel, dialect)),
        });
    }
    out
}

/// The unit starts of a part of a text that begins at `start`: `start`,
/// then the first token of every statement but the first.
fn unit_starts(start: usize, sig: &[Token], spans: &[(usize, usize)]) -> Vec<usize> {
    let mut starts = vec![start];
    starts.extend(spans.iter().skip(1).map(|&(f, _)| sig[f].start));
    starts
}

impl Script {
    pub fn parse(text: &str, dialect: Dialect) -> Script {
        let toks = lex(text, dialect);
        let sig: Vec<Token> = toks.iter().copied().filter(|t| !t.is_trivia()).collect();
        let slashes: Vec<usize> = sig
            .iter()
            .filter(|t| t.kind == TokenKind::SlashLine)
            .map(|t| t.start)
            .collect();
        let slash_ahead = |at: usize| slashes.partition_point(|&p| p < at) < slashes.len();
        let (spans, _) = statement_spans(text, &sig, dialect, &slash_ahead);
        let starts = unit_starts(0, &sig, &spans);
        Script {
            text: text.to_string(),
            dialect,
            units: units_of(text, &toks, &starts, text.len(), dialect),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The root nodes of the units, in order.
    pub fn statements(&self) -> impl Iterator<Item = SyntaxNode<'_>> + '_ {
        self.units.iter().map(|u| u.tree.root(u.start, &self.text))
    }

    fn unit_index(&self, offset: usize) -> usize {
        self.units
            .partition_point(|u| u.start <= offset)
            .saturating_sub(1)
    }

    /// The root of the unit `offset` is in (the last unit for the end of the
    /// text).
    pub fn root_at(&self, offset: usize) -> SyntaxNode<'_> {
        let u = &self.units[self.unit_index(offset)];
        u.tree.root(u.start, &self.text)
    }

    /// The innermost node at `offset`, see [`SyntaxNode::node_at`].
    pub fn node_at(&self, offset: usize) -> Option<SyntaxNode<'_>> {
        self.root_at(offset).node_at(offset)
    }

    /// The nearest node of `kind` around `offset`.
    pub fn enclosing(&self, offset: usize, kind: SyntaxKind) -> Option<SyntaxNode<'_>> {
        self.node_at(offset)?.enclosing(kind)
    }

    /// The token that contains `offset`, trivia included; at a boundary
    /// between two tokens the one after it.
    pub fn token_at(&self, offset: usize) -> Option<SyntaxToken<'_>> {
        let root = self.root_at(offset);
        root.preorder()
            .filter_map(|e| e.as_token())
            .find(|t| t.range().contains(&offset))
    }

    /// What may stand at `offset`: the expected set of the parser at that
    /// point of the statement, for completion. A word that ends at `offset`
    /// is the prefix being typed and does not count; text after `offset`
    /// does not count either. After `c.` this is a column name or `*`, after
    /// `FROM` a table name, at the start of a statement the statement
    /// keywords.
    pub fn expected_at(&self, offset: usize) -> Vec<Expected> {
        let offset = floor_boundary(&self.text, offset);
        let u = &self.units[self.unit_index(offset)];
        if let Some(f) = u.tree.root(u.start, &self.text).fragment_at(offset) {
            let proc = match f.language {
                FragmentLanguage::PlPgSql => parser::Proc::PlPgSql,
                FragmentLanguage::Sql => parser::Proc::No,
            };
            return parser::expected_at(
                &self.text[f.body.clone()],
                f.root.statement_tree().tokens(),
                self.dialect,
                offset - f.body.start,
                proc,
            );
        }
        let end = u.start + u.tree.len();
        parser::expected_at(
            &self.text[u.start..end],
            u.tree.tokens(),
            self.dialect,
            offset - u.start,
            parser::Proc::No,
        )
    }

    /// The objects the DDL statement at `offset` creates, alters, drops,
    /// truncates or comments, read from the tree; `None` for other statements
    /// and for DDL the parser does not know. The token scan of
    /// [`crate::classify`] gives the same answer for well-formed statements.
    pub fn ddl_target(&self, offset: usize) -> Option<DdlTarget> {
        let stmt = self.root_at(offset).child_nodes().next()?;
        crate::targets::ddl_target(stmt, self.dialect)
    }

    /// The syntax errors: one per `Error` node and per position with
    /// `Missing` nodes, in document order.
    pub fn errors(&self) -> Vec<SyntaxError> {
        let mut out: Vec<SyntaxError> = Vec::new();
        for root in self.statements() {
            collect_errors(root, &mut out);
        }
        out
    }

    /// The text printed from the trees; always equal to [`Script::text`].
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        for root in self.statements() {
            out.push_str(&root.print());
        }
        out
    }

    /// An estimate of the heap memory of the trees in bytes.
    pub fn heap_size(&self) -> usize {
        self.text.capacity()
            + self.units.capacity() * size_of::<Unit>()
            + self.units.iter().map(|u| u.tree.heap_size()).sum::<usize>()
    }

    /// The script after `edit`. The units the edit touches are re-lexed and
    /// re-parsed, up to the first later unit whose start lexes to the same
    /// token and that starts a statement again; the units after it are
    /// shared and shifted. The result equals a full parse of the new text.
    pub fn reparse(&self, edit: &TextEdit) -> Script {
        let old = &self.text;
        let a = floor_boundary(old, edit.range.start);
        let b = floor_boundary(old, edit.range.end).max(a);
        let mut text = String::with_capacity(old.len() - (b - a) + edit.text.len());
        text.push_str(&old[..a]);
        text.push_str(&edit.text);
        text.push_str(&old[b..]);
        let delta = edit.text.len() as isize - (b - a) as isize;
        let shift = |p: usize| (p as isize + delta) as usize;
        let units = &self.units;
        let n = units.len();
        let dialect = self.dialect;

        let mut i = self.unit_index(a);
        if i > 0 && a < units[i].start + LOOKAHEAD {
            i -= 1;
        }
        'restart: loop {
            let s = units[i].start;
            // Re-lex from the unit's start until a later unit (after the edit)
            // starts at the same token.
            let mut k = units.partition_point(|u| u.start <= b).max(i + 1);
            let mut region: Vec<Token> = Vec::new();
            let mut synced = false;
            let lexer = Lexer::new(&text, dialect, dialect.default_params()).starting_at(s);
            for tok in lexer {
                while k < n && shift(units[k].start) < tok.start {
                    k += 1;
                }
                if k < n && shift(units[k].start) == tok.start {
                    let first = units[k].tree.tokens()[0];
                    if first.kind == tok.kind
                        && first.end - first.start == tok.end - tok.start
                        && first.unterminated == tok.unterminated
                    {
                        synced = true;
                        break;
                    }
                }
                region.push(tok);
            }
            if !synced {
                k = n;
            }
            loop {
                let end = if k < n {
                    shift(units[k].start)
                } else {
                    text.len()
                };
                let sig: Vec<Token> = region.iter().copied().filter(|t| !t.is_trivia()).collect();
                let later_slash = units[k..].iter().any(|u| u.tree.has_slash_line());
                let region_slashes: Vec<usize> = sig
                    .iter()
                    .filter(|t| t.kind == TokenKind::SlashLine)
                    .map(|t| t.start)
                    .collect();
                let slash_ahead = |at: usize| {
                    later_slash
                        || region_slashes.partition_point(|&p| p < at) < region_slashes.len()
                };
                let (spans, closed) = statement_spans(&text, &sig, dialect, &slash_ahead);
                if i > 0 && spans.first().is_none_or(|&(f, _)| sig[f].start != s) {
                    // The unit no longer starts a statement: it joins the one before.
                    i -= 1;
                    continue 'restart;
                }
                if k < n && (!closed || (i == 0 && spans.is_empty())) {
                    // The last statement runs on into the next unit.
                    let u = &units[k];
                    let base = shift(u.start);
                    region.extend(u.tree.tokens().iter().map(|t| Token {
                        start: t.start + base,
                        end: t.end + base,
                        ..*t
                    }));
                    k += 1;
                    continue;
                }
                // A `/` line added or removed changes how PL/SQL blocks before
                // the region end (they run to the next `/` line when there is one).
                let old_slash = units[i..k].iter().any(|u| u.tree.has_slash_line());
                if old_slash != !region_slashes.is_empty() && !later_slash {
                    return Script::parse(&text, dialect);
                }
                let starts = unit_starts(s, &sig, &spans);
                let mut out: Vec<Unit> = Vec::with_capacity(n + starts.len());
                out.extend_from_slice(&units[..i]);
                out.extend(units_of(&text, &region, &starts, end, dialect));
                out.extend(units[k..].iter().map(|u| Unit {
                    start: shift(u.start),
                    tree: Arc::clone(&u.tree),
                }));
                return Script {
                    text,
                    dialect,
                    units: out,
                };
            }
        }
    }
}
