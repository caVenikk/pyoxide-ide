use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Change, FileStatus};

/// What a commit takes from the Commit tool window: the included changes
/// and unversioned files, the message and the options of the commit
/// options popup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRequest {
    pub message: String,
    /// Changes against `HEAD` to commit (a rename with its old path).
    pub changes: Vec<Change>,
    /// Unversioned files to add and commit.
    pub unversioned: Vec<PathBuf>,
    /// Replace the last commit (`--amend`).
    pub amend: bool,
    /// Add a `Signed-off-by` line (`--signoff`).
    pub sign_off: bool,
    /// Run the repository's commit hooks (else `--no-verify`).
    pub run_hooks: bool,
    /// Another author than the configured user (`--author`).
    pub author: Option<String>,
}

impl CommitRequest {
    /// The paths the commit is limited to: every included change (both
    /// sides of a rename) and unversioned file.
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for c in &self.changes {
            out.push(c.path.clone());
            out.extend(c.old_path.clone());
        }
        out.extend(self.unversioned.iter().cloned());
        out.sort();
        out.dedup();
        out
    }

    /// Number of files the commit takes (a rename is one).
    pub fn file_count(&self) -> usize {
        self.changes.len() + self.unversioned.len()
    }
}

/// The last commit of the current branch, as Amend shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCommit {
    pub oid: String,
    /// The first line of the message.
    pub subject: String,
    /// The whole message.
    pub message: String,
    /// `Name <email>`.
    pub author: String,
}

/// What a rollback does to one change (`GitRollbackEnvironment`): modified
/// and deleted files get their committed content back, added files leave
/// the index, a rename gets the old file back and drops the new one.
pub fn rollback_kind(status: FileStatus) -> RollbackKind {
    match status {
        FileStatus::Added => RollbackKind::Unindex,
        FileStatus::Renamed => RollbackKind::UndoRename,
        _ => RollbackKind::Revert,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackKind {
    /// `git checkout HEAD -- <path>`.
    Revert,
    /// `git rm --cached -f -- <path>`: the file becomes unversioned (and is
    /// deleted when asked).
    Unindex,
    /// The old path is reverted, the new one unindexed and deleted.
    UndoRename,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_take_both_sides_of_a_rename() {
        let request = CommitRequest {
            changes: vec![
                Change {
                    path: "/r/b.md".into(),
                    status: FileStatus::Renamed,
                    old_path: Some("/r/a.txt".into()),
                },
                Change {
                    path: "/r/m.py".into(),
                    status: FileStatus::Modified,
                    old_path: None,
                },
            ],
            unversioned: vec!["/r/new.py".into()],
            ..CommitRequest::default()
        };
        assert_eq!(
            request.paths(),
            vec![
                PathBuf::from("/r/a.txt"),
                "/r/b.md".into(),
                "/r/m.py".into(),
                "/r/new.py".into()
            ]
        );
        assert_eq!(request.file_count(), 3);
        assert_eq!(rollback_kind(FileStatus::Added), RollbackKind::Unindex);
        assert_eq!(rollback_kind(FileStatus::Deleted), RollbackKind::Revert);
    }
}
