//! Change markers of an editor against its base text (the reference's
//! line status tracker, `LineStatusTrackerBase` and
//! `LineStatusMarkerRenderer`): a marker between the line numbers and the
//! text for every changed range, a thin mark on the error stripe, Next and
//! Previous Change, and Rollback Lines as one undoable edit.
//!
//! The ranges move with an edit at once (the edited lines count as
//! changed) and are computed again from the whole text off the UI thread;
//! a computation that started before the last edit is thrown away. Lines
//! of a modified range that differ only in whitespace get the whitespace
//! colour (`RangesBuilder.createInnerRanges`).

use std::ops::Range as Lines;
use std::sync::Arc;
use std::time::{Duration, Instant};

use editor::{ChangeKind, Decoration, DecorationId, Editor, EditorEvent};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, Pixels, Subscription, Task, WeakEntity,
};
use imara_diff::{Algorithm, Diff, InternedInput, Interner};
use text::{Buffer, CaretPlacement, Edit, EditKind, SelectionSet};
use themes::UiRole;
use vcs::{LineRanges, Range, RangeKind, split_lines};

/// The decoration group of the change markers.
pub const GROUP: &str = "vcs";

/// A piece of a changed range as its marker draws it: document lines
/// relative to the range's first line (an empty piece is a boundary of
/// deleted lines) and what changed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub start: u32,
    pub end: u32,
    pub kind: ChangeKind,
}

/// The pieces of a range: the whole range in its kind's colour, except
/// for a modified range, whose lines that are equal to base lines when
/// whitespace is ignored are drawn in the whitespace colour.
/// `base` and `doc` are the range's lines of the base text and of the
/// document.
pub fn parts(range: &Range, base: &[&str], doc: &[&str]) -> Vec<Part> {
    let len = range.line2 - range.line1;
    match range.kind() {
        RangeKind::Inserted => vec![Part {
            start: 0,
            end: len,
            kind: ChangeKind::Added,
        }],
        RangeKind::Deleted => vec![Part {
            start: 0,
            end: 0,
            kind: ChangeKind::Deleted,
        }],
        RangeKind::Modified => inner_parts(base, doc),
    }
}

fn without_whitespace(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The reference's inner ranges: the lines compared with whitespace
/// ignored; equal ones changed only in whitespace.
fn inner_parts(base: &[&str], doc: &[&str]) -> Vec<Part> {
    let before: Vec<String> = base.iter().map(|l| without_whitespace(l)).collect();
    let after: Vec<String> = doc.iter().map(|l| without_whitespace(l)).collect();
    let mut input = InternedInput {
        before: Vec::with_capacity(before.len()),
        after: Vec::with_capacity(after.len()),
        interner: Interner::new(before.len() + after.len()),
    };
    input.update_before(before.iter().map(String::as_str));
    input.update_after(after.iter().map(String::as_str));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let mut out: Vec<Part> = Vec::new();
    let mut push = |start: u32, end: u32, kind: ChangeKind| {
        if let Some(last) = out.last_mut()
            && last.kind == kind
            && last.end == start
            && start != end
            && last.start != last.end
        {
            last.end = end;
            return;
        }
        out.push(Part { start, end, kind });
    };
    let mut at = 0u32;
    for h in diff.hunks() {
        if h.after.start > at {
            push(at, h.after.start, ChangeKind::Whitespace);
        }
        let kind = match (h.before.is_empty(), h.after.is_empty()) {
            (false, false) => ChangeKind::Modified,
            (false, true) => ChangeKind::Deleted,
            _ => ChangeKind::Added,
        };
        push(h.after.start, h.after.end, kind);
        at = h.after.end;
    }
    let len = doc.len() as u32;
    if len > at {
        push(at, len, ChangeKind::Whitespace);
    }
    out
}

/// A document line selects a range: one of its lines, or the boundary
/// of deleted lines (`DiffUtil.isSelectedByLine`).
pub fn selected_by_line(range: &Range, line: u32) -> bool {
    if range.line1 == range.line2 {
        line == range.line1
    } else {
        range.line1 <= line && line < range.line2
    }
}

/// The range after line `line` (`getNextRange`).
pub fn next_range(ranges: &[Range], line: u32) -> Option<usize> {
    ranges
        .iter()
        .position(|r| line < r.line2 && !selected_by_line(r, line))
}

/// The range before line `line` (`getPrevRange`).
pub fn previous_range(ranges: &[Range], line: u32) -> Option<usize> {
    ranges
        .iter()
        .rposition(|r| line > r.line1 && !selected_by_line(r, line))
}

/// The range a line selects.
pub fn range_at_line(ranges: &[Range], line: u32) -> Option<usize> {
    ranges.iter().position(|r| selected_by_line(r, line))
}

/// The ranges that lines select (`lines`: inclusive spans).
pub fn ranges_for_lines(ranges: &[Range], lines: &[(u32, u32)]) -> Vec<usize> {
    ranges
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            lines.iter().any(|&(first, last)| {
                if r.line1 == r.line2 {
                    first <= r.line1 && r.line1 <= last
                } else {
                    first < r.line2 && r.line1 <= last
                }
            })
        })
        .map(|(i, _)| i)
        .collect()
}

