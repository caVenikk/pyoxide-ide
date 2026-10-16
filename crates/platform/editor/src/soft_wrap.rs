//! Soft wraps of an editor (the reference's soft wrap model): lines wider
//! than the view go on in the next visual rows. Carets, selections,
//! scrolling and painting work on visual rows; without soft wraps a row
//! is a line.
//!
//! The rows of every line are kept in a [`WrapMap`]. An edit updates the
//! lines it touches on the UI thread; a new width or a new text computes
//! all of them, at once for small texts and on the background executor for
//! large ones, where the edits made meanwhile are applied to the result
//! when it arrives. Until then the map and the wraps of the lines stay
//! those of the previous width, so that rows and lines never disagree.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{AppContext as _, Context, Pixels, Task, Window, px};
use text::AppliedEdit;

use crate::commands::ToggleSoftWraps;
use crate::editor::Editor;
use crate::wrap::{self, LineEdit, Segment, Wrap, WrapMap, WrapParams, WrapRule};

/// Texts up to this many bytes are wrapped at once on the UI thread.
const SYNC_BYTES: usize = 256 * 1024;
/// More line edits at once than this compute all rows again.
const MAX_LINE_EDITS: usize = 64;
/// Columns of the signs drawn at a wrap (the reference's arrows, one char
/// of the editor font each).
const SIGN_COLUMNS: usize = 1;
/// Lines whose wraps are kept for the current text.
const CACHED_LINES: usize = 512;
/// Lines at least this long keep the wraps an edit computed for them.
const LONG_LINE_CHARS: usize = 1000;

thread_local! {
    static NO_WRAPS: Rc<[Wrap]> = Rc::from(Vec::new());
}

/// Soft wrap state of an editor that has soft wraps on.
pub(crate) struct SoftWrap {
    /// Rows of every line for `params`.
    map: WrapMap,
    /// What the map was computed for; `None` until the first frame tells
    /// the width (every line is one row until then).
    params: Option<WrapParams>,
    /// All rows computed in the background for new parameters.
    job: Option<WrapJob>,
    /// Wraps of lines looked at in this version of the text (a frame
    /// asks for the same lines many times; a long line is wrapped once).
    cache: RefCell<WrapsCache>,
}

#[derive(Default)]
struct WrapsCache {
    /// The text's version and length the wraps are for (a buffer replaced
    /// by its owner starts its versions again).
    text: (u64, usize),
    lines: HashMap<usize, Rc<[Wrap]>>,
}

impl WrapsCache {
    fn for_text(&mut self, buffer: &text::Buffer) -> &mut HashMap<usize, Rc<[Wrap]>> {
        let text = (buffer.version(), buffer.len_chars());
        if self.text != text || self.lines.len() >= CACHED_LINES {
            self.text = text;
            self.lines.clear();
        }
        &mut self.lines
    }
}

impl SoftWrap {
    fn new(lines: usize) -> Self {
        Self {
            map: WrapMap::uniform(lines),
            params: None,
            job: None,
            cache: RefCell::default(),
        }
    }
}

struct WrapJob {
    params: WrapParams,
    cancel: Arc<AtomicBool>,
    /// Line edits made since the job's snapshot of the text.
    edits: Vec<LineEdit>,
    _task: Task<()>,
}

impl Drop for WrapJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The lines an applied edit replaced.
fn line_edit(e: &AppliedEdit) -> LineEdit {
    LineEdit {
        start: e.start_position.0,
        old_end: e.old_end_position.0,
        new_end: e.new_end_position.0,
    }
}

impl Editor {
    /// Turns soft wraps on or off (the reference's Soft-Wrap of the active
    /// editor). The line at the top of the view stays there.
    pub fn set_soft_wraps(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.soft_wrap.is_some() {
            return;
        }
        let lh = self.row_height();
        let anchor = self.top_anchor(lh);
        self.soft_wrap = on.then(|| SoftWrap::new(self.buffer.len_lines()));
        self.lean_back.clear();
        if on {
            self.scroll.x = px(0.0);
            // The width of the last frame, if there was one.
            if let Some(params) = self.last_wrap_params.clone() {
                self.rewrap(params, cx);
            }
        }
        self.restore_top(anchor, lh);
        self.autoscroll = false;
        cx.notify();
    }

    /// Whether soft wraps are on.
    pub fn soft_wraps(&self) -> bool {
        self.soft_wrap.is_some()
    }

    pub(crate) fn toggle_soft_wraps(
        &mut self,
        _: &ToggleSoftWraps,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on = !self.soft_wraps();
        self.set_soft_wraps(on, cx);
    }

