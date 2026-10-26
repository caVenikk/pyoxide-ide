//! The operations of the Commit tool window (the reference's
//! `GitCheckinEnvironment`, `GitRollbackEnvironment`, `GitConfigUtil`):
//! commit and amend (`git commit --only` with the included paths, so
//! other staged changes stay staged; with files committed in part, from a
//! temporary index of `HEAD` and the included content), the last commit
//! for Amend, the user's name and e-mail (`Git User Name Is Not Defined`),
//! rollback of changes, adding files and whether the repository has
//! commit hooks.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use vcs::{Cancel, Change, CommitRequest, LastCommit, RollbackKind, VcsError, rollback_kind};

use crate::backend::GitRepo;
use crate::exe::Version;
use crate::parse;
use crate::runner::{Access, Output, error_text};

/// The length of the paths one command line takes; longer lists are
/// split (or, for a commit, read from a file). Far below the limit of
/// Windows (32 767 characters).
const MAX_ARGS_LEN: usize = 16_000;

/// `--pathspec-from-file` came with git 2.25.
const PATHSPEC_FILE: Version = Version(2, 25, 0);

/// The user name and e-mail git commits with, from any of its config
/// files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl Identity {
    /// Both are set (the reference asks for them otherwise).
    pub fn is_complete(&self) -> bool {
        self.name.is_some() && self.email.is_some()
    }
}

/// A commit that was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// The new `HEAD`.
    pub oid: String,
    /// Files committed (a rename counts once).
    pub files: usize,
}

/// A temporary file removed when dropped (the commit message, a pathspec
/// list).
struct TempFile(PathBuf);

impl TempFile {
    fn new(purpose: &str, content: &[u8]) -> Result<Self, VcsError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "{}-{purpose}-{}-{n}.txt",
            app_info::slug(),
            std::process::id()
        ));
        std::fs::write(&path, content)
            .map_err(|e| VcsError::failed(format!("Cannot write {}: {e}", path.display())))?;
        Ok(Self(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A temporary index in the git folder (a partial commit's), removed with
/// its lock when dropped.
struct TempIndex(PathBuf);

impl TempIndex {
    fn new(git_dir: &Path) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(git_dir.join(format!(
            "{}-index-{}-{n}",
            app_info::slug(),
            std::process::id()
        )))
    }

    fn env(&self) -> [(&str, &OsStr); 1] {
        [("GIT_INDEX_FILE", self.0.as_os_str())]
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(lock);
    }
}

/// The paths as git names them, relative to `root`.
fn relative_all(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| parse::relative(root, p))
        .collect()
}

/// Splits paths into groups whose total length stays under the limit.
fn chunks(paths: &[String]) -> Vec<&[String]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut len = 0;
    for (i, p) in paths.iter().enumerate() {
        if len + p.len() + 1 > MAX_ARGS_LEN && i > start {
            out.push(&paths[start..i]);
            start = i;
            len = 0;
        }
        len += p.len() + 1;
    }
    if start < paths.len() {
        out.push(&paths[start..]);
    }
    out
}

/// The text of a failed command: its error output, then its output (a
/// hook prints to either).
fn failure_text(out: &Output) -> String {
    let err = out.stderr.trim();
    let std = out.stdout_text();
    let std = std.trim();
    match (err.is_empty(), std.is_empty()) {
        (false, false) => format!("{std}\n{err}"),
        (false, true) => err.to_owned(),
        _ => std.to_owned(),
    }
}

impl GitRepo {
    /// Runs a write command once per group of paths: `args`, `--`, the
    /// paths.
    fn write_paths(
        &self,
        args: &[&str],
        paths: &[String],
        cancel: &Cancel,
    ) -> Result<(), VcsError> {
        for chunk in chunks(paths) {
            let mut all: Vec<&str> = vec!["--literal-pathspecs"];
            all.extend_from_slice(args);
            all.push("--");
            all.extend(chunk.iter().map(String::as_str));
            let out = self.write(&all, cancel)?;
            if !out.success() {
                return Err(VcsError::failed(error_text(&out)));
            }
        }
        Ok(())
    }

