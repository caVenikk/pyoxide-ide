//! Links in the terminal output, worked out from a snapshot without the UI
//! toolkit (the reference's terminal hyperlinks, `plugins/terminal`:
//! `FrontendTerminalHyperlinksProcessing`, `FrontendOsc8HyperlinksProcessing`).
//!
//! Two kinds:
//! - *visible* links, underlined all the time and opened by a plain click:
//!   URLs (the console's [`UrlFilter`]), OSC 8 hyperlinks, and what the
//!   product's [`LinkFilter`]s find (the Python product: tracebacks);
//! - *invisible* links, the generic file links: any text that names a file
//!   or a folder (`src/app.py:3:5`, `/tmp`, `~/notes`), underlined only
//!   while Cmd (Ctrl) is held over it and opened by Cmd/Ctrl+click. The
//!   path is read from the output under the pointer and checked against the
//!   file system by the view, off the UI thread.
//!
//! A link can run over several rows when the line soft-wraps: a screen row
//! whose last cell carries the wrap flag continues on the next one.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use console::{Link, LinkFilter, LinkTarget, UrlFilter};
use terminal::{CellFlags, Point, Snapshot};

/// What a product adds to the terminal's links: the filters for a session
/// whose shell works in the given folder (the Python product links
/// tracebacks, which name files relative to it).
pub type LinkFilters = Arc<dyn Fn(Option<PathBuf>) -> Vec<Arc<dyn LinkFilter>> + Send + Sync>;

/// A link on the screen: the cells of its text, row by row, and where it
/// leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridLink {
    /// `(row, columns)` of each stretch of the link; one per row it covers.
    pub segments: Vec<(usize, Range<usize>)>,
    pub target: LinkTarget,
    /// Underlined all the time (a URL, an OSC 8 link, what a filter found),
    /// not only while Cmd (Ctrl) is held (a path found in the text).
    pub visible: bool,
}

impl GridLink {
    /// The cell is on the link.
    pub fn contains(&self, point: Point) -> bool {
        self.segments
            .iter()
            .any(|(line, columns)| *line == point.line && columns.contains(&point.column))
    }

    /// The columns of the link on `line`.
    pub fn columns_on(&self, line: usize) -> impl Iterator<Item = Range<usize>> + '_ {
        self.segments
            .iter()
            .filter(move |(l, _)| *l == line)
            .map(|(_, c)| c.clone())
    }

    /// The link's text as a short description for tests and the state.
    pub fn describe(&self) -> String {
        match &self.target {
            LinkTarget::Url(url) => url.clone(),
            LinkTarget::File { path, line, column } => {
                let mut out = path.display().to_string();
                if let Some(line) = line {
                    out.push_str(&format!(":{line}"));
                    if let Some(column) = column {
                        out.push_str(&format!(":{column}"));
                    }
                }
                out
            }
        }
    }
}

/// A line of the output, rows joined across soft wraps, with where each of
/// its characters sits on the screen.
#[derive(Clone, Debug, Default)]
pub struct LogicalLine {
    pub text: String,
    /// Per character: its row, first column and width in cells.
    cells: Vec<(usize, usize, usize)>,
    /// Per character: its byte offset in `text`, and the text's length at
    /// the end.
    offsets: Vec<usize>,
}

impl LogicalLine {
    /// The character at a cell, as an index into the line's characters.
    fn char_at(&self, point: Point) -> Option<usize> {
        self.cells.iter().position(|&(line, column, width)| {
            line == point.line && (column..column + width).contains(&point.column)
        })
    }

    /// The character index of a byte offset.
    fn char_of_byte(&self, byte: usize) -> usize {
        self.offsets.partition_point(|&o| o < byte)
    }

    /// The cells of the characters in a byte range of the text, as
    /// segments per row.
    pub fn segments(&self, bytes: Range<usize>) -> Vec<(usize, Range<usize>)> {
        let first = self.char_of_byte(bytes.start);
        let last = self.char_of_byte(bytes.end);
        let mut out: Vec<(usize, Range<usize>)> = Vec::new();
        for &(line, column, width) in
            &self.cells[first.min(self.cells.len())..last.min(self.cells.len())]
        {
            match out.last_mut() {
                Some((l, columns)) if *l == line && columns.end == column => {
                    columns.end = column + width
                }
                _ => out.push((line, column..column + width)),
            }
        }
        out
    }

