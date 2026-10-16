//! Branch operations of the branches popup and the VCS widget (the
//! reference's `GitBranchWorker` and its operations): checkout of a local
//! branch, of a remote branch as a new tracking branch, of a tag or a
//! revision; checkout with the local changes stashed around it (Smart
//! Checkout) or overwritten (Force Checkout); new branches, rename,
//! delete and restore; the recently checked out branches from the reflog;
//! `git init`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use vcs::{Cancel, VcsError};

use crate::backend::GitRepo;
use crate::exe::GitExe;
use crate::runner::{Access, Output, Runner, error_text};

/// How a checkout treats local changes it would overwrite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutMode {
    /// Stops with [`CheckoutError::LocalChanges`].
    #[default]
    Normal,
    /// Throws them away (`git checkout --force`).
    Force,
    /// Stashes them, checks out and puts them back (`Smart Checkout`).
    Smart,
}

/// Why a checkout did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutError {
    /// Local changes to these files (relative to the work tree's root, `/`
    /// separators) would be overwritten.
    LocalChanges(Vec<String>),
    /// Untracked files are in the way.
    Untracked(Vec<String>),
    /// No branch, tag or commit has the name.
    NotFound(String),
    Failed(VcsError),
}

impl std::fmt::Display for CheckoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalChanges(files) => write!(
                f,
                "Your local changes to the following files would be overwritten by checkout: {}",
                files.join(", ")
            ),
            Self::Untracked(files) => write!(
                f,
                "The following untracked working tree files would be overwritten by checkout: {}",
                files.join(", ")
            ),
            Self::NotFound(_) => f.write_str("Revision not found"),
            Self::Failed(e) => e.fmt(f),
        }
    }
}

impl From<VcsError> for CheckoutError {
    fn from(e: VcsError) -> Self {
        Self::Failed(e)
    }
}

/// What was checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckedOut {
    /// A local branch.
    Branch { name: String },
    /// A new local branch tracking a remote one (`origin/feat` as `feat`).
    NewBranch { name: String, from: String },
    /// A tag or a commit: `HEAD` is detached.
    Detached { target: String },
}

/// A checkout and what became of the local changes stashed around it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Checkout {
    pub checked_out: CheckedOut,
    /// Smart Checkout put the changes back with conflicts; they stay in the
    /// stash.
    pub conflicts_on_restore: bool,
}

/// Paths a failed checkout lists after `header`: tab-indented lines.
fn listed_files(text: &str, header: &str) -> Option<Vec<String>> {
    let start = text.find(header)?;
    let files = text[start..]
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with('\t'))
        .map(|l| l.trim().to_owned())
        .collect();
    Some(files)
}

/// The failure of a checkout from git's message.
fn checkout_error(out: &Output) -> CheckoutError {
    let text = error_text(out);
    if let Some(files) = listed_files(
        &text,
        "Your local changes to the following files would be overwritten",
    ) {
        return CheckoutError::LocalChanges(files);
    }
    if let Some(files) = listed_files(&text, "untracked working tree files would be overwritten") {
        return CheckoutError::Untracked(files);
    }
    CheckoutError::Failed(VcsError::failed(text))
}

impl GitRepo {
    /// The commit `rev` names, `None` when it names none.
    pub fn resolve_commit(&self, rev: &str, cancel: &Cancel) -> Result<Option<String>, VcsError> {
        let spec = format!("{rev}^{{commit}}");
        let out = self.runner().run(
            &self.repository().root,
            ["rev-parse", "--verify", "--quiet", spec.as_str()],
            Access::Read,
            cancel,
        )?;
        let oid = out.stdout_text().trim().to_owned();
        Ok((out.success() && !oid.is_empty()).then_some(oid))
    }

    fn has_ref(&self, full: &str, cancel: &Cancel) -> Result<bool, VcsError> {
        let out = self.runner().run(
            &self.repository().root,
            ["rev-parse", "--verify", "--quiet", full],
            Access::Read,
            cancel,
        )?;
        Ok(out.success())
    }

