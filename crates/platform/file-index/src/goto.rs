//! Go to File as the reference ranks it.
//!
//! The pattern is split at its last `/` into a folder part and a name
//! part. Names are tried with the whole name part first (`*app`, start
//! matches preferred), then with shorter and shorter suffixes of it while
//! the letters dropped from the front go to the folder part instead
//! (`flaskapp` finds `app.py` in `flask/`). For each round:
//!
//! - the matching names are ordered: names that start with the pattern
//!   (as typed) first, then by matching degree; names of equal rank by
//!   their name without extension;
//! - the files of one name (or of names that differ only in case or
//!   extension) go open files first, then files in the folder of the
//!   current file, then project files before non-project ones, then the
//!   shallower, then by path;
//! - with a folder part, the folders must match it and the files are
//!   ranked by how well their folder path matches it;
//! - folders come after the files of a round.
//!
//! Each file gets a weight, the matching degree (folders 0); the list is
//! finally ordered by weight, keeping this order between equal weights.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf, Prefix};

use name_match::{Matcher, MatchingMode, START_MATCH_WEIGHT};

use crate::index::{EntryId, FileIndex, ROOT};
use crate::position::parse_position;

/// The weight of an exact match (a path typed in full).
pub const EXACT_MATCH_DEGREE: i32 = START_MATCH_WEIGHT + 5_000;
/// The weight of a folder.
pub const DIRECTORY_MATCH_DEGREE: i32 = 0;

/// What Go to File looks for.
#[derive(Debug, Clone, Default)]
pub struct GotoQuery<'a> {
    /// The text typed, possibly with a line and column (`app.py:12`).
    pub text: &'a str,
    /// Also the content of excluded folders ("Include non-project items").
    pub non_project: bool,
    /// Files open in editors: they go first among files of one name.
    pub open_files: &'a [PathBuf],
    /// The file of the active editor: files in its folder go next.
    pub context: Option<&'a Path>,
    /// The home folder, for paths typed as `~/...`.
    pub home: Option<&'a Path>,
}

/// The file or folder an absolute path typed in full points to, in the
/// project or not (Go to File finds it first, whatever the scope): the line
/// and column are left out, `~/` stands for `home` (not on Windows), and
/// the filesystem root, a network path or a path that does not exist point
/// nowhere.
pub fn absolute_path(text: &str, home: Option<&Path>) -> Option<PathBuf> {
    if !text.contains(['/', '\\']) {
        return None;
    }
    let path = parse_position(text).pattern;
    let path = path.trim_end();
    let path = match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) if !cfg!(windows) => home.join(rest),
        _ => PathBuf::from(path),
    };
    // A network path could keep the search waiting.
    let network = path.to_string_lossy().starts_with("//")
        || matches!(
            path.components().next(),
            Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
        );
    if !path.is_absolute() || path.parent().is_none() || network {
        return None;
    }
    // The path as the index keeps it: no `.` or `..` parts, no trailing
    // slash.
    let mut normal = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    (normal.parent().is_some() && normal.exists()).then_some(normal)
}

/// A file or folder found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub id: EntryId,
    pub is_dir: bool,
    /// Not project content (inside an excluded folder).
    pub non_project: bool,
    /// Higher first.
    pub weight: i32,
}

/// One name that matched, with its degree.
struct NameMatch {
    name: u32,
    degree: i32,
}

