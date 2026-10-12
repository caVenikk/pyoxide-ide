//! The preview below the results: the selected row's file in an editor, as
//! in the reference IDE, with its syntax colours, the row's matches marked
//! and its line in the middle, shown [`PREVIEW_DELAY`](super::PREVIEW_DELAY)
//! after the selection settles.
//!
//! The preview edits the file's document. When the file is open in a tab,
//! the preview starts from the tab's text and every edit, in either view,
//! goes to the other one too, so both show the same text and the tab saves
//! it. A file that is not open gets a document of its own, saved like any
//! edit (after the idle time, when the window loses the focus, when the
//! preview shows another file and when the popup closes) without opening a
//! tab; opening it from the popup hands this document to the new tab.
//!
//! The rows keep the text they were found with, as in the reference, but
//! where their matches are moves with the edits: the marked matches, the
//! place a row opens at and the match a Replace changes follow the text.
//! Find Next and Find Previous in the preview are the editor's.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use editor::find::{Direction, FindBar};
use editor::{Editor, EditorEvent, SearchHighlights};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task, WeakEntity, Window};
use project_search::Row;
use serde_json::json;
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, Point};

use super::{FindInFiles, PREVIEW_DELAY};

/// A row as found: its line and the column where its first match starts.
type RowKey = (usize, usize);

/// Where a row's matches are now in its file's document.
#[derive(Debug, Clone)]
struct RowMarks {
    /// Char ranges; the first is the match the row opens at.
    ranges: Vec<Range<usize>>,
    /// Where the first one starts.
    first: Point,
}

pub(super) struct Preview {
    editor: Entity<Editor>,
    /// Find Next and Find Previous in the preview (the bar itself is not
    /// shown; its hint is).
    find: Entity<FindBar>,
    /// The file shown.
    path: Option<PathBuf>,
    /// The row shown.
    shown: Option<RowKey>,
    /// Where the rows' matches are now, for the files the preview showed.
    marks: HashMap<PathBuf, HashMap<RowKey, RowMarks>>,
    task: Option<Task<()>>,
    /// The editor of the file's tab: edits go both ways.
    linked: Option<(WeakEntity<Editor>, Subscription)>,
    /// The buffer versions an edit copied from the other view left in the
    /// preview and in the tab: their change events are not copied back.
    echo_preview: Option<u64>,
    echo_tab: Option<u64>,
    /// The idle save of the preview's own document.
    save_task: Option<Task<()>>,
    _subscriptions: [Subscription; 3],
}