/// The line the caret goes to for a range (`moveToRange`): its last
/// line, or the line after deleted lines.
pub fn target_line(range: &Range, len_lines: usize) -> usize {
    let line = if range.line1 == range.line2 {
        range.line2
    } else {
        range.line2 - 1
    } as usize;
    line.min(len_lines.saturating_sub(1))
}

/// The lines of the selections, as inclusive spans: from a selection's
/// first to its last line, the caret's line for a caret; a selection or
/// caret at the end of the text also takes the boundary after the last
/// line, where deleted last lines are (`DiffUtil.getSelectedLines`).
pub fn selected_lines(buffer: &Buffer, selections: &SelectionSet) -> Vec<(u32, u32)> {
    let len = buffer.len_chars();
    selections
        .iter()
        .map(|s| {
            let range = s.range();
            let first = buffer.char_to_line(range.start) as u32;
            let mut last = buffer.char_to_line(range.end) as u32;
            if range.end == len {
                last = buffer.len_lines() as u32;
            }
            (first, last)
        })
        .collect()
}

/// The edit that puts a range's base lines back: `base` is every line of
/// the base text.
pub fn rollback_edit(buffer: &Buffer, base: &[&str], range: &Range) -> Edit {
    let count = buffer.len_lines();
    let (l1, l2) = (range.line1 as usize, range.line2 as usize);
    let v1 = (range.vcs_line1 as usize).min(base.len());
    let v2 = (range.vcs_line2 as usize).min(base.len()).max(v1);
    let lines = base[v1..v2].join("\n");
    if l1 < l2 {
        if v1 < v2 {
            Edit::new(buffer.line_start(l1)..buffer.line_end(l2 - 1), lines)
        } else if l2 < count {
            Edit::delete(buffer.line_start(l1)..buffer.line_start(l2))
        } else if l1 > 0 {
            Edit::delete(buffer.line_end(l1 - 1)..buffer.len_chars())
        } else {
            Edit::delete(0..buffer.len_chars())
        }
    } else if l1 < count {
        Edit::insert(buffer.line_start(l1), lines + "\n")
    } else {
        Edit::insert(buffer.len_chars(), format!("\n{lines}"))
    }
}

/// The colour of a range's mark on the error stripe
/// (`LineStatusMarkerColorScheme.getErrorStripeColor`).
pub fn stripe_role(kind: RangeKind) -> UiRole {
    match kind {
        RangeKind::Inserted => UiRole::StripeAdded,
        RangeKind::Deleted => UiRole::StripeDeleted,
        RangeKind::Modified => UiRole::StripeModified,
    }
}

