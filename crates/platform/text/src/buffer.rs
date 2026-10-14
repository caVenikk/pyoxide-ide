//! The text buffer: a rope with a version counter, the file's line separator
//! and encoding, and the undo history.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use ropey::{Rope, RopeSlice};
use serde::{Deserialize, Serialize};

use crate::encoding::{Decoded, Encoding, LineEnding, normalize_line_endings};
use crate::history::{EditKind, History, Replacement};
use crate::selection::{Selection, SelectionSet};

/// A position as line and column, both zero-based; the column counts chars.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Point {
    pub line: usize,
    pub column: usize,
}

impl Point {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// A replacement of a char range (in the text before the edit) with text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

impl Edit {
    pub fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }

    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self::new(at..at, text)
    }

    pub fn delete(range: Range<usize>) -> Self {
        Self::new(range, "")
    }
}

/// One replacement as applied, in the coordinates of the text at that
/// moment. Replacements of one change are reported in the order they were
/// applied (from the end of the text to the start), so applying them one by
/// one to a syntax tree, a highlight map or a copy of the text keeps every
/// position valid.
///
/// An edit is self-contained: `start..old_end` of the text before it is
/// replaced with `text`. Positions are given in chars, bytes, row and byte
/// column (tree-sitter, LSP `utf-8`) and row and UTF-16 column (LSP
/// `utf-16`), so a language server gets one incremental change per edit
/// without looking at the text in between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEdit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    /// Row and byte column (the form tree-sitter uses).
    pub start_position: (usize, usize),
    pub old_end_position: (usize, usize),
    pub new_end_position: (usize, usize),
    /// UTF-16 column of `start` in its row, in the text before the edit.
    pub start_utf16_col: usize,
    /// UTF-16 column of `old_end` in its row (`old_end_position.0`), in the
    /// text before the edit.
    pub old_end_utf16_col: usize,
    /// The inserted text (empty for a deletion), with `\n` line breaks.
    pub text: Arc<str>,
}

/// The state of a buffer when a write started ([`Buffer::begin_save`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavePoint(u64);

/// Where carets go after [`Buffer::transact`].
#[derive(Debug, Clone)]
pub enum CaretPlacement {
    /// One caret after the inserted text of each edit (at the start of a
    /// deletion).
    AfterEachEdit,
    /// These selections, in the coordinates of the text after the edit.
    Explicit(SelectionSet),
}

