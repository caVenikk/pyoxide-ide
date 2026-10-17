//! Remote operations (the reference's `GitPushOperation`,
//! `GitFetchSupportImpl`, `GitUpdateProcess` with its merge and rebase
//! updaters): fetch with `--prune`, push with `--porcelain` and its
//! results, the default push target and the commits a push sends, and
//! Update Project: fetch the tracked remote, then merge or rebase with
//! the local changes stashed around it.
//!
//! git never asks for credentials here (`GIT_TERMINAL_PROMPT=0`): remotes
//! that need them work through credential helpers or SSH agents, else
//! git's message is the result. Messages shown never carry the password
//! or token of a URL.

use std::collections::HashMap;

use serde::Serialize;
use vcs::{
    Cancel, CommitFile, FileStatus, ForceMode, OutgoingCommit, PushOutcome, PushResult, PushSpec,
    PushTarget, TagMode, UpdateMethod, VcsError,
};

use crate::backend::GitRepo;
use crate::runner::{Access, Output, error_text};

/// One ref of `git push --porcelain` (the reference's
/// `GitPushNativeResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePush {
    /// ` ` fast-forward, `+` forced, `-` deleted, `*` new, `!` rejected,
    /// `=` up to date.
    pub flag: char,
    pub from: String,
    pub to: String,
    pub summary: String,
    pub reason: Option<String>,
}

impl NativePush {
    /// `abc123..def456` (or `...` when forced) of a successful update.
    pub fn range(&self) -> Option<&str> {
        let s = self.summary.as_str();
        let (a, b) = s.split_once("...").or_else(|| s.split_once(".."))?;
        let hex = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit());
        (hex(a) && hex(b)).then_some(s)
    }

    fn reason_has(&self, text: &str) -> bool {
        self.reason
            .as_deref()
            .is_some_and(|r| r.to_ascii_lowercase().contains(text))
    }
}

/// Parses the ref lines of `git push --porcelain`:
/// `<flag> \t <from>:<to> \t <summary> (<reason>)`.
pub fn parse_push(out: &str) -> Vec<NativePush> {
    let mut results = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let Some(flag) = parts[0].chars().last() else {
            continue;
        };
        if !" +-*!=".contains(flag) {
            continue;
        }
        let Some((from, to)) = parts[1].split_once(':') else {
            continue;
        };
        if from.is_empty() || to.is_empty() {
            continue;
        }
        let rest = parts[2];
        let (summary, reason) = match rest.find('(') {
            Some(at) => {
                let reason = rest[at + 1..]
                    .rfind(')')
                    .map(|end| rest[at + 1..at + 1 + end].to_owned());
                (rest[..at].trim().to_owned(), reason)
            }
            None => (rest.trim().to_owned(), None),
        };
        results.push(NativePush {
            flag,
            from: from.to_owned(),
            to: to.to_owned(),
            summary,
            reason,
        });
    }
    results
}

/// `text` without the user information of URLs (`https://user:token@host`
/// is `https://host`), so that no password or token is ever shown.
pub fn redact_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '/' || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(i) => out.push_str(&authority[i + 1..]),
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// The message of a failed remote command, credentials removed.
fn remote_error(out: &Output) -> String {
    redact_credentials(&error_text(out))
}

/// A remote's part of a fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteFetch {
    pub remote: String,
    /// git's message when it failed.
    pub error: Option<String>,
    /// Remote branches gone from the remote (`origin/old`).
    pub pruned: Vec<String>,
}

/// A fetch of one or more remotes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FetchResult {
    pub remotes: Vec<RemoteFetch>,
}

impl FetchResult {
    pub fn is_success(&self) -> bool {
        self.remotes.iter().all(|r| r.error.is_none())
    }

