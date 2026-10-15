//! The dialogs about files changed on disk, one at a time:
//!
//! - File Cache Conflict (the reference's `MemoryDiskConflictResolver`):
//!   a file changed on disk while its editor had changes. Load File System
//!   Changes reloads it (one undo step, so the editor's text can be had
//!   back); Keep Memory Changes (also Escape) keeps the editor's text,
//!   which the next save writes over the file. The reference's Show
//!   Difference needs a diff viewer, which Pyoxide does not have yet.
//! - "Undo Reload from Disk?" (the reference's `UndoRedo.askUser` for a
//!   step that asks for confirmation).
//! - Close Document (the reference's prompt for an editor that is not
//!   saved automatically, from its LightEdit mode): closing a modified
//!   editor whose file is gone from disk, or that is in a conflict.

use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context, Window};
use ui::icons::IconName;

use super::super::Workbench;
use super::super::run_actions::PendingDialog;
use super::disk::{Check, Checked, Known};
use crate::dialog::{Dialog, DialogButton};

/// A dialog about a file, waiting or open.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SyncDialog {
    Conflict(PathBuf),
    UndoReload { path: PathBuf, redo: bool },
    CloseModified(PathBuf),
}

/// Makes a dialog entity's content.
type MakeDialog = Box<dyn FnOnce(&mut Context<Dialog>) -> Dialog>;

fn button(id: &'static str, label: &'static str) -> DialogButton {
    DialogButton {
        id,
        label: label.into(),
    }
}

impl Workbench {
    /// Queues a dialog and shows it when no other dialog is open.
    pub(super) fn queue_sync_dialog(&mut self, dialog: SyncDialog, cx: &mut Context<Self>) {
        if !self.file_sync.dialogs.contains(&dialog) {
            self.file_sync.dialogs.push_back(dialog);
        }
        self.in_window(cx, |this, window, cx| this.show_sync_dialog(window, cx));
    }