    /// The row and columns that make up the first row of the line (for
    /// positions of text found in it).
    pub fn first_row(&self) -> Option<usize> {
        self.cells.first().map(|c| c.0)
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Joins the visible rows into lines of output: a row continues on the
/// next when its last cell has the wrap flag. Trailing blanks of a line are
/// dropped.
pub fn logical_lines(snapshot: &Snapshot) -> Vec<LogicalLine> {
    let mut out = Vec::new();
    let mut current = LogicalLine::default();
    let mut blanks: Vec<(char, (usize, usize, usize))> = Vec::new();
    let finish = |current: &mut LogicalLine, out: &mut Vec<LogicalLine>| {
        if !current.text.is_empty() {
            out.push(std::mem::take(current));
        } else {
            *current = LogicalLine::default();
        }
    };
    for line in 0..snapshot.rows {
        let row = snapshot.row(line);
        let mut column = 0;
        while column < row.len() {
            let cell = &row[column];
            if cell
                .flags
                .intersects(CellFlags::WIDE_SPACER | CellFlags::LEADING_WIDE_SPACER)
            {
                column += 1;
                continue;
            }
            let width = if cell.flags.contains(CellFlags::WIDE) {
                2
            } else {
                1
            };
            let c = if cell.c == '\0' || cell.flags.contains(CellFlags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            if c == ' ' {
                blanks.push((c, (line, column, width)));
            } else {
                for (b, at) in blanks.drain(..) {
                    current.offsets.push(current.text.len());
                    current.text.push(b);
                    current.cells.push(at);
                }
                current.offsets.push(current.text.len());
                current.text.push(c);
                current.cells.push((line, column, width));
            }
            column += width;
        }
        let wraps = row
            .last()
            .is_some_and(|c| c.flags.contains(CellFlags::WRAPLINE));
        if wraps {
            // Blanks at the end of a wrapped row belong to the text.
            for (b, at) in blanks.drain(..) {
                current.offsets.push(current.text.len());
                current.text.push(b);
                current.cells.push(at);
            }
        } else {
            blanks.clear();
            finish(&mut current, &mut out);
        }
    }
    blanks.clear();
    finish(&mut current, &mut out);
    out
}

/// What [`scan`] found.
#[derive(Debug, Default)]
pub struct Scan {
    pub links: Vec<GridLink>,
    /// Lines the product's filters have not looked at yet (they may read
    /// the file system, so the view runs them off the UI thread).
    pub unfiltered: Vec<String>,
}

/// The visible links of a snapshot: URLs, OSC 8 hyperlinks and the results
/// the product's filters gave for the lines in `filtered` (keyed by line).
pub fn scan(
    snapshot: &Snapshot,
    lines: &[LogicalLine],
    filtered: &HashMap<String, Vec<Link>>,
    has_filters: bool,
) -> Scan {
    let mut out = Scan::default();
    for line in lines {
        let mut found: Vec<Link> = Vec::new();
        if has_filters {
            match filtered.get(&line.text) {
                Some(links) => found.extend(links.iter().cloned()),
                None if may_have_link(&line.text) => out.unfiltered.push(line.text.clone()),
                None => {}
            }
        }
        UrlFilter.links(&line.text, &mut found);
        // The first link at a place wins: the product's before the URL
        // filter's (a traceback path is also a `file:` text sometimes).
        let mut kept: Vec<Link> = Vec::new();
        for link in found {
            if kept
                .iter()
                .all(|k| k.range.end <= link.range.start || link.range.end <= k.range.start)
            {
                kept.push(link);
            }
        }
        kept.sort_by_key(|l| l.range.start);
        for link in kept {
            let segments = line.segments(link.range);
            if !segments.is_empty() {
                out.links.push(GridLink {
                    segments,
                    target: link.target,
                    visible: true,
                });
            }
        }
    }
    osc8_links(snapshot, &mut out.links);
    out
}

/// A line the product's filters could link: it has a path-like or file
/// word in it. Cheap, so that plain output costs no filter run.
fn may_have_link(line: &str) -> bool {
    line.contains(['/', '\\', '.']) && line.trim().len() > 2
}

/// OSC 8 hyperlinks: runs of cells with the same link.
fn osc8_links(snapshot: &Snapshot, out: &mut Vec<GridLink>) {
    let mut open: Option<GridLink> = None;
    // `extras` is in screen order.
    let mut last: Option<(&str, &str, Point)> = None;
    for extra in &snapshot.extras {
        let Some(h) = &extra.hyperlink else {
            continue;
        };
        let point = extra.point;
        let width = if snapshot.cell(point).flags.contains(CellFlags::WIDE) {
            2
        } else {
            1
        };
        let continues = last.is_some_and(|(id, uri, p)| {
            id == h.id
                && uri == h.uri
                && ((p.line == point.line && p.column + 1 == point.column)
                    || (p.line + 1 == point.line && point.column == 0))
        });
        if !continues {
            out.extend(open.take());
            let mut urls = Vec::new();
            UrlFilter.links(&h.uri, &mut urls);
            let target = match urls.into_iter().find(|l| l.range == (0..h.uri.len())) {
                Some(link) => link.target,
                None => LinkTarget::Url(h.uri.clone()),
            };
            open = Some(GridLink {
                segments: Vec::new(),
                target,
                visible: true,
            });
        }
        if let Some(link) = &mut open {
            match link.segments.last_mut() {
                Some((l, columns)) if *l == point.line && columns.end == point.column => {
                    columns.end = point.column + width
                }
                _ => link
                    .segments
                    .push((point.line, point.column..point.column + width)),
            }
        }
        last = Some((&h.id, &h.uri, point));
    }
    out.extend(open);
}

/// The pieces of a path-like word: the path as written, and a line and a
/// column after it (`app.py:3:5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathToken {
    pub path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// The byte range of the whole word (suffix included) in the line.
    pub range: Range<usize>,
}

/// Characters that end a word of a path: blanks, quotes, brackets and
/// the separators of lists.
fn ends_path(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | '<'
                | '>'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '|'
                | ','
                | ';'
                | '='
        )
}

/// The word of the line under the character at `point`, as a path with an
/// optional `:line[:column]`.
pub fn path_token_at(line: &LogicalLine, point: Point) -> Option<PathToken> {
    let at = line.char_at(point)?;
    let chars: Vec<char> = line.text.chars().collect();
    if ends_path(chars[at]) {
        return None;
    }
    let mut start = at;
    while start > 0 && !ends_path(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && !ends_path(chars[end]) {
        end += 1;
    }
    // Trailing punctuation of a sentence is not part of the path.
    while end > start + 1 && matches!(chars[end - 1], '.' | ':' | '!' | '?') {
        end -= 1;
    }
    let word: String = chars[start..end].iter().collect();
    let (path, line_no, column) = split_position(&word);
    if path.is_empty() {
        return None;
    }
    let byte_start = line.offsets[start];
    let byte_end = line.offsets.get(end).copied().unwrap_or(line.text.len());
    Some(PathToken {
        path: path.to_owned(),
        line: line_no,
        column,
        range: byte_start..byte_end,
    })
}

/// Splits `path:line:column`; a `:` followed by something that is not a
/// number stays in the path (`C:\work`).
fn split_position(word: &str) -> (&str, Option<u32>, Option<u32>) {
    fn number(s: &str) -> Option<u32> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
            .filter(|n| *n > 0)
    }
    let Some((rest, last)) = word.rsplit_once(':') else {
        return (word, None, None);
    };
    let Some(last) = number(last) else {
        return (word, None, None);
    };
    if let Some((path, pre)) = rest.rsplit_once(':')
        && let Some(pre) = number(pre)
        && !path.is_empty()
    {
        return (path, Some(pre), Some(last));
    }
    if rest.is_empty() {
        return (word, None, None);
    }
    (rest, Some(last), None)
}

/// Where a path written in the output points: `~` is the home folder, a
/// relative path is taken from the shell's working directory.
pub fn resolve_path(path: &str, working_directory: Option<&Path>) -> Option<PathBuf> {
    let expanded = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            let mut home = std::env::home_dir()?;
            let rest = rest.trim_start_matches(['/', '\\']);
            if !rest.is_empty() {
                home.push(rest);
            }
            home
        }
        _ => PathBuf::from(path),
    };
    if expanded.is_absolute() {
        return Some(expanded);
    }
    Some(working_directory?.join(expanded))
}

