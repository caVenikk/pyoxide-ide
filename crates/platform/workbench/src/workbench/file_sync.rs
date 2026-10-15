//! Files changed on disk outside the application: a `git checkout`, a
//! formatter, another editor. Three things find them, as in the reference
//! IDE: the project's file watcher (its native `fsnotifier`), a look at
//! the open files when the window gets the focus ("Synchronize files on
//! frame activation", `SaveAndSyncHandlerImpl`), and Reload All from Disk
//! (`SynchronizeAction`: save everything, then look at everything).
//!
//! An open file that changed on disk (its size or modification time differ
//! from when the application last read or wrote it, and so does its text)
//! is reloaded when its editor is unmodified, as the reference's
//! `FileDocumentManagerImpl.contentsChanged` does: one undo step, carets
//! and the view kept ([`editor::Editor::reload`]). Files the application
//! writes itself are not changes: a check that overlaps a write of its
//! file is dropped, and the write leaves the file's new stamp behind.
//!
//! Created, changed and removed paths reach the project tree (folders
//! listed again), the file index and the language services (watched
//! files). When events were lost, or no watcher runs, the tree and the
//! index are built again instead.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use code_intel_ui::code_intel::{FileChange, FileChangeKind};
use editor::Editor;
use file_index::Change;
use file_watcher::{Batch, FileWatcher, Kind};
use gpui::{AppContext as _, Context, Entity, Task, Window};
use project::{DiskChange, Filter, Stamp};
use text::ReloadPlan;

use super::Workbench;
use crate::commands::Synchronize;
use crate::settings::SyncSettings;

/// What the application knows of an open file on disk.
#[derive(Debug, Default)]
struct DiskState {
    /// Size and modification time when the application last read or
    /// wrote the file; `None` until it has been looked at.
    stamp: Option<Stamp>,
    /// Writes of the file in flight.
    saving: usize,
    /// Changes with every write that starts or ends, so that a check that
    /// read the file around a write is dropped.
    epoch: u64,
}

/// The project's file watcher.
#[derive(Default)]
enum Watch {
    /// Not asked for, or no project.
    #[default]
    Off,
    /// Being set up off the UI thread.
    Starting,
    /// Watching while it is kept here.
    Running { _watcher: FileWatcher },
    /// Could not start: the window's activation looks at the project
    /// instead.
    Failed,
}

/// The watcher, its settings and the open files' state on disk.
#[derive(Default)]
pub(super) struct FileSync {
    settings: SyncSettings,
    watch: Watch,
    /// Receives the watcher's batches.
    events: Option<Task<()>>,
    files: HashMap<PathBuf, DiskState>,
    /// Changes with every project, so late results of the previous one
    /// are dropped.
    generation: u64,
}

/// One open file to look at, off the UI thread.
struct Check {
    path: PathBuf,
    epoch: u64,
    /// Skip reading when the file still has this stamp.
    stamp: Option<Stamp>,
    text: text::Rope,
    version: u64,
    line_ending: text::LineEnding,
    encoding: text::Encoding,
    read_only: bool,
}

/// What a check found.
enum Checked {
    /// The file is as the editor has it; its stamp now.
    Same(Stamp),
    /// The file differs from the editor.
    Changed {
        plan: ReloadPlan,
        read_only: bool,
        stamp: Stamp,
    },
    /// Gone, or not readable as text: left alone for now.
    Unavailable,
}

impl Check {
    fn run(self) -> (PathBuf, u64, Checked) {
        let result = match Stamp::of(&self.path) {
            None => Checked::Unavailable,
            Some(stamp) if Some(stamp) == self.stamp => Checked::Same(stamp),
            Some(_) => match project::read_document(&self.path) {
                Err(_) => Checked::Unavailable,
                Ok(loaded) => {
                    let read_only = loaded.read_only || loaded.decoded.lossy;
                    let plan = ReloadPlan::new(&self.text, self.version, &loaded.decoded);
                    let same = !plan.changes_text()
                        && loaded.decoded.line_ending == self.line_ending
                        && loaded.decoded.encoding == self.encoding
                        && read_only == self.read_only;
                    if same {
                        Checked::Same(loaded.stamp)
                    } else {
                        Checked::Changed {
                            plan,
                            read_only,
                            stamp: loaded.stamp,
                        }
                    }
                }
            },
        };
        (self.path, self.epoch, result)
    }
}

