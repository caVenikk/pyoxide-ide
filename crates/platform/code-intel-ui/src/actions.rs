//! Code actions at a problem: the diagnostics as the services reported
//! them (a code action request sends them back, servers find their fixes
//! through them), the first quick fix (Apply First Quick Fix and the
//! actions row of the documentation popup), and applying an action: its
//! edit, resolved first when the server computes it on demand, then its
//! command.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use code_intel::{
    ActionTrigger, CodeAction, CodeActionContext, Diagnostic, DocumentId, FileChange,
    FileChangeKind, Rope, TextEdit, WorkspaceEdit,
};
use editor::Editor;
use gpui::{App, AppContext as _, Context, Entity};
use text::{CaretPlacement, Edit, EditKind, SelectionSet};

use crate::doc;
use crate::document::point_to_offset;
use crate::view::{CodeIntelEvent, CodeIntelView};

/// The diagnostics of each document as last reported, with the version of
/// the text their ranges are in.
#[derive(Default)]
pub(crate) struct Reported {
    by_path: HashMap<PathBuf, (u64, Vec<Diagnostic>)>,
}

impl Reported {
    pub(crate) fn set(&mut self, path: &Path, version: u64, items: Vec<Diagnostic>) {
        self.by_path.insert(path.to_owned(), (version, items));
    }

    pub(crate) fn forget(&mut self, path: &Path) {
        self.by_path.remove(path);
    }

    pub(crate) fn clear(&mut self) {
        self.by_path.clear();
    }
}

/// A char offset moved through edits (in the coordinates of the text
/// before them, not overlapping): after the inserted text when an edit
/// replaced the text around it.
pub fn map_through(offset: usize, edits: &[Edit]) -> usize {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| e.range.start);
    let mut shift: isize = 0;
    for edit in sorted {
        let inserted = edit.text.chars().count() as isize;
        if edit.range.end <= offset && !(edit.range.is_empty() && edit.range.start == offset) {
            shift += inserted - edit.range.len() as isize;
        } else if edit.range.start < offset {
            return (edit.range.start as isize + shift + inserted).max(0) as usize;
        } else {
            break;
        }
    }
    (offset as isize + shift).max(0) as usize
}

impl CodeIntelView {
    /// The services reported the diagnostics of a document.
    pub(crate) fn diagnostics_reported(
        &mut self,
        doc: &DocumentId,
        version: u64,
        items: &[Diagnostic],
    ) {
        self.reported.set(doc.path(), version, items.to_vec());
    }

    /// The reported diagnostics of a document whose ranges, moved into its
    /// current text, meet a char range.
    pub(crate) fn diagnostics_in(
        &self,
        doc: &DocumentId,
        range: Range<usize>,
        _: &App,
    ) -> Vec<Diagnostic> {
        let (Some(document), Some((version, items))) = (
            self.document(doc.path()),
            self.reported.by_path.get(doc.path()),
        ) else {
            return Vec::new();
        };
        items
            .iter()
            .filter(|d| {
                document
                    .versions
                    .range(*version, d.range.clone())
                    .is_some_and(|r| r.start <= range.end && range.start <= r.end)
            })
            .cloned()
            .collect()
    }

    /// Apply First Quick Fix: the first quick fix of the problem at the
    /// caret of an editor.
    pub(crate) fn apply_first_fix_at_caret(
        &mut self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        let (Some(query), Some(services)) =
            (self.caret_query(editor, cx), self.services().cloned())
        else {
            return;
        };
        let (_, mark) = doc::problems_at(editor.read(cx).diagnostics(), query.offset, true);
        let (Some(range), Some(document)) = (mark, self.document_of(editor)) else {
            return;
        };
        let points = document.versions.point(range.start)..document.versions.point(range.end);
        let context = CodeActionContext {
            only: None,
            diagnostics: self.diagnostics_in(&query.doc, range, cx),
            trigger: ActionTrigger::Invoked,
        };
        let request = services.code_actions(&query.doc, points, context);
        let (editor, version) = (editor.clone(), query.version);
        cx.spawn(async move |this, cx| {
            let Ok(actions) = request.await else {
                return;
            };
            let Some(fix) = actions.into_iter().find(|a| a.kind.is_quick_fix()) else {
                return;
            };
            let _ = this.update(cx, |v, cx| v.apply_code_action(&editor, version, fix, cx));
        })
        .detach();
    }

