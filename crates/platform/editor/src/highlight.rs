//! The interface between the editor and a source of token styles, and the
//! tree-sitter implementation of it.
//!
//! The editor asks a highlighter for the spans of the lines it paints. Heavy
//! work (parsing) is handed to the editor as a [`Job`], which the editor runs
//! on the background executor; the result comes back through
//! [`Highlighter::job_done`] on the UI thread. Edits are applied to the
//! current syntax tree at once, so highlighting stays in place while a
//! reparse runs, and typing never waits for the parser. The language's
//! lexer colours lines without the tree (see the `syntax` crate), so
//! highlighting is there before the first parse and survives syntax errors.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use syntax::LineStates;
use syntax::tree_sitter::{InputEdit, Tree};
use text::{AppliedEdit, Buffer};
use themes::TextRole;

/// A styled range of one line: chars from the start of the line.
pub type Span = (Range<usize>, TextRole);

/// An explicit style of a range of a line, drawn over the role spans (for
/// example a console's ANSI colours). Unset parts keep the text's style.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct InlineStyle {
    pub color: Option<gpui::Hsla>,
    pub background: Option<gpui::Hsla>,
    pub bold: bool,
    pub italic: bool,
    /// Underline thickness in logical pixels.
    pub underline: Option<f32>,
    pub strikethrough: bool,
}

/// Work for a background thread and its result.
pub type JobResult = Box<dyn Any + Send>;
pub type Job = Box<dyn FnOnce() -> JobResult + Send>;

pub trait Highlighter: 'static {
    /// The editor's file changed (also called when the highlighter is
    /// given to an editor that has one), for a language whose colours
    /// depend on the file (the SQL dialect of a folder).
    fn set_path(&mut self, _path: Option<&std::path::Path>) {}

    /// The whole text was (re)loaded.
    fn reset(&mut self, buffer: &Buffer);

    /// The text changed by these replacements (in the order applied).
    fn edited(&mut self, buffer: &Buffer, edits: &[AppliedEdit]);

    /// Work that is due, if any; the editor runs one job at a time.
    fn next_job(&mut self, _buffer: &Buffer) -> Option<Job> {
        None
    }

    /// The result of the last job.
    fn job_done(&mut self, _result: JobResult, _buffer: &Buffer) {}

    /// Called before painting with the lines that will be painted.
    fn prepare(&mut self, _buffer: &Buffer, _lines: Range<usize>) {}

    /// Appends the spans of `line` to `out`, sorted and not overlapping.
    fn line_spans(&self, line: usize, out: &mut Vec<Span>);

    /// Appends the char ranges of `line` to mark as syntax errors.
    fn line_errors(&self, _line: usize, _out: &mut Vec<Range<usize>>) {}

    /// Appends explicit styles of `line` (char ranges, sorted, not
    /// overlapping), drawn over its spans; colours come from `theme`.
    fn line_styles(
        &self,
        _line: usize,
        _theme: &ui::theme::Theme,
        _out: &mut Vec<(Range<usize>, InlineStyle)>,
    ) {
    }

    /// Appends the char ranges of `line` that are hyperlinks (sorted, not
    /// overlapping): drawn in the link style, with the hand pointer; a click
    /// reports [`EditorEvent::LinkClicked`](crate::EditorEvent).
    fn line_links(&self, _line: usize, _out: &mut Vec<Range<usize>>) {}

    /// Highlighting reflects the current text (no parse pending).
    fn is_current(&self) -> bool {
        true
    }

    /// Char ranges of the syntax nodes that contain `range`, innermost
    /// first (for Extend Selection); empty without a syntax tree.
    fn enclosing_ranges(&self, _buffer: &Buffer, _range: Range<usize>) -> Vec<Range<usize>> {
        Vec::new()
    }
}

