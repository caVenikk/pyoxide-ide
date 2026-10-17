//! Version control of the project (ADR 0016): the Git repository at or
//! above the project folder, its statuses kept current by
//! [`git::StatusService`] from the file watcher's batches, and the status
//! of each file for the tree, the tabs and the Commit tool window.
//!
//! Git stays off for a project mapped to no version control: `[vcs]
//! mapping = "none"` in the project settings, or, while the project has no
//! settings of its own, `<mapping directory="" vcs="" />` in the reference
//! IDE's `.idea/vcs.xml`. Without a git executable (or with one older than
//! the minimum) a project with a repository says so once.
//!
//! The change markers of the tabs and the status colours of names are in
//! [`markers`].

mod markers;

pub(super) use markers::base_of;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use file_watcher::{Batch, FileWatcher};
use git::{GitExe, GitRepo, Repository, StatusService, StatusUpdate};
use gpui::{App, AppContext as _, Context, Task};
use serde_json::{Value as Json, json};
use vcs::{FileStatus, Snapshot};

use super::Workbench;

/// The project's version control.
#[derive(Default)]
pub(super) struct VersionControl {
    pub(super) service: Option<Arc<StatusService>>,
    pub(super) snapshot: Option<Arc<Snapshot>>,
    /// Why there is no version control for a project with a repository
    /// (no git, an old git), or the last failed status.
    pub(super) problem: Option<String>,
    /// Git is off for the project by its settings.
    pub(super) off: bool,
    /// The project was looked at for a repository (the VCS widget shows
    /// nothing before).
    pub(super) detected: bool,
    /// Changes with every project, so late results of the previous one
    /// are dropped.
    generation: u64,
    _updates: Option<Task<()>>,
    /// Watches a repository folder outside the project folder (a project
    /// below the work tree's root, a linked work tree).
    _git_dir_watch: Option<(FileWatcher, Task<()>)>,
    /// Change markers of the tabs and status colours (kept across
    /// projects: the tabs stay open).
    pub(super) markers: markers::ChangeMarkers,
}

/// What a project's detection found off the UI thread.
enum Found {
    Off,
    NoRepository,
    Repository(Repository, Result<GitExe, git::LookupError>),
}

/// Git is off for the project by its settings, or by the reference's.
fn mapped_to_none(root: &Path) -> bool {
    if project::store::has_store(root) {
        let value = project::store::load_value(root, "vcs", "mapping")
            .ok()
            .flatten();
        return value.and_then(|v| v.as_str().map(str::to_owned)).as_deref() == Some("none");
    }
    git::idea::project_mapping(root) == Some(git::idea::Mapping::None)
}

fn detect(root: &Path) -> Found {
    if mapped_to_none(root) {
        return Found::Off;
    }
    match Repository::detect(root) {
        // The settings' `[git] path` comes with the settings store (#152).
        Some(repo) => Found::Repository(repo, GitExe::find(None)),
        None => Found::NoRepository,
    }
}

impl Workbench {
    /// A project was opened: its repository is looked for off the UI
    /// thread and its statuses follow.
    pub(super) fn vcs_project_opened(&mut self, root: &Path, cx: &mut Context<Self>) {
        let generation = self.version_control.generation + 1;
        let markers = std::mem::take(&mut self.version_control.markers);
        self.version_control = VersionControl {
            generation,
            markers,
            ..VersionControl::default()
        };
        self.vcs_markers_reset(cx);
        self.vcs_commit_snapshot(cx);
        self.commit_project_opened(root, cx);
        let root = root.to_owned();
        let look = {
            let root = root.clone();
            cx.background_spawn(async move { detect(&root) })
        };
        cx.spawn(async move |this, cx| {
            let found = look.await;
            let _ = this.update(cx, |this, cx| {
                if this.version_control.generation == generation {
                    this.vcs_found(&root, found, cx);
                }
            });
        })
        .detach();
    }

