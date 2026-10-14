//! The file watchers servers register (`workspace/didChangeWatchedFiles`):
//! LSP glob patterns, plain or relative to a base folder, and the kinds of
//! change each watcher wants.
//!
//! Glob syntax (LSP 3.17): `*` any characters of one path segment, `?` one
//! character of a segment, `**` any number of segments (also none), `{a,b}`
//! alternatives, `[a-z]` and `[!a-z]` character ranges. Paths are matched
//! with `/` separators; on Windows without regard to case.

use std::path::{Path, PathBuf};

use code_intel::FileChangeKind;
use serde_json::Value;

use crate::uri;

/// One watcher of a registration.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Watcher {
    /// The folder a relative pattern is relative to.
    base: Option<PathBuf>,
    /// The pattern with its `{}` alternatives expanded.
    alternatives: Vec<Vec<char>>,
    /// The kinds of change it wants (LSP `WatchKind` bits: create 1,
    /// change 2, delete 4).
    kinds: u64,
}

impl Watcher {
    /// The watchers of a `workspace/didChangeWatchedFiles` registration's
    /// options; watchers of an unknown shape are left out.
    pub fn from_options(options: &Value) -> Vec<Watcher> {
        options
            .get("watchers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Watcher::from_value)
            .collect()
    }

    fn from_value(watcher: &Value) -> Option<Watcher> {
        let kinds = watcher.get("kind").and_then(Value::as_u64).unwrap_or(7);
        let (base, pattern) = match watcher.get("globPattern")? {
            Value::String(pattern) => (None, pattern.as_str()),
            Value::Object(relative) => {
                let base = match relative.get("baseUri")? {
                    Value::String(base) => base.as_str(),
                    // A workspace folder.
                    Value::Object(folder) => folder.get("uri")?.as_str()?,
                    _ => return None,
                };
                (
                    Some(uri::str_to_path(base)?),
                    relative.get("pattern")?.as_str()?,
                )
            }
            _ => return None,
        };
        Some(Watcher {
            base,
            alternatives: expand_braces(pattern)
                .into_iter()
                .map(|p| normalize(&p).chars().collect())
                .collect(),
            kinds,
        })
    }

    /// The watcher wants this change of this file.
    pub fn matches(&self, path: &Path, kind: FileChangeKind) -> bool {
        let bit = match kind {
            FileChangeKind::Created => 1,
            FileChangeKind::Changed => 2,
            FileChangeKind::Deleted => 4,
        };
        if self.kinds & bit == 0 {
            return false;
        }
        let path = match &self.base {
            Some(base) => match path.strip_prefix(base) {
                Ok(relative) => relative.to_path_buf(),
                Err(_) => return false,
            },
            None => path.to_path_buf(),
        };
        let path: Vec<char> = normalize(&path.to_string_lossy()).chars().collect();
        self.alternatives.iter().any(|p| glob_match(p, &path))
    }
}

/// `/` separators; lower case on Windows, where file names are compared
/// without regard to case.
fn normalize(s: &str) -> String {
    if cfg!(windows) {
        s.replace('\\', "/").to_lowercase()
    } else {
        s.to_owned()
    }
}

/// `a{b,c{d,e}}f` -> `abf`, `acdf`, `acef`. An unclosed brace is literal.
fn expand_braces(pattern: &str) -> Vec<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let Some(open) = chars.iter().position(|&c| c == '{') else {
        return vec![pattern.to_owned()];
    };
    let mut depth = 0;
    let mut parts = Vec::new();
    let mut start = open + 1;
    let mut close = None;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    parts.push(chars[start..i].iter().collect::<String>());
                    close = Some(i);
                    break;
                }
            }
            ',' if depth == 1 => {
                parts.push(chars[start..i].iter().collect::<String>());
                start = i + 1;
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![pattern.to_owned()];
    };
    let prefix: String = chars[..open].iter().collect();
    let suffix: String = chars[close + 1..].iter().collect();
    parts
        .iter()
        .flat_map(|part| expand_braces(&format!("{prefix}{part}{suffix}")))
        .collect()
}

fn glob_match(p: &[char], s: &[char]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some('*') if p.get(1) == Some(&'*') => {
            let rest = &p[2..];
            match rest.first() {
                // `**` at the end: anything.
                None => true,
                // `**/`: none or any whole segments.
                Some('/') => {
                    let rest = &rest[1..];
                    glob_match(rest, s)
                        || s.iter()
                            .enumerate()
                            .filter(|(_, c)| **c == '/')
                            .any(|(i, _)| glob_match(rest, &s[i + 1..]))
                }
                // `**x`: like `*`.
                Some(_) => star(rest, s),
            }
        }
        Some('*') => star(&p[1..], s),
        Some('?') => matches!(s.first(), Some(c) if *c != '/') && glob_match(&p[1..], &s[1..]),
        Some('[') => match class(p) {
            Some((matcher, len)) => {
                matches!(s.first(), Some(c) if *c != '/' && matcher(*c))
                    && glob_match(&p[len..], &s[1..])
            }
            None => s.first() == Some(&'[') && glob_match(&p[1..], &s[1..]),
        },
        Some(c) => s.first() == Some(c) && glob_match(&p[1..], &s[1..]),
    }
}

