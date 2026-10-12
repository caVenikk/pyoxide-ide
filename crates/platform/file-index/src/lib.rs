//! File name index of Pyoxide IDE and Go to File.
//!
//! [`FileIndex`] holds every file and folder of a project by name. It is
//! built by one parallel walk of the project with the project tree's rules
//! ([`project::Filter`]): hidden names are left out, excluded folders are
//! listed but their content only on request ([`FileIndex::with_non_project`],
//! for "Include non-project items"). Changes made by the application go in
//! with [`FileIndex::add`], [`FileIndex::remove`] and [`FileIndex::rename`],
//! and changes seen on disk can come the same way ([`Change`]).
//!
//! [`FileIndex::goto_file`] finds files and folders as the reference's Go
//! to File does (see the `goto` module), and [`parse_position`] reads the
//! `file:line:column` forms it accepts.
//!
//! No UI toolkit here: the crate is tested on its own.

mod goto;
mod index;
mod position;

#[cfg(test)]
mod reference_tests;

use std::path::PathBuf;

pub use goto::{
    DIRECTORY_MATCH_DEGREE, EXACT_MATCH_DEGREE, FileHit, GotoQuery, qualifier_matcher, rank,
    sanitized_pattern,
};
pub use index::{EntryId, FileIndex, ROOT};
pub use position::{Position, parse_position};

/// A change of the project's files, as a file watcher reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Created { path: PathBuf, is_dir: bool },
    Removed(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

impl FileIndex {
    /// Applies changes in order.
    pub fn apply(&mut self, changes: &[Change], filter: &project::Filter) {
        for change in changes {
            match change {
                Change::Created { path, is_dir } => self.add(path, *is_dir, filter),
                Change::Removed(path) => self.remove(path),
                Change::Renamed { from, to } => self.rename(from, to, filter),
            }
        }
    }
}

#[cfg(test)]
mod tests;
