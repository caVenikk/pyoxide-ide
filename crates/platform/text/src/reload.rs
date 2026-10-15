//! Reloading a buffer from disk (the reference IDE's "Reload from Disk").
//!
//! The file's text replaces the buffer's as the reference's document does
//! it: the common start and end are left alone and the part between them
//! is replaced in one undoable change, after which the buffer is
//! unmodified. A caret before that part stays, one after it moves with the
//! text; a caret inside it keeps its column on the line a line diff of the
//! old and the new part leads to (the reference translates caret lines of
//! a whole-text replacement through a diff the same way), or goes to the
//! end of the new part when the two are too different to diff.
//!
//! The comparison and the diff take time proportional to the text, so a
//! [`ReloadPlan`] is made off the UI thread from a snapshot of the buffer
//! and applied with [`Buffer::apply_reload`] while the buffer still has
//! that text.

use std::time::Instant;

use ropey::Rope;

use crate::buffer::{AppliedEdit, Buffer};
use crate::encoding::{Decoded, Encoding, LineEnding};
use crate::history::{EditKind, Replacement};
use crate::selection::{Selection, SelectionSet};

/// Most lines inserted and deleted that the line diff looks for; beyond
/// that carets inside the change go to its end.
pub const MAX_DIFF_COST: usize = 1000;

/// The buffer's text changed since the plan was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalePlan;

/// The part of the text that differs: chars `start..old_end` of the buffer
/// become `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Change {
    start: usize,
    old_end: usize,
    text: String,
}

/// A run of different lines: `deleted` lines from `line0` of the old part
/// became `inserted` lines from `line1` of the new part (both counted from
/// the change's first line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineChange {
    pub line0: usize,
    pub deleted: usize,
    pub line1: usize,
    pub inserted: usize,
}

/// How a buffer becomes a file's new text.
#[derive(Debug, Clone)]
pub struct ReloadPlan {
    /// Version and length of the text the plan was made from (a buffer
    /// replaced as a whole starts its versions again).
    version: u64,
    old_len: usize,
    change: Option<Change>,
    /// Line of the change's start.
    first_line: usize,
    /// The line diff of the old and new part; `None` when too different.
    lines: Option<Vec<LineChange>>,
    line_ending: LineEnding,
    encoding: Encoding,
}

/// Where a caret was before the change.
#[derive(Debug, Clone, Copy)]
enum Place {
    Before(usize),
    After(usize),
    Inside {
        offset: usize,
        line: usize,
        column: usize,
    },
}

