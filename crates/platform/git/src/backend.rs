use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use vcs::{Backend, Branch, Cancel, Head, RepoInfo, Snapshot, VcsError};

use crate::exe::GitExe;
use crate::parse;
use crate::repo::Repository;
use crate::runner::{Access, Output, Runner};

/// More dirty paths than this and a status looks at the whole work tree
/// (pathspecs stay far below the command-line limit).
pub const MAX_PATHSPECS: usize = 100;

/// A base text known for a path.
struct BaseEntry {
    /// The `HEAD` commit it was looked up for.
    head: String,
    /// The blob in that commit; `None` when the commit has no such file.
    blob: Option<String>,
    text: Option<Arc<str>>,
}

/// One Git repository through the git executable: the readers for its
/// state, status and branches, base texts cached by blob.
pub struct GitRepo {
    repo: Repository,
    runner: Arc<Runner>,
    /// Statuses are limited to this folder (a project inside a larger
    /// work tree).
    scope: Option<PathBuf>,
    bases: Mutex<HashMap<PathBuf, BaseEntry>>,
    generation: AtomicU64,
}

impl GitRepo {
    pub fn new(repo: Repository, exe: GitExe) -> Self {
        Self {
            repo,
            runner: Arc::new(Runner::new(exe)),
            scope: None,
            bases: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }

    /// Limits statuses to `folder` when it is below the work tree's root.
    pub fn with_scope(mut self, folder: &Path) -> Self {
        if folder != self.repo.root && folder.starts_with(&self.repo.root) {
            self.scope = Some(folder.to_owned());
        }
        self
    }

    pub fn repository(&self) -> &Repository {
        &self.repo
    }

    pub fn runner(&self) -> &Arc<Runner> {
        &self.runner
    }

    pub fn scope(&self) -> Option<&Path> {
        self.scope.as_deref()
    }

    /// Runs a read command in the work tree.
    pub fn read<I, S>(&self, args: I, cancel: &Cancel) -> Result<Output, VcsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString>,
    {
        self.runner
            .run_ok(&self.repo.root, args, Access::Read, cancel)
    }

    /// Runs a write command in the work tree: one at a time, file events
    /// of the repository are its own until it ends, then one refresh
    /// follows.
    pub fn write<I, S>(&self, args: I, cancel: &Cancel) -> Result<Output, VcsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString>,
    {
        self.runner
            .run(&self.repo.root, args, Access::Write, cancel)
    }

    fn head_from_git(&self, cancel: &Cancel) -> Result<Head, VcsError> {
        let symbolic = self.runner.run(
            &self.repo.root,
            ["symbolic-ref", "-q", "HEAD"],
            Access::Read,
            cancel,
        )?;
        let oid = self
            .runner
            .run(
                &self.repo.root,
                ["rev-parse", "-q", "--verify", "HEAD^{commit}"],
                Access::Read,
                cancel,
            )?
            .stdout_text()
            .trim()
            .to_owned();
        let oid = (!oid.is_empty()).then_some(oid);
        Ok(match symbolic.success() {
            true => {
                let full = symbolic.stdout_text().trim().to_owned();
                let name = full.strip_prefix("refs/heads/").unwrap_or(&full).to_owned();
                Head::Branch { name, oid }
            }
            false => Head::Detached {
                oid: oid.unwrap_or_default(),
            },
        })
    }

    fn info_with(&self, cancel: &Cancel) -> Result<RepoInfo, VcsError> {
        let config = self.repo.config();
        let head = match self.repo.read_head() {
            Some(head) if !self.repo.uses_reftable(&config) => head,
            _ => self.head_from_git(cancel)?,
        };
        let (state, rebasing_branch) = self.repo.read_state();
        Ok(RepoInfo {
            root: self.repo.root.clone(),
            git_dir: self.repo.git_dir.clone(),
            head,
            state,
            rebasing_branch,
            tracking: None,
            remotes: self.repo.remotes(&config),
        })
    }

    /// The commit `HEAD` is now, `None` before the first commit.
    fn head_oid(&self, cancel: &Cancel) -> Result<Option<String>, VcsError> {
        Ok(self.info_with(cancel)?.head.oid().map(str::to_owned))
    }
}

impl Backend for GitRepo {
    fn name(&self) -> &'static str {
        "Git"
    }

    fn root(&self) -> &Path {
        &self.repo.root
    }

    fn info(&self) -> Result<RepoInfo, VcsError> {
        self.info_with(&Cancel::new())
    }

    fn status(&self, paths: Option<&[PathBuf]>, cancel: &Cancel) -> Result<Snapshot, VcsError> {
        let root = &self.repo.root;
        let mut args: Vec<String> = vec![
            "--literal-pathspecs".into(),
            "status".into(),
            "--porcelain=v2".into(),
            "-z".into(),
            "--branch".into(),
            "--untracked-files=all".into(),
            "--ignored=matching".into(),
        ];
        let specs: Vec<&Path> = match (paths, &self.scope) {
            (Some(paths), _) => paths.iter().map(PathBuf::as_path).collect(),
            (None, Some(scope)) => vec![scope.as_path()],
            (None, None) => Vec::new(),
        };
        if !specs.is_empty() {
            args.push("--".into());
            args.extend(specs.iter().filter_map(|p| parse::relative(root, p)));
        }
        let out = self.read(&args, cancel)?;
        let (status, headers) = parse::status_v2(root, &out.stdout);
        let mut info = self.info_with(cancel)?;
        info.tracking = headers.tracking();
        Ok(Snapshot {
            info,
            status,
            generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
        })
    }

    fn base_text(&self, path: &Path, cancel: &Cancel) -> Result<Option<Arc<str>>, VcsError> {
        let Some(rel) = parse::relative(&self.repo.root, path) else {
            return Ok(None);
        };
        let Some(head) = self.head_oid(cancel)? else {
            return Ok(None);
        };
        let lock = |m: &Mutex<HashMap<PathBuf, BaseEntry>>| {
            m.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(path)
                .map(|e| (e.head.clone(), e.blob.clone(), e.text.clone()))
        };
        let known = lock(&self.bases);
        if let Some((h, _, text)) = &known
            && *h == head
        {
            return Ok(text.clone());
        }
        let listed = self.read(
            [
                "--literal-pathspecs",
                "ls-tree",
                "-z",
                head.as_str(),
                "--",
                rel.as_str(),
            ],
            cancel,
        )?;
        let blob = parse::ls_tree_blob(&listed.stdout);
        let text = match (&blob, known) {
            (None, _) => None,
            // The same blob in another commit: the text is known.
            (Some(blob), Some((_, Some(old), text))) if *blob == old => text,
            (Some(blob), _) => {
                let path_arg = format!("--path={rel}");
                let out = self.read(["cat-file", "--filters", path_arg.as_str(), blob], cancel)?;
                Some(Arc::from(String::from_utf8_lossy(&out.stdout).as_ref()))
            }
        };
        self.bases
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                path.to_owned(),
                BaseEntry {
                    head,
                    blob,
                    text: text.clone(),
                },
            );
        Ok(text)
    }

    fn branches(&self, cancel: &Cancel) -> Result<Vec<Branch>, VcsError> {
        let out = self.read(
            [
                "for-each-ref",
                parse::REFS_FORMAT,
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ],
            cancel,
        )?;
        Ok(parse::refs(&out.stdout_text()))
    }
}
