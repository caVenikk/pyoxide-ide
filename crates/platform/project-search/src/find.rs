//! Searching the files of a scope in parallel.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ignore::{WalkBuilder, WalkState};
use project::{Filter, MAX_FILE_SIZE};
use search::{CancelToken, Matcher};
use text::{Point, Rope};

use crate::mask::FileMask;

/// Lines longer than this (in chars) are kept only around their first
/// match.
pub const MAX_LINE_CHARS: usize = 1000;
/// Chars kept before the first match of a long line.
const LONG_LINE_CONTEXT: usize = 100;

/// Where a search looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// A folder (the project root for In Project), with or without its
    /// subfolders.
    Directory { path: PathBuf, recursive: bool },
    /// Exactly these files (open files, recently viewed files).
    Files(Vec<PathBuf>),
}

/// One search over files.
#[derive(Debug, Clone)]
pub struct FileSearch {
    /// What to look for; `None` lists the files (with a mask and no
    /// query): one result per file, its first line.
    pub matcher: Option<Matcher>,
    /// Only files whose names match (`None`: every file).
    pub mask: Option<FileMask>,
    pub scope: Scope,
    /// The project's root.
    pub root: PathBuf,
    /// The project's hide and exclude rules, as the project tool window
    /// has them; `None`: the rules of `root`, read when the search starts.
    pub filter: Option<Filter>,
    /// The search stops after this many matches.
    pub limit: usize,
    /// Lines shown by an earlier pass of the same search, by file: not
    /// reported or counted again (loading more results).
    pub known: Arc<HashMap<PathBuf, HashSet<usize>>>,
    /// Text of open documents with unsaved changes, searched instead of the
    /// file on disk.
    pub overrides: HashMap<PathBuf, Rope>,
    /// Show the matches in the reference IDE's project configuration files
    /// too (`Filter::is_project_config`). Off by default: they are searched but
    /// their matches are held back ([`Summary::held_back`]), as the
    /// reference does; a mask that names them or a folder inside `.idea`
    /// turns it on by itself.
    pub project_files: bool,
}

/// The matching lines of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatches {
    pub path: PathBuf,
    pub lines: Vec<LineMatch>,
}

impl FileMatches {
    /// Number of matches (a line can hold several).
    pub fn matches(&self) -> usize {
        self.lines.iter().map(|l| l.ranges.len()).sum()
    }
}

/// A line with matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// Zero-based line number.
    pub line: usize,
    /// The line's text without its line break; a long line only around its
    /// first match.
    pub text: String,
    /// Char column of the line where `text` starts (non-zero only for a
    /// shortened long line).
    pub text_column: usize,
    /// The matches as char columns of the line (a match that goes on to
    /// the next lines ends at the end of this one).
    pub ranges: Vec<Range<usize>>,
    /// Where the first match of the line starts and ends (line and char
    /// column; the end may be on a later line), for navigation.
    pub first: Range<Point>,
}

/// What a search did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub files_searched: usize,
    pub files_with_matches: usize,
    pub matches: usize,
    /// The search stopped at the limit; there are more matches.
    pub truncated: bool,
    /// The search was cancelled before it finished.
    pub cancelled: bool,
    /// Files skipped because they are larger than [`MAX_FILE_SIZE`].
    pub large_files: Vec<PathBuf>,
    /// Project configuration files with matches that were not shown.
    pub held_back: usize,
}

/// Shared state of the workers.
struct Run<'a> {
    search: &'a FileSearch,
    filter: Filter,
    /// Project configuration files are searched.
    project_files: bool,
    cancel: &'a CancelToken,
    sink: &'a (dyn Fn(FileMatches) + Sync),
    matches: AtomicUsize,
    files_searched: AtomicUsize,
    files_with_matches: AtomicUsize,
    truncated: AtomicBool,
    large: Mutex<Vec<PathBuf>>,
    held_back: AtomicUsize,
    /// Matches handed to the sink.
    reported: AtomicUsize,
}