/// The link for a path word, when what it names exists: a file with its
/// line and column, or a folder. Reads the file system, so not for the UI
/// thread.
pub fn file_link(token: &PathToken, working_directory: Option<&Path>) -> Option<LinkTarget> {
    let path = resolve_path(&token.path, working_directory)?;
    let meta = std::fs::metadata(&path).ok()?;
    let normal = normalize(&path);
    if meta.is_dir() {
        return Some(LinkTarget::File {
            path: normal,
            line: None,
            column: None,
        });
    }
    Some(LinkTarget::File {
        path: normal,
        line: token.line,
        column: token.column,
    })
}

/// A path without `.` and `..` parts (without touching the file system).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal::{Cell, Hyperlink};

    fn snapshot(rows: &[&str], columns: usize) -> Snapshot {
        let mut s = Snapshot {
            columns,
            rows: rows.len(),
            ..Snapshot::default()
        };
        for row in rows {
            let mut cells: Vec<Cell> = row
                .chars()
                .map(|c| Cell {
                    c,
                    ..Cell::default()
                })
                .collect();
            cells.resize(columns, Cell::default());
            s.cells.extend(cells);
        }
        s
    }

    fn wrap(s: &mut Snapshot, line: usize) {
        let last = line * s.columns + s.columns - 1;
        s.cells[last].flags.insert(CellFlags::WRAPLINE);
    }

    fn link_texts(s: &Snapshot, filtered: &HashMap<String, Vec<Link>>) -> Vec<(String, String)> {
        let lines = logical_lines(s);
        let scan = scan(s, &lines, filtered, true);
        scan.links
            .iter()
            .map(|l| {
                let text: String = l
                    .segments
                    .iter()
                    .map(|(line, cols)| {
                        s.row(*line)[cols.clone()]
                            .iter()
                            .map(|c| c.c)
                            .collect::<String>()
                    })
                    .collect();
                (text, l.describe())
            })
            .collect()
    }

    #[test]
    fn urls_are_links_on_their_cells() {
        let s = snapshot(&["$ echo https://example.com now", "ok"], 40);
        let links = link_texts(&s, &HashMap::new());
        assert_eq!(
            links,
            [(
                "https://example.com".to_owned(),
                "https://example.com".to_owned()
            )]
        );
        let lines = logical_lines(&s);
        let scan = scan(&s, &lines, &HashMap::new(), false);
        assert_eq!(scan.links[0].segments, [(0, 7..26)]);
        assert!(scan.links[0].contains(Point::new(0, 7)));
        assert!(!scan.links[0].contains(Point::new(0, 26)));
        assert!(scan.links[0].visible);
    }

    #[test]
    fn a_url_over_a_soft_wrap_is_one_link() {
        let mut s = snapshot(&["see https://exam", "ple.com/a/b ok"], 16);
        wrap(&mut s, 0);
        let lines = logical_lines(&s);
        assert_eq!(lines.len(), 1);
        let scan = scan(&s, &lines, &HashMap::new(), false);
        assert_eq!(scan.links.len(), 1);
        assert_eq!(scan.links[0].segments, [(0, 4..16), (1, 0..11)]);
        assert_eq!(scan.links[0].describe(), "https://example.com/a/b");
    }

    #[test]
    fn rows_without_a_wrap_are_separate_lines() {
        let s = snapshot(&["http://a.b", "http://c.d"], 12);
        let lines = logical_lines(&s);
        assert_eq!(lines.len(), 2);
        assert_eq!(scan(&s, &lines, &HashMap::new(), false).links.len(), 2);
    }

    #[test]
    fn product_links_come_from_the_cache_and_are_asked_for_once() {
        let s = snapshot(&["  File \"/w/main.py\", line 13, in fail"], 60);
        let lines = logical_lines(&s);
        let first = scan(&s, &lines, &HashMap::new(), true);
        assert!(first.links.is_empty());
        assert_eq!(first.unfiltered, [lines[0].text.clone()]);
        let mut cache = HashMap::new();
        cache.insert(
            lines[0].text.clone(),
            vec![Link {
                range: 8..19,
                target: LinkTarget::File {
                    path: "/w/main.py".into(),
                    line: Some(13),
                    column: None,
                },
            }],
        );
        let second = scan(&s, &lines, &cache, true);
        assert!(second.unfiltered.is_empty());
        assert_eq!(second.links.len(), 1);
        assert_eq!(second.links[0].segments, [(0, 8..19)]);
        assert_eq!(second.links[0].describe(), "/w/main.py:13");
        // Without filters nothing is asked.
        assert!(
            scan(&s, &lines, &HashMap::new(), false)
                .unfiltered
                .is_empty()
        );
    }

    #[test]
    fn osc8_cells_make_a_link() {
        let mut s = snapshot(&["abc link def"], 12);
        for column in 4..8 {
            s.extras.push(terminal::CellExtra {
                point: Point::new(0, column),
                zerowidth: Vec::new(),
                underline_color: None,
                hyperlink: Some(Hyperlink {
                    id: "1".into(),
                    uri: "https://example.org/x".into(),
                }),
            });
        }
        let lines = logical_lines(&s);
        let found = scan(&s, &lines, &HashMap::new(), false);
        assert_eq!(found.links.len(), 1);
        assert_eq!(found.links[0].segments, [(0, 4..8)]);
        assert_eq!(
            found.links[0].target,
            LinkTarget::Url("https://example.org/x".into())
        );
    }

    fn token(text: &str, column: usize) -> Option<PathToken> {
        let s = snapshot(&[text], text.chars().count().max(1) + 2);
        let lines = logical_lines(&s);
        path_token_at(&lines[0], Point::new(0, column))
    }

    #[test]
    fn path_words_with_a_position() {
        let t = token("echo src/app.py:3:5 done", 8).unwrap();
        assert_eq!(
            (t.path.as_str(), t.line, t.column, t.range),
            ("src/app.py", Some(3), Some(5), 5..19)
        );
        let t = token("see ./a.py:12.", 6).unwrap();
        assert_eq!(
            (t.path.as_str(), t.line, t.column),
            ("./a.py", Some(12), None)
        );
        let t = token("/tmp", 0).unwrap();
        assert_eq!((t.path.as_str(), t.line), ("/tmp", None));
        let t = token("  C:\\work\\x.py:7", 5).unwrap();
        assert_eq!((t.path.as_str(), t.line), ("C:\\work\\x.py", Some(7)));
        let t = token("(a/b.py)", 2).unwrap();
        assert_eq!(t.path, "a/b.py");
        assert_eq!(token("echo  x", 5), None);
        let t = token("path: ", 0).unwrap();
        assert_eq!(t.path, "path");
    }

    #[test]
    fn a_colon_that_is_no_position_stays() {
        assert_eq!(split_position("a.py:x"), ("a.py:x", None, None));
        assert_eq!(split_position("a.py:0"), ("a.py:0", None, None));
        assert_eq!(split_position("a.py:2:9"), ("a.py", Some(2), Some(9)));
        assert_eq!(split_position(":5"), (":5", None, None));
    }

    #[test]
    fn paths_resolve_against_the_working_directory() {
        let cwd = Path::new("/work/proj");
        assert_eq!(
            resolve_path("src/a.py", Some(cwd)),
            Some(PathBuf::from("/work/proj/src/a.py"))
        );
        assert_eq!(
            resolve_path("/etc/hosts", Some(cwd)),
            Some(PathBuf::from("/etc/hosts"))
        );
        assert_eq!(resolve_path("src/a.py", None), None);
        assert_eq!(
            normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
    }

    #[test]
    fn existing_files_and_folders_link() {
        let dir = std::env::temp_dir().join(format!("pyoxide-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/app.py"), "x").unwrap();
        let find = |text: &str| {
            let t = token(text, 0).unwrap();
            file_link(&t, Some(&dir))
        };
        assert_eq!(
            find("src/app.py:3:5"),
            Some(LinkTarget::File {
                path: dir.join("src/app.py"),
                line: Some(3),
                column: Some(5),
            })
        );
        assert_eq!(
            find("src"),
            Some(LinkTarget::File {
                path: dir.join("src"),
                line: None,
                column: None,
            })
        );
        assert_eq!(find("src/missing.py"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