impl FileIndex {
    /// The files and folders `query` finds, in the reference's order before
    /// the final sort by weight (see [`rank`]); `cancelled` is polled.
    pub fn goto_file(&self, query: &GotoQuery, cancelled: &dyn Fn() -> bool) -> Vec<FileHit> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut emit = |hit: FileHit, out: &mut Vec<FileHit>| {
            if seen.insert(hit.id) {
                out.push(hit);
            }
        };
        let text = query.text;
        if let Some(id) = absolute_path(text, query.home).and_then(|p| self.find(&p)) {
            emit(self.hit(id, EXACT_MATCH_DEGREE), &mut out);
        }
        let text = text
            .strip_prefix("./")
            .or_else(|| text.strip_prefix(".\\"))
            .map_or(text.to_owned(), |rest| rest.to_owned());
        // A pattern ending with a slash asks for folders only.
        let folders_only = text.ends_with(['/', '\\']);
        let search = Search {
            index: self,
            query,
            folders_only,
            open: query
                .open_files
                .iter()
                .filter_map(|p| self.find(p))
                .collect(),
            context_dir: query
                .context
                .and_then(|p| self.find(p))
                .map(|id| self.entries[id as usize].parent),
        };
        for hit in search.run(&text, cancelled) {
            emit(hit, &mut out);
        }
        // A pattern typed in another keyboard layout, on the English keys.
        if let Some(fixed) = name_match::fix_layout(&text) {
            for hit in search.run(&fixed, cancelled) {
                emit(hit, &mut out);
            }
        }
        out
    }

    fn hit(&self, id: EntryId, weight: i32) -> FileHit {
        FileHit {
            id,
            is_dir: self.is_dir(id),
            non_project: self.is_excluded(id),
            weight,
        }
    }

    /// The folder path of an entry as Go to File matches it:
    /// `flask/src/flask/` for `flask/src/flask/app.py`.
    pub fn qualifier(&self, id: EntryId) -> String {
        let full = self.full_name(id);
        match full.rfind('/') {
            Some(i) => format!("{}/", &full[..i]),
            None => "/".to_owned(),
        }
    }

    fn visible(&self, id: EntryId, non_project: bool) -> bool {
        let e = &self.entries[id as usize];
        e.alive && id != ROOT && (non_project || !(e.excluded || e.config))
    }
}

/// Orders hits by weight, keeping the order of equal ones.
pub fn rank(hits: &mut [FileHit]) {
    hits.sort_by_key(|h| std::cmp::Reverse(h.weight));
}

/// The matcher of a folder pattern: `*` before it and around each `/`.
pub fn qualifier_matcher(pattern: &str) -> Matcher {
    let pattern = format!("*{}", pattern.replace('\\', "*\\*").replace('/', "*/*"));
    Matcher::new(&pattern, MatchingMode::IgnoreCase).prefer_start()
}

/// The pattern as Go to File matches it: no line and column, `/`
/// separators, no slashes at the ends.
pub fn sanitized_pattern(text: &str) -> String {
    let pattern = parse_position(text).pattern.replace('\\', "/");
    pattern.trim_matches('/').to_owned()
}

struct Search<'a> {
    index: &'a FileIndex,
    query: &'a GotoQuery<'a>,
    folders_only: bool,
    open: HashSet<EntryId>,
    context_dir: Option<EntryId>,
}

