//! Positions: the model's char-based [`Point`]s and LSP positions in the
//! encoding negotiated with each server.
//!
//! Lines are the same on both sides: the text breaks lines at `\n` only (it
//! normalises `\r\n` and `\r` on load; the `text` crate's rope does not take
//! form feeds or Unicode separators as breaks), as LSP does. Columns are
//! chars in the model and UTF-8 bytes, UTF-16 code units or UTF-32 units
//! (chars) for a server.

use code_intel::AppliedEdit;
use lsp_types::{Position, Range};
use text::{Point, Rope};

/// How a server counts columns, negotiated at `initialize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PositionEncoding {
    Utf8,
    Utf16,
    Utf32,
}

impl PositionEncoding {
    /// The encodings the client offers, preferred first (ADR 0010).
    pub const OFFERED: [&'static str; 2] = ["utf-8", "utf-16"];

    /// The server's choice (`capabilities.positionEncoding`): UTF-16 when
    /// it names none or one this client does not know, as the protocol
    /// says. UTF-32 is accepted too.
    pub fn negotiated(choice: Option<&str>) -> Self {
        match choice {
            Some("utf-8") => PositionEncoding::Utf8,
            Some("utf-32") => PositionEncoding::Utf32,
            _ => PositionEncoding::Utf16,
        }
    }

    #[cfg(test)]
    pub fn as_str(self) -> &'static str {
        match self {
            PositionEncoding::Utf8 => "utf-8",
            PositionEncoding::Utf16 => "utf-16",
            PositionEncoding::Utf32 => "utf-32",
        }
    }

    /// Units of `s` in this encoding.
    pub fn len(self, s: &str) -> usize {
        match self {
            PositionEncoding::Utf8 => s.len(),
            PositionEncoding::Utf16 => s.encode_utf16().count(),
            PositionEncoding::Utf32 => s.chars().count(),
        }
    }

    /// The byte index in `s` of `units` units of this encoding (clamped to
    /// the end, rounded down to a char boundary).
    pub fn byte_index(self, s: &str, units: usize) -> usize {
        let mut counted = 0;
        for (i, c) in s.char_indices() {
            let len = match self {
                PositionEncoding::Utf8 => c.len_utf8(),
                PositionEncoding::Utf16 => c.len_utf16(),
                PositionEncoding::Utf32 => 1,
            };
            if counted + len > units {
                return i;
            }
            counted += len;
        }
        s.len()
    }

    /// The LSP range an applied edit replaced, from the positions it
    /// carries (`None` for UTF-32, which needs the text before the edit).
    pub fn applied_range(self, edit: &AppliedEdit) -> Option<Range> {
        let (start, end) = match self {
            PositionEncoding::Utf8 => (edit.start_position.1, edit.old_end_position.1),
            PositionEncoding::Utf16 => (edit.start_utf16_col, edit.old_end_utf16_col),
            PositionEncoding::Utf32 => return None,
        };
        Some(Range::new(
            Position::new(saturating_u32(edit.start_position.0), saturating_u32(start)),
            Position::new(saturating_u32(edit.old_end_position.0), saturating_u32(end)),
        ))
    }
}

/// A text snapshot for position conversions.
#[derive(Debug, Clone)]
pub(crate) struct Text {
    rope: Rope,
}

impl Text {
    pub fn new(rope: Rope) -> Self {
        Self { rope }
    }

    #[cfg(test)]
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// The char offset of a point (lines and columns past the end are
    /// clamped).
    pub fn point_to_offset(&self, point: Point) -> usize {
        let rope = &self.rope;
        let line = point.line.min(rope.len_lines().saturating_sub(1));
        rope.line_to_char(line) + point.column.min(line_len(rope, line))
    }

    pub fn offset_to_point(&self, offset: usize) -> Point {
        let rope = &self.rope;
        let offset = offset.min(rope.len_chars());
        let line = rope.char_to_line(offset);
        Point::new(line, offset - rope.line_to_char(line))
    }

