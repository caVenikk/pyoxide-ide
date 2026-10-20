//! The differences of a two-side diff and what the viewer computes from
//! them, as the reference's side-by-side viewer does (`SimpleDiffViewer`,
//! `SimpleDiffChange`, `ByWord`, `BaseSyncScrollable`,
//! `PrevNextDifferenceIterableBase`, `SyncScrollSupport.getTargetOffsets`):
//! changed lines with their changed words, the line of one side that
//! stands at a line of the other, Next and Previous Difference, and where
//! both sides scroll to show a change. No UI here.

use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput, Interner};
use vcs::split_lines;

/// A side of the diff: the base (left) or the current text (right).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    pub fn select<T>(self, left: T, right: T) -> T {
        match self {
            Side::Left => left,
            Side::Right => right,
        }
    }

    pub fn name(self) -> &'static str {
        self.select("left", "right")
    }
}

/// What a change or a changed word did (the reference's `TextDiffType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Only on the right.
    Inserted,
    /// Only on the left.
    Deleted,
    Modified,
}

impl ChangeKind {
    fn of<T>(left: &Range<T>, right: &Range<T>) -> Self
    where
        T: PartialEq,
    {
        if left.start == left.end {
            ChangeKind::Inserted
        } else if right.start == right.end {
            ChangeKind::Deleted
        } else {
            ChangeKind::Modified
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Inserted => "inserted",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Modified => "modified",
        }
    }
}

/// A changed word (`DiffFragment`): char ranges of the left and the right
/// text of its change, from the start of the change's first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordChange {
    pub left: Range<usize>,
    pub right: Range<usize>,
}

impl WordChange {
    pub fn kind(&self) -> ChangeKind {
        ChangeKind::of(&self.left, &self.right)
    }
}

/// A changed range (`LineFragment`): left lines `left` stand where right
/// lines `right` are. Lines are zero-based, ends exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffChange {
    pub left: Range<u32>,
    pub right: Range<u32>,
    /// The changed words of a modified range (highlighted by words);
    /// `None` for inserted and deleted ranges.
    pub words: Option<Vec<WordChange>>,
}

impl DiffChange {
    pub fn kind(&self) -> ChangeKind {
        ChangeKind::of(&self.left, &self.right)
    }

    pub fn lines(&self, side: Side) -> Range<u32> {
        side.select(self.left.clone(), self.right.clone())
    }
}

/// How whitespace counts (the reference's Ignore Differences).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IgnorePolicy {
    /// Every difference (Do not ignore).
    #[default]
    None,
    /// Whitespace at the start and the end of lines (Trim whitespaces).
    Trim,
    /// All whitespace (Ignore whitespaces).
    Whitespace,
}

impl IgnorePolicy {
    pub const ALL: [IgnorePolicy; 3] = [
        IgnorePolicy::None,
        IgnorePolicy::Trim,
        IgnorePolicy::Whitespace,
    ];

    /// The reference's texts (`option.ignore.policy.*`).
    pub fn text(self) -> &'static str {
        match self {
            IgnorePolicy::None => "Do not ignore",
            IgnorePolicy::Trim => "Trim whitespaces",
            IgnorePolicy::Whitespace => "Ignore whitespaces",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            IgnorePolicy::None => "none",
            IgnorePolicy::Trim => "trim",
            IgnorePolicy::Whitespace => "whitespace",
        }
    }

    /// A line as it is compared.
    fn key(self, line: &str) -> std::borrow::Cow<'_, str> {
        match self {
            IgnorePolicy::None => line.into(),
            IgnorePolicy::Trim => line.trim().into(),
            IgnorePolicy::Whitespace => line
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .into(),
        }
    }
}

/// How changes are highlighted (the reference's Highlighting Differences).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HighlightPolicy {
    /// Changed lines (Lines).
    Lines,
    /// Changed lines and the changed words in them (Words).
    #[default]
    Words,
    /// No highlighting (None): the differences are not shown.
    None,
}

impl HighlightPolicy {
    pub const ALL: [HighlightPolicy; 3] = [
        HighlightPolicy::Lines,
        HighlightPolicy::Words,
        HighlightPolicy::None,
    ];

