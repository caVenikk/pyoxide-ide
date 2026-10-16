//! The version control model of Pyoxide IDE (ADR 0016), without a backend
//! and without a UI toolkit: what the project tree, the editor tabs, the
//! gutter markers, the Commit tool window and the branch widget show.
//!
//! - [`FileStatus`], [`Change`] and the [`StatusMap`] of a repository:
//!   the status of any path, ignored folders counting for everything below
//!   them, and an unsaved document of a clean tracked file counting as
//!   modified, as the reference's `FileStatusManagerImpl` does.
//! - [`LineRanges`]: the changed lines of a document against its base text
//!   (git's histogram diff through `imara-diff`), shifted on edits until
//!   they are computed again.
//! - [`Head`], [`RepoState`], [`RepoInfo`] and [`Branch`]: where the
//!   repository is and what it has.
//! - [`Backend`]: what a version control backend (`git`) offers; UI crates
//!   depend on this trait, never on the backend.

mod backend;
mod branch;
mod ranges;
mod status;

pub use backend::{Backend, Cancel, Snapshot, VcsError, VcsErrorKind};
pub use branch::{Branch, BranchKind, Head, Remote, RepoInfo, RepoState, Tracking};
pub use ranges::{LineRanges, Range, RangeKind, split_lines};
pub use status::{Change, FileStatus, StatusMap};