    /// The rules a line wraps by when no space is near (the file type's;
    /// any text's by default).
    pub(crate) fn wrap_rules(&self) -> Arc<[WrapRule]> {
        self.wrap_rules.clone()
    }

    /// The view is `width` columns wide in this frame: computes the rows
    /// again when that changes what they were computed for.
    pub(crate) fn update_wrap_width(&mut self, width: usize, cx: &mut Context<Self>) {
        let params = WrapParams {
            width: width.max(1),
            before_sign: SIGN_COLUMNS,
            after_sign: SIGN_COLUMNS,
            tab_size: self.config().tab_size.max(1),
            relative_indent: Some(0),
            rules: self.wrap_rules(),
        };
        if self.last_wrap_params.as_ref() != Some(&params) {
            self.last_wrap_params = Some(params.clone());
        }
        let Some(sw) = &self.soft_wrap else { return };
        let target = sw.job.as_ref().map(|j| &j.params).or(sw.params.as_ref());
        if target != Some(&params) {
            self.rewrap(params, cx);
        }
    }

    /// Computes the rows of every line for `params`: at once for a small
    /// text, else in the background (the rows stay those of the old
    /// parameters until the result arrives).
    fn rewrap(&mut self, params: WrapParams, cx: &mut Context<Self>) {
        let lines = self.buffer.len_lines();
        let Some(sw) = &mut self.soft_wrap else {
            return;
        };
        if self.buffer.len_bytes() <= SYNC_BYTES {
            sw.job = None;
            let rows = wrap::compute_rows(self.buffer.rope(), &params, &AtomicBool::new(false))
                .unwrap_or_else(|| vec![1; lines]);
            self.replace_wraps(WrapMap::new(rows), params);
            return;
        }
        let rope = self.buffer.rope().clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let work = {
            let (params, cancel) = (params.clone(), cancel.clone());
            cx.background_spawn(async move { wrap::compute_rows(&rope, &params, &cancel) })
        };
        let task = cx.spawn(async move |this, cx| {
            let Some(rows) = work.await else { return };
            let _ = this.update(cx, |this, cx| this.wrap_job_done(rows, cx));
        });
        sw.job = Some(WrapJob {
            params,
            cancel,
            edits: Vec::new(),
            _task: task,
        });
    }

    /// The background job's rows arrived: the edits made meanwhile are
    /// applied to them, then they replace the map.
    fn wrap_job_done(&mut self, rows: Vec<u32>, cx: &mut Context<Self>) {
        let Some(sw) = &mut self.soft_wrap else {
            return;
        };
        let Some(job) = sw.job.take() else { return };
        let mut map = WrapMap::new(rows);
        let mut scratch = Vec::new();
        let buffer = &self.buffer;
        let params = job.params.clone();
        let applied = job.edits.len() <= MAX_LINE_EDITS
            && map.edit(&job.edits, |line| {
                wrap::line_rows(buffer.line(line), &params, &mut scratch)
            })
            && map.lines() == buffer.len_lines();
        if !applied {
            self.rewrap(job.params.clone(), cx);
            return;
        }
        self.replace_wraps(map, job.params.clone());
        cx.notify();
    }

    /// Replaces the rows; the line at the top of the view stays there.
    fn replace_wraps(&mut self, map: WrapMap, params: WrapParams) {
        let lh = self.row_height();
        let anchor = self.top_anchor(lh);
        if let Some(sw) = &mut self.soft_wrap {
            sw.map = map;
            sw.params = Some(params);
            sw.cache.borrow_mut().lines.clear();
        }
        self.lean_back.clear();
        self.restore_top(anchor, lh);
    }

