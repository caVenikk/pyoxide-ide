//! Soft wraps without a toolkit: where a line wider than the view goes on
//! in the next visual row, and how many rows every line takes.
//!
//! The wrap positions follow the reference's soft wrap engine
//! (`SoftWrapEngine`, `LineWrapPositionStrategy`,
//! `GenericLineWrapPositionStrategy`; IntelliJ Platform Community,
//! Apache-2.0, see NOTICE), measured in display columns of the editor's
//! monospace font instead of pixels: a row ends where its next char would
//! pass the view's width; the wrap goes after a space within ten chars
//! before the room kept for the wrap sign, else to the best place the
//! file type's rules give, else at that room's edge. A continuation row
//! starts after the sign and, with the original line's indent, after the
//! line's indentation too.
//!
//! [`WrapMap`] keeps the number of rows of every line with prefix sums
//! per block of lines, so that a row is found from a line and a line from
//! a row without walking the whole text; edits update the lines they touch.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use text::movement::char_width;
use text::{Rope, RopeSlice};

/// Where a rule lets a line wrap around its symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WrapCondition {
    /// After the symbol.
    After,
    /// Before the symbol.
    Before,
    /// Before or after the symbol.
    Both,
}

/// A symbol a line may wrap at when no space is near (a file type's line
/// wrap strategy, the reference's `GenericLineWrapPositionStrategy.Rule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WrapRule {
    pub symbol: char,
    pub condition: WrapCondition,
    /// Of two places, the one with the larger weight times the length of
    /// the row it leaves wins; in tenths of the reference's weights.
    pub weight: u32,
}

impl WrapRule {
    /// The reference's default weight (10), in tenths.
    pub const DEFAULT_WEIGHT: u32 = 100;

    pub const fn new(symbol: char, condition: WrapCondition) -> Self {
        Self {
            symbol,
            condition,
            weight: Self::DEFAULT_WEIGHT,
        }
    }

    pub const fn weighted(symbol: char, condition: WrapCondition, weight: u32) -> Self {
        Self {
            symbol,
            condition,
            weight,
        }
    }

    /// The rules for any text (the reference's
    /// `DefaultLineWrapPositionStrategy`).
    pub fn defaults() -> Vec<WrapRule> {
        use WrapCondition::*;
        vec![
            WrapRule::new(',', After),
            WrapRule::new(' ', Both),
            WrapRule::new('\t', Both),
            WrapRule::new(';', After),
            WrapRule::new(')', After),
            WrapRule::new('(', Before),
            WrapRule::new('.', Before),
        ]
    }
}

/// Weight of a place between an identifier char and another char (the
/// reference's `NON_ID_WEIGHT`, 4 after its integer arithmetic).
const NON_ID_WEIGHT: u32 = 40;
/// Chars looked at before the wrap position for a space (the reference's
/// `BASIC_LOOK_BACK_LENGTH`).
const LOOK_BACK: usize = 10;

/// What wraps are computed for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WrapParams {
    /// Display columns of the view (the reference's visible width).
    pub width: usize,
    /// Columns kept free at the end of a wrapped row for the sign before
    /// the wrap.
    pub before_sign: usize,
    /// Columns of the sign after the wrap: the least indent of a
    /// continuation row.
    pub after_sign: usize,
    pub tab_size: usize,
    /// Continuation rows are also indented as their line plus this many
    /// columns ("use original line's indent"); `None` indents them by the
    /// sign only.
    pub relative_indent: Option<usize>,
    pub rules: Arc<[WrapRule]>,
}

/// A soft wrap: the line goes on at char column `col` in a row indented
/// by `indent` display columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wrap {
    pub col: usize,
    pub indent: usize,
}

/// One visual row of a line: char columns `start..end` after `indent`
/// display columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Segment {
    pub start: usize,
    pub end: usize,
    pub indent: usize,
}

/// The rows of a line of `len` chars with `wraps`.
pub(crate) fn segments(wraps: &[Wrap], len: usize) -> Vec<Segment> {
    let mut out = Vec::with_capacity(wraps.len() + 1);
    let mut start = 0;
    let mut indent = 0;
    for w in wraps {
        out.push(Segment {
            start,
            end: w.col,
            indent,
        });
        start = w.col;
        indent = w.indent;
    }
    out.push(Segment {
        start,
        end: len,
        indent,
    });
    out
}