impl FileSearch {
    /// Searches the scope on all cores and hands every file with matches to
    /// `sink` as soon as it is searched (from the worker threads, in no
    /// particular order). Blocking; stops early when `cancel` is set or the
    /// limit is reached.
    pub fn run(&self, cancel: &CancelToken, sink: &(dyn Fn(FileMatches) + Sync)) -> Summary {
        let filter = self
            .filter
            .clone()
            .unwrap_or_else(|| Filter::for_root(&self.root));
        let project_files = self.searches_project_files(&filter);
        let run = Run {
            search: self,
            filter,
            project_files,
            cancel,
            sink,
            matches: AtomicUsize::new(0),
            files_searched: AtomicUsize::new(0),
            files_with_matches: AtomicUsize::new(0),
            truncated: AtomicBool::new(false),
            large: Mutex::new(Vec::new()),
            held_back: AtomicUsize::new(0),
            reported: AtomicUsize::new(0),
        };
        match &self.scope {
            Scope::Directory { path, recursive } => run.walk(path, *recursive),
            Scope::Files(files) => run.files(files),
        }
        Summary {
            files_searched: run.files_searched.load(Ordering::Relaxed),
            files_with_matches: run.files_with_matches.load(Ordering::Relaxed),
            matches: run.reported.load(Ordering::Relaxed),
            truncated: run.truncated.load(Ordering::Relaxed),
            cancelled: cancel.is_cancelled(),
            large_files: run.large.into_inner().unwrap_or_default(),
            held_back: run.held_back.load(Ordering::Relaxed),
        }
    }

    /// Whether matches in project configuration files are shown: when asked
    /// to, when a mask ends in `.iml`, `.ipr` or `.iws`, or when the folder
    /// searched is in `.idea` (the reference's rules).
    pub fn searches_project_files(&self, filter: &Filter) -> bool {
        if self.project_files {
            return true;
        }
        let named = self.mask.as_ref().is_some_and(|m| {
            m.text()
                .split(',')
                .any(|m| [".iml", ".ipr", ".iws"].iter().any(|e| m.ends_with(e)))
        });
        let inside = match &self.scope {
            Scope::Directory { path, .. } => filter.is_project_config(path),
            Scope::Files(_) => false,
        };
        named || inside
    }
}

impl Run<'_> {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled() || self.truncated.load(Ordering::Relaxed)
    }

    /// The files of a folder that are project content: the project's hidden
    /// names and excluded entries ([`Filter`]) are left out and nothing else
    /// (ignore files such as `.gitignore` are not read here), symbolic links
    /// are not followed. A folder that is itself excluded, or inside an
    /// excluded one, is searched with its excluded entries, as the reference
    /// does when asked to search such a folder.
    fn walk(&self, dir: &Path, recursive: bool) {
        let mut builder = WalkBuilder::new(dir);
        builder.standard_filters(false).follow_links(false);
        if !recursive {
            builder.max_depth(Some(1));
        }
        let filter = self.filter.clone();
        let skip_excluded = !dir
            .ancestors()
            .take_while(|a| a.starts_with(&self.search.root) && *a != self.search.root)
            .any(|a| filter.is_excluded(a, true));
        builder.filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if filter.is_hidden(&name) {
                return false;
            }
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            !(skip_excluded && filter.is_excluded(entry.path(), is_dir))
        });
        builder.build_parallel().run(|| {
            Box::new(|entry| {
                if self.stopped() {
                    return WalkState::Quit;
                }
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    self.file(entry.path());
                }
                if self.stopped() {
                    WalkState::Quit
                } else {
                    WalkState::Continue
                }
            })
        });
    }

    /// A list of files, split among worker threads.
    fn files(&self, files: &[PathBuf]) {
        let next = AtomicUsize::new(0);
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(files.len().max(1));
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= files.len() || self.stopped() {
                            break;
                        }
                        self.file(&files[i]);
                    }
                });
            }
        });
    }

    fn file(&self, path: &Path) {
        if let Some(mask) = &self.search.mask {
            let name = path.file_name().map(|n| n.to_string_lossy());
            if !name.is_some_and(|n| mask.matches(&n)) {
                return;
            }
        }
        let Some(text) = self.text(path) else {
            return;
        };
        self.files_searched.fetch_add(1, Ordering::Relaxed);
        let mut ranges: Vec<Range<usize>> = Vec::new();
        match &self.search.matcher {
            Some(matcher) => {
                for (i, m) in matcher.find_iter(&text).enumerate() {
                    if i % 1024 == 1023 && self.cancel.is_cancelled() {
                        return;
                    }
                    ranges.push(m);
                }
            }
            // The whole file is one result, shown by its first line.
            None => {
                let first = text.find('\n').unwrap_or(text.len());
                let first = text[..first].strip_suffix('\r').map_or(first, str::len);
                ranges.push(0..first);
            }
        }
        if ranges.is_empty() {
            return;
        }
        if !self.project_files && self.filter.is_project_config(path) {
            self.held_back.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // The limit counts matches over all files; lines shown by an
        // earlier pass count no more.
        let limit = self.search.limit;
        let mut before = self.matches.load(Ordering::Acquire);
        if before >= limit {
            self.truncated.store(true, Ordering::Relaxed);
            return;
        }
        let known = self.search.known.get(path);
        let (mut lines, mut cut) = collect_lines(&text, &ranges, known, limit - before);
        // Takes this file's share of the limit; when other files took
        // theirs meanwhile, fewer lines fit.
        let kept = loop {
            cut |= keep_within(&mut lines, limit - before);
            let kept: usize = lines.iter().map(|l| l.ranges.len()).sum();
            match self.matches.compare_exchange(
                before,
                before + kept,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break kept,
                Err(now) if now >= limit => {
                    self.truncated.store(true, Ordering::Relaxed);
                    return;
                }
                Err(now) => before = now,
            }
        };
        if kept == 0 {
            return;
        }
        if cut || before + kept >= limit {
            self.truncated.store(true, Ordering::Relaxed);
        }
        if self.cancel.is_cancelled() {
            return;
        }
        self.files_with_matches.fetch_add(1, Ordering::Relaxed);
        self.reported.fetch_add(kept, Ordering::Relaxed);
        (self.sink)(FileMatches {
            path: path.to_owned(),
            lines,
        });
    }

    /// The text to search: an open document's, else the file's (binary and
    /// over-large files are skipped).
    fn text(&self, path: &Path) -> Option<std::borrow::Cow<'static, str>> {
        if let Some(rope) = self.search.overrides.get(path) {
            let mut text = String::with_capacity(rope.len_bytes());
            for chunk in rope.chunks() {
                text.push_str(chunk);
            }
            return Some(text.into());
        }
        let meta = std::fs::metadata(path).ok()?;
        if meta.len() > MAX_FILE_SIZE {
            if let Ok(mut large) = self.large.lock() {
                large.push(path.to_owned());
            }
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        file_text(bytes)
    }
}