    pub fn text(self) -> &'static str {
        match self {
            HighlightPolicy::Lines => "Lines",
            HighlightPolicy::Words => "Words",
            HighlightPolicy::None => "None",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            HighlightPolicy::Lines => "lines",
            HighlightPolicy::Words => "words",
            HighlightPolicy::None => "none",
        }
    }
}

/// The diff's settings (`TextDiffSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffSettings {
    pub ignore: IgnorePolicy,
    pub highlight: HighlightPolicy,
}

/// The changed ranges of `right` against `left` with git's histogram
/// algorithm (the hunks of `git diff`, as the change markers have them),
/// and for [`HighlightPolicy::Words`] the changed words of modified
/// ranges. Line endings never count.
pub fn compare(left: &str, right: &str, settings: DiffSettings) -> Vec<DiffChange> {
    if settings.highlight == HighlightPolicy::None {
        return Vec::new();
    }
    let before: Vec<&str> = split_lines(left).collect();
    let after: Vec<&str> = split_lines(right).collect();
    let ignore = settings.ignore;
    let before_keys: Vec<std::borrow::Cow<'_, str>> =
        before.iter().map(|l| ignore.key(l)).collect();
    let after_keys: Vec<std::borrow::Cow<'_, str>> = after.iter().map(|l| ignore.key(l)).collect();
    let mut input = InternedInput {
        before: Vec::with_capacity(before.len()),
        after: Vec::with_capacity(after.len()),
        interner: Interner::new(before.len() + after.len()),
    };
    input.update_before(before_keys.iter().map(|k| k.as_ref()));
    input.update_after(after_keys.iter().map(|k| k.as_ref()));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    diff.hunks()
        .map(|h| {
            let left = h.before.start..h.before.end;
            let right = h.after.start..h.after.end;
            let words = (settings.highlight == HighlightPolicy::Words
                && ChangeKind::of(&left, &right) == ChangeKind::Modified)
                .then(|| {
                    let a = before[left.start as usize..left.end as usize].join("\n");
                    let b = after[right.start as usize..right.end as usize].join("\n");
                    word_changes(&a, &b, ignore)
                });
            DiffChange { left, right, words }
        })
        .collect()
}

/// Splits a text into words, whitespace runs and single other chars, as
/// char ranges.
fn tokens(text: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut last: Option<Class> = None;
    for (i, c) in text.chars().enumerate() {
        let k = class(c);
        match out.last_mut() {
            Some(t) if k != Class::Other && last == Some(k) => t.end = i + 1,
            _ => out.push(i..i + 1),
        }
        last = Some(k);
    }
    out
}

/// The changed words of `left` against `right` (`ByWord`): tokens
/// compared with Myers' algorithm, whitespace tokens ignored as the
/// policy says.
pub fn word_changes(left: &str, right: &str, ignore: IgnorePolicy) -> Vec<WordChange> {
    let lc: Vec<char> = left.chars().collect();
    let rc: Vec<char> = right.chars().collect();
    let space =
        |chars: &[char], r: &Range<usize>| chars[r.clone()].iter().all(|c| c.is_whitespace());
    let keep = |chars: &[char], r: &Range<usize>| ignore == IgnorePolicy::None || !space(chars, r);
    let before: Vec<Range<usize>> = tokens(left).into_iter().filter(|r| keep(&lc, r)).collect();
    let after: Vec<Range<usize>> = tokens(right).into_iter().filter(|r| keep(&rc, r)).collect();
    let text = |chars: &[char], r: &Range<usize>| chars[r.clone()].iter().collect::<String>();
    let before_text: Vec<String> = before.iter().map(|r| text(&lc, r)).collect();
    let after_text: Vec<String> = after.iter().map(|r| text(&rc, r)).collect();
    let mut input = InternedInput {
        before: Vec::with_capacity(before.len()),
        after: Vec::with_capacity(after.len()),
        interner: Interner::new(before.len() + after.len()),
    };
    input.update_before(before_text.iter().map(String::as_str));
    input.update_after(after_text.iter().map(String::as_str));
    let diff = Diff::compute(Algorithm::Myers, &input);
    // Where an empty side of a fragment stands: after the token before it.
    let at = |tokens: &[Range<usize>], i: u32| -> usize {
        if i == 0 {
            tokens.first().map_or(0, |t| t.start)
        } else {
            tokens[i as usize - 1].end
        }
    };
    let span = |tokens: &[Range<usize>], r: Range<u32>| -> Range<usize> {
        if r.start == r.end {
            let p = at(tokens, r.start);
            p..p
        } else {
            tokens[r.start as usize].start..tokens[r.end as usize - 1].end
        }
    };
    diff.hunks()
        .map(|h| WordChange {
            left: span(&before, h.before.start..h.before.end),
            right: span(&after, h.after.start..h.after.end),
        })
        .collect()
}

