//! Git for Pyoxide IDE through the git executable (ADR 0016).
//!
//! - [`GitExe`]: the executable from the settings, `PATH` or the usual
//!   folders (on macOS `/usr/bin/git` only with the developer tools), with
//!   its version (2.17 at least, 2.19.2 on Windows).
//! - [`Runner`]: every call with `-c core.quotepath=false -c
//!   log.showSignature=false`, no terminal prompts, reads with
//!   `GIT_OPTIONAL_LOCKS=0` in parallel, writes one at a time and not
//!   during reads; any call can be cancelled.
//! - [`Repository`]: detection at or above a folder (`.git` folders and
//!   `gitdir:` files) and readers of `HEAD`, refs, `packed-refs`, the
//!   config and the state markers; a reftable repository asks git.
//! - [`GitRepo`]: the [`vcs::Backend`]: status (`status --porcelain=v2`),
//!   branches (`for-each-ref`), base texts (`cat-file --filters`, cached by
//!   blob).
//! - [`StatusService`]: statuses kept current from file watcher batches.
//! - [`idea`]: the reference IDE's `.idea/vcs.xml` mapping.
//! - [`commands::register`]: `git.status`, `git.branches`, `git.state`.

mod backend;
pub mod commands;
mod exe;
pub mod idea;
mod parse;
mod repo;
mod runner;
mod service;

use std::path::Path;

use serde_json::{Value as Json, json};

pub use backend::{GitRepo, MAX_PATHSPECS};
pub use exe::{GitExe, LookupError, Version};
pub use repo::{Config, Repository, watched_in_git_dir};
pub use runner::{Access, Output, Runner, error_text};
pub use service::{DEBOUNCE, StatusService, StatusUpdate};

/// A repository's state as JSON (`git.state`, `workbench.state`).
pub fn info_json(info: &vcs::RepoInfo) -> Json {
    json!({
        "root": info.root.display().to_string(),
        "git_dir": info.git_dir.display().to_string(),
        "head": info.head,
        "branch": info.head.branch(),
        "state": info.state,
        "rebasing_branch": info.rebasing_branch,
        "tracking": info.tracking,
        "remotes": info.remotes,
    })
}

/// A snapshot as JSON (`git.status`, `workbench.state`): the state, and
/// the changed, unversioned and ignored paths relative to the root with
/// `/` separators.
pub fn snapshot_json(snapshot: &vcs::Snapshot) -> Json {
    let root = snapshot.info.root.as_path();
    let rel = |p: &Path| commands::relative_path(root, p);
    let changes: Vec<Json> = snapshot
        .status
        .changes()
        .iter()
        .map(|c| {
            json!({
                "path": rel(&c.path),
                "status": c.status.id(),
                "old_path": c.old_path.as_deref().map(rel),
            })
        })
        .collect();
    json!({
        "repository": info_json(&snapshot.info),
        "generation": snapshot.generation,
        "changes": changes,
        "unversioned": snapshot.status.unversioned().iter().map(|p| rel(p)).collect::<Vec<_>>(),
        "ignored": snapshot.status.ignored().iter().map(|p| rel(p)).collect::<Vec<_>>(),
    })
}
