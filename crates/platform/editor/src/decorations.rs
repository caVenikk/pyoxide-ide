//! Decorations an editor's owners paint through it, as the reference's
//! range highlighters of the debugger and version control do: an icon
//! over a line number (a breakpoint), a change marker between the line
//! numbers and the text, a full-line background, texts after the end of a
//! line (inline values) and marks on the error stripe.
//!
//! A decoration is anchored to lines and moves with the text through
//! edits, as the reference's persistent line highlighters do: lines
//! inserted above move it down, edits within its lines keep it, and it is
//! gone when its lines are deleted. Owners keep the [`DecorationId`]s they
//! get, or replace a whole group at once ([`Editor::set_decorations`]).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{Bounds, Context, Modifiers, MouseButton, Pixels, SharedString};
use text::{AppliedEdit, Buffer};
use themes::{TextRole, UiRole};
use ui::icons::IconName;

use crate::diagnostics::move_range;
use crate::editor::{Editor, EditorEvent};

/// Identifies a decoration of an editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DecorationId(pub u64);

/// Layers of full-line backgrounds, the reference's highlighter layers: a
/// background of a higher layer is drawn over a lower one, all of them
/// over the caret row and under the selection.
pub mod layer {
    /// The caret row (`HighlighterLayer.CARET_ROW`).
    pub const CARET_ROW: i32 = 1000;
    /// Lines with a breakpoint (`DebuggerColors.BREAKPOINT_HIGHLIGHTER_LAYER`).
    pub const BREAKPOINT: i32 = 2980;
    /// The line the debugger stopped on
    /// (`DebuggerColors.EXECUTION_LINE_HIGHLIGHTERLAYER`).
    pub const EXECUTION_POINT: i32 = 5998;
}

/// An icon drawn over the line number of a decoration's first line in
/// place of the number (the reference's breakpoints over line numbers).
#[derive(Debug, Clone, PartialEq)]
pub struct GutterIcon {
    pub icon: IconName,
    pub tooltip: Option<SharedString>,
}

/// A background over the whole width of the text on every row of a
/// decoration's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineBackground {
    pub color: UiRole,
    /// See [`layer`].
    pub layer: i32,
}

/// What changed in a decoration's lines, for its change marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Modified,
    /// Lines were deleted between two lines: the decoration has no lines
    /// and its marker sits on the boundary above its line.
    Deleted,
    /// Lines that differ only in whitespace.
    Whitespace,
}

impl ChangeKind {
    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Whitespace => "whitespace",
        }
    }

    pub(crate) fn color(self) -> UiRole {
        match self {
            ChangeKind::Added => UiRole::GutterAdded,
            ChangeKind::Modified => UiRole::GutterModified,
            ChangeKind::Deleted => UiRole::GutterDeleted,
            ChangeKind::Whitespace => UiRole::GutterWhitespaceModified,
        }
    }
}

/// One styled piece of an [`AfterLineText`].
#[derive(Debug, Clone, PartialEq)]
pub struct TextPiece {
    pub text: SharedString,
    /// Colours and font style of the theme role.
    pub role: TextRole,
    /// Italic whatever the role says.
    pub italic: bool,
}

impl TextPiece {
    pub fn new(text: impl Into<SharedString>, role: TextRole) -> Self {
        Self {
            text: text.into(),
            role,
            italic: false,
        }
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// Text drawn after the end of a decoration's last line, as the
/// reference's inline debug renderer draws a value: one space after the
/// line end (or after the previous text of the line), the text half a
/// space further. A clickable text is drawn as a link under the mouse and
/// reports clicks ([`crate::EditorEvent::AfterLineTextClicked`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AfterLineText {
    pub pieces: Vec<TextPiece>,
    pub clickable: bool,
}

/// A mark on the error stripe from a decoration's first to its last line.
#[derive(Debug, Clone, PartialEq)]
pub struct StripeMark {
    pub color: UiRole,
    /// Drawn narrow at the left of the stripe, as the reference draws the
    /// marks of changed lines; else across the stripe like a problem's.
    pub thin: bool,
}

/// What a decoration draws; any combination of parts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decoration {
    pub gutter_icon: Option<GutterIcon>,
    pub background: Option<LineBackground>,
    pub change: Option<ChangeKind>,
    pub after_text: Option<AfterLineText>,
    pub stripe: Option<StripeMark>,
}

impl Decoration {
    pub fn gutter_icon(mut self, icon: IconName, tooltip: Option<SharedString>) -> Self {
        self.gutter_icon = Some(GutterIcon { icon, tooltip });
        self
    }

