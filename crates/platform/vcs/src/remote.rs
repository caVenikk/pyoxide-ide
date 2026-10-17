use serde::{Deserialize, Serialize};

use crate::FileStatus;

/// How Update Project brings the incoming commits into the current branch
/// (the reference's `UpdateMethod`; Branch Default and Reset are not
/// offered).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMethod {
    #[default]
    Merge,
    Rebase,
}

impl UpdateMethod {
    pub fn title(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
        }
    }

    /// The radio button of the Update Project dialog
    /// (`settings.git.update.method.*.description`).
    pub fn description(self) -> &'static str {
        match self {
            Self::Merge => "Merge incoming changes into the current branch",
            Self::Rebase => "Rebase the current branch on top of incoming changes",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "merge" => Some(Self::Merge),
            "rebase" => Some(Self::Rebase),
            _ => None,
        }
    }
}

/// How a push overwrites the remote branch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceMode {
    #[default]
    None,
    /// Force Push of the dialog: `--force-with-lease`, so that commits
    /// not fetched yet are not lost (`git.use.push.force.with.lease`).
    WithLease,
    /// Force push anyway, after a rejected force-with-lease push.
    Force,
}

impl ForceMode {
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WithLease => "with_lease",
            Self::Force => "force",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "none" => Some(Self::None),
            "with_lease" | "with-lease" => Some(Self::WithLease),
            "force" => Some(Self::Force),
            _ => None,
        }
    }
}

/// Which tags a push takes along (Push tags of the dialog).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagMode {
    /// All tags (`--tags`).
    #[default]
    All,
    /// Annotated tags of the pushed commits (`--follow-tags`).
    CurrentBranch,
}

impl TagMode {
    pub fn title(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::CurrentBranch => "Current Branch",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::CurrentBranch => "current_branch",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "all" => Some(Self::All),
            "current_branch" | "current-branch" => Some(Self::CurrentBranch),
            _ => None,
        }
    }
}

/// Where a branch is pushed: a branch of a remote.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PushTarget {
    pub remote: String,
    /// The branch's name on the remote (`main`).
    pub branch: String,
    /// The remote has no such branch yet (the `New` label).
    pub new: bool,
}

impl PushTarget {
    /// `origin/main`.
    pub fn short_name(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }
}

/// What a push does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSpec {
    /// The local branch pushed.
    pub source: String,
    pub target: PushTarget,
    /// Makes the target the source's upstream (`--set-upstream`).
    pub set_upstream: bool,
    pub force: ForceMode,
    pub tags: Option<TagMode>,
}

/// How a push ended (the reference's `GitPushRepoResult.Type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PushOutcome {
    /// Fast-forwarded the remote branch by `commits`.
    Success {
        commits: u32,
    },
    NewBranch,
    UpToDate,
    Forced,
    /// The remote branch has commits the local one has not.
    RejectedNoFastForward,
    /// A force-with-lease push found the remote branch elsewhere.
    RejectedStaleInfo,
    /// The remote refused it (a hook, a protected branch).
    RejectedOther {
        reason: String,
    },
    /// git failed before pushing anything (no remote, authentication).
    Error {
        message: String,
    },
}

impl PushOutcome {
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            Self::Success { .. } | Self::NewBranch | Self::UpToDate | Self::Forced
        )
    }

    pub fn is_rejected(&self) -> bool {
        matches!(
            self,
            Self::RejectedNoFastForward | Self::RejectedStaleInfo | Self::RejectedOther { .. }
        )
    }
}

/// A finished push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResult {
    pub source: String,
    pub target: PushTarget,
    pub outcome: PushOutcome,
    /// Tags pushed along (`v1.0`).
    pub tags: Vec<String>,
}

fn commits(n: u32) -> String {
    if n == 1 {
        "1 commit".to_owned()
    } else {
        format!("{n} commits")
    }
}