/// The row of `wraps` that shows char column `col`; at a wrap, the row it
/// starts, or with `lean` the row it ends.
pub(crate) fn row_of_col(wraps: &[Wrap], col: usize, lean: bool) -> usize {
    let k = wraps.partition_point(|w| w.col <= col);
    if lean && k > 0 && wraps[k - 1].col == col {
        k - 1
    } else {
        k
    }
}

/// Display column (from the left edge of the text, the row's indent
/// included) of char column `col` in `seg` of `line`.
pub(crate) fn visual_col(line: RopeSlice<'_>, seg: Segment, col: usize, tab_size: usize) -> usize {
    let mut x = seg.indent;
    let col = col.clamp(seg.start, seg.end);
    for c in line.chars_at(seg.start).take(col - seg.start) {
        x += char_width(c, x, tab_size);
    }
    x
}

/// The char column of `seg` nearest to display column `target`.
pub(crate) fn col_at_visual(
    line: RopeSlice<'_>,
    seg: Segment,
    target: usize,
    tab_size: usize,
) -> usize {
    let mut x = seg.indent;
    if target <= x {
        return seg.start;
    }
    for (i, c) in line
        .chars_at(seg.start)
        .take(seg.end - seg.start)
        .enumerate()
    {
        let w = char_width(c, x, tab_size);
        if target < x + w {
            let i = seg.start + i;
            return if (target - x) * 2 < w { i } else { i + 1 };
        }
        x += w;
    }
    seg.end
}

/// Whether a line (without its line break) fits in `width` columns
/// without looking at every char: no char but a tab is wider than its
/// UTF-8 bytes.
fn fits(line: RopeSlice<'_>, width: usize) -> bool {
    line.len_bytes() <= width && !line.chunks().any(|c| c.as_bytes().contains(&b'\t'))
}

/// The soft wraps of a line (without its line break).
pub(crate) fn line_wraps(line: RopeSlice<'_>, p: &WrapParams) -> Vec<Wrap> {
    if fits(line, p.width) {
        return Vec::new();
    }
    let chars: Vec<char> = line.chars().collect();
    wraps_of(&chars, p)
}

/// The rows of a line (without its line break); `scratch` holds its chars
/// when it has to be looked at.
pub(crate) fn line_rows(line: RopeSlice<'_>, p: &WrapParams, scratch: &mut Vec<char>) -> u32 {
    if fits(line, p.width) {
        return 1;
    }
    scratch.clear();
    scratch.extend(line.chars());
    wraps_of(scratch, p).len() as u32 + 1
}

/// The wraps of a line's chars (the reference's `SoftWrapEngine.generate`
/// for one logical line, one char per element).
fn wraps_of(chars: &[char], p: &WrapParams) -> Vec<Wrap> {
    let n = chars.len();
    let width = p.width.max(1);
    let max_at_wrap = width.saturating_sub(p.before_sign);
    let mut wraps: Vec<Wrap> = Vec::new();
    let mut x = 0usize;
    let mut min_wrap: Option<usize> = None;
    let mut max_wrap: Option<usize> = None;
    // Char column and display column of the first non-blank char.
    let mut first_text: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if p.relative_indent.is_some() && first_text.is_none() && c != ' ' && c != '\t' {
            first_text = Some((i, x));
        }
        x += char_width(c, x, p.tab_size);
        let Some(min) = min_wrap else {
            // A row holds at least one char.
            min_wrap = Some(i + 1);
            i += 1;
            continue;
        };
        if x > max_at_wrap && max_wrap.is_none() {
            max_wrap = Some(i);
        }
        if x > width {
            let max = max_wrap.unwrap_or(i);
            let at = if min >= max {
                min
            } else {
                wrap_offset(chars, min, max, &p.rules)
            };
            let indent = match (p.relative_indent, wraps.last()) {
                (None, _) => p.after_sign,
                (Some(_), Some(last)) => last.indent,
                (Some(extra), None) => {
                    let text = first_text.filter(|(col, _)| *col < at).map_or(0, |t| t.1);
                    p.after_sign + text + extra
                }
            };
            wraps.push(Wrap { col: at, indent });
            x = indent;
            min_wrap = None;
            max_wrap = None;
            // The chars from the wrap on are measured again in the new row.
            i = at;
            continue;
        }
        i += 1;
    }
    wraps
}

