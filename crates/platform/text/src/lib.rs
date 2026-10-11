//! The text model of Pyoxide IDE (ADR 0007): a rope buffer (`ropey`) with
//! line and offset conversions, selections ready for multiple carets, edits
//! grouped into undoable transactions, and the encoding and line separator
//! of the file. Positions are char offsets; conversions to bytes
//! (tree-sitter) and UTF-16 (platform input methods) are on [`Buffer`].
//!
//! No UI toolkit here: the crate is tested on its own.

mod buffer;
mod encoding;
mod history;
pub mod movement;
mod selection;

pub use buffer::{AppliedEdit, Buffer, CaretPlacement, Edit, Point};
pub use encoding::{
    DecodeError, Decoded, Encoding, LineEnding, decode, encode, normalize_line_endings,
};
pub use history::{EditKind, GROUP_INTERVAL};
pub use ropey::{Rope, RopeSlice};
pub use selection::{Selection, SelectionSet};
