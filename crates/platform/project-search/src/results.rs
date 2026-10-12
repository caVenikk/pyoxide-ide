//! The results of a search as rows: one row per line with matches, in a
//! stable order while files keep coming in.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
/// first row stays first, then files by path, lines by number. A line that
/// is already there is not added again (a later pass of the same search
/// finds it once more).
#[derive(Debug, Clone, Default)]
pub struct Results {
    rows: Vec<Row>,
    /// Path text of each row, for ordering.
    keys: Vec<Arc<str>>,
    first: Option<Arc<str>>,
    lines: HashSet<(Arc<str>, usize)>,
    files: HashSet<Arc<str>>,
    matches: usize,
}

impl Results {
    pub fn new() -> Self {
        Self::default()
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

    /// The row of a line of a file.
    pub fn position(&self, path: &Path, line: usize) -> Option<usize> {
        let key = path_key(path);
        let at = self.lower_bound(&key);
        self.rows[at..]
            .iter()
            .zip(&self.keys[at..])
            .take_while(|(_, k)| ***k == *key)
            .position(|(r, _)| r.line.line == line)
            .map(|i| at + i)
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
        let mut at = self.lower_bound(&key);
        let mut added = 0;
        for line in file.lines {
            if !self.lines.insert((key.clone(), line.line)) {
                continue;
            }
            // Past the rows of this file with smaller line numbers.
            while at < self.rows.len()
                && self.keys[at] == key
                && self.rows[at].line.line < line.line
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
            at += 1;
            added += 1;
        }
        if added > 0 {
            self.files.insert(key);
        }
        added
    }

    /// First row of the file with this key, or where its rows would go.
    fn lower_bound(&self, key: &str) -> usize {
        self.keys
            .partition_point(|k| self.order(k, key) == Ordering::Less)
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
mod tests {
    use text::Point;

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
        assert_eq!(r.position(Path::new("/p/b.py"), 4), Some(7));
        assert_eq!(r.position(Path::new("/p/b.py"), 2), None);
        assert_eq!(r.files(), 5);
        assert_eq!(r.len(), 9);
    }
}
