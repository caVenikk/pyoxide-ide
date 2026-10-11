//! The syntax layer of Pyoxide IDE (ADR 0007): tree-sitter parsing of a rope,
//! incremental after edits and cancellable, and highlight spans for a range
//! of lines with capture names resolved to the text roles of the `themes`
//! crate (docs/theme-mapping.md, "tree-sitter captures").
//!
//! The crate knows no language: a product supplies the grammar and its
//! highlight query as a [`Language`]. Parsing is blocking and meant for a
//! background thread; highlighting works on the requested lines only.

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

/// A grammar with its highlight query.
pub struct Language {
    name: &'static str,
    grammar: tree_sitter::Language,
    query: Query,
    /// Role of every capture of the query (`None`: not drawn).
    roles: Vec<Option<TextRole>>,
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
        })
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

/// Highlight spans of `lines` (one list per line, sorted, not
/// overlapping). For nested captures the innermost wins; for captures of
/// the same node the first pattern of the query wins.
pub fn highlight(
    language: &Language,
    tree: &Tree,
    rope: &Rope,
    lines: Range<usize>,
) -> Vec<Vec<Span>> {
    let total = rope.len_lines();
    let lines = lines.start.min(total)..lines.end.min(total);
    let mut out: Vec<Vec<Span>> = vec![Vec::new(); lines.len()];
    if lines.is_empty() {
        return out;
    }
    let start_byte = rope.line_to_byte(lines.start);
    let end_byte = if lines.end < total {
        rope.line_to_byte(lines.end)
    } else {
        rope.len_bytes()
    };
    // Skip the query when every line is too long to highlight.
    if (lines.start..lines.end).all(|l| rope.line(l).len_bytes() > MAX_HIGHLIGHT_LINE) {
        return out;
    }
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start_byte..end_byte);
    // Captures without a role (`_name`) take part in "first pattern wins"
    // but are not drawn.
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
        items.push((node.start_byte(), node.end_byte(), m.pattern_index, role));
    }
    // Outer nodes first, so inner ones paint over them; equal ranges keep
    // the first pattern only.
    items.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    items.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
    let mut roles: Vec<Option<TextRole>> = vec![None; end_byte - start_byte];
    for (s, e, _, role) in items {
        let s = s.max(start_byte);
        let e = e.min(end_byte);
        if let Some(role) = role
            && s < e
        {
            roles[s - start_byte..e - start_byte].fill(Some(role));
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
    out
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
