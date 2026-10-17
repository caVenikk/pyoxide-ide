//! Files created, renamed or deleted from the project tool window (#37):
//! the file index, the language services, the editor tabs, the navigation
//! history and the lists of recent files follow, as the reference IDE's editors follow its virtual
//! file system (a renamed or moved file keeps its tab, unsaved changes
//! included; the tab of a deleted file closes).

use std::path::{Path, PathBuf};

use code_intel_ui::code_intel::{FileChange, FileChangeKind};
use gpui::{Context, Focusable as _, Window};
use project::fs_ops::FsChange;

use super::Workbench;

/// `path` after `from` became `to`, if it is `from` or inside it.
fn moved(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(from).ok()?;
    Some(if rest.as_os_str().is_empty() {
        to.to_owned()
    } else {
        to.join(rest)
    })
}

impl Workbench {
    /// Changes made on disk from the project tool window.
    pub(super) fn files_changed(
        &mut self,
        changes: &[FsChange],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index: Vec<file_index::Change> = changes
            .iter()
            .map(|c| match c {
                FsChange::Created { path, is_dir } => file_index::Change::Created {
                    path: path.clone(),
                    is_dir: *is_dir,
                },
                FsChange::Removed { path, .. } => file_index::Change::Removed(path.clone()),
                FsChange::Renamed { from, to, .. } => file_index::Change::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                },
            })
            .collect();
        self.project_files
            .update(cx, |files, cx| files.apply(&index, cx));
        for change in changes {
            match change {
                FsChange::Created { .. } => {}
                FsChange::Removed { path, .. } => self.files_removed(path, window, cx),
                FsChange::Renamed { from, to, .. } => self.files_moved(from, to, window, cx),
            }
        }
        if let Some(view) = &self.code_intel {
            let intel: Vec<FileChange> = changes
                .iter()
                .flat_map(|c| match c {
                    FsChange::Created { path, .. } => {
                        vec![FileChange::new(path, FileChangeKind::Created)]
                    }
                    FsChange::Removed { path, .. } => {
                        vec![FileChange::new(path, FileChangeKind::Deleted)]
                    }
                    FsChange::Renamed { from, to, .. } => vec![
                        FileChange::new(from, FileChangeKind::Deleted),
                        FileChange::new(to, FileChangeKind::Created),
                    ],
                })
                .collect();
            view.update(cx, |v, cx| v.files_changed(&intel, cx));
        }
        self.report_active_file(cx);
        cx.notify();
    }

    /// `from` was renamed or moved to `to`: its tabs (and those of the
    /// files inside a folder) keep their editors under the new path.
    fn files_moved(&mut self, from: &Path, to: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation.history.rename(from, to);
        self.debug_files_moved(from, to, cx);
        for list in [&mut self.recently_viewed, &mut self.recently_changed] {
            for p in list.iter_mut() {
                if let Some(new) = moved(p, from, to) {
                    *p = new;
                }
            }
        }
        for index in 0..self.tabs.len() {
            if let Some(new) = moved(&self.tabs[index].path, from, to) {
                self.retarget_tab(index, new, window, cx);
            }
        }
    }

    /// A tab shows its file under a new path: the title, the file type
    /// (highlighting follows a new extension) and the language services.
    fn retarget_tab(
        &mut self,
        index: usize,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old = std::mem::replace(&mut self.tabs[index].path, path.clone());
        let editor = self.tabs[index].editor.clone();
        let old_type = self.file_types.for_path(&old).map(|t| t.name);
        let file_type = self.file_types.for_path(&path).cloned();
        let type_changed = old_type != file_type.as_ref().map(|t| t.name);
        editor.update(cx, |e, cx| {
            e.set_path(Some(path.clone()));
            if type_changed {
                e.set_config(
                    file_type
                        .as_ref()
                        .map(|t| t.editor_config())
                        .unwrap_or_default(),
                );
                let highlighter = file_type
                    .as_ref()
                    .and_then(|t| t.highlighter.as_ref())
                    .map(|factory| factory());
                e.set_highlighter(highlighter, cx);
            }
            cx.notify();
        });
        if let Some(view) = &self.code_intel {
            let language = file_type.as_ref().and_then(|t| t.language_id);
            view.update(cx, |v, cx| {
                v.editor_closed(&old, cx);
                v.editor_opened(&path, language, &editor, window, cx);
            });
        }
        // Not an external change for the file watcher.
        self.sync_tab_moved(&old, &path, &editor, cx);
    }

    /// `path` was deleted: the tabs of it (and of the files inside a
    /// folder) close without saving, and the recent lists forget them.
    fn files_removed(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.recently_viewed.retain(|p| !p.starts_with(path));
        self.recently_changed.retain(|p| !p.starts_with(path));
        while let Some(index) = self.tabs.iter().position(|t| t.path.starts_with(path)) {
            self.discard_tab(index, window, cx);
        }
        self.navigation.history.remove(path);
    }

    /// Closes a tab without writing its file (it is gone). The focus moves
    /// only if it was in that editor.
    fn discard_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.tabs[index]
            .editor
            .focus_handle(cx)
            .contains_focused(window, cx);
        let tab = self.tabs.remove(index);
        self.sync_tab_closed(&tab.path, cx);
        if let Some(view) = &self.code_intel {
            view.update(cx, |v, cx| v.editor_closed(&tab.path, cx));
        }
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.saturating_sub(1).min(self.tabs.len() - 1)),
            other => other,
        };
        if focused {
            match self.active {
                Some(a) => window.focus(&self.tabs[a].editor.focus_handle(cx)),
                None => window.focus(&self.focus),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_a_move() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(moved(&p("/r/a"), &p("/r/a"), &p("/r/b")), Some(p("/r/b")));
        assert_eq!(
            moved(&p("/r/a/x.py"), &p("/r/a"), &p("/r/b")),
            Some(p("/r/b/x.py"))
        );
        assert_eq!(moved(&p("/r/ab"), &p("/r/a"), &p("/r/b")), None);
    }
}
