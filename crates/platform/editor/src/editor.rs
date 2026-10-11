//! The editor entity: a buffer, its selections and the view state, plus
//! every editing and caret command.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, AppContext as _, ClipboardEntry, ClipboardItem, Context, Div, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels, Render, Styled,
    Subscription, Task, Window, div, point, px,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use text::movement::{self, WordStop};
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, Point, Selection, SelectionSet};

use crate::commands::*;
use crate::element::{EditorElement, LayoutInfo};
use crate::highlight::Highlighter;

/// How long the caret stays visible or hidden while blinking.
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Indentation and language hints for one editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    /// Display width of a tab character.
    pub tab_size: usize,
    /// Spaces per indentation level.
    pub indent: usize,
    /// A line ending (before trailing spaces) with one of these characters
    /// gets one more level of indentation on the next line (`:` for Python).
    pub indent_after: Vec<char>,
    /// A line starting with one of these words ends its block: the next
    /// line gets one level less (`return`, `pass`, ... for Python).
    pub dedent_after: Vec<String>,
    /// Prefix of a line comment with the space after it (`"# "`).
    pub line_comment: Option<String>,
    /// Bracket pairs that are closed when the opening one is typed.
    pub brackets: Vec<(char, char)>,
    /// Quote characters that are closed when typed.
    pub quotes: Vec<char>,
    /// A third quote typed after an empty pair opens a triple-quoted string.
    pub triple_quotes: bool,
    /// Letters that may precede an opening quote as a string prefix (`f`,
    /// `r`, `b` ...); after other word characters a quote is not paired.
    pub string_prefixes: Vec<char>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent: 4,
            indent_after: Vec::new(),
            dedent_after: Vec::new(),
            line_comment: None,
            brackets: vec![('(', ')'), ('[', ']'), ('{', '}')],
            quotes: Vec::new(),
            triple_quotes: false,
            string_prefixes: Vec::new(),
        }
    }
}

/// Moves a position (chars, before `edits`) to where it is after them.
/// Insertions at the position do not move it; a deletion around it pulls
/// it to the deletion's start.
fn map_through(edits: &[Edit], pos: usize) -> usize {
    let mut delta: isize = 0;
    for e in edits {
        let (start, end) = (e.range.start, e.range.end);
        if start >= pos {
            continue;
        }
        let removed = pos.min(end) - start;
        delta -= removed as isize;
        delta += e.text.chars().count() as isize;
    }
    (pos as isize + delta).max(0) as usize
}

/// Width of the leading whitespace of `text` in chars.
fn leading_ws(text: &str) -> usize {
    text.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

/// What happened in an editor, for its observers (tabs, status bar,
/// highlighting).
#[derive(Debug, Clone)]
pub enum EditorEvent {
    /// The text changed; replacements as applied.
    Edited(Vec<AppliedEdit>),
    /// Carets or selections moved.
    SelectionsChanged,
}

/// Clipboard metadata: the text is whole lines copied without a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct LineCopy {
    whole_lines: bool,
}

/// What a mouse drag selects.
#[derive(Debug, Clone)]
pub(crate) enum DragUnit {
    Char,
    Word(Range<usize>),
    Line(Range<usize>),
    /// Dragging the vertical scrollbar thumb; offset of the grab point.
    Scrollbar(Pixels),
}

/// A code editor for one buffer.
pub struct Editor {
    pub(crate) buffer: Buffer,
    pub(crate) selections: SelectionSet,
    focus: FocusHandle,
    config: EditorConfig,
    path: Option<PathBuf>,
    /// IME composition range (chars).
    pub(crate) marked: Option<Range<usize>>,
    /// Scroll offset of the text in pixels (both >= 0).
    pub(crate) scroll: gpui::Point<Pixels>,
    /// Bring the primary caret into view on the next frame.
    pub(crate) autoscroll: bool,
    /// Geometry of the last frame, for hit testing and the input method.
    pub(crate) layout: Option<LayoutInfo>,
    pub(crate) caret_visible: bool,
    pub(crate) drag: Option<DragUnit>,
    pub(crate) highlighter: Option<Box<dyn Highlighter>>,
    blink_epoch: usize,
    blink_task: Option<Task<()>>,
    /// The highlighter's background job in flight.
    highlight_task: Option<Task<()>>,
    word_stop: WordStop,
    /// Selections before each Extend Selection, for Shrink Selection.
    extend_stack: Vec<SelectionSet>,
    /// The selections the last Extend or Shrink Selection produced; any
    /// other change of the selections forgets the stack.
    extend_last: Option<SelectionSet>,
    /// Center the caret line on the next frame (Go to Line).
    pub(crate) center_caret: bool,
    /// A one-line text field (no gutter, no scrollbar).
    pub(crate) single_line: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Editor {
    pub fn new(buffer: Buffer, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus(&focus, window, |this, window, cx| {
                this.restart_blink(window, cx);
            }),
            cx.on_blur(&focus, window, |this, _, cx| {
                this.blink_task = None;
                this.buffer.break_undo_group();
                cx.notify();
            }),
        ];
        Self {
            buffer,
            selections: SelectionSet::caret(0),
            focus,
            config: EditorConfig::default(),
            path: None,
            marked: None,
            scroll: point(px(0.0), px(0.0)),
            autoscroll: false,
            layout: None,
            caret_visible: true,
            drag: None,
            highlighter: None,
            blink_epoch: 0,
            blink_task: None,
            highlight_task: None,
            word_stop: WordStop::platform(),
            extend_stack: Vec::new(),
            extend_last: None,
            center_caret: false,
            single_line: false,
            _subscriptions: subscriptions,
        }
    }

    /// Makes this editor a one-line text field (dialogs, search fields).
    pub fn set_single_line(&mut self, single_line: bool) {
        self.single_line = single_line;
    }

    // ---- state ------------------------------------------------------------

