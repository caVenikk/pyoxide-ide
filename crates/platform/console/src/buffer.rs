use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

use crate::{ContentKind, Link, LinkFilter, Style};

/// The reference's default console cycle buffer: 1024 KB, counted in UTF-16
/// units ("Override console cycle buffer size" in Settings | Editor |
/// General | Console).
pub const DEFAULT_CYCLE_BUFFER_SIZE: usize = 1024 * 1024;

/// A run of text with one content kind and style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRun {
    /// Byte range in the line.
    pub range: Range<usize>,
    pub kind: ContentKind,
    pub style: Style,
}

/// One console line, without its line break.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    text: String,
    runs: Vec<StyleRun>,
    links: Vec<Link>,
}

impl Line {
    /// The text, without the line break.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Style runs covering the whole text, in order, without gaps; adjacent
    /// runs differ in kind or style.
    pub fn runs(&self) -> &[StyleRun] {
        &self.runs
    }

    /// Hyperlinks found by the link filters, ordered by start.
    pub fn links(&self) -> &[Link] {
        &self.links
    }
}

/// What changed since the last [`Console::take_changes`]. A view applies it
/// in this order: when `cleared`, reload everything; otherwise drop
/// `trimmed` lines from the front, reload line 0 when `first_line_cut`, then
/// reload lines `changed_from..line_count()` (lines past the view's own
/// count are new).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Changes {
    /// The console was cleared; every line is new.
    pub cleared: bool,
    /// Lines removed from the front by the cycle buffer.
    pub trimmed: usize,
    /// The cycle buffer cut the start of the (new) first line.
    pub first_line_cut: bool,
    /// The first line, in the current numbering, whose text, runs or links
    /// changed or which is new.
    pub changed_from: usize,
}

/// The console's text model: decoded lines with style runs and links, in a
/// cycle buffer. See the crate documentation for the decoding rules.
///
/// A console always has at least one line; the last line is the one output
/// continues on (empty after a line break).
pub struct Console {
    lines: VecDeque<Line>,
    cycle_buffer: Option<usize>,
    filters: Vec<Arc<dyn LinkFilter>>,
}

impl Console {
    /// An empty console with the default cycle buffer
    /// ([`DEFAULT_CYCLE_BUFFER_SIZE`]) and no link filters.
    pub fn new() -> Self {
        Self::with_cycle_buffer(Some(DEFAULT_CYCLE_BUFFER_SIZE))
    }

    /// An empty console keeping at most `size` UTF-16 units of text (line
    /// breaks included); `None` keeps everything.
    pub fn with_cycle_buffer(size: Option<usize>) -> Self {
        let mut lines = VecDeque::new();
        lines.push_back(Line::default());
        Self {
            lines,
            cycle_buffer: size,
            filters: Vec::new(),
        }
    }

    /// Changes the cycle buffer size; trims at once when the text is longer.
    pub fn set_cycle_buffer(&mut self, size: Option<usize>) {
        self.cycle_buffer = size;
        todo!("Console::set_cycle_buffer trimming")
    }

    /// Sets the link filters, applied in order to every line from now on.
    pub fn set_link_filters(&mut self, filters: Vec<Arc<dyn LinkFilter>>) {
        self.filters = filters;
    }

    /// Appends raw process output. [`ContentKind::Normal`] and
    /// [`ContentKind::Error`] are decoded as UTF-8 with ANSI sequences, each
    /// with its own state; other kinds as UTF-8 only.
    pub fn push(&mut self, kind: ContentKind, bytes: &[u8]) {
        let _ = (kind, bytes);
        todo!("Console::push")
    }

    /// Appends text without ANSI decoding: system lines and echoed input.
    /// Line breaks, `\r` and `\b` work as in output.
    pub fn push_str(&mut self, kind: ContentKind, text: &str) {
        let _ = (kind, text);
        todo!("Console::push_str")
    }

    /// The process's output ended: an incomplete UTF-8 character becomes
    /// U+FFFD, a `\r` held back to see whether `\n` follows is applied, and
    /// an incomplete escape sequence is dropped (as the reference's lexer
    /// never completes it). Call before printing the exit line.
    pub fn finish(&mut self) {
        todo!("Console::finish")
    }

    /// Removes all text and resets the decoders.
    pub fn clear(&mut self) {
        todo!("Console::clear")
    }

    /// What changed since the last call; `None` when nothing did.
    pub fn take_changes(&mut self) -> Option<Changes> {
        todo!("Console::take_changes")
    }

    /// The number of lines (at least 1).
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The line at `index`.
    pub fn line(&self, index: usize) -> Option<&Line> {
        self.lines.get(index)
    }

    /// All lines in order.
    pub fn lines(&self) -> impl ExactSizeIterator<Item = &Line> + '_ {
        self.lines.iter()
    }

    /// The text length in UTF-16 units, line breaks included (what the cycle
    /// buffer limits).
    pub fn len(&self) -> usize {
        todo!("Console::len")
    }

    /// No text at all.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].text.is_empty()
    }

    /// The whole text, lines joined with `\n`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&line.text);
        }
        out
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}