/// The status text (`diff.count.differences.status.text`).
pub fn status_text(count: usize) -> String {
    match count {
        0 => "No differences".to_owned(),
        1 => "1 difference".to_owned(),
        n => format!("{n} differences"),
    }
}

/// The status text of a file in the commit's list, `included` of whose
/// `count` changes go into the commit (`LocalTrackerDiffUtil.getStatusText`,
/// `diff.included.count.differences.status.text`): "1 difference, 0
/// included"; nothing more when all of them do.
pub fn status_text_included(count: usize, included: Option<usize>) -> String {
    let mut text = status_text(count);
    if let Some(n) = included
        && n != count
    {
        text.push_str(&format!(", {n} included"));
    }
    text
}

/// The files counter (`diff.files.count.progress`): "2/3 files".
pub fn files_text(index: usize, count: usize) -> String {
    match count {
        0 => "No files".to_owned(),
        1 => "1 file".to_owned(),
        n => format!("{}/{n} files", index + 1),
    }
}

// ---- synchronized scrolling -------------------------------------------------

/// How long an animated scroll over `distance` pixels takes
/// (`ScrollingModelImpl.AnimatedScrollingRunnable.getScrollDuration`):
/// nothing for a line or less, 100 ms for 11 lines or more, in proportion
/// between.
pub fn scroll_duration(distance: f32, line_height: f32) -> std::time::Duration {
    if line_height <= 0.0 {
        return std::time::Duration::ZERO;
    }
    let lines = distance.abs() / line_height;
    let part = ((lines - 1.0) / 10.0).clamp(0.0, 1.0);
    std::time::Duration::from_millis((part * 100.0).round() as u64)
}

/// The animation's progress at `t` of its time (`Easing.EASE_OUT`, the
/// cubic Bézier curve (0, 0), (0, 0), (0.58, 1), (1, 1)).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (c2x, c2y) = (0.58_f32, 1.0_f32);
    // x(s) and y(s) of the curve with its first control point at the origin.
    let x = |s: f32| 3.0 * (1.0 - s) * s * s * c2x + s * s * s;
    let y = |s: f32| 3.0 * (1.0 - s) * s * s * c2y + s * s * s;
    // x is increasing: find s with x(s) = t by bisection.
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if x(mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    y((lo + hi) / 2.0)
}

/// Lines of `side` and of the other side that correspond
/// (`BaseSyncScrollable.getRange`): the block of `side`'s line `line`
/// between change boundaries, as (`start1`, `end1`) of `side` and
/// (`start2`, `end2`) of the other.
pub fn block_at(
    changes: &[DiffChange],
    counts: (u32, u32),
    side: Side,
    line: u32,
) -> (u32, u32, u32, u32) {
    if changes.is_empty() {
        return (line, line + 1, line, line + 1);
    }
    let mut pairs = Vec::with_capacity(changes.len() * 2 + 2);
    pairs.push((0, 0));
    for c in changes {
        pairs.push((c.left.start, c.right.start));
        pairs.push((c.left.end, c.right.end));
    }
    pairs.push(counts);
    let pick = |p: (u32, u32)| match side {
        Side::Left => (p.0, p.1),
        Side::Right => (p.1, p.0),
    };
    let mut prev = (0, 0);
    let mut cur = (0, 0);
    for p in pairs {
        prev = cur;
        cur = pick(p);
        if line <= cur.0 {
            break;
        }
    }
    (prev.0, cur.0, prev.1, cur.1)
}