    /// The LSP position of a point.
    pub fn position(&self, point: Point, encoding: PositionEncoding) -> Position {
        self.offset_position(self.point_to_offset(point), encoding)
    }

    /// The point at an LSP position. Positions past the end of a line are
    /// at its end, lines past the end of the text at the end of the text;
    /// a position inside a character is at its start.
    #[cfg(test)]
    pub fn point(&self, position: Position, encoding: PositionEncoding) -> Point {
        self.offset_to_point(self.position_offset(position, encoding))
    }

    /// The LSP range of a char range.
    pub fn range(&self, range: std::ops::Range<Point>, encoding: PositionEncoding) -> Range {
        Range::new(
            self.position(range.start, encoding),
            self.position(range.end, encoding),
        )
    }

    /// The LSP position of a char offset.
    pub fn offset_position(&self, offset: usize, encoding: PositionEncoding) -> Position {
        let rope = &self.rope;
        let offset = offset.min(rope.len_chars());
        let line = rope.char_to_line(offset);
        let start = rope.line_to_char(line);
        let column = match encoding {
            PositionEncoding::Utf8 => rope.char_to_byte(offset) - rope.char_to_byte(start),
            PositionEncoding::Utf16 => rope.char_to_utf16_cu(offset) - rope.char_to_utf16_cu(start),
            PositionEncoding::Utf32 => offset - start,
        };
        Position::new(saturating_u32(line), saturating_u32(column))
    }

    /// The char offset of an LSP position (clamped as [`Text::point`]).
    pub fn position_offset(&self, position: Position, encoding: PositionEncoding) -> usize {
        let rope = &self.rope;
        let line = position.line as usize;
        if line >= rope.len_lines() {
            return rope.len_chars();
        }
        let start = rope.line_to_char(line);
        let end = start + line_len(rope, line);
        let column = position.character as usize;
        match encoding {
            PositionEncoding::Utf8 => {
                let start_byte = rope.char_to_byte(start);
                let end_byte = rope.char_to_byte(end);
                rope.byte_to_char((start_byte + column).min(end_byte))
            }
            PositionEncoding::Utf16 => {
                let start_unit = rope.char_to_utf16_cu(start);
                let end_unit = rope.char_to_utf16_cu(end);
                let unit = (start_unit + column).min(end_unit);
                let offset = rope.utf16_cu_to_char(unit);
                // Inside a surrogate pair: the start of the character.
                if rope.char_to_utf16_cu(offset) > unit {
                    offset - 1
                } else {
                    offset
                }
            }
            PositionEncoding::Utf32 => start + column.min(end - start),
        }
    }
}

/// Chars of a line without its `\n`.
fn line_len(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let len = slice.len_chars();
    if len > 0 && slice.char(len - 1) == '\n' {
        len - 1
    } else {
        len
    }
}