    /// `git config --get <key>`, `None` when it is not set.
    fn config_value(&self, key: &str, cancel: &Cancel) -> Result<Option<String>, VcsError> {
        let out = self.runner().run(
            &self.repository().root,
            ["config", "--get", key],
            Access::Read,
            cancel,
        )?;
        let value = out.stdout_text().trim().to_owned();
        Ok((out.success() && !value.is_empty()).then_some(value))
    }

    /// The user name and e-mail commits get (`user.name`, `user.email`).
    pub fn identity(&self, cancel: &Cancel) -> Result<Identity, VcsError> {
        Ok(Identity {
            name: self.config_value("user.name", cancel)?,
            email: self.config_value("user.email", cancel)?,
        })
    }

    /// Sets `user.name` and `user.email` for the repository, or for every
    /// repository of the user with `global` ("Set properties globally").
    pub fn set_identity(
        &self,
        name: &str,
        email: &str,
        global: bool,
        cancel: &Cancel,
    ) -> Result<(), VcsError> {
        for (key, value) in [("user.name", name), ("user.email", email)] {
            let mut args = vec!["config"];
            if global {
                args.push("--global");
            }
            args.extend([key, value]);
            let out = self.write(&args, cancel)?;
            if !out.success() {
                return Err(VcsError::failed(error_text(&out)));
            }
        }
        Ok(())
    }

    /// Commits the request's paths (`git commit --only`): unversioned
    /// files are added first; other staged changes stay staged. An amend
    /// without paths changes the message (and author) only. Files
    /// committed in part go through a temporary index (`commit --only`
    /// takes the work tree's file): `HEAD`, the whole files and the
    /// included content of the others; the real index then takes the
    /// committed paths from the new `HEAD`, so the changes left out stay
    /// local changes.
    pub fn commit(&self, request: &CommitRequest, cancel: &Cancel) -> Result<Committed, VcsError> {
        let root = &self.repository().root;
        let added = relative_all(root, &request.unversioned);
        if !added.is_empty() {
            self.write_paths(&["add"], &added, cancel)?;
        }
        let paths = relative_all(root, &request.paths());
        if paths.is_empty() && !request.amend {
            return Err(VcsError::failed("Nothing to commit"));
        }
        let message = TempFile::new("commit-message", request.message.as_bytes())?;
        let partial = !request.partial.is_empty();
        let mut args: Vec<String> = vec!["--literal-pathspecs".into(), "commit".into()];
        if !partial {
            args.push("--only".into());
        }
        args.extend(["-F".into(), message.0.display().to_string()]);
        if request.amend {
            args.push("--amend".into());
        }
        if request.sign_off {
            args.push("--signoff".into());
        }
        if !request.run_hooks {
            args.push("--no-verify".into());
        }
        if let Some(author) = request.author.as_deref().filter(|a| !a.trim().is_empty()) {
            args.push(format!("--author={}", author.trim()));
        }
        if partial {
            let index = self.partial_index(request, &paths, cancel)?;
            let out = self
                .runner()
                .run_env(root, &args, &index.env(), Access::Write, cancel)?;
            if !out.success() {
                return Err(VcsError::failed(failure_text(&out)));
            }
            self.write_paths(&["reset", "-q"], &paths, cancel)?;
            return self.committed(request, cancel);
        }
        let total: usize = paths.iter().map(|p| p.len() + 1).sum();
        let _list;
        if total > MAX_ARGS_LEN && self.runner().exe().version >= PATHSPEC_FILE {
            let mut content = Vec::new();
            for p in &paths {
                content.extend_from_slice(p.as_bytes());
                content.push(0);
            }
            let list = TempFile::new("commit-paths", &content)?;
            args.push(format!("--pathspec-from-file={}", list.0.display()));
            args.push("--pathspec-file-nul".into());
            _list = list;
        } else {
            args.push("--".into());
            args.extend(paths);
        }
        let out = self.write(&args, cancel)?;
        if !out.success() {
            return Err(VcsError::failed(failure_text(&out)));
        }
        self.committed(request, cancel)
    }