    /// The local name of a remote branch: without its remote's name
    /// (`origin/feat/x` is `feat/x`), `None` when no remote prefixes it.
    fn local_name_of_remote(&self, remote_branch: &str) -> Option<String> {
        let config = self.repository().config();
        self.repository()
            .remotes(&config)
            .into_iter()
            .filter_map(|r| {
                remote_branch
                    .strip_prefix(&r.name)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .map(|rest| (r.name.len(), rest.to_owned()))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, name)| name)
    }

    /// Checks out `target`: a local branch; a remote branch as the local
    /// branch of its name (created to track it when there is none); else a
    /// tag or a commit, detaching `HEAD`.
    pub fn checkout(
        &self,
        target: &str,
        mode: CheckoutMode,
        cancel: &Cancel,
    ) -> Result<Checkout, CheckoutError> {
        let (args, checked_out) = self.checkout_plan(target, cancel)?;
        match mode {
            CheckoutMode::Normal | CheckoutMode::Force => {
                let mut args = args;
                if mode == CheckoutMode::Force {
                    args.insert(1, "--force".to_owned());
                }
                let out = self.write(&args, cancel)?;
                if !out.success() {
                    return Err(checkout_error(&out));
                }
                Ok(Checkout {
                    checked_out,
                    conflicts_on_restore: false,
                })
            }
            CheckoutMode::Smart => self.smart_checkout(target, args, checked_out, cancel),
        }
    }

    /// The `git checkout` arguments for `target` and what it checks out.
    fn checkout_plan(
        &self,
        target: &str,
        cancel: &Cancel,
    ) -> Result<(Vec<String>, CheckedOut), CheckoutError> {
        let own = |args: &[&str]| args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        if self.has_ref(&format!("refs/heads/{target}"), cancel)? {
            return Ok((
                own(&["checkout", target, "--"]),
                CheckedOut::Branch {
                    name: target.to_owned(),
                },
            ));
        }
        if self.has_ref(&format!("refs/remotes/{target}"), cancel)?
            && let Some(local) = self.local_name_of_remote(target)
        {
            if self.has_ref(&format!("refs/heads/{local}"), cancel)? {
                return Ok((
                    own(&["checkout", &local, "--"]),
                    CheckedOut::Branch { name: local },
                ));
            }
            return Ok((
                own(&["checkout", "-b", &local, "--track", target, "--"]),
                CheckedOut::NewBranch {
                    name: local,
                    from: target.to_owned(),
                },
            ));
        }
        if self.resolve_commit(target, cancel)?.is_none() {
            return Err(CheckoutError::NotFound(target.to_owned()));
        }
        Ok((
            own(&["checkout", "--detach", target, "--"]),
            CheckedOut::Detached {
                target: target.to_owned(),
            },
        ))
    }

    /// The stash's top commit, `None` without a stash.
    fn stash_top(&self, cancel: &Cancel) -> Result<Option<String>, VcsError> {
        self.resolve_commit("refs/stash", cancel)
    }

    fn smart_checkout(
        &self,
        target: &str,
        args: Vec<String>,
        checked_out: CheckedOut,
        cancel: &Cancel,
    ) -> Result<Checkout, CheckoutError> {
        let before = self.stash_top(cancel)?;
        let message = format!("Uncommitted changes before Checkout {target}");
        let out = self.write(["stash", "push", "-m", message.as_str()], cancel)?;
        if !out.success() {
            return Err(CheckoutError::Failed(VcsError::failed(error_text(&out))));
        }
        let stashed = self.stash_top(cancel)? != before;
        let out = self.write(&args, cancel)?;
        if !out.success() {
            // Put the changes back where they were.
            if stashed {
                let _ = self.write(["stash", "pop", "--index"], cancel);
            }
            return Err(checkout_error(&out));
        }
        let mut conflicts_on_restore = false;
        if stashed {
            let out = self.write(["stash", "pop"], cancel)?;
            conflicts_on_restore = !out.success();
        }
        Ok(Checkout {
            checked_out,
            conflicts_on_restore,
        })
    }