    fn vcs_found(&mut self, project_root: &Path, found: Found, cx: &mut Context<Self>) {
        self.version_control.detected = true;
        self.vcs_commit_snapshot(cx);
        cx.notify();
        let (repo, exe) = match found {
            Found::Off => {
                self.version_control.off = true;
                return;
            }
            Found::NoRepository => return,
            Found::Repository(repo, exe) => (repo, exe),
        };
        let exe = match exe {
            Ok(exe) => exe,
            Err(e) => {
                let text = e.to_string();
                self.version_control.problem = Some(text.clone());
                self.show_notice(text.into(), cx);
                return;
            }
        };
        let outside =
            !repo.git_dir.starts_with(project_root) || !repo.common_dir.starts_with(project_root);
        let folders: Vec<PathBuf> = if outside {
            let mut folders = vec![repo.common_dir.clone()];
            if !repo.git_dir.starts_with(&repo.common_dir) {
                folders.push(repo.git_dir.clone());
            }
            folders
        } else {
            Vec::new()
        };
        let backend = Arc::new(GitRepo::new(repo, exe).with_scope(project_root));
        let service = Arc::new(StatusService::start(backend));
        self.version_control.service = Some(service.clone());
        self.vcs_commit_snapshot(cx);
        let updates = service.updates();
        let generation = self.version_control.generation;
        self.version_control._updates = Some(cx.spawn(async move |this, cx| {
            while let Ok(update) = updates.recv_async().await {
                let alive = this.update(cx, |this, cx| {
                    if this.version_control.generation != generation {
                        return;
                    }
                    match update {
                        StatusUpdate::Snapshot(s) => {
                            this.version_control.snapshot = Some(s);
                            this.version_control.problem = None;
                            this.vcs_statuses_changed(cx);
                            this.vcs_branch_snapshot(cx);
                            this.vcs_commit_snapshot(cx);
                        }
                        StatusUpdate::Failed(e) => this.version_control.problem = Some(e.message),
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        if !folders.is_empty() && self.sync_settings().watch {
            self.watch_git_dirs(folders, service.clone(), cx);
        }
        self.version_control.service = Some(service);
    }

    /// Watches repository folders outside the project folder, passing
    /// their state files to the service.
    fn watch_git_dirs(
        &mut self,
        folders: Vec<PathBuf>,
        service: Arc<StatusService>,
        cx: &mut Context<Self>,
    ) {
        let start = cx.background_spawn(async move {
            let mut watcher =
                FileWatcher::new(|p: &Path| git::watched_in_git_dir(p) != Some(true))?;
            for folder in &folders {
                watcher.watch(folder, true)?;
            }
            Ok::<_, file_watcher::WatchError>(watcher)
        });
        let generation = self.version_control.generation;
        cx.spawn(async move |this, cx| {
            let Ok(watcher) = start.await else {
                return;
            };
            let events = watcher.events();
            let task = cx.background_spawn(async move {
                while let Ok(batch) = events.recv_async().await {
                    service.paths_changed(batch.paths.into_iter().map(|e| e.path).collect());
                }
            });
            let _ = this.update(cx, |this, _| {
                if this.version_control.generation == generation {
                    this.version_control._git_dir_watch = Some((watcher, task));
                }
            });
        })
        .detach();
    }

    /// A batch of the project's file watcher: the service hears about
    /// every path; paths inside `.git` folders are taken out of the batch
    /// (the tree, the index and the language services never see them).
    /// A `.git` folder that appears in a project without a repository
    /// (`git init`) is looked at.
    pub(super) fn vcs_files_changed(&mut self, batch: &mut Batch, cx: &mut Context<Self>) {
        let vc = &self.version_control;
        if batch.rescan
            && let Some(service) = &vc.service
        {
            service.refresh();
        }
        let mut new_repository = false;
        if let Some(service) = &vc.service {
            service.paths_changed(batch.paths.iter().map(|e| e.path.clone()).collect());
        } else if !vc.off && vc.problem.is_none() {
            let root = self.file_sync_root();
            new_repository = root.is_some_and(|root| {
                batch.paths.iter().any(|e| {
                    e.path.parent() == Some(root)
                        && e.path.file_name() == Some(std::ffi::OsStr::new(".git"))
                })
            });
        }
        batch
            .paths
            .retain(|e| git::watched_in_git_dir(&e.path).is_none());
        if new_repository && let Some(root) = self.file_sync_root().map(Path::to_path_buf) {
            self.vcs_project_opened(&root, cx);
        }
    }

    /// Looks at the whole work tree again (the window got the focus while
    /// no file watcher runs).
    pub(super) fn vcs_refresh(&self) {
        if let Some(service) = &self.version_control.service {
            service.refresh();
        }
    }

    /// The repository's last snapshot: its state and statuses.
    pub fn vcs_snapshot(&self) -> Option<&Arc<Snapshot>> {
        self.version_control.snapshot.as_ref()
    }

    /// The Git backend of the project, when it has a repository.
    pub fn git_repo(&self) -> Option<Arc<GitRepo>> {
        self.version_control
            .service
            .as_ref()
            .map(|s| s.repo().clone())
    }

    /// The status of a file, an open document's unsaved changes
    /// included: modified at once when edited, not modified when its text
    /// equals the committed one again.
    pub fn vcs_file_status(&self, path: &Path, _: &App) -> FileStatus {
        self.version_control.markers.colors.status(path, false)
    }

    /// The statuses names are coloured with (shared with the views that
    /// show file names).
    pub(super) fn vcs_file_colors(&self) -> &vcs_ui::FileColors {
        &self.version_control.markers.colors
    }

    /// The version control part of `workbench.state`.
    pub(super) fn vcs_state_json(&self, cx: &App) -> Json {
        let vc = &self.version_control;
        let open_files: Vec<Json> = match &vc.snapshot {
            Some(s) => self
                .tab_paths()
                .iter()
                .map(|p| {
                    json!({
                        "path": git::commands::relative_path(&s.info.root, p),
                        "status": self.vcs_file_status(p, cx).id(),
                        "markers": self.vcs_line_status(p).map(|t| t.read(cx).state_json()),
                    })
                })
                .collect(),
            None => self
                .tab_paths()
                .iter()
                .filter_map(|p| {
                    let markers = self.vcs_line_status(p)?.read(cx).state_json();
                    (markers["base"] == true).then(|| {
                        json!({
                            "path": p.display().to_string(),
                            "status": self.vcs_file_status(p, cx).id(),
                            "markers": markers,
                        })
                    })
                })
                .collect(),
        };
        json!({
            "vcs": vc.service.as_ref().map(|_| "Git"),
            "off": vc.off,
            "problem": vc.problem,
            "status": vc.snapshot.as_deref().map(git::snapshot_json),
            "open_files": open_files,
            "widget": self.vcs_widget_state().state_json(),
            "branches_popup": self.vcs_branches_popup_json(cx),
            "branch_dialog": self.vcs_branch_dialog_json(cx),
            "marker_popup": self.vcs_popup_json(cx),
            "commit": self.vcs_commit_json(cx),
            "remote": self.vcs_remote_json(cx),
        })
    }
}