/// `*` followed by `rest`: any characters of the current segment.
fn star(rest: &[char], s: &[char]) -> bool {
    let segment = s.iter().position(|&c| c == '/').unwrap_or(s.len());
    (0..=segment).any(|i| glob_match(rest, &s[i..]))
}

/// A `[...]` class at the start of `p`: its test and its length.
fn class(p: &[char]) -> Option<(impl Fn(char) -> bool, usize)> {
    let end = p.iter().skip(2).position(|&c| c == ']')? + 2;
    let body: Vec<char> = p[1..end].to_vec();
    let (negated, body) = match body.first() {
        Some('!') | Some('^') => (true, body[1..].to_vec()),
        _ => (false, body),
    };
    let test = move |c: char| {
        let mut i = 0;
        let mut found = false;
        while i < body.len() {
            if i + 2 < body.len() && body[i + 1] == '-' {
                found |= body[i] <= c && c <= body[i + 2];
                i += 3;
            } else {
                found |= body[i] == c;
                i += 1;
            }
        }
        found != negated
    };
    Some((test, end + 1))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn glob(pattern: &str, path: &str) -> bool {
        let s: Vec<char> = path.chars().collect();
        expand_braces(pattern).iter().any(|e| {
            let e: Vec<char> = e.chars().collect();
            glob_match(&e, &s)
        })
    }

    #[test]
    fn glob_syntax() {
        assert!(glob("**/pyproject.toml", "/p/pyproject.toml"));
        assert!(glob("**/pyproject.toml", "pyproject.toml"));
        assert!(glob("**/pyproject.toml", "/p/sub/dir/pyproject.toml"));
        assert!(!glob("**/pyproject.toml", "/p/xpyproject.toml"));
        assert!(glob("**/*.py", "/p/a/b.py"));
        assert!(!glob("**/*.py", "/p/a/b.pyi"));
        assert!(glob("**/*.{py,pyi}", "/p/a/b.pyi"));
        assert!(glob("**/{ruff,.ruff}.toml", "/p/.ruff.toml"));
        assert!(glob("**/{ruff,.ruff}.toml", "/p/ruff.toml"));
        assert!(!glob("**/{ruff,.ruff}.toml", "/p/xruff.toml"));
        assert!(glob("*.py", "a.py"));
        assert!(!glob("*.py", "dir/a.py"));
        assert!(glob("src/**", "src/a/b/c"));
        assert!(glob("src/**/test_?.py", "src/x/test_1.py"));
        assert!(!glob("src/**/test_?.py", "src/x/test_12.py"));
        assert!(glob("example.[0-9]", "example.7"));
        assert!(!glob("example.[!0-9]", "example.7"));
        assert!(glob("example.[!0-9]", "example.x"));
        assert!(glob("a[b", "a[b"));
        assert_eq!(expand_braces("a{b,c{d,e}}f"), ["abf", "acdf", "acef"]);
        assert_eq!(expand_braces("a{b"), ["a{b"]);
    }

    fn path(p: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", p.replace('/', "\\")))
        } else {
            PathBuf::from(p)
        }
    }

    #[test]
    fn watchers_from_registrations() {
        let base = uri::from_path(&path("/p")).to_string();
        let watchers = Watcher::from_options(&json!({"watchers": [
            {"globPattern": "**/pyproject.toml"},
            {"globPattern": {"baseUri": base, "pattern": "**/*.toml"}, "kind": 2},
            {"globPattern": {"baseUri": {"uri": base, "name": "p"}, "pattern": "ty.toml"}, "kind": 5},
            {"globPattern": 42},
        ]}));
        assert_eq!(watchers.len(), 3);
        let pyproject = path("/p/sub/pyproject.toml");
        assert!(watchers[0].matches(&pyproject, FileChangeKind::Deleted));
        assert!(watchers[1].matches(&pyproject, FileChangeKind::Changed));
        assert!(!watchers[1].matches(&pyproject, FileChangeKind::Created));
        assert!(!watchers[1].matches(&path("/q/pyproject.toml"), FileChangeKind::Changed));
        assert!(watchers[2].matches(&path("/p/ty.toml"), FileChangeKind::Created));
        assert!(!watchers[2].matches(&path("/p/ty.toml"), FileChangeKind::Changed));
        assert!(!watchers[2].matches(&path("/p/sub/ty.toml"), FileChangeKind::Created));
    }
}