    pub fn background(mut self, color: UiRole, layer: i32) -> Self {
        self.background = Some(LineBackground { color, layer });
        self
    }

    pub fn change(mut self, kind: ChangeKind) -> Self {
        self.change = Some(kind);
        self
    }

    pub fn after_text(mut self, pieces: Vec<TextPiece>, clickable: bool) -> Self {
        self.after_text = Some(AfterLineText { pieces, clickable });
        self
    }

    pub fn stripe(mut self, color: UiRole, thin: bool) -> Self {
        self.stripe = Some(StripeMark { color, thin });
        self
    }
}

/// A decoration with its anchor in the text.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub id: DecorationId,
    /// Who owns it, for replacing or clearing all of an owner's.
    pub group: &'static str,
    /// Char range of its lines with their line breaks (the last line of
    /// the text has none, so an empty last line has an empty range).
    pub range: Range<usize>,
    /// Between two lines (no lines of its own).
    pub between: bool,
    pub decoration: Decoration,
}

/// The lines of every decoration, by index.
pub(crate) type EntryLines = Rc<Vec<Range<usize>>>;

/// The decorations of an editor, in the order they were added, moved
/// through edits.
#[derive(Debug, Default)]
pub(crate) struct Decorations {
    entries: Vec<Entry>,
    next_id: u64,
    /// Changes whenever the lines of the decorations may have changed.
    generation: u64,
    lines_cache: RefCell<Option<(u64, EntryLines)>>,
}

/// The char range anchoring lines `lines` (end exclusive; empty: the
/// boundary above `lines.start`), clamped to the text.
fn anchor(buffer: &Buffer, lines: Range<usize>) -> Range<usize> {
    let count = buffer.len_lines();
    let first = lines.start.min(count - 1);
    let start = buffer.line_start(first);
    if lines.end <= lines.start {
        return start..start;
    }
    let end = if lines.end >= count {
        buffer.len_chars()
    } else {
        buffer.line_start(lines.end)
    };
    start..end.max(start)
}

impl Decorations {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn add(
        &mut self,
        group: &'static str,
        lines: Range<usize>,
        decoration: Decoration,
        buffer: &Buffer,
    ) -> DecorationId {
        self.next_id += 1;
        let id = DecorationId(self.next_id);
        self.entries.push(Entry {
            id,
            group,
            between: lines.is_empty(),
            range: anchor(buffer, lines),
            decoration,
        });
        self.generation += 1;
        id
    }

    pub fn remove(&mut self, id: DecorationId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() != before;
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Removes every decoration of `group`; returns whether there were any.
    pub fn clear_group(&mut self, group: &'static str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.group != group);
        let removed = self.entries.len() != before;
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Replaces what a decoration draws; returns whether it exists.
    pub fn update(&mut self, id: DecorationId, decoration: Decoration) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.decoration = decoration;
                self.generation += 1;
                true
            }
            None => false,
        }
    }

    pub fn index_of(&self, id: DecorationId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// The lines of every decoration (by index): end exclusive, empty for
    /// a decoration between lines. Kept while edits stay within lines.
    pub fn lines(&self, buffer: &Buffer) -> EntryLines {
        let mut cache = self.lines_cache.borrow_mut();
        if let Some((generation, lines)) = cache.as_ref()
            && *generation == self.generation
        {
            return lines.clone();
        }
        let lines: EntryLines = Rc::new(
            self.entries
                .iter()
                .map(|e| entry_lines(buffer, &e.range, e.between))
                .collect(),
        );
        *cache = Some((self.generation, lines.clone()));
        lines
    }

    /// Moves the decorations through applied edits (in the order they were
    /// applied). A decoration whose lines are deleted is gone; one between
    /// lines stays where it is.
    pub fn edited(&mut self, edits: &[AppliedEdit]) {
        if self.entries.is_empty() || edits.is_empty() {
            return;
        }
        let count = self.entries.len();
        let mut lines_moved = false;
        for e in edits {
            lines_moved |= e.start_position.0 != e.old_end_position.0
                || e.start_position.0 != e.new_end_position.0;
            let breaks = e.start_position.0 != e.old_end_position.0;
            self.entries
                .retain_mut(|d| match move_range(d.range.clone(), e) {
                    // Lines whose text went with their line breaks.
                    Some(r) if r.is_empty() && !d.range.is_empty() && breaks => false,
                    Some(r) => {
                        d.range = r;
                        true
                    }
                    None => false,
                });
        }
        if lines_moved || self.entries.len() != count {
            self.generation += 1;
        }
    }

    /// The text was replaced without known edits: every decoration is
    /// anchored again at the lines it had (as of the last time they were
    /// asked for), clamped to the new text.
    pub fn text_replaced(&mut self, buffer: &Buffer) {
        if self.entries.is_empty() {
            return;
        }
        let cached = self.lines_cache.borrow().as_ref().map(|(_, l)| l.clone());
        let count = buffer.len_lines();
        for (i, e) in self.entries.iter_mut().enumerate() {
            let lines = match cached.as_ref().and_then(|l| l.get(i)) {
                Some(l) => l.clone(),
                None => {
                    let len = buffer.len_chars();
                    let r = e.range.start.min(len)..e.range.end.min(len);
                    entry_lines(buffer, &r, e.between)
                }
            };
            let start = lines.start.min(count - 1);
            let lines = if e.between {
                start..start
            } else {
                start..lines.end.min(count).max(start + 1)
            };
            e.range = anchor(buffer, lines);
        }
        self.generation += 1;
    }
}