/// The decorations of ranges and their pieces: document lines, what to
/// draw, and the range each belongs to.
pub fn decorations(
    ranges: &[Range],
    parts: &[Vec<Part>],
) -> Vec<(Lines<usize>, Decoration, usize)> {
    let mut out = Vec::new();
    for (i, r) in ranges.iter().enumerate() {
        let stripe = stripe_role(r.kind());
        let whole = [Part {
            start: 0,
            end: r.line2 - r.line1,
            kind: match r.kind() {
                RangeKind::Inserted => ChangeKind::Added,
                RangeKind::Deleted => ChangeKind::Deleted,
                RangeKind::Modified => ChangeKind::Modified,
            },
        }];
        let pieces = parts.get(i).filter(|p| !p.is_empty()).map(Vec::as_slice);
        for p in pieces.unwrap_or(&whole) {
            let start = (r.line1 + p.start) as usize;
            let end = (r.line1 + p.end) as usize;
            out.push((
                start..end,
                Decoration::default().change(p.kind).stripe(stripe, true),
                i,
            ));
        }
    }
    out
}

/// Ranges and pieces computed from the texts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Computed {
    pub ranges: Vec<Range>,
    pub parts: Vec<Vec<Part>>,
}

impl Computed {
    /// The ranges of `text` against `base` with their pieces.
    pub fn new(base: &str, text: &str) -> Self {
        let ranges = LineRanges::compute(base, text).ranges().to_vec();
        let base_lines: Vec<&str> = split_lines(base).collect();
        let doc_lines: Vec<&str> = split_lines(text).collect();
        let parts = ranges
            .iter()
            .map(|r| {
                let b = &base_lines[(r.vcs_line1 as usize).min(base_lines.len())
                    ..(r.vcs_line2 as usize).min(base_lines.len())];
                let d = &doc_lines[(r.line1 as usize).min(doc_lines.len())
                    ..(r.line2 as usize).min(doc_lines.len())];
                parts(r, b, d)
            })
            .collect();
        Self { ranges, parts }
    }
}

/// What a [`LineStatus`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum LineStatusEvent {
    /// The ranges, the unsaved state or the base text changed.
    Changed,
    /// A change marker was pressed: range `index`, the popup goes under
    /// document line `line` (the line under the mouse).
    MarkerClicked { index: usize, line: usize },
}

/// The change markers of one editor.
pub struct LineStatus {
    editor: WeakEntity<Editor>,
    base: Option<Arc<str>>,
    ranges: LineRanges,
    parts: Vec<Vec<Part>>,
    /// Decorations drawn, with the range each belongs to.
    markers: Vec<(DecorationId, usize)>,
    /// Changes with every new base text.
    generation: u64,
    computing: Option<Task<()>>,
    /// The text changed while a computation ran.
    again: bool,
    /// The first edit not yet in computed ranges, and the time from the
    /// last such edit to the ranges drawn from a computation.
    edited_at: Option<Instant>,
    latency: Option<Duration>,
    _events: Subscription,
}

impl EventEmitter<LineStatusEvent> for LineStatus {}

impl LineStatus {
    pub fn new(editor: &Entity<Editor>, cx: &mut Context<Self>) -> Self {
        let events = cx.subscribe(
            editor,
            |this, editor, event: &EditorEvent, cx| match event {
                EditorEvent::Edited(applied) => this.edited(applied, cx),
                EditorEvent::Reloaded => this.text_replaced(cx),
                EditorEvent::ChangeMarkerClicked { id, position } => {
                    this.marker_clicked(&editor, *id, position.y, cx)
                }
                _ => {}
            },
        );
        Self {
            editor: editor.downgrade(),
            base: None,
            ranges: LineRanges::default(),
            parts: Vec::new(),
            markers: Vec::new(),
            generation: 0,
            computing: None,
            again: false,
            edited_at: None,
            latency: None,
            _events: events,
        }
    }

    /// The editor whose markers these are.
    pub fn editor(&self) -> Option<Entity<Editor>> {
        self.editor.upgrade()
    }

