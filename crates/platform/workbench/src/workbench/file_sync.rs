//! Files changed on disk outside the application: a `git checkout`, a
//! formatter, another editor. Three things find them, as in the reference
//! IDE: the project's file watcher (its native `fsnotifier`), a look at
//! the open files when the window gets the focus ("Synchronize files on
//! frame activation", `SaveAndSyncHandlerImpl`), and Reload All from Disk
//! (`SynchronizeAction`: save everything, then look at everything).
//!
//! An open file that changed on disk (its size or modification time differ
//! from when the application last read or wrote it, and so does its
//! content) is reloaded when its editor is unmodified, as the reference's
//! `FileDocumentManagerImpl.contentsChanged` does: one undo step, carets
//! and the view kept ([`editor::Editor::reload`]); undoing it asks first.
//! When the editor has changes of its own, the File Cache Conflict dialog
//! asks which to keep, and nothing writes the file until then: every save
//! looks at the file first and leaves one that changed meanwhile alone
//! (`dialogs`). The application's own writes are not changes: a check
//! that overlaps a write of its file is dropped, and the write leaves the
//! file's new stamp and content behind.
//!
//! A deleted file's tab closes, as in the reference (a rename outside the
//! application is a deletion and a creation). An editor with changes
//! stays open instead, its tab marked, and is not saved automatically:
//! Pyoxide has no Local History yet to get them back from. Save writes
//! the file again; closing the tab asks (Close Document).
//!
//! Created, changed and removed paths reach the project tree (folders
//! listed again), the file index and the language services (watched
//! files), also for the application's own writes of files without an
//! editor (Replace in Files). When events were lost, or no watcher runs,
//! the tree and the index are built again instead. Open files outside the
//! project folder, or without a project, are watched by their folders.

mod dialogs;
mod disk;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use code_intel_ui::code_intel::{FileChange, FileChangeKind};
use editor::{Editor, EditorEvent};
use file_index::Change;
use file_watcher::{Batch, FileWatcher, Kind};
use gpui::{AnyWindowHandle, AppContext as _, Context, Entity, Subscription, Task, Window};
use project::{DiskChange, Filter, Stamp};

pub(super) use dialogs::SyncDialog;
use disk::{Check, Checked, Conflict, Known, SaveGuard, Saved};

use super::Workbench;
use crate::commands::Synchronize;
use crate::settings::SyncSettings;

/// A file reported missing is looked at again after this long before its
/// tab closes: tools such as `git` remove a file and write it again.
const DELETE_GRACE: Duration = Duration::from_millis(300);

/// What the application knows of an open file on disk.
#[derive(Default)]
struct DiskState {
    /// Size and modification time when the application last read or
    /// wrote the file; `None` until it has been looked at.
    stamp: Option<Stamp>,
    /// The content the application last read or wrote; `None` until
    /// known (an editor opened with changes).
    known: Option<Known>,
    /// Writes of the file in flight, and the text of the last one.
    saving: usize,
    writing: Option<Known>,
    /// Changes with every write that starts or ends, so that a check that
    /// read the file around a write is dropped.
    epoch: u64,
    /// Changed on disk while the editor had changes: nothing writes the
    /// file until the person chose.
    conflict: Option<Conflict>,
    /// Gone from disk while the editor had changes.
    deleted: bool,
    /// The application wrote the file outside its editor (run
    /// configurations): the next check takes the change as known, with no
    /// conflict.
    own_write: bool,
    /// The editor's requests to confirm undoing a reload.
    _events: Option<Subscription>,
}

/// The file watcher: the project folder with everything below it, and
/// the folders of open files outside it (or of every open file without a
/// project), which the reference has no need for: it always has a
/// project, while Pyoxide opens single files too.
#[derive(Default)]
enum Watch {
    /// Not asked for, or nothing to watch yet.
    #[default]
    Off,
    /// Being set up off the UI thread.
    Starting,
    /// Watching while it is kept here (shared with the background work
    /// that adds and removes folders).
    Running { watcher: Arc<Mutex<FileWatcher>> },
    /// Could not start: the window's activation looks at the project
    /// instead.
    Failed,
}