    pub(crate) fn focus_handle_ref(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Mutable access for owners that load or save (no undo step).
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    pub fn config(&self) -> &EditorConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: EditorConfig) {
        self.config = config;
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn is_modified(&self) -> bool {
        self.buffer.is_modified()
    }

    pub fn set_word_stop(&mut self, stop: WordStop) {
        self.word_stop = stop;
    }

    pub fn set_highlighter(
        &mut self,
        highlighter: Option<Box<dyn Highlighter>>,
        cx: &mut Context<Self>,
    ) {
        self.highlighter = highlighter;
        self.highlight_task = None;
        if let Some(h) = &mut self.highlighter {
            h.reset(&self.buffer);
        }
        self.schedule_highlighting(cx);
        cx.notify();
    }

    /// Highlight spans of one line as the highlighter has them now (empty
    /// while the first parse runs).
    pub fn highlights(&mut self, line: usize) -> Vec<crate::highlight::Span> {
        let mut out = Vec::new();
        if line < self.buffer.len_lines()
            && let Some(h) = &mut self.highlighter
        {
            h.prepare(&self.buffer, line..line + 1);
            h.line_spans(line, &mut out);
        }
        out
    }

    /// Highlighting reflects the current text.
    pub fn highlighting_current(&self) -> bool {
        self.highlighter.as_ref().is_none_or(|h| h.is_current())
    }

    /// Runs the highlighter's next job on the background executor; the
    /// result is applied on the UI thread and the next job follows.
    fn schedule_highlighting(&mut self, cx: &mut Context<Self>) {
        if self.highlight_task.is_some() {
            return;
        }
        let Some(job) = self
            .highlighter
            .as_mut()
            .and_then(|h| h.next_job(&self.buffer))
        else {
            return;
        };
        let work = cx.background_spawn(async move { job() });
        self.highlight_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.highlight_task = None;
                if let Some(h) = &mut this.highlighter {
                    h.job_done(result, &this.buffer);
                }
                this.schedule_highlighting(cx);
                cx.notify();
            });
        }));
    }

    pub fn highlighter(&self) -> Option<&dyn Highlighter> {
        self.highlighter.as_deref()
    }

    pub fn highlighter_mut(&mut self) -> Option<&mut (dyn Highlighter + 'static)> {
        self.highlighter.as_deref_mut()
    }

    /// Zero-based position of the primary caret.
    pub fn caret_point(&self) -> Point {
        self.buffer.offset_to_point(self.selections.primary().head)
    }

    /// Selected chars and line breaks of the primary selection.
    pub fn selection_stats(&self) -> (usize, usize) {
        let range = self.selections.primary().range();
        if range.is_empty() {
            return (0, 0);
        }
        let breaks = self.buffer.char_to_line(range.end) - self.buffer.char_to_line(range.start);
        (range.len(), breaks)
    }

    /// Vertical scroll offset in pixels.
    pub fn scroll_top(&self) -> Pixels {
        self.scroll.y
    }

    /// First and last line drawn in the last frame.
    pub fn visible_lines(&self) -> Option<Range<usize>> {
        self.layout.as_ref().map(|l| l.visible.clone())
    }

    /// Geometry of the last frame: window position of line 0, column 0
    /// (scrolling applied), line height and the width of a character.
    pub fn text_geometry(&self) -> Option<(gpui::Point<Pixels>, Pixels, Pixels)> {
        let l = self.layout.as_ref()?;
        Some((
            point(l.text_x - self.scroll.x, l.bounds.top() - self.scroll.y),
            l.line_height,
            l.char_width,
        ))
    }

    /// Lines shaped in the last frame (virtualization check).
    pub fn shaped_line_count(&self) -> usize {
        self.layout.as_ref().map_or(0, |l| l.lines.len())
    }

    /// The editor's state as JSON (`editor.state`).
    pub fn state_json(&self) -> serde_json::Value {
        let point = |p: usize| {
            let p = self.buffer.offset_to_point(p);
            json!({ "line": p.line, "column": p.column })
        };
        let (chars, breaks) = self.selection_stats();
        json!({
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "lines": self.buffer.len_lines(),
            "chars": self.buffer.len_chars(),
            "version": self.buffer.version(),
            "modified": self.buffer.is_modified(),
            "read_only": self.buffer.is_read_only(),
            "line_ending": self.buffer.line_ending().label(),
            "encoding": self.buffer.encoding().label(),
            "caret": point(self.selections.primary().head),
            "selected_chars": chars,
            "selected_line_breaks": breaks,
            "selections": self.selections.iter().map(|s| json!({
                "anchor": point(s.anchor),
                "head": point(s.head),
            })).collect::<Vec<_>>(),
            "can_undo": self.buffer.can_undo(),
            "can_redo": self.buffer.can_redo(),
            "visible_lines": self.visible_lines().map(|r| json!([r.start, r.end])),
        })
    }

    // ---- changes --------------------------------------------------------

    /// Applies edits as one undo step and moves the carets.
    pub fn edit(
        &mut self,
        edits: Vec<Edit>,
        kind: EditKind,
        carets: CaretPlacement,
        cx: &mut Context<Self>,
    ) {
        let mut selections = self.selections.clone();
        let applied = self.buffer.transact(&mut selections, edits, kind, carets);
        if applied.is_empty() {
            return;
        }
        self.selections = selections;
        self.changed(applied, cx);
    }

    fn changed(&mut self, applied: Vec<AppliedEdit>, cx: &mut Context<Self>) {
        if let Some(h) = &mut self.highlighter {
            h.edited(&self.buffer, &applied);
        }
        self.schedule_highlighting(cx);
        self.autoscroll = true;
        self.caret_visible = true;
        self.blink_epoch += 1;
        cx.emit(EditorEvent::Edited(applied));
        cx.emit(EditorEvent::SelectionsChanged);
        cx.notify();
    }

    /// Replaces the selections; the caret is shown and scrolled into view.
    pub fn set_selections(&mut self, selections: SelectionSet, cx: &mut Context<Self>) {
        let mut selections = selections;
        selections.clamp(self.buffer.len_chars());
        if selections != self.selections {
            self.buffer.break_undo_group();
        }
        self.selections = selections;
        self.marked = None;
        self.autoscroll = true;
        self.caret_visible = true;
        self.blink_epoch += 1;
        cx.emit(EditorEvent::SelectionsChanged);
        cx.notify();
    }

    /// Inserts `text` at every caret, replacing the selections.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let kind = if text.contains('\n') {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        let edits = self
            .selections
            .iter()
            .map(|s| Edit::new(s.range(), text))
            .collect();
        self.edit(edits, kind, CaretPlacement::AfterEachEdit, cx);
    }

    /// Notifies observers that the buffer was replaced from disk or saved.
    pub fn buffer_reloaded(&mut self, cx: &mut Context<Self>) {
        self.selections.clamp(self.buffer.len_chars());
        if let Some(h) = &mut self.highlighter {
            h.reset(&self.buffer);
        }
        self.highlight_task = None;
        self.schedule_highlighting(cx);
        cx.emit(EditorEvent::SelectionsChanged);
        cx.notify();
    }

    // ---- caret blinking ------------------------------------------------------

    fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.caret_visible = true;
        self.blink_epoch += 1;
        self.blink_task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut seen = None;
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update(cx, |this, cx| {
                    // Typing or moving shows the caret for a full interval.
                    if seen == Some(this.blink_epoch) {
                        this.caret_visible = !this.caret_visible;
                    } else {
                        this.caret_visible = true;
                    }
                    seen = Some(this.blink_epoch);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    // ---- movement -------------------------------------------------------------

    fn move_carets(
        &mut self,
        extend: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&Self, &Selection) -> usize,
    ) {
        let mut set = self.selections.clone();
        let targets: Vec<usize> = set.iter().map(|s| f(self, s)).collect();
        let mut i = 0;
        set.update(|s| {
            let to = targets[i];
            i += 1;
            if extend {
                s.extend_to(to);
            } else {
                s.collapse_to(to);
            }
        });
        self.set_selections(set, cx);
    }

    fn vertical(&mut self, lines: isize, extend: bool, cx: &mut Context<Self>) {
        let tab = self.config.tab_size;
        let buffer = &self.buffer;
        let mut set = self.selections.clone();
        set.update(|s| {
            let head = if !extend && !s.is_empty() {
                if lines < 0 { s.start() } else { s.end() }
            } else {
                s.head
            };
            let p = buffer.offset_to_point(head);
            let goal = s
                .goal
                .unwrap_or_else(|| movement::display_column(buffer, p.line, p.column, tab));
            let last = buffer.len_lines() - 1;
            let target_line = (p.line as isize + lines).clamp(0, last as isize) as usize;
            let to = if target_line == p.line {
                // Up on the first line or down on the last: line start or end.
                if lines < 0 && p.line == 0 {
                    0
                } else if lines > 0 && p.line == last {
                    buffer.len_chars()
                } else {
                    head
                }
            } else {
                buffer.line_start(target_line)
                    + movement::column_for_display(buffer, target_line, goal, tab)
            };
            if extend {
                s.head = to;
            } else {
                s.anchor = to;
                s.head = to;
            }
            s.goal = Some(goal);
        });
        self.set_selections(set, cx);
    }

    fn page_lines(&self) -> isize {
        self.layout
            .as_ref()
            .map_or(30, |l| l.viewport_lines.saturating_sub(1).max(1)) as isize
    }

    fn page(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let n = self.page_lines();
        let line_height = self.layout.as_ref().map_or(px(20.0), |l| l.line_height);
        let delta = line_height * n as f32;
        self.scroll.y = if down {
            self.scroll.y + delta
        } else {
            (self.scroll.y - delta).max(px(0.0))
        };
        self.vertical(if down { n } else { -n }, extend, cx);
    }

    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| {
            if s.is_empty() {
                movement::prev_grapheme(&e.buffer, s.head)
            } else {
                s.start()
            }
        });
    }

    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| {
            if s.is_empty() {
                movement::next_grapheme(&e.buffer, s.head)
            } else {
                s.end()
            }
        });
    }

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, cx);
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, cx);
    }

    fn move_word_left(&mut self, _: &MoveWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| movement::word_left(&e.buffer, s.head));
    }

    fn move_word_right(&mut self, _: &MoveWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let stop = self.word_stop;
        self.move_carets(false, cx, move |e, s| {
            movement::word_right(&e.buffer, s.head, stop)
        });
    }

    fn move_line_start(&mut self, _: &MoveLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| {
            movement::smart_line_start(&e.buffer, s.head)
        });
    }

    fn move_line_end(&mut self, _: &MoveLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(false, cx, |e, s| {
            e.buffer.line_end(e.buffer.char_to_line(s.head))
        });
    }

    fn move_document_start(
        &mut self,
        _: &MoveDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_selections(SelectionSet::caret(0), cx);
    }

    fn move_document_end(&mut self, _: &MoveDocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.set_selections(SelectionSet::caret(self.buffer.len_chars()), cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(false, false, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(true, false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| movement::prev_grapheme(&e.buffer, s.head));
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| movement::next_grapheme(&e.buffer, s.head));
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| movement::word_left(&e.buffer, s.head));
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let stop = self.word_stop;
        self.move_carets(true, cx, move |e, s| {
            movement::word_right(&e.buffer, s.head, stop)
        });
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| {
            movement::smart_line_start(&e.buffer, s.head)
        });
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_carets(true, cx, |e, s| {
            e.buffer.line_end(e.buffer.char_to_line(s.head))
        });
    }

    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_carets(true, cx, |_, _| 0);
    }

    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_carets(true, cx, |e, _| e.buffer.len_chars());
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(false, true, cx);
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(true, true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let len = self.buffer.len_chars();
        self.set_selections(SelectionSet::single(Selection::new(0, len)), cx);
    }

    /// Extend Selection: the word at the caret, then the enclosing syntax
    /// nodes (when the file type has a syntax tree), then the text of the
    /// lines, the whole lines and the whole file; always the smallest range
    /// that is larger than the selection.
    fn select_word(&mut self, _: &SelectWord, _: &mut Window, cx: &mut Context<Self>) {
        if self.extend_last.as_ref() != Some(&self.selections) {
            self.extend_stack.clear();
        }
        let s = *self.selections.primary();
        let range = s.range();
        let mut candidates = vec![movement::word_range(&self.buffer, s.head)];
        if let Some(h) = &self.highlighter {
            candidates.extend(h.enclosing_ranges(&self.buffer, range.clone()));
        }
        let lines = self.selected_lines(&s);
        let first_start = self.buffer.line_start(lines.start);
        let last_end = self.buffer.line_end(lines.end - 1);
        let first_text = self.buffer.line_text(lines.start);
        let last_text = self.buffer.line_text(lines.end - 1);
        let content_start = first_start + leading_ws(&first_text);
        let content_end =
            last_end - (last_text.chars().count() - last_text.trim_end().chars().count());
        candidates.push(content_start..content_end.max(content_start));
        candidates.push(first_start..last_end);
        let after_last = if lines.end < self.buffer.len_lines() {
            self.buffer.line_start(lines.end)
        } else {
            self.buffer.len_chars()
        };
        candidates.push(first_start..after_last);
        candidates.push(0..self.buffer.len_chars());
        let Some(next) = candidates
            .into_iter()
            .filter(|c| c.start <= range.start && range.end <= c.end && c.len() > range.len())
            .min_by_key(|c| c.len())
        else {
            return;
        };
        self.extend_stack.push(self.selections.clone());
        self.set_selections(
            SelectionSet::single(Selection::new(next.start, next.end)),
            cx,
        );
        self.extend_last = Some(self.selections.clone());
    }

    fn shrink_selection(&mut self, _: &ShrinkSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.extend_last.as_ref() != Some(&self.selections) {
            self.extend_stack.clear();
            return;
        }
        if let Some(previous) = self.extend_stack.pop() {
            self.set_selections(previous, cx);
            self.extend_last = Some(self.selections.clone());
        }
    }

    fn escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        if self.selections.len() == 1 && self.selections.primary().is_empty() {
            cx.propagate();
            return;
        }
        let head = self.selections.primary().head;
        self.set_selections(SelectionSet::caret(head), cx);
    }

    fn go_to_line(&mut self, a: &GoToLine, _: &mut Window, cx: &mut Context<Self>) {
        self.go_to(a.line, a.column, cx);
    }

    /// Moves the caret to a one-based line and column (both clamped) and
    /// centers the line in the view.
    pub fn go_to(&mut self, line: usize, column: Option<usize>, cx: &mut Context<Self>) {
        let line = (line.max(1) - 1).min(self.buffer.len_lines() - 1);
        let column = column.unwrap_or(1).max(1) - 1;
        let pos = self.buffer.line_start(line) + column.min(self.buffer.line_len(line));
        self.set_selections(SelectionSet::caret(pos), cx);
        self.center_caret = true;
    }

    fn set_selection(&mut self, a: &SetSelection, _: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.buffer.point_to_offset(a.anchor.into());
        let head = self.buffer.point_to_offset(a.head.into());
        self.set_selections(SelectionSet::single(Selection::new(anchor, head)), cx);
    }

    fn scroll_to_line(&mut self, a: &ScrollToLine, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.layout.as_ref().map_or(px(20.0), |l| l.line_height);
        let line = a.line.min(self.buffer.len_lines().saturating_sub(1));
        self.scroll.y = line_height * line as f32;
        cx.notify();
    }

    // ---- editing --------------------------------------------------------------

    fn type_text(&mut self, a: &Type, _: &mut Window, cx: &mut Context<Self>) {
        self.type_str(&a.text, cx);
    }

    /// Typed text: a single character goes through bracket and quote
    /// pairing, anything else is inserted as it is.
    pub(crate) fn type_str(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if self.marked.is_none() => self.type_char(c, cx),
            _ => self.insert(text, cx),
        }
    }

    /// The caret is between an opening bracket or quote and its closing one.
    fn is_empty_pair_at(&self, pos: usize) -> bool {
        let (Some(prev), Some(next)) = (
            pos.checked_sub(1).and_then(|p| self.buffer.char_at(p)),
            self.buffer.char_at(pos),
        ) else {
            return false;
        };
        self.config.brackets.contains(&(prev, next))
            || (prev == next && self.config.quotes.contains(&prev))
    }

    fn closing_bracket(&self, c: char) -> Option<char> {
        self.config.brackets.iter().find(|b| b.0 == c).map(|b| b.1)
    }

    /// Types one character at the caret: wraps a selection in brackets or
    /// quotes, types over a closing bracket or quote, closes an opening
    /// one, and turns a third quote into a triple-quoted pair.
    fn type_char(&mut self, c: char, cx: &mut Context<Self>) {
        let is_quote = self.config.quotes.contains(&c);
        let closing = self.closing_bracket(c).or(is_quote.then_some(c));
        let is_closer = is_quote || self.config.brackets.iter().any(|b| b.1 == c);
        if self.selections.len() != 1 || (closing.is_none() && !is_closer) {
            self.insert(&c.to_string(), cx);
            return;
        }
        let s = *self.selections.primary();
        if !s.is_empty() {
            match closing {
                Some(close) => {
                    let text = format!("{c}{}{close}", self.buffer.slice(s.range()));
                    let after = Selection::new(s.anchor + 1, s.head + 1);
                    self.edit(
                        vec![Edit::new(s.range(), text)],
                        EditKind::Other,
                        CaretPlacement::Explicit(SelectionSet::single(after)),
                        cx,
                    );
                }
                None => self.insert(&c.to_string(), cx),
            }
            return;
        }
        let pos = s.head;
        let next = self.buffer.char_at(pos);
        let prev = pos.checked_sub(1).and_then(|p| self.buffer.char_at(p));
        let line = self.buffer.char_to_line(pos);
        let before = self.buffer.slice(self.buffer.line_start(line)..pos);
        // A third quote right after an empty pair: a triple-quoted string.
        if is_quote && self.config.triple_quotes {
            let tail: Vec<char> = before.chars().rev().take(3).collect();
            if tail.len() >= 2 && tail[0] == c && tail[1] == c && tail.get(2) != Some(&c) {
                let text: String = std::iter::repeat_n(c, 4).collect();
                let caret = pos + 1;
                self.edit(
                    vec![Edit::insert(pos, text)],
                    EditKind::Typing,
                    CaretPlacement::Explicit(SelectionSet::caret(caret)),
                    cx,
                );
                return;
            }
        }
        // Inside a string of this quote: an odd count of it before the caret.
        let inside = is_quote && before.chars().filter(|q| *q == c).count() % 2 == 1;
        // Type over the closing bracket or quote.
        if is_closer && next == Some(c) && (!is_quote || inside) {
            self.buffer.break_undo_group();
            self.set_selections(SelectionSet::caret(pos + 1), cx);
            return;
        }
        let Some(close) = closing else {
            self.insert(&c.to_string(), cx);
            return;
        };
        let next_allows = next.is_none_or(|n| {
            n.is_whitespace()
                || self.config.brackets.iter().any(|b| b.1 == n)
                || matches!(n, ',' | ';' | ':')
        });
        let quote_allowed = !is_quote || {
            // Not inside a string already and not right after a word, unless
            // the word is a string prefix.
            let word: String = before
                .chars()
                .rev()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect();
            let prefix_ok = word.is_empty()
                || (word.chars().count() <= 2
                    && word
                        .chars()
                        .all(|ch| self.config.string_prefixes.contains(&ch)));
            !inside && prefix_ok && prev != Some(c)
        };
        if next_allows && quote_allowed {
            self.edit(
                vec![Edit::insert(pos, format!("{c}{close}"))],
                EditKind::Typing,
                CaretPlacement::Explicit(SelectionSet::caret(pos + 1)),
                cx,
            );
        } else {
            self.insert(&c.to_string(), cx);
        }
    }

    /// Leading whitespace of the line of `pos`, up to `pos`.
    fn indent_at(&self, pos: usize) -> String {
        let line = self.buffer.char_to_line(pos);
        let start = self.buffer.line_start(line);
        self.buffer
            .line(line)
            .chars()
            .take(pos - start)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    /// Indentation of a new line started at `pos`: the indentation of the
    /// line, one level more after a block opener (`:` in Python) or an
    /// unclosed bracket, one level less after a block-ending word.
    fn new_line_indent(&self, pos: usize) -> String {
        let mut indent = self.indent_at(pos);
        let line = self.buffer.char_to_line(pos);
        let start = self.buffer.line_start(line);
        let before: String = self.buffer.slice(start..pos);
        let trimmed = before.trim();
        let last = trimmed.chars().last();
        let opens_block = last.is_some_and(|c| {
            self.config.indent_after.contains(&c) || self.config.brackets.iter().any(|b| b.0 == c)
        });
        let first_word: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if opens_block {
            indent.push_str(&" ".repeat(self.config.indent));
        } else if !first_word.is_empty() && self.config.dedent_after.contains(&first_word) {
            let width = indent.chars().count();
            let keep = width.saturating_sub(self.config.indent);
            indent = indent.chars().take(keep).collect();
        }
        indent
    }

    fn newline_text(&self, pos: usize) -> String {
        format!("\n{}", self.new_line_indent(pos))
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        // Enter between an empty bracket pair puts the closing bracket on a
        // line of its own below the caret line.
        if self.selections.len() == 1 && self.selections.primary().is_empty() {
            let pos = self.selections.primary().head;
            let prev = pos.checked_sub(1).and_then(|p| self.buffer.char_at(p));
            let next = self.buffer.char_at(pos);
            if let (Some(p), Some(n)) = (prev, next)
                && self.config.brackets.contains(&(p, n))
            {
                let inner = self.newline_text(pos);
                let outer = format!("\n{}", self.indent_at(pos));
                let caret = pos + inner.chars().count();
                self.edit(
                    vec![Edit::insert(pos, format!("{inner}{outer}"))],
                    EditKind::Other,
                    CaretPlacement::Explicit(SelectionSet::caret(caret)),
                    cx,
                );
                return;
            }
        }
        let edits = self
            .selections
            .iter()
            .map(|s| Edit::new(s.range(), self.newline_text(s.start())))
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    fn start_new_line(&mut self, _: &StartNewLine, _: &mut Window, cx: &mut Context<Self>) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                let end = self.buffer.line_end(self.buffer.char_to_line(s.head));
                Edit::insert(end, self.newline_text(end))
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    /// Lines touched by a selection (a selection ending at column 0 does not
    /// touch that line).
    fn selected_lines(&self, s: &Selection) -> Range<usize> {
        let first = self.buffer.char_to_line(s.start());
        let mut last = self.buffer.char_to_line(s.end());
        if last > first && self.buffer.offset_to_point(s.end()).column == 0 {
            last -= 1;
        }
        first..last + 1
    }

    fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        let multiline = self
            .selections
            .iter()
            .any(|s| self.buffer.char_to_line(s.start()) != self.buffer.char_to_line(s.end()));
        if multiline {
            self.indent_lines(true, cx);
            return;
        }
        let tab = self.config.tab_size;
        let indent = self.config.indent;
        let edits = self
            .selections
            .iter()
            .map(|s| {
                let p = self.buffer.offset_to_point(s.start());
                let col = movement::display_column(&self.buffer, p.line, p.column, tab);
                Edit::new(s.range(), " ".repeat(indent - col % indent))
            })
            .collect();
        self.edit(edits, EditKind::Typing, CaretPlacement::AfterEachEdit, cx);
    }

    fn unindent(&mut self, _: &Unindent, _: &mut Window, cx: &mut Context<Self>) {
        self.indent_lines(false, cx);
    }

    /// Adds or removes one indentation level on every selected line; the
    /// selections grow and shrink with their lines.
    fn indent_lines(&mut self, add: bool, cx: &mut Context<Self>) {
        let indent = self.config.indent;
        let mut lines: Vec<usize> = self
            .selections
            .iter()
            .flat_map(|s| self.selected_lines(s))
            .collect();
        lines.dedup();
        let mut edits = Vec::new();
        for line in lines {
            let start = self.buffer.line_start(line);
            if add {
                if self.buffer.line_len(line) > 0 {
                    edits.push(Edit::insert(start, " ".repeat(indent)));
                }
            } else {
                let text = self.buffer.line(line);
                let n = if text.chars().next() == Some('\t') {
                    1
                } else {
                    text.chars().take(indent).take_while(|c| *c == ' ').count()
                };
                if n > 0 {
                    edits.push(Edit::delete(start..start + n));
                }
            }
        }
        // Selections follow the text: insertions at a line start do not move
        // a position at that start, deletions pull positions inside them to
        // their start.
        let map = |pos: usize| -> usize {
            let mut delta: isize = 0;
            for e in &edits {
                let (s, end) = (e.range.start, e.range.end);
                if e.text.is_empty() {
                    delta -= (pos.min(end).saturating_sub(s)) as isize;
                } else if s < pos {
                    delta += e.text.chars().count() as isize;
                }
            }
            (pos as isize + delta).max(0) as usize
        };
        let after = SelectionSet::from_vec(
            self.selections
                .iter()
                .map(|s| Selection::new(map(s.anchor), map(s.head)))
                .collect(),
        );
        self.edit(edits, EditKind::Other, CaretPlacement::Explicit(after), cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let indent = self.config.indent;
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return Edit::delete(s.range());
                }
                if self.is_empty_pair_at(s.head) {
                    return Edit::delete(s.head - 1..s.head + 1);
                }
                let p = self.buffer.offset_to_point(s.head);
                let line_start = s.head - p.column;
                let before = self.buffer.slice(line_start..s.head);
                if p.column > 0 && before.chars().all(|c| c == ' ') {
                    // In leading spaces: back to the previous indentation stop.
                    let target = (p.column - 1) / indent * indent;
                    return Edit::delete(line_start + target..s.head);
                }
                Edit::delete(movement::prev_grapheme(&self.buffer, s.head)..s.head)
            })
            .collect();
        self.edit(
            edits,
            EditKind::DeleteBackward,
            CaretPlacement::AfterEachEdit,
            cx,
        );
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Edit::delete(s.head..movement::next_grapheme(&self.buffer, s.head))
                } else {
                    Edit::delete(s.range())
                }
            })
            .collect();
        self.edit(
            edits,
            EditKind::DeleteForward,
            CaretPlacement::AfterEachEdit,
            cx,
        );
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Edit::delete(movement::word_left(&self.buffer, s.head)..s.head)
                } else {
                    Edit::delete(s.range())
                }
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let stop = self.word_stop;
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if s.is_empty() {
                    Edit::delete(s.head..movement::word_right(&self.buffer, s.head, stop))
                } else {
                    Edit::delete(s.range())
                }
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    /// Char range of whole lines, including the line break after them (or
    /// before them for the last line).
    fn lines_range(&self, lines: Range<usize>) -> Range<usize> {
        let start = self.buffer.line_start(lines.start);
        if lines.end < self.buffer.len_lines() {
            start..self.buffer.line_start(lines.end)
        } else if start > 0 {
            start - 1..self.buffer.len_chars()
        } else {
            0..self.buffer.len_chars()
        }
    }

    fn delete_line(&mut self, _: &DeleteLine, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.caret_point();
        let primary = self.selected_lines(self.selections.primary());
        let mut ranges: Vec<Range<usize>> = self
            .selections
            .iter()
            .map(|s| self.lines_range(self.selected_lines(s)))
            .collect();
        ranges.sort_by_key(|r| r.start);
        ranges.dedup();
        // The caret keeps its column on the line that takes the place of the
        // deleted block (the line before it when the block was the last).
        let len_lines = self.buffer.len_lines();
        let target = if primary.end < len_lines {
            Some(primary.end)
        } else {
            primary.start.checked_sub(1)
        };
        let caret = target.map_or(0, |line| {
            let old = self.buffer.line_start(line);
            let removed: usize = ranges
                .iter()
                .map(|r| old.min(r.end).saturating_sub(r.start))
                .sum();
            old - removed + p.column.min(self.buffer.line_len(line))
        });
        let edits = ranges.into_iter().map(Edit::delete).collect();
        self.edit(
            edits,
            EditKind::Other,
            CaretPlacement::Explicit(SelectionSet::caret(caret)),
            cx,
        );
    }

    fn duplicate(&mut self, _: &DuplicateLine, _: &mut Window, cx: &mut Context<Self>) {
        let s = *self.selections.primary();
        if s.is_empty() {
            let line = self.buffer.char_to_line(s.head);
            let text = self.buffer.line_text(line).into_owned();
            let end = self.buffer.line_end(line);
            let caret = s.head + text.chars().count() + 1;
            self.edit(
                vec![Edit::insert(end, format!("\n{text}"))],
                EditKind::Other,
                CaretPlacement::Explicit(SelectionSet::caret(caret)),
                cx,
            );
        } else {
            let text = self.buffer.slice(s.range());
            let len = s.range().len();
            let after = Selection::new(s.anchor + len, s.head + len);
            self.edit(
                vec![Edit::insert(s.end(), text)],
                EditKind::Other,
                CaretPlacement::Explicit(SelectionSet::single(after)),
                cx,
            );
        }
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return Edit::delete(s.range());
                }
                let start = self.buffer.line_start(self.buffer.char_to_line(s.head));
                // At the line start: join with the previous line.
                Edit::delete(if start == s.head {
                    s.head.saturating_sub(1)..s.head
                } else {
                    start..s.head
                })
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    fn delete_to_line_end(&mut self, _: &DeleteToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let edits = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return Edit::delete(s.range());
                }
                let end = self.buffer.line_end(self.buffer.char_to_line(s.head));
                // At the line end: join with the next line.
                Edit::delete(if end == s.head {
                    s.head..(s.head + 1).min(self.buffer.len_chars())
                } else {
                    s.head..end
                })
            })
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    /// Lines covered by all selections.
    fn all_selected_lines(&self) -> Range<usize> {
        let mut lines = self.selected_lines(self.selections.primary());
        for s in self.selections.iter() {
            let l = self.selected_lines(s);
            lines.start = lines.start.min(l.start);
            lines.end = lines.end.max(l.end);
        }
        lines
    }

    fn line_texts(&self, lines: Range<usize>) -> Vec<String> {
        lines
            .map(|l| self.buffer.line_text(l).into_owned())
            .collect()
    }

    /// Replaces whole lines `region` with `lines` and moves every
    /// selection by `shift` chars.
    fn replace_lines(
        &mut self,
        region: Range<usize>,
        lines: Vec<String>,
        shift: isize,
        cx: &mut Context<Self>,
    ) {
        let start = self.buffer.line_start(region.start);
        let end = self.buffer.line_end(region.end - 1);
        let moved = |p: usize| (p as isize + shift).max(0) as usize;
        let after = SelectionSet::from_vec(
            self.selections
                .iter()
                .map(|s| Selection::new(moved(s.anchor), moved(s.head)))
                .collect(),
        );
        self.edit(
            vec![Edit::new(start..end, lines.join("\n"))],
            EditKind::Other,
            CaretPlacement::Explicit(after),
            cx,
        );
    }

    /// Swaps the lines `block` with the line above or below.
    fn move_lines(&mut self, block: Range<usize>, up: bool, cx: &mut Context<Self>) {
        let n = self.buffer.len_lines();
        if (up && block.start == 0) || (!up && block.end >= n) {
            return;
        }
        let mut lines = self.line_texts(block.clone());
        if up {
            let above = self.buffer.line_text(block.start - 1).into_owned();
            let shift = -(above.chars().count() as isize + 1);
            lines.push(above);
            self.replace_lines(block.start - 1..block.end, lines, shift, cx);
        } else {
            let below = self.buffer.line_text(block.end).into_owned();
            let shift = below.chars().count() as isize + 1;
            lines.insert(0, below);
            self.replace_lines(block.start..block.end + 1, lines, shift, cx);
        }
    }

    fn move_line_up(&mut self, _: &MoveLineUp, _: &mut Window, cx: &mut Context<Self>) {
        let block = self.all_selected_lines();
        self.move_lines(block, true, cx);
    }

    fn move_line_down(&mut self, _: &MoveLineDown, _: &mut Window, cx: &mut Context<Self>) {
        let block = self.all_selected_lines();
        self.move_lines(block, false, cx);
    }

    /// Indentation width of a line, `None` for a blank line.
    fn line_indent(&self, line: usize) -> Option<usize> {
        let text = self.buffer.line_text(line);
        let tab = self.config.tab_size;
        if text.trim().is_empty() {
            return None;
        }
        let mut width = 0;
        for c in text.chars() {
            match c {
                ' ' => width += 1,
                '\t' => width += tab - width % tab,
                _ => break,
            }
        }
        Some(width)
    }

    fn starts_with_closer(&self, line: usize) -> bool {
        let text = self.buffer.line_text(line);
        let first = text.trim_start().chars().next();
        first.is_some_and(|c| self.config.brackets.iter().any(|b| b.1 == c))
    }

    /// End (exclusive) of the statement starting at `line`: the line and
    /// the following lines indented deeper (or closing a bracket at the same
    /// indentation), without trailing blank lines.
    fn statement_end(&self, line: usize) -> usize {
        let Some(indent) = self.line_indent(line) else {
            return line + 1;
        };
        let mut end = line + 1;
        for l in line + 1..self.buffer.len_lines() {
            match self.line_indent(l) {
                None => continue,
                Some(i) if i > indent || (i == indent && self.starts_with_closer(l)) => end = l + 1,
                Some(_) => break,
            }
        }
        end
    }

    /// Start of the statement before `line` at the same indentation.
    fn previous_statement(&self, line: usize, indent: usize) -> Option<usize> {
        for l in (0..line).rev() {
            match self.line_indent(l) {
                None => continue,
                Some(i) if i > indent || (i == indent && self.starts_with_closer(l)) => continue,
                Some(i) if i == indent => return Some(l),
                Some(_) => return None,
            }
        }
        None
    }

    /// Start of the statement after the one ending at `end`.
    fn next_statement(&self, end: usize, indent: usize) -> Option<usize> {
        (end..self.buffer.len_lines())
            .find_map(|l| self.line_indent(l).map(|i| (l, i)))
            .and_then(|(l, i)| (i == indent).then_some(l))
    }

    /// Swaps the statement at the caret (with its block) with the previous
    /// or next statement of the same indentation; blank lines between them
    /// stay where they are. On a blank line it moves the line.
    fn move_statement(&mut self, up: bool, cx: &mut Context<Self>) {
        let lines = self.all_selected_lines();
        let Some(indent) = self.line_indent(lines.start) else {
            self.move_lines(lines, up, cx);
            return;
        };
        let end = self.statement_end(lines.start).max(lines.end);
        let block = self.line_texts(lines.start..end);
        if up {
            let Some(prev) = self.previous_statement(lines.start, indent) else {
                return;
            };
            let prev_end = self.statement_end(prev);
            let other = self.line_texts(prev..prev_end);
            let gap = self.line_texts(prev_end..lines.start);
            let shift = -(self.buffer.line_start(lines.start) as isize
                - self.buffer.line_start(prev) as isize);
            let mut out = block;
            out.extend(gap);
            out.extend(other);
            self.replace_lines(prev..end, out, shift, cx);
        } else {
            let Some(next) = self.next_statement(end, indent) else {
                return;
            };
            let next_end = self.statement_end(next);
            let other = self.line_texts(next..next_end);
            let gap = self.line_texts(end..next);
            let moved: usize = other
                .iter()
                .chain(&gap)
                .map(|l| l.chars().count() + 1)
                .sum();
            let mut out = other;
            out.extend(gap);
            out.extend(block);
            self.replace_lines(lines.start..next_end, out, moved as isize, cx);
        }
    }

    fn move_statement_up(&mut self, _: &MoveStatementUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_statement(true, cx);
    }

    fn move_statement_down(
        &mut self,
        _: &MoveStatementDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_statement(false, cx);
    }

    /// Joins the next line to the caret line, or all selected lines: the
    /// line break and the indentation after it become one space (none
    /// next to brackets or blank lines).
    fn join_lines(&mut self, _: &JoinLines, _: &mut Window, cx: &mut Context<Self>) {
        let lines = self.selected_lines(self.selections.primary());
        let last = if lines.len() > 1 {
            lines.end - 1
        } else {
            lines.start + 1
        };
        if last >= self.buffer.len_lines() {
            return;
        }
        let mut edits = Vec::new();
        for l in lines.start..last {
            let text = self.buffer.line_text(l);
            let next = self.buffer.line_text(l + 1);
            let trimmed = text.trim_end();
            let cut_start = self.buffer.line_start(l) + trimmed.chars().count();
            let next_start = self.buffer.line_start(l + 1);
            let cut_end = next_start + leading_ws(&next);
            let opens = trimmed
                .chars()
                .last()
                .is_some_and(|c| self.closing_bracket(c).is_some());
            let closes = next
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| self.config.brackets.iter().any(|b| b.1 == c));
            let sep =
                if trimmed.trim_start().is_empty() || next.trim().is_empty() || opens || closes {
                    ""
                } else {
                    " "
                };
            edits.push(Edit::new(cut_start..cut_end, sep));
        }
        let first = &edits[0];
        let caret = first.range.start;
        let caret = SelectionSet::caret(caret);
        self.edit(edits, EditKind::Other, CaretPlacement::Explicit(caret), cx);
    }

    /// Comments the selected lines (or the caret line) with the file type's
    /// line comment at their smallest indentation, or uncomments them when
    /// every non-blank line is commented. Without a selection the caret
    /// moves to the next line.
    fn toggle_comment(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        let Some(prefix) = self.config.line_comment.clone() else {
            return;
        };
        let marker = prefix.trim_end().to_string();
        let lines = self.all_selected_lines();
        let texts: Vec<(usize, String)> = lines
            .clone()
            .map(|l| (l, self.buffer.line_text(l).into_owned()))
            .collect();
        let code: Vec<&(usize, String)> =
            texts.iter().filter(|(_, t)| !t.trim().is_empty()).collect();
        let commented = !code.is_empty()
            && code
                .iter()
                .all(|(_, t)| t.trim_start().starts_with(&marker));
        let mut edits = Vec::new();
        if commented {
            for (l, t) in &code {
                let at = self.buffer.line_start(*l) + leading_ws(t);
                let rest = &t.trim_start()[marker.len()..];
                let space = usize::from(rest.starts_with(' ') && prefix.ends_with(' '));
                edits.push(Edit::delete(at..at + marker.chars().count() + space));
            }
        } else {
            let targets: Vec<&(usize, String)> = if code.is_empty() {
                texts.iter().collect()
            } else {
                code
            };
            let indent = targets
                .iter()
                .map(|(_, t)| leading_ws(t))
                .min()
                .unwrap_or(0);
            for (l, _) in targets {
                edits.push(Edit::insert(self.buffer.line_start(*l) + indent, &prefix));
            }
        }
        let single_caret = self.selections.len() == 1 && self.selections.primary().is_empty();
        let after = if single_caret && lines.end < self.buffer.len_lines() {
            // The caret goes down one line, keeping its column.
            let column = self.caret_point().column;
            let next_start = map_through(&edits, self.buffer.line_start(lines.end));
            let next_len = self.buffer.line_len(lines.end);
            SelectionSet::caret(next_start + column.min(next_len))
        } else {
            SelectionSet::from_vec(
                self.selections
                    .iter()
                    .map(|s| {
                        Selection::new(map_through(&edits, s.anchor), map_through(&edits, s.head))
                    })
                    .collect(),
            )
        };
        self.edit(edits, EditKind::Other, CaretPlacement::Explicit(after), cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((applied, selections)) = self.buffer.undo() {
            self.selections = selections;
            self.selections.clamp(self.buffer.len_chars());
            self.changed(applied, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((applied, selections)) = self.buffer.redo() {
            self.selections = selections;
            self.selections.clamp(self.buffer.len_chars());
            self.changed(applied, cx);
        }
    }

    // ---- clipboard ------------------------------------------------------------

    /// Text to copy and whether it is whole lines (nothing selected).
    fn copy_text(&self) -> (String, bool) {
        if self.selections.iter().all(|s| s.is_empty()) {
            let mut text = String::new();
            let mut last_line = None;
            for s in self.selections.iter() {
                let line = self.buffer.char_to_line(s.head);
                if last_line == Some(line) {
                    continue;
                }
                last_line = Some(line);
                text.push_str(&self.buffer.line_text(line));
                text.push('\n');
            }
            (text, true)
        } else {
            let parts: Vec<String> = self
                .selections
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| self.buffer.slice(s.range()))
                .collect();
            (parts.join("\n"), false)
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let (text, whole_lines) = self.copy_text();
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            LineCopy { whole_lines },
        ));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let (text, whole_lines) = self.copy_text();
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            text,
            LineCopy { whole_lines },
        ));
        if whole_lines {
            self.delete_line(&DeleteLine, window, cx);
        } else {
            let edits = self
                .selections
                .iter()
                .map(|s| Edit::delete(s.range()))
                .collect();
            self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text() else {
            return;
        };
        let whole_lines = item.entries().iter().any(|e| {
            matches!(e, ClipboardEntry::String(s)
                if s.metadata_json::<LineCopy>().is_some_and(|m| m.whole_lines))
        });
        let text = text::normalize_line_endings(&text).into_owned();
        if whole_lines && self.selections.iter().all(|s| s.is_empty()) {
            // Whole lines go above the caret line; the caret stays on its text.
            let len = text.chars().count();
            let edits: Vec<Edit> = self
                .selections
                .iter()
                .map(|s| {
                    Edit::insert(
                        self.buffer.line_start(self.buffer.char_to_line(s.head)),
                        text.clone(),
                    )
                })
                .collect();
            let after = SelectionSet::from_vec(
                self.selections
                    .iter()
                    .enumerate()
                    .map(|(i, s)| Selection::caret(s.head + len * (i + 1)))
                    .collect(),
            );
            self.edit(edits, EditKind::Other, CaretPlacement::Explicit(after), cx);
            return;
        }
        let edits = self
            .selections
            .iter()
            .map(|s| Edit::new(s.range(), text.clone()))
            .collect();
        self.edit(edits, EditKind::Other, CaretPlacement::AfterEachEdit, cx);
    }

    // ---- mouse ----------------------------------------------------------------

    /// Mouse press at char offset `pos` with `clicks` (1, 2, 3).
    pub(crate) fn mouse_down(
        &mut self,
        pos: usize,
        clicks: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        let s = *self.selections.primary();
        match clicks {
            2 => {
                let word = movement::word_range(&self.buffer, pos);
                self.drag = Some(DragUnit::Word(word.clone()));
                self.set_selections(
                    SelectionSet::single(Selection::new(word.start, word.end)),
                    cx,
                );
            }
            n if n >= 3 => {
                let line = self.buffer.char_to_line(pos);
                let range = self.lines_range_with_break(line);
                self.drag = Some(DragUnit::Line(range.clone()));
                self.set_selections(
                    SelectionSet::single(Selection::new(range.start, range.end)),
                    cx,
                );
            }
            _ => {
                self.drag = Some(DragUnit::Char);
                let sel = if shift {
                    Selection::new(s.anchor, pos)
                } else {
                    Selection::caret(pos)
                };
                self.set_selections(SelectionSet::single(sel), cx);
            }
        }
    }

    fn lines_range_with_break(&self, line: usize) -> Range<usize> {
        let start = self.buffer.line_start(line);
        let end = if line + 1 < self.buffer.len_lines() {
            self.buffer.line_start(line + 1)
        } else {
            self.buffer.len_chars()
        };
        start..end
    }

    /// Mouse drag to char offset `pos`.
    pub(crate) fn mouse_drag(&mut self, pos: usize, cx: &mut Context<Self>) {
        let s = *self.selections.primary();
        let sel = match self.drag.clone() {
            Some(DragUnit::Char) => Selection::new(s.anchor, pos),
            Some(DragUnit::Word(origin)) => {
                let word = movement::word_range(&self.buffer, pos);
                if pos < origin.start {
                    Selection::new(origin.end, word.start.min(pos))
                } else {
                    Selection::new(origin.start, word.end.max(pos))
                }
            }
            Some(DragUnit::Line(origin)) => {
                let line = self.lines_range_with_break(self.buffer.char_to_line(pos));
                if line.start < origin.start {
                    Selection::new(origin.end, line.start)
                } else {
                    Selection::new(origin.start, line.end)
                }
            }
            _ => return,
        };
        if sel != s {
            self.set_selections(SelectionSet::single(sel), cx);
        }
    }

    pub(crate) fn mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    /// Scrolls by a wheel or trackpad delta (positive y scrolls down).
    pub(crate) fn scroll_by(&mut self, delta: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let max_y = self.max_scroll_y();
        self.scroll.y = (self.scroll.y + delta.y).clamp(px(0.0), max_y);
        self.scroll.x = (self.scroll.x + delta.x).max(px(0.0));
        cx.notify();
    }

    pub(crate) fn max_scroll_y(&self) -> Pixels {
        let line_height = self.layout.as_ref().map_or(px(20.0), |l| l.line_height);
        line_height * self.buffer.len_lines().saturating_sub(1) as f32
    }

    pub(crate) fn set_scroll_y(&mut self, y: Pixels, cx: &mut Context<Self>) {
        self.scroll.y = y.clamp(px(0.0), self.max_scroll_y());
        cx.notify();
    }
}