/// A space, a tab or a char of the eastern scripts: a line may wrap there
/// (`LineWrapPositionStrategy.canWrapLineAtOffset`).
fn can_wrap_at(c: char) -> bool {
    c == ' ' || c == '\t' || ('\u{2f00}'..'\u{10000}').contains(&c)
}

/// Where to wrap between columns `min` and `max` (both included).
fn wrap_offset(chars: &[char], min: usize, max: usize, rules: &[WrapRule]) -> usize {
    if let Some(at) = find_wrap_position(chars, max, min) {
        return at;
    }
    match strategy_position(chars, min - 1, max + 1, max, rules) {
        Some(at) => at.clamp(min, max),
        None => max,
    }
}

/// At `max` when a space is there, else after a space at most ten chars
/// before it.
fn find_wrap_position(chars: &[char], max: usize, min: usize) -> Option<usize> {
    if can_wrap_at(chars[max]) {
        return Some(max);
    }
    let mut offset = max;
    for _ in 0..LOOK_BACK {
        if offset < min {
            break;
        }
        let prev = offset - 1;
        if can_wrap_at(chars[prev]) {
            return Some(offset);
        }
        offset = prev;
    }
    None
}

fn is_id(c: char) -> bool {
    c == '_' || c == '$' || c.is_ascii_alphanumeric()
}

/// The best place by the rules in `(start, max_preferred]` (the
/// reference's `GenericLineWrapPositionStrategy.calculateWrapPosition`
/// without going past the preferred end).
fn strategy_position(
    chars: &[char],
    start: usize,
    end: usize,
    max_preferred: usize,
    rules: &[WrapRule],
) -> Option<usize> {
    if end <= start {
        return Some(end);
    }
    let rule = |c: char| rules.iter().find(|r| r.symbol == c);
    let max_pref = max_preferred.min(end - 1).max(start);
    // Weights by offset from `start`.
    let mut weights = vec![0u32; max_pref - start + 2];
    let mut store = |at: usize, w: u32| weights[at - start] = w;
    let mut i = max_pref.min(chars.len().saturating_sub(1));
    while i > start {
        let c = chars[i];
        if let Some(r) = rule(c) {
            if matches!(r.condition, WrapCondition::Both | WrapCondition::After) {
                let mut target = i + 1;
                if r.symbol != ' ' {
                    while target < max_pref && chars[target] == ' ' {
                        target += 1;
                    }
                }
                if target <= max_pref {
                    store(target, r.weight);
                }
            }
            if matches!(r.condition, WrapCondition::Both | WrapCondition::Before) {
                store(i, r.weight);
            }
        } else if !is_id(c) && i > start + 1 && is_id(chars[i - 1]) && rule(chars[i - 1]).is_none()
        {
            store(i, NON_ID_WEIGHT);
        }
        i -= 1;
    }
    // The last place, unless an earlier one weighs more for its length.
    let (mut best_w, mut best) = (0u32, 0usize);
    for (idx, &w) in weights.iter().enumerate().rev() {
        if w == 0 {
            continue;
        }
        if best_w == 0 || (best_w < w && w as usize * idx > best * best_w as usize) {
            (best_w, best) = (w, idx);
        }
    }
    (best_w > 0).then_some(start + best)
}

/// The rows of every line of `rope`, or `None` when cancelled.
pub(crate) fn compute_rows(rope: &Rope, p: &WrapParams, cancel: &AtomicBool) -> Option<Vec<u32>> {
    let mut rows = Vec::with_capacity(rope.len_lines());
    let mut scratch = Vec::new();
    for (i, line) in rope.lines().enumerate() {
        if i % 4096 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let len = line.len_chars();
        let line = if len > 0 && line.char(len - 1) == '\n' {
            line.slice(..len - 1)
        } else {
            line
        };
        rows.push(line_rows(line, p, &mut scratch));
    }
    Some(rows)
}