    /// Creates the branch `name` at `start` (`HEAD` when `None`) and checks
    /// it out when `checkout` is set; `overwrite` resets an existing branch
    /// of the name.
    pub fn new_branch(
        &self,
        name: &str,
        start: Option<&str>,
        checkout: bool,
        overwrite: bool,
        cancel: &Cancel,
    ) -> Result<(), CheckoutError> {
        let mut args: Vec<&str> = if checkout {
            vec!["checkout", if overwrite { "-B" } else { "-b" }, name]
        } else if overwrite {
            vec!["branch", "--force", name]
        } else {
            vec!["branch", name]
        };
        args.extend(start);
        let out = self.write(&args, cancel)?;
        if out.success() {
            Ok(())
        } else if checkout {
            Err(checkout_error(&out))
        } else {
            Err(CheckoutError::Failed(VcsError::failed(error_text(&out))))
        }
    }

    /// Renames the local branch `from` to `to`.
    pub fn rename_branch(&self, from: &str, to: &str, cancel: &Cancel) -> Result<(), VcsError> {
        let out = self.write(["branch", "-m", from, to], cancel)?;
        if out.success() {
            Ok(())
        } else {
            Err(VcsError::failed(error_text(&out)))
        }
    }

    /// Deletes the local branch `name`, merged or not, and returns the
    /// commit it pointed at (to restore it).
    pub fn delete_branch(&self, name: &str, cancel: &Cancel) -> Result<String, VcsError> {
        let oid = self
            .resolve_commit(&format!("refs/heads/{name}"), cancel)?
            .ok_or_else(|| VcsError::failed(format!("Branch {name} not found")))?;
        let out = self.write(["branch", "-D", name], cancel)?;
        if out.success() {
            Ok(oid)
        } else {
            Err(VcsError::failed(error_text(&out)))
        }
    }

    /// Creates the branch `name` at `oid` again (Restore after a delete).
    pub fn restore_branch(&self, name: &str, oid: &str, cancel: &Cancel) -> Result<(), VcsError> {
        let out = self.write(["branch", name, oid], cancel)?;
        if out.success() {
            Ok(())
        } else {
            Err(VcsError::failed(error_text(&out)))
        }
    }

    /// Local branches checked out lately, newest first, from the last
    /// `entries` checkouts of the reflog (`GitRecentCheckoutBranches`);
    /// only branches that still exist.
    pub fn recent_checkouts(
        &self,
        entries: usize,
        cancel: &Cancel,
    ) -> Result<Vec<String>, VcsError> {
        let count = entries.to_string();
        let out = self.runner().run(
            &self.repository().root,
            [
                "reflog",
                "--max-count",
                count.as_str(),
                "--grep-reflog=checkout:",
                "--format=%gs",
                "--",
            ],
            Access::Read,
            cancel,
        )?;
        if !out.success() {
            return Ok(Vec::new());
        }
        let mut names: Vec<String> = Vec::new();
        for line in out.stdout_text().lines() {
            let Some(at) = line.rfind(" to ") else {
                continue;
            };
            let name = line[at + " to ".len()..].trim();
            if name.is_empty() || names.iter().any(|n| n == name) {
                continue;
            }
            if self.has_ref(&format!("refs/heads/{name}"), cancel)? {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }
}

/// Creates a repository in `folder` (`git init`).
pub fn init(exe: &GitExe, folder: &Path, cancel: &Cancel) -> Result<(), VcsError> {
    Runner::new(exe.clone())
        .run_ok(folder, ["init"], Access::Write, cancel)
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(stderr: &str) -> Output {
        Output {
            stdout: Vec::new(),
            stderr: stderr.to_owned(),
            code: Some(1),
        }
    }

    #[test]
    fn local_changes_in_the_way_are_listed() {
        let e = checkout_error(&out(
            "error: Your local changes to the following files would be overwritten by checkout:\n\
             \tmain.py\n\tsrc/ünï.py\n\
             Please commit your changes or stash them before you switch branches.\nAborting\n",
        ));
        assert_eq!(
            e,
            CheckoutError::LocalChanges(vec!["main.py".into(), "src/ünï.py".into()])
        );
        let e = checkout_error(&out(
            "error: The following untracked working tree files would be overwritten by checkout:\n\
             \tnew.py\nPlease move or remove them before you switch branches.\nAborting\n",
        ));
        assert_eq!(e, CheckoutError::Untracked(vec!["new.py".into()]));
        let e = checkout_error(&out("fatal: something else"));
        assert_eq!(
            e,
            CheckoutError::Failed(VcsError::failed("fatal: something else"))
        );
    }
}
