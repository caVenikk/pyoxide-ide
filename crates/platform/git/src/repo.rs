//! A repository's folders and what its files say without running git
//! (the reference's `GitRepositoryReader`): `HEAD`, loose and packed refs,
//! the config's remotes and upstreams, and the markers of an operation in
//! progress.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use vcs::{Head, Remote, RepoState};

/// The folders of a work tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// The work tree's root.
    pub root: PathBuf,
    /// The repository folder: `.git`, or what a `gitdir:` file names (a
    /// linked work tree, a submodule).
    pub git_dir: PathBuf,
    /// Where refs, objects and the config are shared (`commondir` of a
    /// linked work tree), else `git_dir`.
    pub common_dir: PathBuf,
}

impl Repository {
    /// The work tree that contains `path` (a folder), at it or above.
    pub fn detect(path: &Path) -> Option<Self> {
        path.ancestors().find_map(Self::at)
    }

    /// The work tree whose root is `root`.
    pub fn at(root: &Path) -> Option<Self> {
        let dot_git = root.join(".git");
        let meta = fs::metadata(&dot_git).ok()?;
        let git_dir = if meta.is_dir() {
            dot_git
        } else {
            let text = fs::read_to_string(&dot_git).ok()?;
            let target = text.trim().strip_prefix("gitdir:")?.trim();
            let target = Path::new(target);
            if target.is_absolute() {
                target.to_owned()
            } else {
                normalize(&root.join(target))
            }
        };
        if !git_dir.join("HEAD").is_file() {
            return None;
        }
        let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
            Ok(text) => {
                let dir = Path::new(text.trim());
                if dir.is_absolute() {
                    dir.to_owned()
                } else {
                    normalize(&git_dir.join(dir))
                }
            }
            Err(_) => git_dir.clone(),
        };
        Some(Self {
            root: root.to_owned(),
            git_dir,
            common_dir,
        })
    }

    /// The config, shared by linked work trees.
    pub fn config(&self) -> Config {
        Config::parse(&fs::read_to_string(self.common_dir.join("config")).unwrap_or_default())
    }

    /// The refs are in a reftable (`extensions.refStorage`): git has to
    /// read them.
    pub fn uses_reftable(&self, config: &Config) -> bool {
        config
            .get("extensions", None, "refstorage")
            .is_some_and(|v| v.eq_ignore_ascii_case("reftable"))
    }

    /// `HEAD` from the files; `None` when it cannot be read.
    pub fn read_head(&self) -> Option<Head> {
        let text = fs::read_to_string(self.git_dir.join("HEAD")).ok()?;
        let text = text.trim();
        match text.strip_prefix("ref:") {
            Some(target) => {
                let target = target.trim();
                let name = target.strip_prefix("refs/heads/").unwrap_or(target);
                Some(Head::Branch {
                    name: name.to_owned(),
                    oid: self.resolve(target),
                })
            }
            None if is_oid(text) => Some(Head::Detached {
                oid: text.to_owned(),
            }),
            None => None,
        }
    }

    /// The commit a ref names, from its loose file or `packed-refs`,
    /// following symbolic refs.
    pub fn resolve(&self, name: &str) -> Option<String> {
        let mut name = name.to_owned();
        for _ in 0..5 {
            // Refs of the work tree itself stay in its own folder.
            let per_worktree = name == "HEAD"
                || name.starts_with("refs/bisect/")
                || name.starts_with("refs/worktree/")
                || name.starts_with("refs/rewritten/");
            let dir = if per_worktree {
                &self.git_dir
            } else {
                &self.common_dir
            };
            let loose = name.split('/').fold(dir.clone(), |p, part| p.join(part));
            match fs::read_to_string(&loose) {
                Ok(text) => {
                    let text = text.trim();
                    match text.strip_prefix("ref:") {
                        Some(next) => name = next.trim().to_owned(),
                        None => return is_oid(text).then(|| text.to_owned()),
                    }
                }
                Err(_) => return self.packed_ref(&name),
            }
        }
        None
    }

    fn packed_ref(&self, name: &str) -> Option<String> {
        let text = fs::read_to_string(self.common_dir.join("packed-refs")).ok()?;
        text.lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
            .find_map(|l| {
                let (oid, refname) = l.split_once(' ')?;
                (refname.trim() == name).then(|| oid.to_owned())
            })
    }

    /// The operation in progress, and the branch being rebased.
    pub fn read_state(&self) -> (RepoState, Option<String>) {
        let dir = &self.git_dir;
        for rebase in ["rebase-merge", "rebase-apply"] {
            let folder = dir.join(rebase);
            if folder.is_dir() {
                let branch = fs::read_to_string(folder.join("head-name"))
                    .ok()
                    .and_then(|t| t.trim().strip_prefix("refs/heads/").map(str::to_owned));
                return (RepoState::Rebasing, branch);
            }
        }
        let state = if dir.join("MERGE_HEAD").is_file() {
            RepoState::Merging
        } else if dir.join("CHERRY_PICK_HEAD").is_file() {
            RepoState::CherryPicking
        } else if dir.join("REVERT_HEAD").is_file() {
            RepoState::Reverting
        } else {
            RepoState::Normal
        };
        (state, None)
    }

    /// The remotes of the config, by name.
    pub fn remotes(&self, config: &Config) -> Vec<Remote> {
        config
            .subsections("remote")
            .into_iter()
            .map(|name| Remote {
                urls: config.get_all("remote", Some(&name), "url"),
                name,
            })
            .collect()
    }

    /// A path whose change may change the repository's state or statuses:
    /// the files the readers look at, the index, `info/exclude`. Lock
    /// files, objects and logs are left out.
    pub fn is_state_path(&self, path: &Path) -> bool {
        [&self.git_dir, &self.common_dir]
            .iter()
            .any(|dir| path.strip_prefix(dir).is_ok_and(is_state_rel))
    }
}