    /// Sets the text the document is compared with (`None`: no markers,
    /// a file without a committed version).
    pub fn set_base(&mut self, base: Option<Arc<str>>, cx: &mut Context<Self>) {
        if base.as_deref() == self.base.as_deref() {
            return;
        }
        self.base = base;
        self.generation += 1;
        self.computing = None;
        self.again = false;
        if self.base.is_none() {
            self.ranges = LineRanges::default();
            self.parts.clear();
            self.draw(cx);
            cx.emit(LineStatusEvent::Changed);
        } else {
            self.compute(cx);
        }
    }

    pub fn base(&self) -> Option<&Arc<str>> {
        self.base.as_ref()
    }

    /// The changed ranges, in document order.
    pub fn ranges(&self) -> &[Range] {
        self.ranges.ranges()
    }

    /// The pieces of each range.
    pub fn parts(&self) -> &[Vec<Part>] {
        &self.parts
    }

    /// Whether the text differs from the base text (`None` without one).
    pub fn changed(&self) -> Option<bool> {
        self.base.as_ref().map(|_| !self.ranges.is_empty())
    }

    /// The editor has unsaved changes.
    pub fn unsaved(&self, cx: &gpui::App) -> bool {
        self.editor
            .upgrade()
            .is_some_and(|e| e.read(cx).is_modified())
    }

    /// The ranges are being computed again.
    pub fn is_computing(&self) -> bool {
        self.computing.is_some()
    }

    /// The time from the last edit to its ranges drawn.
    pub fn latency(&self) -> Option<Duration> {
        self.latency
    }

    /// The range whose identity is `range` (the same lines, as the
    /// reference's `findRange` looks for a popup's range again).
    pub fn find(&self, range: &Range) -> Option<usize> {
        self.ranges().iter().position(|r| r == range)
    }

    /// The base lines of a range, each followed by a line break (what
    /// Copy puts on the clipboard).
    pub fn base_lines(&self, range: &Range) -> Option<String> {
        let base = self.base.as_ref()?;
        let lines: Vec<&str> = split_lines(base)
            .skip(range.vcs_line1 as usize)
            .take((range.vcs_line2 - range.vcs_line1) as usize)
            .collect();
        Some(lines.join("\n") + "\n")
    }

    /// Puts the base lines of ranges back as one undoable edit; the caret
    /// goes to the start of the first range (`moveCaretToLineRangeIfNeeded`
    /// moves it there, the replaced text takes it there otherwise).
    pub fn rollback(&mut self, indices: &[usize], cx: &mut Context<Self>) -> bool {
        let (Some(base), Some(editor)) = (self.base.clone(), self.editor.upgrade()) else {
            return false;
        };
        let ranges: Vec<Range> = indices
            .iter()
            .filter_map(|&i| self.ranges().get(i).copied())
            .collect();
        if ranges.is_empty() {
            return false;
        }
        let base_lines: Vec<&str> = split_lines(&base).collect();
        editor.update(cx, |e, cx| {
            let buffer = e.buffer();
            let mut edits: Vec<Edit> = ranges
                .iter()
                .map(|r| rollback_edit(buffer, &base_lines, r))
                .collect();
            edits.sort_by_key(|e| e.range.start);
            // Text before the first edit does not move.
            let first = ranges.iter().map(|r| r.line1).min().unwrap_or(0) as usize;
            let start = buffer.line_start(first.min(buffer.len_lines() - 1));
            let place = CaretPlacement::Explicit(SelectionSet::caret(start));
            e.edit(edits, EditKind::Other, place, cx);
        });
        true
    }

    /// The ranges that the editor's selections touch.
    pub fn selected(&self, cx: &gpui::App) -> Vec<usize> {
        let Some(editor) = self.editor.upgrade() else {
            return Vec::new();
        };
        let e = editor.read(cx);
        ranges_for_lines(self.ranges(), &selected_lines(e.buffer(), e.selections()))
    }