/// The text of a file's bytes: UTF-8 as it is (line breaks of any kind are
/// matched by the queries), anything else through the text model's
/// decoding; `None` for binary files.
pub fn file_text(bytes: Vec<u8>) -> Option<std::borrow::Cow<'static, str>> {
    let probe = &bytes[..bytes.len().min(8 * 1024)];
    let bom = bytes.starts_with(b"\xEF\xBB\xBF")
        || bytes.starts_with(b"\xFF\xFE")
        || bytes.starts_with(b"\xFE\xFF");
    if !bom && !probe.contains(&0) {
        match String::from_utf8(bytes) {
            Ok(text) => return Some(text.into()),
            Err(e) => return text::decode(e.as_bytes()).ok().map(|d| d.text.into()),
        }
    }
    text::decode(&bytes).ok().map(|d| d.text.into())
}

/// Groups matches (sorted byte ranges of `text`) by line: the line's text,
/// its number and the matches as char columns. Line breaks may be `\n` or
/// `\r\n`.
pub fn line_matches(text: &str, ranges: &[Range<usize>]) -> Vec<LineMatch> {
    collect_lines(text, ranges, None, usize::MAX).0
}

/// [`line_matches`] leaving out the `known` lines and stopping before the
/// line that would start past `budget` matches; also says whether it
/// stopped early.
fn collect_lines(
    text: &str,
    ranges: &[Range<usize>],
    known: Option<&HashSet<usize>>,
    budget: usize,
) -> (Vec<LineMatch>, bool) {
    let mut kept = 0usize;
    let bytes = text.as_bytes();
    let mut out: Vec<LineMatch> = Vec::new();
    // Line number at `pos`, counted forwards.
    let (mut pos, mut line) = (0usize, 0usize);
    let mut last_start = None;
    // The current line is a known one.
    let mut skipping = false;
    for r in ranges {
        line += bytes[pos..r.start].iter().filter(|&&b| b == b'\n').count();
        pos = r.start;
        let start = bytes[..r.start]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let mut end = bytes[r.start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| r.start + i);
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        let same_line = last_start == Some(start);
        if same_line && skipping {
            continue;
        }
        if !same_line {
            skipping = known.is_some_and(|k| k.contains(&line));
            if skipping {
                last_start = Some(start);
                continue;
            }
        }
        if kept >= budget {
            return (out, true);
        }
        kept += 1;
        let col = |byte: usize| text[start..byte.clamp(start, end)].chars().count();
        let first_col = col(r.start);
        let range = first_col..col(r.end).max(first_col);
        if same_line && let Some(last) = out.last_mut() {
            last.ranges.push(range);
            continue;
        }
        last_start = Some(start);
        // The end of the match as a point: on a later line when it spans
        // line breaks.
        let matched = &text[r.start..r.end];
        let breaks = matched.bytes().filter(|&b| b == b'\n').count();
        let end_point = match matched.rfind('\n') {
            Some(i) => Point::new(line + breaks, matched[i + 1..].chars().count()),
            None => Point::new(line, range.end),
        };
        let (text_column, shown) = shorten(&text[start..end], first_col);
        out.push(LineMatch {
            line,
            text: shown,
            text_column,
            ranges: vec![range],
            first: Point::new(line, first_col)..end_point,
        });
    }
    (out, false)
}

