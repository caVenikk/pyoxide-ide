//! The syntax layer of Pyoxide IDE (ADR 0007): tree-sitter parsing of a rope,
//! incremental after edits and cancellable, and highlight spans for a range
//! of lines with capture names resolved to the text roles of the `themes`
//! crate (docs/theme-mapping.md, "tree-sitter captures").
//!
//! The crate knows no language: a product supplies the grammar, its
//! highlight query and a line [`Lexer`] as a [`Language`]. Parsing is
//! blocking and meant for a background thread; highlighting works on the
//! requested lines only.
//!
//! Highlighting has two layers. The lexer colours tokens line by line
//! (keywords, strings, numbers, comments, ...) and never depends on the
//! parse tree, so text the parser cannot make sense of while it is being
//! typed keeps its colours. The tree's captures are painted on top for what
//! needs structure (definitions, calls, docstrings), except where the tree
//! has an ERROR or MISSING node: there the lexer's colours stay and the
//! error is marked.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use text::{AppliedEdit, Rope};
use themes::TextRole;
use tree_sitter::{
    InputEdit, Node, ParseOptions, Parser, Point, Query, QueryCursor, StreamingIterator,
    TextProvider, Tree,
};

pub use tree_sitter;

/// Lines longer than this are not highlighted (minified files).
pub const MAX_HIGHLIGHT_LINE: usize = 20_000;

/// A role for a byte range of one line (bytes from the start of the line).
pub type LexSpan = (Range<usize>, TextRole);

/// The lexical layer of a language: colours one line at a time, carrying a
/// small state from line to line (for example "inside a triple-quoted
/// string"), without a parse tree.
pub trait Lexer: Send + Sync {
    /// The state at the start of a file.
    fn initial_state(&self) -> u32 {
        0
    }

    /// Colours `line` (without its line break), which starts in `state`;
    /// appends spans in order and returns the state at the end of the line.
    fn lex_line(&self, line: &str, state: u32, out: &mut Vec<LexSpan>) -> u32;
}

/// A grammar with its highlight query and lexer.
pub struct Language {
    name: &'static str,
    grammar: tree_sitter::Language,
    query: Query,
    /// Role of every capture of the query (`None`: not drawn).
    roles: Vec<Option<TextRole>>,
    lexer: Option<std::sync::Arc<dyn Lexer>>,
    /// Node kinds whose children are statements (`module`, `block`).
    statement_containers: &'static [&'static str],
}

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Language")
            .field("name", &self.name)
            .finish()
    }
}

impl Language {
    /// Compiles the highlight query. Capture names are resolved with
    /// `extra` first (language-specific names such as `keyword.argument`),
    /// then with the shared capture table of the themes crate.
    pub fn new(
        name: &'static str,
        grammar: tree_sitter::Language,
        highlights: &str,
        extra: &[(&str, TextRole)],
    ) -> Result<Self, String> {
        let query = Query::new(&grammar, highlights).map_err(|e| format!("{name}: {e}"))?;
        let roles = query
            .capture_names()
            .iter()
            .map(|capture| {
                extra
                    .iter()
                    .find(|(n, _)| n == capture)
                    .map(|(_, role)| *role)
                    .or_else(|| TextRole::from_capture(capture))
            })
            .collect();
        Ok(Self {
            name,
            grammar,
            query,
            roles,
            lexer: None,
            statement_containers: &[],
        })
    }

    /// The same language with the node kinds whose children are
    /// statements: around a syntax error the whole statement that contains
    /// it is coloured by the lexer, as the tree's shape there is a guess.
    pub fn with_statement_containers(mut self, kinds: &'static [&'static str]) -> Self {
        self.statement_containers = kinds;
        self
    }

    /// The same language with a lexical layer.
    pub fn with_lexer(mut self, lexer: std::sync::Arc<dyn Lexer>) -> Self {
        self.lexer = Some(lexer);
        self
    }

    pub fn lexer(&self) -> Option<&dyn Lexer> {
        self.lexer.as_deref()
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Capture names that resolve to no role (for tests of queries).
    pub fn unresolved_captures(&self) -> Vec<&str> {
        self.query
            .capture_names()
            .iter()
            .zip(&self.roles)
            .filter(|(n, r)| r.is_none() && !n.starts_with('_'))
            .map(|(n, _)| *n)
            .collect()
    }
}