impl Preview {
    pub fn new(window: &mut Window, cx: &mut Context<FindInFiles>) -> Self {
        let editor = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.buffer_mut().set_read_only(true);
            e
        });
        let find = cx.new(|cx| FindBar::new(editor.clone(), window, cx));
        let subscriptions = [
            cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
                if let EditorEvent::Edited(applied) = event {
                    this.preview_edited(applied, cx);
                }
            }),
            // The bar's hint ("not found, press ...") is drawn by the popup;
            // the next key hides it (also one an action takes).
            cx.observe(&find, |_, _, cx| cx.notify()),
            cx.intercept_keystrokes({
                let find = find.downgrade();
                move |_, _, cx| {
                    if let Some(bar) = find.upgrade() {
                        bar.update(cx, |bar, cx| bar.clear_hint(cx));
                    }
                }
            }),
        ];
        Self {
            editor,
            find,
            path: None,
            shown: None,
            marks: HashMap::new(),
            task: None,
            linked: None,
            echo_preview: None,
            echo_tab: None,
            save_task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// Find Next and Find Previous in the preview.
    pub fn find_bar(&self) -> &Entity<FindBar> {
        &self.find
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn state_json(&self, popup: &FindInFiles, cx: &App) -> serde_json::Value {
        let Some(path) = &self.path else {
            return serde_json::Value::Null;
        };
        let e = self.editor.read(cx);
        json!({
            "path": popup.rel(path),
            "caret": [e.caret_point().line, e.caret_point().column],
            "highlights": e.search_highlights().map(|h| h.matches.len()).unwrap_or(0),
            "read_only": e.buffer().is_read_only(),
            "linked": self.linked.is_some(),
            "modified": self.linked.is_none() && e.is_modified(),
            "hint": self.find.read(cx).hint().map(|h| h.to_string()),
        })
    }
}

impl FindInFiles {
    /// Shows the selected row in the preview once the selection settles.
    pub(super) fn preview_selected(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.preview.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DELAY).await;
            let _ = this.update(cx, |this, cx| this.show_preview(cx));
        }));
    }

    /// The preview's editor when it edits `path`'s own document (the file
    /// is not open in a tab).
    pub(super) fn preview_own_editor(&self, path: &Path) -> Option<Entity<Editor>> {
        (self.preview.path.as_deref() == Some(path) && self.preview.linked.is_none())
            .then(|| self.preview.editor.clone())
    }

    /// Where a row's first match is now: moved by the edits made while the
    /// preview showed its file, else where it was found.
    pub(super) fn row_point(&self, row: &Row) -> Point {
        self.preview
            .marks
            .get(row.path.as_ref())
            .and_then(|m| m.get(&row_key(row)))
            .map_or(row.line.first.start, |m| m.first)
    }

    /// The rows changed (a new search): their matches are where they were
    /// found.
    pub(super) fn forget_marks(&mut self) {
        self.preview.marks.clear();
    }

    /// The rows of the shown file that have no marks yet get them, from
    /// where they were found.
    pub(super) fn sync_marks(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preview.path.clone() else {
            return;
        };
        let b = self.preview.editor.read(cx).buffer();
        let marks = self.preview.marks.entry(path.clone()).or_default();
        for row in self.results.rows().iter().filter(|r| *r.path == path) {
            marks.entry(row_key(row)).or_insert_with(|| {
                let ranges = found_ranges(row, b);
                let first = ranges
                    .first()
                    .map_or(row.line.first.start, |r| b.offset_to_point(r.start));
                RowMarks { ranges, first }
            });
        }
    }

    /// The edit moved the matches of the shown file's rows; the marked ones
    /// are shown where they are now.
    fn move_marks(&mut self, applied: &[AppliedEdit], cx: &mut Context<Self>) {
        let Some(path) = self.preview.path.clone() else {
            return;
        };
        let b = self.preview.editor.read(cx).buffer();
        let len = b.len_chars();
        let Some(marks) = self.preview.marks.get_mut(&path) else {
            return;
        };
        for m in marks.values_mut() {
            for r in &mut m.ranges {
                let moved = map_range(r.clone(), applied);
                *r = moved.start.min(len)..moved.end.min(len);
            }
            if let Some(r) = m.ranges.first() {
                m.first = b.offset_to_point(r.start);
            }
        }
        let shown = self
            .preview
            .shown
            .and_then(|key| marks.get(&key))
            .map(|m| m.ranges.clone());
        if let Some(ranges) = shown {
            self.show_marks(&ranges, false, cx);
        }
    }

    /// Find Next or Find Previous in the preview: the next match of the
    /// last search in the file, as in any editor; nothing when there was
    /// no search in a file yet (the preview has no find bar to open).
    pub(super) fn find_in_preview(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let last = editor::find::last_search(cx);
        if !last.performed || last.text.is_empty() || self.preview.path.is_none() {
            return;
        }
        self.preview
            .find
            .update(cx, |bar, cx| bar.find(direction, window, cx));
    }

    /// The editor of the tab of `path`, when the file is open.
    fn tab_editor(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        let host = self.host.as_ref()?.upgrade()?;
        host.read(cx).editor_for(path)
    }

    /// Nothing shown (the popup closed, or there are no rows); the
    /// preview's own document is saved first.
    pub(super) fn clear_preview(&mut self, cx: &mut Context<Self>) {
        self.save_preview(cx);
        let p = &mut self.preview;
        p.task = None;
        p.shown = None;
        p.linked = None;
        if p.path.take().is_some() {
            p.editor.update(cx, |e, cx| {
                *e.buffer_mut() = Buffer::new("");
                e.buffer_mut().set_read_only(true);
                e.set_path(None);
                e.buffer_reloaded(cx);
                e.set_search_highlights(None, cx);
            });
        }
    }

    /// Loads the selected row's file into the preview (the tab's text when
    /// the file is open, else the file read in the background) and marks
    /// the row.
    fn show_preview(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self
            .selected
            .filter(|_| !self.stale)
            .and_then(|i| self.results.row(i))
            .cloned()
        else {
            self.clear_preview(cx);
            cx.notify();
            return;
        };
        let path = (*row.path).clone();
        if self.preview.path.as_ref() == Some(&path) {
            self.mark_preview(&row, cx);
            return;
        }
        // Another file: the one shown so far is saved.
        self.save_preview(cx);
        self.preview.linked = None;
        if let Some(tab) = self.tab_editor(&path, cx) {
            let buffer = tab.read(cx).buffer().clone();
            self.load_preview(path.clone(), buffer, &row, cx);
            self.link_preview(&tab, cx);
            return;
        }
        let read = cx.background_spawn({
            let path = path.clone();
            async move {
                project::read_document(&path).map(|loaded| {
                    let read_only = loaded.read_only || loaded.decoded.lossy;
                    let mut buffer = Buffer::from_decoded(loaded.decoded);
                    buffer.set_read_only(read_only);
                    buffer
                })
            }
        });
        self.preview.task = Some(cx.spawn(async move |this, cx| {
            let Ok(buffer) = read.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Still the selected row?
                let current = this.selected.and_then(|i| this.results.row(i));
                if current.is_some_and(|r| *r.path == path) {
                    this.load_preview(path, buffer, &row, cx);
                }
            });
        }));
    }

    fn load_preview(
        &mut self,
        path: PathBuf,
        buffer: Buffer,
        row: &project_search::Row,
        cx: &mut Context<Self>,
    ) {
        let file_type = self.file_types.for_path(&path).cloned();
        self.preview.editor.update(cx, |e, cx| {
            *e.buffer_mut() = buffer;
            e.buffer_reloaded(cx);
            e.set_path(Some(path.clone()));
            match &file_type {
                Some(t) => {
                    e.set_config(t.editor_config());
                    e.set_highlighter(t.highlighter.as_ref().map(|f| f()), cx);
                }
                None => {
                    e.set_config(editor::EditorConfig::default());
                    e.set_highlighter(None, cx);
                }
            }
        });
        self.preview.path = Some(path);
        self.preview.shown = None;
        self.sync_marks(cx);
        self.mark_preview(row, cx);
    }

    /// Edits of the tab's editor go to the preview from now on (and the
    /// preview's to the tab).
    fn link_preview(&mut self, tab: &Entity<Editor>, cx: &mut Context<Self>) {
        let subscription = cx.subscribe(tab, |this, tab, event: &EditorEvent, cx| {
            let EditorEvent::Edited(applied) = event else {
                return;
            };
            let version = tab.read(cx).buffer().version();
            if this.preview.echo_tab.take() == Some(version) {
                return;
            }
            let preview = this.preview.editor.clone();
            mirror(applied, &tab, &preview, cx);
            this.preview.echo_preview = Some(preview.read(cx).buffer().version());
        });
        self.preview.linked = Some((tab.downgrade(), subscription));
    }

    /// The preview's text changed: the edit goes to the tab, or the
    /// preview's own document is saved after the idle time.
    fn preview_edited(&mut self, applied: &[AppliedEdit], cx: &mut Context<Self>) {
        if self.preview.path.is_none() {
            return;
        }
        // Also for an edit copied from the tab.
        self.move_marks(applied, cx);
        let version = self.preview.editor.read(cx).buffer().version();
        if self.preview.echo_preview.take() == Some(version) {
            return;
        }
        let path = self.preview.path.clone().unwrap_or_default();
        // Still the tab's editor? (The tab may have been closed.)
        let linked = self
            .preview
            .linked
            .as_ref()
            .and_then(|(tab, _)| tab.upgrade())
            .filter(|tab| self.tab_editor(&path, cx).as_ref() == Some(tab));
        match linked {
            Some(tab) => {
                let preview = self.preview.editor.clone();
                mirror(applied, &preview, &tab, cx);
                self.preview.echo_tab = Some(tab.read(cx).buffer().version());
            }
            None => {
                self.preview.linked = None;
                self.schedule_preview_save(cx);
            }
        }
    }

    /// Saves the preview's own document once there was no input for the
    /// idle time (the workbench's setting).
    fn schedule_preview_save(&mut self, cx: &mut Context<Self>) {
        let settings = self
            .host
            .as_ref()
            .and_then(|h| h.upgrade())
            .map(|h| h.read(cx).save_settings())
            .unwrap_or_default();
        if !settings.save_on_idle {
            return;
        }
        let idle = settings.idle();
        self.preview.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(idle).await;
            let _ = this.update(cx, |this, cx| this.save_preview(cx));
        }));
    }

    /// Writes the preview's own document when it has unsaved edits (a
    /// file open in a tab is saved by its tab).
    pub fn save_preview(&mut self, cx: &mut Context<Self>) {
        self.preview.save_task = None;
        let Some(path) = self.preview.path.clone() else {
            return;
        };
        if self.preview.linked.is_some() || !self.preview.editor.read(cx).is_modified() {
            return;
        }
        let editor = self.preview.editor.clone();
        let (bytes, point) = editor.update(cx, |e, _| {
            let point = e.buffer_mut().begin_save();
            (e.buffer().encoded(), point)
        });
        let write = cx.background_spawn({
            let path = path.clone();
            async move { project::write_document(&path, &bytes) }
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(()) => editor.update(cx, |e, cx| {
                    e.buffer_mut().mark_saved_at(point);
                    cx.notify();
                }),
                Err(e) => cx.emit(super::FindInFilesEvent::Message(format!(
                    "Cannot save {}: {e}",
                    path.display()
                ))),
            });
        })
        .detach();
    }

    /// The preview's own document of `path`, for a tab that opens the file
    /// (the tab continues with its text and history).
    pub fn preview_document(&self, path: &Path, cx: &App) -> Option<Buffer> {
        (self.preview.path.as_deref() == Some(path) && self.preview.linked.is_none())
            .then(|| self.preview.editor.read(cx).buffer().clone())
    }

    /// [`Self::preview_document`], and the preview lets go of it (it is
    /// not saved by the preview any more).
    pub(super) fn take_preview_document(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Option<Buffer> {
        let document = self.preview_document(path, cx)?;
        let p = &mut self.preview;
        p.save_task = None;
        p.path = None;
        p.shown = None;
        p.editor.update(cx, |e, cx| {
            *e.buffer_mut() = Buffer::new("");
            e.buffer_mut().set_read_only(true);
            e.set_path(None);
            e.buffer_reloaded(cx);
            e.set_search_highlights(None, cx);
        });
        Some(document)
    }

    /// A tab opened `path`: when the preview shows it, the two views are
    /// linked (they hold the same text).
    pub fn tab_opened(&mut self, path: &Path, tab: &Entity<Editor>, cx: &mut Context<Self>) {
        if self.preview.path.as_deref() == Some(path) && self.preview.linked.is_none() {
            self.preview.save_task = None;
            self.link_preview(tab, cx);
        }
    }

    /// Marks the row's matches (outlined in replace mode) and puts the
    /// caret on the first one, its line in the middle.
    fn mark_preview(&mut self, row: &Row, cx: &mut Context<Self>) {
        let key = row_key(row);
        let fresh = self.preview.shown != Some(key);
        self.preview.shown = Some(key);
        let marked = self
            .preview
            .marks
            .get(row.path.as_ref())
            .and_then(|m| m.get(&key));
        let ranges = match marked {
            Some(m) => m.ranges.clone(),
            None => found_ranges(row, self.preview.editor.read(cx).buffer()),
        };
        self.show_marks(&ranges, fresh, cx);
    }

    /// Marks matches in the preview; with `go`, the caret goes on the first
    /// one, its line in the middle.
    fn show_marks(&mut self, ranges: &[Range<usize>], go: bool, cx: &mut Context<Self>) {
        let replace = self.replace_mode;
        self.preview.editor.update(cx, |e, cx| {
            let first = ranges.first().map(|r| e.buffer().offset_to_point(r.start));
            let mut matches: Vec<_> = ranges.iter().filter(|r| r.end > r.start).cloned().collect();
            matches.sort_by_key(|r| r.start);
            let current = (replace && !matches.is_empty()).then_some(0);
            e.set_search_highlights(
                Some(SearchHighlights {
                    matches,
                    current,
                    current_background: false,
                    excluded: Default::default(),
                    lines: first.map(|p| vec![p.line]).unwrap_or_default(),
                }),
                cx,
            );
            if go && let Some(p) = first {
                e.go_to(p.line + 1, Some(p.column + 1), cx);
            }
        });
        cx.notify();
    }
}

