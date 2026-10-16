use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Head {
    /// A local branch; `oid` is `None` before its first commit.
    Branch { name: String, oid: Option<String> },
    /// A commit, not a branch (a checked out tag or revision, a rebase).
    Detached { oid: String },
}

impl Head {
    /// The branch's short name.
    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Branch { name, .. } => Some(name),
            Self::Detached { .. } => None,
        }
    }

    /// The commit, when there is one.
    pub fn oid(&self) -> Option<&str> {
        match self {
            Self::Branch { oid, .. } => oid.as_deref(),
            Self::Detached { oid } => Some(oid),
        }
    }
}

/// An operation in progress (the reference's `Repository.State`; a
/// detached `HEAD` is [`Head::Detached`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoState {
    Normal,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

/// The branch a local branch tracks, with the commits between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracking {
    /// Short name of the upstream (`origin/main`).
    pub upstream: String,
    /// Commits on the local branch that the upstream has not.
    pub ahead: u32,
    /// Commits on the upstream that the local branch has not.
    pub behind: u32,
    /// The upstream branch is gone.
    pub gone: bool,
}

/// A remote of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub urls: Vec<String>,
}

/// Where a repository is: its folders, `HEAD`, the operation in progress,
/// the current branch's upstream and the remotes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    /// The work tree.
    pub root: PathBuf,
    /// The repository folder (`.git`, or the folder a `gitdir:` file names).
    pub git_dir: PathBuf,
    pub head: Head,
    pub state: RepoState,
    /// The branch being rebased ("Rebasing {b}"), during a rebase.
    pub rebasing_branch: Option<String>,
    pub tracking: Option<Tracking>,
    pub remotes: Vec<Remote>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchKind {
    Local,
    Remote,
    Tag,
}

/// A branch or tag of the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// Short name (`main`, `origin/main`, `v1.0`).
    pub name: String,
    pub kind: BranchKind,
    /// The commit (for an annotated tag, the commit it points at).
    pub oid: String,
    /// `HEAD` is this branch.
    pub current: bool,
    /// A local branch's upstream.
    pub tracking: Option<Tracking>,
}
