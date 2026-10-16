//! What the VCS widget of the main toolbar shows (the reference's
//! `GitToolbarWidgetAction`, `GitCurrentBranchPresenter`,
//! `GitBranchUtil.getDisplayableBranchText`): the current branch with the
//! branch icon, cut in the middle beyond 80 characters; a detached `HEAD`
//! as the tag's name with the tag icon, or 8 characters of the commit with
//! a warning icon; an operation in progress ("Merging main") with a
//! warning icon; arrows for incoming and outgoing commits; without a
//! repository "Version Control" and no icon.

use serde_json::{Value as Json, json};
use ui::icons::IconName;
use vcs::{Head, RepoInfo, RepoState, Tracking};

/// The widget's text without a repository (`git.toolbar.widget.no.repo`)
/// and its tooltip.
pub const NO_REPO_TEXT: &str = "Version Control";
pub const NO_REPO_TOOLTIP: &str = "Configure a version control for the project";
/// The text while the repository is being read
/// (`git.toolbar.widget.no.loaded.repo`).
pub const LOADING_TEXT: &str = "Git";
/// The longest branch name shown whole, the characters of its end kept and
/// the slack before cutting (`GitBranchPresentation`).
const MAX_NAME: usize = 80;
const SUFFIX: usize = 5;
const DELTA: usize = 4;

/// The widget's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetIcon {
    /// On a branch: the branch icon (`AllIcons.General.Vcs`).
    Branch,
    /// A tag is checked out (`DvcsImplIcons.BranchLabel`).
    Tag,
    /// A commit is checked out, or an operation is in progress.
    Warning,
}

impl WidgetIcon {
    pub fn icon(self) -> IconName {
        match self {
            Self::Branch => IconName::VCS,
            Self::Tag => IconName::BRANCH_LABEL,
            Self::Warning => IconName::WARNING,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Branch => "branch",
            Self::Tag => "tag",
            Self::Warning => "warning",
        }
    }
}

/// What the widget shows for a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presentation {
    pub icon: WidgetIcon,
    pub text: String,
    /// Lines of the tooltip.
    pub tooltip: Vec<String>,
    pub incoming: bool,
    pub outgoing: bool,
}

/// What the widget shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidgetState {
    /// No project, or version control is off for it.
    Hidden,
    /// The project has no repository: "Version Control", the popup offers
    /// Create Git Repository….
    NoVcs,
    /// A repository is there, its state not read yet.
    Loading,
    Repository(Presentation),
}

impl WidgetState {
    /// The text, `None` when hidden.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Hidden => None,
            Self::NoVcs => Some(NO_REPO_TEXT),
            Self::Loading => Some(LOADING_TEXT),
            Self::Repository(p) => Some(&p.text),
        }
    }

    /// The icon before the text.
    pub fn icon(&self) -> Option<IconName> {
        match self {
            Self::Hidden | Self::NoVcs => None,
            Self::Loading => Some(IconName::VCS),
            Self::Repository(p) => Some(p.icon.icon()),
        }
    }

    pub fn tooltip(&self) -> Option<String> {
        match self {
            Self::NoVcs => Some(NO_REPO_TOOLTIP.to_owned()),
            Self::Repository(p) if !p.tooltip.is_empty() => Some(p.tooltip.join("\n")),
            _ => None,
        }
    }

    /// The state for `vcs.state`.
    pub fn state_json(&self) -> Json {
        let kind = match self {
            Self::Hidden => "hidden",
            Self::NoVcs => "no_vcs",
            Self::Loading => "loading",
            Self::Repository(_) => "repository",
        };
        let (icon, incoming, outgoing) = match self {
            Self::Repository(p) => (Some(p.icon.id()), p.incoming, p.outgoing),
            Self::Loading => (Some(WidgetIcon::Branch.id()), false, false),
            _ => (None, false, false),
        };
        json!({
            "kind": kind,
            "text": self.text(),
            "icon": icon,
            "tooltip": self.tooltip(),
            "incoming": incoming,
            "outgoing": outgoing,
        })
    }
}

/// `text` cut to `max` characters with `…` in place of its middle, the
/// last `suffix` characters kept (`StringUtil.shortenTextWithEllipsis`).
fn shorten_middle(text: &str, max: usize, suffix: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_owned();
    }
    let prefix = max - suffix - 1;
    let mut out: String = chars[..prefix].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - suffix..]);
    out
}

/// A branch name as the widget shows it: whole up to 84 characters, else
/// 73 of its beginning, `…` and 5 of its end
/// (`GitBranchPresentation.truncateBranchName`).
pub fn truncate_branch_name(name: &str) -> String {
    if name.chars().count() <= MAX_NAME + DELTA {
        return name.to_owned();
    }
    shorten_middle(name, MAX_NAME - 1, SUFFIX)
}

/// The short form of a commit (`DvcsUtil.getShortHash`).
pub fn short_hash(oid: &str) -> String {
    oid.chars().take(8).collect()
}

/// The tooltip line of incoming and outgoing commits
/// (`GitInOutCountersInProject.calcTooltip`).
pub fn in_out_tooltip(incoming: u32, outgoing: u32) -> Option<String> {
    let commits = |n: u32| if n == 1 { "commit" } else { "commits" };
    match (incoming, outgoing) {
        (0, 0) => None,
        (i, 0) => Some(format!("{i} incoming {}", commits(i))),
        (0, o) => Some(format!("{o} outgoing {}", commits(o))),
        (i, o) => Some(format!("{i} incoming and {o} outgoing commits")),
    }
}