fn row_key(row: &Row) -> RowKey {
    (
        row.line.line,
        row.line.ranges.first().map_or(0, |m| m.start),
    )
}

/// A row's matches as char ranges of `b` where they were found: the first
/// one (which may go on to later lines) from its points, the others from
/// their columns.
fn found_ranges(row: &Row, b: &Buffer) -> Vec<Range<usize>> {
    let first = &row.line.first;
    let line = row.line.line.min(b.len_lines().saturating_sub(1));
    let start = b.line_start(line);
    let len = b.line_len(line);
    row.line
        .ranges
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if i == 0 {
                b.point_to_offset(first.start)..b.point_to_offset(first.end)
            } else {
                start + r.start.min(len)..start + r.end.min(len)
            }
        })
        .collect()
}

/// Moves a char range through edits as applied (each in the text the
/// ones before it left): an edit before the range shifts it, one inside it
/// grows or shrinks it; text typed at its start or end stays outside it.
fn map_range(range: Range<usize>, applied: &[AppliedEdit]) -> Range<usize> {
    let mut r = range;
    for e in applied {
        let shift = |p: usize| (p as isize + e.new_end as isize - e.old_end as isize) as usize;
        let start = if r.start < e.start {
            r.start
        } else if r.start >= e.old_end {
            shift(r.start)
        } else {
            e.start
        };
        let end = if r.end <= e.start {
            r.end
        } else if r.end >= e.old_end {
            shift(r.end)
        } else {
            e.new_end
        };
        r = start..end.max(start);
    }
    r
}

