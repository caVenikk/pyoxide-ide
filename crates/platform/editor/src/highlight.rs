//! The interface between the editor and a source of token styles, and the
//! tree-sitter implementation of it.
//!
//! The editor asks a highlighter for the spans of the lines it paints. Heavy
//! work (parsing) is handed to the editor as a [`Job`], which the editor runs
//! on the background executor; the result comes back through
//! [`Highlighter::job_done`] on the UI thread. Edits are applied to the
//! current syntax tree at once, so highlighting stays in place while a
//! reparse runs, and typing never waits for the parser.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use syntax::tree_sitter::{InputEdit, Tree};
use text::{AppliedEdit, Buffer};
use themes::TextRole;

/// A styled range of one line: chars from the start of the line.
pub type Span = (Range<usize>, TextRole);

/// Work for a background thread and its result.
pub type JobResult = Box<dyn Any + Send>;
pub type Job = Box<dyn FnOnce() -> JobResult + Send>;

pub trait Highlighter: 'static {
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
    /// Spans of the prepared lines.
    cache: Vec<Vec<Span>>,
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
            cache: Vec::new(),
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
        self.cache_valid = false;
    }

    fn edited(&mut self, _: &Buffer, edits: &[AppliedEdit]) {
        for e in edits {
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
        Some(Box::new(move || {
            let tree = syntax::parse(&language, &rope, old.as_ref(), &cancel);
            Box::new(tree) as JobResult
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
        if let Ok(tree) = result.downcast::<Option<Tree>>()
            && let Some(mut tree) = *tree
        {
            for edit in self.edits_during_job.drain(..) {
                tree.edit(&edit);
            }
            self.tree = Some(tree);
            self.cache_valid = false;
        }
    }

    fn prepare(&mut self, buffer: &Buffer, lines: Range<usize>) {
        let cached = self.cache_start..self.cache_start + self.cache.len();
        if self.cache_valid && cached.start <= lines.start && lines.end <= cached.end {
            return;
        }
        let Some(tree) = &self.tree else {
            self.cache.clear();
            self.cache_valid = true;
            return;
        };
        self.cache = syntax::highlight(&self.language, tree, buffer.rope(), lines.clone());
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
