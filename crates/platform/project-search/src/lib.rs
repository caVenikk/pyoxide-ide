//! Find in Files of Pyoxide IDE: searching the files of a project.
//!
//! A [`FileSearch`] looks for a compiled query (`search::Matcher`) in a
//! [`Scope`] (a folder, recursively or not, or a list of files). Everything
//! that is project content is searched: only the project's hidden and
//! excluded entries (`project::Filter`) are left out, ignore files such as
//! `.gitignore` are not exclusions. As in the reference IDE, binary files
//! and files larger than the editor opens are skipped, matches in the
//! reference's project configuration files (`.idea`, `*.iml`) are held back
//! unless asked for, and open documents with unsaved changes are searched
//! instead of their files. A [`FileMask`]
//! narrows the files by name. Files are searched on all cores and every file
//! with matches goes to the caller as soon as it is done ([`FileMatches`],
//! its lines with the matches as char columns), so results can be shown
//! while the search runs; a `search::CancelToken` stops it, and it stops by
//! itself at a limit. [`Results`] keeps what was found as rows in a stable
//! order, for a list that shows them while they come in.
//!
//! No UI toolkit here: the crate is tested on its own.

mod find;
mod mask;
mod replace;
mod results;

pub use find::{
    FileMatches, FileSearch, LineMatch, MAX_LINE_CHARS, PROJECT_STORE, Scope, Summary, file_text,
    is_project_file, line_matches,
};
pub use mask::{FileMask, MaskError};
pub use replace::{FileReplaceError, Replacement, apply, replace_in_file, replacements};
pub use results::{Results, Row};