/// Lines per block of the prefix sums.
const BLOCK: usize = 256;

/// Lines replaced by an edit: `start..=old_end` before it are
/// `start..=new_end` after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineEdit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

/// The visual rows of every line.
#[derive(Debug, Clone, Default)]
pub(crate) struct WrapMap {
    /// Rows of each line (at least one; zero only while being updated).
    rows: Vec<u32>,
    /// Rows before each block of `BLOCK` lines, and the total at the end.
    prefix: Vec<usize>,
}

impl WrapMap {
    pub fn new(rows: Vec<u32>) -> Self {
        let mut map = Self {
            rows,
            prefix: Vec::new(),
        };
        map.rebuild_from(0);
        map
    }

    /// One row for each of `lines` lines.
    pub fn uniform(lines: usize) -> Self {
        Self::new(vec![1; lines])
    }

    pub fn lines(&self) -> usize {
        self.rows.len()
    }

    /// Rows of all lines.
    pub fn total(&self) -> usize {
        *self.prefix.last().unwrap_or(&0)
    }

    pub fn rows(&self, line: usize) -> usize {
        self.rows.get(line).map_or(1, |r| *r as usize)
    }

    /// The first row of `line` (the total past the last line).
    pub fn row_of_line(&self, line: usize) -> usize {
        let line = line.min(self.rows.len());
        let block = line / BLOCK;
        let within: usize = self.rows[block * BLOCK..line]
            .iter()
            .map(|r| *r as usize)
            .sum();
        self.prefix[block] + within
    }