impl Search<'_> {
    fn run(&self, text: &str, cancelled: &dyn Fn() -> bool) -> Vec<FileHit> {
        let mut out = Vec::new();
        let sanitized = sanitized_pattern(text);
        let qualifier_end = sanitized.rfind('/').map_or(0, |i| i + 1);
        let name_pattern: Vec<char> = sanitized[qualifier_end..].chars().collect();
        if name_pattern.is_empty() {
            return out;
        }
        let mut grouper = NameGrouper::new(&name_pattern);
        for (id, name) in self.index.names.iter().enumerate() {
            if name
                .entries
                .iter()
                .any(|&e| self.index.visible(e, self.query.non_project))
            {
                grouper.add(id as u32, &name.chars);
            }
        }
        let Some(mut dirs) = DirMatcher::root(self, &sanitized[..qualifier_end]) else {
            return out;
        };
        let complete = parse_position(text).pattern;
        let mut index = 0;
        loop {
            if cancelled() {
                return out;
            }
            let Some(group) = grouper.next_group(self.index, index) else {
                break;
            };
            self.process(&group, &dirs, &complete, &mut out);
            match dirs.append_char(self, name_pattern[index]) {
                Some(next) => dirs = next,
                None => break,
            }
            index += 1;
        }
        out
    }

    /// One round: the names that matched, ranked, their entries emitted.
    fn process(
        &self,
        group: &SuffixGroup,
        dirs: &DirMatcher,
        complete: &str,
        out: &mut Vec<FileHit>,
    ) {
        let idx = self.index;
        let suffix: String = group.suffix.iter().collect();
        let exact_prefix = |m: &NameMatch| idx.names[m.name as usize].text.starts_with(&suffix);
        let degree_order = |a: &NameMatch, b: &NameMatch| match (exact_prefix(a), exact_prefix(b)) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => b.degree.cmp(&a.degree),
        };
        let mut names: Vec<&NameMatch> = group.matches.iter().collect();
        names.sort_by(|a, b| degree_order(a, b));
        let qualifiers = (!dirs.pattern.is_empty()).then(|| qualifier_matcher(&dirs.pattern));
        let base_name = |m: &&NameMatch| {
            let text = &idx.names[m.name as usize].text;
            let stem = text.rfind('.').map_or(text.as_str(), |i| &text[..i]);
            stem.to_lowercase()
        };
        for degree_group in names.chunk_by(|a, b| degree_order(a, b) == Ordering::Equal) {
            let mut by_name = degree_group.to_vec();
            by_name.sort_by_cached_key(base_name);
            let mut files: Vec<(EntryId, i32)> = Vec::new();
            for same in by_name.chunk_by(|a, b| base_name(a) == base_name(b)) {
                let mut items: Vec<(EntryId, i32)> = same
                    .iter()
                    .flat_map(|m| {
                        idx.entries_of(m.name)
                            .iter()
                            .filter(|&&e| {
                                idx.visible(e, self.query.non_project)
                                    && (!self.folders_only || idx.is_dir(e))
                                    && dirs.contains(idx, e)
                            })
                            .map(move |&e| (e, m.degree))
                    })
                    .collect();
                if items.len() > 1 {
                    items.sort_by_cached_key(|&(e, _)| self.proximity_key(e));
                }
                files.extend(items);
            }
            let matched = match &qualifiers {
                None => files,
                Some(matcher) => self.match_qualifiers(matcher, files, complete),
            };
            // Folders after the files of the round, with no weight.
            let (folders, files): (Vec<_>, Vec<_>) =
                matched.into_iter().partition(|&(e, _)| idx.is_dir(e));
            for (e, weight) in files {
                out.push(idx.hit(e, weight));
            }
            for (e, _) in folders {
                out.push(idx.hit(e, DIRECTORY_MATCH_DEGREE));
            }
        }
    }

    /// With a folder part: the files whose folder path matches it, by how
    /// well (minus the unmatched end of the path), a path typed in full
    /// first.
    fn match_qualifiers(
        &self,
        matcher: &Matcher,
        files: Vec<(EntryId, i32)>,
        complete: &str,
    ) -> Vec<(EntryId, i32)> {
        let idx = self.index;
        let typed = complete.trim_end().replace('\\', "/");
        let mut matching = Vec::new();
        for (e, _) in files {
            let qualifier: Vec<char> = idx.qualifier(e).chars().collect();
            let exact = idx.full_name(e).ends_with(&typed);
            let bonus = if exact { EXACT_MATCH_DEGREE } else { 0 };
            match matcher.match_chars(&qualifier) {
                Some(m) => {
                    let gap = m.fragments.last().map_or(0, |f| qualifier.len() - f.end) as i32;
                    matching.push((e, m.degree - gap + bonus));
                }
                None if exact => matching.push((e, EXACT_MATCH_DEGREE)),
                None => {}
            }
        }
        matching.sort_by_key(|m| std::cmp::Reverse(m.1));
        matching
    }

    /// Open files first, then files in the current file's folder, project
    /// files before non-project ones, then shallower ones, then by path.
    fn proximity_key(&self, e: EntryId) -> (bool, bool, bool, usize, String) {
        let idx = self.index;
        let qualifier = idx.qualifier(e);
        let same_dir = self
            .context_dir
            .is_none_or(|d| idx.entries[e as usize].parent == d);
        (
            !self.open.contains(&e),
            !same_dir,
            idx.is_excluded(e),
            qualifier.matches('/').count(),
            idx.full_name(e),
        )
    }
}

/// Names sorted by the longest suffix of the name pattern they could match
/// (they contain its letters in order), so each round tries only names
/// that can match.
struct NameGrouper {
    lower: Vec<char>,
    upper: Vec<char>,
    pattern: Vec<char>,
    buckets: Vec<Vec<u32>>,
}

/// The names of one round.
struct SuffixGroup {
    suffix: Vec<char>,
    matches: Vec<NameMatch>,
}

impl NameGrouper {
    fn new(pattern: &[char]) -> Self {
        Self {
            lower: pattern
                .iter()
                .flat_map(|c| c.to_lowercase().take(1))
                .collect(),
            upper: pattern
                .iter()
                .flat_map(|c| c.to_uppercase().take(1))
                .collect(),
            pattern: pattern.to_vec(),
            buckets: vec![Vec::new(); pattern.len()],
        }
    }

