use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::ansi::{Lexer, Sgr};
use crate::utf8::Utf8Decoder;
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
    /// The length in UTF-16 units.
    units: usize,
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

    fn push(&mut self, text: &str, kind: ContentKind, style: Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        self.units += utf16_len(text);
        let end = self.text.len();
        match self.runs.last_mut() {
            Some(run) if run.kind == kind && run.style == style => run.range.end = end,
            _ => self.runs.push(StyleRun {
                range: start..end,
                kind,
                style,
            }),
        }
    }

    fn clear(&mut self) {
        self.text.clear();
        self.runs.clear();
        self.links.clear();
        self.units = 0;
    }

    /// Deletes the last character; `false` when the line is empty.
    fn pop_char(&mut self) -> bool {
        let Some(c) = self.text.pop() else {
            return false;
        };
        self.units -= c.len_utf16();
        let end = self.text.len();
        while let Some(run) = self.runs.last_mut() {
            if run.range.start >= end {
                self.runs.pop();
            } else {
                run.range.end = run.range.end.min(end);
                break;
            }
        }
        true
    }

    /// Removes the first `units` UTF-16 units (whole characters; a character
    /// straddling the cut goes too).
    fn cut_front(&mut self, units: usize) {
        let mut removed = 0;
        let mut cut = self.text.len();
        for (i, c) in self.text.char_indices() {
            if removed >= units {
                cut = i;
                break;
            }
            removed += c.len_utf16();
        }
        self.text.drain(..cut);
        self.units -= removed.min(self.units);
        self.runs.retain_mut(|run| {
            if run.range.end <= cut {
                return false;
            }
            run.range.start = run.range.start.saturating_sub(cut);
            run.range.end -= cut;
            true
        });
        self.links.retain_mut(|link| {
            if link.range.end <= cut {
                return false;
            }
            link.range.start = link.range.start.saturating_sub(cut);
            link.range.end -= cut;
            true
        });
    }
}

