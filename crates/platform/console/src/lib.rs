//! The console's text model of Pyoxide IDE: what the Run tool window shows.
//!
//! A [`Console`] is fed with raw bytes from a process (see the `process`
//! crate) and with the IDE's own text (the command line, the exit line, echoed
//! input). It decodes them the way the reference console does without
//! "Emulate terminal in output console":
//!
//! - UTF-8, incrementally (a character split between chunks is joined);
//! - ANSI escape sequences of standard output and standard error, each
//!   stream with its own state: SGR attributes become [`Style`]s (16, 256
//!   and 24-bit colours, bold, faint, italic, underline, inverse, crossed
//!   out); other CSI sequences, `ESC =` and `ESC >` are dropped; any other
//!   escape sequence (OSC included) stays in the text as the reference
//!   leaves it;
//! - `\r\n` is a line break, also when a chunk ends between the two (a
//!   trailing `\r` is held until the stream's next chunk, as the reference's
//!   output reader joins them); a lone `\r` clears the current line, so the
//!   following text replaces it (no column overwrite: the reference does
//!   not emulate a terminal); `\b` deletes the previous character of the
//!   current line.
//!
//! Lines are kept in a cycle buffer like the reference's (1024 KB by
//! default, counted in UTF-16 units as the reference's document is; the
//! oldest text is cut off exactly at the front, so the first line may lose
//! its start). Each line carries style runs with the [`ContentKind`] of its
//! text and the hyperlinks found by the [`LinkFilter`]s.
//! [`Console::take_changes`] tells a view which lines to reload.
//!
//! No UI toolkit and no theme here: styles are data, a view maps them to its
//! theme.

mod ansi;
mod buffer;
mod link;
mod utf8;

pub use ansi::{Color, Style, Underline};
pub use buffer::{Changes, Console, DEFAULT_CYCLE_BUFFER_SIZE, Line, StyleRun};
pub use link::{Link, LinkFilter, LinkTarget, UrlFilter};

/// Where a piece of console text comes from (the reference's
/// `ConsoleViewContentType`: normal output, error output, system output and
/// user input). A view colours plain text by its kind; text with an ANSI
/// style is coloured by the style instead (see [`Style`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentKind {
    /// Standard output.
    Normal,
    /// Standard error.
    Error,
    /// The IDE's own lines: the command line and "Process finished with exit
    /// code N". No ANSI decoding.
    System,
    /// Text the user typed into the console, echoed. No ANSI decoding.
    UserInput,
}
