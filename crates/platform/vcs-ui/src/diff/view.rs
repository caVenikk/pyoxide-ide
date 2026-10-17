//! The diff viewer's entity: the toolbar (Previous and Next Difference,
//! Jump to Source, the previous and next file with the files counter; the
//! count of differences, the viewer's mode and the settings), the titles of
//! both sides, the two editors and the divider (`SimpleDiffViewer`,
//! `TwosideTextDiffViewer`, `DiffRequestProcessor`'s toolbar).

use std::cell::Cell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use editor::{Decoration, DiffHighlights, Editor, EditorConfig, EditorEvent, Highlighter};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Task, Window, anchored,
    canvas, deferred, div, point, px,
};
use serde_json::{Value as Json, json};
use text::{Buffer, Selection, SelectionSet};
use themes::UiRole;
use ui::components::{Tooltip, checkbox, icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem};
use ui::theme::ThemeExt as _;

use super::divider::{self, change_role};
use super::model::{
    self, ChangeKind, DiffChange, DiffSettings, HighlightPolicy, IgnorePolicy, Side,
};
use super::{
    CONTEXT, FocusOpposite, JumpToSource, NextDifference, NextFile, PreviousDifference,
    PreviousFile, SetHighlightPolicy, SetIgnorePolicy, ShowSettings,
};