fn utf16_len(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.chars().map(char::len_utf16).sum()
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

/// The decoding state of one stream.
#[derive(Debug, Default)]
struct Stream {
    utf8: Utf8Decoder,
    lexer: Lexer,
    sgr: Sgr,
    /// A `\r` at the end of the last chunk, held to see whether `\n` follows
    /// (the reference's output reader joins `\r\n` across reads).
    pending_cr: bool,
}

/// The console's text model: decoded lines with style runs and links, in a
/// cycle buffer. See the crate documentation for the decoding rules.
///
/// A console always has at least one line; the last line is the one output
/// continues on (empty after a line break).
pub struct Console {
    lines: VecDeque<Line>,
    /// Text length in UTF-16 units, line breaks included.
    units: usize,
    cycle_buffer: Option<usize>,
    filters: Vec<Arc<dyn LinkFilter>>,
    streams: [Stream; 4],
    scratch: String,
    changes: Option<Changes>,
    /// The first line whose links must be computed again.
    links_from: Option<usize>,
}

impl Console {
    /// An empty console with the default cycle buffer
    /// ([`DEFAULT_CYCLE_BUFFER_SIZE`]) and no link filters.
    pub fn new() -> Self {
        Self::with_cycle_buffer(Some(DEFAULT_CYCLE_BUFFER_SIZE))
    }

    /// An empty console keeping at most `size` UTF-16 units of text (line
    /// breaks included); `None` keeps everything. As in the reference, the
    /// oldest text is cut off exactly, so the first line may lose its start.
    pub fn with_cycle_buffer(size: Option<usize>) -> Self {
        let mut lines = VecDeque::new();
        lines.push_back(Line::default());
        Self {
            lines,
            units: 0,
            cycle_buffer: size,
            filters: Vec::new(),
            streams: Default::default(),
            scratch: String::new(),
            changes: None,
            links_from: None,
        }
    }

    /// Changes the cycle buffer size; trims at once when the text is longer.
    pub fn set_cycle_buffer(&mut self, size: Option<usize>) {
        self.cycle_buffer = size;
        self.trim();
        self.update_links();
    }

    /// Sets the link filters, applied in order to every line from now on.
    pub fn set_link_filters(&mut self, filters: Vec<Arc<dyn LinkFilter>>) {
        self.filters = filters;
    }

    /// Appends raw process output. [`ContentKind::Normal`] and
    /// [`ContentKind::Error`] are decoded as UTF-8 with ANSI sequences, each
    /// with its own state; other kinds as UTF-8 only.
    pub fn push(&mut self, kind: ContentKind, bytes: &[u8]) {
        let mut text = std::mem::take(&mut self.scratch);
        text.clear();
        self.streams[kind as usize].utf8.decode(bytes, &mut text);
        self.push_decoded(kind, &text);
        self.scratch = text;
        self.trim();
        self.update_links();
    }

    /// Appends text without ANSI decoding: system lines and echoed input.
    /// Line breaks, `\r` and `\b` work as in output.
    pub fn push_str(&mut self, kind: ContentKind, text: &str) {
        let style = Style::default();
        if std::mem::take(&mut self.streams[kind as usize].pending_cr) && !text.starts_with('\n') {
            self.append("\r", kind, style);
        }
        self.append(text, kind, style);
        self.trim();
        self.update_links();
    }

    /// The process's output ended: an incomplete UTF-8 character becomes
    /// U+FFFD, a `\r` held back to see whether `\n` follows is applied (it
    /// clears the line), and an incomplete escape sequence is dropped (the
    /// reference's lexer never completes it). Call before printing the exit
    /// line.
    pub fn finish(&mut self) {
        for kind in [
            ContentKind::Normal,
            ContentKind::Error,
            ContentKind::System,
            ContentKind::UserInput,
        ] {
            let mut text = String::new();
            self.streams[kind as usize].utf8.finish(&mut text);
            if !text.is_empty() {
                self.push_decoded(kind, &text);
            }
            let stream = &mut self.streams[kind as usize];
            stream.lexer.finish();
            if std::mem::take(&mut stream.pending_cr) {
                let style = stream.sgr.style();
                self.append("\r", kind, style);
            }
        }
        self.trim();
        self.update_links();
    }

    /// Removes all text. The decoding state of the streams stays (output
    /// continues with the same colours, as in the reference's Clear All).
    pub fn clear(&mut self) {
        self.lines.clear();
        self.lines.push_back(Line::default());
        self.units = 0;
        self.links_from = None;
        self.changes = Some(Changes {
            cleared: true,
            ..Changes::default()
        });
    }

    /// What changed since the last call; `None` when nothing did.
    pub fn take_changes(&mut self) -> Option<Changes> {
        self.changes.take()
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
        self.units
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

    /// Decoded text of `kind`: the held `\r`, then ANSI decoding for the
    /// output streams.
    fn push_decoded(&mut self, kind: ContentKind, text: &str) {
        let index = kind as usize;
        let mut text = text;
        if text.is_empty() {
            return;
        }
        if std::mem::take(&mut self.streams[index].pending_cr) && !text.starts_with('\n') {
            let style = self.streams[index].sgr.style();
            self.append("\r", kind, style);
        }
        if let Some(rest) = text.strip_suffix('\r') {
            text = rest;
            self.streams[index].pending_cr = true;
        }
        match kind {
            ContentKind::Normal | ContentKind::Error => {
                let mut stream = std::mem::take(&mut self.streams[index]);
                stream.lexer.feed(text, &mut stream.sgr, |piece, style| {
                    self.append(piece, kind, style)
                });
                self.streams[index] = stream;
            }
            ContentKind::System | ContentKind::UserInput => {
                self.append(text, kind, Style::default());
            }
        }
    }

    /// Appends decoded text: `\n` (and `\r\n`) breaks the line, a lone `\r`
    /// clears the current line, `\b` deletes its last character.
    fn append(&mut self, text: &str, kind: ContentKind, style: Style) {
        let bytes = text.as_bytes();
        let mut start = 0;
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c != b'\n' && c != b'\r' && c != 8 {
                i += 1;
                continue;
            }
            self.append_plain(&text[start..i], kind, style);
            match c {
                b'\n' => self.new_line(),
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    self.new_line();
                    i += 1;
                }
                b'\r' => self.clear_last_line(),
                _ => self.backspace(),
            }
            i += 1;
            start = i;
        }
        self.append_plain(&text[start..], kind, style);
    }

    fn append_plain(&mut self, text: &str, kind: ContentKind, style: Style) {
        if text.is_empty() {
            return;
        }
        let last = self.lines.len() - 1;
        let before = self.lines[last].units;
        self.lines[last].push(text, kind, style);
        self.units += self.lines[last].units - before;
        self.changed(last);
    }

    fn new_line(&mut self) {
        let last = self.lines.len() - 1;
        self.lines.push_back(Line::default());
        self.units += 1;
        self.changed(last);
    }

    fn clear_last_line(&mut self) {
        let last = self.lines.len() - 1;
        let line = &mut self.lines[last];
        if line.text.is_empty() {
            return;
        }
        self.units -= line.units;
        line.clear();
        self.changed(last);
    }

    fn backspace(&mut self) {
        let last = self.lines.len() - 1;
        let before = self.lines[last].units;
        if self.lines[last].pop_char() {
            self.units -= before - self.lines[last].units;
            self.changed(last);
        }
    }

    fn changed(&mut self, line: usize) {
        let changes = self.changes.get_or_insert(Changes {
            changed_from: line,
            ..Changes::default()
        });
        changes.changed_from = changes.changed_from.min(line);
        self.links_from = Some(self.links_from.map_or(line, |l| l.min(line)));
    }

    /// Cuts the oldest text off until the cycle buffer limit holds.
    fn trim(&mut self) {
        let Some(limit) = self.cycle_buffer else {
            return;
        };
        if self.units <= limit {
            return;
        }
        let mut removed = 0;
        let mut cut = false;
        while self.units > limit {
            let excess = self.units - limit;
            let first = self.lines[0].units;
            if self.lines.len() > 1 && first < excess {
                self.lines.pop_front();
                self.units -= first + 1;
                removed += 1;
            } else {
                self.lines[0].cut_front(excess);
                self.units -= first - self.lines[0].units;
                cut = true;
                break;
            }
        }
        let changes = self.changes.get_or_insert(Changes {
            changed_from: self.lines.len(),
            ..Changes::default()
        });
        changes.trimmed += removed;
        changes.first_line_cut = cut || (changes.first_line_cut && removed == 0);
        changes.changed_from = changes.changed_from.saturating_sub(removed);
        if changes.cleared {
            changes.trimmed = 0;
            changes.first_line_cut = false;
            changes.changed_from = 0;
        }
        self.links_from = self.links_from.map(|l| l.saturating_sub(removed));
    }

    /// Runs the link filters on the lines that changed.
    fn update_links(&mut self) {
        let Some(from) = self.links_from.take() else {
            return;
        };
        let mut found = Vec::new();
        for line in self.lines.range_mut(from..) {
            found.clear();
            for filter in &self.filters {
                filter.links(&line.text, &mut found);
            }
            found.retain(|l| {
                l.range.start < l.range.end
                    && l.range.end <= line.text.len()
                    && line.text.is_char_boundary(l.range.start)
                    && line.text.is_char_boundary(l.range.end)
            });
            found.sort_by_key(|l| (l.range.start, l.range.end));
            found.dedup();
            line.links.clear();
            line.links.append(&mut found);
        }
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Console {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Console")
            .field("lines", &self.lines.len())
            .field("len", &self.units)
            .field("cycle_buffer", &self.cycle_buffer)
            .field("filters", &self.filters.len())
            .finish()
    }
}