pub(crate) fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    use PositionEncoding::{Utf8, Utf16, Utf32};

    fn text(s: &str) -> Text {
        Text::new(Rope::from_str(s))
    }

    fn pos(line: u32, character: u32) -> Position {
        Position::new(line, character)
    }

    #[test]
    fn negotiation() {
        assert_eq!(PositionEncoding::negotiated(Some("utf-8")), Utf8);
        assert_eq!(PositionEncoding::negotiated(Some("utf-16")), Utf16);
        assert_eq!(PositionEncoding::negotiated(Some("utf-32")), Utf32);
        assert_eq!(PositionEncoding::negotiated(None), Utf16);
        assert_eq!(PositionEncoding::negotiated(Some("ucs-2")), Utf16);
        assert_eq!(Utf8.as_str(), "utf-8");
        assert_eq!(Utf16.len("a🦀中"), 4);
        assert_eq!(Utf8.len("a🦀中"), 8);
        assert_eq!(Utf32.len("a🦀中"), 3);
        assert_eq!(Utf16.byte_index("a🦀中", 3), 5);
        assert_eq!(Utf16.byte_index("a🦀中", 2), 1);
        assert_eq!(Utf8.byte_index("a🦀中", 99), 8);
        assert_eq!(Utf32.byte_index("a🦀中", 2), 5);
    }

    #[test]
    fn ascii_is_the_same_in_every_encoding() {
        let t = text("import os\nx = 1\n");
        for enc in [Utf8, Utf16, Utf32] {
            assert_eq!(t.position(Point::new(1, 4), enc), pos(1, 4));
            assert_eq!(t.point(pos(1, 4), enc), Point::new(1, 4));
            // The empty last line.
            assert_eq!(t.position(Point::new(2, 0), enc), pos(2, 0));
            assert_eq!(t.point(pos(2, 0), enc), Point::new(2, 0));
        }
    }

    #[test]
    fn emoji_cjk_and_tabs() {
        // 🦀 is 4 UTF-8 bytes and 2 UTF-16 units; 中 is 3 bytes and 1 unit.
        let t = text("\ts = \"🦀中\"  # é\nnext");
        let after_crab = Point::new(0, 7); // \t s _ = _ " 🦀 |
        assert_eq!(t.position(after_crab, Utf8), pos(0, 10));
        assert_eq!(t.position(after_crab, Utf16), pos(0, 8));
        assert_eq!(t.position(after_crab, Utf32), pos(0, 7));
        let end = Point::new(0, 14);
        assert_eq!(t.position(end, Utf8), pos(0, 20));
        assert_eq!(t.position(end, Utf16), pos(0, 15));
        for enc in [Utf8, Utf16, Utf32] {
            for column in 0..=14 {
                let p = Point::new(0, column);
                assert_eq!(t.point(t.position(p, enc), enc), p, "{enc:?} {column}");
            }
            assert_eq!(
                t.point(t.position(Point::new(1, 2), enc), enc),
                Point::new(1, 2)
            );
        }
    }

    #[test]
    fn positions_inside_characters_and_past_the_end_are_clamped() {
        let t = text("a🦀b\nxy");
        // Inside the crab: its start.
        assert_eq!(t.point(pos(0, 2), Utf16), Point::new(0, 1));
        assert_eq!(t.point(pos(0, 3), Utf8), Point::new(0, 1));
        assert_eq!(t.point(pos(0, 4), Utf8), Point::new(0, 1));
        assert_eq!(t.point(pos(0, 5), Utf8), Point::new(0, 2));
        // Past the end of the line: its end, not the next line.
        assert_eq!(t.point(pos(0, 99), Utf16), Point::new(0, 3));
        assert_eq!(t.point(pos(0, 99), Utf8), Point::new(0, 3));
        assert_eq!(t.point(pos(0, 99), Utf32), Point::new(0, 3));
        // Past the last line: the end of the text.
        assert_eq!(t.point(pos(7, 0), Utf16), Point::new(1, 2));
        // Points past the end are clamped too.
        assert_eq!(t.position(Point::new(0, 50), Utf16), pos(0, 4));
        assert_eq!(t.position(Point::new(9, 50), Utf8), pos(1, 2));
        // Empty text.
        let empty = text("");
        assert_eq!(empty.position(Point::new(0, 0), Utf16), pos(0, 0));
        assert_eq!(empty.point(pos(3, 3), Utf16), Point::new(0, 0));
    }

    #[test]
    fn form_feeds_and_unicode_separators_stay_inside_a_line() {
        let t = text("a\u{0C}b\nc\u{2028}d🦀");
        assert_eq!(t.rope().len_lines(), 2);
        assert_eq!(t.position(Point::new(0, 2), Utf16), pos(0, 2));
        assert_eq!(t.point(pos(1, 4), Utf8), Point::new(1, 2));
        assert_eq!(t.position(Point::new(1, 4), Utf16), pos(1, 5));
        for offset in 0..=t.rope().len_chars() {
            for enc in [Utf8, Utf16, Utf32] {
                let p = t.offset_position(offset, enc);
                assert_eq!(t.position_offset(p, enc), offset, "{enc:?} {offset}");
            }
        }
    }
}