    fn committed(&self, request: &CommitRequest, cancel: &Cancel) -> Result<Committed, VcsError> {
        let oid = self
            .resolve_commit("HEAD", cancel)?
            .ok_or_else(|| VcsError::failed("HEAD not found after the commit"))?;
        Ok(Committed {
            oid,
            files: request.file_count(),
        })
    }

    /// The index a partial commit commits from: `HEAD`, the whole files
    /// of `paths` as they are in the work tree, and of each file committed
    /// in part its base text with the included changes
    /// (`PartialCommitHelper`, `GitIndexUtil.write`).
    fn partial_index(
        &self,
        request: &CommitRequest,
        paths: &[String],
        cancel: &Cancel,
    ) -> Result<TempIndex, VcsError> {
        let root = &self.repository().root;
        let index = TempIndex::new(&self.repository().git_dir);
        let env = index.env();
        let run = |args: &[&str]| -> Result<Output, VcsError> {
            self.runner()
                .run_env(root, args, &env, Access::Write, cancel)
                .and_then(|out| {
                    if out.success() {
                        Ok(out)
                    } else {
                        Err(VcsError::failed(error_text(&out)))
                    }
                })
        };
        run(&["read-tree", "HEAD"])?;
        let parts: Vec<(String, &vcs::PartialFile)> = request
            .partial
            .iter()
            .filter_map(|p| Some((parse::relative(root, &p.path)?, p)))
            .collect();
        let in_part: HashSet<&str> = parts.iter().map(|(rel, _)| rel.as_str()).collect();
        let whole: Vec<String> = paths
            .iter()
            .filter(|p| !in_part.contains(p.as_str()))
            .cloned()
            .collect();
        for chunk in chunks(&whole) {
            let mut args = vec!["--literal-pathspecs", "add", "-A", "--"];
            args.extend(chunk.iter().map(String::as_str));
            run(&args)?;
        }
        for (rel, part) in &parts {
            // The base: `HEAD`'s file (a renamed file's old one) as the
            // work tree has it.
            let old = request
                .changes
                .iter()
                .find(|c| c.path == part.path)
                .and_then(|c| c.old_path.as_deref())
                .and_then(|p| parse::relative(root, p))
                .unwrap_or_else(|| rel.clone());
            let base = self.read(["cat-file", "--filters", &format!("HEAD:{old}")], cancel)?;
            let text = std::fs::read(&part.path).map_err(|e| {
                VcsError::failed(format!("Cannot read {}: {e}", part.path.display()))
            })?;
            let content = vcs::partial_text(
                &String::from_utf8_lossy(&base.stdout),
                &String::from_utf8_lossy(&text),
                &part.excluded,
            );
            let file = TempFile::new("partial", content.as_bytes())?;
            let path_arg = format!("--path={rel}");
            let file_arg = file.0.display().to_string();
            let blob = run(&["hash-object", "-w", &path_arg, "--", &file_arg])?
                .stdout_text()
                .trim()
                .to_owned();
            let listed = run(&["--literal-pathspecs", "ls-files", "-s", "--", rel])?;
            let mode = listed
                .stdout_text()
                .split_whitespace()
                .next()
                .unwrap_or("100644")
                .to_owned();
            let info = format!("{mode},{blob},{rel}");
            run(&["update-index", "--add", "--cacheinfo", &info])?;
        }
        Ok(index)
    }