    /// The editor's caret line.
    pub fn caret_line(&self, cx: &gpui::App) -> Option<u32> {
        let editor = self.editor.upgrade()?;
        let e = editor.read(cx);
        Some(e.buffer().char_to_line(e.selections().primary().head) as u32)
    }

    /// Moves the caret to a range (its last line) and scrolls it to the
    /// middle of the view; returns the line.
    pub fn go_to(&mut self, index: usize, cx: &mut Context<Self>) -> Option<usize> {
        let range = *self.ranges().get(index)?;
        let editor = self.editor.upgrade()?;
        let line = target_line(&range, editor.read(cx).buffer().len_lines());
        editor.update(cx, |e, cx| e.go_to(line + 1, None, cx));
        Some(line)
    }

    // ---- tracking -------------------------------------------------------

    fn edited(&mut self, applied: &[text::AppliedEdit], cx: &mut Context<Self>) {
        if self.base.is_none() {
            cx.emit(LineStatusEvent::Changed);
            return;
        }
        // Markers follow at once: the edited lines are changed until the
        // ranges are computed again.
        let before: Vec<(Range, Vec<Part>)> = self
            .ranges()
            .iter()
            .copied()
            .zip(self.parts.iter().cloned())
            .collect();
        for e in applied {
            let start = e.start_position.0 as u32;
            self.ranges.edited(
                start,
                e.old_end_position.0 as u32 + 1,
                e.new_end_position.0 as u32 + 1,
            );
        }
        self.parts = self
            .ranges()
            .iter()
            .map(|r| {
                before
                    .iter()
                    .find(|(b, _)| {
                        b.vcs_line1 == r.vcs_line1
                            && b.vcs_line2 == r.vcs_line2
                            && b.line2 - b.line1 == r.line2 - r.line1
                    })
                    .map(|(_, p)| p.clone())
                    .unwrap_or_default()
            })
            .collect();
        self.draw(cx);
        self.edited_at = Some(Instant::now());
        self.compute(cx);
        cx.emit(LineStatusEvent::Changed);
    }

    fn text_replaced(&mut self, cx: &mut Context<Self>) {
        if self.base.is_some() {
            self.compute(cx);
        }
        cx.emit(LineStatusEvent::Changed);
    }

    /// Computes the ranges from the whole text off the UI thread.
    fn compute(&mut self, cx: &mut Context<Self>) {
        let (Some(base), Some(editor)) = (self.base.clone(), self.editor.upgrade()) else {
            return;
        };
        if self.computing.is_some() {
            self.again = true;
            return;
        }
        let buffer = editor.read(cx).buffer();
        let version = buffer.version();
        let rope = buffer.rope().clone();
        let generation = self.generation;
        let work = cx.background_spawn(async move { Computed::new(&base, &rope.to_string()) });
        self.computing = Some(cx.spawn(async move |this, cx| {
            let computed = work.await;
            let _ = this.update(cx, |this, cx| {
                this.computing = None;
                if this.generation != generation {
                    return;
                }
                let current = this.editor.upgrade().map(|e| e.read(cx).buffer().version());
                if this.again || current != Some(version) {
                    this.again = false;
                    this.compute(cx);
                    return;
                }
                this.apply(computed, cx);
            });
        }));
    }

    fn apply(&mut self, computed: Computed, cx: &mut Context<Self>) {
        let same = computed.ranges == self.ranges() && computed.parts == self.parts;
        self.ranges = LineRanges::from_ranges(computed.ranges);
        self.parts = computed.parts;
        if !same || self.markers.is_empty() != self.ranges.is_empty() {
            self.draw(cx);
        }
        if let Some(at) = self.edited_at.take() {
            self.latency = Some(at.elapsed());
        }
        cx.emit(LineStatusEvent::Changed);
    }