    /// Applies a code action offered for a document's editor, whose edits
    /// are in the text of `version`: resolves its edit when the server
    /// computes it on demand, applies it, then runs its command.
    pub(crate) fn apply_code_action(
        &mut self,
        editor: &Entity<Editor>,
        version: u64,
        action: CodeAction,
        cx: &mut Context<Self>,
    ) {
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let doc = document.id.clone();
        let resolve = (action.edit.is_none() && action.token.is_some())
            .then(|| services.resolve_code_action(&doc, action.clone()));
        cx.spawn(async move |this, cx| {
            let action = match resolve {
                Some(request) => request.await.unwrap_or(action),
                None => action,
            };
            let _ = this.update(cx, |v, cx| {
                if let Some(edit) = &action.edit {
                    v.apply_workspace_edit(edit, Some((doc.path(), version)), cx);
                }
                if let Some(command) = action.command {
                    cx.background_spawn(services.execute_command(command))
                        .detach();
                }
            });
        })
        .detach();
    }

    /// Applies the edits of a workspace edit to the documents of the
    /// window, each document's edits as one undo step. Edits of `origin`
    /// are in its text of the given version (they are moved into the
    /// current text), those of other documents in their current text.
    /// Returns false when edits of a file that is not open were left out.
    pub(crate) fn apply_workspace_edit(
        &mut self,
        edit: &WorkspaceEdit,
        origin: Option<(&Path, u64)>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut complete = true;
        let mut on_disk = Vec::new();
        for file in &edit.files {
            if file.edits.is_empty() {
                continue;
            }
            let edits = if let Some(document) = self.document(&file.path) {
                let version = match origin {
                    Some((path, version)) if path == file.path => version,
                    _ => document.version(),
                };
                let edits: Option<Vec<Edit>> = file
                    .edits
                    .iter()
                    .map(|e| {
                        document
                            .versions
                            .range(version, e.range.clone())
                            .map(|r| Edit::new(r, e.text.clone()))
                    })
                    .collect();
                edits.map(|edits| (document.editor.clone(), edits))
            } else if let Some(editor) = self.tab_editor(&file.path) {
                // A tab that is no document: the edits are in its text.
                let rope = editor.read(cx).buffer().rope().clone();
                let edits = file
                    .edits
                    .iter()
                    .map(|e| {
                        let start = point_to_offset(&rope, e.range.start);
                        let end = point_to_offset(&rope, e.range.end).max(start);
                        Edit::new(start..end, e.text.clone())
                    })
                    .collect();
                Some((editor, edits))
            } else if self
                .project_root()
                .is_some_and(|root| file.path.starts_with(root))
            {
                on_disk.push(file.clone());
                continue;
            } else {
                // Files outside the project are not changed.
                cx.emit(CodeIntelEvent::Notice(format!(
                    "Not changed (outside the project): {}",
                    file.path.display()
                )));
                complete = false;
                continue;
            };
            let Some((editor, edits)) = edits else {
                // The text the edits were made for is gone.
                complete = false;
                continue;
            };
            editor.update(cx, |e, cx| {
                let caret = map_through(e.selections().primary().head, &edits);
                e.edit(
                    edits,
                    EditKind::Other,
                    CaretPlacement::Explicit(SelectionSet::caret(caret)),
                    cx,
                );
            });
        }
        if !on_disk.is_empty() {
            // Files that are not open: changed (or created) on disk, off
            // the UI thread, and reported to the services.
            let write = cx.background_spawn(async move {
                on_disk
                    .iter()
                    .filter_map(|f| {
                        edit_file(&f.path, &f.edits)
                            .ok()
                            .map(|kind| FileChange::new(f.path.clone(), kind))
                    })
                    .collect::<Vec<_>>()
            });
            cx.spawn(async move |this, cx| {
                let changes = write.await;
                let _ = this.update(cx, |v, cx| v.files_changed(&changes, cx));
            })
            .detach();
        }
        complete
    }
}