/// Copies an edit of `from` (as applied) to `to`, which held the same text
/// before it; `to` keeps its carets where its text kept them.
fn mirror(applied: &[AppliedEdit], from: &Entity<Editor>, to: &Entity<Editor>, cx: &mut App) {
    let edits = replayed(applied, from.read(cx).buffer());
    let delta: isize = edits
        .iter()
        .map(|e| e.text.chars().count() as isize - e.range.len() as isize)
        .sum();
    let before = (from.read(cx).buffer().len_chars() as isize - delta) as usize;
    if to.read(cx).buffer().len_chars() != before {
        // The views went apart (not expected): the target takes the text.
        let text = from.read(cx).text();
        to.update(cx, |e, cx| {
            let len = e.buffer().len_chars();
            e.edit(
                vec![Edit::new(0..len, text)],
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
                cx,
            )
        });
        return;
    }
    let typing = edits.len() == 1 && edits[0].range.is_empty() && !edits[0].text.contains('\n');
    let kind = if typing {
        EditKind::Typing
    } else {
        EditKind::Other
    };
    let ranges: Vec<(Range<usize>, usize)> = edits
        .iter()
        .map(|e| (e.range.clone(), e.text.chars().count()))
        .collect();
    to.update(cx, |e, cx| {
        let mut carets = e.selections().clone();
        carets.update(|s| {
            s.anchor = map_offset(s.anchor, &ranges);
            s.head = map_offset(s.head, &ranges);
        });
        e.edit(edits, kind, CaretPlacement::Explicit(carets), cx)
    });
}