/// Parses `rope`, reusing `old` (a tree that has been told about every edit
/// since it was produced). Returns `None` when `cancel` was set while
/// parsing.
pub fn parse(
    language: &Language,
    rope: &Rope,
    old: Option<&Tree>,
    cancel: &AtomicBool,
) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&language.grammar).ok()?;
    let len = rope.len_bytes();
    let mut input = |byte: usize, _: Point| -> &[u8] {
        if byte >= len {
            return &[];
        }
        let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - chunk_start..]
    };
    let mut progress = |_: &tree_sitter::ParseState| cancel.load(Ordering::Relaxed);
    let options = ParseOptions::new().progress_callback(&mut progress);
    parser.parse_with_options(&mut input, old, Some(options))
}

/// Byte ranges of the nodes of `tree` that contain `range`, innermost
/// first, without repeats (a parent with the same extent as its child).
pub fn enclosing_nodes(tree: &Tree, range: Range<usize>) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(range.start, range.end);
    while let Some(n) = node {
        let r = n.byte_range();
        if r.start <= range.start && range.end <= r.end && out.last() != Some(&r) {
            out.push(r);
        }
        node = n.parent();
    }
    out
}

/// The tree-sitter form of an applied edit.
pub fn input_edit(e: &AppliedEdit) -> InputEdit {
    let point = |(row, column): (usize, usize)| Point { row, column };
    InputEdit {
        start_byte: e.start_byte,
        old_end_byte: e.old_end_byte,
        new_end_byte: e.new_end_byte,
        start_position: point(e.start_position),
        old_end_position: point(e.old_end_position),
        new_end_position: point(e.new_end_position),
    }
}

/// Node text straight from the rope, for query predicates.
struct RopeText<'a>(&'a Rope);

type Chunks<'a> = std::iter::Map<ropey::iter::Chunks<'a>, fn(&'a str) -> &'a [u8]>;

impl<'a> TextProvider<&'a [u8]> for RopeText<'a> {
    type I = Chunks<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        let range = node.byte_range();
        let end = range.end.min(self.0.len_bytes());
        let start = range.start.min(end);
        self.0
            .byte_slice(start..end)
            .chunks()
            .map(str::as_bytes as fn(&str) -> &[u8])
    }
}

/// A styled range of a line: chars from the start of the line.
pub type Span = (Range<usize>, TextRole);

/// The lexer state at the start of each line, computed as far as needed.
/// An edit invalidates the states after the edited line.
#[derive(Debug, Clone, Default)]
pub struct LineStates {
    /// `states[i]` is the state at the start of line `i`.
    states: Vec<u32>,
}

impl LineStates {
    /// Forgets the states after `line` (its own start state stays valid).
    pub fn invalidate_after(&mut self, line: usize) {
        self.states.truncate(line + 1);
    }

    pub fn clear(&mut self) {
        self.states.clear();
    }

    /// Lines with a known start state.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// The state at the start of `line`, lexing the lines before it that
    /// are not known yet.
    pub fn ensure(&mut self, lexer: &dyn Lexer, rope: &Rope, line: usize) -> u32 {
        if self.states.is_empty() {
            self.states.push(lexer.initial_state());
        }
        let mut text = String::new();
        let mut scratch = Vec::new();
        while self.states.len() <= line && self.states.len() < rope.len_lines() {
            let l = self.states.len() - 1;
            line_text(rope, l, &mut text);
            scratch.clear();
            let next = lexer.lex_line(&text, self.states[l], &mut scratch);
            self.states.push(next);
        }
        self.states[line.min(self.states.len() - 1)]
    }

    fn record(&mut self, line: usize, state: u32) {
        if line == self.states.len() {
            self.states.push(state);
        }
    }
}

/// The text of a line without its line break, into `out`.
fn line_text(rope: &Rope, line: usize, out: &mut String) {
    out.clear();
    for chunk in rope.line(line).chunks() {
        out.push_str(chunk);
    }
    while out.ends_with(['\n', '\r']) {
        out.pop();
    }
}

/// Highlighting of a range of lines: syntax spans and error marks per line
/// (chars from the start of the line).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Highlighted {
    pub spans: Vec<Vec<Span>>,
    pub errors: Vec<Vec<Range<usize>>>,
}