impl ReloadPlan {
    /// Compares a snapshot of a buffer (`rope` at `version`, see
    /// [`Buffer::rope`] and [`Buffer::version`]) with a file's decoded text
    /// (blocking; run off the UI thread).
    pub fn new(rope: &Rope, version: u64, decoded: &Decoded) -> Self {
        let new = decoded.text.as_str();
        let old_len = rope.len_chars();
        let new_len = new.chars().count();
        let prefix = rope
            .chars()
            .zip(new.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let room = old_len.min(new_len) - prefix;
        let suffix = rope
            .chars_at(old_len)
            .reversed()
            .zip(new.chars().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let mut plan = Self {
            version,
            old_len,
            change: None,
            first_line: 0,
            lines: Some(Vec::new()),
            line_ending: decoded.line_ending,
            encoding: decoded.encoding,
        };
        if prefix == old_len && prefix == new_len {
            return plan;
        }
        let (start, old_end, new_end) = (prefix, old_len - suffix, new_len - suffix);
        let byte = |chars: usize| new.char_indices().nth(chars).map_or(new.len(), |(b, _)| b);
        let (start_byte, end_byte) = (byte(start), byte(new_end));
        let text = new[start_byte..end_byte].to_owned();
        // Whole lines around the change, old and new.
        let first_line = rope.char_to_line(start);
        let old_lines: Vec<String> = (first_line..=rope.char_to_line(old_end))
            .map(|l| line_text(rope, l))
            .collect();
        let line_start = rope.line_to_byte(first_line);
        let tail_end = new[end_byte..]
            .find('\n')
            .map_or(new.len(), |i| end_byte + i);
        let new_lines: Vec<&str> = new[line_start..tail_end].split('\n').collect();
        let old_refs: Vec<&str> = old_lines.iter().map(String::as_str).collect();
        plan.first_line = first_line;
        plan.lines = diff_lines(&old_refs, &new_lines, MAX_DIFF_COST);
        plan.change = Some(Change {
            start,
            old_end,
            text,
        });
        plan
    }

    /// The text differs (not only the line separator or the encoding).
    pub fn changes_text(&self) -> bool {
        self.change.is_some()
    }

    /// The new line of an old line inside the change (the reference's
    /// `translateLineViaDiff`): a line in a run of different lines goes to
    /// the line at the same distance into the run's new lines, or to the
    /// run's last new line.
    fn translate_line(&self, line: usize) -> Option<usize> {
        let changes = self.lines.as_ref()?;
        let line = line.checked_sub(self.first_line)?;
        let mut new = line;
        for c in changes {
            if line < c.line0 {
                break;
            }
            if line >= c.line0 + c.deleted {
                new = new + c.inserted - c.deleted;
            } else {
                new = c.line1 + c.inserted.min(line - c.line0);
                break;
            }
        }
        Some(self.first_line + new)
    }

    fn place(&self, rope: &Rope, offset: usize) -> Place {
        let Some(change) = &self.change else {
            return Place::Before(offset);
        };
        if offset <= change.start {
            Place::Before(offset)
        } else if offset >= change.old_end {
            Place::After(offset)
        } else {
            let line = rope.char_to_line(offset);
            Place::Inside {
                offset,
                line,
                column: offset - rope.line_to_char(line),
            }
        }
    }

    /// The offset of a place in the buffer after the change.
    fn resolve(&self, buffer: &Buffer, place: Place) -> usize {
        let Some(change) = &self.change else {
            return match place {
                Place::Before(o) | Place::After(o) | Place::Inside { offset: o, .. } => o,
            };
        };
        let new_end = change.start + change.text.chars().count();
        match place {
            Place::Before(o) => o,
            Place::After(o) => o - change.old_end + new_end,
            Place::Inside {
                offset,
                line,
                column,
            } => match self.translate_line(line) {
                Some(line) => {
                    let line = line.min(buffer.len_lines() - 1);
                    buffer.line_start(line) + column.min(buffer.line_len(line))
                }
                None => offset.min(new_end),
            },
        }
    }
}

/// A line of a rope without its line break.
fn line_text(rope: &Rope, line: usize) -> String {
    let mut s = rope.line(line).to_string();
    if s.ends_with('\n') {
        s.pop();
    }
    s
}

/// The runs of different lines between `a` and `b` (Myers' algorithm),
/// or `None` when they need more than `max` inserted and deleted lines.
pub fn diff_lines<T: PartialEq>(a: &[T], b: &[T], max: usize) -> Option<Vec<LineChange>> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let (n, m) = (a.len(), b.len());
    if n == 0 && m == 0 {
        return Some(Vec::new());
    }
    if n == 0 || m == 0 {
        return Some(vec![LineChange {
            line0: prefix,
            deleted: n,
            line1: prefix,
            inserted: m,
        }]);
    }
    let max_d = (n + m).min(max);
    // `v[k + max_d]`: the furthest x on diagonal k; `trace[d]` keeps the
    // diagonals -d..=d of `v` as they were before step d.
    let width = 2 * max_d + 1;
    let mut v = vec![0usize; width + 2];
    let at = |k: isize| (k + max_d as isize) as usize;
    let mut trace: Vec<Vec<usize>> = Vec::new();
    for d in 0..=max_d as isize {
        trace.push((-d..=d).map(|k| v[at(k)]).collect());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)]
            } else {
                v[at(k - 1)] + 1
            };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                return Some(runs(&trace, n, m, prefix));
            }
            k += 2;
        }
    }
    None
}