/// The edits that turn the text before an edit into the text after it, in
/// the coordinates before it, from the edit as applied (descending
/// positions, each applied in turn) and the text after it.
fn replayed(applied: &[AppliedEdit], after: &Buffer) -> Vec<Edit> {
    let mut shift: isize = 0;
    let mut edits = Vec::with_capacity(applied.len());
    for a in applied.iter().rev() {
        let start = (a.start as isize + shift) as usize;
        let end = (a.new_end as isize + shift) as usize;
        edits.push(Edit::new(a.start..a.old_end, after.slice(start..end)));
        shift += a.new_end as isize - a.old_end as isize;
    }
    edits.sort_by_key(|e| e.range.start);
    edits
}

/// Where a position of the text before edits (ranges and new lengths,
/// ascending) is after them; a position inside a replaced range goes to
/// the end of its replacement.
fn map_offset(pos: usize, edits: &[(Range<usize>, usize)]) -> usize {
    let mut delta: isize = 0;
    for (range, new_len) in edits {
        if pos < range.start {
            break;
        }
        if range.is_empty() || pos >= range.end {
            // Before the position (an insertion at it goes before it too).
            delta += *new_len as isize - range.len() as isize;
        } else {
            return (range.start as isize + delta) as usize + new_len;
        }
    }
    (pos as isize + delta) as usize
}