/// Keeps the first `budget` matches of the lines; says whether any were
/// dropped.
fn keep_within(lines: &mut Vec<LineMatch>, budget: usize) -> bool {
    let mut left = budget;
    let mut dropped = false;
    lines.retain_mut(|l| {
        if left == 0 {
            dropped = true;
            return false;
        }
        if l.ranges.len() > left {
            l.ranges.truncate(left);
            dropped = true;
        }
        left -= l.ranges.len();
        true
    });
    dropped
}

/// A line's text for display: the whole line, or for a long one the part
/// around `first` (a char column), with the column where it starts.
fn shorten(line: &str, first: usize) -> (usize, String) {
    let len = line.chars().count();
    if len <= MAX_LINE_CHARS {
        return (0, line.to_owned());
    }
    let from = first.saturating_sub(LONG_LINE_CONTEXT);
    let text = line.chars().skip(from).take(MAX_LINE_CHARS).collect();
    (from, text)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // expected ranges
mod tests {
    use std::collections::BTreeMap;

    use search::{Query, SearchOptions};

    use super::*;

    fn matcher(text: &str, options: SearchOptions) -> Matcher {
        Query::new(text, options).compile().unwrap()
    }

    fn plain(text: &str) -> Matcher {
        matcher(text, SearchOptions::default())
    }

    /// Runs a search and returns the matching lines by relative path.
    fn run(search: &FileSearch) -> (BTreeMap<String, Vec<LineMatch>>, Summary) {
        let found = Mutex::new(BTreeMap::new());
        let summary = search.run(&CancelToken::new(), &|m: FileMatches| {
            let rel = m
                .path
                .strip_prefix(&search.root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            found.lock().unwrap().insert(rel, m.lines);
        });
        (found.into_inner().unwrap(), summary)
    }

    fn project(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, bytes) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        dir
    }

    fn search_in(dir: &Path, matcher: Matcher) -> FileSearch {
        FileSearch {
            matcher: Some(matcher),
            mask: None,
            scope: Scope::Directory {
                path: dir.to_owned(),
                recursive: true,
            },
            root: dir.to_owned(),
            filter: None,
            limit: 1000,
            known: Arc::default(),
            overrides: HashMap::new(),
            project_files: false,
        }
    }

    #[test]
    fn lines_group_matches_with_char_columns() {
        let text = "ä x x\r\nno\r\n  x end\nlast x";
        let m = plain("x");
        let ranges: Vec<_> = m.find_iter(text).collect();
        let lines = line_matches(text, &ranges);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].line, 0);
        assert_eq!(lines[0].text, "ä x x");
        assert_eq!(lines[0].ranges, vec![2..3, 4..5]);
        assert_eq!(lines[0].first, Point::new(0, 2)..Point::new(0, 3));
        assert_eq!(lines[1].line, 2);
        assert_eq!(lines[1].text, "  x end");
        assert_eq!(lines[1].ranges, vec![2..3]);
        assert_eq!(lines[2].line, 3);
        assert_eq!(lines[2].ranges, vec![5..6]);
        // A match over a line break ends at the end of its first line.
        let m = plain("x\nl");
        let ranges: Vec<_> = m.find_iter("a\nend x\nlast").collect();
        let lines = line_matches("a\nend x\nlast", &ranges);
        assert_eq!(lines[0].line, 1);
        assert_eq!(lines[0].ranges, vec![4..5]);
        assert_eq!(lines[0].first, Point::new(1, 4)..Point::new(2, 1));
    }

    #[test]
    fn long_lines_are_kept_around_their_match() {
        let line = format!("{}needle{}", "a".repeat(5000), "b".repeat(5000));
        let m = plain("needle");
        let ranges: Vec<_> = m.find_iter(&line).collect();
        let lines = line_matches(&line, &ranges);
        assert_eq!(lines[0].text_column, 4900);
        assert_eq!(lines[0].text.chars().count(), MAX_LINE_CHARS);
        assert!(lines[0].text[100..].starts_with("needle"));
        assert_eq!(lines[0].ranges, vec![5000..5006]);
    }

    #[test]
    fn masks_hidden_and_excluded_entries() {
        let dir = project(&[
            ("a.py", b"needle\n"),
            ("b.txt", b"needle\n"),
            ("test_a.py", b"needle\n"),
            ("pkg/c.py", b"x\nneedle needle\n"),
            (".git/config", b"needle\n"),
            (".venv/pyvenv.cfg", b"home = /usr\n"),
            (".venv/lib/e.py", b"needle\n"),
            (".github/ci.yml", b"needle\n"),
            ("pkg/__pycache__/c.py", b"needle\n"),
            ("pkg/c.pyc", b"needle\n"),
            // Ignore files of other tools are not exclusions.
            ("pkg/.ignore", b"skip.py\n"),
            ("pkg/skip.py", b"needle\n"),
        ]);
        let mut search = search_in(dir.path(), plain("needle"));
        // The project excludes `.venv` (as the Python product does on its
        // own).
        let mut excluded = project::Exclusions::default();
        excluded.add(".venv");
        search.filter = Some(Filter::new(
            dir.path(),
            excluded,
            project::AutoExclusions::NONE,
        ));
        let (found, summary) = run(&search);
        let files: Vec<_> = found.keys().cloned().collect();
        assert_eq!(
            files,
            [
                ".github/ci.yml",
                "a.py",
                "b.txt",
                "pkg/c.py",
                "pkg/skip.py",
                "test_a.py"
            ]
        );
        assert_eq!(found["pkg/c.py"][0].line, 1);
        assert_eq!(found["pkg/c.py"][0].ranges.len(), 2);
        assert_eq!(summary.matches, 7);
        assert_eq!(summary.files_with_matches, 6);
        assert!(!summary.truncated);
        // With a mask.
        search.mask = Some(FileMask::parse("*.py, !test_*, !skip*").unwrap());
        let (found, _) = run(&search);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["a.py", "pkg/c.py"]);
        // Not recursive.
        search.mask = None;
        search.scope = Scope::Directory {
            path: dir.path().join("pkg"),
            recursive: false,
        };
        let (found, _) = run(&search);
        assert_eq!(
            found.keys().collect::<Vec<_>>(),
            ["pkg/c.py", "pkg/skip.py"]
        );
        // An excluded folder searched on its own shows its content.
        search.scope = Scope::Directory {
            path: dir.path().join(".venv"),
            recursive: true,
        };
        let (found, _) = run(&search);
        assert_eq!(found.keys().collect::<Vec<_>>(), [".venv/lib/e.py"]);
    }

    #[test]
    fn project_configuration_files_are_held_back() {
        let dir = project(&[
            ("a.py", b"needle\n"),
            ("app.iml", b"needle\n"),
            (".idea/workspace.xml", b"needle\n"),
            (".idea/inspectionProfiles/p.xml", b"needle\n"),
        ]);
        let mut search = search_in(dir.path(), plain("needle"));
        let (found, summary) = run(&search);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["a.py"]);
        assert_eq!(summary.held_back, 3, "searched, matches held back");
        // Asked for.
        search.project_files = true;
        let (found, _) = run(&search);
        assert_eq!(found.len(), 4);
        // A mask naming module files.
        search.project_files = false;
        search.mask = Some(FileMask::parse("*.py,*.iml").unwrap());
        let (found, _) = run(&search);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["a.py", "app.iml"]);
        // A folder in `.idea`.
        search.mask = None;
        search.scope = Scope::Directory {
            path: dir.path().join(".idea"),
            recursive: true,
        };
        let (found, _) = run(&search);
        assert_eq!(
            found.keys().collect::<Vec<_>>(),
            [".idea/inspectionProfiles/p.xml", ".idea/workspace.xml"]
        );
        let filter = Filter::for_root(dir.path());
        assert!(filter.is_project_config(&dir.path().join(".idea/a.xml")));
        assert!(filter.is_project_config(&dir.path().join("m.IML")));
        assert!(!filter.is_project_config(&dir.path().join("idea/a.xml")));
    }

    #[test]
    fn binary_large_and_encoded_files() {
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "needle\r\n".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let dir = project(&[
            ("bin.dat", b"needle\0\x01"),
            ("utf16.txt", &utf16),
            ("latin1.txt", b"caf\xe9 needle"),
            ("crlf.txt", b"one\r\nneedle\r\n"),
        ]);
        let big = dir.path().join("big.txt");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_FILE_SIZE + 1).unwrap();
        let (found, summary) = run(&search_in(dir.path(), plain("needle")));
        assert_eq!(
            found.keys().collect::<Vec<_>>(),
            ["crlf.txt", "latin1.txt", "utf16.txt"]
        );
        assert_eq!(found["crlf.txt"][0].line, 1);
        assert_eq!(found["crlf.txt"][0].text, "needle");
        assert_eq!(found["latin1.txt"][0].ranges, vec![5..11]);
        assert_eq!(summary.large_files, vec![big]);
    }

    #[test]
    fn open_documents_win_over_the_disk_copy() {
        let dir = project(&[("a.py", b"old needle\n"), ("b.py", b"needle\n")]);
        let mut search = search_in(dir.path(), plain("needle"));
        search.overrides.insert(
            dir.path().join("a.py"),
            Rope::from_str("new\n\nneedle here"),
        );
        let (found, _) = run(&search);
        assert_eq!(found["a.py"][0].line, 2);
        assert_eq!(found["a.py"][0].text, "needle here");
        // A file list scope.
        search.scope = Scope::Files(vec![dir.path().join("b.py")]);
        let (found, _) = run(&search);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["b.py"]);
    }

    #[test]
    fn the_limit_stops_the_search() {
        let files: Vec<(String, Vec<u8>)> = (0..50)
            .map(|i| (format!("f{i}.txt"), b"x x x\n".to_vec()))
            .collect();
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        let dir = project(&refs);
        let mut search = search_in(dir.path(), plain("x"));
        search.limit = 10;
        let (found, summary) = run(&search);
        let total: usize = found
            .values()
            .map(|l| l.iter().map(|l| l.ranges.len()).sum::<usize>())
            .sum();
        assert_eq!(total, 10);
        assert!(summary.truncated);
        assert_eq!(summary.matches, 10);
        // Loading more: the lines shown are not counted again.
        let known: HashMap<PathBuf, HashSet<usize>> = found
            .keys()
            .map(|rel| (dir.path().join(rel), HashSet::from([0])))
            .collect();
        search.known = Arc::new(known);
        search.limit = 6;
        let (more, summary) = run(&search);
        assert_eq!(summary.matches, 6);
        assert!(more.keys().all(|k| !found.contains_key(k)));
        search.known = Arc::default();
        search.limit = 1000;
        let (_, summary) = run(&search);
        assert_eq!(summary.matches, 150);
        assert!(!summary.truncated);
    }

    #[test]
    fn without_a_query_the_files_of_the_mask_are_listed() {
        let dir = project(&[
            ("a.py", b"first line\r\nsecond\n"),
            ("b.py", b""),
            ("c.txt", b"text\n"),
        ]);
        let mut search = search_in(dir.path(), plain("x"));
        search.matcher = None;
        search.mask = Some(FileMask::parse("*.py").unwrap());
        let (found, summary) = run(&search);
        assert_eq!(found.keys().collect::<Vec<_>>(), ["a.py", "b.py"]);
        assert_eq!(found["a.py"][0].text, "first line");
        assert_eq!(found["a.py"][0].ranges, vec![0..10]);
        assert_eq!(summary.matches, 2);
    }

    #[test]
    fn cancelling_stops_at_once() {
        let dir = project(&[("a.txt", b"x\n"), ("b.txt", b"x\n")]);
        let search = search_in(dir.path(), plain("x"));
        let cancel = CancelToken::new();
        cancel.cancel();
        let calls = AtomicUsize::new(0);
        let summary = search.run(&cancel, &|_| {
            calls.fetch_add(1, Ordering::Relaxed);
        });
        assert!(summary.cancelled);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn options_apply_per_file() {
        let dir = project(&[("a.py", b"Foo foo food\n")]);
        let words = SearchOptions {
            case_sensitive: true,
            whole_words: true,
            regex: false,
        };
        let (found, _) = run(&search_in(dir.path(), matcher("foo", words)));
        assert_eq!(found["a.py"][0].ranges, vec![4..7]);
        let regex = SearchOptions {
            regex: true,
            ..SearchOptions::default()
        };
        let (found, _) = run(&search_in(dir.path(), matcher(r"fo+\b", regex)));
        assert_eq!(found["a.py"][0].ranges, vec![0..3, 4..7]);
    }
}
