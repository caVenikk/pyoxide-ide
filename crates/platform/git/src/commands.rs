//! The `git.*` engine commands: the same functions the IDE uses, for the
//! command line and agents.

use std::path::{Path, PathBuf};

use commands::{CommandError, CommandMeta, Registry, typed};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use vcs::{Backend, Cancel, VcsError};

use crate::{GitExe, GitRepo, Repository};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArgs {
    /// A folder in the work tree; the current folder when missing.
    #[serde(default)]
    path: Option<String>,
}

/// Registers `git.status`, `git.branches` and `git.state`.
pub fn register(registry: &mut Registry) {
    registry.register(typed(
        CommandMeta {
            id: "git.status",
            title: "Git Status",
            summary: "Returns the repository at or above a folder with the status of its changed, unversioned and ignored files.",
            args_example: r#"{"path": "."}"#,
        },
        |a: PathArgs| off_thread(move || status(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.branches",
            title: "Git Branches",
            summary: "Lists the local and remote branches and tags of the repository at or above a folder, with upstreams and ahead / behind counts.",
            args_example: r#"{"path": "."}"#,
        },
        |a: PathArgs| off_thread(move || branches(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.state",
            title: "Git Repository State",
            summary: "Returns the git executable and the repository at or above a folder: HEAD, the operation in progress, remotes.",
            args_example: r#"{"path": "."}"#,
        },
        |a: PathArgs| off_thread(move || state(a)),
    ));
}

/// Runs blocking work on a thread of its own and awaits it.
async fn off_thread(
    f: impl FnOnce() -> Result<Json, CommandError> + Send + 'static,
) -> Result<Json, CommandError> {
    let (tx, rx) = flume::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_async()
        .await
        .map_err(|_| CommandError::new("internal", "the git command stopped"))?
}

fn folder(args: &PathArgs) -> Result<PathBuf, CommandError> {
    let path = match &args.path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().map_err(|e| CommandError::new("io", e.to_string()))?,
    };
    std::fs::canonicalize(&path)
        .map_err(|e| CommandError::not_found(format!("{}: {e}", path.display())))
}

fn open(args: &PathArgs) -> Result<(PathBuf, GitRepo), CommandError> {
    let folder = folder(args)?;
    let exe =
        GitExe::find(None).map_err(|e| CommandError::new("git.unavailable", e.to_string()))?;
    let repo = Repository::detect(&folder).ok_or_else(|| {
        CommandError::new(
            "git.no_repository",
            format!("{} is not in a Git work tree", folder.display()),
        )
    })?;
    Ok((folder, GitRepo::new(repo, exe)))
}

fn failed(e: VcsError) -> CommandError {
    CommandError::new("git.failed", e.message)
}

fn status(args: PathArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&args)?;
    let snapshot = repo.status(None, &Cancel::new()).map_err(failed)?;
    Ok(crate::snapshot_json(&snapshot))
}

fn branches(args: PathArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&args)?;
    let list = repo.branches(&Cancel::new()).map_err(failed)?;
    Ok(json!({ "branches": list }))
}

fn state(args: PathArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&args)?;
    let info = repo.info().map_err(failed)?;
    let exe = repo.runner().exe();
    Ok(json!({
        "git": { "path": exe.path.display().to_string(), "version": exe.version.to_string() },
        "repository": crate::info_json(&info),
    }))
}

/// Shortens `path` for JSON: relative to `root` with `/`, or as it is.
pub(crate) fn rel(root: &Path, path: &Path) -> String {
    crate::parse::relative(root, path).unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_registered() {
        let mut r = Registry::new();
        register(&mut r);
        let ids: Vec<&str> = r.list().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["git.branches", "git.state", "git.status"]);
    }
}
