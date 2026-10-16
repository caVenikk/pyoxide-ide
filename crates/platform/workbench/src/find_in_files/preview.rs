//! Find in Files' preview: the shared file preview
//! ([`crate::file_preview`], editable, with the Soft-Wrap button) showing
//! the selected row's file with the row's matches marked and its line in
//! the middle, [`PREVIEW_DELAY`](super::PREVIEW_DELAY) after the selection
//! settles. The preview edits the file's document (the shared preview
//! sends the edits to the file's tab or saves them).
//!
//! The rows keep the text they were found with, as in the reference, but
//! where their matches are moves with the edits: the marked matches, the
//! place a row opens at and the match a Replace changes follow the text.
//! Find Next and Find Previous in the preview are the editor's.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use editor::Editor;
use editor::find::{Direction, FindBar};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task, WeakEntity, Window};
use project_search::Row;
use text::{AppliedEdit, Buffer, Point};

use super::{FindInFiles, FindInFilesEvent, PREVIEW_DELAY};
use crate::file_preview::{FilePreview, FilePreviewEvent, NO_PREVIEW, PreviewOptions};
use crate::file_types::FileTypes;
use crate::workbench::Workbench;

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
    view: Entity<FilePreview>,
    editor: Entity<Editor>,
    /// Find Next and Find Previous in the preview (the bar itself is not
    /// shown; its hint is).
    find: Entity<FindBar>,
    /// The row shown.
    shown: Option<RowKey>,
    /// Where the rows' matches are now, for the files the preview showed.
    marks: HashMap<PathBuf, HashMap<RowKey, RowMarks>>,
    /// The wait for the selection to settle.
    task: Option<Task<()>>,
    _subscriptions: [Subscription; 3],
}

impl Preview {
    pub fn new(
        file_types: Rc<FileTypes>,
        window: &mut Window,
        cx: &mut Context<FindInFiles>,
    ) -> Self {
        let options = PreviewOptions {
            editable: true,
            soft_wrap_button: true,
            empty_text: NO_PREVIEW.into(),
            name: "find-in-files-preview",
        };
        let view = cx.new(|cx| FilePreview::new(options, file_types, window, cx));
        let editor = view.read(cx).editor().clone();
        let find = cx.new(|cx| FindBar::new(editor.clone(), window, cx));
        let subscriptions = [
            cx.subscribe(&view, |this, _, event: &FilePreviewEvent, cx| match event {
                FilePreviewEvent::Loaded(path) => this.preview_loaded(path, cx),
                FilePreviewEvent::Edited(applied) => this.move_marks(applied, cx),
                FilePreviewEvent::Message(text) => cx.emit(FindInFilesEvent::Message(text.clone())),
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
            view,
            editor,
            find,
            shown: None,
            marks: HashMap::new(),
            task: None,
            _subscriptions: subscriptions,
        }
    }

    /// The shared preview (drawn by the popup).
    pub fn view(&self) -> &Entity<FilePreview> {
        &self.view
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// Find Next and Find Previous in the preview.
    pub fn find_bar(&self) -> &Entity<FindBar> {
        &self.find
    }

    /// The file shown.
    pub fn path<'a>(&self, cx: &'a App) -> Option<&'a PathBuf> {
        self.view.read(cx).path()
    }

    pub fn set_host(&self, host: WeakEntity<Workbench>, cx: &mut App) {
        self.view.update(cx, |p, _| p.set_host(host));
    }

    pub fn state_json(&self, popup: &FindInFiles, cx: &App) -> serde_json::Value {
        let mut state = self.view.read(cx).state_json(|p| popup.rel(p), cx);
        if let Some(map) = state.as_object_mut() {
            let hint = self.find.read(cx).hint().map(|h| h.to_string());
            map.insert("hint".into(), serde_json::json!(hint));
        }
        state
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
    pub(super) fn preview_own_editor(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        self.preview.view.read(cx).own_editor(path)
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
        let Some(path) = self.preview.path(cx).cloned() else {
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
        let Some(path) = self.preview.path(cx).cloned() else {
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
        if !last.performed || last.text.is_empty() || self.preview.path(cx).is_none() {
            return;
        }
        self.preview
            .find
            .update(cx, |bar, cx| bar.find(direction, window, cx));
    }

    /// Nothing shown (the popup closed, or there are no rows); the
    /// preview's own document is saved first.
    pub(super) fn clear_preview(&mut self, cx: &mut Context<Self>) {
        self.preview.task = None;
        self.preview.shown = None;
        self.preview.view.update(cx, |p, cx| p.clear(cx));
    }

    /// Shows the selected row's file in the preview (the tab's text when
    /// the file is open, else the file read in the background) and marks
    /// the row once it is there.
    fn show_preview(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            self.clear_preview(cx);
            cx.notify();
            return;
        };
        let path = (*row.path).clone();
        let shown = self.preview.view.update(cx, |p, cx| p.show_file(path, cx));
        if shown {
            self.mark_preview(&row, cx);
        }
    }

    fn selected_row(&self) -> Option<Row> {
        self.selected
            .filter(|_| !self.stale)
            .and_then(|i| self.results.row(i))
            .cloned()
    }

    /// The preview shows `path` now: its rows get their marks and the
    /// selected row is marked when it is still in that file.
    fn preview_loaded(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.preview.shown = None;
        self.sync_marks(cx);
        if let Some(row) = self.selected_row().filter(|r| *r.path == path) {
            self.mark_preview(&row, cx);
        }
    }

    /// Writes the preview's own document when it has unsaved edits (a
    /// file open in a tab is saved by its tab).
    pub fn save_preview(&mut self, cx: &mut Context<Self>) {
        self.preview.view.update(cx, |p, cx| p.save(cx));
    }

    /// The preview's own document of `path`, for a tab that opens the file
    /// (the tab continues with its text and history).
    pub fn preview_document(&self, path: &Path, cx: &App) -> Option<Buffer> {
        self.preview.view.read(cx).document(path, cx)
    }

    /// [`Self::preview_document`], and the preview lets go of it (it is
    /// not saved by the preview any more).
    pub(super) fn take_preview_document(
        &mut self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Option<Buffer> {
        let document = self
            .preview
            .view
            .update(cx, |p, cx| p.take_document(path, cx))?;
        self.preview.shown = None;
        Some(document)
    }

    /// A tab opened `path`: when the preview shows it, the two views are
    /// linked (they hold the same text).
    pub fn tab_opened(&mut self, path: &Path, tab: &Entity<Editor>, cx: &mut Context<Self>) {
        self.preview
            .view
            .update(cx, |p, cx| p.tab_opened(path, tab, cx));
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
        let first = ranges.first().map(|r| {
            self.preview
                .editor
                .read(cx)
                .buffer()
                .offset_to_point(r.start)
        });
        self.preview.view.update(cx, |p, cx| {
            p.mark(
                ranges,
                replace.then_some(0),
                first.map(|p| p.line),
                first.filter(|_| go),
                cx,
            )
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

#[cfg(test)]
mod tests {
    use text::{CaretPlacement, Edit, EditKind, SelectionSet};

    use super::*;

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
}