#[cfg(test)]
mod tests {
    use text::SelectionSet;

    use super::*;

    #[test]
    fn edits_are_replayed_in_the_text_before_them() {
        let mut a = Buffer::new("one two three four");
        let mut sel = SelectionSet::caret(0);
        let applied = a.transact(
            &mut sel,
            vec![Edit::new(0..3, "1"), Edit::new(8..13, "THREE!")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        let edits = replayed(&applied, &a);
        let mut b = Buffer::new("one two three four");
        let mut sel = SelectionSet::caret(0);
        b.transact(
            &mut sel,
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), a.text());
        assert_eq!(b.text(), "1 two THREE! four");
        // An undo is replayed the same way.
        let (undone, _) = a.undo().unwrap();
        let edits = replayed(&undone, &a);
        b.transact(
            &mut sel,
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "one two three four");
    }

    #[test]
    fn marked_matches_follow_the_edits() {
        let edit = |text: &str, sel: Range<usize>, new: &str| {
            let mut b = Buffer::new(text);
            let mut s = SelectionSet::caret(0);
            let applied = b.transact(
                &mut s,
                vec![Edit::new(sel, new)],
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
            );
            (applied, b)
        };
        // "a needle b": the match is 2..8.
        let (applied, b) = edit("a needle b", 0..0, "XY");
        assert_eq!(
            b.slice(map_range(2..8, &applied)),
            "needle",
            "before: moved"
        );
        let (applied, b) = edit("a needle b", 2..2, "ZZ");
        assert_eq!(
            b.slice(map_range(2..8, &applied)),
            "needle",
            "typed at the start"
        );
        let (applied, b) = edit("a needle b", 8..8, "s");
        assert_eq!(
            b.slice(map_range(2..8, &applied)),
            "needle",
            "typed at the end"
        );
        let (applied, b) = edit("a needle b", 4..5, "EE");
        assert_eq!(
            b.slice(map_range(2..8, &applied)),
            "neEEdle",
            "inside: grows"
        );
        let (applied, b) = edit("a needle b", 9..10, "c");
        assert_eq!(b.slice(map_range(2..8, &applied)), "needle", "after: stays");
        let (applied, b) = edit("a needle b", 0..4, "");
        assert_eq!(
            b.slice(map_range(2..8, &applied)),
            "edle",
            "its start deleted"
        );
    }

    #[test]
    fn carets_follow_the_edits() {
        let edits = [(2..4, 5), (10..10, 3)];
        assert_eq!(map_offset(1, &edits), 1);
        assert_eq!(map_offset(3, &edits), 7, "inside: the end of the new text");
        assert_eq!(map_offset(6, &edits), 9);
        assert_eq!(map_offset(10, &edits), 16, "at an insertion: after it");
        assert_eq!(map_offset(12, &edits), 18);
    }
}
