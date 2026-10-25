use std::cell::Cell;

use text::Buffer;
use text::movement::char_width;

/// Columns of space the reference adds after the longest line (the
/// "additional columns" of an editor, 3 by default): the text scrolls
/// sideways until the end of the longest line plus this margin is at the
/// right edge of the view.
pub(crate) const ADDITIONAL_COLUMNS: usize = 3;

/// The display width of the longest line of a buffer, kept for the text's
/// version so a frame or a wheel event does not scan the text again. The
/// scan itself is one pass over the rope's chunks and only happens when the
/// text is scrolled sideways (a view at the left edge needs no limit).
#[derive(Default)]
pub(crate) struct LongestLine {
    entry: Cell<Option<Entry>>,
}

#[derive(Clone, Copy)]
struct Entry {
    /// The text's version and length the width is for (a buffer replaced by
    /// its owner starts its versions again).
    text: (u64, usize),
    tab_size: usize,
    columns: usize,
}

impl LongestLine {
    /// Display columns of the longest line of `buffer` (tabs expanded to
    /// `tab_size`, wide characters count two).
    pub fn columns(&self, buffer: &Buffer, tab_size: usize) -> usize {
        let text = (buffer.version(), buffer.len_chars());
        if let Some(e) = self.entry.get()
            && e.text == text
            && e.tab_size == tab_size
        {
            return e.columns;
        }
        let columns = scan(buffer, tab_size);
        self.entry.set(Some(Entry {
            text,
            tab_size,
            columns,
        }));
        columns
    }
}

fn scan(buffer: &Buffer, tab_size: usize) -> usize {
    let tab_size = tab_size.max(1);
    let mut col = 0usize;
    let mut longest = 0usize;
    for chunk in buffer.rope().chunks() {
        for (i, part) in chunk.split('\n').enumerate() {
            if i > 0 {
                longest = longest.max(col);
                col = 0;
            }
            if part.bytes().all(|b| (0x20..0x7f).contains(&b)) {
                // Printable ASCII: a column per byte.
                col += part.len();
            } else {
                for c in part.chars() {
                    col += char_width(c, col, tab_size);
                }
            }
        }
    }
    longest.max(col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(text: &str) -> usize {
        LongestLine::default().columns(&Buffer::new(text), 4)
    }

    #[test]
    fn longest_line_counts_display_columns() {
        assert_eq!(columns(""), 0);
        assert_eq!(columns("abc\nabcdef\nab"), 6);
        assert_eq!(columns("a\n\tb\n"), 5);
        assert_eq!(columns("ab\tc"), 5);
        assert_eq!(columns("日本\nabc"), 4);
        assert_eq!(columns("abc\r\nab"), 3);
    }

    #[test]
    fn width_is_cached_per_version() {
        let mut buffer = Buffer::new("abc");
        let longest = LongestLine::default();
        assert_eq!(longest.columns(&buffer, 4), 3);
        let mut selections = text::SelectionSet::single(text::Selection::new(3, 3));
        buffer.transact(
            &mut selections,
            vec![text::Edit::new(3..3, "defgh")],
            text::EditKind::Typing,
            text::CaretPlacement::AfterEachEdit,
        );
        assert_eq!(longest.columns(&buffer, 4), 8);
    }
}
