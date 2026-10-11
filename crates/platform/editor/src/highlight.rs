//! The interface between the editor and a source of token styles. The
//! editor asks for the spans of the lines it paints; a highlighter keeps
//! itself current through the edits it is told about. Highlighters do their
//! heavy work off the UI thread and only answer from what they have.

use std::ops::Range;

use text::{AppliedEdit, Buffer};
use themes::TextRole;

/// A styled range of one line: chars from the start of the line.
pub type Span = (Range<usize>, TextRole);

pub trait Highlighter: 'static {
    /// The whole text was (re)loaded.
    fn reset(&mut self, buffer: &Buffer);

    /// The text changed by these replacements (in the order applied).
    fn edited(&mut self, buffer: &Buffer, edits: &[AppliedEdit]);

    /// Appends the spans of `line` to `out`, sorted and not overlapping.
    fn line_spans(&self, line: usize, out: &mut Vec<Span>);

    /// The lines the editor is about to paint, so work can be prioritised.
    fn set_visible(&mut self, _lines: Range<usize>) {}
}