/// The URLs of a line inside its strings and comments (chars from the
/// start of the line), found with the console's URL filter
/// (`URLUtil.URL_PATTERN`).
fn line_urls(buffer: &Buffer, line: usize, spans: &[Span]) -> Vec<Range<usize>> {
    use console::LinkFilter as _;
    let literal = |role: TextRole| {
        matches!(
            role,
            TextRole::String
                | TextRole::StringEscape
                | TextRole::Comment
                | TextRole::Docstring
                | TextRole::DocstringTag
                | TextRole::Todo
        )
    };
    let mut out = Vec::new();
    if line >= buffer.len_lines() {
        return out;
    }
    let text = buffer.line(line);
    let len = text.len_chars();
    let mut i = 0;
    while i < spans.len() {
        if !literal(spans[i].1) {
            i += 1;
            continue;
        }
        // A run of string and comment spans next to each other.
        let start = spans[i].0.start;
        let mut end = spans[i].0.end;
        i += 1;
        while i < spans.len() && literal(spans[i].1) && spans[i].0.start == end {
            end = spans[i].0.end;
            i += 1;
        }
        let (start, end) = (start.min(len), end.min(len));
        if end <= start {
            continue;
        }
        let piece = text.slice(start..end).to_string();
        let mut links = Vec::new();
        console::UrlFilter.links(&piece, &mut links);
        for link in links {
            let from = start + piece[..link.range.start].chars().count();
            let to = from + piece[link.range].chars().count();
            out.push(from..to);
        }
    }
    out
}

/// Drops a value on a thread of its own, one value after another.
fn drop_off_thread<T: Send + 'static>(value: T) {
    use std::sync::Mutex;
    use std::sync::mpsc::{Sender, channel};
    type Garbage = Box<dyn Send>;
    static DROPPER: Mutex<Option<Sender<Garbage>>> = Mutex::new(None);
    let mut dropper = DROPPER.lock().unwrap_or_else(|e| e.into_inner());
    let sender = dropper.get_or_insert_with(|| {
        let (tx, rx) = channel::<Garbage>();
        let _ = std::thread::Builder::new()
            .name("editor drop".to_owned())
            .spawn(move || {
                for garbage in rx {
                    drop(garbage)
                }
            });
        tx
    });
    // If the thread is gone, the value is dropped here instead.
    let _ = sender.send(Box::new(value));
}

/// Highlighting with a tree-sitter grammar of the `syntax` crate.
pub struct SyntaxHighlighter {
    language: Arc<syntax::Language>,
    /// The latest tree, told about every edit (it may be older than the text
    /// until the running parse finishes).
    tree: Option<Tree>,
    /// A parse is due.
    dirty: bool,
    /// Cancel flag of the running parse.
    running: Option<Arc<AtomicBool>>,
    /// Edits made while the running parse works on an older text.
    edits_during_job: Vec<InputEdit>,
    /// Lexer state at the start of each line, as far as known.
    states: LineStates,
    /// Spans, error marks and URLs of the prepared lines.
    cache: Vec<Vec<Span>>,
    errors: Vec<Vec<Range<usize>>>,
    urls: Vec<Vec<Range<usize>>>,
    cache_start: usize,
    cache_valid: bool,
}

impl SyntaxHighlighter {
    pub fn new(language: Arc<syntax::Language>) -> Self {
        Self {
            language,
            tree: None,
            dirty: true,
            running: None,
            edits_during_job: Vec::new(),
            states: LineStates::default(),
            cache: Vec::new(),
            errors: Vec::new(),
            urls: Vec::new(),
            cache_start: 0,
            cache_valid: false,
        }
    }

    /// A factory for a file type: one highlighter per editor.
    pub fn factory(language: Arc<syntax::Language>) -> impl Fn() -> Box<dyn Highlighter> {
        move || Box::new(Self::new(language.clone()))
    }
}

impl Highlighter for SyntaxHighlighter {
    fn reset(&mut self, _: &Buffer) {
        if let Some(flag) = self.running.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.tree = None;
        self.dirty = true;
        self.edits_during_job.clear();
        self.states.clear();
        self.cache_valid = false;
    }

    fn edited(&mut self, _: &Buffer, edits: &[AppliedEdit]) {
        for e in edits {
            self.states.invalidate_after(e.start_position.0);
            let edit = syntax::input_edit(e);
            if let Some(tree) = &mut self.tree {
                tree.edit(&edit);
            }
            if self.running.is_some() {
                self.edits_during_job.push(edit);
            }
        }
        self.dirty = true;
        self.cache_valid = false;
    }

    fn next_job(&mut self, buffer: &Buffer) -> Option<Job> {
        if self.running.is_some() || !self.dirty {
            return None;
        }
        self.dirty = false;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some(cancel.clone());
        self.edits_during_job.clear();
        let language = self.language.clone();
        let rope = buffer.rope().clone();
        let old = self.tree.clone();
        // The known lexer states are extended to every line in the
        // background, so that the UI thread never lexes a long stretch of
        // the file itself.
        let mut states = self.states.clone();
        Some(Box::new(move || {
            let tree = syntax::parse(&language, &rope, old.as_ref(), &cancel);
            let states = language.lexer().filter(|_| tree.is_some()).map(|lexer| {
                states.ensure(lexer, &rope, rope.len_lines().saturating_sub(1));
                states
            });
            Box::new((tree, states)) as JobResult
        }))
    }

