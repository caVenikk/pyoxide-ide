use std::ops::Range;

use crate::classify::{Classification, Danger, DdlTarget, StatementKind, classify, classify_text};
use crate::dialect::Dialect;
use crate::lexer::{Token, TokenKind, lex};

/// A statement of a script: its text (trimmed; without the closing `;`,
/// except for PL/SQL blocks, which keep theirs), its byte range in the script
/// and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub text: String,
    pub range: Range<usize>,
    pub kind: StatementKind,
    /// Changes data, see [`Classification::writes`].
    pub writes: bool,
    /// A `DELETE` or `UPDATE` without `WHERE`.
    pub danger: Option<Danger>,
    /// The objects a DDL statement names.
    pub ddl: Option<DdlTarget>,
}

/// Splits a script into statements by the lexer: semicolons inside strings,
/// quoted names, comments, `$tag$` quotes and `q'[..]'` do not split;
/// a statement of a PL/SQL block (Oracle), a `BEGIN ATOMIC` body
/// (PostgreSQL) or a trigger body (SQLite) runs to the end of the block.
/// Oracle also ends a statement or block at a line with only `/`.
/// Fragments with nothing but comments are dropped. Never panics; the ranges
/// are ordered, do not overlap and fall on character boundaries.
pub fn split(script: &str, dialect: Dialect) -> Vec<Statement> {
    let toks = lex(script, dialect);
    let sig: Vec<Token> = toks.iter().copied().filter(|t| !t.is_trivia()).collect();
    // Start of every `/` line, to know whether a block has one ahead.
    let slashes: Vec<usize> = sig
        .iter()
        .filter(|t| t.kind == TokenKind::SlashLine)
        .map(|t| t.start)
        .collect();
    let slash_ahead = |start: usize| slashes.partition_point(|&p| p < start) < slashes.len();
    let (spans, _) = statement_spans(script, &sig, dialect, &slash_ahead);
    spans
        .into_iter()
        .map(|(first, last)| {
            let toks = &sig[first..=last];
            let range = toks[0].start..toks[toks.len() - 1].end;
            let Classification {
                kind,
                writes,
                danger,
                ddl,
            } = classify(script, toks, dialect);
            Statement {
                text: script[range.clone()].to_string(),
                range,
                kind,
                writes,
                danger,
                ddl,
            }
        })
        .collect()
}