/// The lines a decoration's anchor covers.
fn entry_lines(buffer: &Buffer, range: &Range<usize>, between: bool) -> Range<usize> {
    let first = buffer.char_to_line(range.start);
    if between {
        return first..first;
    }
    if range.is_empty() {
        // The empty last line.
        return first..first + 1;
    }
    first..buffer.char_to_line(range.end - 1) + 1
}

// ---- the editor's API --------------------------------------------------

impl Editor {
    /// Adds a decoration over lines `lines` (zero-based, end exclusive; an
    /// empty range is the boundary above `lines.start`) for `group`.
    pub fn add_decoration(
        &mut self,
        group: &'static str,
        lines: Range<usize>,
        decoration: Decoration,
        cx: &mut Context<Self>,
    ) -> DecorationId {
        let id = self.decorations.add(group, lines, decoration, &self.buffer);
        cx.notify();
        id
    }

    /// Replaces every decoration of `group` with `items`; returns their ids
    /// in order.
    pub fn set_decorations(
        &mut self,
        group: &'static str,
        items: Vec<(Range<usize>, Decoration)>,
        cx: &mut Context<Self>,
    ) -> Vec<DecorationId> {
        self.decorations.clear_group(group);
        let ids = items
            .into_iter()
            .map(|(lines, d)| self.decorations.add(group, lines, d, &self.buffer))
            .collect();
        self.decoration_hover_reset();
        cx.notify();
        ids
    }

    /// Removes a decoration; returns whether it was there (a decoration
    /// whose lines were deleted is gone already).
    pub fn remove_decoration(&mut self, id: DecorationId, cx: &mut Context<Self>) -> bool {
        let removed = self.decorations.remove(id);
        if removed {
            self.decoration_hover_reset();
            cx.notify();
        }
        removed
    }

    /// Removes every decoration of `group`.
    pub fn clear_decorations(&mut self, group: &'static str, cx: &mut Context<Self>) {
        if self.decorations.clear_group(group) {
            self.decoration_hover_reset();
            cx.notify();
        }
    }

    /// Replaces what a decoration draws, keeping its place; returns whether
    /// it exists.
    pub fn update_decoration(
        &mut self,
        id: DecorationId,
        decoration: Decoration,
        cx: &mut Context<Self>,
    ) -> bool {
        let found = self.decorations.update(id, decoration);
        if found {
            cx.notify();
        }
        found
    }

    /// The lines a decoration is on now (zero-based, end exclusive; empty
    /// for one between lines), `None` when it is gone.
    pub fn decoration_lines(&self, id: DecorationId) -> Option<Range<usize>> {
        let i = self.decorations.index_of(id)?;
        self.decorations.lines(&self.buffer).get(i).cloned()
    }

    /// What a decoration draws, `None` when it is gone.
    pub fn decoration(&self, id: DecorationId) -> Option<&Decoration> {
        let i = self.decorations.index_of(id)?;
        Some(&self.decorations.entries()[i].decoration)
    }

    /// The decorations of `group` with their current lines, in the order
    /// they were added.
    pub fn decorations_of(&self, group: &str) -> Vec<(DecorationId, Range<usize>)> {
        let lines = self.decorations.lines(&self.buffer);
        self.decorations
            .entries()
            .iter()
            .zip(lines.iter())
            .filter(|(e, _)| e.group == group)
            .map(|(e, l)| (e.id, l.clone()))
            .collect()
    }