    /// The first rows of lines asked for in increasing order, each found
    /// from the previous one.
    pub fn cursor(&self) -> impl FnMut(usize) -> usize + '_ {
        let (mut line, mut row) = (0usize, 0usize);
        move |target: usize| {
            let target = target.min(self.rows.len());
            if target < line || target - line > BLOCK {
                (line, row) = (target, self.row_of_line(target));
            } else {
                row += self.rows[line..target]
                    .iter()
                    .map(|r| *r as usize)
                    .sum::<usize>();
                line = target;
            }
            row
        }
    }

    /// The line showing `row` and the row's index in it (the last row of
    /// the last line past the end).
    pub fn line_of_row(&self, row: usize) -> (usize, usize) {
        if self.rows.is_empty() {
            return (0, 0);
        }
        if row >= self.total() {
            let last = self.rows.len() - 1;
            return (last, self.rows(last) - 1);
        }
        // The last block starting at or before `row`.
        let block = self.prefix.partition_point(|p| *p <= row) - 1;
        let mut at = self.prefix[block];
        let mut line = block * BLOCK;
        loop {
            let r = self.rows[line] as usize;
            if row < at + r {
                return (line, row - at);
            }
            at += r;
            line += 1;
        }
    }

    /// Applies line edits in the order they were made; the lines they
    /// leave get their rows from `rows_of` (in the text after all of
    /// them). Returns false, leaving the map as it was up to the edit,
    /// when an edit does not fit it (the map is out of step with the
    /// text).
    #[must_use]
    pub fn edit(&mut self, edits: &[LineEdit], mut rows_of: impl FnMut(usize) -> u32) -> bool {
        let mut dirty: Option<Range<usize>> = None;
        for e in edits {
            let old = e.start..e.old_end + 1;
            let new_len = e.new_end + 1 - e.start;
            if old.end > self.rows.len() || e.old_end < e.start || e.new_end < e.start {
                return false;
            }
            if old.len() == new_len {
                self.rows[old.clone()].fill(0);
            } else {
                self.rows
                    .splice(old.clone(), std::iter::repeat_n(0, new_len));
            }
            let delta = new_len as isize - old.len() as isize;
            let shift = |at: usize| -> usize {
                if at > e.old_end {
                    (at as isize + delta) as usize
                } else {
                    at
                }
            };
            dirty = Some(match dirty {
                None => e.start..e.new_end + 1,
                Some(d) => {
                    let start = shift(d.start).min(e.start);
                    let end = if d.end > e.old_end + 1 {
                        (d.end as isize + delta) as usize
                    } else {
                        d.end
                    };
                    start..end.max(e.new_end + 1)
                }
            });
        }
        let Some(dirty) = dirty else { return true };
        let dirty = dirty.start.min(self.rows.len())..dirty.end.min(self.rows.len());
        for line in dirty.clone() {
            if self.rows[line] == 0 {
                self.rows[line] = rows_of(line).max(1);
            }
        }
        self.rebuild_from(dirty.start / BLOCK);
        true
    }

    /// Makes the map `lines` lines long (any lines added have one row).
    pub fn resize(&mut self, lines: usize) {
        if lines != self.rows.len() {
            let from = lines.min(self.rows.len()) / BLOCK;
            self.rows.resize(lines, 1);
            self.rebuild_from(from);
        }
    }

    fn rebuild_from(&mut self, block: usize) {
        let blocks = self.rows.len().div_ceil(BLOCK);
        self.prefix.resize(blocks + 1, 0);
        let block = block.min(blocks);
        let mut at = if block == 0 { 0 } else { self.prefix[block] };
        for b in block..blocks {
            self.prefix[b] = at;
            let end = ((b + 1) * BLOCK).min(self.rows.len());
            at += self.rows[b * BLOCK..end]
                .iter()
                .map(|r| *r as usize)
                .sum::<usize>();
        }
        self.prefix[blocks] = at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(width: usize) -> WrapParams {
        WrapParams {
            width,
            before_sign: 1,
            after_sign: 1,
            tab_size: 4,
            relative_indent: Some(0),
            rules: WrapRule::defaults().into(),
        }
    }

    /// The rows of `text` as strings, each after its indent in spaces.
    fn rows(text: &str, p: &WrapParams) -> Vec<String> {
        let rope = Rope::from_str(text);
        let wraps = line_wraps(rope.slice(..), p);
        let chars: Vec<char> = text.chars().collect();
        segments(&wraps, chars.len())
            .iter()
            .map(|s| {
                let mut row = " ".repeat(s.indent);
                row.extend(&chars[s.start..s.end]);
                row
            })
            .collect()
    }

    #[test]
    fn a_line_that_fits_is_one_row() {
        assert_eq!(rows("print(1)", &params(8)), ["print(1)"]);
        assert_eq!(rows("", &params(8)), [""]);
    }

    #[test]
    fn words_wrap_after_the_space_before_the_edge() {
        // Width 12, the last column kept for the wrap sign.
        let p = params(12);
        assert_eq!(
            rows("alpha beta gamma delta", &p),
            ["alpha beta ", " gamma delta"]
        );
        // Every row of the rest fits in the width (the last one up to the
        // edge, the others before the sign's column).
        for r in rows("one two three four five six seven eight nine ten", &p) {
            assert!(r.chars().count() <= 12, "{r:?}");
        }
    }

    #[test]
    fn continuation_rows_keep_the_line_indent() {
        let p = params(16);
        assert_eq!(
            rows("    value = first + second + third", &p),
            [
                "    value = ",
                "     first + ",
                "     second + ",
                "     third"
            ]
        );
        // Without the original indent: the sign's column only.
        let p = WrapParams {
            relative_indent: None,
            ..params(16)
        };
        assert_eq!(rows("    value = first + second", &p)[1], " first + second");
    }

    #[test]
    fn a_long_word_breaks_at_the_rules_or_the_edge() {
        let p = params(10);
        // No space near: after the comma (the rules of any text).
        assert_eq!(rows("abcdef,ghijklmnop", &p)[0], "abcdef,");
        // Before a dot.
        assert_eq!(rows("abcdefg.hijklmnop", &p)[0], "abcdefg");
        // Nothing at all: at the sign's column.
        assert_eq!(rows("abcdefghijklmnopqrstu", &p)[0], "abcdefghi");
    }

    #[test]
    fn tabs_and_wide_chars_take_their_columns() {
        let p = WrapParams {
            relative_indent: None,
            ..params(10)
        };
        // A tab reaches the next stop: 4 + 6 chars fill the width, one
        // more does not.
        assert_eq!(rows("\tabcdef", &p).len(), 1);
        assert_eq!(rows("\tabcdefg", &p).len(), 2);
        // Wide chars count two columns each; they may wrap anywhere.
        let wide = rows("日本語の文章を折り返す", &p);
        assert_eq!(wide, ["日本語の", " 文章を折", " り返す"]);
    }

    #[test]
    fn a_row_holds_a_char_even_when_the_indent_fills_the_view() {
        let p = params(8);
        let r = rows("     ab cdefghijkl", &p);
        assert_eq!(r[0], "     ab");
        assert!(r.len() > 5);
        for row in &r[1..] {
            assert!(row.starts_with("      "), "{row:?}");
            assert!(row.chars().count() > 6, "{row:?}");
        }
    }

    #[test]
    fn visual_columns_and_back() {
        let p = params(16);
        let text = "    alpha beta gamma";
        let rope = Rope::from_str(text);
        let line = rope.slice(..);
        let wraps = line_wraps(line, &p);
        let segs = segments(&wraps, text.len());
        assert_eq!(segs.len(), 2);
        let second = segs[1];
        assert_eq!(second.indent, 5);
        // The first char of the second row is right after its indent.
        assert_eq!(visual_col(line, second, second.start, 4), 5);
        assert_eq!(col_at_visual(line, second, 5, 4), second.start);
        assert_eq!(col_at_visual(line, second, 0, 4), second.start);
        assert_eq!(col_at_visual(line, second, 100, 4), second.end);
        // At the wrap: the second row, or the first leaning back.
        assert_eq!(row_of_col(&wraps, second.start, false), 1);
        assert_eq!(row_of_col(&wraps, second.start, true), 0);
        assert_eq!(row_of_col(&wraps, 0, true), 0);
    }

    #[test]
    fn the_map_finds_rows_and_lines() {
        let map = WrapMap::new((0..1000).map(|i| (i % 3 + 1) as u32).collect());
        let mut row = 0;
        for line in 0..1000 {
            assert_eq!(map.row_of_line(line), row);
            for sub in 0..map.rows(line) {
                assert_eq!(map.line_of_row(row + sub), (line, sub));
            }
            row += map.rows(line);
        }
        assert_eq!(map.total(), row);
        assert_eq!(map.line_of_row(row + 5), (999, map.rows(999) - 1));
        assert_eq!(map.row_of_line(1000), row);
    }

    #[test]
    fn edits_update_the_lines_they_touch() {
        let mut map = WrapMap::new(vec![1; 600]);
        // Line 300 becomes three lines of two rows; line 10 gets three
        // rows; then lines 500..=502 (after the first edit) become one.
        let applied = map.edit(
            &[
                LineEdit {
                    start: 300,
                    old_end: 300,
                    new_end: 302,
                },
                LineEdit {
                    start: 10,
                    old_end: 10,
                    new_end: 10,
                },
                LineEdit {
                    start: 500,
                    old_end: 502,
                    new_end: 500,
                },
            ],
            |line| match line {
                300..=302 => 2,
                10 => 3,
                _ => 5,
            },
        );
        assert!(applied);
        assert_eq!(map.lines(), 600);
        assert_eq!(map.rows(10), 3);
        assert_eq!(
            (300..=302).map(|l| map.rows(l)).collect::<Vec<_>>(),
            [2, 2, 2]
        );
        assert_eq!(map.rows(500), 5);
        assert_eq!(map.rows(501), 1);
        assert_eq!(map.total(), 600 + 2 + 3 + 4);
        let fresh = WrapMap::new((0..600).map(|l| map.rows(l) as u32).collect());
        for line in 0..600 {
            assert_eq!(map.row_of_line(line), fresh.row_of_line(line));
        }
        // An edit past the end leaves the map to be computed again.
        let edit = LineEdit {
            start: 599,
            old_end: 600,
            new_end: 599,
        };
        assert!(!map.edit(&[edit], |_| 1));
    }

    #[test]
    fn all_rows_of_a_text_off_the_ui_thread() {
        let p = params(12);
        let text = "short\nalpha beta gamma delta\n\n    value = first + second + third\n";
        let rope = Rope::from_str(text);
        let rows = compute_rows(&rope, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(rows, [1, 2, 1, 6, 1]);
        assert!(compute_rows(&rope, &p, &AtomicBool::new(true)).is_none());
    }
}