/// Walks the trace back from the end and gathers the runs of deletions
/// and insertions, offset by the common prefix.
fn runs(trace: &[Vec<usize>], n: usize, m: usize, prefix: usize) -> Vec<LineChange> {
    // (x, y) of each deleted (`true`) or inserted line, from the end.
    let mut edits: Vec<(usize, usize, bool)> = Vec::new();
    let (mut x, mut y) = (n as isize, m as isize);
    for d in (1..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let get = |k: isize| v[(k + d) as usize] as isize;
        let k = x - y;
        let prev_k = if k == -d || (k != d && get(k - 1) < get(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = get(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
        }
        if x == prev_x {
            edits.push((prev_x as usize, prev_y as usize, false));
        } else {
            edits.push((prev_x as usize, prev_y as usize, true));
        }
        x = prev_x;
        y = prev_y;
    }
    edits.reverse();
    let mut out: Vec<LineChange> = Vec::new();
    for (x, y, deleted) in edits {
        let joins = out.last().is_some_and(|c| {
            c.line0 + c.deleted == prefix + x && c.line1 + c.inserted == prefix + y
        });
        if !joins {
            out.push(LineChange {
                line0: prefix + x,
                deleted: 0,
                line1: prefix + y,
                inserted: 0,
            });
        }
        let last = out.last_mut().expect("a run");
        if deleted {
            last.deleted += 1;
        } else {
            last.inserted += 1;
        }
    }
    out
}

impl Buffer {
    /// Brings the buffer to a file's text as worked out by `plan` (made
    /// from this buffer's text): the differing part is replaced in one
    /// undo step, carets follow as described in the module, the encoding
    /// and the line separator are the file's, and the buffer is
    /// unmodified. Read-only buffers are reloaded too. Returns the
    /// replacement (`None` when only the line separator or the encoding
    /// changed), or [`StalePlan`] when the text changed since the plan was
    /// made.
    pub fn apply_reload(
        &mut self,
        plan: ReloadPlan,
        selections: &mut SelectionSet,
    ) -> Result<Option<AppliedEdit>, StalePlan> {
        if plan.version != self.version() || plan.old_len != self.len_chars() {
            return Err(StalePlan);
        }
        self.set_format(plan.line_ending, plan.encoding);
        let Some(change) = plan.change.clone() else {
            self.mark_saved();
            return Ok(None);
        };
        let places: Vec<(Place, Place)> = selections
            .iter()
            .map(|s| {
                (
                    plan.place(self.rope(), s.anchor),
                    plan.place(self.rope(), s.head),
                )
            })
            .collect();
        let before = selections.clone();
        let step = vec![Replacement {
            start: change.start,
            old: self.slice(change.start..change.old_end),
            new: change.text,
        }];
        let applied = self.apply_step(&step);
        let mut mapped: Vec<Selection> = places
            .into_iter()
            .map(|(anchor, head)| {
                let h = plan.resolve(self, head);
                match (anchor, head) {
                    (Place::Inside { .. }, _) | (_, Place::Inside { .. }) => Selection::caret(h),
                    _ => Selection::new(plan.resolve(self, anchor), h),
                }
            })
            .collect();
        // The primary selection goes last, where `from_vec` takes it from.
        let primary = mapped.remove(selections.primary_index());
        mapped.push(primary);
        *selections = SelectionSet::from_vec(mapped);
        selections.clamp(self.len_chars());
        self.record_step(step, EditKind::Other, &before, selections, Instant::now());
        self.mark_saved();
        Ok(applied.into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::decode;

    fn decoded(text: &str) -> Decoded {
        decode(text.as_bytes()).unwrap()
    }

    /// Reloads `old` (caret at `|`) with `new`; the text with the caret.
    fn reload(old: &str, new: &str) -> String {
        let caret = old.find('|').unwrap();
        let old = old.replace('|', "");
        let mut b = Buffer::new(&old);
        let mut sel = SelectionSet::caret(old[..caret].chars().count());
        let plan = ReloadPlan::new(b.rope(), b.version(), &decoded(new));
        b.apply_reload(plan, &mut sel).unwrap();
        assert_eq!(b.text(), new.replace("\r\n", "\n"));
        assert!(!b.is_modified());
        let at = sel.primary().head;
        let mut out: Vec<char> = b.text().chars().collect();
        out.insert(at, '|');
        out.into_iter().collect()
    }

    #[test]
    fn carets_outside_the_change_stay_with_their_text() {
        assert_eq!(reload("a|b\nc\n", "ab\nX\n"), "a|b\nX\n");
        assert_eq!(reload("a\nb\nc|d\n", "a\nXX\nb\ncd\n"), "a\nXX\nb\nc|d\n");
        assert_eq!(reload("x = 1|", "x = 1"), "x = 1|");
    }

    #[test]
    fn carets_inside_the_change_follow_their_line_through_a_diff() {
        // A line was inserted above the caret's line and the caret's line
        // changed: the caret keeps its column on that line.
        let old = "def f():\n    retu|rn 1\n";
        let new = "import os\n\ndef f():\n    return 2\n";
        assert_eq!(reload(old, new), "import os\n\ndef f():\n    retu|rn 2\n");
        // A formatter split a line above the caret.
        let old = "x = [1,2]\ny = 3\nz = 4|\n";
        let new = "x = [\n    1,\n    2,\n]\ny = 3\nz = 5\n";
        assert_eq!(
            reload(old, new),
            "x = [\n    1,\n    2,\n]\ny = 3\nz = 5|\n"
        );
        // The caret's line went away: the line at the same distance into
        // the new lines, the column cut to its length.
        let old = "a\nbbbbbb|\nc\nd";
        let new = "a\nx\nd";
        assert_eq!(reload(old, new), "a\nx|\nd");
    }

    #[test]
    fn line_separator_and_encoding_follow_the_file() {
        let mut b = Buffer::new("a\nb\n");
        let mut sel = SelectionSet::caret(1);
        let plan = ReloadPlan::new(b.rope(), b.version(), &decoded("a\r\nb\r\n"));
        assert!(!plan.changes_text());
        assert_eq!(b.apply_reload(plan, &mut sel).unwrap(), None);
        assert_eq!(b.line_ending(), LineEnding::Crlf);
        assert_eq!(sel, SelectionSet::caret(1));
        let bom = [&[0xEF, 0xBB, 0xBF][..], b"a\nb\n"].concat();
        let plan = ReloadPlan::new(b.rope(), b.version(), &decode(&bom).unwrap());
        b.apply_reload(plan, &mut sel).unwrap();
        assert_eq!(b.encoding(), Encoding::Utf8Bom);
        assert_eq!(b.line_ending(), LineEnding::Lf);
    }

    #[test]
    fn a_reload_is_one_undo_step_and_stale_plans_are_refused() {
        let mut b = Buffer::new("one\n");
        let mut sel = SelectionSet::caret(0);
        b.transact(
            &mut sel,
            vec![crate::Edit::insert(0, "x")],
            EditKind::Typing,
            crate::CaretPlacement::AfterEachEdit,
        );
        b.mark_saved();
        let plan = ReloadPlan::new(b.rope(), b.version(), &decoded("two\n"));
        let mut edited = b.clone();
        edited.transact(
            &mut sel.clone(),
            vec![crate::Edit::insert(0, "y")],
            EditKind::Typing,
            crate::CaretPlacement::AfterEachEdit,
        );
        assert_eq!(edited.apply_reload(plan.clone(), &mut sel), Err(StalePlan));
        let applied = b.apply_reload(plan, &mut sel).unwrap().unwrap();
        assert_eq!((applied.start, applied.old_end, applied.new_end), (0, 4, 3));
        assert_eq!(b.text(), "two\n");
        assert!(!b.is_modified());
        // Undo brings the text from before the reload back, which then
        // differs from the file; the typing before stays its own step.
        b.undo().unwrap();
        assert_eq!(b.text(), "xone\n");
        assert!(b.is_modified());
        b.undo().unwrap();
        assert_eq!(b.text(), "one\n");
    }

    #[test]
    fn read_only_buffers_are_reloaded_and_selections_kept_or_collapsed() {
        let mut b = Buffer::new("keep\nold\nend\n");
        b.set_read_only(true);
        // A selection of the first line stays; one into the change
        // collapses to its caret.
        let mut sel = SelectionSet::from_vec(vec![Selection::new(0, 4), Selection::new(5, 7)]);
        let plan = ReloadPlan::new(b.rope(), b.version(), &decoded("keep\nnew line\nend\n"));
        b.apply_reload(plan, &mut sel).unwrap();
        assert_eq!(b.text(), "keep\nnew line\nend\n");
        assert_eq!(sel.as_slice()[0], Selection::new(0, 4));
        assert!(sel.as_slice()[1].is_empty());
    }

    #[test]
    fn line_diff_finds_runs_of_changed_lines() {
        let a = ["a", "b", "c", "d", "e"];
        let b = ["a", "x", "c", "e", "f"];
        assert_eq!(
            diff_lines(&a, &b, 100).unwrap(),
            vec![
                LineChange {
                    line0: 1,
                    deleted: 1,
                    line1: 1,
                    inserted: 1
                },
                LineChange {
                    line0: 3,
                    deleted: 1,
                    line1: 3,
                    inserted: 0
                },
                LineChange {
                    line0: 5,
                    deleted: 0,
                    line1: 4,
                    inserted: 1
                },
            ]
        );
        assert_eq!(diff_lines(&a, &a, 100).unwrap(), vec![]);
        assert_eq!(
            diff_lines(&a, &[], 100).unwrap(),
            vec![LineChange {
                line0: 0,
                deleted: 5,
                line1: 0,
                inserted: 0
            }]
        );
        // Too different for the cost allowed.
        let many: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        let other: Vec<String> = (0..50).map(|i| format!("x{i}")).collect();
        assert_eq!(diff_lines(&many, &other, 20), None);
        assert!(diff_lines(&many, &other, 100).is_some());
    }

    #[test]
    fn carets_keep_their_offset_within_a_change_too_different_to_diff() {
        // No line in common: more than MAX_DIFF_COST lines to diff.
        let old: String = (0..1500).map(|i| format!("{i}\n")).collect();
        let new = "-\n".repeat(1500);
        let mut b = Buffer::new(&old);
        let near = b.line_start(10) + 1;
        let far = b.line_start(1400) + 1;
        let mut sel = SelectionSet::from_vec(vec![Selection::caret(near), Selection::caret(far)]);
        let plan = ReloadPlan::new(b.rope(), b.version(), &decoded(&new));
        b.apply_reload(plan, &mut sel).unwrap();
        // The offset stays unless it is past the new part, which ends
        // before the last line break.
        let heads: Vec<usize> = sel.iter().map(|s| s.head).collect();
        assert_eq!(heads, vec![near, b.len_chars() - 1]);
        assert_eq!(sel.primary().head, b.len_chars() - 1);
    }
}