    /// Updates the rows of the lines that edits touched (in the order they
    /// were applied).
    pub(crate) fn wrap_edited(&mut self, applied: &[AppliedEdit], cx: &mut Context<Self>) {
        self.lean_back.clear();
        let lines = self.buffer.len_lines();
        let Some(sw) = &mut self.soft_wrap else {
            return;
        };
        let edits: Vec<LineEdit> = applied.iter().map(line_edit).collect();
        let mut restart = None;
        if let Some(job) = &mut sw.job {
            job.edits.extend_from_slice(&edits);
            if job.edits.len() > MAX_LINE_EDITS {
                restart = Some(job.params.clone());
            }
        }
        let buffer = &self.buffer;
        let mut scratch = Vec::new();
        let in_step = match &sw.params {
            _ if edits.len() > MAX_LINE_EDITS => false,
            Some(p) => {
                let mut cache = sw.cache.borrow_mut();
                let cached = cache.for_text(buffer);
                sw.map.edit(&edits, |line| {
                    let text = buffer.line(line);
                    if text.len_chars() < LONG_LINE_CHARS {
                        return wrap::line_rows(text, p, &mut scratch);
                    }
                    // The frame will want these wraps again.
                    let wraps: Rc<[Wrap]> = wrap::line_wraps(text, p).into();
                    let rows = wraps.len() as u32 + 1;
                    cached.insert(line, wraps);
                    rows
                })
            }
            None => sw.map.edit(&edits, |_| 1),
        } && sw.map.lines() == lines;
        if !in_step {
            sw.map.resize(lines);
            // The parameters a job in flight computes for, else the map's.
            let params = restart
                .take()
                .or_else(|| sw.job.as_ref().map(|j| j.params.clone()))
                .or_else(|| sw.params.clone());
            if let Some(params) = params {
                self.rewrap(params, cx);
            }
        } else if let Some(params) = restart {
            self.rewrap(params, cx);
        }
    }

    /// The whole text was replaced: all rows are computed again.
    pub(crate) fn wrap_reloaded(&mut self, cx: &mut Context<Self>) {
        self.lean_back.clear();
        let lines = self.buffer.len_lines();
        let Some(sw) = &mut self.soft_wrap else {
            return;
        };
        let params = sw
            .job
            .take()
            .map(|j| j.params.clone())
            .or(sw.params.clone());
        *sw = SoftWrap::new(lines);
        if let Some(params) = params {
            self.rewrap(params, cx);
        }
    }

    // ---- rows -----------------------------------------------------------

    /// Height of a row as of the last frame.
    pub(crate) fn row_height(&self) -> Pixels {
        self.layout.as_ref().map_or(px(20.0), |l| l.line_height)
    }

    /// The parameters the rows are computed for, when lines wrap.
    fn wrap_params(&self) -> Option<&WrapParams> {
        self.soft_wrap.as_ref()?.params.as_ref()
    }

    /// The width in columns lines wrap at, when they wrap.
    pub(crate) fn wrap_columns(&self) -> Option<usize> {
        self.wrap_params().map(|p| p.width)
    }

    /// Whether lines wrap (soft wraps on and the width known).
    pub(crate) fn wrapping(&self) -> bool {
        self.wrap_params().is_some()
    }

    /// Visual rows of the text.
    pub(crate) fn display_rows(&self) -> usize {
        match &self.soft_wrap {
            Some(sw) if sw.params.is_some() => sw.map.total(),
            _ => self.buffer.len_lines(),
        }
    }

    /// The first visual row of a line.
    pub(crate) fn line_row(&self, line: usize) -> usize {
        match &self.soft_wrap {
            Some(sw) if sw.params.is_some() => sw.map.row_of_line(line),
            _ => line,
        }
    }

