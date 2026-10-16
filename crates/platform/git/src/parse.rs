//! Parsers of git's machine-readable output: `status --porcelain=v2 -z
//! --branch`, `for-each-ref --format` and `ls-tree -z`.

use std::path::{Path, PathBuf};

use vcs::{Branch, BranchKind, FileStatus, StatusMap, Tracking};

/// The branch headers of `status --porcelain=v2 --branch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchHeaders {
    /// `None` before the first commit.
    pub oid: Option<String>,
    /// `None` when `HEAD` is detached.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// `# branch.ab` came: the upstream exists.
    pub has_ab: bool,
}

impl BranchHeaders {
    pub fn tracking(&self) -> Option<Tracking> {
        let upstream = self.upstream.clone()?;
        Some(Tracking {
            upstream,
            ahead: self.ahead,
            behind: self.behind,
            gone: !self.has_ab,
        })
    }
}

/// A path of git's output (relative, `/` separators) under `root`, with
/// the system's separators.
pub fn join(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    out
}

/// A path under `root` as git names it (relative, `/` separators).
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

/// The status of a tracked file from the `XY` field of an ordinary entry:
/// the change of the work tree against `HEAD`, through the index.
fn tracked_status(xy: &[u8]) -> Option<FileStatus> {
    let (x, y) = (xy.first().copied()?, xy.get(1).copied()?);
    Some(match (x, y) {
        // Added to the index, then removed from the work tree: nothing
        // against HEAD.
        (b'A', b'D') => return None,
        (b'A', _) | (_, b'A') => FileStatus::Added,
        (b'D', _) | (_, b'D') => FileStatus::Deleted,
        (b'.', b'.') => return None,
        _ => FileStatus::Modified,
    })
}

