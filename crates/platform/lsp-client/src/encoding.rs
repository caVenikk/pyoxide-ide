//! Positions: the model's char-based [`Point`]s and LSP positions in the
//! encoding negotiated with each server.
//!
//! LSP counts lines by `\n` (the text has no other separators: `\r\n` and
//! `\r` are normalised on load) and columns in UTF-8 bytes, UTF-16 code
//! units or UTF-32 units (chars). The rope also breaks lines at vertical
//! tab, form feed, NEL and the Unicode line and paragraph separators, so
//! in a text with any of those its line numbers differ from the server's;
//! [`Text`] notices them once and converts through char offsets then.

use lsp_types::Position;
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
}

/// Chars the rope takes as line breaks besides `\n`.
fn is_extra_break(c: char) -> bool {
    matches!(
        c,
        '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// The text has a line break the rope knows and LSP does not.
pub(crate) fn has_extra_breaks(s: &str) -> bool {
    // Every such char starts with one of these bytes; check the cheap way
    // first.
    s.bytes()
        .any(|b| matches!(b, 0x0B | 0x0C | b'\r' | 0xC2 | 0xE2))
        && s.chars().any(is_extra_break)
}

/// A text snapshot with what position conversions need.
#[derive(Debug, Clone)]
pub(crate) struct Text {
    rope: Rope,
    /// The rope's lines are the server's lines (no extra breaks).
    plain: bool,
}

impl Text {
    pub fn new(rope: Rope) -> Self {
        let plain = !rope.chunks().any(has_extra_breaks);
        Self { rope, plain }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// The rope's lines are the server's lines.
    pub fn is_plain(&self) -> bool {
        self.plain
    }

    /// The char offset of a point (lines and columns past the end are
    /// clamped).
    pub fn point_to_offset(&self, point: Point) -> usize {
        let rope = &self.rope;
        let line = point.line.min(rope.len_lines().saturating_sub(1));
        let start = rope.line_to_char(line);
        start + point.column.min(rope_line_len(rope, line))
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
    pub fn point(&self, position: Position, encoding: PositionEncoding) -> Point {
        self.offset_to_point(self.position_offset(position, encoding))
    }

    /// The LSP position of a char offset.
    pub fn offset_position(&self, offset: usize, encoding: PositionEncoding) -> Position {
        let rope = &self.rope;
        let offset = offset.min(rope.len_chars());
        let (line, start) = if self.plain {
            let line = rope.char_to_line(offset);
            (line, rope.line_to_char(line))
        } else {
            self.lsp_line_of(offset)
        };
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
        let Some((start, end)) = self.lsp_line_range(position.line as usize) else {
            return rope.len_chars();
        };
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

    /// The server's line of a char offset and the offset where it starts
    /// (for texts with extra breaks).
    fn lsp_line_of(&self, offset: usize) -> (usize, usize) {
        let rope = &self.rope;
        let rope_line = rope.char_to_line(offset);
        let mut line = 0;
        let mut start = 0;
        for l in 0..rope_line {
            let next = rope.line_to_char(l + 1);
            if rope.char(next - 1) == '\n' {
                line += 1;
                start = next;
            }
        }
        (line, start)
    }

    /// Start and end (before the `\n`) of a server line, `None` past the
    /// last line.
    fn lsp_line_range(&self, line: usize) -> Option<(usize, usize)> {
        let rope = &self.rope;
        if self.plain {
            if line >= rope.len_lines() {
                return None;
            }
            let start = rope.line_to_char(line);
            return Some((start, start + rope_line_len(rope, line)));
        }
        let lines = rope.len_lines();
        let mut current = 0;
        let mut start = 0;
        for l in 0..lines {
            let last = l + 1 == lines;
            let next = if last {
                rope.len_chars()
            } else {
                rope.line_to_char(l + 1)
            };
            // A line other than the last ends with its break.
            let ends_line = !last && rope.char(next - 1) == '\n';
            if ends_line || last {
                if current == line {
                    let end = if ends_line { next - 1 } else { next };
                    return Some((start, end));
                }
                current += 1;
                start = next;
            }
        }
        None
    }
}

/// Chars of a rope line without its break.
fn rope_line_len(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let len = slice.len_chars();
    if len > 0 && (slice.char(len - 1) == '\n' || is_extra_break(slice.char(len - 1))) {
        len - 1
    } else {
        len
    }
}

fn saturating_u32(n: usize) -> u32 {
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
    }

    #[test]
    fn ascii_is_the_same_in_every_encoding() {
        let t = text("import os\nx = 1\n");
        assert!(t.is_plain());
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
    fn form_feeds_and_unicode_separators_are_not_server_lines() {
        // The rope sees four lines, the server two.
        let t = text("a\u{0C}b\nc\u{2028}d🦀");
        assert!(!t.is_plain());
        assert_eq!(t.rope().len_lines(), 4);
        // `b` is rope line 1, column 0; server line 0, column 2.
        assert_eq!(t.position(Point::new(1, 0), Utf16), pos(0, 2));
        assert_eq!(t.point(pos(0, 2), Utf16), Point::new(1, 0));
        // `d` is rope line 3; server line 1, column 2.
        assert_eq!(t.position(Point::new(3, 0), Utf8), pos(1, 4));
        assert_eq!(t.point(pos(1, 4), Utf8), Point::new(3, 0));
        assert_eq!(t.point(pos(1, 2), Utf16), Point::new(3, 0));
        // End of the text.
        assert_eq!(t.position(Point::new(3, 2), Utf16), pos(1, 5));
        assert_eq!(t.point(pos(1, 99), Utf32), Point::new(3, 2));
        assert_eq!(t.point(pos(5, 0), Utf32), Point::new(3, 2));
        // Every offset goes there and back.
        for s in ["a\u{0C}b\nc\u{2028}d🦀", "x\u{0B}\n\n\u{85}y\n", "\u{2029}"] {
            let t = text(s);
            for offset in 0..=t.rope().len_chars() {
                for enc in [Utf8, Utf16, Utf32] {
                    let p = t.offset_position(offset, enc);
                    assert_eq!(t.position_offset(p, enc), offset, "{s:?} {enc:?} {offset}");
                }
            }
        }
        // The server's lines of the second text: "x\v", "", "\u{85}y", "".
        let t = text("x\u{0B}\n\n\u{85}y\n");
        assert_eq!(t.offset_position(5, Utf16), pos(2, 1));
        assert_eq!(t.offset_position(5, Utf8), pos(2, 2));
        assert_eq!(t.point(pos(3, 0), Utf16), Point::new(5, 0));
        assert!(has_extra_breaks("x\u{85}y"));
        assert!(!has_extra_breaks("é中🦀\n\t"));
    }
}