    /// Opens the next waiting dialog that still applies.
    pub(in crate::workbench) fn show_sync_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while self.dialog.is_none() {
            let Some(next) = self.file_sync.dialogs.pop_front() else {
                return;
            };
            let Some(dialog) = self.sync_dialog(&next, cx) else {
                continue;
            };
            let dialog = cx.new(|cx| dialog(cx));
            self.open_dialog(dialog, PendingDialog::FileSync(next), window, cx);
        }
    }

    /// The dialog to show, unless it no longer applies.
    fn sync_dialog(&self, which: &SyncDialog, cx: &mut Context<Self>) -> Option<MakeDialog> {
        match which {
            SyncDialog::Conflict(path) => {
                self.file_sync.files.get(path)?.conflict.as_ref()?;
                self.editor_for(path)?;
                let message = format!(
                    "Changes have been made to '{}' in memory and on disk.",
                    path.display()
                );
                Some(Box::new(move |cx| {
                    Dialog::new(
                        "File Cache Conflict",
                        message,
                        vec![
                            button("keep", "&Keep Memory Changes"),
                            button("load", "&Load File System Changes"),
                        ],
                        cx,
                    )
                    .with_icon(IconName::QUESTION_DIALOG)
                }))
            }
            SyncDialog::UndoReload { path, redo } => {
                let editor = self.editor_for(path)?;
                let kind = editor.read(cx).buffer();
                let next = if *redo {
                    kind.redo_kind()
                } else {
                    kind.undo_kind()
                };
                (next == Some(text::EditKind::Reload)).then_some(())?;
                let (title, message) = if *redo {
                    ("Redo", "Redo Reload from Disk?")
                } else {
                    ("Undo", "Undo Reload from Disk?")
                };
                Some(Box::new(move |cx| {
                    Dialog::new(
                        title,
                        message,
                        vec![button("cancel", "Cancel"), button("ok", "OK")],
                        cx,
                    )
                    .with_icon(IconName::QUESTION_DIALOG)
                }))
            }
            SyncDialog::CloseModified(path) => {
                self.editor_for(path)?;
                Some(Box::new(|cx| {
                    Dialog::new(
                        "Close Document",
                        "The document has been modified.\nDo you want to save the changes?",
                        vec![
                            button("cancel", "Cancel"),
                            button("discard", "Discard"),
                            button("save", "Save"),
                        ],
                        cx,
                    )
                    .with_icon(IconName::WARNING_DIALOG)
                }))
            }
        }
    }

    /// A dialog about a file was closed with `button` (`None`: Escape).
    pub(in crate::workbench) fn sync_dialog_closed(
        &mut self,
        dialog: SyncDialog,
        button: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (dialog, button) {
            (SyncDialog::Conflict(path), Some("load")) => self.load_file_system_changes(path, cx),
            // Keep Memory Changes, and Escape.
            (SyncDialog::Conflict(path), _) => self.keep_memory_changes(&path, cx),
            (SyncDialog::UndoReload { path, redo }, Some("ok")) => {
                if let Some(editor) = self.editor_for(&path) {
                    editor.update(cx, |e, cx| e.undo_confirmed(redo, cx));
                }
            }
            (SyncDialog::CloseModified(path), Some(choice @ ("save" | "discard"))) => {
                self.close_modified(&path, choice == "save", window, cx)
            }
            _ => {}
        }
    }

    /// The editor's text is kept: the file's content becomes what the
    /// application knows, and the next save writes the editor's text over
    /// it.
    fn keep_memory_changes(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(state) = self.file_sync.files.get_mut(path) else {
            return;
        };
        if let Some(conflict) = state.conflict.take() {
            state.known = Some(conflict.disk);
            state.stamp = Some(conflict.stamp);
        }
        if let Some(editor) = self.editor_for(path) {
            editor.update(cx, |e, cx| {
                e.buffer_mut().mark_unsaved();
                cx.notify();
            });
        }
    }

    /// The editor takes the file's content now on disk, whatever its own
    /// changes (one undo step).
    fn load_file_system_changes(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_for(&path) else {
            return;
        };
        let Some(state) = self.file_sync.files.get(&path) else {
            return;
        };
        let check = {
            let e = editor.read(cx);
            let buffer = e.buffer();
            Check {
                path: path.clone(),
                epoch: state.epoch,
                // Read the file whatever its stamp.
                stamp: None,
                known: None,
                text: buffer.rope().clone(),
                version: buffer.version(),
                line_ending: buffer.line_ending(),
                encoding: buffer.encoding(),
                read_only: buffer.is_read_only(),
            }
        };
        self.pending += 1;
        let run = cx.background_spawn(async move { check.run() });
        cx.spawn(async move |this, cx| {
            let (path, _, _, checked) = run.await;
            let _ = this.update(cx, |this, cx| {
                this.pending -= 1;
                let Some(state) = this.file_sync.files.get_mut(&path) else {
                    return;
                };
                state.conflict = None;
                match checked {
                    Checked::Changed {
                        disk,
                        read_only,
                        stamp,
                        plan,
                    } => {
                        let reloaded = editor.update(cx, |e, cx| e.reload(plan, read_only, cx));
                        if reloaded {
                            state.known = Some(disk);
                            state.stamp = Some(stamp);
                        }
                    }
                    // The file is the editor's text after all.
                    Checked::Unchanged { stamp, disk } => {
                        state.known = disk.or_else(|| Some(Known::of(editor.read(cx).buffer())));
                        state.stamp = Some(stamp);
                        editor.update(cx, |e, cx| {
                            e.buffer_mut().mark_saved();
                            cx.notify();
                        });
                    }
                    Checked::Missing => this.file_missing(path, cx),
                    Checked::Unavailable => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Close Document: Save writes the editor's text (making the file
    /// again, or over a newer one), Discard drops the changes; the tab
    /// closes either way.
    fn close_modified(
        &mut self,
        path: &Path,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.editor_for(path) else {
            return;
        };
        if let Some(state) = self.file_sync.files.get_mut(path) {
            state.deleted = false;
            state.conflict = None;
            // Written without looking at the file.
            state.known = None;
            state.stamp = None;
        }
        if !save {
            editor.update(cx, |e, _| e.buffer_mut().mark_saved());
        }
        if let Some(index) = self.tabs.iter().position(|t| t.path == path) {
            self.close_tab(index, window, cx);
        }
    }
}
