//! The text buffer: a rope with a version counter, the file's line separator
//! and encoding, and the undo history.

use std::borrow::Cow;
use std::ops::Range;
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
/// one to a syntax tree or a highlight map keeps every position valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

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
        }
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