/// A path inside some `.git` folder that the file watcher passes on: `Some`
/// for paths inside a `.git` folder (true for `HEAD`, `index`, refs, the
/// config and the state markers, also of linked work trees in
/// `worktrees/<name>/`; false for objects, logs and lock files), `None`
/// for other paths.
pub fn watched_in_git_dir(path: &Path) -> Option<bool> {
    let mut components = path.components();
    // The last `.git` component, and what follows it.
    let mut rest = None;
    while let Some(c) = components.next() {
        if c.as_os_str() == ".git" {
            rest = Some(components.clone());
        }
    }
    let rest = rest?.as_path().to_owned();
    if rest.as_os_str().is_empty() {
        // The folder itself (created or removed: `git init`).
        return Some(true);
    }
    let rel = match rest.strip_prefix("worktrees") {
        Ok(inner) => {
            let mut c = inner.components();
            c.next();
            c.as_path().to_owned()
        }
        Err(_) => rest,
    };
    Some(is_state_rel(&rel))
}

fn is_state_rel(rel: &Path) -> bool {
    let first = rel
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy());
    let Some(first) = first else {
        return false;
    };
    let name = rel
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    if name.ends_with(".lock") {
        return false;
    }
    match first.as_ref() {
        "HEAD" | "index" | "packed-refs" | "config" | "MERGE_HEAD" | "CHERRY_PICK_HEAD"
        | "REVERT_HEAD" | "ORIG_HEAD" | "refs" | "rebase-merge" | "rebase-apply" => true,
        "info" => name == "exclude",
        _ => false,
    }
}

