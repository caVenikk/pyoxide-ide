use imara_diff::{Algorithm, Diff, InternedInput, Interner};
use serde::{Deserialize, Serialize};

/// What a changed range did to the base text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeKind {
    /// Lines added; nothing of the base is gone.
    Inserted,
    /// Base lines gone; the range is empty in the document.
    Deleted,
    Modified,
}

/// A changed range of a document against its base text, as the
/// reference's line status tracker keeps it (`com.intellij.openapi.vcs.ex.Range`):
/// document lines `line1..line2` stand where base lines
/// `vcs_line1..vcs_line2` were. Lines are zero-based, ends exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub line1: u32,
    pub line2: u32,
    pub vcs_line1: u32,
    pub vcs_line2: u32,
}

impl Range {
    pub fn new(line1: u32, line2: u32, vcs_line1: u32, vcs_line2: u32) -> Self {
        Self {
            line1,
            line2,
            vcs_line1,
            vcs_line2,
        }
    }

    pub fn kind(&self) -> RangeKind {
        if self.vcs_line1 == self.vcs_line2 {
            RangeKind::Inserted
        } else if self.line1 == self.line2 {
            RangeKind::Deleted
        } else {
            RangeKind::Modified
        }
    }
}

/// The lines of a text as the editor counts them: split at `\n`, a
/// trailing `\r` dropped from each, so that `"a\nb\n"` has the lines
/// `a`, `b` and an empty last one. Line endings never count as changes.
pub fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// The changed ranges of a document against its base text.
///
/// [`LineRanges::compute`] diffs the texts (off the UI thread for large
/// ones); [`LineRanges::edited`] moves the ranges with an edit of the
/// document so that markers follow at once, the edited lines counting as
/// changed until the next computation (the reference's document tracker
/// does the same).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineRanges {
    ranges: Vec<Range>,
}

impl LineRanges {
    /// Diffs `text` against `base` by lines with git's histogram
    /// algorithm, so that the ranges are the hunks of `git diff`.
    pub fn compute(base: &str, text: &str) -> Self {
        let before: Vec<&str> = split_lines(base).collect();
        let after: Vec<&str> = split_lines(text).collect();
        let mut input = InternedInput {
            before: Vec::with_capacity(before.len()),
            after: Vec::with_capacity(after.len()),
            interner: Interner::new(before.len() + after.len()),
        };
        input.update_before(before.iter().copied());
        input.update_after(after.iter().copied());
        let mut diff = Diff::compute(Algorithm::Histogram, &input);
        diff.postprocess_lines(&input);
        let ranges = diff
            .hunks()
            .map(|h| Range::new(h.after.start, h.after.end, h.before.start, h.before.end))
            .collect();
        Self { ranges }
    }