macro_rules! editor_actions {
    ($($action:ty => $method:ident),* $(,)?) => {
        impl Editor {
            /// The editor's own action handlers.
            fn bind_actions(el: Div, cx: &mut Context<Self>) -> Div {
                el $(.on_action(cx.listener(|this, a: &$action, window, cx| this.$method(a, window, cx))))*
            }
        }

        /// Adds handlers to `el` that forward every editor command to
        /// `editor`; the workbench uses this so commands reach the active
        /// editor when another component has the focus.
        pub fn forward_actions<E: InteractiveElement>(el: E, editor: Entity<Editor>) -> E {
            el $(.on_action({
                let editor = editor.clone();
                move |a: &$action, window, cx| editor.update(cx, |e, cx| e.$method(a, window, cx))
            }))*
        }
    };
}

editor_actions! {
    MoveLeft => move_left,
    MoveRight => move_right,
    MoveUp => move_up,
    MoveDown => move_down,
    MoveWordLeft => move_word_left,
    MoveWordRight => move_word_right,
    MoveLineStart => move_line_start,
    MoveLineEnd => move_line_end,
    MoveDocumentStart => move_document_start,
    MoveDocumentEnd => move_document_end,
    PageUp => page_up,
    PageDown => page_down,
    SelectLeft => select_left,
    SelectRight => select_right,
    SelectUp => select_up,
    SelectDown => select_down,
    SelectWordLeft => select_word_left,
    SelectWordRight => select_word_right,
    SelectLineStart => select_line_start,
    SelectLineEnd => select_line_end,
    SelectDocumentStart => select_document_start,
    SelectDocumentEnd => select_document_end,
    SelectPageUp => select_page_up,
    SelectPageDown => select_page_down,
    SelectAll => select_all,
    SelectWord => select_word,
    SetSelection => set_selection,
    Type => type_text,
    Newline => newline,
    StartNewLine => start_new_line,
    Tab => tab,
    Unindent => unindent,
    Backspace => backspace,
    Delete => delete,
    DeleteWordLeft => delete_word_left,
    DeleteWordRight => delete_word_right,
    DeleteLine => delete_line,
    DuplicateLine => duplicate,
    Undo => undo,
    Redo => redo,
    Copy => copy,
    Cut => cut,
    Paste => paste,
    ShrinkSelection => shrink_selection,
    Escape => escape,
    DeleteToLineStart => delete_to_line_start,
    DeleteToLineEnd => delete_to_line_end,
    MoveLineUp => move_line_up,
    MoveLineDown => move_line_down,
    MoveStatementUp => move_statement_up,
    MoveStatementDown => move_statement_down,
    JoinLines => join_lines,
    ToggleComment => toggle_comment,
    GoToLine => go_to_line,
    ScrollToLine => scroll_to_line,
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let el = div()
            .key_context(if self.single_line {
                "Editor single_line"
            } else {
                "Editor"
            })
            .track_focus(&self.focus)
            .size_full()
            .child(EditorElement::new(cx.entity()));
        Self::bind_actions(el, cx)
    }
}

/// Opens an editor for `text` (convenience for tests and tools).
pub fn new_editor(text: &str, window: &mut Window, cx: &mut App) -> Entity<Editor> {
    cx.new(|cx| Editor::new(Buffer::new(text), window, cx))
}