impl PushResult {
    /// The text of the push's balloon (`formRepoDescription`), without what
    /// an update in between did.
    pub fn description(&self) -> String {
        let target = self.target.short_name();
        let remote = &self.target.remote;
        let tags = &self.tags;
        match &self.outcome {
            PushOutcome::Success { commits: n } => match tags.len() {
                0 => format!("Pushed {} to {target}", commits(*n)),
                1 => format!(
                    "Pushed {} to {target}, and tag {} to {remote}",
                    commits(*n),
                    tags[0]
                ),
                k => format!(
                    "Pushed {} to {target}, and {k} tags to {remote}",
                    commits(*n)
                ),
            },
            PushOutcome::NewBranch => match tags.len() {
                0 => format!("Pushed {} to new branch {target}", self.source),
                1 => format!(
                    "Pushed {} to new branch {target}, and tag {} to {remote}",
                    self.source, tags[0]
                ),
                k => format!(
                    "Pushed {} to new branch {target}, and {k} tags to {remote}",
                    self.source
                ),
            },
            PushOutcome::UpToDate => match tags.len() {
                0 => "Everything is up to date".to_owned(),
                1 => format!("Pushed tag {} to {remote}", tags[0]),
                k => format!("Pushed {k} tags to {remote}"),
            },
            PushOutcome::Forced => format!("Force pushed {} to {target}", self.source),
            PushOutcome::RejectedNoFastForward => format!("Push to {target} was rejected"),
            PushOutcome::RejectedStaleInfo => format!(
                "Force-with-lease push {} to {target} was rejected",
                self.source
            ),
            PushOutcome::RejectedOther { .. } => format!(
                "Push {} to {target} was rejected by the remote",
                self.source
            ),
            PushOutcome::Error { message } => message.clone(),
        }
    }
}

/// A commit of the push dialog's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingCommit {
    pub oid: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
}

impl OutgoingCommit {
    pub fn short_oid(&self) -> &str {
        &self.oid[..self.oid.len().min(8)]
    }
}

/// A file a commit changed, relative to the work tree's root with `/`
/// separators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitFile {
    pub path: String,
    pub status: FileStatus,
    pub old_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(outcome: PushOutcome, tags: &[&str]) -> PushResult {
        PushResult {
            source: "main".into(),
            target: PushTarget {
                remote: "origin".into(),
                branch: "main".into(),
                new: false,
            },
            outcome,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    #[test]
    fn push_balloons() {
        let text = |o, tags: &[&str]| result(o, tags).description();
        assert_eq!(
            text(PushOutcome::Success { commits: 1 }, &[]),
            "Pushed 1 commit to origin/main"
        );
        assert_eq!(
            text(PushOutcome::Success { commits: 3 }, &["v1"]),
            "Pushed 3 commits to origin/main, and tag v1 to origin"
        );
        assert_eq!(
            text(PushOutcome::NewBranch, &[]),
            "Pushed main to new branch origin/main"
        );
        assert_eq!(text(PushOutcome::UpToDate, &[]), "Everything is up to date");
        assert_eq!(
            text(PushOutcome::UpToDate, &["v1", "v2"]),
            "Pushed 2 tags to origin"
        );
        assert_eq!(
            text(PushOutcome::Forced, &[]),
            "Force pushed main to origin/main"
        );
        assert_eq!(
            text(PushOutcome::RejectedNoFastForward, &[]),
            "Push to origin/main was rejected"
        );
        assert_eq!(
            text(PushOutcome::RejectedStaleInfo, &[]),
            "Force-with-lease push main to origin/main was rejected"
        );
        assert!(PushOutcome::Forced.is_success());
        assert!(PushOutcome::RejectedStaleInfo.is_rejected());
        assert_eq!(UpdateMethod::from_id("rebase"), Some(UpdateMethod::Rebase));
        assert_eq!(ForceMode::from_id("with-lease"), Some(ForceMode::WithLease));
    }
}
