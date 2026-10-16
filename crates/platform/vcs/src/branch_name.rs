//! Names of new branches, as the reference's New Branch dialog checks and
//! cleans them (`GitRefNameValidator`, `GitBranchValidator`,
//! `GitNewBranchDialog`): the rules of `git check-ref-format`, characters
//! git refuses dropped while typing and spaces turned into `-`, and the
//! conflicts with existing branches.

use crate::{Branch, BranchKind};

/// What a name of a new branch is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Head,
    Invalid(String),
    /// A local branch has the name (an overwrite can be asked for).
    Exists(String),
    /// A remote branch has the name (`origin/main`).
    ClashesWithRemote(String),
    /// Local branches are below the name as a folder (`feat` with
    /// `feat/x`).
    ClashesWithDirectory(String),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("Specify name for the new branch"),
            Self::Head => f.write_str("HEAD is a reserved keyword"),
            Self::Invalid(n) => write!(f, "Branch name {n} is not valid"),
            Self::Exists(n) => write!(f, "Branch name {n} already exists"),
            Self::ClashesWithRemote(n) => {
                write!(
                    f,
                    "Branch name {n} conflicts with remote branch with the same name"
                )
            }
            Self::ClashesWithDirectory(n) => write!(
                f,
                "Branch name {n} conflicts with local branch directory with the same name"
            ),
        }
    }
}

impl std::error::Error for NameError {}

/// Characters git never takes in a ref name.
const FORBIDDEN: &[char] = &['~', ':', '^', '?', '*', '"', '[', '\\'];

fn is_control(c: char) -> bool {
    (c as u32) < 32 || c == '\u{7f}'
}

/// The name is a valid ref name (`GitRefNameValidator.checkInput`).
pub fn is_valid(name: &str) -> bool {
    if name.trim().is_empty() {
        return false;
    }
    let bad_start = name.starts_with(['.', '-', '/']);
    let bad_end = name.ends_with(['.', '/']) || name.ends_with(".lock");
    let bad_inside = name.contains(FORBIDDEN)
        || name.contains("@{")
        || name.contains("//")
        || name.contains("..")
        || name.contains("./")
        || name.contains("/.")
        || name.contains(' ')
        || name.chars().any(is_control);
    !(bad_start || bad_end || bad_inside)
}

/// The name as typed with what git refuses dropped and spaces (and
/// control characters) replaced by `-`
/// (`GitRefNameValidator.cleanUpBranchNameOnTyping`).
pub fn clean_up_on_typing(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut kept = String::with_capacity(name.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let drop = (i == 0 && matches!(c, '.' | '-' | '/'))
            || FORBIDDEN.contains(&c)
            || (c == '/' && next == Some('/'))
            || (c == '.' && matches!(next, Some('.') | Some('/')))
            || (c == '.' && i > 0 && chars[i - 1] == '/');
        if c == '@' && next == Some('{') {
            i += 2;
            continue;
        }
        if !drop {
            kept.push(c);
        }
        i += 1;
    }
    let mut out = String::with_capacity(kept.len());
    let mut in_spaces = false;
    for c in kept.chars() {
        if c == ' ' {
            if !in_spaces {
                out.push('-');
            }
            in_spaces = true;
            continue;
        }
        in_spaces = false;
        out.push(if is_control(c) { '-' } else { c });
    }
    out
}

/// The name as it is created: cleaned as on typing, a trailing `.`, `/`
/// or `.lock` dropped, spaces around trimmed
/// (`GitRefNameValidator.cleanUpBranchName`).
pub fn clean_up(name: &str) -> String {
    let mut s = clean_up_on_typing(name);
    if s.ends_with(".lock") {
        s.truncate(s.len() - ".lock".len());
    } else if s.ends_with(['.', '/']) {
        s.pop();
    }
    s.trim().to_owned()
}

/// The folders of the local branches' names (`feat` of `feat/x`,
/// `GitNewBranchDialog.collectDirectories`).
pub fn directories<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        for (i, _) in name.match_indices('/') {
            let dir = &name[..i];
            if !out.iter().any(|d| d == dir) {
                out.push(dir.to_owned());
            }
        }
    }
    out
}