/// The other side's line that stands at `side`'s line `line`
/// (`BaseSyncScrollable.transferLine`).
pub fn transfer_line(changes: &[DiffChange], counts: (u32, u32), side: Side, line: u32) -> u32 {
    let (start1, end1, start2, end2) = block_at(changes, counts, side, line);
    if start1 == line {
        start2
    } else if end1 == line {
        end2
    } else if end1 < line {
        line - end1 + end2
    } else {
        (start2 + (line - start1)).min(end2)
    }
}

/// Where the other side scrolls when `side` scrolled to `top`
/// (`SyncScrollSupport.ScrollHelper.syncVerticalScroll`): the line a third
/// down the view is kept against its counterpart. `None` keeps the other
/// side where it is (a move of less than a line inside a change, which
/// would make it jitter). Heights and offsets in pixels.
pub fn synced_top(
    changes: &[DiffChange],
    counts: (u32, u32),
    side: Side,
    top: f32,
    view_height: f32,
    line_height: f32,
    other_top: f32,
) -> Option<f32> {
    if line_height <= 0.0 {
        return None;
    }
    let middle = (view_height / 3.0).floor();
    let master = top + middle;
    let master_line = (master / line_height).floor().max(0.0) as u32;
    let converted = transfer_line(changes, counts, side, master_line);
    let slave = converted as f32 * line_height;
    let correction = (master - master_line as f32 * line_height) % line_height;
    let offset = slave - middle + correction;
    let only_backward = correction < line_height / 2.0
        && master_line > 0
        && converted == transfer_line(changes, counts, side, master_line - 1);
    let only_forward = correction > line_height / 2.0
        && converted == transfer_line(changes, counts, side, master_line + 1);
    if ((only_forward && offset > other_top) || (only_backward && offset < other_top))
        && (offset - other_top).abs() < line_height
    {
        return None;
    }
    Some(offset)
}

/// Where both sides scroll to show lines `lines[i]` of side `i`
/// (`SyncScrollSupport.getTargetOffsets`): the whole range a third down
/// the view when it fits, both at the same distance from the top; at the
/// end of a text as much of the range as fits. Pixels.
pub fn target_tops(
    lines: [Range<u32>; 2],
    line_height: f32,
    view_heights: [f32; 2],
    max_tops: [f32; 2],
) -> [f32; 2] {
    let lh = line_height;
    let top: [f32; 2] = [lines[0].start as f32 * lh, lines[1].start as f32 * lh];
    let bottom: [f32; 2] = [
        (lines[0].end + 1) as f32 * lh,
        (lines[1].end + 1) as f32 * lh,
    ];
    let height = [bottom[0] - top[0], bottom[1] - top[1]];
    let gap = 2.0 * lh;
    let mut shifts = [0.0; 2];
    for i in 0..2 {
        let can_show = 2.0 * gap + height[i] <= view_heights[i];
        let shift = (view_heights[i] / 3.0).floor();
        shifts[i] = if can_show {
            (view_heights[i] - gap - height[i]).min(shift)
        } else {
            gap
        };
    }
    let mut shift = shifts[0].min(shifts[1]);
    shift = shift.min(top[0]).min(top[1]);
    let offsets = [top[0] - shift, top[1] - shift];
    if max_tops[0] > offsets[0] && max_tops[1] > offsets[1] {
        return offsets;
    }
    // A range at the end of a text: it cannot scroll that far.
    let mut shift: f32 = 0.0;
    for i in 0..2 {
        shift = shift.max(top[i] - max_tops[i]);
    }
    let mut out = [0.0; 2];
    for i in 0..2 {
        out[i] = top[i] - shift + (shift + height[i] + gap - view_heights[i]).max(0.0);
        out[i] = out[i].min(top[i] - gap);
    }
    out
}

// ---- Next and Previous Difference -------------------------------------------

/// The change Next Difference goes to from the caret's line, when there
/// is one (`PrevNextDifferenceIterableBase.canGoNext` and `goNext`).
pub fn next_change(
    changes: &[DiffChange],
    side: Side,
    line: u32,
    line_count: u32,
) -> Option<usize> {
    let last = changes.last()?;
    if line + 1 >= line_count || last.lines(side).start <= line {
        return None;
    }
    changes.iter().position(|c| c.lines(side).start > line)
}

