//! The preview below the results: the selected row's file in a read-only
//! editor with its syntax colours, the row's matches marked and its line in
//! the middle, shown [`PREVIEW_DELAY`](super::PREVIEW_DELAY) after the
//! selection settles.

use std::path::PathBuf;

use editor::{Editor, SearchHighlights};
use gpui::{App, AppContext as _, Context, Entity, Task, Window};
use serde_json::json;
use text::{Buffer, Point};

use super::{FindInFiles, PREVIEW_DELAY};

pub(super) struct Preview {
    editor: Entity<Editor>,
    /// The file shown.
    path: Option<PathBuf>,
    /// The row shown: its line and where its first match starts.
    shown: Option<Point>,
    task: Option<Task<()>>,
}

impl Preview {
    pub fn new(window: &mut Window, cx: &mut Context<FindInFiles>) -> Self {
        let editor = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.buffer_mut().set_read_only(true);
            e
        });
        Self {
            editor,
            path: None,
            shown: None,
            task: None,
        }
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Nothing shown (the popup closed, or there are no rows).
    pub fn clear(&mut self, cx: &mut App) {
        self.task = None;
        self.shown = None;
        if self.path.take().is_some() {
            self.editor.update(cx, |e, cx| {
                *e.buffer_mut() = Buffer::new("");
                e.buffer_mut().set_read_only(true);
                e.buffer_reloaded(cx);
                e.set_search_highlights(None, cx);
            });
        }
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

    /// Loads the selected row's file into the preview (the open editor's
    /// text, else the file read in the background) and marks the row.
    fn show_preview(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self
            .selected
            .filter(|_| !self.stale)
            .and_then(|i| self.results.row(i))
            .cloned()
        else {
            self.preview.clear(cx);
            cx.notify();
            return;
        };
        let path = (*row.path).clone();
        if self.preview.path.as_ref() == Some(&path) {
            self.mark_preview(&row, cx);
            return;
        }
        let open = self
            .context
            .editors
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, e)| e.read(cx).text());
        match open {
            Some(text) => {
                self.load_preview(path, Buffer::new(&text), &row, cx);
            }
            None => {
                let read =
                    cx.background_spawn({
                        let path = path.clone();
                        async move {
                            project::read_document(&path).map(|l| Buffer::from_decoded(l.decoded))
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
        }
    }

    fn load_preview(
        &mut self,
        path: PathBuf,
        mut buffer: Buffer,
        row: &project_search::Row,
        cx: &mut Context<Self>,
    ) {
        buffer.set_read_only(true);
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
