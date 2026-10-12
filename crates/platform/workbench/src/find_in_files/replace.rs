//! Replace in Files: Replace (the selected match) and Replace All (every
//! match of the scope, after a confirmation), as the reference IDE does.
//!
//! An open file is changed through its editor, one undo step per file; a
//! file that is not open is changed on disk, keeping its encoding and line
//! separator. When a file cannot be written nothing is replaced and the
//! status bar names the read-only files.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, PromptLevel, Window};
use search::{CancelToken, Matcher};
use text::{CaretPlacement, Edit, EditKind, Selection, SelectionSet};

use super::{FindInFiles, FindInFilesEvent};

impl FindInFiles {
    /// The matcher of the shown results and the replacement, or why there
    /// is none (a malformed replacement marks the replace field).
    fn replace_parts(&self, cx: &mut Context<Self>) -> Option<(Matcher, String)> {
        let matcher = self.last_search.as_ref()?.matcher.clone();
        let template = self.replace.read(cx).text();
        if let Err(e) = matcher.check_template(&template) {
            self.replace.update(cx, |f, cx| f.set_error(true, cx));
            cx.emit(FindInFilesEvent::Message(format!(
                "Malformed replacement: {e}"
            )));
            return None;
        }
        Some((matcher, template))
    }

    fn editor_of(&self, path: &PathBuf) -> Option<Entity<editor::Editor>> {
        self.context
            .editors
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, e)| e.clone())
    }

    /// Replaces the selected match; its row goes and the next one is
    /// selected. The popup stays open.
    pub(super) fn replace_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        if !self.replace_mode || self.stale {
            return;
        }
        let Some(index) = self.selected else {
            return;
        };
        let Some(row) = self.results.row(index).cloned() else {
            return;
        };
        let Some((matcher, template)) = self.replace_parts(cx) else {
            return;
        };
        let path = (*row.path).clone();
        let only = HashSet::from([row.line.first.start]);
        if let Some(editor) = self.editor_of(&path) {
            let text = editor.read(cx).text();
            let done = match project_search::replacements(&text, &matcher, &template, Some(&only)) {
                Ok(reps) if editor.read(cx).buffer().is_read_only() && !reps.is_empty() => {
                    self.read_only(&[path], cx);
                    return;
                }
                Ok(reps) => {
                    let count = reps.len();
                    apply_in_editor(&editor, reps, cx);
                    count
                }
                Err(_) => 0,
            };
            self.replaced_row(index, done, cx);
            return;
        }
        let task = cx.background_spawn(async move {
            project_search::replace_in_file(&path, &matcher, &template, Some(&only))
                .map_err(|e| (path, e))
        });
        self.replace_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(count) => this.replaced_row(index, count, cx),
                Err((path, project_search::FileReplaceError::ReadOnly)) => {
                    this.read_only(&[path], cx)
                }
                Err((_, e)) => cx.emit(FindInFilesEvent::Message(format!("Cannot replace: {e}"))),
            });
        }));
    }

    /// The selected match was replaced (or was no longer there): its row
    /// goes, the row in its place is selected.
    fn replaced_row(&mut self, index: usize, count: usize, cx: &mut Context<Self>) {
        let _ = count;
        self.results.remove(index);
        self.selected = if self.results.is_empty() {
            None
        } else {
            Some(index.min(self.results.len() - 1))
        };
        self.preview_selected(cx);
        cx.notify();
    }

    fn read_only(&mut self, files: &[PathBuf], cx: &mut Context<Self>) {
        let names: Vec<String> = files.iter().map(|p| self.rel(p)).collect();
        let message = match names.as_slice() {
            [one] => format!("Cannot replace: {one} is read-only"),
            many => format!(
                "Cannot replace: {} files are read-only: {}",
                many.len(),
                many.join(", ")
            ),
        };
        cx.emit(FindInFilesEvent::Message(message));
        cx.notify();
    }

    /// Replace All: after a confirmation when the matches are in several
    /// files, every match of the scope is replaced (not only the rows
    /// shown) and the popup closes.
    pub(super) fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.replace_mode || self.stale || self.results.is_empty() {
            return;
        }
        let Some((matcher, template)) = self.replace_parts(cx) else {
            return;
        };
        let (matches, files) = self.counts();
        let several = self.results.len() > 1 && files != "1";
        if !several {
            self.run_replace_all(matcher, template, window, cx);
            return;
        }
        let query = self.query.read(cx).text();
        let detail = format!(
            "Replace {matches} occurrences of '{query}' across {files} files with '{template}'?"
        );
        let answer = window.prompt(
            PromptLevel::Info,
            "Replace All",
            Some(&detail),
            &["Replace", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.run_replace_all(matcher, template, window, cx)
                });
            }
        })
        .detach();
    }

    fn run_replace_all(
        &mut self,
        matcher: Matcher,
        template: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut search) = self.last_search.clone() else {
            return;
        };
        // Every match of the scope, with the open documents as they are now.
        search.limit = usize::MAX;
        search.known = Arc::default();
        search.overrides = self.overrides(cx);
        let editors = self.context.editors.clone();
        let find = cx.background_spawn(async move {
            let found = std::sync::Mutex::new(Vec::new());
            search.run(&CancelToken::new(), &|m| {
                if let Ok(mut f) = found.lock() {
                    f.push(m.path);
                }
            });
            let files = found.into_inner().unwrap_or_default();
            // Files that are not open and cannot be written.
            let read_only: Vec<PathBuf> = files
                .iter()
                .filter(|p| !editors_contain(&editors, p))
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.permissions().readonly()))
                .cloned()
                .collect();
            (files, read_only)
        });
        self.replace_task = Some(cx.spawn_in(window, async move |this, cx| {
            let (files, mut read_only) = find.await;
            let Ok(closed) = this.update(cx, |this, cx| {
                for path in &files {
                    if let Some(e) = this.editor_of(path)
                        && e.read(cx).buffer().is_read_only()
                    {
                        read_only.push(path.clone());
                    }
                }
                if !read_only.is_empty() {
                    this.read_only(&read_only, cx);
                    return None;
                }
                // Open files: one undo step each.
                let mut count = 0;
                let mut closed = Vec::new();
                for path in files {
                    match this.editor_of(&path) {
                        Some(editor) => {
                            let text = editor.read(cx).text();
                            if let Ok(reps) =
                                project_search::replacements(&text, &matcher, &template, None)
                            {
                                count += reps.len();
                                apply_in_editor(&editor, reps, cx);
                            }
                        }
                        None => closed.push(path),
                    }
                }
                Some((count, closed))
            }) else {
                return;
            };
            let Some((open_count, closed)) = closed else {
                return;
            };
            let written = cx
                .background_spawn(async move {
                    let mut count = 0;
                    let mut failed = Vec::new();
                    for path in closed {
                        match project_search::replace_in_file(&path, &matcher, &template, None) {
                            Ok(n) => count += n,
                            Err(e) => failed.push((path, e)),
                        }
                    }
                    (count, failed)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let (disk_count, failed) = written;
                let count = open_count + disk_count;
                let mut message = match count {
                    1 => "1 occurrence replaced".to_owned(),
                    n => format!("{n} occurrences replaced"),
                };
                if !failed.is_empty() {
                    let names: Vec<String> = failed.iter().map(|(p, _)| this.rel(p)).collect();
                    message.push_str(&format!("; not changed: {}", names.join(", ")));
                }
                this.save(cx);
                // The focus goes back to where it was.
                this.close(true, window, cx);
                cx.emit(FindInFilesEvent::Message(message));
            });
        }));
    }
}

fn editors_contain(editors: &[(PathBuf, Entity<editor::Editor>)], path: &PathBuf) -> bool {
    editors.iter().any(|(p, _)| p == path)
}

/// Makes the replacements in an open editor as one undo step; the caret
/// goes after the last one.
fn apply_in_editor(
    editor: &Entity<editor::Editor>,
    reps: Vec<project_search::Replacement>,
    cx: &mut gpui::App,
) {
    if reps.is_empty() {
        return;
    }
    let mut delta: isize = 0;
    let mut caret = 0;
    let edits: Vec<Edit> = reps
        .into_iter()
        .map(|r| {
            let len = r.text.chars().count() as isize;
            caret = (r.range.start as isize + delta + len) as usize;
            delta += len - r.range.len() as isize;
            Edit::new(r.range, r.text)
        })
        .collect();
    editor.update(cx, |e, cx| {
        e.edit(
            edits,
            EditKind::Other,
            CaretPlacement::Explicit(SelectionSet::single(Selection::caret(caret))),
            cx,
        )
    });
}