    /// Presses on the line numbers are reported as
    /// [`crate::EditorEvent::GutterClicked`] instead of moving the caret
    /// (whoever sets breakpoints there turns this on).
    pub fn set_line_number_clicks(&mut self, on: bool) {
        self.line_number_clicks = on;
    }

    /// The icon drawn translucent over the line number of `line` while the
    /// mouse is over it (the reference's breakpoint hover icon), set by the
    /// owner on [`crate::EditorEvent::GutterHovered`]; `None` removes it.
    pub fn set_gutter_hover_icon(
        &mut self,
        icon: Option<(usize, IconName)>,
        cx: &mut Context<Self>,
    ) {
        if self.gutter_hover_icon != icon {
            self.gutter_hover_icon = icon;
            cx.notify();
        }
    }

    /// The line whose line number the mouse is over, as last reported.
    pub fn gutter_hovered_line(&self) -> Option<usize> {
        self.gutter_hover
    }

    /// Window bounds of the line numbers' area on the first row of `line`
    /// in the last frame: where its gutter icon is drawn (popups about it
    /// go below). `None` before the first frame or without a gutter.
    pub fn line_number_bounds(&self, line: usize) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let lanes = layout.decorations.lanes;
        if lanes.numbers.1 <= lanes.numbers.0 || line >= self.buffer.len_lines() {
            return None;
        }
        let top =
            layout.bounds.top() + layout.line_height * self.line_row(line) as f32 - self.scroll.y;
        Some(Bounds::from_corners(
            gpui::point(lanes.numbers.0, top),
            gpui::point(lanes.numbers.1, top + layout.line_height),
        ))
    }

    /// Window bounds of a decoration's change marker as it reacts to the
    /// mouse in the last frame (`None` when it was not visible).
    pub fn change_marker_bounds(&self, id: DecorationId) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let g = &layout.decorations;
        let c = g.changes.iter().find(|c| c.id == id)?;
        Some(Bounds::from_corners(
            gpui::point(g.lanes.markers.0, c.top),
            gpui::point(g.lanes.markers.1, c.bottom),
        ))
    }

    /// Window bounds of a decoration's clickable after-line text in the last
    /// frame (`None` when it was not visible).
    pub fn after_text_bounds(&self, id: DecorationId) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        layout
            .decorations
            .after_texts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.bounds)
    }

    /// The decoration whose change marker the mouse is over.
    pub fn hovered_change_marker(&self) -> Option<DecorationId> {
        self.hovered_change
    }

    /// The decoration whose clickable after-line text the mouse is over.
    pub fn hovered_after_text(&self) -> Option<DecorationId> {
        self.hovered_after
    }

    /// The line under a window position over the line numbers in the
    /// last frame.
    fn gutter_line_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        if !layout.bounds.contains(&position) || !layout.decorations.lanes.in_numbers(position.x) {
            return None;
        }
        let y = position.y - layout.bounds.top() + self.scroll.y;
        let row = (y / layout.line_height).floor();
        if row < 0.0 || row as usize >= self.display_rows() {
            return None;
        }
        Some(self.row_line(row as usize).0)
    }

    /// A mouse press: on a change marker, a clickable after-line text or
    /// (when the owner takes them) the line numbers it is reported and
    /// taken; returns whether it was.
    pub(crate) fn decoration_press(
        &mut self,
        position: gpui::Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(layout) = &self.layout else {
            return false;
        };
        let geometry = &layout.decorations;
        if button == MouseButton::Left {
            if let Some(id) = geometry.change_at(position) {
                cx.emit(EditorEvent::ChangeMarkerClicked { id, position });
                return true;
            }
            if let Some(id) = geometry.after_text_at(position) {
                cx.emit(EditorEvent::AfterLineTextClicked { id, position });
                return true;
            }
        }
        if self.line_number_clicks
            && let Some(line) = self.gutter_line_at(position)
        {
            window.focus(self.focus_handle_ref());
            cx.emit(EditorEvent::GutterClicked {
                line,
                button,
                modifiers,
                position,
            });
            return true;
        }
        false
    }

    /// The mouse moved without a button: the line number, change marker
    /// and clickable after-line text under it.
    pub(crate) fn decoration_hover(
        &mut self,
        position: gpui::Point<Pixels>,
        over_editor: bool,
        cx: &mut Context<Self>,
    ) {
        let (line, change, after) = match self.layout.as_ref().filter(|_| over_editor) {
            Some(layout) => (
                self.gutter_line_at(position),
                layout.decorations.change_at(position),
                layout.decorations.after_text_at(position),
            ),
            None => (None, None, None),
        };
        if self.gutter_hover != line {
            self.gutter_hover = line;
            cx.emit(EditorEvent::GutterHovered { line });
            cx.notify();
        }
        if self.hovered_change != change || self.hovered_after != after {
            self.hovered_change = change;
            self.hovered_after = after;
            cx.notify();
        }
    }

    /// Hover states that refer to decorations by index or id.
    pub(crate) fn decoration_hover_reset(&mut self) {
        self.hovered_change = None;
        self.hovered_after = None;
    }

    /// `editor.sample-decorations`: what the debugger and version control
    /// draw, on the lines of the reference screenshots' sample file
    /// (clamped to the text), replacing the previous samples.
    pub(crate) fn sample_decorations(
        &mut self,
        _: &crate::commands::SampleDecorations,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let last = self.buffer.len_lines().saturating_sub(1);
        let line = |l: usize| l.min(last);
        let value = |name: &str, value: &str| {
            vec![
                TextPiece::new(format!("{name}: "), TextRole::InlayHint).italic(),
                TextPiece::new(value.to_owned(), TextRole::InlayHint).italic(),
            ]
        };
        let breakpoint = IconName::BREAKPOINT;
        let items = vec![
            (
                line(6)..line(6) + 1,
                Decoration::default()
                    .gutter_icon(breakpoint, Some("Line breakpoint".into()))
                    .background(UiRole::Breakpoint, layer::BREAKPOINT)
                    .stripe(UiRole::DiagnosticErrorStripe, false),
            ),
            (
                line(6)..line(6) + 1,
                Decoration::default().background(UiRole::ExecutionPoint, layer::EXECUTION_POINT),
            ),
            (
                line(3)..line(3) + 1,
                Decoration::default().after_text(value("items", "[1, 2, 3, 4]"), true),
            ),
            (
                line(4)..line(4) + 1,
                Decoration::default().after_text(value("summary", "{'count': 4, 'sum': 0}"), true),
            ),
            (
                line(5)..line(5) + 1,
                Decoration::default().after_text(value("value", "1"), true),
            ),
            (
                line(9)..line(9),
                Decoration::default()
                    .change(ChangeKind::Deleted)
                    .stripe(UiRole::GutterDeleted, true),
            ),
            (
                line(12)..line(14),
                Decoration::default()
                    .change(ChangeKind::Modified)
                    .stripe(UiRole::GutterModified, true),
            ),
            (
                line(21)..line(21) + 1,
                Decoration::default()
                    .change(ChangeKind::Added)
                    .stripe(UiRole::GutterAdded, true),
            ),
        ];
        self.set_decorations("sample", items, cx);
        self.line_number_clicks = true;
    }

    /// The decorations as JSON for `editor.state`.
    pub(crate) fn decorations_json(&self) -> serde_json::Value {
        let lines = self.decorations.lines(&self.buffer);
        serde_json::Value::Array(
            self.decorations
                .entries()
                .iter()
                .zip(lines.iter())
                .map(|(e, l)| {
                    let d = &e.decoration;
                    serde_json::json!({
                        "id": e.id.0,
                        "group": e.group,
                        "lines": [l.start, l.end],
                        "gutter_icon": d.gutter_icon.as_ref().map(|g| g.icon.0),
                        "background": d.background.map(|b| b.layer),
                        "change": d.change.map(ChangeKind::name),
                        "after_text": d.after_text.as_ref().map(|a| a
                            .pieces
                            .iter()
                            .map(|p| p.text.as_ref())
                            .collect::<String>()),
                        "stripe": d.stripe.as_ref().map(|s| if s.thin { "thin" } else { "wide" }),
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use text::{CaretPlacement, Edit, EditKind, SelectionSet};

    fn apply(buffer: &mut Buffer, d: &mut Decorations, range: Range<usize>, text: &str) {
        let mut sel = SelectionSet::caret(0);
        let applied = buffer.transact(
            &mut sel,
            vec![Edit::new(range, text)],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        d.edited(&applied);
    }

    fn lines_of(d: &Decorations, buffer: &Buffer) -> Vec<Range<usize>> {
        d.lines(buffer).iter().cloned().collect()
    }

    #[test]
    fn line_decorations_follow_lines_inserted_and_deleted_above() {
        let mut b = Buffer::new("a\nbb\nccc\ndddd\n");
        let mut d = Decorations::default();
        d.add("t", 2..3, Decoration::default(), &b);
        assert_eq!(lines_of(&d, &b), vec![2..3]);
        // A line inserted above moves it down.
        apply(&mut b, &mut d, 0..0, "new\n");
        assert_eq!(lines_of(&d, &b), vec![3..4]);
        // Enter at its start moves it down with its text.
        let start = b.line_start(3);
        apply(&mut b, &mut d, start..start, "\n");
        assert_eq!(lines_of(&d, &b), vec![4..5]);
        // Typing within the line and replacing its text keep it.
        let start = b.line_start(4);
        apply(&mut b, &mut d, start + 1..start + 1, "xx");
        apply(&mut b, &mut d, start..start + 5, "z");
        assert_eq!(lines_of(&d, &b), vec![4..5]);
        assert_eq!(b.line(4).to_string(), "z");
        // Deleting the lines above moves it up.
        let end = b.line_start(2);
        apply(&mut b, &mut d, 0..end, "");
        assert_eq!(lines_of(&d, &b), vec![2..3]);
    }

    #[test]
    fn a_decoration_is_gone_with_its_lines() {
        let mut b = Buffer::new("a\nbb\nccc\n");
        let mut d = Decorations::default();
        let gone = d.add("t", 1..2, Decoration::default(), &b);
        let kept = d.add("t", 2..3, Decoration::default(), &b);
        let between = d.add(
            "t",
            1..1,
            Decoration::default().change(ChangeKind::Deleted),
            &b,
        );
        // Delete line 1 with its line break.
        apply(&mut b, &mut d, 2..5, "");
        assert!(d.index_of(gone).is_none());
        assert!(d.index_of(kept).is_some());
        // The marker between lines stays at its boundary.
        assert!(d.index_of(between).is_some());
        assert_eq!(lines_of(&d, &b), vec![1..2, 1..1]);
    }

    #[test]
    fn several_lines_widen_and_narrow_with_their_text() {
        let mut b = Buffer::new("0\n1\n2\n3\n4\n");
        let mut d = Decorations::default();
        d.add(
            "t",
            1..3,
            Decoration::default().change(ChangeKind::Added),
            &b,
        );
        assert_eq!(lines_of(&d, &b), vec![1..3]);
        // A line inserted inside widens it.
        let at = b.line_start(2);
        apply(&mut b, &mut d, at..at, "x\n");
        assert_eq!(lines_of(&d, &b), vec![1..4]);
        // The last line of the text without a line break.
        let mut b = Buffer::new("0\n1");
        let mut d = Decorations::default();
        d.add("t", 1..2, Decoration::default(), &b);
        assert_eq!(lines_of(&d, &b), vec![1..2]);
        apply(&mut b, &mut d, 3..3, "23");
        assert_eq!(lines_of(&d, &b), vec![1..2]);
    }

    #[test]
    fn the_last_line_keeps_its_decoration_when_its_text_is_deleted() {
        let mut b = Buffer::new("a\nbc");
        let mut d = Decorations::default();
        d.add("t", 1..2, Decoration::default(), &b);
        apply(&mut b, &mut d, 2..4, "");
        assert_eq!(lines_of(&d, &b), vec![1..2]);
        apply(&mut b, &mut d, 2..2, "x");
        assert_eq!(lines_of(&d, &b), vec![1..2]);
        // With the line break before it, the line is gone.
        apply(&mut b, &mut d, 1..3, "");
        assert!(d.entries().is_empty());
    }

    #[test]
    fn groups_are_replaced_and_cleared_apart() {
        let b = Buffer::new("a\nb\nc\n");
        let mut d = Decorations::default();
        let git = d.add("git", 0..1, Decoration::default(), &b);
        let debug = d.add("debug", 1..2, Decoration::default(), &b);
        assert!(d.clear_group("git"));
        assert!(!d.clear_group("git"));
        assert!(d.index_of(git).is_none());
        assert!(d.index_of(debug).is_some());
        assert!(d.remove(debug));
        assert!(d.entries().is_empty());
    }

    #[test]
    fn a_replaced_text_keeps_the_lines_clamped() {
        let b = Buffer::new("a\nb\nc\nd\n");
        let mut d = Decorations::default();
        d.add("t", 3..4, Decoration::default(), &b);
        d.add("t", 1..2, Decoration::default(), &b);
        let _ = d.lines(&b);
        let b = Buffer::new("x\ny\n");
        d.text_replaced(&b);
        assert_eq!(lines_of(&d, &b), vec![2..3, 1..2]);
    }
}