/// The change Previous Difference goes to (`canGoPrev` and `goPrev`).
pub fn previous_change(changes: &[DiffChange], side: Side, line: u32) -> Option<usize> {
    let first = changes.first()?;
    let lines = first.lines(side);
    if line == 0 || lines.end > line || lines.start >= line {
        return None;
    }
    (0..changes.len()).find(|&i| match changes.get(i + 1) {
        None => true,
        Some(next) => {
            let n = next.lines(side);
            n.end > line || n.start >= line
        }
    })
}

/// The change whose lines contain `line` of `side` (an empty range
/// contains the line it stands before).
pub fn change_at(changes: &[DiffChange], side: Side, line: u32) -> Option<usize> {
    changes.iter().position(|c| {
        let r = c.lines(side);
        if r.start == r.end {
            r.start == line
        } else {
            r.start <= line && line < r.end
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scroll_to_a_change_eases_out_in_up_to_a_tenth_of_a_second() {
        use std::time::Duration;
        assert_eq!(scroll_duration(20.0, 20.0), Duration::ZERO);
        assert_eq!(scroll_duration(-120.0, 20.0), Duration::from_millis(50));
        assert_eq!(scroll_duration(2000.0, 20.0), Duration::from_millis(100));
        assert_eq!(ease_out(0.0), 0.0);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-4);
        // Fast at first, slow at the end.
        assert!(ease_out(0.5) > 0.6);
        assert!(ease_out(0.25) < ease_out(0.5));
    }

    fn ranges(left: &str, right: &str) -> Vec<(Range<u32>, Range<u32>, ChangeKind)> {
        compare(left, right, DiffSettings::default())
            .into_iter()
            .map(|c| {
                let k = c.kind();
                (c.left, c.right, k)
            })
            .collect()
    }

    #[test]
    fn changes_are_the_hunks_of_git_diff() {
        let base = "a\nb\nc\nd\n";
        assert_eq!(ranges(base, base), vec![]);
        assert_eq!(
            ranges(base, "a\nB\nc\nd\n"),
            vec![(1..2, 1..2, ChangeKind::Modified)]
        );
        assert_eq!(
            ranges(base, "a\nb\nc\nd\n# changed\n"),
            vec![(4..4, 4..5, ChangeKind::Inserted)]
        );
        assert_eq!(
            ranges(base, "a\nd\n"),
            vec![(1..3, 1..1, ChangeKind::Deleted)]
        );
        // Line endings are not differences.
        assert_eq!(ranges("a\r\nb\r\n", "a\nb\n"), vec![]);
    }

    #[test]
    fn modified_lines_have_their_changed_words() {
        let changes = compare(
            "x = compute(1)\n",
            "x = compute(2)\n",
            DiffSettings::default(),
        );
        assert_eq!(changes.len(), 1);
        let words = changes[0].words.clone().unwrap();
        assert_eq!(
            words,
            vec![WordChange {
                left: 12..13,
                right: 12..13
            }]
        );
        assert_eq!(words[0].kind(), ChangeKind::Modified);
        // A word added: empty on the left, after the token before it.
        let words = word_changes("a b", "a new b", IgnorePolicy::None);
        assert_eq!(
            words,
            vec![WordChange {
                left: 2..2,
                right: 2..6
            }]
        );
        assert_eq!(words[0].kind(), ChangeKind::Inserted);
        // Chars, not bytes.
        let words = word_changes("é = 1", "é = 2", IgnorePolicy::None);
        assert_eq!(
            words,
            vec![WordChange {
                left: 4..5,
                right: 4..5
            }]
        );
        // Lines only: no words.
        let lines = compare(
            "a\n",
            "b\n",
            DiffSettings {
                highlight: HighlightPolicy::Lines,
                ..DiffSettings::default()
            },
        );
        assert_eq!(lines[0].words, None);
    }

    #[test]
    fn whitespace_is_ignored_as_asked() {
        let trim = DiffSettings {
            ignore: IgnorePolicy::Trim,
            ..DiffSettings::default()
        };
        let all = DiffSettings {
            ignore: IgnorePolicy::Whitespace,
            ..DiffSettings::default()
        };
        assert_eq!(compare("a = 1\n", "  a = 1  \n", trim), vec![]);
        assert_eq!(compare("a = 1\n", "a=1\n", trim).len(), 1);
        assert_eq!(compare("a = 1\n", "a=1\n", all), vec![]);
        assert_eq!(
            compare("a = 1\n", "a  =  1\n", DiffSettings::default()).len(),
            1
        );
    }

    fn change(left: Range<u32>, right: Range<u32>) -> DiffChange {
        DiffChange {
            left,
            right,
            words: None,
        }
    }

    #[test]
    fn lines_transfer_through_the_changes() {
        // Two lines inserted on the right after left line 2.
        let changes = vec![change(2..2, 2..4)];
        let counts = (6, 8);
        assert_eq!(transfer_line(&changes, counts, Side::Left, 1), 1);
        assert_eq!(transfer_line(&changes, counts, Side::Left, 2), 2);
        assert_eq!(transfer_line(&changes, counts, Side::Left, 3), 5);
        assert_eq!(transfer_line(&changes, counts, Side::Right, 3), 2);
        assert_eq!(transfer_line(&changes, counts, Side::Right, 5), 3);
        // No changes: the same line.
        assert_eq!(transfer_line(&[], counts, Side::Right, 4), 4);
    }

    #[test]
    fn scrolling_one_side_moves_the_other_to_the_same_place() {
        let changes = vec![change(2..2, 2..12)];
        // The right side scrolled ten lines down: its line 11, a third
        // down a 60 px view, is one of the inserted lines, which stand at
        // the left's line 2.
        let top = synced_top(&changes, (20, 30), Side::Right, 200.0, 60.0, 20.0, 0.0);
        assert_eq!(top, Some(2.0 * 20.0 - 20.0));
        // Below the inserted lines both sides move together.
        let top = synced_top(&changes, (20, 30), Side::Right, 400.0, 60.0, 20.0, 0.0);
        assert_eq!(top, Some(400.0 - 10.0 * 20.0));
        // Without changes both sides keep the same top.
        let top = synced_top(&[], (20, 20), Side::Left, 120.0, 300.0, 20.0, 0.0);
        assert_eq!(top, Some(120.0));
    }

    #[test]
    fn a_change_is_shown_a_third_down_both_views() {
        let tops = target_tops([10..11, 20..22], 20.0, [600.0, 600.0], [2000.0, 2000.0]);
        // A third of 600 above both ranges.
        assert_eq!(tops, [200.0 - 200.0, 400.0 - 200.0]);
        // Near the top: no further than the first line.
        let tops = target_tops([1..2, 1..2], 20.0, [600.0, 600.0], [2000.0, 2000.0]);
        assert_eq!(tops, [0.0, 0.0]);
    }

    #[test]
    fn next_and_previous_difference_go_by_the_caret_line() {
        let changes = vec![
            change(2..3, 2..3),
            change(8..8, 8..10),
            change(20..21, 22..22),
        ];
        let side = Side::Right;
        assert_eq!(next_change(&changes, side, 0, 40), Some(0));
        assert_eq!(next_change(&changes, side, 2, 40), Some(1));
        assert_eq!(next_change(&changes, side, 8, 40), Some(2));
        assert_eq!(next_change(&changes, side, 22, 40), None);
        assert_eq!(previous_change(&changes, side, 22), Some(1));
        assert_eq!(previous_change(&changes, side, 9), Some(0));
        assert_eq!(previous_change(&changes, side, 2), None);
        assert_eq!(previous_change(&changes, side, 0), None);
        // The left side counts its own lines.
        assert_eq!(next_change(&changes, Side::Left, 8, 40), Some(2));
        assert_eq!(change_at(&changes, side, 9), Some(1));
        assert_eq!(change_at(&changes, side, 22), Some(2));
        assert_eq!(change_at(&changes, side, 5), None);
    }

    #[test]
    fn texts() {
        assert_eq!(status_text(0), "No differences");
        assert_eq!(status_text(1), "1 difference");
        assert_eq!(status_text(3), "3 differences");
        assert_eq!(files_text(1, 3), "2/3 files");
        assert_eq!(files_text(0, 1), "1 file");
    }
}
