//! The `git.*` engine commands: the same functions the IDE uses, for the
//! command line and agents.

use std::path::{Path, PathBuf};

use commands::{CommandError, CommandMeta, Registry, typed};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use vcs::{Backend, Cancel, VcsError};

use crate::{CheckoutError, CheckoutMode, GitExe, GitRepo, Repository};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArgs {
    /// A folder in the work tree; the current folder when missing.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutArgs {
    #[serde(default)]
    path: Option<String>,
    /// A local or remote branch, a tag or a commit.
    #[serde(rename = "ref")]
    target: String,
    #[serde(default)]
    mode: CheckoutMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewBranchArgs {
    #[serde(default)]
    path: Option<String>,
    name: String,
    /// Where the branch starts; `HEAD` when missing.
    #[serde(default)]
    from: Option<String>,
    #[serde(default = "yes")]
    checkout: bool,
    #[serde(default)]
    overwrite: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchArgs {
    #[serde(default)]
    path: Option<String>,
    /// One remote; every remote when missing.
    #[serde(default)]
    remote: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushArgs {
    #[serde(default)]
    path: Option<String>,
    /// The local branch; the current one when missing.
    #[serde(default)]
    branch: Option<String>,
    /// `origin/main`; the branch's default target when missing.
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    set_upstream: Option<bool>,
    /// `none`, `with_lease` or `force`.
    #[serde(default)]
    force: Option<String>,
    /// `all` or `current_branch`.
    #[serde(default)]
    tags: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateArgs {
    #[serde(default)]
    path: Option<String>,
    /// `merge` (the default) or `rebase`.
    #[serde(default)]
    method: Option<String>,
    /// A remote branch to pull from (`origin/main`); the tracked branch
    /// when missing.
    #[serde(default)]
    from: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    #[serde(default)]
    path: Option<String>,
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteArgs {
    #[serde(default)]
    path: Option<String>,
    name: String,
}

/// Registers `git.status`, `git.branches`, `git.state` and the branch
/// operations.
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
    registry.register(typed(
        CommandMeta {
            id: "git.checkout",
            title: "Git Checkout",
            summary: "Checks out a local branch, a remote branch (as a new local branch tracking it), a tag or a commit; mode normal stops at local changes in the way, force overwrites them, smart stashes them around the checkout.",
            args_example: r#"{"path": ".", "ref": "main", "mode": "smart"}"#,
        },
        |a: CheckoutArgs| off_thread(move || checkout(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.new-branch",
            title: "Git New Branch",
            summary: "Creates a branch at a starting point (HEAD by default) and checks it out unless checkout is false; overwrite resets an existing branch of the name.",
            args_example: r#"{"path": ".", "name": "feat/x", "from": "main", "checkout": true}"#,
        },
        |a: NewBranchArgs| off_thread(move || new_branch(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.rename-branch",
            title: "Git Rename Branch",
            summary: "Renames a local branch.",
            args_example: r#"{"path": ".", "from": "feat/x", "to": "feat/y"}"#,
        },
        |a: RenameArgs| off_thread(move || rename_branch(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.delete-branch",
            title: "Git Delete Branch",
            summary: "Deletes a local branch, merged or not, and returns the commit it pointed at.",
            args_example: r#"{"path": ".", "name": "feat/x"}"#,
        },
        |a: DeleteArgs| off_thread(move || delete_branch(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.init",
            title: "Create Git Repository",
            summary: "Creates a Git repository in a folder (git init); the current folder when no path is given.",
            args_example: r#"{"path": "."}"#,
        },
        |a: PathArgs| off_thread(move || init(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.fetch",
            title: "Git Fetch",
            summary: "Fetches every remote (or one) with --prune and returns each remote's error and pruned references.",
            args_example: r#"{"path": ".", "remote": "origin"}"#,
        },
        |a: FetchArgs| off_thread(move || fetch(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.push",
            title: "Git Push",
            summary: "Pushes a local branch (the current one by default) to its default target or to target (origin/main); force with_lease or force, tags all or current_branch, set_upstream. Returns the outcome and the balloon's text.",
            args_example: r#"{"path": ".", "target": "origin/main", "set_upstream": false}"#,
        },
        |a: PushArgs| off_thread(move || push(a)),
    ));
    registry.register(typed(
        CommandMeta {
            id: "git.update",
            title: "Git Update Project",
            summary: "Fetches the tracked remote of the current branch (or pulls from `from`) and merges or rebases the incoming commits, with local changes stashed around it; conflicts leave the repository as it is.",
            args_example: r#"{"path": ".", "method": "rebase"}"#,
        },
        |a: UpdateArgs| off_thread(move || update(a)),
    ));
}

/// `origin/feat/x` as its remote and branch, by the repository's remotes.
pub fn split_remote_branch(repo: &GitRepo, name: &str) -> Option<(String, String)> {
    repo.remote_names()
        .into_iter()
        .filter_map(|r| {
            name.strip_prefix(&r)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(|rest| (r.clone(), rest.to_owned()))
        })
        .max_by_key(|(r, _)| r.len())
}

fn fetch(args: FetchArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let c = Cancel::new();
    let result = match args.remote {
        Some(remote) => crate::FetchResult {
            remotes: vec![repo.fetch(&remote, None, &c).map_err(failed)?],
        },
        None => repo.fetch_all(&c).map_err(failed)?,
    };
    Ok(json!({
        "success": result.is_success(),
        "message": result.message(),
        "remotes": result.remotes,
    }))
}

fn push(args: PushArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let c = Cancel::new();
    let source = match args.branch {
        Some(b) => b,
        None => repo
            .info()
            .map_err(failed)?
            .head
            .branch()
            .map(str::to_owned)
            .ok_or_else(|| CommandError::new("git.detached", "HEAD is not on a branch"))?,
    };
    let target = match &args.target {
        Some(t) => {
            let (remote, branch) = split_remote_branch(&repo, t).ok_or_else(|| {
                CommandError::new("git.invalid_target", format!("No remote in {t}"))
            })?;
            repo.push_target(&remote, &branch, &c).map_err(failed)?
        }
        None => repo
            .default_push_target(&source, &c)
            .map_err(failed)?
            .ok_or_else(|| CommandError::new("git.no_remote", "The repository has no remotes"))?,
    };
    let force = match args.force.as_deref() {
        None => vcs::ForceMode::None,
        Some(f) => vcs::ForceMode::from_id(f)
            .ok_or_else(|| CommandError::new("git.invalid_force", format!("Unknown force {f}")))?,
    };
    let tags =
        match args.tags.as_deref() {
            None => None,
            Some(t) => Some(vcs::TagMode::from_id(t).ok_or_else(|| {
                CommandError::new("git.invalid_tags", format!("Unknown tags {t}"))
            })?),
        };
    let spec = vcs::PushSpec {
        source,
        set_upstream: args.set_upstream.unwrap_or(target.new),
        target,
        force,
        tags,
    };
    let result = repo.push(&spec, &c).map_err(failed)?;
    Ok(json!({
        "outcome": result.outcome,
        "description": result.description(),
        "tags": result.tags,
        "target": result.target,
    }))
}

fn update(args: UpdateArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let method = match args.method.as_deref() {
        None => vcs::UpdateMethod::Merge,
        Some(m) => vcs::UpdateMethod::from_id(m).ok_or_else(|| {
            CommandError::new("git.invalid_method", format!("Unknown method {m}"))
        })?,
    };
    let from =
        match &args.from {
            Some(f) => Some(split_remote_branch(&repo, f).ok_or_else(|| {
                CommandError::new("git.invalid_target", format!("No remote in {f}"))
            })?),
            None => None,
        };
    let result = repo
        .update(
            method,
            from.as_ref().map(|(r, b)| (r.as_str(), b.as_str())),
            &Cancel::new(),
        )
        .map_err(failed)?;
    Ok(serde_json::to_value(result).unwrap_or(Json::Null))
}

/// The JSON of a checkout's failure.
pub fn checkout_error(e: CheckoutError) -> CommandError {
    match e {
        CheckoutError::LocalChanges(files) => CommandError::new(
            "git.local_changes",
            format!(
                "Your local changes to the following files would be overwritten by checkout: {}",
                files.join(", ")
            ),
        ),
        CheckoutError::Untracked(files) => CommandError::new(
            "git.untracked_files",
            format!(
                "The following untracked working tree files would be overwritten by checkout: {}",
                files.join(", ")
            ),
        ),
        CheckoutError::NotFound(target) => {
            CommandError::not_found(format!("Revision not found: {target}"))
        }
        CheckoutError::Failed(e) => failed(e),
    }
}

fn checkout(args: CheckoutArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let done = repo
        .checkout(&args.target, args.mode, &Cancel::new())
        .map_err(checkout_error)?;
    Ok(json!({
        "checked_out": done.checked_out,
        "conflicts_on_restore": done.conflicts_on_restore,
    }))
}

fn new_branch(args: NewBranchArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let branches = repo.branches(&Cancel::new()).map_err(failed)?;
    let name = vcs::branch_name::clean_up(&args.name);
    vcs::branch_name::check_new(&name, &branches, args.overwrite, true)
        .map_err(|e| CommandError::new("git.invalid_name", e.to_string()))?;
    repo.new_branch(
        &name,
        args.from.as_deref(),
        args.checkout,
        args.overwrite,
        &Cancel::new(),
    )
    .map_err(checkout_error)?;
    Ok(json!({ "name": name, "checked_out": args.checkout }))
}

fn rename_branch(args: RenameArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let branches = repo.branches(&Cancel::new()).map_err(failed)?;
    let to = vcs::branch_name::clean_up(&args.to);
    vcs::branch_name::check_new(&to, &branches, false, true)
        .map_err(|e| CommandError::new("git.invalid_name", e.to_string()))?;
    repo.rename_branch(&args.from, &to, &Cancel::new())
        .map_err(failed)?;
    Ok(json!({ "from": args.from, "to": to }))
}

fn delete_branch(args: DeleteArgs) -> Result<Json, CommandError> {
    let (_, repo) = open(&PathArgs { path: args.path })?;
    let oid = repo
        .delete_branch(&args.name, &Cancel::new())
        .map_err(failed)?;
    Ok(json!({ "name": args.name, "oid": oid }))
}

fn init(args: PathArgs) -> Result<Json, CommandError> {
    let folder = folder(&args)?;
    let exe =
        GitExe::find(None).map_err(|e| CommandError::new("git.unavailable", e.to_string()))?;
    crate::branches::init(&exe, &folder, &Cancel::new()).map_err(failed)?;
    Ok(json!({ "root": folder.display().to_string() }))
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

/// A path for JSON: relative to `root` with `/` separators, or as it is
/// when outside.
pub fn relative_path(root: &Path, path: &Path) -> String {
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
        assert_eq!(
            ids,
            vec![
                "git.branches",
                "git.checkout",
                "git.delete-branch",
                "git.fetch",
                "git.init",
                "git.new-branch",
                "git.push",
                "git.rename-branch",
                "git.state",
                "git.status",
                "git.update"
            ]
        );
    }
}
