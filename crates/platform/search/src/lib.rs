//! Text search of Pyoxide IDE. Find and Replace in the editor and Find in
//! Files use the same engine.
//!
//! A [`Query`] (text and [`SearchOptions`]) compiles into a [`Matcher`],
//! which finds matches in a `&str`: plain text or a regular expression (the
//! syntax of the `regex` crate), with or without matching case, optionally
//! restricted to whole words. A search works on an immutable snapshot of the
//! text, so it can run off the UI thread, and a long search stops early when
//! its [`CancelToken`] is cancelled. The replacement for a regular
//! expression match is built from a template with group references (`$1`,
//! `${name}`), escapes (`\n`, `\t`) and case conversion (`\u`, `\U...\E`).
//!
//! Positions are byte offsets into the searched text; [`to_char_ranges`]
//! converts sorted matches to char offsets for the text model. Line breaks
//! are expected to be `\n` (the text model normalises them); `^` and `$` also
//! treat `\r\n` as one line break, so raw file text works as well.
//!
//! No UI toolkit here: the crate is tested on its own.

mod cancel;
mod matcher;
mod offsets;
mod replace;

pub use cancel::{CancelToken, Cancelled};
pub use matcher::{
    Found, Matcher, Matches, Query, QueryError, SearchOptions, escape, is_word_char,
};
pub use offsets::{match_lines, to_byte_ranges, to_char_ranges};
pub use replace::ReplaceError;
