//! The diff viewer's entity: the toolbar (Previous and Next Difference,
//! Jump to Source, the previous and next file with the files counter; the
//! count of differences, the viewer's mode and the settings), the titles of
//! both sides, the two editors and the divider (`SimpleDiffViewer`,
//! `TwosideTextDiffViewer`, `DiffRequestProcessor`'s toolbar), and the
//! changes' actions in the gutters: `>>` by the left numbers reverts a
//! change, a check box by the right ones includes it into the commit
//! (`SimpleDiffChangeUi`, `LocalTrackerDiffUtil.createCheckboxToggle`).

use std::cell::Cell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use editor::side_by_side::DiffGutter;
use editor::{
    Decoration, DecorationId, DiffHighlights, Editor, EditorConfig, EditorEvent, GutterFill,
    Highlighter,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, Window, anchored,
    canvas, deferred, div, point, px,
};
use serde_json::{Value as Json, json};
use text::{Buffer, CaretPlacement, EditKind, Selection, SelectionSet};
use themes::UiRole;
use ui::components::{Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem};
use ui::theme::ThemeExt as _;

use super::divider::{self, change_role};
use super::model::{
    self, ChangeKind, DiffChange, DiffSettings, HighlightPolicy, IgnorePolicy, Side,
};
use super::{
    ApplyLeftSide, CONTEXT, FocusOpposite, IncludeChange, JumpToSource, NextDifference, NextFile,
    PreviousDifference, PreviousFile, SetHighlightPolicy, SetIgnorePolicy, ShowSettings,
};

/// The decorations' group in both editors.
const GROUP: &str = "diff";
/// Rows of room below the last one (`EditorSettings`' additional lines).
const ROOM_AFTER_END: usize = 5;
/// Edits of the current text are compared again after this long.
const REDIFF_DELAY: Duration = Duration::from_millis(50);
/// The toolbar (`DiffRequestProcessor`'s, measured on the reference): an
/// island (`DiffToolbarIslandPanelUI`: the search field's colours, corners
/// of 6 px) with room at its sides, its height with the borders, and the
/// room inside at its ends; the New UI's buttons of 26 px with icons of
/// 16; the separators, 1 px with room at both sides; the gap between the
/// count of differences and the viewer's modes, whose segments are 50 x 24,
/// and the gap after them.
const ISLAND_SIDE: f32 = 5.0;
const ISLAND_HEIGHT: f32 = 43.0;
const ISLAND_RADIUS: f32 = 6.0;
const TOOLBAR_LEFT: f32 = 5.5;
const TOOLBAR_RIGHT: f32 = 5.5;
const BUTTON: f32 = 26.0;
const ICON: f32 = 16.0;
const SEPARATOR_ROOM: f32 = 3.5;
const SEPARATOR_HEIGHT: f32 = 17.0;
const FILES_GAP: f32 = 1.5;
const STATUS_GAP: f32 = 13.5;
const MODE_SIZE: (f32, f32) = (48.0, 26.0);
const MODE_GAP: f32 = 10.5;
/// The titles (`DiffContentPanel`, measured on the reference): their
/// height above the tear line of 1 px; the room at their ends
/// (`Diff.ContentTitle.insets`) and the lock's; the gap between a revision
/// and its path (`ipadx` 8 and the label's own); the right title's check
/// box over the gutter's action column, its label this far before the
/// text.
const TITLE_HEIGHT: f32 = 30.0;
const TITLE_LEFT: f32 = 4.0;
const LOCK_ROOM: f32 = 16.0;
const PATH_GAP: f32 = 9.0;
const LABEL_AFTER_CHECKBOX: f32 = 27.3;
/// The check box's place before the editor has been laid out (two-digit
/// line numbers).
const CHECKBOX_FALLBACK: f32 = 30.0;
/// The tooltips of the gutter's actions (`action.presentation.diff.revert.text`,
/// `action.presentation.diff.include.into.commit.text`).
const REVERT: &str = "Revert";
const INCLUDE: &str = "Include into commit";

/// The title of a side (`DiffEditorTitleDetails`): a revision or a name,
/// then the file's path in grey.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SideTitle {
    pub label: SharedString,
    pub path: Option<SharedString>,
    /// The full path, the path's tooltip.
    pub tooltip: Option<SharedString>,
    /// The lock of a read-only side.
    pub read_only: bool,
}

/// What a diff shows: its file, its tab title, the sides' titles, and
/// where it is in the list it was opened for.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffRequest {
    /// The file whose current text is on the right.
    pub path: PathBuf,
    /// The tab's title (`Commit: main.py`).
    pub title: SharedString,
    pub left: SideTitle,
    pub right: SideTitle,
    /// The right title's check box: the file goes into the commit (`None`:
    /// not in the commit's list, no check boxes).
    pub included: Option<bool>,
    /// The changes left out of the commit while the file goes in (it is
    /// committed in part), by their lines in the base text.
    pub excluded: Vec<Range<u32>>,
    /// The only side shown (`SimpleOnesideDiffViewer`): the current text of
    /// an added or unversioned file, the base of a deleted one; `None`:
    /// both sides.
    pub one_side: Option<Side>,
    /// The file's index in its group of the Commit tool window's tree (or
    /// in the selected files) and the group's length: the files counter
    /// (`iterateChangesInSameGroup`).
    pub files: (usize, usize),
    /// Compare Previous and Next File can go on: the files before and
    /// after this one in the list the diff steps through, all groups of
    /// the Commit tool window's tree in order or the selected files
    /// (`getSelectionStrategy`).
    pub steps: (bool, bool),
    /// Shown in a preview tab (grey title, the next diff of the list
    /// takes its place).
    pub preview: bool,
}

/// Where a diff scrolls once its differences are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTo {
    FirstChange,
    LastChange,
    /// A line of the right side (the change there, or the line).
    Line(u32),
}

/// What the owner is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffEvent {
    /// Jump to Source: the file at a line and column of its current text.
    JumpToSource {
        line: u32,
        column: u32,
    },
    NextFile,
    /// The previous file; at its last difference when reached with
    /// Previous Difference.
    PreviousFile {
        last_change: bool,
    },
    /// The right title's check box was clicked.
    SetIncluded(bool),
    /// A change's check box was clicked: the change, by its lines in the
    /// base text, goes into the commit or not; `all` are the lines of
    /// every change of the file.
    SetChangeIncluded {
        change: Range<u32>,
        all: Vec<Range<u32>>,
        included: bool,
    },
    /// The differences were computed again (after an edit).
    Changed,
}