/// The decorations' group in both editors.
const GROUP: &str = "diff";
/// Rows of room below the last one (`EditorSettings`' additional lines).
const ROOM_AFTER_END: usize = 5;
/// Edits of the current text are compared again after this long.
const REDIFF_DELAY: Duration = Duration::from_millis(50);
/// The toolbar (`DiffRequestProcessor`'s, measured on the reference): its
/// height and the room at its ends; the New UI's buttons of 26 px with
/// icons of 16; the separators, 1 px with room at both sides; the gap
/// between the count of differences and the viewer's modes, whose
/// segments are 50 x 24, and the gap after them.
const TOOLBAR_HEIGHT: f32 = 34.0;
const TOOLBAR_LEFT: f32 = 4.5;
const TOOLBAR_RIGHT: f32 = 5.5;
const BUTTON: f32 = 26.0;
const ICON: f32 = 16.0;
const SEPARATOR_ROOM: f32 = 3.5;
const SEPARATOR_HEIGHT: f32 = 17.0;
const FILES_GAP: f32 = 1.5;
const STATUS_GAP: f32 = 15.0;
const MODE_SIZE: (f32, f32) = (50.0, 24.0);
const MODE_GAP: f32 = 8.0;
/// The titles (`DiffContentPanel`): their height above a border of 1 px;
/// the lock's room at the left; the gap between a revision and its path
/// (`ipadx` 8 and the label's own); the right title's check box where the
/// reference has the gutter's check boxes, and the gap after it.
const TITLE_HEIGHT: f32 = 19.0;
const TITLE_LEFT: f32 = 2.5;
const LOCK_ROOM: f32 = 16.5;
const PATH_GAP: f32 = 9.0;
const CHECKBOX_LEFT: f32 = 30.0;
const CHECKBOX_GAP: f32 = 12.5;

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
    /// no check box).
    pub included: Option<bool>,
    /// The file's index in the list and the list's length.
    pub files: (usize, usize),
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
    settings_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    /// The gear's position (where the settings popup opens).
    gear_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
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
        // text (`DiffUtil.createEditor`).
        for e in [&left, &right] {
            e.update(cx, |e, cx| {
                e.set_caret_row_shown(false, cx);
                e.set_room_after_end(Some(ROOM_AFTER_END), cx);
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
            settings_menu: None,
            gear_bounds: Rc::new(Cell::new(None)),
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
        });
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

    /// Updates the titles and the list position (the base stays).
    pub fn set_included(&mut self, included: Option<bool>, cx: &mut Context<Self>) {
        if let Some(r) = &mut self.request
            && r.included != included
        {
            r.included = included;
            cx.notify();
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
        for side in [Side::Left, Side::Right] {
            let (lines, ranges) =
                decorations(&changes, side, self.editor(side).read(cx).buffer(), cx);
            self.editor(side).update(cx, |e, cx| {
                e.set_decorations(GROUP, lines, cx);
                e.set_diff_highlights(ranges, cx);
            });
        }
        self.changes = Rc::new(changes);
        self.computed_for = Some(version);
        if let Some(scroll) = self.pending_scroll {
            self.apply_scroll(scroll, cx);
        }
        cx.emit(DiffEvent::Changed);
        cx.notify();
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
    /// and puts both carets at its start (`scrollToChange`).
    fn scroll_to_change(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get(index).cloned() else {
            return;
        };
        for side in [Side::Left, Side::Right] {
            let line = change.lines(side).start as usize;
            self.editor(side).update(cx, |e, cx| {
                let line = line.min(e.buffer().len_lines() - 1);
                let at = e.buffer().line_start(line);
                e.set_selections(SelectionSet::single(Selection::new(at, at)), cx);
            });
        }
        let (l, r) = (self.left.read(cx), self.right.read(cx));
        let (Some((lh, h1)), Some((_, h2))) = (l.view_metrics(), r.view_metrics()) else {
            return;
        };
        let tops = model::target_tops(
            [change.left.clone(), change.right.clone()],
            f32::from(lh),
            [f32::from(h1), f32::from(h2)],
            [f32::from(l.max_scroll_top()), f32::from(r.max_scroll_top())],
        );
        let x = [l.scroll_position().x, r.scroll_position().x];
        for (i, side) in [Side::Left, Side::Right].into_iter().enumerate() {
            self.editor(side).update(cx, |e, cx| {
                e.set_scroll_position(point(x[i], px(tops[i])), cx)
            });
        }
    }

    /// The initial scroll, once the differences are known and the views
    /// have been laid out.
    fn apply_scroll(&mut self, scroll: ScrollTo, cx: &mut Context<Self>) {
        if self.computed_for.is_none() || self.right.read(cx).view_metrics().is_none() {
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
            let (index, count) = r.files;
            if next { index + 1 < count } else { index > 0 }
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
            "preview": self.request.as_ref().is_some_and(|r| r.preview),
            "files_text": self.files_text(),
            "status": model::status_text(self.changes.len()),
            "computed": self.is_computed(cx),
            "changes": self.changes.iter().map(|c| json!({
                "left": [c.left.start, c.left.end],
                "right": [c.right.start, c.right.end],
                "kind": c.kind().name(),
                "words": c.words.as_ref().map(|w| w.len()),
            })).collect::<Vec<_>>(),
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

    fn files_text(&self) -> Option<String> {
        let (index, count) = self.request.as_ref()?.files;
        (count > 1).then(|| model::files_text(index, count))
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
        let (muted, link, separator, pressed, mode_border, radius, background) = {
            let theme = cx.theme();
            (
                theme.color(UiRole::TextMuted),
                theme.color(UiRole::TextLink),
                theme.color(UiRole::Divider),
                theme.color(UiRole::Pressed),
                theme.color(UiRole::BorderInput),
                theme.metric(themes::Metric::CornerRadius),
                // A panel above the editors (`Panel.background`).
                theme.color(UiRole::Surface),
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
                        .text_color(link)
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
        let status = model::status_text(self.changes.len());
        // The viewer's modes (`DiffToolChooser`): side by side is the one
        // there is.
        let mode = |id: &'static str, icon: IconName, selected: bool, tooltip: &'static str| {
            div()
                .id(id)
                .debug_selector(move || format!("diff-{id}"))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(MODE_SIZE.0))
                .h(px(MODE_SIZE.1))
                .rounded(radius)
                .when(selected, |d| {
                    d.bg(pressed).border_1().border_color(mode_border)
                })
                .when(!selected, |d| d.opacity(ui::components::DISABLED_ALPHA))
                .tooltip(Tooltip::text(tooltip, None))
                .child(Icon::new(icon).size(px(ICON)))
        };
        let gear = icon_button(
            "settings",
            IconName::SETTINGS_SMALL,
            px(BUTTON),
            px(ICON),
            false,
            cx,
        )
        .debug_selector(|| "diff-settings".into())
        .tooltip(Tooltip::text("Show Diff Settings", None))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.show_settings(&ShowSettings, window, cx)
        }));
        let gear_bounds = self.gear_bounds_tracker();
        let right = div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .debug_selector(|| "diff-status".into())
                    .mr(px(STATUS_GAP))
                    .whitespace_nowrap()
                    .text_color(muted)
                    .child(status),
            )
            .child(mode(
                "side-by-side",
                IconName::DIFF_SIDE_BY_SIDE,
                true,
                "Side-by-side viewer",
            ))
            .child(mode(
                "unified",
                IconName::DIFF_UNIFIED,
                false,
                "Unified viewer",
            ))
            .child(div().w(px(MODE_GAP)))
            .child(div().relative().child(gear).child(gear_bounds));
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(TOOLBAR_HEIGHT))
            .pl(px(TOOLBAR_LEFT))
            .pr(px(TOOLBAR_RIGHT))
            .bg(background)
            .child(left)
            .child(right)
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
            .flex()
            .flex_row()
            .flex_1()
            .min_w(px(0.0))
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
        if side == Side::Right
            && let Some(included) = request.included
        {
            row = row.child(
                div()
                    .flex_none()
                    .pl(px(CHECKBOX_LEFT - TITLE_LEFT))
                    .pr(px(CHECKBOX_GAP))
                    .child(
                        checkbox("diff-included", included, cx)
                            .debug_selector(|| "diff-included".into())
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(DiffEvent::SetIncluded(!included))
                            })),
                    ),
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

/// The decorations of one side: changed lines (faded where their words
/// are highlighted, a line on the boundary for an empty side) with their
/// error stripe marks, and the changed words (an empty one is a bar).
fn decorations(
    changes: &[DiffChange],
    side: Side,
    buffer: &Buffer,
    cx: &App,
) -> (Vec<(Range<usize>, Decoration)>, Option<DiffHighlights>) {
    let theme = cx.theme();
    let mut lines = Vec::with_capacity(changes.len());
    let mut words = Vec::new();
    let line_count = buffer.len_lines();
    for c in changes {
        let kind = c.kind();
        let role = change_role(kind);
        let stripe = match kind {
            ChangeKind::Inserted => UiRole::StripeAdded,
            ChangeKind::Deleted => UiRole::StripeDeleted,
            ChangeKind::Modified => UiRole::StripeModified,
        };
        let r = c.lines(side);
        let range = r.start as usize..r.end as usize;
        let mut d = Decoration::default()
            .background(role, editor::decorations::layer::DIFF)
            .over_gutter()
            .stripe(stripe, false);
        if c.words.is_some() {
            d = d.faded();
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
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(scroll) = self.pending_scroll {
            self.apply_scroll(scroll, cx);
            if self.pending_scroll.is_some() && self.computed_for.is_some() {
                // Laid out in this frame: scrolled in the next.
                cx.on_next_frame(window, |_, _, cx| cx.notify());
            }
        }
        let (background, border, family, size) = {
            let theme = cx.theme();
            (
                theme.color(UiRole::EditorBackground),
                theme.color(UiRole::Border),
                theme.ui_family(),
                theme.ui_size(),
            )
        };
        let toolbar = self.render_toolbar(window, cx).into_any_element();
        let titles = div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(TITLE_HEIGHT + 1.0))
            .border_b_1()
            .border_color(border)
            .child(self.render_title(Side::Left, cx))
            .child(div().flex_none().w(px(divider::WIDTH)))
            .child(self.render_title(Side::Right, cx));
        let content = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .debug_selector(|| "diff-left".into())
                    .child(self.left.clone()),
            )
            .child(divider::render(
                &self.left,
                &self.right,
                self.changes.clone(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .debug_selector(|| "diff-right".into())
                    .child(self.right.clone()),
            );
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