/// The watcher, its settings and the open files' state on disk.
#[derive(Default)]
pub(super) struct FileSync {
    settings: SyncSettings,
    watch: Watch,
    /// The project folder, watched with everything below it.
    root: Option<PathBuf>,
    /// Folders watched for open files outside the project folder.
    folders: BTreeSet<PathBuf>,
    /// Receives the watcher's batches.
    events: Option<Task<()>>,
    files: HashMap<PathBuf, DiskState>,
    /// Changes with every project, so late results of the previous one
    /// are dropped.
    generation: u64,
    /// The window, for dialogs and tabs closed after background work.
    window: Option<AnyWindowHandle>,
    /// Dialogs waiting for the open one.
    dialogs: VecDeque<SyncDialog>,
    /// Writes the open files when the application quits.
    _quit: Option<Subscription>,
}

/// Paths the watcher leaves out: what is never shown (a version control
/// folder, byte code; below the project folder by any of its names,
/// elsewhere by the file's name) and the application's own temporary save
/// files.
fn ignored(root: Option<&Path>, path: &Path) -> bool {
    // Inside a `.git` folder: the files of the repository's state reach
    // version control (`version_control.rs`), the rest stays out.
    if let Some(state) = git::watched_in_git_dir(path) {
        return !state;
    }
    let hidden = Filter::default();
    let name_hidden = |name: &std::ffi::OsStr| hidden.is_hidden(&name.to_string_lossy());
    let hidden_path = match root.and_then(|r| path.strip_prefix(r).ok()) {
        Some(rel) => rel.components().any(|c| name_hidden(c.as_os_str())),
        None => path.file_name().is_some_and(name_hidden),
    };
    hidden_path
        || path
            .file_name()
            .is_some_and(|n| project::is_save_temp(&n.to_string_lossy()))
}

impl Workbench {
    pub fn sync_settings(&self) -> SyncSettings {
        self.file_sync.settings
    }

    /// Takes effect for the next project opened.
    pub fn set_sync_settings(&mut self, settings: SyncSettings) {
        self.file_sync.settings = settings;
    }

    /// The project's file watcher runs.
    pub fn is_watching(&self) -> bool {
        matches!(self.file_sync.watch, Watch::Running { .. })
    }

    /// The file of an open tab is gone from disk and its editor has
    /// changes (the tab stays, marked).
    pub fn is_deleted_on_disk(&self, path: &Path) -> bool {
        self.file_sync.files.get(path).is_some_and(|s| s.deleted)
    }

