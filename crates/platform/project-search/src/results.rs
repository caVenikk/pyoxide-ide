//! The results of a search as rows: one row per line with matches, in a
//! stable order while files keep coming in.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use text::Point;

use crate::find::{FileMatches, LineMatch};

/// One line with matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: Arc<PathBuf>,
    pub line: LineMatch,
}

impl Row {
    /// Number of matches on the line.
    pub fn matches(&self) -> usize {
        self.line.ranges.len()
    }
}

/// The rows of a search, kept in the reference IDE's order: the file of the
/// first row stays first, then files by path, lines by number. A row that
/// is already there is not added again (a later pass of the same search
/// finds it once more). In replace mode every match is a row of its own.
///
/// Loading more keeps the rows on screen where they are: after
/// [`freeze`](Self::freeze) new rows go in order after the frozen ones.
#[derive(Debug, Clone, Default)]
pub struct Results {
    rows: Vec<Row>,
    /// Path text of each row, for ordering.
    keys: Vec<Arc<str>>,
    first: Option<Arc<str>>,
    /// File, line and column of every row.
    seen: HashSet<(Arc<str>, usize, usize)>,
    files: HashSet<Arc<str>>,
    matches: usize,
    per_match: bool,
    frozen: usize,
}

impl Results {
    /// One row per line.
    pub fn new() -> Self {
        Self::default()
    }

    /// One row per match (Replace in Files).
    pub fn per_match() -> Self {
        Self {
            per_match: true,
            ..Self::default()
        }
    }

    pub fn is_per_match(&self) -> bool {
        self.per_match
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Files with rows.
    pub fn files(&self) -> usize {
        self.files.len()
    }

    /// Matches over all rows.
    pub fn matches(&self) -> usize {
        self.matches
    }

    /// The rows stay where they are; later rows go after them.
    pub fn freeze(&mut self) {
        self.frozen = self.rows.len();
    }

    /// The lines with rows, by file (for a later pass to skip).
    pub fn lines_by_file(&self) -> std::collections::HashMap<PathBuf, HashSet<usize>> {
        let mut out: std::collections::HashMap<PathBuf, HashSet<usize>> = Default::default();
        for row in &self.rows {
            out.entry((*row.path).clone())
                .or_default()
                .insert(row.line.line);
        }
        out
    }

    /// The row of a match: its file, line and the char column where it
    /// starts (the first match of the line for a row per line).
    pub fn position(&self, path: &Path, line: usize, column: usize) -> Option<usize> {
        self.rows.iter().position(|r| {
            r.line.line == line
                && r.line.ranges.first().map(|m| m.start) == Some(column)
                && *r.path == path
        })
    }

    /// Removes a row (its match was replaced).
    pub fn remove(&mut self, index: usize) -> Option<Row> {
        if index >= self.rows.len() {
            return None;
        }
        let row = self.rows.remove(index);
        let key = self.keys.remove(index);
        if index < self.frozen {
            self.frozen -= 1;
        }
        self.matches -= row.matches();
        let column = row.line.ranges.first().map_or(0, |m| m.start);
        self.seen.remove(&(key.clone(), row.line.line, column));
        if !self.keys.contains(&key) {
            self.files.remove(&key);
        }
        Some(row)
    }

    /// Adds the lines of a file; returns how many rows were added.
    pub fn add(&mut self, file: FileMatches) -> usize {
        if file.lines.is_empty() {
            return 0;
        }
        let key: Arc<str> = path_key(&file.path).into();
        if self.first.is_none() {
            self.first = Some(key.clone());
        }
        let path = Arc::new(file.path);
        let lines: Vec<LineMatch> = if self.per_match {
            file.lines.into_iter().flat_map(split_matches).collect()
        } else {
            file.lines
        };
        let start = self.lower_bound(&key);
        let mut added = 0;
        for line in lines {
            let column = line.ranges.first().map_or(0, |m| m.start);
            if !self.seen.insert((key.clone(), line.line, column)) {
                continue;
            }
            // Past the rows of this file that come before.
            let mut at = start;
            while at < self.rows.len()
                && self.keys[at] == key
                && (self.rows[at].line.line, self.rows[at].line.ranges[0].start)
                    < (line.line, column)
            {
                at += 1;
            }
            self.matches += line.ranges.len();
            self.rows.insert(
                at,
                Row {
                    path: path.clone(),
                    line,
                },
            );
            self.keys.insert(at, key.clone());
            added += 1;
        }
        if added > 0 {
            self.files.insert(key);
        }
        added
    }

    /// First row of the file with this key after the frozen rows, or where
    /// its rows would go.
    fn lower_bound(&self, key: &str) -> usize {
        self.frozen
            + self.keys[self.frozen..].partition_point(|k| self.order(k, key) == Ordering::Less)
    }

    /// The order of two files: the first file found, then by path.
    fn order(&self, a: &str, b: &str) -> Ordering {
        let first = self.first.as_deref();
        match (Some(a) == first, Some(b) == first) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.cmp(b),
        }
    }
}