    fn job_done(&mut self, result: JobResult, _: &Buffer) {
        let cancelled = self
            .running
            .take()
            .is_none_or(|flag| flag.load(Ordering::Relaxed));
        if cancelled {
            // A reset happened meanwhile: this tree belongs to old text.
            return;
        }
        if let Ok(result) = result.downcast::<(Option<Tree>, Option<LineStates>)>()
            && let (Some(mut tree), states) = *result
        {
            let first_edited = self
                .edits_during_job
                .iter()
                .map(|e| e.start_position.row)
                .min();
            for edit in self.edits_during_job.drain(..) {
                tree.edit(&edit);
            }
            // Freeing the tree of a large file takes long (hundreds of
            // milliseconds under memory pressure): not on the UI thread.
            if let Some(old) = self.tree.replace(tree) {
                drop_off_thread(old);
            }
            if let Some(mut states) = states {
                if let Some(row) = first_edited {
                    states.invalidate_after(row);
                }
                if states.len() > self.states.len() {
                    self.states = states;
                }
            }
            self.cache_valid = false;
        }
    }

    fn prepare(&mut self, buffer: &Buffer, lines: Range<usize>) {
        let cached = self.cache_start..self.cache_start + self.cache.len();
        if self.cache_valid && cached.start <= lines.start && lines.end <= cached.end {
            return;
        }
        let h = syntax::highlight(
            &self.language,
            self.tree.as_ref(),
            buffer.rope(),
            lines.clone(),
            &mut self.states,
        );
        self.urls = h
            .spans
            .iter()
            .enumerate()
            .map(|(i, spans)| line_urls(buffer, lines.start + i, spans))
            .collect();
        self.cache = h.spans;
        self.errors = h.errors;
        self.cache_start = lines.start;
        self.cache_valid = true;
    }

    fn line_spans(&self, line: usize, out: &mut Vec<Span>) {
        if let Some(spans) = line
            .checked_sub(self.cache_start)
            .and_then(|i| self.cache.get(i))
        {
            out.extend_from_slice(spans);
        }
    }

    fn line_errors(&self, line: usize, out: &mut Vec<Range<usize>>) {
        if let Some(errors) = line
            .checked_sub(self.cache_start)
            .and_then(|i| self.errors.get(i))
        {
            out.extend_from_slice(errors);
        }
    }

    /// URLs in strings and comments are underlined in their colour (the
    /// reference's web references, `INACTIVE_HYPERLINK_ATTRIBUTES`).
    fn line_styles(
        &self,
        line: usize,
        _theme: &ui::theme::Theme,
        out: &mut Vec<(Range<usize>, InlineStyle)>,
    ) {
        if let Some(urls) = line
            .checked_sub(self.cache_start)
            .and_then(|i| self.urls.get(i))
        {
            let underline = InlineStyle {
                underline: Some(1.0),
                ..InlineStyle::default()
            };
            out.extend(urls.iter().map(|r| (r.clone(), underline)));
        }
    }

    fn is_current(&self) -> bool {
        !self.dirty && self.running.is_none() && self.tree.is_some()
    }

    fn enclosing_ranges(&self, buffer: &Buffer, range: Range<usize>) -> Vec<Range<usize>> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let bytes = buffer.char_to_byte(range.start)..buffer.char_to_byte(range.end);
        let len = buffer.len_bytes();
        syntax::enclosing_nodes(tree, bytes)
            .into_iter()
            .map(|r| buffer.byte_to_char(r.start.min(len))..buffer.byte_to_char(r.end.min(len)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_found_in_strings_and_comments_only() {
        let buffer =
            Buffer::new("x = \"see https://example.com and\"  # www.example.org\nhttp://a.b\n");
        let spans = [(4..33, TextRole::String), (35..53, TextRole::Comment)];
        assert_eq!(line_urls(&buffer, 0, &spans), vec![9..28, 37..52]);
        // Outside a string or a comment: no link.
        assert!(line_urls(&buffer, 1, &[(0..4, TextRole::Keyword)]).is_empty());
    }
}
