//! The commit legend (`CommitLegendComponent`, `ChangeInfoCalculator`):
//! what the included files do, "1 added   2 modified   1 deleted", or
//! compact "+1 *2 -1" when the full text does not fit. A rename counts as
//! modified; included unversioned files count as added ("2+1 added" with
//! both).

use vcs::{Change, FileStatus};

/// One part of the legend and the status whose colour it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub status: FileStatus,
}

/// Counts of the included files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Legend {
    pub added: usize,
    pub unversioned: usize,
    pub modified: usize,
    pub deleted: usize,
}

/// Between two chunks.
pub const SEPARATOR: &str = "   ";

/// `%,d`: digits grouped by thousands.
fn number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Legend {
    pub fn of(changes: &[Change], unversioned: usize) -> Self {
        let count = |f: &dyn Fn(FileStatus) -> bool| changes.iter().filter(|c| f(c.status)).count();
        Self {
            added: count(&|s| s == FileStatus::Added),
            unversioned,
            modified: count(&|s| {
                matches!(
                    s,
                    FileStatus::Modified | FileStatus::Renamed | FileStatus::Conflict
                )
            }),
            deleted: count(&|s| s == FileStatus::Deleted),
        }
    }

    /// The chunks, full (`commit.legend.*`) or compact.
    pub fn chunks(&self, compact: bool) -> Vec<Chunk> {
        let added = match (self.added, self.unversioned) {
            (0, 0) => None,
            (a, 0) => Some(number(a)),
            (0, u) => Some(number(u)),
            (a, u) => Some(format!("{}+{}", number(a), number(u))),
        };
        let parts = [
            (added, "added", "+", FileStatus::Added),
            (
                (self.modified > 0).then(|| number(self.modified)),
                "modified",
                "*",
                FileStatus::Modified,
            ),
            (
                (self.deleted > 0).then(|| number(self.deleted)),
                "deleted",
                "-",
                FileStatus::Deleted,
            ),
        ];
        parts
            .into_iter()
            .filter_map(|(value, label, short, status)| {
                let value = value?;
                let text = if compact {
                    format!("{short}{value}")
                } else {
                    format!("{value} {label}")
                };
                Some(Chunk { text, status })
            })
            .collect()
    }

    /// The legend as one text.
    pub fn text(&self, compact: bool) -> String {
        self.chunks(compact)
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join(SEPARATOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(status: FileStatus) -> Change {
        Change {
            path: "/r/f".into(),
            status,
            old_path: None,
        }
    }

    #[test]
    fn the_legend_counts_and_shortens() {
        let changes = [
            change(FileStatus::Added),
            change(FileStatus::Modified),
            change(FileStatus::Renamed),
        ];
        let legend = Legend::of(&changes, 0);
        assert_eq!(legend.text(false), "1 added   2 modified");
        assert_eq!(legend.text(true), "+1   *2");
        let legend = Legend::of(&[change(FileStatus::Deleted)], 2);
        assert_eq!(legend.text(false), "2 added   1 deleted");
        let legend = Legend::of(&[change(FileStatus::Added)], 3);
        assert_eq!(legend.text(true), "+1+3");
        assert_eq!(Legend::default().text(false), "");
        assert_eq!(number(1234567), "1,234,567");
    }
}
