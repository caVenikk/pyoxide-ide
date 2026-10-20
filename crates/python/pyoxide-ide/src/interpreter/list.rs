//! The application's list of configured interpreters in memory
//! ([`Table`]): read once in the background when the product starts,
//! changed at once in memory and written in the background. Views observe
//! the [`Interpreters`] global. The reference IDE's interpreters come
//! through the first-run window's importer (`import`); only people who
//! never saw that window get them straight from the reference's folder
//! ([`import_reference`]).
//!
//! The application keeps the list in its configuration folder
//! ([`keep_in_config_dir`], called by `main`); tests keep it in memory, or
//! in a file of their own ([`set_file`]).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{App, AppContext as _, Global, Task};

use super::table::{self, Change, Table};

/// The list and where it is kept.
#[derive(Default)]
pub(crate) struct Interpreters {
    table: Arc<Table>,
    loaded: bool,
    /// Counts the changes made in memory: a read that started before one
    /// does not replace it.
    generation: u64,
    /// The file; `None` keeps the list in memory.
    file: Option<PathBuf>,
    /// The reference IDE's configuration folders to import from.
    reference: Vec<PathBuf>,
    configured: bool,
    load: Option<Task<()>>,
}

impl Global for Interpreters {}

/// The file of the application's list (set once by `main`).
static APP_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
/// One write at a time: each reads the file again before it writes.
static WRITE: Mutex<()> = Mutex::new(());

/// Runs `write` (a change of the file made off the UI thread) while no
/// other write of the list runs.
pub(super) fn exclusive<T>(write: impl FnOnce() -> T) -> T {
    let _one = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    write()
}

/// Keeps the list in `interpreters.toml` of the configuration folder.
/// Called by the application before it starts; without it the list stays
/// in memory.
pub fn keep_in_config_dir() {
    APP_FILE.get_or_init(|| settings::config_dir().map(|d| d.join(table::FILE)));
}

/// Keeps the list of this application in `file`, imported from the
/// reference configuration folders `reference` (tests).
#[cfg(test)]
pub(crate) fn set_file(cx: &mut App, file: PathBuf, reference: Vec<PathBuf>) {
    let list = cx.default_global::<Interpreters>();
    list.file = Some(file);
    list.reference = reference;
    list.configured = true;
}

/// Takes the application's file the first time (a global changed
/// notifies its observers: only when there is something to change).
fn configure(cx: &mut App) {
    if cx
        .try_global::<Interpreters>()
        .is_some_and(|l| l.configured)
    {
        return;
    }
    let list = cx.default_global::<Interpreters>();
    list.configured = true;
    if let Some(Some(file)) = APP_FILE.get() {
        list.file = Some(file.clone());
        list.reference = super::pycharm_config_dirs();
    }
}

/// The list as it is now (empty until it is read).
pub(crate) fn table(cx: &App) -> Arc<Table> {
    cx.try_global::<Interpreters>()
        .map(|l| l.table.clone())
        .unwrap_or_default()
}

/// The list has been read.
pub(crate) fn is_loaded(cx: &App) -> bool {
    cx.try_global::<Interpreters>().is_some_and(|l| l.loaded)
}

/// Reads the list in the background unless it is read or being read.
pub(crate) fn ensure_loaded(cx: &mut App) {
    configure(cx);
    if cx
        .try_global::<Interpreters>()
        .is_some_and(|l| l.loaded || l.load.is_some())
    {
        return;
    }
    read(false, cx);
}

/// Adds the reference IDE's interpreters to a list file that does not
/// exist yet, once, for people who never saw the first-run window: their
/// configuration has no `[import]` record. With a record (skipped, or
/// imported: the importer brings the interpreters from the snapshot) none
/// come from here. Called when the main window opens, so the first-run
/// window has decided by then.
pub(crate) fn import_reference(cx: &mut App) {
    if !settings_ui::first_run::undecided(cx) {
        return;
    }
    configure(cx);
    let list = cx.default_global::<Interpreters>();
    if list.file.is_none() || list.reference.is_empty() {
        return;
    }
    read(true, cx);
    cx.default_global::<Interpreters>().reference.clear();
}

/// Reads the list again after a file written elsewhere (the import from
/// the snapshot of the reference's configuration): whatever wrote the
/// file decided what it holds.
pub(crate) fn reload(cx: &mut App) {
    configure(cx);
    read(false, cx);
}

/// Starts reading the list; `import` adds the reference IDE's interpreters
/// to a file that does not exist yet ([`import_reference`]).
fn read(import: bool, cx: &mut App) {
    let list = cx.default_global::<Interpreters>();
    let Some(file) = list.file.clone() else {
        list.loaded = true;
        return;
    };
    let reference = if import {
        list.reference.clone()
    } else {
        Vec::new()
    };
    let generation = list.generation;
    let read = cx.background_spawn(async move { table::load_or_import(&file, &reference) });
    let task = cx.spawn(async move |cx| {
        let read = read.await;
        let _ = cx.update(|cx| {
            let list = cx.default_global::<Interpreters>();
            list.load = None;
            list.loaded = true;
            match read {
                Ok(table) if list.generation == generation => list.table = Arc::new(table),
                Ok(_) => {}
                Err(e) => eprintln!("interpreters: {e}"),
            }
        });
    });
    cx.default_global::<Interpreters>().load = Some(task);
}

/// Changes the list: at once in memory (the views follow), then in the
/// file in the background. Resolves to the list as written, or the error
/// that kept the change from being made.
pub(crate) fn change(changes: Vec<Change>, cx: &mut App) -> Task<Result<Arc<Table>, String>> {
    configure(cx);
    let list = cx.default_global::<Interpreters>();
    let mut table = (*list.table).clone();
    for change in &changes {
        if let Err(e) = table.apply(change) {
            return Task::ready(Err(e));
        }
    }
    let table = Arc::new(table);
    list.table = table.clone();
    list.generation += 1;
    let Some(file) = list.file.clone() else {
        return Task::ready(Ok(table));
    };
    let write = cx.background_spawn(async move { exclusive(|| table::save(&file, &changes)) });
    cx.spawn(async move |cx| {
        let written = write.await.map(Arc::new);
        match &written {
            Ok(table) => {
                let table = table.clone();
                let _ = cx.update(|cx| {
                    let list = cx.default_global::<Interpreters>();
                    list.table = table;
                    list.loaded = true;
                });
            }
            Err(e) => eprintln!("interpreters: {e}"),
        }
        written
    })
}