/// A text with edits applied (positions in the text before them, which
/// do not overlap).
pub fn edit_text(text: &str, edits: &[TextEdit]) -> String {
    let crlf = text.contains("\r\n");
    let mut rope = Rope::from_str(&text.replace("\r\n", "\n"));
    let mut ranges: Vec<(Range<usize>, &str)> = edits
        .iter()
        .map(|e| {
            let start = point_to_offset(&rope, e.range.start);
            let end = point_to_offset(&rope, e.range.end).max(start);
            (start..end, e.text.as_str())
        })
        .collect();
    // From the end, so that earlier positions stay valid.
    ranges.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    for (range, text) in ranges {
        rope.remove(range.clone());
        rope.insert(range.start, &text.replace("\r\n", "\n"));
    }
    let out = rope.to_string();
    if crlf { out.replace('\n', "\r\n") } else { out }
}

/// Applies edits to a file on disk; a file that does not exist is created
/// (with its folders) from the edits.
pub fn edit_file(path: &Path, edits: &[TextEdit]) -> std::io::Result<FileChangeKind> {
    let (text, kind) = match std::fs::read_to_string(path) {
        Ok(text) => (text, FileChangeKind::Changed),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            (String::new(), FileChangeKind::Created)
        }
        Err(e) => return Err(e),
    };
    std::fs::write(path, edit_text(&text, edits))?;
    Ok(kind)
}

#[cfg(test)]
mod tests {
    use code_intel::Point;

    use super::*;

    #[test]
    fn carets_move_with_edits() {
        let edits = [Edit::new(0..2, "abc"), Edit::new(5..7, "")];
        assert_eq!(map_through(0, &edits), 0);
        assert_eq!(map_through(3, &edits), 4);
        // Inside a replaced part: after its new text.
        assert_eq!(map_through(1, &edits), 3);
        assert_eq!(map_through(6, &edits), 6);
        assert_eq!(map_through(9, &edits), 8);
        // Text inserted at the caret goes after it.
        assert_eq!(map_through(4, &[Edit::new(4..4, "xy")]), 4);
        assert_eq!(map_through(5, &[Edit::new(4..4, "xy")]), 7);
    }

    #[test]
    fn files_on_disk_are_edited_or_created() {
        let edits = [
            TextEdit::insert(Point::new(0, 0), "# ruff: noqa: F401\n"),
            TextEdit::new(Point::new(1, 0)..Point::new(1, 3), "y"),
        ];
        assert_eq!(
            edit_text("import os\r\nabc = 1\r\n", &edits),
            "# ruff: noqa: F401\r\nimport os\r\ny = 1\r\n"
        );
        let dir = std::env::temp_dir().join(format!("code-intel-ui-{}", std::process::id()));
        let new = dir.join("sub/ruff.toml");
        let kind = edit_file(
            &new,
            &[TextEdit::insert(
                Point::new(0, 0),
                "[lint]\nignore = [\"F401\"]\n",
            )],
        )
        .unwrap();
        assert_eq!(kind, FileChangeKind::Created);
        assert_eq!(
            std::fs::read_to_string(&new).unwrap(),
            "[lint]\nignore = [\"F401\"]\n"
        );
        let kind = edit_file(
            &new,
            &[TextEdit::new(
                Point::new(1, 10)..Point::new(1, 16),
                "\"E711\"",
            )],
        )
        .unwrap();
        assert_eq!(kind, FileChangeKind::Changed);
        assert_eq!(
            std::fs::read_to_string(&new).unwrap(),
            "[lint]\nignore = [\"E711\"]\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
