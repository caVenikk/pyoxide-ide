//! Positions and char offsets of a text, and offsets moved through edits:
//! what is needed to find a position an answer gave in an older text in
//! the current one.

use text::{AppliedEdit, Point, Rope};

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