    /// Ranges as they are, in document order.
    pub fn from_ranges(ranges: Vec<Range>) -> Self {
        Self { ranges }
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The range that touches document line `line` (a deleted range
    /// touches the line it stands before).
    pub fn at_line(&self, line: u32) -> Option<&Range> {
        self.ranges.iter().find(|r| {
            if r.line1 == r.line2 {
                r.line1 == line
            } else {
                r.line1 <= line && line < r.line2
            }
        })
    }

    /// The document's lines `start..old_end` were replaced by
    /// `start..new_end`. Ranges below the edit move by the difference;
    /// the ranges the edit touches and the edited lines become one
    /// changed range.
    pub fn edited(&mut self, start: u32, old_end: u32, new_end: u32) {
        let delta = i64::from(new_end) - i64::from(old_end);
        let touches = |r: &Range| {
            if r.line1 == r.line2 || start == old_end {
                r.line1 <= old_end && start <= r.line2
            } else {
                r.line1 < old_end && start < r.line2
            }
        };
        let first = self.ranges.iter().position(touches);
        let (first, last) = match first {
            Some(first) => {
                let count = self.ranges[first..]
                    .iter()
                    .take_while(|r| touches(r))
                    .count();
                (first, first + count)
            }
            // No range touched: the new range goes before the first one
            // below the edit.
            None => {
                let at = self.ranges.partition_point(|r| r.line2 <= start);
                (at, at)
            }
        };
        // Document lines map to base lines by the offset of the range
        // before them.
        let offset_before = |i: usize| -> i64 {
            if i == 0 {
                0
            } else {
                let r = self.ranges[i - 1];
                i64::from(r.vcs_line2) - i64::from(r.line2)
            }
        };
        let to_base = |line: u32, offset: i64| (i64::from(line) + offset).max(0) as u32;
        let base_offset = offset_before(first);
        let (line1, vcs_line1) = match self.ranges.get(first).filter(|_| first < last) {
            Some(r) if r.line1 <= start => (r.line1, r.vcs_line1),
            _ => (start, to_base(start, base_offset)),
        };
        let (line2, vcs_line2) = match last.checked_sub(1).and_then(|i| self.ranges.get(i)) {
            Some(r) if first < last && r.line2 >= old_end => (r.line2, r.vcs_line2),
            _ => {
                let offset = if first < last {
                    offset_before(last)
                } else {
                    base_offset
                };
                (old_end, to_base(old_end, offset))
            }
        };
        let merged = Range::new(
            line1,
            (i64::from(line2) + delta).max(i64::from(line1)) as u32,
            vcs_line1,
            vcs_line2,
        );
        let shift = |r: &Range| Range {
            line1: (i64::from(r.line1) + delta) as u32,
            line2: (i64::from(r.line2) + delta) as u32,
            ..*r
        };
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        out.extend_from_slice(&self.ranges[..first]);
        // An edit of no lines into no lines adds nothing.
        if merged.line1 != merged.line2 || merged.vcs_line1 != merged.vcs_line2 {
            out.push(merged);
        }
        out.extend(self.ranges[last..].iter().map(shift));
        self.ranges = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(base: &str, text: &str) -> Vec<(u32, u32, u32, u32)> {
        LineRanges::compute(base, text)
            .ranges()
            .iter()
            .map(|r| (r.line1, r.line2, r.vcs_line1, r.vcs_line2))
            .collect()
    }

    #[test]
    fn ranges_are_the_changed_lines() {
        let base = "a\nb\nc\nd\n";
        assert_eq!(ranges(base, base), vec![]);
        // Modified.
        assert_eq!(ranges(base, "a\nB\nc\nd\n"), vec![(1, 2, 1, 2)]);
        // Inserted.
        assert_eq!(ranges(base, "a\nb\nx\ny\nc\nd\n"), vec![(2, 4, 2, 2)]);
        // Deleted.
        assert_eq!(ranges(base, "a\nd\n"), vec![(1, 1, 1, 3)]);
        // Appended at the end.
        assert_eq!(ranges(base, "a\nb\nc\nd\ne\n"), vec![(4, 5, 4, 4)]);
        let r = LineRanges::compute(base, "a\nd\n");
        assert_eq!(r.ranges()[0].kind(), RangeKind::Deleted);
        assert_eq!(r.at_line(1), Some(&r.ranges()[0]));
    }

    #[test]
    fn line_endings_are_not_changes() {
        assert_eq!(ranges("a\r\nb\r\n", "a\nb\n"), vec![]);
        assert_eq!(ranges("a\r\nb\r\n", "a\r\nB\r\n"), vec![(1, 2, 1, 2)]);
    }

    #[test]
    fn an_edit_below_ranges_moves_nothing_and_marks_its_lines() {
        let mut r = LineRanges::compute("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n");
        // Typing in line 3 (one line replaced by one).
        r.edited(3, 4, 4);
        assert_eq!(
            r.ranges(),
            &[Range::new(1, 2, 1, 2), Range::new(3, 4, 3, 4)]
        );
    }

    #[test]
    fn an_edit_above_ranges_moves_them() {
        let mut r = LineRanges::compute("a\nb\nc\nd\ne\n", "a\nb\nc\nD\ne\n");
        assert_eq!(r.ranges(), &[Range::new(3, 4, 3, 4)]);
        // Two new lines after line 0 (line 0 replaced by three lines).
        r.edited(0, 1, 3);
        assert_eq!(
            r.ranges(),
            &[Range::new(0, 3, 0, 1), Range::new(5, 6, 3, 4)]
        );
    }

    #[test]
    fn an_edit_inside_a_range_grows_it() {
        let mut r = LineRanges::compute("a\nb\nc\nd\n", "a\nx\ny\nc\nd\n");
        assert_eq!(r.ranges(), &[Range::new(1, 3, 1, 2)]);
        // A new line inside the range.
        r.edited(2, 3, 4);
        assert_eq!(r.ranges(), &[Range::new(1, 4, 1, 2)]);
        // An edit that spans the range and the line after it.
        r.edited(3, 5, 4);
        assert_eq!(r.ranges(), &[Range::new(1, 4, 1, 3)]);
    }

    #[test]
    fn edited_ranges_match_a_new_computation_after_shifting() {
        // Lines removed above a deleted range.
        let base = "a\nb\nc\nd\ne\nf\n";
        let mut r = LineRanges::compute(base, "a\nb\nc\nf\n");
        assert_eq!(r.ranges(), &[Range::new(3, 3, 3, 5)]);
        r.edited(0, 2, 0);
        assert_eq!(
            r.ranges(),
            &[Range::new(0, 0, 0, 2), Range::new(1, 1, 3, 5)]
        );
        assert_eq!(
            LineRanges::compute(base, "c\nf\n").ranges(),
            &[Range::new(0, 0, 0, 2), Range::new(1, 1, 3, 5)]
        );
    }
}