/// Checks `name` (already cleaned) for a new branch, or for a branch
/// renamed, against the repository's `branches`, in the reference's order:
/// empty or `HEAD` (only when the dialog is confirmed, `complete`), a
/// remote branch, a folder of local branches, a local branch (unless it is
/// to be overwritten).
pub fn check_new(
    name: &str,
    branches: &[Branch],
    overwrite: bool,
    complete: bool,
) -> Result<(), NameError> {
    if complete {
        if name.trim().is_empty() {
            return Err(NameError::Empty);
        }
        if name.eq_ignore_ascii_case("HEAD") {
            return Err(NameError::Head);
        }
        if !is_valid(name) {
            return Err(NameError::Invalid(name.to_owned()));
        }
    }
    let of = |kind: BranchKind| branches.iter().filter(move |b| b.kind == kind);
    if of(BranchKind::Remote).any(|b| b.name == name) {
        return Err(NameError::ClashesWithRemote(name.to_owned()));
    }
    if directories(of(BranchKind::Local).map(|b| b.name.as_str()))
        .iter()
        .any(|d| d == name)
    {
        return Err(NameError::ClashesWithDirectory(name.to_owned()));
    }
    if !overwrite && of(BranchKind::Local).any(|b| b.name == name) {
        return Err(NameError::Exists(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, kind: BranchKind) -> Branch {
        Branch {
            name: name.to_owned(),
            kind,
            oid: "0".repeat(40),
            current: false,
            tracking: None,
        }
    }

    #[test]
    fn ref_format_rules() {
        for good in ["main", "feat/x", "v1.0", "a-b_c", "feat/x.y", "ёлка"] {
            assert!(is_valid(good), "{good}");
        }
        for bad in [
            "", " ", ".x", "-x", "/x", "x/", "x.", "x.lock", "a..b", "a//b", "a/.b", "a./b", "a b",
            "a~b", "a:b", "a^b", "a?b", "a*b", "a[b", "a\\b", "a@{b", "a\u{7}b",
        ] {
            assert!(!is_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn typing_drops_what_git_refuses_and_turns_spaces_into_dashes() {
        assert_eq!(clean_up_on_typing("my new  branch"), "my-new-branch");
        assert_eq!(clean_up_on_typing(".hidden"), "hidden");
        assert_eq!(clean_up_on_typing("-x"), "x");
        assert_eq!(clean_up_on_typing("a~b:c^d?e*f[g\\h\"i"), "abcdefghi");
        assert_eq!(clean_up_on_typing("a@{b"), "ab");
        assert_eq!(clean_up_on_typing("a//b"), "a/b");
        assert_eq!(clean_up_on_typing("a..b"), "a.b");
        assert_eq!(clean_up_on_typing("a./b"), "a/b");
        assert_eq!(clean_up_on_typing("a/.b"), "a/b");
        // A trailing dot stays while typing ("v1." on the way to "v1.2").
        assert_eq!(clean_up_on_typing("v1."), "v1.");
        assert_eq!(clean_up("v1."), "v1");
        assert_eq!(clean_up("feat/"), "feat");
        assert_eq!(clean_up("x.lock"), "x");
        assert_eq!(clean_up("  name "), "-name-");
    }

    #[test]
    fn directories_of_branch_names() {
        assert_eq!(
            directories(["feat/a/b", "feat/c", "main"]),
            vec!["feat".to_owned(), "feat/a".to_owned()]
        );
    }

    #[test]
    fn conflicts_in_the_reference_order() {
        let branches = vec![
            branch("main", BranchKind::Local),
            branch("feat/x", BranchKind::Local),
            branch("origin/main", BranchKind::Remote),
            branch("v1", BranchKind::Tag),
        ];
        let check = |n: &str| check_new(n, &branches, false, true);
        assert_eq!(check(""), Err(NameError::Empty));
        assert_eq!(check("head"), Err(NameError::Head));
        assert_eq!(check("main"), Err(NameError::Exists("main".into())));
        assert_eq!(
            check("origin/main"),
            Err(NameError::ClashesWithRemote("origin/main".into()))
        );
        assert_eq!(
            check("feat"),
            Err(NameError::ClashesWithDirectory("feat".into()))
        );
        assert_eq!(check("feat/y"), Ok(()));
        assert_eq!(check("v1"), Ok(()));
        // Overwriting an existing branch was asked for.
        assert_eq!(check_new("main", &branches, true, true), Ok(()));
        // While typing an empty name is not reported.
        assert_eq!(check_new("", &branches, false, false), Ok(()));
        assert_eq!(
            NameError::Exists("main".into()).to_string(),
            "Branch name main already exists"
        );
    }
}
