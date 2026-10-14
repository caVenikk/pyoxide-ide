//! The documents of a window: the editors of its tabs whose file type has
//! a language id, with the version counter the services know them by and
//! the recent texts, so that positions an answer gives in an older text
//! can be found in the current one.

use std::collections::VecDeque;
use std::ops::Range;

use code_intel::DocumentId;
use editor::Editor;
use gpui::{Entity, Subscription};
use text::{AppliedEdit, Point, Rope};

/// How many earlier versions of a document are kept for answers that come
/// late (each shares most of its text with the next).
const KEPT_VERSIONS: usize = 128;

/// An editor tab the services know as a document.
pub struct Document {
    pub id: DocumentId,
    pub language: String,
    pub editor: Entity<Editor>,
    pub versions: Versions,
    /// Diagnostics reports for the current version so far (one comes from
    /// each server that reports on it).
    pub reports: usize,
    _subscriptions: Vec<Subscription>,
}

impl Document {
    pub fn new(
        id: DocumentId,
        language: &str,
        editor: Entity<Editor>,
        text: Rope,
        subscriptions: Vec<Subscription>,
    ) -> Self {
        Self {
            id,
            language: language.to_owned(),
            editor,
            versions: Versions::new(text),
            reports: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The version the services saw last.
    pub fn version(&self) -> u64 {
        self.versions.version()
    }
}

/// An earlier text of a document and the edits that made the next version
/// of it.
struct Revision {
    version: u64,
    text: Rope,
    edits: Vec<AppliedEdit>,
}

/// The versions of a document's text: the current one and some before it.
/// The controller counts on its own: an editor's buffer starts again at 0
/// when it is replaced, the services must only ever see higher numbers.
pub struct Versions {
    version: u64,
    /// The text of `version`.
    text: Rope,
    /// Earlier versions, oldest first.
    history: VecDeque<Revision>,
}

impl Versions {
    /// The first version (1) of a text.
    pub fn new(text: Rope) -> Self {
        Self {
            version: 1,
            text,
            history: VecDeque::new(),
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// The current text.
    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// The text changed by `edits`: the new version. Its text is the old
    /// one with the edits applied, so that it matches the edits even when
    /// the editor changed more before they were reported; `current` (the
    /// editor's text) stands in for edits that do not fit.
    pub fn edited(&mut self, edits: &[AppliedEdit], current: &Rope) -> u64 {
        let mut text = self.text.clone();
        for edit in edits {
            if edit.start > edit.old_end || edit.old_end > text.len_chars() {
                text = current.clone();
                break;
            }
            text.remove(edit.start..edit.old_end);
            text.insert(edit.start, &edit.text);
        }
        let old = std::mem::replace(&mut self.text, text);
        self.history.push_back(Revision {
            version: self.version,
            text: old,
            edits: edits.to_vec(),
        });
        while self.history.len() > KEPT_VERSIONS {
            self.history.pop_front();
        }
        self.version += 1;
        self.version
    }

    /// The whole text was replaced: the new version, without history
    /// (positions from before mean nothing in it).
    pub fn reloaded(&mut self, text: &Rope) -> u64 {
        self.text = text.clone();
        self.history.clear();
        self.version += 1;
        self.version
    }

    /// The char offset in the current text of a position in the text of
    /// `version`, moved through the edits since; `None` for a version that
    /// is not kept (or not reached yet).
    pub fn offset(&self, version: u64, at: Point) -> Option<usize> {
        self.range(version, at..at).map(|r| r.start)
    }

    /// As [`Self::offset`] for a range: a range around an edit grows with
    /// it, a range inside a replaced part shrinks to its edges.
    pub fn range(&self, version: u64, range: Range<Point>) -> Option<Range<usize>> {
        let (text, revisions) = if version == self.version {
            (&self.text, self.history.len())
        } else {
            let first = self.history.iter().position(|r| r.version == version)?;
            (&self.history[first].text, first)
        };
        let mut start = point_to_offset(text, range.start);
        let mut end = point_to_offset(text, range.end).max(start);
        for revision in self.history.iter().skip(revisions) {
            for edit in &revision.edits {
                start = map_offset(start, edit, false);
                end = map_offset(end, edit, true).max(start);
            }
        }
        Some(start..end)
    }

    /// The position of a char offset of the current text.
    pub fn point(&self, offset: usize) -> Point {
        offset_to_point(&self.text, offset)
    }
}

/// The char offset of a position, clamped to the text (a column past the
/// end of its line is the end of the line, a line past the end is the end
/// of the text).
pub fn point_to_offset(text: &Rope, at: Point) -> usize {
    if at.line >= text.len_lines() {
        return text.len_chars();
    }
    let start = text.line_to_char(at.line);
    let line = text.line(at.line);
    let mut len = line.len_chars();
    if len > 0 && line.char(len - 1) == '\n' {
        len -= 1;
    }
    start + at.column.min(len)
}

/// The position of a char offset (clamped to the text).
pub fn offset_to_point(text: &Rope, offset: usize) -> Point {
    let offset = offset.min(text.len_chars());
    let line = text.char_to_line(offset);
    Point::new(line, offset - text.line_to_char(line))
}

/// A char offset moved through an edit: before it unchanged, after it
/// shifted; inside the replaced part it goes to the start of the new text,
/// or to its end for the end of a range (`end`). An insertion at the offset
/// pushes the end of a range, not its start.
pub fn map_offset(offset: usize, edit: &AppliedEdit, end: bool) -> usize {
    let inserted_at = edit.start == edit.old_end && offset == edit.start;
    if offset < edit.start || (inserted_at && !end) {
        offset
    } else if offset >= edit.old_end {
        offset - edit.old_end + edit.new_end
    } else if end {
        edit.new_end
    } else {
        edit.start
    }
}

#[cfg(test)]
mod tests {
    use text::{Buffer, CaretPlacement, Edit, EditKind, SelectionSet};

    use super::*;

    fn edit(buffer: &mut Buffer, versions: &mut Versions, edits: Vec<Edit>) -> u64 {
        let mut selections = SelectionSet::caret(0);
        let applied = buffer.transact(
            &mut selections,
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        versions.edited(&applied, buffer.rope())
    }

    #[test]
    fn points_and_offsets_clamp_to_the_text() {
        let text = Rope::from_str("ab\ncde\n");
        assert_eq!(point_to_offset(&text, Point::new(1, 1)), 4);
        assert_eq!(point_to_offset(&text, Point::new(0, 9)), 2);
        assert_eq!(point_to_offset(&text, Point::new(2, 0)), 7);
        assert_eq!(point_to_offset(&text, Point::new(9, 0)), 7);
        assert_eq!(offset_to_point(&text, 4), Point::new(1, 1));
        assert_eq!(offset_to_point(&text, 99), Point::new(2, 0));
    }

    #[test]
    fn positions_of_older_versions_move_through_the_edits() {
        let mut buffer = Buffer::new("x = 1\ny = 2\n");
        let mut versions = Versions::new(buffer.rope().clone());
        // Version 2: a line inserted at the top; version 3: `1` replaced
        // by `100` (in the second line now).
        assert_eq!(
            edit(
                &mut buffer,
                &mut versions,
                vec![Edit::insert(0, "import os\n")]
            ),
            2
        );
        let at = buffer.point_to_offset(Point::new(1, 4));
        assert_eq!(
            edit(
                &mut buffer,
                &mut versions,
                vec![Edit::new(at..at + 1, "100")]
            ),
            3
        );
        // `y` of version 1 is on the third line now.
        let y = versions
            .range(1, Point::new(1, 0)..Point::new(1, 1))
            .unwrap();
        assert_eq!(buffer.slice(y), "y");
        // `1` of version 1 grew into `100`.
        let one = versions
            .range(1, Point::new(0, 4)..Point::new(0, 5))
            .unwrap();
        assert_eq!(buffer.slice(one), "100");
        // Version 2 needs one edit, the current version none.
        assert_eq!(
            versions.offset(2, Point::new(1, 5)),
            Some(buffer.point_to_offset(Point::new(1, 7)))
        );
        assert_eq!(
            versions.range(3, Point::new(0, 0)..Point::new(0, 6)),
            Some(0..6)
        );
        assert_eq!(versions.offset(7, Point::new(0, 0)), None);
        assert_eq!(versions.point(11), Point::new(1, 1));
        // A reload forgets the history.
        assert_eq!(versions.reloaded(&Rope::from_str("z")), 4);
        assert_eq!(versions.offset(3, Point::new(0, 0)), None);
        assert_eq!(versions.text().to_string(), "z");
    }

    #[test]
    fn each_version_has_the_text_of_its_edits() {
        // Two changes reported after both were made: the first version
        // still gets the text after the first change only.
        let mut buffer = Buffer::new("ab");
        let mut versions = Versions::new(buffer.rope().clone());
        let mut selections = SelectionSet::caret(0);
        let mut apply = |buffer: &mut Buffer, edit| {
            buffer.transact(
                &mut selections,
                vec![edit],
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
            )
        };
        let first = apply(&mut buffer, Edit::insert(0, "x"));
        let second = apply(&mut buffer, Edit::insert(3, "y"));
        versions.edited(&first, buffer.rope());
        assert_eq!(versions.text().to_string(), "xab");
        versions.edited(&second, buffer.rope());
        assert_eq!(versions.text().to_string(), "xaby");
        // Edits that do not fit: the editor's text.
        versions.edited(&first, &Rope::from_str("z"));
        assert_eq!(versions.text().to_string(), "xxaby");
        let stray = apply(&mut buffer, Edit::delete(0..5));
        let mut versions = Versions::new(Rope::from_str("ab"));
        versions.edited(&stray, buffer.rope());
        assert_eq!(versions.text().to_string(), "");
    }

    #[test]
    fn offsets_inside_a_replacement_go_to_its_edges() {
        let mut buffer = Buffer::new("abcdef");
        let mut selections = SelectionSet::caret(0);
        let mut apply = |edits| {
            buffer.transact(
                &mut selections,
                edits,
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
            )
        };
        let applied = apply(vec![Edit::new(1..4, "XY")]);
        let e = &applied[0];
        assert_eq!(map_offset(0, e, false), 0);
        assert_eq!(map_offset(2, e, false), 1);
        assert_eq!(map_offset(2, e, true), 3);
        assert_eq!(map_offset(5, e, false), 4);
        let applied = apply(vec![Edit::insert(1, "--")]);
        let e = &applied[0];
        assert_eq!(map_offset(1, e, false), 1);
        assert_eq!(map_offset(1, e, true), 3);
    }
}