    /// The last commit of `HEAD`, `None` before the first one.
    pub fn last_commit(&self, cancel: &Cancel) -> Result<Option<LastCommit>, VcsError> {
        let out = self.runner().run(
            &self.repository().root,
            [
                "log",
                "-1",
                "--format=%H%x00%s%x00%an <%ae>%x00%B",
                "HEAD",
                "--",
            ],
            Access::Read,
            cancel,
        )?;
        if !out.success() {
            return Ok(None);
        }
        let text = out.stdout_text();
        let mut parts = text.splitn(4, '\0');
        let (Some(oid), Some(subject), Some(author), Some(message)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Ok(None);
        };
        Ok(Some(LastCommit {
            oid: oid.trim().to_owned(),
            subject: subject.to_owned(),
            message: message.trim_end().to_owned(),
            author: author.to_owned(),
        }))
    }

    /// Rolls back changes (`GitRollbackEnvironment`): added files leave the
    /// index (and are deleted with `delete_added`), a rename's new file
    /// leaves the index and is deleted, modified and deleted files (and a
    /// rename's old one) get the content of `HEAD` back.
    pub fn rollback(
        &self,
        changes: &[Change],
        delete_added: bool,
        cancel: &Cancel,
    ) -> Result<(), VcsError> {
        let root = &self.repository().root;
        let (mut revert, mut unindex, mut delete) = (Vec::new(), Vec::new(), Vec::new());
        for c in changes {
            match rollback_kind(c.status) {
                RollbackKind::Revert => revert.push(c.path.clone()),
                RollbackKind::Unindex => {
                    unindex.push(c.path.clone());
                    if delete_added {
                        delete.push(c.path.clone());
                    }
                }
                RollbackKind::UndoRename => {
                    revert.extend(c.old_path.clone());
                    unindex.push(c.path.clone());
                    delete.push(c.path.clone());
                }
            }
        }
        let unindex = relative_all(root, &unindex);
        if !unindex.is_empty() {
            self.write_paths(&["rm", "--cached", "-f", "-q"], &unindex, cancel)?;
        }
        for path in &delete {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(VcsError::failed(format!(
                        "Cannot delete file: {}: {e}",
                        path.display()
                    )));
                }
            }
        }
        let revert = relative_all(root, &revert);
        if !revert.is_empty() {
            self.write_paths(&["checkout", "HEAD"], &revert, cancel)?;
        }
        Ok(())
    }

    /// Adds files to the index (Add to VCS).
    pub fn add(&self, paths: &[PathBuf], cancel: &Cancel) -> Result<(), VcsError> {
        let paths = relative_all(&self.repository().root, paths);
        if paths.is_empty() {
            return Ok(());
        }
        self.write_paths(&["add"], &paths, cancel)
    }

    /// The repository has a hook that `--no-verify` skips (`pre-commit`,
    /// `commit-msg`): the commit options offer "Run Git hooks".
    pub fn has_commit_hooks(&self) -> bool {
        let repo = self.repository();
        let config = repo.config();
        let dir = match config.get("core", None, "hookspath") {
            Some(p) => {
                let p = PathBuf::from(p);
                if p.is_absolute() {
                    p
                } else {
                    repo.root.join(p)
                }
            }
            None => repo.common_dir.join("hooks"),
        };
        ["pre-commit", "commit-msg"]
            .iter()
            .any(|name| is_hook(&dir.join(name)))
    }
}

/// A hook git runs: a file, executable where that is a file mode.
fn is_hook(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_path_lists_are_split() {
        let paths: Vec<String> = (0..2000).map(|i| format!("folder/file{i:05}.py")).collect();
        let groups = chunks(&paths);
        assert!(groups.len() > 1);
        assert_eq!(groups.iter().map(|g| g.len()).sum::<usize>(), paths.len());
        for g in &groups {
            assert!(g.iter().map(|p| p.len() + 1).sum::<usize>() <= MAX_ARGS_LEN);
        }
        assert!(chunks(&[]).is_empty());
    }

    #[test]
    fn a_failure_shows_the_output_and_the_errors() {
        let out = Output {
            stdout: b"hook says no\n".to_vec(),
            stderr: "error: failed\n".to_owned(),
            code: Some(1),
        };
        assert_eq!(failure_text(&out), "hook says no\nerror: failed");
    }
}