/// The statements of the significant tokens `sig` of `script`, as the
/// indices of their first and last token, and whether the last statement
/// was closed (by a terminator) rather than cut by the end of `sig`.
/// `slash_ahead(offset)`: a `/` line follows `offset` somewhere in the
/// script (Oracle: a PL/SQL block then runs to it).
pub(crate) fn statement_spans(
    script: &str,
    sig: &[Token],
    dialect: Dialect,
    slash_ahead: &dyn Fn(usize) -> bool,
) -> (Vec<(usize, usize)>, bool) {
    let mode = match dialect {
        Dialect::Oracle => Mode::Oracle,
        Dialect::Postgres => Mode::Postgres,
        Dialect::Generic | Dialect::Sqlite => Mode::Sql,
    };
    let mut out = Vec::new();
    let mut cur = Pending::default();
    for (i, tok) in sig.iter().enumerate() {
        match tok.kind {
            TokenKind::SlashLine => {
                cur.flush(&mut out);
                continue;
            }
            TokenKind::Semicolon => {
                if let Some(first) = cur.first
                    && is_block_statement(&cur.head, dialect)
                {
                    let start = sig[first].start;
                    cur.blocks.token(";", false, mode, None);
                    let slash_ahead = mode == Mode::Oracle && slash_ahead(start);
                    if slash_ahead || !cur.blocks.stack.is_empty() {
                        cur.extend(i);
                        continue;
                    }
                    if mode == Mode::Oracle {
                        // PL/SQL keeps the `;` after its `END`.
                        cur.extend(i);
                    }
                }
                cur.flush(&mut out);
                continue;
            }
            _ => {}
        }
        cur.extend(i);
        let text = tok.text(script);
        let is_word = tok.kind == TokenKind::Word;
        let up = if is_word {
            text.to_ascii_uppercase()
        } else {
            text.to_string()
        };
        if cur.in_label {
            cur.in_label = up != ">>";
        } else if cur.head.is_empty() && up == "<<" {
            cur.in_label = true;
        } else if is_word && cur.head.len() < 10 {
            cur.head.push(up.clone());
        }
        let next_word = sig
            .get(i + 1)
            .filter(|t| t.kind == TokenKind::Word)
            .map(|t| t.text(script));
        cur.blocks.token(&up, is_word, mode, next_word);
    }
    let closed = cur.first.is_none();
    cur.flush(&mut out);
    (out, closed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Oracle,
    Postgres,
    Sql,
}

/// The statement being collected by [`split`].
#[derive(Default)]
struct Pending {
    /// Indices into the significant tokens: the first and the last of it.
    first: Option<usize>,
    last: usize,
    /// First words, upper case (to recognise blocks).
    head: Vec<String>,
    in_label: bool,
    blocks: Blocks,
}

impl Pending {
    fn extend(&mut self, i: usize) {
        if self.first.is_none() {
            self.first = Some(i);
        }
        self.last = i;
    }

    fn flush(&mut self, out: &mut Vec<(usize, usize)>) {
        if let Some(first) = self.first {
            out.push((first, self.last));
        }
        *self = Pending::default();
    }
}

/// Nesting of procedural blocks inside a statement.
#[derive(Default)]
struct Blocks {
    /// Open levels; `true` = a declaration section (`DECLARE`, `IS`/`AS` of a
    /// subprogram or package) whose `BEGIN` does not open another level.
    stack: Vec<bool>,
    /// After `PROCEDURE`/`FUNCTION`/`PACKAGE`/`TYPE BODY` (Oracle): the next
    /// `IS`/`AS` opens a declaration section.
    header: bool,
    /// The previous token (words upper case).
    prev: String,
}

impl Blocks {
    fn token(&mut self, t: &str, is_word: bool, mode: Mode, next_word: Option<&str>) {
        let oracle = mode == Mode::Oracle;
        if is_word {
            let after_end = self.prev == "END";
            // Where a procedural statement can start (`IF(..)` is also a function).
            let stmt_start = matches!(
                self.prev.as_str(),
                "" | ";"
                    | ":"
                    | ">"
                    | "BEGIN"
                    | "THEN"
                    | "ELSE"
                    | "DO"
                    | "LOOP"
                    | "REPEAT"
                    | "DECLARE"
                    | "IS"
                    | "AS"
            );
            match t {
                "BEGIN"
                    if mode == Mode::Postgres
                        && !next_word.is_some_and(|w| w.eq_ignore_ascii_case("ATOMIC")) => {}
                "BEGIN" => match self.stack.last_mut() {
                    Some(decl @ true) => *decl = false,
                    _ => self.stack.push(false),
                },
                "DECLARE" if oracle => self.stack.push(true),
                "END" => {
                    self.stack.pop();
                }
                "CASE" if !after_end => self.stack.push(false),
                "LOOP" if !after_end && mode != Mode::Postgres => self.stack.push(false),
                "IF" if !after_end && stmt_start && mode != Mode::Postgres => {
                    self.stack.push(false)
                }
                "WHILE" | "REPEAT" if mode == Mode::Sql && !after_end && stmt_start => {
                    self.stack.push(false)
                }
                "PROCEDURE" | "FUNCTION" | "PACKAGE" if oracle => self.header = true,
                "BODY" if oracle && self.prev == "TYPE" => self.header = true,
                "IS" | "AS" if oracle && self.header => {
                    self.header = false;
                    // `AS LANGUAGE JAVA NAME '..'` has no body.
                    if !next_word.is_some_and(|w| {
                        w.eq_ignore_ascii_case("LANGUAGE") || w.eq_ignore_ascii_case("EXTERNAL")
                    }) {
                        self.stack.push(true);
                    }
                }
                _ => {}
            }
        } else if t == ";" {
            self.header = false;
        }
        self.prev = t.to_string();
    }
}

/// Does a statement that starts with the words `head` contain procedural
/// blocks whose inner `;` do not end it?
fn is_block_statement(head: &[String], dialect: Dialect) -> bool {
    let oracle = dialect == Dialect::Oracle;
    let words: Vec<&str> = head.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["BEGIN" | "DECLARE", ..] if oracle => true,
        ["BEGIN", "NOT", "ATOMIC", ..] => !oracle,
        ["CREATE", rest @ ..] => {
            for w in rest {
                match *w {
                    "OR" | "REPLACE" | "TEMP" | "TEMPORARY" | "CONSTRAINT" => {}
                    "EDITIONABLE" | "NONEDITIONABLE" if oracle => {}
                    "PROCEDURE" | "FUNCTION" | "TRIGGER" => return true,
                    "PACKAGE" | "TYPE" => return oracle,
                    _ => return false,
                }
            }
            false
        }
        _ => false,
    }
}

fn floor_boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The index of the statement `caret` (a byte offset) belongs to: the one it
/// is in; between statements the previous one, except in the indentation of
/// the line where the next one starts (the next one); before the first, the
/// first.
fn pick(script: &str, stmts: &[Statement], caret: usize) -> Option<usize> {
    if stmts.is_empty() {
        return None;
    }
    let caret = floor_boundary(script, caret);
    let next_idx = stmts.partition_point(|s| s.range.start <= caret);
    if let Some(next) = stmts.get(next_idx) {
        let line_start = script[..caret].rfind('\n').map_or(0, |i| i + 1);
        let before_blank = script[line_start..caret].trim().is_empty();
        let gap = &script[caret..next.range.start];
        let gap_blank = !gap.contains('\n') && gap.trim().is_empty();
        if next_idx == 0 || (before_blank && gap_blank) {
            return Some(next_idx);
        }
    }
    Some(next_idx - 1)
}