    /// Replaces the editor's change markers with the ranges'.
    fn draw(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let items = decorations(self.ranges(), &self.parts);
        let owners: Vec<usize> = items.iter().map(|(_, _, i)| *i).collect();
        let items = items.into_iter().map(|(l, d, _)| (l, d)).collect();
        let ids = editor.update(cx, |e, cx| e.set_decorations(GROUP, items, cx));
        self.markers = ids.into_iter().zip(owners).collect();
    }

    fn marker_clicked(
        &mut self,
        editor: &Entity<Editor>,
        id: DecorationId,
        y: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(&(_, index)) = self.markers.iter().find(|(m, _)| *m == id) else {
            return;
        };
        let e = editor.read(cx);
        let line = e
            .text_area_bounds()
            .and_then(|area| e.offset_at(gpui::point(area.left() + gpui::px(1.0), y)))
            .map(|offset| e.buffer().char_to_line(offset))
            .or_else(|| {
                let r = self.ranges().get(index)?;
                Some(target_line(r, e.buffer().len_lines()))
            });
        if let Some(line) = line {
            cx.emit(LineStatusEvent::MarkerClicked { index, line });
        }
    }

    /// The ranges as JSON (`workbench.state`).
    pub fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "base": self.base.is_some(),
            "computing": self.computing.is_some(),
            "latency_ms": self.latency.map(|d| d.as_secs_f64() * 1000.0),
            "ranges": self.ranges().iter().zip(self.parts.iter().chain(std::iter::repeat(&Vec::new()))).map(|(r, parts)| {
                serde_json::json!({
                    "lines": [r.line1, r.line2],
                    "vcs_lines": [r.vcs_line1, r.vcs_line2],
                    "kind": match r.kind() {
                        RangeKind::Inserted => "added",
                        RangeKind::Deleted => "deleted",
                        RangeKind::Modified => "modified",
                    },
                    "parts": parts.iter().map(|p| serde_json::json!({
                        "lines": [r.line1 + p.start, r.line1 + p.end],
                        "kind": p.kind.name(),
                    })).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(base: &str, text: &str) -> Vec<Vec<(u32, u32, &'static str)>> {
        Computed::new(base, text)
            .parts
            .iter()
            .map(|p| p.iter().map(|p| (p.start, p.end, p.kind.name())).collect())
            .collect()
    }

    #[test]
    fn whitespace_only_lines_get_their_own_colour() {
        // An indented line.
        assert_eq!(
            kinds("a\n  b\nc\n", "a\n    b\nc\n"),
            vec![vec![(0, 1, "whitespace")]]
        );
        // A real change and an indented line in one range.
        assert_eq!(
            kinds("a\nb\nc\nd\n", "a\nB\n  c\nd\n"),
            vec![vec![(0, 1, "modified"), (1, 2, "whitespace")]]
        );
        // Added and deleted lines keep their kind.
        assert_eq!(kinds("a\n", "a\nb\n"), vec![vec![(0, 1, "added")]]);
        assert_eq!(kinds("a\nb\n", "a\n"), vec![vec![(0, 0, "deleted")]]);
    }

    #[test]
    fn next_and_previous_follow_the_reference() {
        // Modified line 2, added lines 5..7, deleted lines before line 9.
        let ranges = [
            Range::new(2, 3, 2, 3),
            Range::new(5, 7, 5, 5),
            Range::new(9, 9, 7, 8),
        ];
        assert_eq!(next_range(&ranges, 0), Some(0));
        assert_eq!(next_range(&ranges, 2), Some(1));
        assert_eq!(next_range(&ranges, 6), Some(2));
        assert_eq!(next_range(&ranges, 9), None);
        assert_eq!(previous_range(&ranges, 12), Some(2));
        assert_eq!(previous_range(&ranges, 9), Some(1));
        assert_eq!(previous_range(&ranges, 6), Some(0));
        assert_eq!(previous_range(&ranges, 2), None);
        assert_eq!(range_at_line(&ranges, 9), Some(2));
        assert_eq!(range_at_line(&ranges, 4), None);
        assert_eq!(target_line(&ranges[1], 20), 6);
        assert_eq!(target_line(&ranges[2], 20), 9);
        assert_eq!(target_line(&ranges[2], 5), 4);
        assert_eq!(ranges_for_lines(&ranges, &[(2, 2)]), vec![0]);
        assert_eq!(ranges_for_lines(&ranges, &[(3, 5), (9, 9)]), vec![1, 2]);
        assert_eq!(ranges_for_lines(&ranges, &[(3, 4)]), Vec::<usize>::new());
    }

    #[test]
    fn selected_lines_take_the_end_of_the_text() {
        let buffer = Buffer::new("a\nb\nc\n");
        let caret = |at| SelectionSet::caret(at);
        assert_eq!(selected_lines(&buffer, &caret(2)), vec![(1, 1)]);
        let all = SelectionSet::single(text::Selection::new(0, buffer.len_chars()));
        assert_eq!(selected_lines(&buffer, &all), vec![(0, 4)]);
        // Lines deleted at the end are selected from the end of the text.
        let ranges = [Range::new(4, 4, 4, 5)];
        assert_eq!(
            ranges_for_lines(&ranges, &selected_lines(&buffer, &caret(6))),
            vec![0]
        );
    }

    /// Rolling back every range gives the base text, for changes at the
    /// start, in the middle and at the end, with and without a final line
    /// break.
    #[test]
    fn rollback_restores_the_base_text() {
        let cases = [
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nb", "a"),
            ("a", "a\nb"),
            ("a\n", "a\nb\n"),
            ("a\nb", "a\nB"),
            ("a\nb\nc", "a\nX\nY\nc"),
            ("", "x\n"),
            ("x\n", ""),
            ("a\nb\nc\nd\ne\n", "z\na\nB\nc\ne\nf\n"),
            ("one\ntwo\n", "zero\none\ntwo\nthree"),
        ];
        for (base, text) in cases {
            let ranges = LineRanges::compute(base, text).ranges().to_vec();
            let base_lines: Vec<&str> = split_lines(base).collect();
            // One at a time, from the last range up.
            let mut buffer = Buffer::new(text);
            for r in ranges.iter().rev() {
                let edit = rollback_edit(&buffer, &base_lines, r);
                let mut sel = SelectionSet::caret(0);
                buffer.transact(
                    &mut sel,
                    vec![edit],
                    EditKind::Other,
                    CaretPlacement::AfterEachEdit,
                );
            }
            assert_eq!(buffer.text(), base, "one by one: {text:?} back to {base:?}");
            // All at once.
            let mut buffer = Buffer::new(text);
            let mut edits: Vec<Edit> = ranges
                .iter()
                .map(|r| rollback_edit(&buffer, &base_lines, r))
                .collect();
            edits.sort_by_key(|e| e.range.start);
            let mut sel = SelectionSet::caret(0);
            buffer.transact(
                &mut sel,
                edits,
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
            );
            assert_eq!(buffer.text(), base, "at once: {text:?} back to {base:?}");
        }
    }

    #[test]
    fn decorations_cover_the_pieces_and_mark_the_stripe() {
        let c = Computed::new("a\nb\nc\nd\n", "a\nB\n  c\nd\nx\n");
        let items = decorations(&c.ranges, &c.parts);
        let lines: Vec<(Lines<usize>, Option<ChangeKind>, usize)> = items
            .iter()
            .map(|(l, d, i)| (l.clone(), d.change, *i))
            .collect();
        assert_eq!(
            lines,
            vec![
                (1..2, Some(ChangeKind::Modified), 0),
                (2..3, Some(ChangeKind::Whitespace), 0),
                (4..5, Some(ChangeKind::Added), 1),
            ]
        );
        assert_eq!(
            items[0].1.stripe.as_ref().map(|s| s.color),
            Some(UiRole::StripeModified)
        );
        assert_eq!(
            items[2].1.stripe.as_ref().map(|s| s.color),
            Some(UiRole::StripeAdded)
        );
    }
}