    /// The balloon's text (`buildMessage`): the errors, then the pruned
    /// references; prefixed with the remote when there are several.
    pub fn message(&self) -> String {
        let several = self.remotes.len() > 1;
        let prefixed = |remote: &str, text: &str| {
            if several {
                format!("{remote}: {text}")
            } else {
                text.to_owned()
            }
        };
        let mut lines: Vec<String> = Vec::new();
        for r in &self.remotes {
            if let Some(e) = &r.error {
                lines.push(prefixed(&r.remote, e));
            }
        }
        let pruned: Vec<&String> = self.remotes.iter().flat_map(|r| &r.pruned).collect();
        if !pruned.is_empty() {
            let word = if pruned.len() == 1 {
                "reference"
            } else {
                "references"
            };
            let names: Vec<&str> = pruned.iter().map(|s| s.as_str()).collect();
            lines.push(format!(
                "Pruned obsolete remote {word}: {}",
                names.join(", ")
            ));
        }
        lines.join("\n")
    }
}

/// The pruned ref of a fetch line (` - [deleted]  (none) -> origin/old`;
/// older git versions flag it with `x`).
fn pruned_ref(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix(['x', '-'])?.trim_start();
    let rest = rest.strip_prefix("[deleted]")?;
    let (_, name) = rest.split_once("->")?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// What a finished Update Project did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateResult {
    /// Nothing came in ("All files are up to date").
    UpToDate,
    /// The incoming commits are in the branch.
    Updated {
        commits: u32,
        files: u32,
        /// The stashed local changes came back with conflicts (they stay
        /// in the stash too).
        restore_conflicts: bool,
    },
    /// Conflicts stopped the merge or rebase: the repository stays as it
    /// is, the local changes (if any) in the stash.
    Incomplete { method: UpdateMethod, stashed: bool },
    /// The update could not start (a merge in progress, a detached `HEAD`,
    /// no tracked branch).
    NotReady { title: String, message: String },
    /// git failed (the fetch, the merge or rebase).
    Failed { title: String, message: String },
}

impl UpdateResult {
    /// Something came in, or nothing had to.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::UpToDate | Self::Updated { .. })
    }
}

/// `n files` / `1 file`.
pub fn files_text(n: u32) -> String {
    if n == 1 {
        "1 file".to_owned()
    } else {
        format!("{n} files")
    }
}

/// `n commits` / `1 commit`.
pub fn commits_text(n: u32) -> String {
    if n == 1 {
        "1 commit".to_owned()
    } else {
        format!("{n} commits")
    }
}

/// The status of a `--name-status` letter.
fn name_status(letter: &str) -> FileStatus {
    match letter.chars().next() {
        Some('A') | Some('C') => FileStatus::Added,
        Some('D') => FileStatus::Deleted,
        Some('R') => FileStatus::Renamed,
        Some('U') => FileStatus::Conflict,
        _ => FileStatus::Modified,
    }
}

/// Parses `--name-status -z` output.
fn parse_name_status(out: &[u8]) -> Vec<CommitFile> {
    let text = String::from_utf8_lossy(out);
    let mut parts = text.split('\0').filter(|p| !p.is_empty());
    let mut files = Vec::new();
    while let Some(letter) = parts.next() {
        let status = name_status(letter);
        let renamed = letter.starts_with('R') || letter.starts_with('C');
        let Some(first) = parts.next() else {
            break;
        };
        if renamed {
            let Some(second) = parts.next() else {
                break;
            };
            files.push(CommitFile {
                path: second.to_owned(),
                status,
                old_path: Some(first.to_owned()),
            });
        } else {
            files.push(CommitFile {
                path: first.to_owned(),
                status,
                old_path: None,
            });
        }
    }
    files
}

