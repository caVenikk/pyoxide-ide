//! The project's file name index (`file-index`) for Go to File and Search
//! Everywhere: built in the background when the project's rules are read
//! (when a folder opens) and again when they change (a folder marked
//! excluded), extended with the content of excluded folders when
//! non-project items are first asked for, and kept current with
//! [`ProjectFiles::apply`] (files created, deleted or renamed by the
//! application, and the changes the file watcher reports).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use file_index::{Change, FileIndex};
use gpui::{AppContext as _, Context, Task};
use project::Filter;

pub struct ProjectFiles {
    index: Option<Arc<FileIndex>>,
    /// The product's source roots of the project (found with the index).
    source_roots: Arc<Vec<PathBuf>>,
    find_source_roots: Option<fn(&Path) -> Vec<PathBuf>>,
    filter: Option<Filter>,
    /// The build running and its cancel flag.
    task: Option<Task<()>>,
    cancel: Arc<AtomicBool>,
    /// Non-project items are wanted (built once asked for).
    non_project: bool,
    /// How long the last full build took.
    build_time: Option<Duration>,
    generation: u64,
}

impl ProjectFiles {
    pub fn new() -> Self {
        Self {
            index: None,
            source_roots: Arc::default(),
            find_source_roots: None,
            filter: None,
            task: None,
            cancel: Arc::new(AtomicBool::new(false)),
            non_project: false,
            build_time: None,
            generation: 0,
        }
    }

    /// How the product finds a project's source roots.
    pub fn set_source_roots_finder(&mut self, find: Option<fn(&Path) -> Vec<PathBuf>>) {
        self.find_source_roots = find;
    }

    /// The project's source roots, found when the index was built.
    pub fn source_roots(&self) -> Arc<Vec<PathBuf>> {
        self.source_roots.clone()
    }

    /// The index, once built (it may lag behind a running rebuild).
    pub fn index(&self) -> Option<Arc<FileIndex>> {
        self.index.clone()
    }

    /// A build is running.
    pub fn is_building(&self) -> bool {
        self.task.is_some()
    }

    pub fn build_time(&self) -> Option<Duration> {
        self.build_time
    }

    /// The project's rules were read or changed: the index is built again.
    pub fn set_rules(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = Some(filter);
        self.rebuild(cx);
    }

    /// Builds the index again (changes on disk may have been missed).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
    }

    /// No project.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation += 1;
        self.task = None;
        self.index = None;
        self.source_roots = Arc::default();
        self.filter = None;
        cx.notify();
    }

    /// The content of excluded folders is wanted too (Include non-project
    /// items).
    pub fn want_non_project(&mut self, cx: &mut Context<Self>) {
        if self.non_project {
            return;
        }
        self.non_project = true;
        if self.task.is_none() {
            self.extend(cx);
        }
    }

    /// Lists the content of the excluded folders into the index built.
    fn extend(&mut self, cx: &mut Context<Self>) {
        let (Some(index), Some(filter)) = (self.index.clone(), self.filter.clone()) else {
            return;
        };
        let cancel = self.cancel.clone();
        let generation = self.generation;
        let build = cx.background_spawn(async move { index.with_non_project(&filter, &cancel) });
        self.task = Some(cx.spawn(async move |this, cx| {
            let built = build.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.task = None;
                if let Some(index) = built {
                    this.index = Some(Arc::new(index));
                }
                cx.notify();
            });
        }));
    }

    /// Files created, deleted or renamed: the index follows at once.
    pub fn apply(&mut self, changes: &[Change], cx: &mut Context<Self>) {
        let (Some(index), Some(filter)) = (&mut self.index, &self.filter) else {
            return;
        };
        Arc::make_mut(index).apply(changes, filter);
        // A build running started from the files before the change.
        if self.task.is_some() {
            self.rebuild(cx);
        }
        cx.notify();
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(filter) = self.filter.clone() else {
            return;
        };
        let Some(root) = filter.root().map(|r| r.to_path_buf()) else {
            return;
        };
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.generation += 1;
        let generation = self.generation;
        let non_project = self.non_project;
        let find_roots = self.find_source_roots;
        let build = cx.background_spawn(async move {
            let start = std::time::Instant::now();
            let index = FileIndex::build(&root, &filter, non_project, &cancel);
            let time = start.elapsed();
            let roots = find_roots.map(|find| find(&root)).unwrap_or_default();
            index.map(|i| (i, time, roots))
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let built = build.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.task = None;
                if let Some((index, time, roots)) = built {
                    this.index = Some(Arc::new(index));
                    this.build_time = Some(time);
                    this.source_roots = Arc::new(roots);
                }
                // Asked for non-project items while this build ran.
                if this.non_project && this.index.as_ref().is_some_and(|i| !i.has_non_project()) {
                    this.extend(cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Default for ProjectFiles {
    fn default() -> Self {
        Self::new()
    }
}