    /// The first rows of lines asked for in increasing order (fast for
    /// many lines).
    pub(crate) fn line_row_cursor(&self) -> Box<dyn FnMut(usize) -> usize + '_> {
        match &self.soft_wrap {
            Some(sw) if sw.params.is_some() => Box::new(sw.map.cursor()),
            _ => Box::new(|line| line),
        }
    }

    /// Visual rows of a line.
    pub(crate) fn line_rows(&self, line: usize) -> usize {
        match &self.soft_wrap {
            Some(sw) if sw.params.is_some() => sw.map.rows(line),
            _ => 1,
        }
    }

    /// The line showing a visual row, and the row's index in it (clamped
    /// to the last row).
    pub(crate) fn row_line(&self, row: usize) -> (usize, usize) {
        match &self.soft_wrap {
            Some(sw) if sw.params.is_some() => sw.map.line_of_row(row),
            _ => (row.min(self.buffer.len_lines() - 1), 0),
        }
    }

    /// The soft wraps of a line (none without soft wraps).
    pub(crate) fn line_wraps(&self, line: usize) -> Rc<[Wrap]> {
        let Some(sw) = &self.soft_wrap else {
            return NO_WRAPS.with(Rc::clone);
        };
        let Some(p) = &sw.params else {
            return NO_WRAPS.with(Rc::clone);
        };
        let mut cache = sw.cache.borrow_mut();
        let lines = cache.for_text(&self.buffer);
        lines
            .entry(line)
            .or_insert_with(|| wrap::line_wraps(self.buffer.line(line), p).into())
            .clone()
    }

    /// The visual rows of a line.
    pub(crate) fn line_segments(&self, line: usize) -> Vec<Segment> {
        wrap::segments(&self.line_wraps(line), self.buffer.line_len(line))
    }

    /// The caret at `offset` is drawn at the end of the row before a wrap
    /// there rather than at the start of the next.
    pub(crate) fn leans_back(&self, offset: usize) -> bool {
        self.lean_back.contains(&offset)
    }

    /// Visual row and display column (the row's indent included) of a
    /// position; `lean` puts a position at a wrap at the end of the row
    /// before it.
    pub(crate) fn visual_point(&self, offset: usize, lean: bool) -> (usize, usize) {
        let p = self
            .buffer
            .offset_to_point(offset.min(self.buffer.len_chars()));
        let wraps = self.line_wraps(p.line);
        let segs = wrap::segments(&wraps, self.buffer.line_len(p.line));
        let sub = wrap::row_of_col(&wraps, p.column, lean);
        let col = wrap::visual_col(
            self.buffer.line(p.line),
            segs[sub],
            p.column,
            self.config().tab_size.max(1),
        );
        (self.line_row(p.line) + sub, col)
    }

    /// The position nearest to display column `col` of a visual row, and
    /// whether it is the end of a row before a wrap.
    pub(crate) fn offset_at_visual(&self, row: usize, col: usize) -> (usize, bool) {
        let (line, sub) = self.row_line(row);
        let segs = self.line_segments(line);
        let sub = sub.min(segs.len() - 1);
        let seg = segs[sub];
        let at = wrap::col_at_visual(
            self.buffer.line(line),
            seg,
            col,
            self.config().tab_size.max(1),
        );
        let lean = at == seg.end && sub + 1 < segs.len();
        (self.buffer.line_start(line) + at, lean)
    }

    /// The visual row of a position.
    pub(crate) fn caret_row(&self, offset: usize, lean: bool) -> usize {
        if !self.wrapping() {
            return self
                .buffer
                .char_to_line(offset.min(self.buffer.len_chars()));
        }
        let p = self
            .buffer
            .offset_to_point(offset.min(self.buffer.len_chars()));
        let wraps = self.line_wraps(p.line);
        self.line_row(p.line) + wrap::row_of_col(&wraps, p.column, lean)
    }

    /// The row of the primary caret.
    pub(crate) fn primary_row(&self) -> usize {
        let head = self.selections.primary().head;
        self.caret_row(head, self.leans_back(head))
    }

    /// Where Home goes from `head` on a wrapped line: from inside a
    /// continuation row to its start, from its start to the first non-blank
    /// char of the line (the reference's start of a soft-wrapped line);
    /// on a line's first row as without wraps.
    pub(crate) fn row_start_target(&self, head: usize) -> usize {
        let p = self.buffer.offset_to_point(head);
        let wraps = self.line_wraps(p.line);
        let sub = wrap::row_of_col(&wraps, p.column, self.leans_back(head));
        if sub > 0 {
            let start = self.buffer.line_start(p.line) + wraps[sub - 1].col;
            if head != start || self.leans_back(head) {
                return start;
            }
        }
        text::movement::smart_line_start(&self.buffer, head)
    }

    /// Where End goes from `head`: to the end of its row, or from the end
    /// of a row before a wrap to the end of the line; `true` when the
    /// target is the end of a row before a wrap.
    pub(crate) fn row_end_target(&self, head: usize) -> (usize, bool) {
        let p = self.buffer.offset_to_point(head);
        let wraps = self.line_wraps(p.line);
        let lean = self.leans_back(head);
        let sub = wrap::row_of_col(&wraps, p.column, lean);
        if let Some(w) = wraps.get(sub) {
            let end = self.buffer.line_start(p.line) + w.col;
            if !(head == end && lean) {
                return (end, true);
            }
        }
        (self.buffer.line_end(p.line), false)
    }

    /// The top of the view as a line, a row in it and the part of the row
    /// above the view.
    fn top_anchor(&self, lh: Pixels) -> (usize, usize, Pixels) {
        let row = (self.scroll.y / lh).floor().max(0.0) as usize;
        let (line, sub) = self.row_line(row);
        (line, sub, self.scroll.y - lh * row as f32)
    }

    fn restore_top(&mut self, (line, sub, within): (usize, usize, Pixels), lh: Pixels) {
        let line = line.min(self.buffer.len_lines() - 1);
        let sub = sub.min(self.line_rows(line) - 1);
        self.scroll.y = (lh * (self.line_row(line) + sub) as f32 + within).max(px(0.0));
    }
}