fn is_oid(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `a/./b/../c` as `a/c`, without looking at the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A git config file as far as the readers need it: values by section,
/// subsection and key (`[remote "origin"] url`). Section and key names are
/// case-insensitive, subsections are not; includes are not followed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    values: BTreeMap<(String, Option<String>, String), Vec<String>>,
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let mut values: BTreeMap<_, Vec<String>> = BTreeMap::new();
        let mut section = (String::new(), None::<String>);
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(header) = line.strip_prefix('[') {
                let Some(end) = header.find(']') else {
                    continue;
                };
                let header = &header[..end];
                section = match header.split_once(char::is_whitespace) {
                    Some((name, sub)) => (
                        name.to_ascii_lowercase(),
                        Some(sub.trim().trim_matches('"').replace("\\\"", "\"")),
                    ),
                    None => match header.split_once('.') {
                        // The old `[remote.origin]` form.
                        Some((name, sub)) => (name.to_ascii_lowercase(), Some(sub.to_owned())),
                        None => (header.to_ascii_lowercase(), None),
                    },
                };
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some((k, v)) => (k.trim(), unquote(v.trim())),
                None => (line, "true".to_owned()),
            };
            values
                .entry((
                    section.0.clone(),
                    section.1.clone(),
                    key.to_ascii_lowercase(),
                ))
                .or_default()
                .push(value);
        }
        Self { values }
    }

    /// The last value of a key.
    pub fn get(&self, section: &str, sub: Option<&str>, key: &str) -> Option<String> {
        self.get_all(section, sub, key).pop()
    }

    pub fn get_all(&self, section: &str, sub: Option<&str>, key: &str) -> Vec<String> {
        self.values
            .get(&(
                section.to_ascii_lowercase(),
                sub.map(str::to_owned),
                key.to_ascii_lowercase(),
            ))
            .cloned()
            .unwrap_or_default()
    }

    /// The subsections of a section (`remote` -> `origin`, ...), in order of
    /// name.
    pub fn subsections(&self, section: &str) -> Vec<String> {
        let section = section.to_ascii_lowercase();
        let mut out: Vec<String> = self
            .values
            .keys()
            .filter(|(s, sub, _)| *s == section && sub.is_some())
            .filter_map(|(_, sub, _)| sub.clone())
            .collect();
        out.dedup();
        out
    }
}

/// A config value without its quotes and comment.
fn unquote(value: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(match next {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                }
            }
            '#' | ';' if !quoted => break,
            c => out.push(c),
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_values_by_section_and_subsection() {
        let c = Config::parse(
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@example.com:a/b.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n[Extensions]\n\trefStorage = \"reftable\" ; comment\n[remote.up]\nurl=u\n",
        );
        assert_eq!(c.get("core", None, "bare").as_deref(), Some("false"));
        assert_eq!(
            c.get("remote", Some("origin"), "URL").as_deref(),
            Some("git@example.com:a/b.git")
        );
        assert_eq!(
            c.get("branch", Some("main"), "merge").as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(
            c.get("extensions", None, "refstorage").as_deref(),
            Some("reftable")
        );
        assert_eq!(
            c.subsections("remote"),
            vec!["origin".to_owned(), "up".to_owned()]
        );
    }

    #[test]
    fn the_watcher_passes_state_paths_of_git_folders() {
        let p = |s: &str| watched_in_git_dir(Path::new(s));
        assert_eq!(p("/r/.git/HEAD"), Some(true));
        assert_eq!(p("/r/.git/index"), Some(true));
        assert_eq!(p("/r/.git/refs/heads/feat/x"), Some(true));
        assert_eq!(p("/r/.git/rebase-merge/done"), Some(true));
        assert_eq!(p("/r/.git/info/exclude"), Some(true));
        assert_eq!(p("/r/.git/worktrees/w/HEAD"), Some(true));
        assert_eq!(p("/r/.git"), Some(true));
        assert_eq!(p("/r/.git/index.lock"), Some(false));
        assert_eq!(p("/r/.git/refs/heads/main.lock"), Some(false));
        assert_eq!(p("/r/.git/objects/ab/cdef"), Some(false));
        assert_eq!(p("/r/.git/logs/HEAD"), Some(false));
        assert_eq!(p("/r/.git/FETCH_HEAD"), Some(false));
        assert_eq!(p("/r/src/a.py"), None);
    }
}