/// Pushes the children of `node` that contain errors, with the statement
/// around each. Of statements (children of a container or of the root) only
/// those that touch `range` are visited; inside a statement every error
/// counts, as it makes the whole statement untrusted.
fn push_children<'t>(
    node: Node<'t>,
    statement: Option<Range<usize>>,
    children_are_statements: bool,
    range: &Range<usize>,
    stack: &mut Vec<(Node<'t>, Option<Range<usize>>)>,
) {
    let mut cursor = node.walk();
    let first = if children_are_statements {
        cursor.goto_first_child_for_byte(range.start)
    } else {
        cursor.goto_first_child().then_some(0)
    };
    if first.is_none() {
        return;
    }
    loop {
        let child = cursor.node();
        if children_are_statements && child.start_byte() > range.end {
            break;
        }
        if child.has_error() || child.is_missing() {
            let statement = if children_are_statements {
                Some(child.byte_range())
            } else {
                statement.clone()
            };
            stack.push((child, statement));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// ERROR nodes (and the tokens before MISSING nodes) that touch a byte
/// range: the ranges where the tree is not trusted (the ERROR nodes and the
/// statements around them: children of a statement container or of the
/// root), and the ranges to mark. Only nodes with errors that touch the
/// range are visited.
fn error_regions(
    tree: &Tree,
    rope: &Rope,
    range: Range<usize>,
    containers: &[&str],
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut untrusted = Vec::new();
    let mut marks = Vec::new();
    let root = tree.root_node();
    if !root.has_error() {
        return (untrusted, marks);
    }
    // (node, the statement around it)
    let mut stack: Vec<(Node, Option<Range<usize>>)> = Vec::new();
    push_children(root, None, true, &range, &mut stack);
    while let Some((node, statement)) = stack.pop() {
        if node.is_error() {
            let r = node.byte_range();
            // Mark the rest of the first line of the error.
            let line = rope.byte_to_line(r.start.min(rope.len_bytes()));
            let line_end = rope.line_to_byte(line) + rope.line(line).len_bytes();
            let mut end = r.end.min(line_end);
            while end > r.start && matches!(rope.byte(end - 1), b'\n' | b'\r' | b' ' | b'\t') {
                end -= 1;
            }
            if end > r.start {
                marks.push(r.start..end);
            }
            untrusted.push(r);
            untrusted.extend(statement);
            continue;
        }
        if node.is_missing() {
            untrusted.extend(statement);
            // Mark the word (or character) before the missing token.
            let at = node.start_byte().min(rope.len_bytes());
            let mut start = at;
            while start > 0 && {
                let b = rope.byte(start - 1);
                b.is_ascii_alphanumeric() || b == b'_'
            } {
                start -= 1;
            }
            if start == at && at > 0 {
                start = at - 1;
            }
            if start < at {
                marks.push(start..at);
            }
            continue;
        }
        let children_are_statements = containers.contains(&node.kind());
        push_children(node, statement, children_are_statements, &range, &mut stack);
    }
    (untrusted, marks)
}

/// Highlight spans of `lines` (one list per line, sorted, not
/// overlapping) and error marks. The lexer colours first; captures of
/// `tree` (if there is one yet) paint over it, except in ERROR regions. For
/// nested captures the innermost wins; for captures of the same node the
/// first pattern of the query wins.
pub fn highlight(
    language: &Language,
    tree: Option<&Tree>,
    rope: &Rope,
    lines: Range<usize>,
    states: &mut LineStates,
) -> Highlighted {
    let total = rope.len_lines();
    let lines = lines.start.min(total)..lines.end.min(total);
    let mut out: Vec<Vec<Span>> = vec![Vec::new(); lines.len()];
    let mut errors: Vec<Vec<Range<usize>>> = vec![Vec::new(); lines.len()];
    if lines.is_empty() {
        return Highlighted { spans: out, errors };
    }
    let start_byte = rope.line_to_byte(lines.start);
    let end_byte = if lines.end < total {
        rope.line_to_byte(lines.end)
    } else {
        rope.len_bytes()
    };
    let mut roles: Vec<Option<TextRole>> = vec![None; end_byte - start_byte];
    // The lexical layer.
    if let Some(lexer) = language.lexer() {
        let mut state = states.ensure(lexer, rope, lines.start);
        let mut text = String::new();
        let mut spans = Vec::new();
        for line in lines.clone() {
            line_text(rope, line, &mut text);
            spans.clear();
            let next = lexer.lex_line(&text, state, &mut spans);
            states.record(line + 1, next);
            state = next;
            if text.len() > MAX_HIGHLIGHT_LINE {
                continue;
            }
            let base = rope.line_to_byte(line) - start_byte;
            for (r, role) in &spans {
                let s = (base + r.start).min(roles.len());
                let e = (base + r.end).min(roles.len());
                roles[s..e].fill(Some(*role));
            }
        }
    }
    // The tree's captures on top, outside ERROR regions.
    let too_long = (lines.start..lines.end).all(|l| rope.line(l).len_bytes() > MAX_HIGHLIGHT_LINE);
    if let Some(tree) = tree.filter(|_| !too_long) {
        let (untrusted, marks) = error_regions(
            tree,
            rope,
            start_byte..end_byte,
            language.statement_containers,
        );
        let overlaps = |s: usize, e: usize| {
            untrusted
                .iter()
                .any(|r| s < r.end.max(r.start + 1) && r.start < e.max(s + 1))
        };
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start_byte..end_byte);
        // Captures without a role (`_name`) take part in "first pattern
        // wins" but are not drawn.
        let mut items: Vec<(usize, usize, usize, Option<TextRole>)> = Vec::new();
        let mut captures = cursor.captures(&language.query, tree.root_node(), RopeText(rope));
        while let Some((m, index)) = captures.next() {
            let capture = m.captures[*index];
            let role = language
                .roles
                .get(capture.index as usize)
                .copied()
                .flatten();
            let node = capture.node;
            if overlaps(node.start_byte(), node.end_byte()) {
                continue;
            }
            items.push((node.start_byte(), node.end_byte(), m.pattern_index, role));
        }
        // Outer nodes first, so inner ones paint over them; equal ranges
        // keep the first pattern only.
        items.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        items.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
        paint_items(&mut roles, items, start_byte, end_byte);
        for m in marks {
            let s = m.start.max(start_byte);
            let e = m.end.min(end_byte);
            if s >= e {
                continue;
            }
            let line = rope.byte_to_line(s);
            if !lines.contains(&line) {
                continue;
            }
            let line_start = rope.line_to_byte(line);
            let col = |b: usize| rope.byte_to_char(b) - rope.byte_to_char(line_start);
            let line_end = line_start + rope.line(line).len_bytes();
            errors[line - lines.start].push(col(s)..col(e.min(line_end)));
        }
    }
    for (i, line) in lines.clone().enumerate() {
        let slice = rope.line(line);
        if slice.len_bytes() > MAX_HIGHLIGHT_LINE {
            continue;
        }
        let line_byte = rope.line_to_byte(line) - start_byte;
        let spans = &mut out[i];
        let mut byte = line_byte;
        for (col, c) in slice.chars().enumerate() {
            if c == '\n' {
                break;
            }
            let role = roles.get(byte).copied().flatten();
            byte += c.len_utf8();
            let Some(role) = role else { continue };
            match spans.last_mut() {
                Some((range, last)) if *last == role && range.end == col => range.end = col + 1,
                _ => spans.push((col..col + 1, role)),
            }
        }
    }
    Highlighted { spans: out, errors }
}

/// Paints capture roles (outer first) into the byte roles of a range.
fn paint_items(
    roles: &mut [Option<TextRole>],
    items: Vec<(usize, usize, usize, Option<TextRole>)>,
    start_byte: usize,
    end_byte: usize,
) {
    for (s, e, _, role) in items {
        let s = s.max(start_byte);
        let e = e.min(end_byte);
        if let Some(role) = role
            && s < e
        {
            roles[s - start_byte..e - start_byte].fill(Some(role));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_edits_carry_bytes_and_points() {
        let e = AppliedEdit {
            start: 1,
            old_end: 2,
            new_end: 4,
            start_byte: 1,
            old_end_byte: 2,
            new_end_byte: 4,
            start_position: (0, 1),
            old_end_position: (0, 2),
            new_end_position: (1, 1),
        };
        let ie = input_edit(&e);
        assert_eq!(ie.new_end_position, Point { row: 1, column: 1 });
        assert_eq!(ie.old_end_byte, 2);
    }
}