/// Writes a document (blocking) and returns the file's stamp after the
/// write, which tells the application's own writes from changes made
/// outside it.
pub(super) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<Option<Stamp>> {
    project::write_document(path, bytes)?;
    Ok(Stamp::of(path))
}

/// Paths the watcher leaves out: what is never shown (a version control
/// folder, byte code) and the application's own temporary save files.
fn ignored(root: &Path, path: &Path) -> bool {
    let hidden = Filter::default();
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .any(|c| hidden.is_hidden(&c.as_os_str().to_string_lossy()))
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

    /// A project was opened: the files of the previous one are forgotten
    /// and the new folder is watched (set up off the UI thread).
    pub(super) fn sync_project_opened(&mut self, root: &Path, cx: &mut Context<Self>) {
        let sync = &mut self.file_sync;
        sync.generation += 1;
        sync.files.clear();
        sync.watch = Watch::Off;
        sync.events = None;
        if !sync.settings.watch {
            return;
        }
        sync.watch = Watch::Starting;
        let generation = sync.generation;
        let root = root.to_owned();
        let start = cx.background_spawn(async move {
            let ignore_root = root.clone();
            let mut watcher = FileWatcher::new(move |p: &Path| ignored(&ignore_root, p))?;
            watcher.watch(&root, true)?;
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
        self.file_sync.watch = Watch::Running { _watcher: watcher };
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

    /// An editor tab was opened for a file.
    pub(super) fn sync_tab_opened(&mut self, path: &Path) {
        if path.is_absolute() {
            self.file_sync.files.entry(path.to_owned()).or_default();
        }
    }

    /// The tab of a file was closed.
    pub(super) fn sync_tab_closed(&mut self, path: &Path) {
        if !self.tabs.iter().any(|t| t.path == path) {
            self.file_sync.files.remove(path);
        }
    }

    /// The application starts writing a file.
    pub(super) fn sync_save_started(&mut self, path: &Path) {
        if let Some(state) = self.file_sync.files.get_mut(path) {
            state.saving += 1;
            state.epoch += 1;
        }
    }

    /// The application's write of a file ended; `stamp` is the file's
    /// stamp after a successful write.
    pub(super) fn sync_save_finished(&mut self, path: &Path, stamp: Option<Stamp>) {
        if let Some(state) = self.file_sync.files.get_mut(path) {
            state.saving = state.saving.saturating_sub(1);
            state.epoch += 1;
            if stamp.is_some() {
                state.stamp = stamp;
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

    /// The tree's folders and the file index are made again from the disk.
    fn rescan_project(&mut self, cx: &mut Context<Self>) {
        self.panel.update(cx, |panel, cx| panel.refresh_all(cx));
        self.project_files.update(cx, |files, cx| files.refresh(cx));
    }

    /// A batch of the watcher: the paths are looked at off the UI thread,
    /// then the tree, the index, the language services and the open files
    /// follow.
    fn files_changed_on_disk(&mut self, batch: Batch, cx: &mut Context<Self>) {
        if batch.rescan {
            self.rescan_project(cx);
            self.check_open_files(None, cx);
        }
        if batch.paths.is_empty() {
            return;
        }
        let open: Vec<PathBuf> = batch
            .paths
            .iter()
            .filter(|e| self.file_sync.files.contains_key(&e.path))
            .map(|e| e.path.clone())
            .collect();
        if !open.is_empty() {
            self.check_open_files(Some(open), cx);
        }
        let filter = self.panel.read(cx).filter().cloned().unwrap_or_default();
        let generation = self.file_sync.generation;
        let paths: Vec<(PathBuf, bool)> = batch
            .paths
            .into_iter()
            .map(|e| (e.path, e.kind != Kind::Changed))
            .collect();
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
        let mut dirs = BTreeSet::new();
        let mut index = Vec::new();
        let mut watched = Vec::new();
        for change in changes {
            let path = change.path();
            if let Some(parent) = path.parent() {
                dirs.insert(parent.to_owned());
            }
            if change.is_dir() {
                dirs.insert(path.to_owned());
            }
            let open = self.file_sync.files.contains_key(path);
            match change {
                DiskChange::Created { path, is_dir } | DiskChange::Changed { path, is_dir } => {
                    index.push(Change::Created {
                        path: path.clone(),
                        is_dir: *is_dir,
                    });
                }
                DiskChange::Removed(path) => index.push(Change::Removed(path.clone())),
            }
            let kind = match change {
                _ if open => None,
                DiskChange::Created { .. } => Some(FileChangeKind::Created),
                DiskChange::Changed { is_dir: false, .. } => Some(FileChangeKind::Changed),
                DiskChange::Changed { .. } => None,
                DiskChange::Removed(_) => Some(FileChangeKind::Deleted),
            };
            if let Some(kind) = kind {
                watched.push(FileChange::new(path, kind));
            }
        }
        let dirs: Vec<PathBuf> = dirs.into_iter().collect();
        self.panel
            .update(cx, |panel, cx| panel.refresh_dirs(&dirs, cx));
        self.project_files
            .update(cx, |files, cx| files.apply(&index, cx));
        if let Some(view) = &self.code_intel
            && !watched.is_empty()
        {
            view.update(cx, |v, cx| v.files_changed(&watched, cx));
        }
    }

    /// Looks at open files (`None`: all) off the UI thread and reloads the
    /// ones that changed on disk and are unmodified in their editors.
    fn check_open_files(&mut self, paths: Option<Vec<PathBuf>>, cx: &mut Context<Self>) {
        let checks: Vec<Check> = self
            .tabs
            .iter()
            .filter(|t| paths.as_ref().is_none_or(|p| p.contains(&t.path)))
            .filter_map(|t| {
                let state = self.file_sync.files.get(&t.path)?;
                let editor = t.editor.read(cx);
                // A modified editor keeps its text (a conflict, later).
                if state.saving > 0 || editor.is_modified() {
                    return None;
                }
                let buffer = editor.buffer();
                Some(Check {
                    path: t.path.clone(),
                    epoch: state.epoch,
                    stamp: state.stamp,
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
                    for (path, epoch, checked) in results {
                        this.checked(&path, epoch, checked, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn checked(&mut self, path: &Path, epoch: u64, checked: Checked, cx: &mut Context<Self>) {
        // The application wrote the file meanwhile: what was read may be
        // its own text or the one before.
        let current = self
            .file_sync
            .files
            .get(path)
            .is_some_and(|s| s.epoch == epoch && s.saving == 0);
        if !current {
            return;
        }
        let stamp = match checked {
            Checked::Same(stamp) => stamp,
            Checked::Unavailable => return,
            Checked::Changed {
                plan,
                read_only,
                stamp,
            } => {
                let Some(editor) = self.editor_for(path) else {
                    return;
                };
                if !reload(&editor, plan, read_only, cx) {
                    return;
                }
                if let Some(view) = &self.code_intel {
                    let change = FileChange::new(path, FileChangeKind::Changed);
                    view.update(cx, |v, cx| v.files_changed(&[change], cx));
                }
                stamp
            }
        };
        if let Some(state) = self.file_sync.files.get_mut(path) {
            state.stamp = Some(stamp);
        }
    }
}

/// Reloads an editor that is still unmodified and still has the text the
/// plan was made from.
fn reload(
    editor: &Entity<Editor>,
    plan: ReloadPlan,
    read_only: bool,
    cx: &mut Context<Workbench>,
) -> bool {
    editor.update(cx, |e, cx| {
        !e.is_modified() && e.reload(plan, read_only, cx)
    })
}