    fn add(&mut self, name: u32, chars: &[char]) {
        let position = self.start_position(chars);
        if position < self.pattern.len() {
            self.buckets[position].push(name);
        }
    }

    /// The smallest `pos` such that `chars` contains the letters and digits
    /// of `pattern[pos..]` in order.
    fn start_position(&self, chars: &[char]) -> usize {
        let mut candidate = chars.len();
        let mut pos = self.pattern.len();
        while pos > 0 {
            let c = self.lower[pos - 1];
            if !c.is_alphanumeric() {
                pos -= 1;
                continue;
            }
            let upper = self.upper[pos - 1];
            let mut found = false;
            while candidate > 0 {
                candidate -= 1;
                let x = chars[candidate];
                if x == c || x == upper {
                    found = true;
                    break;
                }
            }
            if !found {
                break;
            }
            pos -= 1;
        }
        pos
    }

    /// Round `index`: the names that match `pattern[index..]`; the others
    /// try the next, shorter suffix.
    fn next_group(&mut self, index_data: &FileIndex, index: usize) -> Option<SuffixGroup> {
        if index >= self.pattern.len() {
            return None;
        }
        let suffix = self.pattern[index..].to_vec();
        let suffix_text: String = suffix.iter().collect();
        let prefer_start = index == 0 && !suffix_text.starts_with('*');
        let lead = if index > 0 { " " } else { "*" };
        let mut matcher = Matcher::new(&format!("{lead}{suffix_text}"), MatchingMode::IgnoreCase);
        if prefer_start {
            matcher = matcher.prefer_start();
        }
        let names = std::mem::take(&mut self.buckets[index]);
        let mut matches = Vec::new();
        for name in names {
            match matcher.match_chars(&index_data.names[name as usize].chars) {
                Some(m) => matches.push(NameMatch {
                    name,
                    degree: m.degree,
                }),
                None if index + 1 < self.pattern.len() => self.buckets[index + 1].push(name),
                None => {}
            }
        }
        Some(SuffixGroup { suffix, matches })
    }
}

/// The folders a folder pattern matches (`None`: no folder part, the
/// whole project).
struct DirMatcher {
    dirs: Option<Vec<(EntryId, String)>>,
    pattern: String,
}

impl DirMatcher {
    fn root(search: &Search, pattern: &str) -> Option<Self> {
        let mut matcher = Self {
            dirs: None,
            pattern: String::new(),
        };
        for c in pattern.chars() {
            matcher = matcher.append_char(search, c)?;
        }
        Some(matcher)
    }

    /// The folder pattern one char longer: the folders that still match,
    /// or the topmost folders under them that do.
    fn append_char(&self, search: &Search, c: char) -> Option<Self> {
        let pattern = format!("{}{c}", self.pattern);
        if matches!(c, '*' | '/' | ' ') {
            return Some(Self {
                dirs: self.dirs.clone(),
                pattern,
            });
        }
        let idx = search.index;
        let roots = self
            .dirs
            .clone()
            .unwrap_or_else(|| vec![(ROOT, idx.root_name().to_owned())]);
        let matcher = qualifier_matcher(&pattern);
        let has = |text: &str| text.chars().any(|x| x.to_lowercase().eq(c.to_lowercase()));
        let mut next = Vec::new();
        for (dir, full) in roots {
            if has(&full) && matcher.matches(&full) {
                next.push((dir, full));
                continue;
            }
            let mut stack: Vec<(EntryId, String)> = vec![(dir, full)];
            while let Some((d, full)) = stack.pop() {
                for &child in &idx.entries[d as usize].children {
                    if !idx.is_dir(child) || !idx.visible(child, search.query.non_project) {
                        continue;
                    }
                    let name = idx.name(child);
                    let child_full = format!("{full}/{name}");
                    if has(name) && matcher.matches(&child_full) {
                        next.push((child, child_full));
                    } else {
                        stack.push((child, child_full));
                    }
                }
            }
        }
        (!next.is_empty()).then_some(Self {
            dirs: Some(next),
            pattern,
        })
    }

    /// `e` is one of the folders or inside one.
    fn contains(&self, idx: &FileIndex, e: EntryId) -> bool {
        let Some(dirs) = &self.dirs else {
            return true;
        };
        let mut id = e;
        loop {
            if dirs.iter().any(|(d, _)| *d == id) {
                return true;
            }
            if id == ROOT {
                return false;
            }
            id = idx.entries[id as usize].parent;
        }
    }
}