    /// Runs `f` with the window, after the current update.
    fn in_window(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(handle) = self.file_sync.window else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| f(this, window, cx));
            });
        })
        .detach();
    }

    /// A project was opened: the files of the previous one are forgotten
    /// and the new folder is watched (set up off the UI thread).
    pub(super) fn sync_project_opened(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync = &mut self.file_sync;
        sync.window = Some(window.window_handle());
        sync.files.clear();
        sync.dialogs.clear();
        sync.root = Some(root.to_owned());
        self.start_watcher(cx);
        self.vcs_project_opened(root, cx);
    }

    /// The project folder the watcher watches.
    pub(super) fn file_sync_root(&self) -> Option<&Path> {
        self.file_sync.root.as_deref()
    }

    /// Starts the watcher afresh (off the UI thread): the project folder,
    /// then the folders of open files outside it.
    fn start_watcher(&mut self, cx: &mut Context<Self>) {
        let sync = &mut self.file_sync;
        sync.generation += 1;
        sync.watch = Watch::Off;
        sync.events = None;
        sync.folders.clear();
        if !sync.settings.watch {
            return;
        }
        sync.watch = Watch::Starting;
        let generation = sync.generation;
        let root = sync.root.clone();
        let start = cx.background_spawn(async move {
            let ignore_root = root.clone();
            let mut watcher = FileWatcher::new(move |p: &Path| ignored(ignore_root.as_deref(), p))?;
            if let Some(root) = &root {
                watcher.watch(root, true)?;
            }
            Ok::<_, file_watcher::WatchError>(watcher)
        });
        cx.spawn(async move |this, cx| {
            let started = start.await;
            let _ = this.update(cx, |this, cx| {
                if this.file_sync.generation != generation {
                    return;
                }
                match started {
                    Ok(watcher) => this.watcher_started(watcher, cx),
                    // The reference warns that external changes may then
                    // be found late; the window's activation still finds
                    // them.
                    Err(e) => {
                        this.file_sync.watch = Watch::Failed;
                        let notice = format!("File watcher is not available: {e}");
                        this.show_notice(notice.into(), cx)
                    }
                }
            });
        })
        .detach();
    }

    fn watcher_started(&mut self, watcher: FileWatcher, cx: &mut Context<Self>) {
        let events = watcher.events();
        self.file_sync.watch = Watch::Running {
            watcher: Arc::new(Mutex::new(watcher)),
        };
        self.watch_folders_of_open_files(cx);
        self.file_sync.events = Some(cx.spawn(async move |this, cx| {
            while let Ok(batch) = events.recv_async().await {
                if this
                    .update(cx, |this, cx| this.files_changed_on_disk(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        // Files may have changed while the watcher was set up.
        self.check_open_files(None, cx);
        cx.notify();
    }

    /// Watches the folders of the open files outside the project folder
    /// (without their subfolders) and stops watching folders no open file
    /// is in any more. The watcher starts when the first such file opens.
    fn watch_folders_of_open_files(&mut self, cx: &mut Context<Self>) {
        let sync = &mut self.file_sync;
        let root = sync.root.as_deref();
        let wanted: BTreeSet<PathBuf> = sync
            .files
            .keys()
            .filter(|p| root.is_none_or(|r| !p.starts_with(r)))
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .collect();
        let watcher = match &sync.watch {
            Watch::Running { watcher } => watcher.clone(),
            Watch::Off if !wanted.is_empty() && sync.settings.watch => {
                return self.start_watcher(cx);
            }
            _ => return,
        };
        let added: Vec<PathBuf> = wanted.difference(&sync.folders).cloned().collect();
        let removed: Vec<PathBuf> = sync.folders.difference(&wanted).cloned().collect();
        if added.is_empty() && removed.is_empty() {
            return;
        }
        sync.folders = wanted;
        cx.background_spawn(async move {
            let mut watcher = watcher.lock().unwrap_or_else(|e| e.into_inner());
            for folder in removed {
                let _ = watcher.unwatch(&folder);
            }
            for folder in added {
                let _ = watcher.watch(&folder, false);
            }
        })
        .detach();
    }

    /// An editor tab was opened for a file.
    pub(super) fn sync_tab_opened(
        &mut self,
        path: &Path,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_sync.window = Some(window.window_handle());
        if self.file_sync._quit.is_none() {
            self.file_sync._quit = Some(cx.on_app_quit(|this, cx| {
                this.save_on_quit(cx);
                async {}
            }));
        }
        if !path.is_absolute() {
            return;
        }
        let buffer = editor.read(cx).buffer();
        // An editor opened with changes (from the Find in Files preview)
        // does not tell what the file is.
        let known = (!buffer.is_modified()).then(|| Known::of(buffer));
        editor.update(cx, |e, _| e.set_confirm_reload_undo(true));
        let events = Self::undo_reload_events(path, editor, cx);
        self.file_sync.files.insert(
            path.to_owned(),
            DiskState {
                known,
                _events: Some(events),
                ..DiskState::default()
            },
        );
        self.watch_folders_of_open_files(cx);
    }

    /// The editor's requests to confirm undoing a reload of `path`.
    fn undo_reload_events(
        path: &Path,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let key = path.to_owned();
        cx.subscribe(editor, move |this, _, event: &EditorEvent, cx| {
            if let EditorEvent::ConfirmUndo { redo } = event {
                let dialog = SyncDialog::UndoReload {
                    path: key.clone(),
                    redo: *redo,
                };
                this.queue_sync_dialog(dialog, cx);
            }
        })
    }

    /// The tab of a file was closed.
    pub(super) fn sync_tab_closed(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.tabs.iter().any(|t| t.is_file(path)) {
            self.file_sync.files.remove(path);
            self.watch_folders_of_open_files(cx);
        }
    }

    /// The application renamed or moved an open file (the project tool
    /// window, #37): what is known of it on disk goes with it, so the
    /// watcher's report of the old path gone and the new one created is
    /// not an external change (no tab closes, nothing reloads).
    pub(super) fn sync_tab_moved(
        &mut self,
        from: &Path,
        to: &Path,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut state) = self.file_sync.files.remove(from) else {
            return;
        };
        state._events = Some(Self::undo_reload_events(to, editor, cx));
        self.file_sync.files.insert(to.to_owned(), state);
        self.watch_folders_of_open_files(cx);
    }

    /// Before a tab closes: a modified editor that is not saved
    /// automatically (its file is gone, or changed on disk) asks first.
    /// True when the tab stays for now.
    pub(super) fn sync_close_asks(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        let Some((path, editor)) = self
            .tabs
            .get(index)
            .and_then(|t| Some((t.path()?, t.editor()?)))
        else {
            return false;
        };
        let held = self
            .file_sync
            .files
            .get(path)
            .is_some_and(|s| s.deleted || s.conflict.is_some());
        if !held || !editor.read(cx).is_modified() {
            return false;
        }
        let dialog = SyncDialog::CloseModified(path.clone());
        self.queue_sync_dialog(dialog, cx);
        true
    }

    /// An explicit save (Save, Save All): a file gone from disk is written
    /// again.
    pub(super) fn sync_explicit_save(&mut self) {
        for state in self.file_sync.files.values_mut() {
            if state.deleted {
                state.deleted = false;
                state.known = None;
                state.stamp = None;
            }
        }
    }

    /// The application is about to write a file: the guard that makes the
    /// write leave a file that changed meanwhile alone, or `None` when the
    /// file must not be written now (a conflict waits for the person's
    /// choice, or the file is gone).
    pub(super) fn sync_save_started(
        &mut self,
        path: &Path,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> Option<SaveGuard> {
        let text = Known::of(editor.read(cx).buffer());
        let Some(state) = self.file_sync.files.get_mut(path) else {
            return Some(SaveGuard::new(None, Vec::new(), text));
        };
        if state.conflict.is_some() || state.deleted {
            return None;
        }
        let expected: Vec<Known> = match &state.known {
            Some(known) => [Some(known.clone()), state.writing.clone()]
                .into_iter()
                .flatten()
                .collect(),
            None => Vec::new(),
        };
        // While another write runs, the stamp says nothing.
        let stamp = state.stamp.filter(|_| state.saving == 0);
        state.saving += 1;
        state.epoch += 1;
        state.writing = Some(text.clone());
        Some(SaveGuard::new(stamp, expected, text))
    }

    /// A write of a file ended. Returns how it went for the caller, or
    /// `None` when it was not written because the file changed or went
    /// away (handled here).
    pub(super) fn sync_save_finished(
        &mut self,
        path: &Path,
        result: std::io::Result<Saved>,
        cx: &mut Context<Self>,
    ) -> Option<std::io::Result<()>> {
        let mut state = self.file_sync.files.get_mut(path);
        if let Some(state) = state.as_mut() {
            state.saving = state.saving.saturating_sub(1);
            state.epoch += 1;
            if state.saving == 0 {
                state.writing = None;
            }
        }
        match result {
            Err(e) => Some(Err(e)),
            Ok(Saved::Written { stamp, text }) => {
                if let Some(state) = state {
                    state.stamp = stamp;
                    state.known = Some(text);
                }
                Some(Ok(()))
            }
            Ok(Saved::Conflict(conflict)) => {
                if let Some(state) = state {
                    state.conflict = Some(conflict);
                }
                self.queue_sync_dialog(SyncDialog::Conflict(path.to_owned()), cx);
                None
            }
            Ok(Saved::Missing) => {
                self.file_missing(path.to_owned(), cx);
                None
            }
        }
    }

    /// Writes every modified file that may be written when the
    /// application quits (the reference saves all documents on exit). The
    /// writes run here: the application does not wait for background work
    /// when it quits.
    fn save_on_quit(&mut self, cx: &mut Context<Self>) {
        for i in 0..self.tabs.len() {
            let (Some(path), Some(editor)) =
                (self.tabs[i].path().cloned(), self.tabs[i].editor().cloned())
            else {
                continue;
            };
            if !editor.read(cx).is_modified() {
                continue;
            }
            let Some(guard) = self.sync_save_started(&path, &editor, cx) else {
                continue;
            };
            let bytes = editor.read(cx).buffer().encoded();
            let result = guard.write(&path, &bytes);
            if let Some(Ok(())) = self.sync_save_finished(&path, result, cx) {
                editor.update(cx, |e, _| e.buffer_mut().mark_saved());
            }
        }
    }

    /// The window got the focus.
    pub(super) fn window_activated(&mut self, cx: &mut Context<Self>) {
        if !self.file_sync.settings.sync_on_activation {
            return;
        }
        self.check_open_files(None, cx);
        if matches!(self.file_sync.watch, Watch::Off | Watch::Failed) {
            self.rescan_project(cx);
            self.vcs_refresh();
        }
    }

    /// Reload All from Disk: every modified file is written, then the open
    /// files are looked at and the tree and the index built again.
    pub(super) fn synchronize(&mut self, _: &Synchronize, _: &mut Window, cx: &mut Context<Self>) {
        let saves = self.save_all_tasks(cx);
        cx.spawn(async move |this, cx| {
            futures::future::join_all(saves).await;
            let _ = this.update(cx, |this, cx| {
                this.check_open_files(None, cx);
                this.rescan_project(cx);
            });
        })
        .detach();
    }

    /// Reload from Disk on files and folders of the project tool window
    /// (`SynchronizeCurrentFileAction`): unlike Reload All from Disk nothing
    /// is written first; the open files under them are looked at (and
    /// reloaded when they changed outside), the index is built again (the
    /// tree lists their folders itself), and the status bar says so.
    pub(super) fn synchronize_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let open: Vec<PathBuf> = self
            .tab_paths()
            .into_iter()
            .filter(|t| paths.iter().any(|p| t.starts_with(p)))
            .collect();
        if !open.is_empty() {
            self.check_open_files(Some(open), cx);
        }
        self.project_files.update(cx, |files, cx| files.refresh(cx));
        self.show_notice("File synchronization completed successfully.".into(), cx);
    }

    /// The tree's folders and the file index are made again from the disk.
    fn rescan_project(&mut self, cx: &mut Context<Self>) {
        self.panel.update(cx, |panel, cx| panel.refresh_all(cx));
        self.project_files.update(cx, |files, cx| files.refresh(cx));
    }

    /// A batch of the watcher: the paths are looked at off the UI thread,
    /// then the tree, the index, the language services and the open files
    /// follow.
    fn files_changed_on_disk(&mut self, mut batch: Batch, cx: &mut Context<Self>) {
        self.vcs_files_changed(&mut batch, cx);
        if batch.rescan {
            self.rescan_project(cx);
            self.check_open_files(None, cx);
        }
        if batch.paths.is_empty() {
            return;
        }
        // Open files reported, and those in a folder reported (removed).
        let open: Vec<PathBuf> = self
            .file_sync
            .files
            .keys()
            .filter(|f| batch.paths.iter().any(|e| f.starts_with(&e.path)))
            .cloned()
            .collect();
        if !open.is_empty() {
            self.check_open_files(Some(open), cx);
        }
        let filter = self.panel.read(cx).filter().cloned().unwrap_or_default();
        let generation = self.file_sync.generation;
        // Folders outside the project are watched for their open files
        // only: the tree, the index and the services follow the project.
        let Some(root) = self.file_sync.root.clone() else {
            return;
        };
        let paths: Vec<(PathBuf, bool)> = batch
            .paths
            .into_iter()
            .filter(|e| e.path.starts_with(&root))
            .map(|e| (e.path, e.kind != Kind::Changed))
            .collect();
        if paths.is_empty() {
            return;
        }
        self.pending += 1;
        let look = cx.background_spawn(async move { project::disk_changes(paths, &filter) });
        cx.spawn(async move |this, cx| {
            let changes = look.await;
            let _ = this.update(cx, |this, cx| {
                this.pending -= 1;
                if this.file_sync.generation == generation {
                    this.apply_disk_changes(&changes, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The tree lists the folders that changed again, the index takes the
    /// changes, and the language services hear about the files that are
    /// not open (an open file's change is reported once its check finds
    /// it is not the application's own write).
    fn apply_disk_changes(&mut self, changes: &[DiskChange], cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        let index = self.project_files.read(cx).index();
        let mut dirs = BTreeSet::new();
        let mut index_changes = Vec::new();
        let mut watched = Vec::new();
        for change in changes {
            let path = change.path();
            if let Some(parent) = path.parent() {
                dirs.insert(parent.to_owned());
            }
            if change.is_dir() {
                dirs.insert(path.to_owned());
            }
            // A file written over (an atomic save) was there before.
            let indexed = index.as_ref().is_some_and(|i| i.find(path).is_some());
            let kind = match change {
                _ if self.file_sync.files.contains_key(path) => None,
                DiskChange::Created { .. } if indexed => Some(FileChangeKind::Changed),
                DiskChange::Created { .. } => Some(FileChangeKind::Created),
                DiskChange::Changed { is_dir: false, .. } => Some(FileChangeKind::Changed),
                DiskChange::Changed { .. } => None,
                DiskChange::Removed(_) => Some(FileChangeKind::Deleted),
            };
            if let Some(kind) = kind {
                watched.push(FileChange::new(path, kind));
            }
            index_changes.push(match change {
                DiskChange::Created { path, is_dir } | DiskChange::Changed { path, is_dir } => {
                    Change::Created {
                        path: path.clone(),
                        is_dir: *is_dir,
                    }
                }
                DiskChange::Removed(path) => Change::Removed(path.clone()),
            });
        }
        let dirs: Vec<PathBuf> = dirs.into_iter().collect();
        self.panel
            .update(cx, |panel, cx| panel.refresh_dirs(&dirs, cx));
        self.project_files
            .update(cx, |files, cx| files.apply(&index_changes, cx));
        if let Some(view) = &self.code_intel
            && !watched.is_empty()
        {
            view.update(cx, |v, cx| v.files_changed(&watched, cx));
        }
    }

    /// Files the application wrote or removed outside their editors (run
    /// configurations in the project folder, which the watcher leaves out):
    /// an open one is looked at now, and a change found is the
    /// application's own: an unmodified editor shows the new text, an
    /// editor with changes keeps them and asks nothing (its next save
    /// writes them).
    pub(super) fn sync_files_written(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let open: Vec<PathBuf> = paths
            .iter()
            .filter(|p| self.file_sync.files.contains_key(*p))
            .cloned()
            .collect();
        if open.is_empty() {
            return;
        }
        for path in &open {
            if let Some(state) = self.file_sync.files.get_mut(path) {
                state.own_write = true;
            }
        }
        self.check_open_files(Some(open), cx);
    }

    /// Looks at open files (`None`: all) off the UI thread.
    pub(super) fn check_open_files(&mut self, paths: Option<Vec<PathBuf>>, cx: &mut Context<Self>) {
        let checks: Vec<Check> = self
            .tabs
            .iter()
            .filter_map(|t| Some((t.path()?, t.editor()?)))
            .filter(|(path, _)| paths.as_ref().is_none_or(|p| p.contains(path)))
            .filter_map(|(path, editor)| {
                let state = self.file_sync.files.get(path)?;
                if state.saving > 0 {
                    return None;
                }
                let buffer = editor.read(cx).buffer();
                Some(Check {
                    path: path.clone(),
                    epoch: state.epoch,
                    stamp: state.stamp,
                    known: state.known.clone(),
                    text: buffer.rope().clone(),
                    version: buffer.version(),
                    line_ending: buffer.line_ending(),
                    encoding: buffer.encoding(),
                    read_only: buffer.is_read_only(),
                })
            })
            .collect();
        if checks.is_empty() {
            return;
        }
        let generation = self.file_sync.generation;
        self.pending += 1;
        let run =
            cx.background_spawn(
                async move { checks.into_iter().map(Check::run).collect::<Vec<_>>() },
            );
        cx.spawn(async move |this, cx| {
            let results = run.await;
            let _ = this.update(cx, |this, cx| {
                this.pending -= 1;
                if this.file_sync.generation == generation {
                    for (path, epoch, version, checked) in results {
                        this.checked(path, epoch, version, checked, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn checked(
        &mut self,
        path: PathBuf,
        epoch: u64,
        version: u64,
        checked: Checked,
        cx: &mut Context<Self>,
    ) {
        // The application wrote the file meanwhile: what was read may be
        // its own text or the one before.
        let current = self
            .file_sync
            .files
            .get(&path)
            .is_some_and(|s| s.epoch == epoch && s.saving == 0);
        let Some(editor) = self.editor_for(&path).filter(|_| current) else {
            return;
        };
        let (modified, now) = {
            let e = editor.read(cx);
            (e.is_modified(), e.buffer().version())
        };
        match checked {
            Checked::Unavailable => {}
            Checked::Missing => self.file_missing(path, cx),
            Checked::Unchanged { stamp, disk } => {
                let state = self.file_sync.files.get_mut(&path).expect("checked");
                state.stamp = Some(stamp);
                state.deleted = false;
                state.own_write = false;
                if let Some(disk) = disk {
                    state.known = Some(disk);
                    // The editor's changes are what the file has now.
                    if modified && now == version {
                        editor.update(cx, |e, _| e.buffer_mut().mark_saved());
                    }
                }
            }
            Checked::Changed {
                disk,
                read_only,
                stamp,
                plan,
            } => {
                let state = self.file_sync.files.get_mut(&path).expect("checked");
                state.deleted = false;
                let own = std::mem::take(&mut state.own_write);
                if modified && own {
                    // The application's own write under the person's
                    // changes: no conflict; their save writes them.
                    state.known = Some(disk);
                    state.stamp = Some(stamp);
                    cx.notify();
                    return;
                }
                if !modified {
                    if !editor.update(cx, |e, cx| e.reload(plan, read_only, cx)) {
                        // Edited meanwhile: the next check looks again.
                        return;
                    }
                    state.known = Some(disk);
                    state.stamp = Some(stamp);
                } else if state.known.is_none() {
                    // What the editor was opened from is not known.
                    state.known = Some(disk);
                    state.stamp = Some(stamp);
                    return;
                } else if state.conflict.is_none() {
                    state.conflict = Some(Conflict { disk, stamp });
                    self.queue_sync_dialog(SyncDialog::Conflict(path.clone()), cx);
                }
                if let Some(view) = &self.code_intel {
                    let change = FileChange::new(&path, FileChangeKind::Changed);
                    view.update(cx, |v, cx| v.files_changed(&[change], cx));
                }
            }
        }
        cx.notify();
    }

    /// An open file was found missing: after a moment, if it is still
    /// gone, its tab closes (as in the reference), or, when its editor
    /// has changes, stays marked.
    fn file_missing(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let generation = self.file_sync.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DELETE_GRACE).await;
            let probe = path.clone();
            let gone = cx
                .background_spawn(async move { matches!(probe.try_exists(), Ok(false)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.file_sync.generation != generation {
                    return;
                }
                if gone {
                    this.file_deleted(&path, cx);
                } else {
                    this.check_open_files(Some(vec![path]), cx);
                }
            });
        })
        .detach();
    }

    fn file_deleted(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(editor) = self.editor_for(path) else {
            return;
        };
        let Some(state) = self.file_sync.files.get_mut(path) else {
            return;
        };
        if editor.read(cx).is_modified() {
            state.deleted = true;
            state.conflict = None;
            state.stamp = None;
            cx.notify();
            return;
        }
        let path = path.to_owned();
        self.in_window(cx, move |this, window, cx| {
            if let Some(index) = this.file_tab(&path) {
                this.close_tab(index, window, cx);
            }
        });
    }
}