impl EventEmitter<DiffEvent> for DiffView {}

/// Pressed at the last (first) difference: pressed again it goes to the
/// next (previous) file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileJump {
    Next,
    Previous,
}

pub struct DiffView {
    focus: FocusHandle,
    left: Entity<Editor>,
    right: Entity<Editor>,
    request: Option<DiffRequest>,
    settings: DiffSettings,
    changes: Rc<Vec<DiffChange>>,
    /// The right text's version the changes are for.
    computed_for: Option<u64>,
    compute: Option<Task<()>>,
    /// The side that had the focus last (`getCurrentSide`).
    current: Side,
    /// Applied once the differences are known and the views laid out.
    pending_scroll: Option<ScrollTo>,
    /// Next or Previous Difference at the end: the hint shows and the same
    /// command goes to the next file.
    file_jump: Option<FileJump>,
    /// Both sides on their way to a change.
    animation: Option<Task<()>>,
    settings_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    /// The gear's position (where the settings popup opens).
    gear_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The changes' decorations on the left and the right side, by change
    /// (their gutter actions report these).
    actions: [Vec<DecorationId>; 2],
    _subscriptions: Vec<Subscription>,
}

impl DiffView {
    /// A viewer whose right side is `right`, the editor of the file's
    /// document (its owner loads it and keeps it with the file's tab).
    pub fn new(right: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let left = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.buffer_mut().set_read_only(true);
            e.set_viewer(true);
            e.set_mirrored(true, cx);
            e
        });
        // The reference's diff editors: no caret row, no page below the
        // text (`DiffUtil.createEditor`), the diff gutter.
        for e in [&left, &right] {
            e.update(cx, |e, cx| {
                e.set_caret_row_shown(false, cx);
                e.set_room_after_end(Some(ROOM_AFTER_END), cx);
                e.set_diff_gutter(DiffGutter::Actions, cx);
            });
        }
        right.update(cx, |e, cx| {
            e.set_soft_wraps(false, cx);
            e.set_mirrored(false, cx);
        });
        let subscriptions = vec![
            cx.subscribe(&left, |this, _, event: &EditorEvent, cx| {
                this.editor_event(Side::Left, event, cx)
            }),
            cx.subscribe(&right, |this, _, event: &EditorEvent, cx| {
                this.editor_event(Side::Right, event, cx)
            }),
            cx.on_focus(&left.focus_handle(cx), window, |this, _, cx| {
                this.current = Side::Left;
                cx.notify();
            }),
            cx.on_focus(&right.focus_handle(cx), window, |this, _, cx| {
                this.current = Side::Right;
                cx.notify();
            }),
            cx.observe(&left, |_, _, cx| cx.notify()),
            cx.observe(&right, |_, _, cx| cx.notify()),
        ];
        Self {
            focus: cx.focus_handle(),
            left,
            right,
            request: None,
            settings: DiffSettings::default(),
            changes: Rc::new(Vec::new()),
            computed_for: None,
            compute: None,
            current: Side::Right,
            pending_scroll: None,
            file_jump: None,
            animation: None,
            settings_menu: None,
            gear_bounds: Rc::new(Cell::new(None)),
            actions: [Vec::new(), Vec::new()],
            _subscriptions: subscriptions,
        }
    }

    /// Shows a file's diff: `base` on the left (none: an empty left side),
    /// highlighted like the file; the right editor holds the file's text
    /// already. The differences are computed off the UI thread; then the
    /// views scroll to `scroll`.
    pub fn set_request(
        &mut self,
        request: DiffRequest,
        base: Option<Arc<str>>,
        highlighter: Option<Box<dyn Highlighter>>,
        config: EditorConfig,
        scroll: ScrollTo,
        cx: &mut Context<Self>,
    ) {
        // One side alone is laid out as any editor (not mirrored).
        let one_side = request.one_side;
        if let Some(side) = one_side {
            self.current = side;
        }
        self.left.update(cx, |e, cx| {
            let mut buffer = Buffer::new(base.as_deref().unwrap_or(""));
            buffer.set_read_only(true);
            *e.buffer_mut() = buffer;
            e.set_path(Some(request.path.clone()));
            e.set_config(config);
            e.buffer_reloaded(cx);
            e.set_highlighter(highlighter, cx);
            e.set_selections(SelectionSet::caret(0), cx);
            e.set_scroll_position(point(px(0.0), px(0.0)), cx);
            e.set_mirrored(one_side.is_none(), cx);
        });
        let gutter = if one_side.is_some() {
            DiffGutter::Plain
        } else {
            DiffGutter::Actions
        };
        for e in [&self.left, &self.right] {
            e.update(cx, |e, cx| e.set_diff_gutter(gutter, cx));
        }
        self.right.update(cx, |e, cx| {
            e.set_soft_wraps(false, cx);
            e.set_selections(SelectionSet::caret(0), cx);
            e.set_scroll_position(point(px(0.0), px(0.0)), cx);
        });
        self.request = Some(request);
        self.file_jump = None;
        self.pending_scroll = Some(scroll);
        self.changes = Rc::new(Vec::new());
        self.computed_for = None;
        self.rediff(Duration::ZERO, cx);
        cx.notify();
    }

    /// The file's inclusion in the commit changed: the check boxes of the
    /// title and the changes follow.
    pub fn set_inclusion(
        &mut self,
        included: Option<bool>,
        excluded: Vec<Range<u32>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(r) = &mut self.request
            && (r.included != included || r.excluded != excluded)
        {
            r.included = included;
            r.excluded = excluded;
            self.decorate(cx);
        }
    }

    pub fn request(&self) -> Option<&DiffRequest> {
        self.request.as_ref()
    }

    pub fn editor(&self, side: Side) -> &Entity<Editor> {
        side.select(&self.left, &self.right)
    }

    /// The side with the focus, or that had it last.
    pub fn current_side(&self) -> Side {
        self.current
    }

    pub fn changes(&self) -> &[DiffChange] {
        &self.changes
    }

    /// The differences are known for the current text.
    pub fn is_computed(&self, cx: &App) -> bool {
        self.computed_for == Some(self.right.read(cx).buffer().version())
    }

    pub fn settings(&self) -> DiffSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: DiffSettings, cx: &mut Context<Self>) {
        if self.settings != settings {
            self.settings = settings;
            self.computed_for = None;
            self.rediff(Duration::ZERO, cx);
            cx.notify();
        }
    }

    // ---- differences ----------------------------------------------------------

    fn editor_event(&mut self, side: Side, event: &EditorEvent, cx: &mut Context<Self>) {
        match event {
            EditorEvent::Edited(_) | EditorEvent::Reloaded if side == Side::Right => {
                self.file_jump = None;
                self.rediff(REDIFF_DELAY, cx);
            }
            EditorEvent::Scrolled => self.scrolled(side, cx),
            EditorEvent::GutterActionClicked { id } => self.gutter_action(side, *id, cx),
            EditorEvent::SelectionsChanged => {
                let moved = self.file_jump.take().is_some();
                if moved {
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// Compares the texts again off the UI thread, after `delay`.
    fn rediff(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let left = self.left.read(cx).buffer().rope().clone();
        let right_buffer = self.right.read(cx).buffer();
        let right = right_buffer.rope().clone();
        let version = right_buffer.version();
        let settings = self.settings;
        let executor = cx.background_executor().clone();
        self.compute = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                executor.timer(delay).await;
            }
            let changes = cx
                .background_spawn(async move {
                    model::compare(&left.to_string(), &right.to_string(), settings)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.right.read(cx).buffer().version() != version {
                    return;
                }
                this.compute = None;
                this.apply(changes, version, cx);
            });
        }));
    }

    /// Shows computed differences in both editors.
    fn apply(&mut self, changes: Vec<DiffChange>, version: u64, cx: &mut Context<Self>) {
        self.changes = Rc::new(changes);
        self.decorate(cx);
        self.computed_for = Some(version);
        if let Some(scroll) = self.pending_scroll {
            self.apply_scroll(scroll, cx);
        }
        cx.emit(DiffEvent::Changed);
        cx.notify();
    }

    /// The changes' highlights in both editors: each side's lines and
    /// words, in the gutter as included in the commit or not; one side
    /// alone is all inserted or deleted (`SimpleOnesideDiffViewer`).
    fn decorate(&mut self, cx: &mut Context<Self>) {
        let one_side = self.one_side();
        let excluded = self.excluded_changes();
        // `>>` while the right side can be edited, the check boxes while the
        // file is in the commit's list.
        let revert = self.can_revert(cx);
        let checkboxes = self.request.as_ref().is_some_and(|r| r.included.is_some());
        for side in [Side::Left, Side::Right] {
            let action = |i: usize| match side {
                Side::Left => revert.then_some((IconName::DIFF_ARROW_RIGHT, REVERT)),
                Side::Right => checkboxes.then(|| {
                    let icon = if excluded.get(i).copied().unwrap_or(false) {
                        IconName::DIFF_CHECKBOX
                    } else {
                        IconName::DIFF_CHECKBOX_SELECTED
                    };
                    (icon, INCLUDE)
                }),
            };
            let buffer = self.editor(side).read(cx).buffer();
            let (lines, ranges) = match one_side {
                Some(shown) if shown == side => (vec![one_side_decoration(side, buffer)], None),
                Some(_) => (Vec::new(), None),
                None => decorations(&self.changes, &excluded, side, buffer, &action, cx),
            };
            let ids = self.editor(side).update(cx, |e, cx| {
                e.set_diff_highlights(ranges, cx);
                e.set_decorations(GROUP, lines, cx)
            });
            self.actions[side.select(0, 1)] = if one_side.is_none() { ids } else { Vec::new() };
        }
        cx.notify();
    }

    /// The right side can be edited: a change can be reverted.
    fn can_revert(&self, cx: &App) -> bool {
        self.one_side().is_none()
            && self.request.as_ref().is_some_and(|r| !r.right.read_only)
            && !self.right.read(cx).buffer().is_read_only()
    }

    /// A change's action in a gutter was clicked: `>>` on the left, the
    /// check box on the right.
    fn gutter_action(&mut self, side: Side, id: DecorationId, cx: &mut Context<Self>) {
        let Some(index) = self.actions[side.select(0, 1)]
            .iter()
            .position(|a| *a == id)
        else {
            return;
        };
        match side {
            Side::Left => self.revert_change(index, cx),
            Side::Right => self.set_change_included(index, None, cx),
        }
    }

    /// Replaces the right lines of change `index` with its left lines as
    /// one undoable edit (`SimpleDiffViewer.replaceChange`, "Replace
    /// change"); the caret goes to where the change was.
    pub fn revert_change(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.can_revert(cx) || !self.is_computed(cx) {
            return;
        }
        let Some(c) = self.changes.get(index).cloned() else {
            return;
        };
        let base = self.left.read(cx).buffer().rope().to_string();
        let base_lines: Vec<&str> = vcs::split_lines(&base).collect();
        let range = vcs::Range::new(c.right.start, c.right.end, c.left.start, c.left.end);
        self.right.update(cx, |e, cx| {
            let buffer = e.buffer();
            let edit = crate::markers::rollback_edit(buffer, &base_lines, &range);
            let line = (c.right.start as usize).min(buffer.len_lines() - 1);
            let at = buffer.line_start(line);
            let place = CaretPlacement::Explicit(SelectionSet::caret(at));
            e.edit(vec![edit], EditKind::Other, place, cx);
        });
    }

    /// Includes change `index` into the commit or leaves it out (`None`:
    /// the other way than now); the owner keeps the state
    /// ([`DiffEvent::SetChangeIncluded`]).
    pub fn set_change_included(
        &mut self,
        index: usize,
        included: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if self.request.as_ref().and_then(|r| r.included).is_none() {
            return;
        }
        let excluded = self.excluded_changes();
        let Some(c) = self.changes.get(index) else {
            return;
        };
        cx.emit(DiffEvent::SetChangeIncluded {
            change: c.left.clone(),
            all: self.changes.iter().map(|c| c.left.clone()).collect(),
            included: included.unwrap_or(excluded[index]),
        });
    }

    /// Window bounds of a change's action in a side's gutter in the last
    /// frame (`>>` on the left, the check box on the right).
    pub fn action_bounds(&self, side: Side, index: usize, cx: &App) -> Option<Bounds<Pixels>> {
        let id = *self.actions[side.select(0, 1)].get(index)?;
        self.editor(side).read(cx).gutter_action_bounds(id)
    }

    /// The change at the caret of the current side.
    fn change_at_caret(&self, cx: &App) -> Option<usize> {
        let side = self.current;
        model::change_at(&self.changes, side, self.caret_line(side, cx))
    }

    fn apply_left_side(&mut self, a: &ApplyLeftSide, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = a.index.or_else(|| self.change_at_caret(cx)) {
            self.revert_change(i, cx);
        }
    }

    fn include_change(&mut self, a: &IncludeChange, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = a.index.or_else(|| self.change_at_caret(cx)) {
            self.set_change_included(i, a.included, cx);
        }
    }

    /// The side shown alone, if any.
    pub fn one_side(&self) -> Option<Side> {
        self.request.as_ref().and_then(|r| r.one_side)
    }

    /// Whether each change is left out of the commit (`isExcluded`): all
    /// of a file whose check box is off, the ones left out of a file
    /// committed in part.
    fn excluded_changes(&self) -> Vec<bool> {
        let n = self.changes.len();
        match self
            .request
            .as_ref()
            .and_then(|r| Some((r.included?, &r.excluded)))
        {
            Some((false, _)) => vec![true; n],
            Some((true, excluded)) => self
                .changes
                .iter()
                .map(|c| excluded.contains(&c.left))
                .collect(),
            None => vec![false; n],
        }
    }

    /// The changes going into the commit, when the file is in the commit's
    /// list.
    fn included_count(&self) -> Option<usize> {
        self.request.as_ref()?.included?;
        Some(self.excluded_changes().iter().filter(|e| !**e).count())
    }

    fn line_counts(&self, cx: &App) -> (u32, u32) {
        (
            self.left.read(cx).buffer().len_lines() as u32,
            self.right.read(cx).buffer().len_lines() as u32,
        )
    }

    // ---- scrolling ------------------------------------------------------------

    /// One side scrolled: the other follows (`SyncScrollSupport`).
    fn scrolled(&mut self, side: Side, cx: &mut Context<Self>) {
        self.file_jump = None;
        // Scrolled by hand: the way to a change ends here.
        self.animation = None;
        let master = self.editor(side).read(cx);
        let Some((line_height, view_height)) = master.view_metrics() else {
            return;
        };
        let top = master.scroll_position();
        let counts = self.line_counts(cx);
        let slave = self.editor(side.other()).clone();
        let other = slave.read(cx).scroll_position();
        let y = model::synced_top(
            &self.changes,
            counts,
            side,
            f32::from(top.y),
            f32::from(view_height),
            f32::from(line_height),
            f32::from(other.y),
        )
        .map_or(other.y, px);
        slave.update(cx, |e, cx| e.set_scroll_position(point(top.x, y), cx));
        cx.notify();
    }

    /// Scrolls both sides to show a change, its first line a third down,
    /// and puts both carets at its start (`scrollToChange`): animated, the
    /// sides together (`SyncScrollSupport.doMakeVisible` with its anchor).
    fn scroll_to_change(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get(index).cloned() else {
            return;
        };
        self.animation = None;
        for side in [Side::Left, Side::Right] {
            let line = change.lines(side).start as usize;
            self.editor(side).update(cx, |e, cx| {
                let line = line.min(e.buffer().len_lines() - 1);
                let at = e.buffer().line_start(line);
                e.set_selections_in_place(SelectionSet::single(Selection::new(at, at)), cx);
            });
        }
        let (l, r) = (self.left.read(cx), self.right.read(cx));
        let (Some((lh, h1)), Some((_, h2))) = (l.view_metrics(), r.view_metrics()) else {
            // Not laid out yet (a diff just opened): each view scrolls in
            // its first layout, both of the same height.
            let counts = self.line_counts(cx);
            for side in [Side::Left, Side::Right] {
                let lines = [change.left.clone(), change.right.clone()];
                self.editor(side).update(cx, |e, cx| {
                    e.scroll_on_layout(
                        Box::new(move |lh, height| {
                            let (lh, h) = (f32::from(lh), f32::from(height));
                            let max_top = |rows: u32| {
                                (lh * (rows as usize + ROOM_AFTER_END) as f32 - h).max(0.0)
                            };
                            let tops = model::target_tops(
                                lines,
                                lh,
                                [h, h],
                                [max_top(counts.0), max_top(counts.1)],
                            );
                            Some(point(px(0.0), px(side.select(tops[0], tops[1]))))
                        }),
                        cx,
                    )
                });
            }
            return;
        };
        let tops = model::target_tops(
            [change.left.clone(), change.right.clone()],
            f32::from(lh),
            [f32::from(h1), f32::from(h2)],
            [f32::from(l.max_scroll_top()), f32::from(r.max_scroll_top())],
        );
        let from = [
            f32::from(l.scroll_position().y),
            f32::from(r.scroll_position().y),
        ];
        let master = self.current.select(0, 1);
        let duration = model::scroll_duration(tops[master] - from[master], f32::from(lh));
        self.animate_scroll(from, tops, duration, cx);
    }

    /// Moves both sides from `from` to `to` (tops in pixels, the text at
    /// its left edge) in `duration`, eased out (`ScrollingModelImpl`'s
    /// animated scrolling, every 4 ms).
    fn animate_scroll(
        &mut self,
        from: [f32; 2],
        to: [f32; 2],
        duration: Duration,
        cx: &mut Context<Self>,
    ) {
        if duration.is_zero() {
            self.set_tops(to, cx);
            return;
        }
        let executor = cx.background_executor().clone();
        let started = std::time::Instant::now();
        self.animation = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(4)).await;
                let t = started.elapsed().as_secs_f32() / duration.as_secs_f32();
                let k = model::ease_out(t);
                let tops = [
                    from[0] + (to[0] - from[0]) * k,
                    from[1] + (to[1] - from[1]) * k,
                ];
                let done = t >= 1.0;
                let tops = if done { to } else { tops };
                let alive = this.update(cx, |this, cx| this.set_tops(tops, cx)).is_ok();
                if done || !alive {
                    break;
                }
            }
        }));
    }

    /// Both sides' tops in pixels, the text at its left edge.
    fn set_tops(&mut self, tops: [f32; 2], cx: &mut Context<Self>) {
        for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
            self.editor(side).update(cx, |e, cx| {
                e.set_scroll_position(point(px(0.0), px(tops[i])), cx)
            });
        }
        cx.notify();
    }

    /// The initial scroll, once the differences are known (views not laid
    /// out yet scroll in their first layout).
    fn apply_scroll(&mut self, scroll: ScrollTo, cx: &mut Context<Self>) {
        if self.computed_for.is_none() {
            return;
        }
        self.pending_scroll = None;
        let index = match scroll {
            ScrollTo::FirstChange => (!self.changes.is_empty()).then_some(0),
            ScrollTo::LastChange => self.changes.len().checked_sub(1),
            ScrollTo::Line(line) => {
                model::change_at(&self.changes, Side::Right, line).or_else(|| {
                    self.right.update(cx, |e, cx| {
                        let line = (line as usize).min(e.buffer().len_lines() - 1);
                        let at = e.buffer().line_start(line);
                        e.set_selections(SelectionSet::caret(at), cx);
                    });
                    None
                })
            }
        };
        if let Some(index) = index {
            self.scroll_to_change(index, cx);
        }
    }

    // ---- commands -------------------------------------------------------------

    fn caret_line(&self, side: Side, cx: &App) -> u32 {
        self.editor(side).read(cx).caret_point().line as u32
    }

    fn has_file(&self, next: bool) -> bool {
        self.request.as_ref().is_some_and(|r| {
            let (previous, following) = r.steps;
            if next { following } else { previous }
        })
    }

    /// Next Difference; at the last one the hint, then the next file
    /// (`DiffRequestProcessor`'s `MyNextDifferenceAction`).
    pub fn next_difference(&mut self, cx: &mut Context<Self>) {
        let side = self.current;
        let count = self.editor(side).read(cx).buffer().len_lines() as u32;
        let line = self.caret_line(side, cx);
        if let Some(i) = model::next_change(&self.changes, side, line, count) {
            self.file_jump = None;
            self.scroll_to_change(i, cx);
        } else if self.has_file(true) {
            if self.file_jump == Some(FileJump::Next) {
                self.file_jump = None;
                cx.emit(DiffEvent::NextFile);
            } else {
                self.file_jump = Some(FileJump::Next);
            }
        }
        cx.notify();
    }

    pub fn previous_difference(&mut self, cx: &mut Context<Self>) {
        let side = self.current;
        let line = self.caret_line(side, cx);
        if let Some(i) = model::previous_change(&self.changes, side, line) {
            self.file_jump = None;
            self.scroll_to_change(i, cx);
        } else if self.has_file(false) {
            if self.file_jump == Some(FileJump::Previous) {
                self.file_jump = None;
                cx.emit(DiffEvent::PreviousFile { last_change: true });
            } else {
                self.file_jump = Some(FileJump::Previous);
            }
        }
        cx.notify();
    }

    fn next_difference_action(
        &mut self,
        _: &NextDifference,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_difference(cx);
    }

    fn previous_difference_action(
        &mut self,
        _: &PreviousDifference,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.previous_difference(cx);
    }

    fn next_file(&mut self, _: &NextFile, _: &mut Window, cx: &mut Context<Self>) {
        self.file_jump = None;
        if self.has_file(true) {
            cx.emit(DiffEvent::NextFile);
        }
    }

    fn previous_file(&mut self, _: &PreviousFile, _: &mut Window, cx: &mut Context<Self>) {
        self.file_jump = None;
        if self.has_file(false) {
            cx.emit(DiffEvent::PreviousFile { last_change: false });
        }
    }

    /// Select Opposite Diff Pane.
    pub fn focus_opposite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.one_side().is_some() {
            return;
        }
        let other = self.current.other();
        self.current = other;
        window.focus(&self.editor(other).focus_handle(cx));
        cx.notify();
    }

    fn focus_opposite_action(
        &mut self,
        _: &FocusOpposite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_opposite(window, cx);
    }

    /// Jump to Source: the current side's caret, a line of the left side
    /// taken to the line that stands there on the right.
    pub fn jump_to_source(&mut self, cx: &mut Context<Self>) {
        let side = self.current;
        let caret = self.editor(side).read(cx).caret_point();
        let (line, column) = match side {
            Side::Right => (caret.line as u32, caret.column as u32),
            Side::Left => {
                let counts = self.line_counts(cx);
                let line =
                    model::transfer_line(&self.changes, counts, Side::Left, caret.line as u32);
                (line, 0)
            }
        };
        cx.emit(DiffEvent::JumpToSource { line, column });
    }

    fn jump_to_source_action(&mut self, _: &JumpToSource, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_to_source(cx);
    }

    fn set_ignore_policy(&mut self, a: &SetIgnorePolicy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(policy) = IgnorePolicy::ALL.into_iter().find(|p| p.id() == a.policy) {
            let settings = DiffSettings {
                ignore: policy,
                ..self.settings
            };
            self.set_settings(settings, cx);
        }
    }

    fn set_highlight_policy(
        &mut self,
        a: &SetHighlightPolicy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(policy) = HighlightPolicy::ALL
            .into_iter()
            .find(|p| p.id() == a.policy)
        {
            let settings = DiffSettings {
                highlight: policy,
                ..self.settings
            };
            self.set_settings(settings, cx);
        }
    }

    /// The settings popup under the gear (`ShowDiffSettingsPopup`).
    fn show_settings(&mut self, _: &ShowSettings, window: &mut Window, cx: &mut Context<Self>) {
        let check = |on: bool, item: MenuItem| {
            if on {
                item.icon(IconName::CHECKED)
            } else {
                item
            }
        };
        let mut items = vec![MenuItem::header("Ignore Differences")];
        for p in IgnorePolicy::ALL {
            items.push(check(
                self.settings.ignore == p,
                MenuItem::action(
                    p.text(),
                    SetIgnorePolicy {
                        policy: p.id().into(),
                    },
                ),
            ));
        }
        items.push(MenuItem::header("Highlighting Differences"));
        for p in HighlightPolicy::ALL {
            items.push(check(
                self.settings.highlight == p,
                MenuItem::action(
                    p.text(),
                    SetHighlightPolicy {
                        policy: p.id().into(),
                    },
                ),
            ));
        }
        let at = self.gear_bounds.get().map_or(point(px(0.0), px(0.0)), |b| {
            point(b.left(), b.bottom() + px(2.0))
        });
        let menu = ContextMenu::build(items, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &gpui::DismissEvent, cx| {
            this.settings_menu = None;
            cx.notify();
        });
        self.settings_menu = Some((menu, at, subscription));
        cx.notify();
    }

    /// The settings popup while it is open.
    pub fn settings_menu(&self) -> Option<&Entity<ContextMenu>> {
        self.settings_menu.as_ref().map(|(m, _, _)| m)
    }

    // ---- state ----------------------------------------------------------------

    /// The viewer as JSON (`workbench.state`).
    pub fn state_json(&self, cx: &App) -> Json {
        let side = |side: Side| {
            let e = self.editor(side).read(cx);
            let caret = e.caret_point();
            json!({
                "caret": [caret.line, caret.column],
                "scroll_top": f32::from(e.scroll_position().y),
                "lines": e.buffer().len_lines(),
                "read_only": e.buffer().is_read_only() || e.is_viewer(),
                "focused_last": self.current == side,
                "words": e.diff_highlights().map_or(0, |d| d.words.len()),
            })
        };
        let title = |t: &SideTitle| {
            json!({
                "label": t.label.to_string(),
                "path": t.path.as_ref().map(|p| p.to_string()),
                "read_only": t.read_only,
            })
        };
        json!({
            "path": self.request.as_ref().map(|r| r.path.display().to_string()),
            "title": self.request.as_ref().map(|r| r.title.to_string()),
            "left_title": self.request.as_ref().map(|r| title(&r.left)),
            "right_title": self.request.as_ref().map(|r| title(&r.right)),
            "included": self.request.as_ref().and_then(|r| r.included),
            "files": self.request.as_ref().map(|r| json!([r.files.0, r.files.1])),
            "steps": self.request.as_ref().map(|r| json!([r.steps.0, r.steps.1])),
            "preview": self.request.as_ref().is_some_and(|r| r.preview),
            "files_text": self.files_text(),
            "status": self.status_text(),
            "one_side": self.one_side().map(|s| s.name()),
            "computed": self.is_computed(cx),
            "changes": self.changes.iter().zip(self.excluded_changes()).map(|(c, excluded)| json!({
                "left": [c.left.start, c.left.end],
                "right": [c.right.start, c.right.end],
                "kind": c.kind().name(),
                "words": c.words.as_ref().map(|w| w.len()),
                "excluded": excluded,
            })).collect::<Vec<_>>(),
            "revert": self.can_revert(cx),
            "current_side": self.current.name(),
            "left": side(Side::Left),
            "right": side(Side::Right),
            "hint": self.hint(),
            "settings": {
                "ignore": self.settings.ignore.id(),
                "highlight": self.settings.highlight.id(),
                "open": self.settings_menu.is_some(),
            },
        })
    }

    /// The files counter: for a list of several files, or of the Commit
    /// tool window's even with one.
    fn files_text(&self) -> Option<String> {
        let r = self.request.as_ref()?;
        let (index, count) = r.files;
        (count > 1 || r.preview).then(|| model::files_text(index, count))
    }

    /// "Press again to go to the next file" (`DiffBundle`).
    pub fn hint(&self) -> Option<&'static str> {
        self.file_jump.map(|j| match j {
            FileJump::Next => "Press again to go to the next file",
            FileJump::Previous => "Press again to go to the previous file",
        })
    }

    // ---- drawing --------------------------------------------------------------

    fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (muted, link, separator, background, island_border) = {
            let theme = cx.theme();
            (
                theme.color(UiRole::TextMuted),
                theme.color(UiRole::TextLink),
                theme.color(UiRole::Divider),
                // The island's colours (`Editor.SearchField.*`).
                theme.color(UiRole::SearchField),
                theme.color(UiRole::SearchFieldBorder),
            )
        };
        let shortcut = |action: &dyn gpui::Action| {
            ui::keys::shortcut_in(action, &self.focus, window).map(SharedString::from)
        };
        let side = self.current;
        let line = self.caret_line(side, cx);
        let count = self.editor(side).read(cx).buffer().len_lines() as u32;
        let can_next =
            model::next_change(&self.changes, side, line, count).is_some() || self.has_file(true);
        let can_previous =
            model::previous_change(&self.changes, side, line).is_some() || self.has_file(false);
        let button = |id: &'static str,
                      icon: IconName,
                      enabled: bool,
                      tooltip: &'static str,
                      keys: Option<SharedString>,
                      cx: &mut Context<Self>,
                      on: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
                .debug_selector(move || format!("diff-{id}"))
                .when(!enabled, |b| b.opacity(ui::components::DISABLED_ALPHA))
                .tooltip(Tooltip::text(tooltip, keys))
                .when(enabled, |b| {
                    b.on_click(
                        cx.listener(move |this, _: &ClickEvent, window, cx| on(this, window, cx)),
                    )
                })
        };
        let vertical_separator = || {
            div()
                .flex_none()
                .mx(px(SEPARATOR_ROOM))
                .w(px(1.0))
                .h(px(SEPARATOR_HEIGHT))
                .bg(separator)
        };
        let mut left = div().flex().flex_row().items_center();
        left = left
            .child(button(
                "previous",
                IconName::ARROW_UP,
                can_previous,
                "Previous Difference",
                shortcut(&PreviousDifference),
                cx,
                |this, _, cx| this.previous_difference(cx),
            ))
            .child(button(
                "next",
                IconName::ARROW_DOWN,
                can_next,
                "Next Difference",
                shortcut(&NextDifference),
                cx,
                |this, _, cx| this.next_difference(cx),
            ))
            .child(vertical_separator())
            .child(button(
                "jump-to-source",
                IconName::EDIT,
                true,
                "Jump to Source",
                shortcut(&JumpToSource),
                cx,
                |this, _, cx| this.jump_to_source(cx),
            ));
        if let Some(files) = self.files_text() {
            // One file: no list to show, the counter is grey.
            let several = self.request.as_ref().is_some_and(|r| r.files.1 > 1);
            left = left
                .child(vertical_separator())
                .child(button(
                    "previous-file",
                    IconName::BACK,
                    self.has_file(false),
                    "Compare Previous File",
                    shortcut(&PreviousFile),
                    cx,
                    |this, window, cx| this.previous_file(&PreviousFile, window, cx),
                ))
                .child(
                    div()
                        .debug_selector(|| "diff-files".into())
                        .px(px(FILES_GAP))
                        .whitespace_nowrap()
                        .text_color(if several { link } else { muted })
                        .child(files),
                )
                .child(button(
                    "next-file",
                    IconName::FORWARD,
                    self.has_file(true),
                    "Compare Next File",
                    shortcut(&NextFile),
                    cx,
                    |this, window, cx| this.next_file(&NextFile, window, cx),
                ));
        }
        // One side alone has no status (`SimpleOnesideDiffViewer`).
        let status = self.status_text();
        // The viewer's modes (`DiffToolChooser`): side by side is the one
        // there is.
        let modes = ui::form::segmented_icons(
            "diff-mode",
            &[
                (IconName::DIFF_SIDE_BY_SIDE, "Side-by-side viewer"),
                (IconName::DIFF_UNIFIED, "Unified viewer"),
            ],
            Some(0),
            (px(MODE_SIZE.0), px(MODE_SIZE.1)),
            |_, _, _| {},
            cx,
        );
        let gear = icon_button(
            "settings",
            IconName::SETTINGS_SMALL,
            px(BUTTON),
            px(ICON),
            false,
            cx,
        )
        .debug_selector(|| "diff-settings".into())
        .relative()
        // A popup group: the corner arrow (`ActionButtonLook.paintDownArrow`:
        // the arrow icon 1 px right of and below the icon).
        .child(
            div()
                .absolute()
                .left(px((BUTTON - ICON) / 2.0 + 1.0))
                .top(px((BUTTON - ICON) / 2.0 + 1.0))
                .child(Icon::new(IconName::DROPDOWN_CORNER).size(px(ICON))),
        )
        .tooltip(Tooltip::text("Show Diff Settings", None))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.show_settings(&ShowSettings, window, cx)
        }));
        let gear_bounds = self.gear_bounds_tracker();
        let right = div()
            .flex()
            .flex_row()
            .items_center()
            .children(status.map(|status| {
                div()
                    .debug_selector(|| "diff-status".into())
                    .mr(px(STATUS_GAP))
                    .whitespace_nowrap()
                    .child(status)
            }))
            .child(modes)
            .child(div().w(px(MODE_GAP)))
            .child(div().relative().child(gear).child(gear_bounds));
        // The island (`DiffToolbarIslandPanelUI`).
        div().flex_none().px(px(ISLAND_SIDE)).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(ISLAND_HEIGHT))
                .pl(px(TOOLBAR_LEFT))
                .pr(px(TOOLBAR_RIGHT))
                .rounded(px(ISLAND_RADIUS))
                .border_1()
                .border_color(island_border)
                .bg(background)
                .child(left)
                .child(right),
        )
    }

    /// The status text: the differences, how many go into the commit;
    /// none for one side alone.
    fn status_text(&self) -> Option<String> {
        if self.one_side().is_some() {
            return None;
        }
        Some(model::status_text_included(
            self.changes.len(),
            self.included_count(),
        ))
    }

    /// Remembers where the gear is drawn (the settings popup opens there).
    fn gear_bounds_tracker(&self) -> impl IntoElement {
        let cell = self.gear_bounds.clone();
        canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| ())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }

    fn render_title(&self, side: Side, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(request) = &self.request else {
            return div().flex_1().into_any_element();
        };
        let title = side.select(&request.left, &request.right);
        let muted = theme.color(UiRole::TextMuted);
        let mut row = div()
            .debug_selector(move || format!("diff-title-{}", side.name()))
            .relative()
            .flex()
            .flex_row()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .items_center()
            .pl(px(TITLE_LEFT))
            .pr(px(TITLE_LEFT))
            .overflow_hidden()
            .whitespace_nowrap();
        if title.read_only {
            row = row.child(
                div()
                    .flex_none()
                    .w(px(LOCK_ROOM))
                    .child(Icon::new(IconName::LOCKED).size(px(ICON))),
            );
        }
        // The check box of the file's changes over the gutter's check
        // boxes, the label after it (`ExcludeAllCheckboxPanel`): ticked,
        // partly ticked or not as the changes are; not for one side alone.
        if side == Side::Right
            && request.one_side.is_none()
            && let Some(included) = request.included
        {
            let x = self
                .right
                .read(cx)
                .diff_action_offset()
                .map_or(CHECKBOX_FALLBACK, f32::from);
            let partly = included && self.excluded_changes().contains(&true);
            let icon = if partly {
                IconName::DIFF_CHECKBOX_INDETERMINATE
            } else if included {
                IconName::DIFF_CHECKBOX_SELECTED
            } else {
                IconName::DIFF_CHECKBOX
            };
            let included = included && !partly;
            let size = editor::side_by_side::ACTION_SIZE;
            row = row
                .child(
                    div()
                        .id("diff-included")
                        .debug_selector(|| "diff-included".into())
                        .absolute()
                        .left(px(x))
                        .top(px((TITLE_HEIGHT - size) / 2.0))
                        .size(px(size))
                        .child(Icon::new(icon).size(px(size)))
                        .tooltip(Tooltip::text("Include into commit", None))
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(DiffEvent::SetIncluded(!included))
                        })),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(x + LABEL_AFTER_CHECKBOX - TITLE_LEFT)),
                );
        }
        row = row.child(div().flex_none().child(title.label.clone()));
        if let Some(path) = &title.path {
            row = row.child(
                div()
                    .id(("diff-title-path", side as usize))
                    .ml(px(PATH_GAP))
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_color(muted)
                    .child(path.clone())
                    .when_some(title.tooltip.clone(), |d, t| {
                        d.tooltip(Tooltip::text(t, None))
                    }),
            );
        }
        row.into_any_element()
    }

    /// The hint under the current side's caret (`DiffUtil.showSuccessPopup`
    /// at the editor's best popup location).
    fn render_hint(&self, cx: &App) -> Option<AnyElement> {
        let text = self.hint()?;
        let theme = cx.theme();
        let caret = self.editor(self.current).read(cx).caret_bounds()?;
        Some(
            deferred(
                anchored()
                    .position(point(caret.left(), caret.bottom() + px(2.0)))
                    .child(
                        div()
                            .debug_selector(|| "diff-hint".into())
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme.color(UiRole::PopupBorder))
                            .bg(theme.color(UiRole::SurfaceRaised))
                            .shadow(ui::components::popup_shadow())
                            .text_color(theme.color(UiRole::Text))
                            .whitespace_nowrap()
                            .child(text),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// The error stripe's colour of a change.
fn stripe_role(kind: ChangeKind) -> UiRole {
    match kind {
        ChangeKind::Inserted => UiRole::StripeAdded,
        ChangeKind::Deleted => UiRole::StripeDeleted,
        ChangeKind::Modified => UiRole::StripeModified,
    }
}

/// One side shown alone: all of its lines inserted (the current text) or
/// deleted (the base), the gutter as the text (`DiffDrawUtil.createHighlighter`
/// over the whole document).
fn one_side_decoration(side: Side, buffer: &Buffer) -> (Range<usize>, Decoration) {
    let kind = side.select(ChangeKind::Deleted, ChangeKind::Inserted);
    let d = Decoration::default()
        .background(change_role(kind), editor::decorations::layer::DIFF)
        .gutter_fill(GutterFill {
            faded: false,
            border: false,
        })
        .stripe(stripe_role(kind), false);
    (0..buffer.len_lines(), d)
}

/// The decorations of one side: changed lines (faded where their words
/// are highlighted, a line on the boundary for an empty side) with their
/// error stripe marks, the gutter in the full colour, or faded between
/// two lines of it for a change left out of the commit
/// (`LineHighlighterBuilder.withExcludedInGutter`); and the changed words
/// (an empty one is a bar).
fn decorations(
    changes: &[DiffChange],
    excluded: &[bool],
    side: Side,
    buffer: &Buffer,
    action: &dyn Fn(usize) -> Option<(IconName, &'static str)>,
    cx: &App,
) -> (Vec<(Range<usize>, Decoration)>, Option<DiffHighlights>) {
    let theme = cx.theme();
    let mut lines = Vec::with_capacity(changes.len());
    let mut words = Vec::new();
    let line_count = buffer.len_lines();
    for (i, c) in changes.iter().enumerate() {
        let kind = c.kind();
        let role = change_role(kind);
        let r = c.lines(side);
        let range = r.start as usize..r.end as usize;
        let excluded = excluded.get(i).copied().unwrap_or(false);
        let mut d = Decoration::default()
            .background(role, editor::decorations::layer::DIFF)
            .gutter_fill(GutterFill {
                faded: excluded,
                border: excluded,
            })
            .stripe(stripe_role(kind), false);
        if c.words.is_some() {
            d = d.faded();
        }
        if let Some((icon, tooltip)) = action(i) {
            d = d.action(icon, Some(tooltip.into()));
        }
        lines.push((range, d));
        let Some(fragments) = &c.words else {
            continue;
        };
        let first = (r.start as usize).min(line_count - 1);
        let start = buffer.line_start(first);
        for w in fragments {
            let wr = side.select(&w.left, &w.right);
            words.push((
                start + wr.start..start + wr.end,
                theme.color(change_role(w.kind())),
            ));
        }
    }
    let words = (!words.is_empty()).then(|| DiffHighlights {
        words,
        ..DiffHighlights::default()
    });
    (lines, words)
}

impl Focusable for DiffView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor(self.current).focus_handle(cx)
    }
}

macro_rules! diff_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl DiffView {
            fn bind_actions<E: InteractiveElement>(el: E, cx: &mut Context<Self>) -> E {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }

        /// Forwards the diff commands to `view` (the workbench, for the
        /// active diff tab when the focus is elsewhere).
        pub fn forward_actions<E: InteractiveElement>(el: E, view: Entity<DiffView>) -> E {
            el $(.on_action({
                let view = view.clone();
                move |a: &$action, window, cx| view.update(cx, |v, cx| v.$method(a, window, cx))
            }))*
        }
    };
}

diff_actions! {
    NextDifference => next_difference_action,
    PreviousDifference => previous_difference_action,
    NextFile => next_file,
    PreviousFile => previous_file,
    FocusOpposite => focus_opposite_action,
    JumpToSource => jump_to_source_action,
    SetIgnorePolicy => set_ignore_policy,
    SetHighlightPolicy => set_highlight_policy,
    ShowSettings => show_settings,
    ApplyLeftSide => apply_left_side,
    IncludeChange => include_change,
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, tearline, family, size) = {
            let theme = cx.theme();
            (
                theme.color(UiRole::EditorBackground),
                theme.color(UiRole::Tearline),
                theme.ui_family(),
                theme.ui_size(),
            )
        };
        let toolbar = self.render_toolbar(window, cx).into_any_element();
        let one_side = self.one_side();
        let mut titles = div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(TITLE_HEIGHT + 1.0))
            .border_b_1()
            .border_color(tearline);
        let side_view = |side: Side, view: Entity<Editor>| {
            div()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .debug_selector(move || format!("diff-{}", side.name()))
                .child(view)
        };
        let mut content = div().flex().flex_row().flex_1().min_h(px(0.0));
        match one_side {
            Some(side) => {
                titles = titles.child(self.render_title(side, cx));
                content = content.child(side_view(side, self.editor(side).clone()));
            }
            None => {
                titles = titles
                    .child(self.render_title(Side::Left, cx))
                    .child(div().flex_none().w(px(divider::WIDTH)))
                    .child(self.render_title(Side::Right, cx));
                content = content
                    .child(side_view(Side::Left, self.left.clone()))
                    .child(divider::render(
                        &self.left,
                        &self.right,
                        self.changes.clone(),
                        Rc::new(self.excluded_changes()),
                    ))
                    .child(side_view(Side::Right, self.right.clone()));
            }
        }
        let menu = self
            .settings_menu
            .as_ref()
            .map(|(m, at, _)| ui::menu::overlay(m, *at, window));
        let root = div()
            .id("diff")
            .debug_selector(|| "diff".into())
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .font_family(family)
            .text_size(size)
            .child(toolbar)
            .child(titles)
            .child(content)
            .children(self.render_hint(cx))
            .children(menu);
        Self::bind_actions(root, cx)
    }
}