/// The changes of several commits as one (oldest first), the way the
/// reference's changes browser zips them: added then modified is added,
/// added then deleted is gone.
pub fn zip_changes(per_commit: Vec<Vec<CommitFile>>) -> Vec<CommitFile> {
    let mut order: Vec<String> = Vec::new();
    let mut by_path: HashMap<String, CommitFile> = HashMap::new();
    for files in per_commit {
        for f in files {
            let key = f.path.clone();
            match by_path.get(&key).map(|e| e.status) {
                Some(FileStatus::Added) if f.status == FileStatus::Deleted => {
                    by_path.remove(&key);
                    order.retain(|p| *p != key);
                }
                Some(FileStatus::Added) => {}
                Some(_) => {
                    by_path.insert(key, f);
                }
                None => {
                    order.push(key.clone());
                    by_path.insert(key, f);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|p| by_path.remove(&p))
        .collect()
}

/// The stash message of the changes put aside by an update
/// (`stash.changes.message`).
const UPDATE_STASH_MESSAGE: &str = "Uncommitted changes before Update";

impl GitRepo {
    fn run_root(&self, args: &[&str], access: Access, cancel: &Cancel) -> Result<Output, VcsError> {
        self.runner()
            .run(&self.repository().root, args, access, cancel)
    }

    /// The remote branch a local branch tracks: the remote and the branch's
    /// name there (`branch.<b>.remote`, `branch.<b>.merge`); `None` for no
    /// upstream or a local one.
    pub fn upstream_of(&self, branch: &str) -> Option<(String, String)> {
        let config = self.repository().config();
        let remote = config.get("branch", Some(branch), "remote")?;
        let merge = config.get("branch", Some(branch), "merge")?;
        if remote == "." || remote.is_empty() {
            return None;
        }
        let name = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
        Some((remote, name.to_owned()))
    }

    /// The names of the remotes.
    pub fn remote_names(&self) -> Vec<String> {
        let config = self.repository().config();
        self.repository()
            .remotes(&config)
            .into_iter()
            .map(|r| r.name)
            .collect()
    }

    /// `origin`, else the only remote (`getDefaultOrFirstRemote`).
    pub fn default_remote(&self) -> Option<String> {
        let names = self.remote_names();
        if names.iter().any(|n| n == "origin") {
            return Some("origin".to_owned());
        }
        names.into_iter().next()
    }

    /// A branch of a remote as a push target: new when the remote has no
    /// such branch.
    pub fn push_target(
        &self,
        remote: &str,
        branch: &str,
        cancel: &Cancel,
    ) -> Result<PushTarget, VcsError> {
        let full = format!("refs/remotes/{remote}/{branch}");
        let exists = self
            .run_root(
                &["rev-parse", "--verify", "--quiet", full.as_str()],
                Access::Read,
                cancel,
            )?
            .success();
        Ok(PushTarget {
            remote: remote.to_owned(),
            branch: branch.to_owned(),
            new: !exists,
        })
    }

    /// Where a branch is pushed by default (`GitPushSupport.
    /// getDefaultTarget`): its upstream, else the branch of its name on
    /// `origin` (or the only remote); `None` without remotes.
    pub fn default_push_target(
        &self,
        source: &str,
        cancel: &Cancel,
    ) -> Result<Option<PushTarget>, VcsError> {
        if let Some((remote, branch)) = self.upstream_of(source)
            && self.remote_names().contains(&remote)
        {
            return self.push_target(&remote, &branch, cancel).map(Some);
        }
        match self.default_remote() {
            Some(remote) => self.push_target(&remote, source, cancel).map(Some),
            None => Ok(None),
        }
    }

    /// The commits a push of `source` to `target` sends, newest first
    /// (`GitOutgoingCommitsProvider`): `target..source`, or for a new
    /// branch those on no branch of the target's remote (at most 1000).
    pub fn outgoing(
        &self,
        source: &str,
        target: &PushTarget,
        cancel: &Cancel,
    ) -> Result<Vec<OutgoingCommit>, VcsError> {
        let format = "--format=%H%x1f%an%x1f%at%x1f%s%x1e";
        let src = format!("refs/heads/{source}");
        let out = if target.new {
            let remotes = format!("--remotes={}", target.remote);
            self.read(
                [
                    "log",
                    format,
                    "--max-count=1000",
                    src.as_str(),
                    "--not",
                    remotes.as_str(),
                    "--",
                ],
                cancel,
            )?
        } else {
            let range = format!("refs/remotes/{}/{}..{src}", target.remote, target.branch);
            self.read(["log", format, range.as_str(), "--"], cancel)?
        };
        let text = out.stdout_text();
        Ok(text
            .split('\x1e')
            .filter_map(|record| {
                let record = record.trim_matches(['\n', '\r']);
                let mut f = record.split('\x1f');
                let oid = f.next()?.trim();
                if oid.is_empty() {
                    return None;
                }
                Some(OutgoingCommit {
                    oid: oid.to_owned(),
                    author: f.next().unwrap_or_default().to_owned(),
                    time: f.next().and_then(|t| t.parse().ok()).unwrap_or_default(),
                    subject: f.next().unwrap_or_default().to_owned(),
                })
            })
            .collect())
    }

    /// The files changed by `commits` (any order), zipped as one change.
    pub fn commit_files(
        &self,
        commits: &[String],
        cancel: &Cancel,
    ) -> Result<Vec<CommitFile>, VcsError> {
        if commits.is_empty() {
            return Ok(Vec::new());
        }
        // Oldest first: `rev-list --no-walk` with the topological order.
        let mut args: Vec<&str> = vec!["rev-list", "--no-walk=sorted", "--reverse"];
        args.extend(commits.iter().map(String::as_str));
        let ordered = self.read(&args, cancel)?.stdout_text();
        let mut per_commit = Vec::new();
        for oid in ordered.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let out = self.read(
                [
                    "diff-tree",
                    "--no-commit-id",
                    "-r",
                    "-z",
                    "--name-status",
                    "-M",
                    "--root",
                    oid,
                ],
                cancel,
            )?;
            per_commit.push(parse_name_status(&out.stdout));
        }
        Ok(zip_changes(per_commit))
    }

    /// Pushes as `spec` says. Errors of git (no such remote, an
    /// authentication failure) are a [`PushOutcome::Error`]; only a
    /// cancellation or a git that cannot run is an `Err`.
    pub fn push(&self, spec: &PushSpec, cancel: &Cancel) -> Result<PushResult, VcsError> {
        let target = &spec.target;
        let refspec = format!("refs/heads/{}:refs/heads/{}", spec.source, target.branch);
        let mut args: Vec<String> = vec![
            "push".into(),
            "--progress".into(),
            "--porcelain".into(),
            target.remote.clone(),
            refspec,
        ];
        if spec.set_upstream {
            args.push("--set-upstream".into());
        }
        match spec.force {
            ForceMode::None => {}
            ForceMode::WithLease => {
                let tracking = format!("refs/remotes/{}/{}", target.remote, target.branch);
                let expected = self.resolve_commit(&tracking, cancel)?.unwrap_or_default();
                args.push(format!("--force-with-lease={}:{expected}", target.branch));
            }
            ForceMode::Force => args.push("--force".into()),
        }
        match spec.tags {
            Some(TagMode::All) => args.push("--tags".into()),
            Some(TagMode::CurrentBranch) => args.push("--follow-tags".into()),
            None => {}
        }
        let out = self.write(&args, cancel)?;
        let native = parse_push(&out.stdout_text());
        let branch_ref = format!("refs/heads/{}", spec.source);
        let result = |outcome, tags| PushResult {
            source: spec.source.clone(),
            target: target.clone(),
            outcome,
            tags,
        };
        let Some(main) = native.iter().find(|n| n.from == branch_ref) else {
            let message = remote_error(&out);
            let message = if message.is_empty() {
                "Push failed".to_owned()
            } else {
                message
            };
            return Ok(result(PushOutcome::Error { message }, Vec::new()));
        };
        let tags: Vec<String> = native
            .iter()
            .filter(|n| n.from != branch_ref && matches!(n.flag, '*' | '+'))
            .map(|n| {
                n.from
                    .strip_prefix("refs/tags/")
                    .unwrap_or(&n.from)
                    .to_owned()
            })
            .collect();
        let outcome = match main.flag {
            ' ' => {
                let commits = match main.range() {
                    Some(range) => self
                        .read(["rev-list", "--count", range], cancel)
                        .ok()
                        .and_then(|o| o.stdout_text().trim().parse().ok())
                        .unwrap_or(0),
                    None => 0,
                };
                PushOutcome::Success { commits }
            }
            '+' => PushOutcome::Forced,
            '*' => PushOutcome::NewBranch,
            '=' => PushOutcome::UpToDate,
            '!' if main.reason_has("non-fast-forward") || main.reason_has("fetch first") => {
                PushOutcome::RejectedNoFastForward
            }
            '!' if main.reason_has("stale info") => PushOutcome::RejectedStaleInfo,
            _ => PushOutcome::RejectedOther {
                reason: main.reason.clone().unwrap_or_default(),
            },
        };
        Ok(result(outcome, tags))
    }

    /// Fetches one remote with `--prune`, or `refspec` of it
    /// (`origin main:main` updates a local branch that is not checked
    /// out, fast-forward only).
    pub fn fetch(
        &self,
        remote: &str,
        refspec: Option<&str>,
        cancel: &Cancel,
    ) -> Result<RemoteFetch, VcsError> {
        let mut args = vec!["fetch", remote];
        args.extend(refspec);
        args.extend(["--recurse-submodules=no", "--progress", "--prune"]);
        let out = self.write(&args, cancel)?;
        let pruned = out
            .stderr
            .lines()
            .chain(out.stdout_text().lines())
            .filter_map(pruned_ref)
            .collect();
        Ok(RemoteFetch {
            remote: remote.to_owned(),
            error: (!out.success()).then(|| remote_error(&out)),
            pruned,
        })
    }

    /// Fetches every remote (Fetch of the branches popup).
    pub fn fetch_all(&self, cancel: &Cancel) -> Result<FetchResult, VcsError> {
        let mut result = FetchResult::default();
        for remote in self.remote_names() {
            result.remotes.push(self.fetch(&remote, None, cancel)?);
        }
        Ok(result)
    }

    /// The local changes of tracked files (staged or not).
    fn has_tracked_changes(&self, cancel: &Cancel) -> Result<bool, VcsError> {
        let out = self.read(["status", "--porcelain", "--untracked-files=no"], cancel)?;
        Ok(!out.stdout.is_empty())
    }

    /// Files with conflicts.
    fn has_unmerged_files(&self, cancel: &Cancel) -> Result<bool, VcsError> {
        let out = self.read(["diff", "--name-only", "--diff-filter=U"], cancel)?;
        Ok(!out.stdout_text().trim().is_empty())
    }

    /// Locally changed files that the incoming commits change too, or
    /// staged changes (`GitMergeUpdater.isSaveNeeded`).
    fn merge_needs_save(&self, upstream: &str, cancel: &Cancel) -> Result<bool, VcsError> {
        let staged = self.read(["diff", "--cached", "--name-only"], cancel)?;
        if !staged.stdout_text().trim().is_empty() {
            return Ok(true);
        }
        let local = self.read(["diff", "--name-only"], cancel)?.stdout_text();
        if local.trim().is_empty() {
            return Ok(false);
        }
        let range = format!("HEAD..{upstream}");
        let incoming = self
            .read(["diff", "--name-only", range.as_str()], cancel)?
            .stdout_text();
        let incoming: std::collections::HashSet<&str> = incoming.lines().collect();
        Ok(local.lines().any(|l| incoming.contains(l)))
    }

    /// Update Project of the current branch (`GitUpdateProcess`): checks
    /// that an update can start, fetches the remote of `target` (the
    /// tracked branch when `None`), and when commits came in merges or
    /// rebases them with the local changes stashed around it.
    pub fn update(
        &self,
        method: UpdateMethod,
        target: Option<(&str, &str)>,
        cancel: &Cancel,
    ) -> Result<UpdateResult, VcsError> {
        let not_ready = |title: &str, message: &str| {
            Ok(UpdateResult::NotReady {
                title: title.to_owned(),
                message: message.to_owned(),
            })
        };
        let (state, _) = self.repository().read_state();
        match state {
            vcs::RepoState::Rebasing => {
                return not_ready(
                    "Cannot update",
                    "You have unfinished rebase process. These conflicts must be resolved before update.",
                );
            }
            vcs::RepoState::Merging => {
                return not_ready(
                    "Cannot update",
                    "You have unfinished merge. These conflicts must be resolved before update.",
                );
            }
            _ => {}
        }
        if self.has_unmerged_files(cancel)? {
            return not_ready(
                "Cannot update",
                "Unmerged files detected. These conflicts must be resolved before update.",
            );
        }
        let info = vcs::Backend::info(self)?;
        let Some(branch) = info.head.branch().map(str::to_owned) else {
            return not_ready(
                "Cannot Update: No Current Branch",
                "You are in the 'detached HEAD' state, which means that you're not on any branch\n\
                 Checkout a branch to be able to update the project.",
            );
        };
        let no_tracked = format!("{branch} has no tracked branch");
        let (remote, remote_branch) = match target {
            Some((r, b)) => (r.to_owned(), b.to_owned()),
            None => match self.upstream_of(&branch) {
                Some(t) => t,
                None => return not_ready("Cannot update", &no_tracked),
            },
        };
        let fetched = self.fetch(&remote, None, cancel)?;
        if let Some(e) = fetched.error {
            return Ok(UpdateResult::Failed {
                title: "Update failed".to_owned(),
                message: e,
            });
        }
        let upstream = format!("refs/remotes/{remote}/{remote_branch}");
        if self.resolve_commit(&upstream, cancel)?.is_none() {
            return not_ready("Cannot update", &no_tracked);
        }
        let range = format!("HEAD..{upstream}");
        let incoming = self.read(["rev-list", "-1", range.as_str()], cancel)?;
        if incoming.stdout_text().trim().is_empty() {
            return Ok(UpdateResult::UpToDate);
        }
        // The commits that came in are counted from where the branches
        // met (`GitUpdatedRanges`): the merge commit is not one of them.
        let before = self.merge_base("HEAD", &upstream, cancel)?;
        let changed = self.has_tracked_changes(cancel)?;
        // A rebase with local changes first tries a fast-forward, which
        // needs no stash.
        if method == UpdateMethod::Rebase && changed {
            let ff = self.write(["merge", "--ff-only", upstream.as_str()], cancel)?;
            if ff.success() {
                return self.updated(before, &upstream, false, cancel);
            }
        }
        let save = match method {
            UpdateMethod::Merge => self.merge_needs_save(&upstream, cancel)?,
            UpdateMethod::Rebase => changed,
        };
        let mut stashed = false;
        if save {
            let top = self.resolve_commit("refs/stash", cancel)?;
            let out = self.write(["stash", "push", "-m", UPDATE_STASH_MESSAGE], cancel)?;
            if !out.success() {
                return Ok(UpdateResult::Failed {
                    title: "Could not save uncommitted changes on update".to_owned(),
                    message: format!(
                        "Could not stash local uncommitted changes:\n{}",
                        error_text(&out)
                    ),
                });
            }
            stashed = self.resolve_commit("refs/stash", cancel)? != top;
        }
        let out = match method {
            UpdateMethod::Merge => self.write(
                ["merge", "--no-stat", "-v", "--no-edit", upstream.as_str()],
                cancel,
            )?,
            UpdateMethod::Rebase => self.write(["rebase", upstream.as_str()], cancel)?,
        };
        if !out.success() {
            let text = format!("{}\n{}", out.stdout_text(), out.stderr);
            let (state, _) = self.repository().read_state();
            let conflicts = match method {
                UpdateMethod::Merge => {
                    text.contains("Automatic merge failed") || state == vcs::RepoState::Merging
                }
                UpdateMethod::Rebase => state == vcs::RepoState::Rebasing,
            };
            if conflicts {
                return Ok(UpdateResult::Incomplete { method, stashed });
            }
            // Nothing changed: the local changes come back.
            if stashed {
                let _ = self.write(["stash", "pop", "--index"], cancel);
            }
            let (title, message) = match method {
                UpdateMethod::Merge => ("Merge Error", error_text(&out)),
                UpdateMethod::Rebase => (
                    "Rebase Error",
                    format!("Could not proceed with rebase. {}", error_text(&out)),
                ),
            };
            return Ok(UpdateResult::Failed {
                title: title.to_owned(),
                message,
            });
        }
        self.updated(before, &upstream, stashed, cancel)
    }

    /// Where `a` and `b` met.
    fn merge_base(&self, a: &str, b: &str, cancel: &Cancel) -> Result<Option<String>, VcsError> {
        let out = self.read(["merge-base", a, b], cancel)?;
        let oid = out.stdout_text().trim().to_owned();
        Ok((out.success() && !oid.is_empty()).then_some(oid))
    }

    /// The counts of an update that went through, after the local changes
    /// came back: the commits and files between where the branch met
    /// `upstream` before and where it meets it now.
    fn updated(
        &self,
        before: Option<String>,
        upstream: &str,
        stashed: bool,
        cancel: &Cancel,
    ) -> Result<UpdateResult, VcsError> {
        let mut restore_conflicts = false;
        if stashed {
            let out = self.write(["stash", "pop", "--index"], cancel)?;
            if !out.success() {
                // `--index` fails when the index cannot be restored; the
                // work tree's changes are what matters.
                let (state, _) = self.repository().read_state();
                let conflicts = self.has_unmerged_files(cancel)?;
                if !conflicts && state == vcs::RepoState::Normal {
                    let out = self.write(["stash", "pop"], cancel)?;
                    restore_conflicts = !out.success();
                } else {
                    restore_conflicts = true;
                }
            }
        }
        let after = self.merge_base("HEAD", upstream, cancel)?;
        let (commits, files) = match (&before, &after) {
            (Some(b), Some(a)) if b != a => {
                let range = format!("{b}..{a}");
                let commits = self
                    .read(["rev-list", "--count", range.as_str()], cancel)?
                    .stdout_text()
                    .trim()
                    .parse()
                    .unwrap_or(0);
                let files = self
                    .read(["diff", "--name-only", b.as_str(), a.as_str()], cancel)?
                    .stdout_text()
                    .lines()
                    .filter(|l| !l.is_empty())
                    .count() as u32;
                (commits, files)
            }
            _ => (0, 0),
        };
        Ok(UpdateResult::Updated {
            commits,
            files,
            restore_conflicts,
        })
    }

    /// Update of a local branch that is not checked out (the branch's
    /// Update in the branches popup): its upstream fetched into it,
    /// fast-forward only (`git fetch origin main:main`).
    pub fn update_branch(&self, branch: &str, cancel: &Cancel) -> Result<RemoteFetch, VcsError> {
        let Some((remote, remote_branch)) = self.upstream_of(branch) else {
            return Ok(RemoteFetch {
                remote: String::new(),
                error: Some(format!("Tracked branch is not configured for {branch}")),
                pruned: Vec::new(),
            });
        };
        let refspec = format!("{remote_branch}:{branch}");
        self.fetch(&remote, Some(&refspec), cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_push_lines_are_parsed() {
        let out = concat!(
            "To ../remote.git\n",
            " \trefs/heads/main:refs/heads/main\t1a2b3c4..5d6e7f8\n",
            "*\trefs/tags/v1:refs/tags/v1\t[new tag]\n",
            "!\trefs/heads/x:refs/heads/x\t[rejected] (fetch first)\n",
            "=\trefs/heads/y:refs/heads/y\t[up to date]\n",
            "+\trefs/heads/z:refs/heads/z\t1a2b3c4...5d6e7f8 (forced update)\n",
            "-\t:refs/heads/gone\t[deleted]\n",
            "Done\n",
        );
        let r = parse_push(out);
        assert_eq!(r.len(), 5);
        assert_eq!(r[0].flag, ' ');
        assert_eq!(r[0].from, "refs/heads/main");
        assert_eq!(r[0].range(), Some("1a2b3c4..5d6e7f8"));
        assert_eq!(r[1].flag, '*');
        assert_eq!(r[1].summary, "[new tag]");
        assert_eq!(r[2].reason.as_deref(), Some("fetch first"));
        assert!(r[2].reason_has("fetch first"));
        assert_eq!(r[3].flag, '=');
        assert_eq!(r[3].range(), None);
        assert_eq!(r[4].range(), Some("1a2b3c4...5d6e7f8"));
        assert_eq!(r[4].reason.as_deref(), Some("forced update"));
    }

    #[test]
    fn credentials_never_show() {
        assert_eq!(
            redact_credentials(
                "fatal: Authentication failed for 'https://me:s3cret@example.com/r.git/'"
            ),
            "fatal: Authentication failed for 'https://example.com/r.git/'"
        );
        assert_eq!(
            redact_credentials("fatal: unable to access 'https://tok@host/x': 403"),
            "fatal: unable to access 'https://host/x': 403"
        );
        assert_eq!(
            redact_credentials("ssh://git@host:22/x and https://plain/y"),
            "ssh://host:22/x and https://plain/y"
        );
        assert_eq!(redact_credentials("no urls"), "no urls");
    }

    #[test]
    fn pruned_refs_and_fetch_messages() {
        assert_eq!(
            pruned_ref(" x [deleted]         (none)     -> origin/old"),
            Some("origin/old".to_owned())
        );
        assert_eq!(
            pruned_ref(" - [deleted]         (none)     -> origin/gone"),
            Some("origin/gone".to_owned())
        );
        assert_eq!(pruned_ref(" * [new branch] main -> origin/main"), None);
        let one = FetchResult {
            remotes: vec![RemoteFetch {
                remote: "origin".into(),
                error: None,
                pruned: vec!["origin/old".into()],
            }],
        };
        assert!(one.is_success());
        assert_eq!(
            one.message(),
            "Pruned obsolete remote reference: origin/old"
        );
        let two = FetchResult {
            remotes: vec![
                RemoteFetch {
                    remote: "origin".into(),
                    error: Some("fatal: no".into()),
                    pruned: Vec::new(),
                },
                RemoteFetch {
                    remote: "up".into(),
                    error: None,
                    pruned: Vec::new(),
                },
            ],
        };
        assert!(!two.is_success());
        assert_eq!(two.message(), "origin: fatal: no");
    }

    #[test]
    fn changes_of_several_commits_are_zipped() {
        let f = |p: &str, s| CommitFile {
            path: p.into(),
            status: s,
            old_path: None,
        };
        let zipped = zip_changes(vec![
            vec![f("a", FileStatus::Added), f("b", FileStatus::Modified)],
            vec![
                f("a", FileStatus::Modified),
                f("b", FileStatus::Deleted),
                f("c", FileStatus::Added),
            ],
            vec![f("c", FileStatus::Deleted)],
        ]);
        assert_eq!(
            zipped,
            vec![f("a", FileStatus::Added), f("b", FileStatus::Deleted)]
        );
        let parsed = parse_name_status(b"M\0x.py\0R100\0old.md\0new.md\0A\0d/e.py\0");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1].status, FileStatus::Renamed);
        assert_eq!(parsed[1].old_path.as_deref(), Some("old.md"));
        assert_eq!(parsed[2].path, "d/e.py");
        assert_eq!(files_text(1), "1 file");
        assert_eq!(commits_text(2), "2 commits");
    }
}
