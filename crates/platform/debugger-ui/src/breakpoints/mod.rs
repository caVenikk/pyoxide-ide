//! Breakpoints in the UI (#164): the gutter of an editor ([`gutter`]),
//! the properties popup ([`popup`]), the Breakpoints dialog ([`dialog`]),
//! how breakpoints are shown ([`presentation`]) and the commands
//! ([`commands`]).

pub mod commands;
pub mod dialog;
pub mod gutter;
pub mod popup;
pub mod presentation;

use std::path::Path;

/// A path as `debug.state` shows it: relative to the project root with
/// `/`, else absolute.
pub(crate) fn storage_path(path: &Path, root: &Path) -> String {
    debugger::storage::stored_path(path, root)
}
