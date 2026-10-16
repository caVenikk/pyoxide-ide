//! Caret movement over a buffer: grapheme clusters, words, line starts and
//! display columns. Work is limited to a small window around the caret, so
//! moving stays cheap on very long lines.

use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};
use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;

/// Chars inspected on each side when looking for a grapheme boundary.
const GRAPHEME_WINDOW: usize = 64;

/// Where a word move to the right stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordStop {
    /// At the end of the next word (macOS convention).
    End,
    /// At the start of the next word (Windows and Linux convention).
    Start,
}

impl WordStop {
    /// The convention of the platform the IDE runs on.
    pub const fn platform() -> Self {
        if cfg!(target_os = "macos") {
            WordStop::End
        } else {
            WordStop::Start
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Newline,
    Punct,
}

fn class(c: char) -> Class {
    if c == '\n' {
        Class::Newline
    } else if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// A window of the text around `pos` and the offset of its first char.
fn window(buffer: &Buffer, pos: usize) -> (String, usize) {
    let start = pos.saturating_sub(GRAPHEME_WINDOW);
    let end = (pos + GRAPHEME_WINDOW).min(buffer.len_chars());
    (buffer.slice(start..end), start)
}

/// Char offset of byte `byte` in `text`.
fn chars_before(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// Byte offset of char `chars` in `text`.
fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(b, _)| b)
}

/// The start of the grapheme cluster before `pos`.
pub fn prev_grapheme(buffer: &Buffer, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let (text, start) = window(buffer, pos);
    let at = byte_of(&text, pos - start);
    let mut cursor = GraphemeCursor::new(at, text.len(), true);
    match cursor.prev_boundary(&text, 0) {
        Ok(Some(b)) => start + chars_before(&text, b),
        _ => pos - 1,
    }
}

/// The end of the grapheme cluster after `pos`.
pub fn next_grapheme(buffer: &Buffer, pos: usize) -> usize {
    let len = buffer.len_chars();
    if pos >= len {
        return len;
    }
    let (text, start) = window(buffer, pos);
    let at = byte_of(&text, pos - start);
    let mut cursor = GraphemeCursor::new(at, text.len(), true);
    match cursor.next_boundary(&text, 0) {
        Ok(Some(b)) => start + chars_before(&text, b),
        _ => pos + 1,
    }
}

/// The start of the word before `pos`; at the start of a line, the end of
/// the previous line.
pub fn word_left(buffer: &Buffer, pos: usize) -> usize {
    let at = |p: usize| buffer.char_at(p).map_or(Class::Newline, class);
    if pos == 0 {
        return 0;
    }
    let mut p = pos;
    if at(p - 1) == Class::Newline {
        return p - 1;
    }
    while p > 0 && at(p - 1) == Class::Space {
        p -= 1;
    }
    if p == 0 || at(p - 1) == Class::Newline {
        return p;
    }
    let c = at(p - 1);
    while p > 0 && at(p - 1) == c {
        p -= 1;
    }
    p
}

/// The next word stop after `pos`; at the end of a line, the start of the
/// next line.
pub fn word_right(buffer: &Buffer, pos: usize, stop: WordStop) -> usize {
    let len = buffer.len_chars();
    let at = |p: usize| buffer.char_at(p).map_or(Class::Newline, class);
    if pos >= len {
        return len;
    }
    let mut p = pos;
    if at(p) == Class::Newline {
        return p + 1;
    }
    match stop {
        WordStop::End => {
            while p < len && at(p) == Class::Space {
                p += 1;
            }
            if p < len && at(p) != Class::Newline {
                let c = at(p);
                while p < len && at(p) == c {
                    p += 1;
                }
            }
        }
        WordStop::Start => {
            let c = at(p);
            if c != Class::Space {
                while p < len && at(p) == c {
                    p += 1;
                }
            }
            while p < len && at(p) == Class::Space {
                p += 1;
            }
        }
    }
    p
}

/// The word (or run of punctuation or spaces) at `pos`, for double clicks.
pub fn word_range(buffer: &Buffer, pos: usize) -> Range<usize> {
    let len = buffer.len_chars();
    let at = |p: usize| buffer.char_at(p).map(class);
    // Prefer the word before the caret when it sits at a word's end.
    let probe = match (at(pos), pos.checked_sub(1).and_then(at)) {
        (Some(Class::Word), _) => pos,
        (_, Some(Class::Word)) => pos - 1,
        (Some(Class::Newline) | None, _) => return pos..pos,
        _ => pos,
    };
    let c = at(probe);
    let mut start = probe;
    while start > 0 && at(start - 1) == c {
        start -= 1;
    }
    let mut end = probe;
    while end < len && at(end) == c {
        end += 1;
    }
    start..end
}

/// The first non-blank column of the line of `pos`, or the line start when
/// the caret is already there ("smart home").
pub fn smart_line_start(buffer: &Buffer, pos: usize) -> usize {
    let line = buffer.char_to_line(pos);
    let start = buffer.line_start(line);
    let indent = buffer
        .line(line)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count();
    let first = start + indent;
    if pos == first || first == buffer.line_end(line) {
        start
    } else {
        first
    }
}

/// Display width of a char at display column `col`: a tab reaches the
/// next tab stop, wide characters count two, control characters none.
pub fn char_width(c: char, col: usize, tab_size: usize) -> usize {
    if c == '\t' {
        tab_size - col % tab_size
    } else {
        c.width().unwrap_or(0).max(usize::from(!c.is_control()))
    }
}

/// Display column of char column `column` of `line` (tabs expanded, wide
/// characters count two).
pub fn display_column(buffer: &Buffer, line: usize, column: usize, tab_size: usize) -> usize {
    let mut col = 0;
    for c in buffer.line(line).chars().take(column) {
        col += char_width(c, col, tab_size);
    }
    col
}

/// The char column of `line` closest to display column `target`.
pub fn column_for_display(buffer: &Buffer, line: usize, target: usize, tab_size: usize) -> usize {
    let mut col = 0;
    for (i, c) in buffer.line(line).chars().enumerate() {
        let w = char_width(c, col, tab_size);
        if target < col + w {
            // Inside this char: the nearer of its two edges.
            return if (target - col) * 2 < w { i } else { i + 1 };
        }
        col += w;
    }
    buffer.line_len(line)
}

/// Splits `text` into grapheme clusters (helper for tests and callers that
/// work on short strings).
pub fn graphemes(text: &str) -> Vec<&str> {
    text.graphemes(true).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_over_whole_graphemes() {
        let b = Buffer::new("e\u{301}x👍🏽y");
        assert_eq!(next_grapheme(&b, 0), 2);
        assert_eq!(prev_grapheme(&b, 2), 0);
        assert_eq!(next_grapheme(&b, 3), 5);
        assert_eq!(prev_grapheme(&b, 5), 3);
        assert_eq!(next_grapheme(&b, 6), 6);
        assert_eq!(prev_grapheme(&b, 0), 0);
        assert_eq!(graphemes("e\u{301}x").len(), 2);
    }

    #[test]
    fn word_moves() {
        let b = Buffer::new("foo.bar  baz\n  qux");
        // To the right, macOS: ends of words.
        assert_eq!(word_right(&b, 0, WordStop::End), 3);
        assert_eq!(word_right(&b, 3, WordStop::End), 4);
        assert_eq!(word_right(&b, 7, WordStop::End), 12);
        assert_eq!(word_right(&b, 12, WordStop::End), 13);
        // To the right, Windows and Linux: starts of words.
        assert_eq!(word_right(&b, 0, WordStop::Start), 3);
        assert_eq!(word_right(&b, 4, WordStop::Start), 9);
        assert_eq!(word_right(&b, 9, WordStop::Start), 12);
        // To the left: starts of words, line breaks are stops.
        assert_eq!(word_left(&b, 12), 9);
        assert_eq!(word_left(&b, 9), 4);
        assert_eq!(word_left(&b, 4), 3);
        assert_eq!(word_left(&b, 13), 12);
        assert_eq!(word_left(&b, 15), 13);
        assert_eq!(word_left(&b, 0), 0);
    }

    #[test]
    fn word_under_the_caret() {
        let b = Buffer::new("let x_1 = foo(bar)");
        assert_eq!(word_range(&b, 5), 4..7);
        assert_eq!(word_range(&b, 7), 4..7);
        assert_eq!(word_range(&b, 13), 10..13);
        assert_eq!(word_range(&b, 3), 0..3);
    }

    #[test]
    fn smart_home_toggles() {
        let b = Buffer::new("x\n    pass");
        assert_eq!(smart_line_start(&b, 10), 6);
        assert_eq!(smart_line_start(&b, 6), 2);
        assert_eq!(smart_line_start(&b, 2), 6);
    }

    #[test]
    fn display_columns_expand_tabs_and_wide_chars() {
        let b = Buffer::new("\tab\n日本x");
        assert_eq!(display_column(&b, 0, 1, 4), 4);
        assert_eq!(display_column(&b, 0, 3, 4), 6);
        assert_eq!(display_column(&b, 1, 2, 4), 4);
        assert_eq!(column_for_display(&b, 1, 4, 4), 2);
        assert_eq!(column_for_display(&b, 1, 3, 4), 2);
        assert_eq!(column_for_display(&b, 0, 5, 4), 2);
        assert_eq!(column_for_display(&b, 0, 99, 4), 3);
        assert_eq!(column_for_display(&b, 0, 0, 4), 0);
    }
}