/// Text of a document with its history.
#[derive(Debug, Clone)]
pub struct Buffer {
    rope: Rope,
    version: u64,
    line_ending: LineEnding,
    encoding: Encoding,
    read_only: bool,
    history: History,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl Buffer {
    /// A buffer with `text`; any line separators are normalised to `\n`.
    pub fn new(text: &str) -> Self {
        let line_ending = LineEnding::detect(text).unwrap_or(LineEnding::Lf);
        Self {
            rope: Rope::from_str(&normalize_line_endings(text)),
            version: 0,
            line_ending,
            encoding: Encoding::Utf8,
            read_only: false,
            history: History::new(),
        }
    }

    /// A buffer for a decoded file; lossy decodes are read-only.
    pub fn from_decoded(decoded: Decoded) -> Self {
        Self {
            rope: Rope::from_str(&decoded.text),
            version: 0,
            line_ending: decoded.line_ending,
            encoding: decoded.encoding,
            read_only: decoded.lossy,
            history: History::new(),
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Increases with every change of the text (also undo and redo).
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.line_ending = line_ending;
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// The whole text with `\n` breaks.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The bytes to write to disk (line separator and encoding restored).
    pub fn encoded(&self) -> Vec<u8> {
        crate::encoding::encode(&self.text(), self.line_ending, self.encoding)
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Number of lines; an empty text and a text ending in `\n` count the
    /// empty last line.
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Line `line` without its line break.
    pub fn line(&self, line: usize) -> RopeSlice<'_> {
        let slice = self.rope.line(line);
        let len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            slice.slice(..len - 1)
        } else {
            slice
        }
    }

    /// Line `line` without its line break, as a string.
    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        let slice = self.line(line);
        match slice.as_str() {
            Some(s) => Cow::Borrowed(s),
            None => Cow::Owned(slice.to_string()),
        }
    }

    /// Chars of line `line`, without the line break.
    pub fn line_len(&self, line: usize) -> usize {
        self.line(line).len_chars()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    pub fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_len(line)
    }

    pub fn char_to_line(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.len_chars()))
    }

    pub fn offset_to_point(&self, pos: usize) -> Point {
        let pos = pos.min(self.len_chars());
        let line = self.rope.char_to_line(pos);
        Point::new(line, pos - self.rope.line_to_char(line))
    }

    /// The offset of a point; lines and columns past the end are clamped.
    pub fn point_to_offset(&self, point: Point) -> usize {
        let line = point.line.min(self.len_lines().saturating_sub(1));
        self.line_start(line) + point.column.min(self.line_len(line))
    }

    pub fn char_to_byte(&self, pos: usize) -> usize {
        self.rope.char_to_byte(pos)
    }

    pub fn byte_to_char(&self, byte: usize) -> usize {
        self.rope.byte_to_char(byte)
    }

    pub fn char_to_utf16(&self, pos: usize) -> usize {
        self.rope.char_to_utf16_cu(pos)
    }

    pub fn utf16_to_char(&self, unit: usize) -> usize {
        self.rope
            .utf16_cu_to_char(unit.min(self.rope.len_utf16_cu()))
    }

    /// Text of a char range.
    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        self.rope.slice(start..end).to_string()
    }

    pub fn char_at(&self, pos: usize) -> Option<char> {
        (pos < self.len_chars()).then(|| self.rope.char(pos))
    }

    /// Row and byte column of a char offset.
    fn position(&self, pos: usize) -> (usize, usize) {
        let line = self.rope.char_to_line(pos);
        (
            line,
            self.rope.char_to_byte(pos) - self.rope.line_to_byte(line),
        )
    }

    /// UTF-16 column of a char offset in row `line` (the row of `pos`).
    fn utf16_column(&self, line: usize, pos: usize) -> usize {
        self.rope.char_to_utf16_cu(pos) - self.rope.char_to_utf16_cu(self.rope.line_to_char(line))
    }

    /// Applies edits (char ranges in the current text, any order, not
    /// overlapping) as one undoable change and moves the carets.
    ///
    /// Returns the replacements as applied, for syntax trees and caches.
    /// Read-only buffers and empty edit lists change nothing.
    pub fn transact(
        &mut self,
        selections: &mut SelectionSet,
        edits: Vec<Edit>,
        kind: EditKind,
        carets: CaretPlacement,
    ) -> Vec<AppliedEdit> {
        let mut edits: Vec<Edit> = edits
            .into_iter()
            .filter(|e| !(e.range.is_empty() && e.text.is_empty()))
            .collect();
        if self.read_only || edits.is_empty() {
            return Vec::new();
        }
        edits.sort_by_key(|e| (e.range.start, e.range.end));
        let len = self.len_chars();
        for e in &mut edits {
            e.range.end = e.range.end.min(len);
            e.range.start = e.range.start.min(e.range.end);
        }
        debug_assert!(
            edits.windows(2).all(|w| w[0].range.end <= w[1].range.start),
            "overlapping edits"
        );
        let before = selections.clone();
        let step: Vec<Replacement> = edits
            .iter()
            .map(|e| Replacement {
                start: e.range.start,
                old: self.slice(e.range.clone()),
                new: normalize_line_endings(&e.text).into_owned(),
            })
            .collect();
        let applied = self.apply_forward(&step);
        let after = match carets {
            CaretPlacement::AfterEachEdit => {
                let mut delta: isize = 0;
                let carets = step
                    .iter()
                    .map(|r| {
                        let start = (r.start as isize + delta) as usize;
                        let new_len = r.new.chars().count();
                        delta += new_len as isize - r.old.chars().count() as isize;
                        Selection::caret(start + new_len)
                    })
                    .collect();
                SelectionSet::from_vec(carets)
            }
            CaretPlacement::Explicit(set) => set,
        };
        *selections = after;
        selections.clamp(self.len_chars());
        self.history
            .record(step, kind, &before, selections, Instant::now());
        applied
    }

    /// Applies a step (replacements in pre-step coordinates).
    fn apply_forward(&mut self, step: &[Replacement]) -> Vec<AppliedEdit> {
        step.iter()
            .rev()
            .map(|r| self.replace(r.start, r.old.chars().count(), &r.new))
            .collect()
    }

    /// Reverts a step.
    fn apply_backward(&mut self, step: &[Replacement]) -> Vec<AppliedEdit> {
        let mut delta: isize = 0;
        let mut post: Vec<(usize, &Replacement)> = Vec::with_capacity(step.len());
        for r in step {
            post.push(((r.start as isize + delta) as usize, r));
            delta += r.new.chars().count() as isize - r.old.chars().count() as isize;
        }
        post.iter()
            .rev()
            .map(|(start, r)| self.replace(*start, r.new.chars().count(), &r.old))
            .collect()
    }

    fn replace(&mut self, start: usize, old_len: usize, new: &str) -> AppliedEdit {
        let old_end = start + old_len;
        let start_byte = self.rope.char_to_byte(start);
        let old_end_byte = self.rope.char_to_byte(old_end);
        let start_position = self.position(start);
        let old_end_position = self.position(old_end);
        let start_utf16_col = self.utf16_column(start_position.0, start);
        let old_end_utf16_col = self.utf16_column(old_end_position.0, old_end);
        if old_len > 0 {
            self.rope.remove(start..old_end);
        }
        if !new.is_empty() {
            self.rope.insert(start, new);
        }
        self.version += 1;
        let new_end = start + new.chars().count();
        AppliedEdit {
            start,
            old_end,
            new_end,
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + new.len(),
            start_position,
            old_end_position,
            new_end_position: self.position(new_end),
            start_utf16_col,
            old_end_utf16_col,
            text: Arc::from(new),
        }
    }

    /// Replaces a char range with text outside the undo history, also in a
    /// read-only buffer: a console's output. The text keeps its modified
    /// state. Edits are applied in the order given, each in the coordinates
    /// of the text left by the previous one.
    pub fn replace_untracked(&mut self, range: Range<usize>, text: &str) -> AppliedEdit {
        let len = self.len_chars();
        let end = range.end.min(len);
        let start = range.start.min(end);
        let text = normalize_line_endings(text);
        self.replace(start, end - start, &text)
    }

    /// Reverts the last undo group; returns the applied replacements and the
    /// selections from before it.
    pub fn undo(&mut self) -> Option<(Vec<AppliedEdit>, SelectionSet)> {
        if self.read_only {
            return None;
        }
        let t = self.history.pop_undo()?;
        let mut applied = Vec::new();
        for step in t.steps.iter().rev() {
            applied.extend(self.apply_backward(step));
        }
        let selections = t.before.clone();
        self.history.push_redo(t);
        Some((applied, selections))
    }

    /// Applies the last undone group again.
    pub fn redo(&mut self) -> Option<(Vec<AppliedEdit>, SelectionSet)> {
        if self.read_only {
            return None;
        }
        let t = self.history.pop_redo()?;
        let mut applied = Vec::new();
        for step in &t.steps {
            applied.extend(self.apply_forward(step));
        }
        let selections = t.after.clone();
        self.history.push_undo(t);
        Some((applied, selections))
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Ends the current undo group (the caret moved, focus changed, ...).
    pub fn break_undo_group(&mut self) {
        self.history.seal();
    }

    /// The text differs from what was last saved (or loaded).
    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }

    /// The current text is what is on disk now.
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// Starts writing the current text: ends the undo group and returns the
    /// point to pass to [`Buffer::mark_saved_at`] once the write succeeded.
    /// Edits made while the write runs keep the buffer modified.
    pub fn begin_save(&mut self) -> SavePoint {
        SavePoint(self.history.save_point())
    }

    pub fn mark_saved_at(&mut self, point: SavePoint) {
        self.history.mark_saved_at(point.0);
    }

    /// Replaces the whole text (reload from disk) and forgets the history.
    pub fn reset(&mut self, decoded: Decoded) {
        let version = self.version + 1;
        *self = Self::from_decoded(decoded);
        self.version = version;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::decode;

    fn set(pos: usize) -> SelectionSet {
        SelectionSet::caret(pos)
    }

    #[test]
    fn lines_points_and_offsets() {
        let b = Buffer::new("ab\r\ncd\n\nü😀x");
        assert_eq!(b.text(), "ab\ncd\n\nü😀x");
        assert_eq!(b.line_ending(), LineEnding::Crlf);
        assert_eq!(b.len_lines(), 4);
        assert_eq!(b.line_text(1), "cd");
        assert_eq!(b.line_len(2), 0);
        assert_eq!(b.line_end(0), 2);
        assert_eq!(b.offset_to_point(4), Point::new(1, 1));
        assert_eq!(b.point_to_offset(Point::new(3, 99)), b.len_chars());
        assert_eq!(b.point_to_offset(Point::new(99, 0)), b.line_start(3));
        let emoji = b.line_start(3) + 1;
        assert_eq!(b.char_to_utf16(emoji + 1) - b.char_to_utf16(emoji), 2);
        assert_eq!(b.utf16_to_char(b.char_to_utf16(emoji + 1)), emoji + 1);
        assert_eq!(Buffer::new("").len_lines(), 1);
        assert_eq!(Buffer::new("a\n").len_lines(), 2);
    }

    /// Only `\n` breaks lines, as in the reference and in LSP: form feed,
    /// vertical tab, NEL and the Unicode line and paragraph separators stay
    /// inside a line.
    #[test]
    fn only_line_feeds_break_lines() {
        let b = Buffer::new("a\u{0C}b\u{0B}c\u{85}d\u{2028}e\u{2029}f\ng");
        assert_eq!(b.len_lines(), 2);
        assert_eq!(b.line_text(0), "a\u{0C}b\u{0B}c\u{85}d\u{2028}e\u{2029}f");
        assert_eq!(b.line_len(0), 11);
        assert_eq!(b.offset_to_point(10), Point::new(0, 10));
        assert_eq!(b.offset_to_point(12), Point::new(1, 0));
        assert_eq!(b.point_to_offset(Point::new(0, 99)), 11);
        let mut b = Buffer::new("x\n");
        let applied = b.transact(
            &mut set(1),
            vec![Edit::insert(1, "\u{0C}\u{2028}")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        // `x`, a form feed (1 byte) and U+2028 (3 bytes), on line 0.
        assert_eq!(applied[0].new_end_position, (0, 5));
        assert_eq!(b.len_lines(), 2);
    }

    #[test]
    fn untracked_replacements_skip_history_and_read_only() {
        let mut b = Buffer::new("ab\ncd");
        b.set_read_only(true);
        let v = b.version();
        let applied = b.replace_untracked(5..5, "\r\nef");
        assert_eq!(b.text(), "ab\ncd\nef");
        assert_eq!((applied.start, applied.old_end, applied.new_end), (5, 5, 8));
        b.replace_untracked(0..3, "");
        assert_eq!(b.text(), "cd\nef");
        assert!(b.version() > v);
        assert!(!b.can_undo());
        assert!(!b.is_modified());
        // Ranges are clamped to the text.
        b.replace_untracked(4..99, "X");
        assert_eq!(b.text(), "cd\neX");
    }

    #[test]
    fn typing_groups_into_one_undo_step() {
        let mut b = Buffer::new("x");
        let mut sel = set(1);
        for c in ["h", "i", "!"] {
            let at = sel.primary().head;
            b.transact(
                &mut sel,
                vec![Edit::insert(at, c)],
                EditKind::Typing,
                CaretPlacement::AfterEachEdit,
            );
        }
        assert_eq!(b.text(), "xhi!");
        assert_eq!(sel.primary().head, 4);
        assert!(b.is_modified());
        let (_, before) = b.undo().unwrap();
        assert_eq!(b.text(), "x");
        assert_eq!(before.primary().head, 1);
        assert!(!b.is_modified());
        assert!(b.undo().is_none());
        let (_, after) = b.redo().unwrap();
        assert_eq!(b.text(), "xhi!");
        assert_eq!(after.primary().head, 4);
    }

    #[test]
    fn a_caret_move_or_another_kind_starts_a_new_group() {
        let mut b = Buffer::new("");
        let mut sel = set(0);
        b.transact(
            &mut sel,
            vec![Edit::insert(0, "abc")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        // The caret moved away: the next typing is a new group.
        sel = set(1);
        b.transact(
            &mut sel,
            vec![Edit::insert(1, "X")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        b.transact(
            &mut sel,
            vec![Edit::delete(1..2)],
            EditKind::DeleteBackward,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "abc");
        b.undo();
        assert_eq!(b.text(), "aXbc");
        b.undo();
        assert_eq!(b.text(), "abc");
        b.undo();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn multi_edit_steps_undo_and_report_positions() {
        let mut b = Buffer::new("one\ntwo\nthree");
        let mut sel = SelectionSet::from_vec(vec![Selection::caret(0), Selection::caret(4)]);
        let applied = b.transact(
            &mut sel,
            vec![Edit::insert(0, "# "), Edit::insert(4, "# ")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "# one\n# two\nthree");
        assert_eq!(sel.iter().map(|s| s.head).collect::<Vec<_>>(), [2, 8]);
        // Applied from the end: the second line first.
        assert_eq!(applied[0].start_position, (1, 0));
        assert_eq!(applied[0].new_end_byte, applied[0].start_byte + 2);
        assert_eq!(applied[1].start, 0);
        b.undo();
        assert_eq!(b.text(), "one\ntwo\nthree");
        b.redo();
        assert_eq!(b.text(), "# one\n# two\nthree");
    }

    #[test]
    fn save_state_follows_undo_and_redo() {
        let mut b = Buffer::new("a");
        let mut sel = set(1);
        b.transact(
            &mut sel,
            vec![Edit::insert(1, "b")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        b.mark_saved();
        assert!(!b.is_modified());
        // Typing after a save is a new group.
        b.transact(
            &mut sel,
            vec![Edit::insert(2, "c")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        assert!(b.is_modified());
        b.undo();
        assert_eq!(b.text(), "ab");
        assert!(!b.is_modified());
        b.undo();
        assert!(b.is_modified());
        // A new edit drops the redo history: the saved state is gone.
        b.transact(
            &mut sel,
            vec![Edit::insert(0, "z")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert!(b.is_modified());
        assert!(!b.can_redo());
    }

    #[test]
    fn edits_during_a_write_keep_the_buffer_modified() {
        let mut b = Buffer::new("a");
        let mut sel = set(1);
        b.transact(
            &mut sel,
            vec![Edit::insert(1, "b")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        let point = b.begin_save();
        // Typing while the write runs: a new group, not merged into the saved one.
        b.transact(
            &mut sel,
            vec![Edit::insert(2, "c")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        b.mark_saved_at(point);
        assert!(b.is_modified());
        b.undo();
        assert_eq!(b.text(), "ab");
        assert!(!b.is_modified());
    }

    #[test]
    fn inserted_crlf_is_normalised_and_saved_with_the_file_separator() {
        let mut b = Buffer::from_decoded(decode(b"a\r\nb").unwrap());
        let mut sel = set(1);
        b.transact(
            &mut sel,
            vec![Edit::insert(1, "x\r\ny")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "ax\ny\nb");
        assert_eq!(b.encoded(), b"ax\r\ny\r\nb");
        assert_eq!(sel.primary().head, 4);
    }

    #[test]
    fn read_only_buffers_do_not_change() {
        let mut b = Buffer::from_decoded(decode(b"bad \xFF").unwrap());
        assert!(b.is_read_only());
        let mut sel = set(0);
        assert!(
            b.transact(
                &mut sel,
                vec![Edit::insert(0, "x")],
                EditKind::Typing,
                CaretPlacement::AfterEachEdit
            )
            .is_empty()
        );
        assert_eq!(b.version(), 0);
    }

    /// Applies reported edits to a copy of `before` the way a language
    /// server does: by row and UTF-16 column, and separately by row and
    /// byte column, checking both against the char offsets.
    fn replay(before: &str, applied: &[AppliedEdit]) -> String {
        let mut by_utf16 = Rope::from_str(before);
        let mut by_bytes = Rope::from_str(before);
        for e in applied {
            let utf16 = |rope: &Rope, row: usize, col: usize| {
                rope.utf16_cu_to_char(rope.char_to_utf16_cu(rope.line_to_char(row)) + col)
            };
            let start = utf16(&by_utf16, e.start_position.0, e.start_utf16_col);
            let end = utf16(&by_utf16, e.old_end_position.0, e.old_end_utf16_col);
            assert_eq!((start, end), (e.start, e.old_end), "utf-16 range of {e:?}");
            by_utf16.remove(start..end);
            by_utf16.insert(start, &e.text);

            let byte = |rope: &Rope, (row, col): (usize, usize)| {
                rope.byte_to_char(rope.line_to_byte(row) + col)
            };
            let start = byte(&by_bytes, e.start_position);
            let end = byte(&by_bytes, e.old_end_position);
            assert_eq!((start, end), (e.start, e.old_end), "byte range of {e:?}");
            by_bytes.remove(start..end);
            by_bytes.insert(start, &e.text);

            assert_eq!(e.new_end, e.start + e.text.chars().count());
            assert_eq!(e.new_end_byte, e.start_byte + e.text.len());
        }
        assert_eq!(by_utf16, by_bytes);
        by_utf16.to_string()
    }

    #[test]
    fn edits_carry_their_text_and_utf16_columns() {
        // An emoji is two UTF-16 units and four bytes, a CJK character one
        // unit and three bytes, a tab one of each.
        let before = "a😀b\n中文\tx\nlast";
        let mut b = Buffer::new(before);
        let after_emoji = 2;
        let after_tab = b.line_start(1) + 3;
        let mut sel = SelectionSet::from_vec(vec![
            Selection::caret(after_emoji),
            Selection::caret(after_tab),
        ]);
        let applied = b.transact(
            &mut sel,
            vec![
                Edit::insert(after_emoji, "é"),
                Edit::insert(after_tab, "🎉"),
            ],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "a😀éb\n中文\t🎉x\nlast");
        // Applied from the end: the second row first.
        assert_eq!(applied[0].start_position, (1, 7));
        assert_eq!(applied[0].start_utf16_col, 3);
        assert_eq!(&*applied[0].text, "🎉");
        assert_eq!(applied[1].start_position, (0, 5));
        assert_eq!(applied[1].start_utf16_col, 3);
        assert_eq!(applied[1].old_end_utf16_col, 3);
        assert_eq!(&*applied[1].text, "é");
        assert_eq!(replay(before, &applied), b.text());
    }

    #[test]
    fn multi_line_replacements_report_the_old_end_in_the_text_before() {
        let before = "x = 1\n# 😀😀 ok\n\tend";
        let mut b = Buffer::new(before);
        // From after "x = " to the middle of row 1 (after both emoji).
        let start = 4;
        let end = b.line_start(1) + 4;
        let mut sel = SelectionSet::single(Selection::new(start, end));
        let applied = b.transact(
            &mut sel,
            vec![Edit::new(start..end, "(\r\n    2,\n)")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "x = (\n    2,\n) ok\n\tend");
        let e = &applied[0];
        assert_eq!(e.old_end_position, (1, 10));
        assert_eq!(e.old_end_utf16_col, 6);
        // Line breaks of the inserted text are normalised.
        assert_eq!(&*e.text, "(\n    2,\n)");
        assert_eq!(e.new_end_position, (2, 1));
        assert_eq!(replay(before, &applied), b.text());

        // A deletion has no text.
        let before = b.text();
        let applied = b.transact(
            &mut sel,
            vec![Edit::delete(0..6)],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(&*applied[0].text, "");
        assert_eq!(
            (applied[0].old_end_position, applied[0].old_end_utf16_col),
            ((1, 0), 0)
        );
        assert_eq!(replay(&before, &applied), b.text());
    }

    #[test]
    fn undo_and_redo_report_replayable_edits() {
        let mut b = Buffer::new("中\n😀");
        let mut texts = vec![b.text()];
        for (at, text) in [(1, "文"), (2, "\n\t"), (0, "🎉 ")] {
            let mut sel = set(at);
            b.transact(
                &mut sel,
                vec![Edit::insert(at, text)],
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
            );
            texts.push(b.text());
        }
        // Two caret edits in one step, then undo and redo across all steps.
        let before = b.text();
        let mut sel = SelectionSet::from_vec(vec![Selection::caret(0), Selection::caret(4)]);
        let applied = b.transact(
            &mut sel,
            vec![Edit::insert(0, "a"), Edit::new(4..6, "b")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(replay(&before, &applied), b.text());
        let mut current = b.text();
        while let Some((applied, _)) = b.undo() {
            assert_eq!(replay(&current, &applied), b.text());
            current = b.text();
        }
        assert_eq!(current, texts[0]);
        while let Some((applied, _)) = b.redo() {
            assert_eq!(replay(&current, &applied), b.text());
            current = b.text();
        }
        assert_eq!(current, b.text());

        // Untracked replacements too (a console's output).
        let before = b.text();
        let applied = b.replace_untracked(1..3, "x\r\ny");
        assert_eq!(&*applied.text, "x\ny");
        assert_eq!(replay(&before, &[applied]), b.text());
    }

    #[test]
    fn versions_increase_with_every_change() {
        let mut b = Buffer::new("");
        let mut sel = set(0);
        b.transact(
            &mut sel,
            vec![Edit::insert(0, "a")],
            EditKind::Typing,
            CaretPlacement::AfterEachEdit,
        );
        let v = b.version();
        b.undo();
        assert!(b.version() > v);
    }
}
