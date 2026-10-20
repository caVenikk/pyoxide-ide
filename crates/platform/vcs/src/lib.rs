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
//! - [`branch_name`]: names of new branches checked and cleaned as the
//!   reference's New Branch dialog does.
//! - [`CommitRequest`], [`LastCommit`] and [`rollback_kind`]: what the
//!   Commit tool window asks of a backend.
//! - [`Backend`]: what a version control backend (`git`) offers; UI crates
//!   depend on this trait, never on the backend.
//! - [`remote`]: pushes, their targets and results, update methods and the
//!   commits a push sends.

mod backend;
mod branch;
pub mod branch_name;
mod commit;
mod ranges;
pub mod remote;
mod status;

pub use backend::{Backend, Cancel, Snapshot, VcsError, VcsErrorKind};
pub use branch::{Branch, BranchKind, Head, Remote, RepoInfo, RepoState, Tracking};
pub use commit::{CommitRequest, LastCommit, PartialFile, RollbackKind, rollback_kind};
pub use ranges::{LineRanges, Range, RangeKind, partial_text, split_lines};
pub use remote::{
    CommitFile, ForceMode, OutgoingCommit, PushOutcome, PushResult, PushSpec, PushTarget, TagMode,
    UpdateMethod,
};
pub use status::{Change, FileStatus, StatusMap};