/// The widget for a repository's state; `tag` is the tag `HEAD` is at
/// when it is detached.
pub fn presentation(info: &RepoInfo, tag: Option<&str>) -> Presentation {
    let branch = info.head.branch().map(truncate_branch_name);
    let (icon, text) = match (&info.head, info.state) {
        (Head::Detached { oid }, RepoState::Normal) => match tag {
            Some(tag) => (WidgetIcon::Tag, truncate_branch_name(tag)),
            None => (WidgetIcon::Warning, short_hash(oid)),
        },
        (_, RepoState::Normal) => (WidgetIcon::Branch, branch.unwrap_or_default()),
        (_, state) => {
            // During a rebase HEAD is detached: the branch being rebased.
            let name = branch
                .or_else(|| info.rebasing_branch.as_deref().map(truncate_branch_name))
                .unwrap_or_default();
            let text = match state {
                RepoState::Merging => format!("Merging {name}"),
                RepoState::Rebasing => format!("Rebasing {name}"),
                RepoState::CherryPicking => format!("Cherry-picking in {name}"),
                RepoState::Reverting => format!("Reverting in {name}"),
                RepoState::Normal => name,
            };
            (WidgetIcon::Warning, text)
        }
    };
    let (incoming, outgoing) = match &info.tracking {
        Some(Tracking { ahead, behind, .. }) if info.head.branch().is_some() => (*behind, *ahead),
        _ => (0, 0),
    };
    let tooltip = match (&info.head, info.state) {
        (Head::Detached { .. }, RepoState::Rebasing) | (_, RepoState::Rebasing) => {
            vec!["Git: Rebase in progress".to_owned()]
        }
        (Head::Detached { .. }, _) => {
            vec!["Git: Detached HEAD doesn't point to any branch".to_owned()]
        }
        (Head::Branch { name, .. }, _) => {
            let mut lines = vec![format!("Git Branch: {name}")];
            lines.extend(in_out_tooltip(incoming, outgoing));
            lines
        }
    };
    Presentation {
        icon,
        text,
        tooltip,
        incoming: incoming > 0,
        outgoing: outgoing > 0,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn info(head: Head, state: RepoState) -> RepoInfo {
        RepoInfo {
            root: PathBuf::from("/r"),
            git_dir: PathBuf::from("/r/.git"),
            head,
            state,
            rebasing_branch: None,
            tracking: None,
            remotes: Vec::new(),
        }
    }

    fn on(name: &str) -> Head {
        Head::Branch {
            name: name.to_owned(),
            oid: Some("a".repeat(40)),
        }
    }

    #[test]
    fn long_branch_names_are_cut_in_the_middle() {
        let name = "a".repeat(84);
        assert_eq!(truncate_branch_name(&name), name);
        let long = format!("{}{}", "b".repeat(80), "12345");
        let cut = truncate_branch_name(&long);
        assert_eq!(cut.chars().count(), 79);
        assert_eq!(cut, format!("{}\u{2026}12345", "b".repeat(73)));
    }

    #[test]
    fn a_branch_with_outgoing_commits() {
        let mut i = info(on("main"), RepoState::Normal);
        i.tracking = Some(Tracking {
            upstream: "origin/main".into(),
            ahead: 1,
            behind: 0,
            gone: false,
        });
        let p = presentation(&i, None);
        assert_eq!(p.icon, WidgetIcon::Branch);
        assert_eq!(p.text, "main");
        assert!(p.outgoing && !p.incoming);
        assert_eq!(p.tooltip, vec!["Git Branch: main", "1 outgoing commit"]);
        i.tracking.as_mut().unwrap().behind = 2;
        assert_eq!(
            presentation(&i, None).tooltip[1],
            "2 incoming and 1 outgoing commits"
        );
    }

    #[test]
    fn detached_head_shows_the_tag_or_the_commit() {
        let oid = "0123456789abcdef0123456789abcdef01234567".to_owned();
        let i = info(Head::Detached { oid }, RepoState::Normal);
        let p = presentation(&i, Some("v1.0"));
        assert_eq!((p.icon, p.text.as_str()), (WidgetIcon::Tag, "v1.0"));
        let p = presentation(&i, None);
        assert_eq!((p.icon, p.text.as_str()), (WidgetIcon::Warning, "01234567"));
        assert_eq!(
            p.tooltip,
            vec!["Git: Detached HEAD doesn't point to any branch"]
        );
    }

    #[test]
    fn operations_in_progress() {
        let p = presentation(&info(on("main"), RepoState::Merging), None);
        assert_eq!(
            (p.icon, p.text.as_str()),
            (WidgetIcon::Warning, "Merging main")
        );
        let mut i = info(
            Head::Detached {
                oid: "c".repeat(40),
            },
            RepoState::Rebasing,
        );
        i.rebasing_branch = Some("feat".into());
        let p = presentation(&i, None);
        assert_eq!(p.text, "Rebasing feat");
        assert_eq!(p.tooltip, vec!["Git: Rebase in progress"]);
    }

    #[test]
    fn states_without_a_repository() {
        assert_eq!(WidgetState::NoVcs.text(), Some("Version Control"));
        assert_eq!(WidgetState::NoVcs.icon(), None);
        assert_eq!(
            WidgetState::NoVcs.tooltip().as_deref(),
            Some("Configure a version control for the project")
        );
        assert_eq!(WidgetState::Hidden.text(), None);
        assert_eq!(WidgetState::NoVcs.state_json()["kind"], "no_vcs");
    }
}