/// The statement at `caret` (a byte offset; clamped, and moved back to a
/// character boundary). Between statements: the previous one, except in the
/// indentation before the next one, which gives the next. `None` for a script
/// without statements.
pub fn statement_at(script: &str, caret: usize, dialect: Dialect) -> Option<Statement> {
    let mut stmts = split(script, dialect);
    let i = pick(script, &stmts, caret)?;
    Some(stmts.swap_remove(i))
}

/// What can be run from a caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateKind {
    /// A parenthesised `SELECT`/`WITH`/`VALUES` around the caret.
    Subquery,
    Statement,
    Script,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    pub range: Range<usize>,
}

/// The things that can be run from `caret`, innermost first: every
/// parenthesised select around it (an unclosed one reads to the end of the
/// statement), then the statement, then the whole script when it is more than
/// the statement. Empty for a script without statements.
pub fn candidates_at(script: &str, caret: usize, dialect: Dialect) -> Vec<Candidate> {
    let stmts = split(script, dialect);
    let Some(i) = pick(script, &stmts, caret) else {
        return Vec::new();
    };
    let caret = floor_boundary(script, caret);
    let stmt = &stmts[i];
    let sig: Vec<Token> = lex(script, dialect)
        .into_iter()
        .filter(|t| !t.is_trivia() && t.start >= stmt.range.start && t.end <= stmt.range.end)
        .collect();

    let mut open: Vec<usize> = Vec::new(); // indices into `sig`
    let mut pairs: Vec<(usize, usize)> = Vec::new(); // (open, close) indices; close == sig.len() when unclosed
    for (k, t) in sig.iter().enumerate() {
        match t.kind {
            TokenKind::LParen => open.push(k),
            TokenKind::RParen => {
                if let Some(o) = open.pop() {
                    pairs.push((o, k));
                }
            }
            _ => {}
        }
    }
    pairs.extend(open.into_iter().map(|o| (o, sig.len())));

    let is_query = |o: usize| {
        let mut k = o + 1;
        while sig.get(k).is_some_and(|t| t.kind == TokenKind::LParen) {
            k += 1;
        }
        sig.get(k).is_some_and(|t| {
            t.kind == TokenKind::Word
                && ["SELECT", "WITH", "VALUES"]
                    .iter()
                    .any(|w| t.text(script).eq_ignore_ascii_case(w))
        })
    };
    let mut subs: Vec<Range<usize>> = Vec::new();
    for (o, c) in pairs {
        let from = sig[o].end;
        let to = sig.get(c).map_or(stmt.range.end, |t| t.start);
        if from <= caret && caret <= to && is_query(o) && o + 1 < c.min(sig.len()) {
            let end = if c < sig.len() {
                sig[c - 1].end
            } else {
                sig[sig.len() - 1].end
            };
            subs.push(sig[o + 1].start..end);
        }
    }
    subs.sort_by_key(|r| (r.len(), r.start));
    subs.dedup();

    let mut out: Vec<Candidate> = subs
        .into_iter()
        .map(|range| Candidate {
            kind: CandidateKind::Subquery,
            range,
        })
        .collect();
    out.push(Candidate {
        kind: CandidateKind::Statement,
        range: stmt.range.clone(),
    });
    let whole = stmts[0].range.start..stmts[stmts.len() - 1].range.end;
    if whole != stmt.range {
        out.push(Candidate {
            kind: CandidateKind::Script,
            range: whole,
        });
    }
    out
}

/// The text in `range` of `script` as one statement, whatever it holds:
/// trimmed, a closing `;` dropped (a PL/SQL block keeps its own), the range
/// narrowed to match. `None` when nothing but blanks and `;` is left.
pub fn statement_of(script: &str, range: Range<usize>, dialect: Dialect) -> Option<Statement> {
    let start = floor_boundary(script, range.start);
    let end = floor_boundary(script, range.end).max(start);
    let piece = &script[start..end];
    let lead = piece.len() - piece.trim_start().len();
    let mut text = piece.trim();
    let block = split(text, dialect)
        .last()
        .is_some_and(|s| s.text.trim_end().ends_with(';'));
    if !block {
        text = text.trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    }
    if text.is_empty() {
        return None;
    }
    let range = start + lead..start + lead + text.len();
    let Classification {
        kind,
        writes,
        danger,
        ddl,
    } = classify_text(text, dialect);
    Some(Statement {
        text: text.to_string(),
        range,
        kind,
        writes,
        danger,
        ddl,
    })
}

/// The statements of the text in `range` (a selection), split as a script of
/// its own; their ranges are offsets into `script`. The ends of `range` are
/// clamped and moved back to character boundaries.
pub fn statements_in(script: &str, range: Range<usize>, dialect: Dialect) -> Vec<Statement> {
    let start = floor_boundary(script, range.start);
    let end = floor_boundary(script, range.end).max(start);
    split(&script[start..end], dialect)
        .into_iter()
        .map(|mut s| {
            s.range = s.range.start + start..s.range.end + start;
            s
        })
        .collect()
}