/// A line's matches as lines of one match each.
fn split_matches(line: LineMatch) -> Vec<LineMatch> {
    let first = line.first.clone();
    line.ranges
        .iter()
        .enumerate()
        .map(|(i, m)| LineMatch {
            line: line.line,
            text: line.text.clone(),
            text_column: line.text_column,
            ranges: vec![m.clone()],
            first: if i == 0 {
                first.clone()
            } else {
                Point::new(line.line, m.start)..Point::new(line.line, m.end)
            },
        })
        .collect()
}

/// A path as text with `/` separators, compared as text (as the reference
/// IDE compares its results).
fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // expected ranges
mod tests {
    use super::*;

    fn file(path: &str, lines: &[usize]) -> FileMatches {
        FileMatches {
            path: PathBuf::from(path),
            lines: lines
                .iter()
                .map(|&line| LineMatch {
                    line,
                    text: format!("line {line}"),
                    text_column: 0,
                    ranges: vec![0..1, 2..3],
                    first: Point::new(line, 0)..Point::new(line, 1),
                })
                .collect(),
        }
    }

    fn order(r: &Results) -> Vec<(String, usize)> {
        r.rows()
            .iter()
            .map(|row| (row.path.to_string_lossy().into_owned(), row.line.line))
            .collect()
    }

    #[test]
    fn the_first_file_stays_first_then_by_path_and_line() {
        let mut r = Results::new();
        assert_eq!(r.add(file("/p/m.py", &[3, 9])), 2);
        r.add(file("/p/b.py", &[1]));
        r.add(file("/p/z.py", &[0]));
        r.add(file("/p/a/c.py", &[5]));
        r.add(file("/p/a.py", &[2]));
        assert_eq!(
            order(&r),
            [
                ("/p/m.py".into(), 3),
                ("/p/m.py".into(), 9),
                ("/p/a.py".into(), 2),
                ("/p/a/c.py".into(), 5),
                ("/p/b.py".into(), 1),
                ("/p/z.py".into(), 0),
            ]
        );
        assert_eq!(r.files(), 5);
        assert_eq!(r.matches(), 12);
        // A later pass adds only the lines that are new, in their places.
        assert_eq!(r.add(file("/p/b.py", &[0, 1, 4])), 2);
        assert_eq!(r.add(file("/p/m.py", &[3, 5])), 1);
        assert_eq!(
            order(&r)[..3],
            [
                ("/p/m.py".into(), 3),
                ("/p/m.py".into(), 5),
                ("/p/m.py".into(), 9)
            ]
        );
        assert_eq!(r.position(Path::new("/p/b.py"), 4, 0), Some(7));
        assert_eq!(r.position(Path::new("/p/b.py"), 2, 0), None);
        assert_eq!(r.files(), 5);
        assert_eq!(r.len(), 9);
    }

    #[test]
    fn frozen_rows_keep_their_places() {
        let mut r = Results::new();
        r.add(file("/p/m.py", &[3]));
        r.add(file("/p/z.py", &[1]));
        r.freeze();
        // A file that sorts before the frozen ones goes after them.
        r.add(file("/p/a.py", &[2]));
        r.add(file("/p/m.py", &[1, 3]));
        assert_eq!(
            order(&r),
            [
                ("/p/m.py".into(), 3),
                ("/p/z.py".into(), 1),
                ("/p/m.py".into(), 1),
                ("/p/a.py".into(), 2),
            ]
        );
        assert_eq!(r.files(), 3);
        assert_eq!(r.lines_by_file()[&PathBuf::from("/p/m.py")].len(), 2);
    }

    #[test]
    fn a_row_per_match_in_replace_mode() {
        let mut r = Results::per_match();
        r.add(file("/p/a.py", &[4, 2]));
        assert_eq!(r.len(), 4);
        let cols: Vec<_> = r
            .rows()
            .iter()
            .map(|row| (row.line.line, row.line.ranges.clone(), row.line.first.start))
            .collect();
        assert_eq!(cols[0], (2, vec![0..1], Point::new(2, 0)));
        assert_eq!(cols[1], (2, vec![2..3], Point::new(2, 2)));
        assert_eq!(cols[2].0, 4);
        assert_eq!(r.matches(), 4);
        assert_eq!(r.position(Path::new("/p/a.py"), 2, 2), Some(1));
        // Replacing one match removes its row.
        let row = r.remove(1).unwrap();
        assert_eq!(row.line.ranges, vec![2..3]);
        assert_eq!(r.len(), 3);
        assert_eq!(r.matches(), 3);
        assert_eq!(r.files(), 1);
    }
}
