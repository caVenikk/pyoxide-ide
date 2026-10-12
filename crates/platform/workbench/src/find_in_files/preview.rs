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
//! The marked matches move with the edits.

use std::ops::Range;
use std::path::{Path, PathBuf};

use editor::{Editor, EditorEvent, SearchHighlights};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task, WeakEntity, Window};
use serde_json::json;
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, Point};

use super::{FindInFiles, PREVIEW_DELAY};

pub(super) struct Preview {
    editor: Entity<Editor>,
    /// The file shown.
    path: Option<PathBuf>,
    /// The row shown: its line and where its first match starts.
    shown: Option<Point>,
    task: Option<Task<()>>,
    /// The editor of the file's tab: edits go both ways.
    linked: Option<(WeakEntity<Editor>, Subscription)>,
    /// The buffer versions an edit copied from the other view left in the
    /// preview and in the tab: their change events are not copied back.
    echo_preview: Option<u64>,
    echo_tab: Option<u64>,
    /// The idle save of the preview's own document.
    save_task: Option<Task<()>>,
    _subscription: Subscription,
}

impl Preview {
    pub fn new(window: &mut Window, cx: &mut Context<FindInFiles>) -> Self {
        let editor = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.buffer_mut().set_read_only(true);
            e
        });
        let subscription = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            if let EditorEvent::Edited(applied) = event {
                this.preview_edited(applied, cx);
            }
        });
        Self {
            editor,
            path: None,
            shown: None,
            task: None,
            linked: None,
            echo_preview: None,
            echo_tab: None,
            save_task: None,
            _subscription: subscription,
        }
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
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
    fn mark_preview(&mut self, row: &project_search::Row, cx: &mut Context<Self>) {
        let first = row.line.first.clone();
        let replace = self.replace_mode;
        let fresh = self.preview.shown != Some(first.start);
        self.preview.shown = Some(first.start);
        self.preview.editor.update(cx, |e, cx| {
            let b = e.buffer();
            let line = row.line.line.min(b.len_lines().saturating_sub(1));
            let start = b.line_start(line);
            let len = b.line_len(line);
            let mut matches: Vec<_> = row
                .line
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
                .filter(|r| r.end > r.start)
                .collect();
            matches.sort_by_key(|r| r.start);
            e.set_search_highlights(
                Some(SearchHighlights {
                    matches,
                    current: replace.then_some(0),
                    current_background: false,
                    excluded: Default::default(),
                    lines: vec![line],
                }),
                cx,
            );
            if fresh {
                e.go_to(first.start.line + 1, Some(first.start.column + 1), cx);
            }
        });
        cx.notify();
    }
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
    fn carets_follow_the_edits() {
        let edits = [(2..4, 5), (10..10, 3)];
        assert_eq!(map_offset(1, &edits), 1);
        assert_eq!(map_offset(3, &edits), 7, "inside: the end of the new text");
        assert_eq!(map_offset(6, &edits), 9);
        assert_eq!(map_offset(10, &edits), 16, "at an insertion: after it");
        assert_eq!(map_offset(12, &edits), 18);
    }
}