/// Parses `git status --porcelain=v2 -z --branch` output into a status
/// map of the work tree at `root`.
pub fn status_v2(root: &Path, out: &[u8]) -> (StatusMap, BranchHeaders) {
    let mut map = StatusMap::new(root);
    let mut headers = BranchHeaders::default();
    let mut records = out.split(|b| *b == 0).filter(|r| !r.is_empty());
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    while let Some(record) = records.next() {
        match record.first() {
            Some(b'#') => {
                let line = text(record);
                let mut parts = line.splitn(3, ' ');
                let (_, key, value) = (parts.next(), parts.next(), parts.next().unwrap_or(""));
                match key {
                    Some("branch.oid") => {
                        headers.oid = (value != "(initial)").then(|| value.to_owned());
                    }
                    Some("branch.head") => {
                        headers.head = (value != "(detached)").then(|| value.to_owned());
                    }
                    Some("branch.upstream") => headers.upstream = Some(value.to_owned()),
                    Some("branch.ab") => {
                        let mut ab = value.split(' ');
                        let num = |s: Option<&str>| {
                            s.and_then(|s| s.get(1..)).and_then(|n| n.parse().ok())
                        };
                        headers.ahead = num(ab.next()).unwrap_or(0);
                        headers.behind = num(ab.next()).unwrap_or(0);
                        headers.has_ab = true;
                    }
                    _ => {}
                }
            }
            // 1 XY sub mH mI mW hH hI path
            Some(b'1') => {
                let mut fields = record.splitn(9, |b| *b == b' ');
                let xy = fields.nth(1).unwrap_or_default();
                let Some(path) = fields.nth(6) else { continue };
                if let Some(status) = tracked_status(xy) {
                    map.insert(join(root, &text(path)), status);
                }
            }
            // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
            Some(b'2') => {
                let mut fields = record.splitn(10, |b| *b == b' ');
                let xy = fields.nth(1).unwrap_or_default().to_vec();
                let score = fields.nth(6).unwrap_or_default().to_vec();
                let Some(path) = fields.next() else { continue };
                let path = join(root, &text(path));
                let Some(orig) = records.next() else { break };
                let orig = join(root, &text(orig));
                if xy.get(1) == Some(&b'D') {
                    // Renamed in the index, then removed: the old file is
                    // gone.
                    map.insert(orig, FileStatus::Deleted);
                } else if score.first() == Some(&b'C') {
                    map.insert(path, FileStatus::Added);
                } else {
                    map.insert_rename(path, orig);
                }
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            Some(b'u') => {
                let mut fields = record.splitn(11, |b| *b == b' ');
                if let Some(path) = fields.nth(10) {
                    map.insert(join(root, &text(path)), FileStatus::Conflict);
                }
            }
            Some(b'?') => {
                if let Some(path) = record.get(2..) {
                    map.insert(join(root, &text(path)), FileStatus::Unversioned);
                }
            }
            Some(b'!') => {
                if let Some(path) = record.get(2..) {
                    let rel = text(path);
                    match rel.strip_suffix('/') {
                        Some(dir) => map.insert_ignored_dir(join(root, dir)),
                        None => map.insert(join(root, &rel), FileStatus::Ignored),
                    }
                }
            }
            _ => {}
        }
    }
    (map, headers)
}

/// The format given to `git for-each-ref` for [`refs`].
pub const REFS_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(symref)";

/// Parses `git for-each-ref` output in [`REFS_FORMAT`]: local branches,
/// remote branches (without the remotes' `HEAD`) and tags.
pub fn refs(out: &str) -> Vec<Branch> {
    let mut list = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 7 || !f[6].is_empty() {
            continue;
        }
        let (refname, oid, peeled, head, upstream, track) = (f[0], f[1], f[2], f[3], f[4], f[5]);
        let (kind, name) = if let Some(n) = refname.strip_prefix("refs/heads/") {
            (BranchKind::Local, n)
        } else if let Some(n) = refname.strip_prefix("refs/remotes/") {
            (BranchKind::Remote, n)
        } else if let Some(n) = refname.strip_prefix("refs/tags/") {
            (BranchKind::Tag, n)
        } else {
            continue;
        };
        let tracking = (!upstream.is_empty()).then(|| {
            let mut t = Tracking {
                upstream: upstream.to_owned(),
                ahead: 0,
                behind: 0,
                gone: track == "gone",
            };
            for part in track.split(", ") {
                if let Some(n) = part.strip_prefix("ahead ") {
                    t.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix("behind ") {
                    t.behind = n.parse().unwrap_or(0);
                }
            }
            t
        });
        list.push(Branch {
            name: name.to_owned(),
            kind,
            oid: if peeled.is_empty() { oid } else { peeled }.to_owned(),
            current: head == "*",
            tracking,
        });
    }
    list
}

/// The blob of the one entry of `git ls-tree -z <tree> -- <path>`.
pub fn ls_tree_blob(out: &[u8]) -> Option<String> {
    let entry = out.split(|b| *b == 0).find(|e| !e.is_empty())?;
    let entry = String::from_utf8_lossy(entry);
    let (meta, _path) = entry.split_once('\t')?;
    let mut parts = meta.split(' ');
    let (_mode, kind, oid) = (parts.next()?, parts.next()?, parts.next()?);
    (kind == "blob").then(|| oid.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        join(Path::new("/r"), s)
    }

    #[test]
    fn porcelain_v2_entries_become_statuses() {
        let out = b"# branch.oid 1234abcd\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaaa bbbb src/a b.py\0\
1 A. N... 000000 100644 100644 0000 cccc added.py\0\
1 .D N... 100644 100644 000000 dddd dddd gone.py\0\
1 AD N... 000000 100644 000000 0000 eeee ghost.py\0\
2 R. N... 100644 100644 100644 ffff ffff R100 new.md\0old.md\0\
u UU N... 100644 100644 100644 100644 1111 2222 3333 both.py\0\
? notes.txt\0! logs/\0! .env\0";
        let (map, headers) = status_v2(Path::new("/r"), out);
        assert_eq!(map.status(&p("src/a b.py")), FileStatus::Modified);
        assert_eq!(map.status(&p("added.py")), FileStatus::Added);
        assert_eq!(map.status(&p("gone.py")), FileStatus::Deleted);
        assert_eq!(map.status(&p("ghost.py")), FileStatus::NotChanged);
        assert_eq!(map.status(&p("new.md")), FileStatus::Renamed);
        assert_eq!(map.old_path(&p("new.md")), Some(p("old.md").as_path()));
        assert_eq!(map.status(&p("both.py")), FileStatus::Conflict);
        assert_eq!(map.status(&p("notes.txt")), FileStatus::Unversioned);
        assert_eq!(map.status(&p("logs/today.log")), FileStatus::Ignored);
        assert_eq!(map.status(&p(".env")), FileStatus::Ignored);
        assert_eq!(headers.head.as_deref(), Some("main"));
        assert_eq!(
            headers.tracking(),
            Some(Tracking {
                upstream: "origin/main".into(),
                ahead: 2,
                behind: 1,
                gone: false
            })
        );
    }

    #[test]
    fn a_detached_head_and_an_initial_commit_have_no_names() {
        let (_, h) = status_v2(
            Path::new("/r"),
            b"# branch.oid (initial)\0# branch.head (detached)\0",
        );
        assert_eq!(h.oid, None);
        assert_eq!(h.head, None);
        assert_eq!(h.tracking(), None);
    }

    #[test]
    fn refs_are_branches_and_tags() {
        let out = "refs/heads/main\0aaa\0\0*\0origin/main\0ahead 1, behind 2\0\n\
refs/heads/old\0bbb\0\0 \0origin/old\0gone\0\n\
refs/remotes/origin/HEAD\0aaa\0\0 \0\0\0refs/remotes/origin/main\n\
refs/remotes/origin/main\0ccc\0\0 \0\0\0\n\
refs/tags/v1\0ddd\0eee\0 \0\0\0\n";
        let list = refs(out);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].name, "main");
        assert!(list[0].current);
        let t = list[0].tracking.as_ref().unwrap();
        assert_eq!((t.ahead, t.behind, t.gone), (1, 2, false));
        assert!(list[1].tracking.as_ref().unwrap().gone);
        assert_eq!(list[2].kind, BranchKind::Remote);
        assert_eq!(list[2].name, "origin/main");
        assert_eq!(list[3].kind, BranchKind::Tag);
        assert_eq!(list[3].oid, "eee");
    }

    #[test]
    fn paths_are_joined_with_the_systems_separators() {
        let root = Path::new("/r");
        assert_eq!(
            relative(root, &join(root, "a/b.py")).as_deref(),
            Some("a/b.py")
        );
        assert_eq!(relative(root, root).as_deref(), Some("."));
        assert_eq!(
            ls_tree_blob(b"100644 blob 0123abcd\ta.py\0").as_deref(),
            Some("0123abcd")
        );
        assert_eq!(ls_tree_blob(b""), None);
    }
}
