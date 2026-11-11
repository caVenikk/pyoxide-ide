//! The metadata models SQL files resolve through (ADR 0021, 0022): the live
//! model of a data source as the Database tool window holds it, else the
//! model cached on disk, which is read on a thread of its own the first
//! time a file asks for it. Asking never waits: the answer is `None` until
//! a model is there, and the hub's listeners hear when it arrives.
//!
//! A data source with no cached model is reported once per run to the
//! handler the hub holds, which loads the model in the background when that
//! needs no password; else the file's toolbar offers to connect.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use db_core::DataSourceId;
use db_model::Model;

use super::hub::Changes;
use crate::store::DataSourceStore;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    root: PathBuf,
    source: String,
}

impl Key {
    fn new(root: &Path, source: &str) -> Self {
        Self {
            root: root.to_owned(),
            source: source.to_owned(),
        }
    }
}

enum Entry {
    /// The cache is being read.
    Loading,
    /// There is no cache (or it cannot be read); a model arrives when the
    /// tool window loads one.
    Absent,
    Ready(Arc<Model>),
}

type Entries = Arc<Mutex<HashMap<Key, Entry>>>;

/// Told once for each data source whose model is not cached: the project's
/// folder and the data source's name. Called from the thread that read the
/// cache.
pub(crate) type Absent = Arc<dyn Fn(&Path, &str) + Send + Sync>;

fn lock(entries: &Entries) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
    entries.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Models {
    store: Arc<dyn DataSourceStore>,
    changes: Arc<Changes>,
    entries: Entries,
    absent: Arc<Mutex<Option<Absent>>>,
    /// Data sources whose model can only be had by connecting with a
    /// password the person types.
    needs_connect: Mutex<HashSet<Key>>,
}

impl Models {
    pub(crate) fn new(store: Arc<dyn DataSourceStore>, changes: Arc<Changes>) -> Self {
        Self {
            store,
            changes,
            entries: Arc::default(),
            absent: Arc::default(),
            needs_connect: Mutex::default(),
        }
    }

    /// Who is told of a data source without a cached model.
    pub(crate) fn on_absent(&self, handler: Absent) {
        *self.absent.lock().unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    /// Says that `source` cannot be connected to without a typed password
    /// (or that it can again); the listeners hear of a change.
    pub(crate) fn set_needs_connect(&self, root: &Path, source: &str, needs: bool) {
        let changed = {
            let mut hints = self
                .needs_connect
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let key = Key::new(root, source);
            if needs {
                hints.insert(key)
            } else {
                hints.remove(&key)
            }
        };
        if changed {
            self.changes.bump();
        }
    }

    /// The model of `source` is wanted and only a connection can bring it:
    /// it has none yet and needs a typed password for one.
    pub(crate) fn connect_hint(&self, root: &Path, source: &str) -> bool {
        let key = Key::new(root, source);
        let flagged = self
            .needs_connect
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&key);
        flagged && !matches!(lock(&self.entries).get(&key), Some(Entry::Ready(_)))
    }

    /// The model of `source`; the first ask starts reading the cache.
    pub(crate) fn get(&self, root: &Path, source: &str) -> Option<Arc<Model>> {
        let key = Key::new(root, source);
        {
            let mut entries = lock(&self.entries);
            match entries.get(&key) {
                Some(Entry::Ready(model)) => return Some(model.clone()),
                Some(Entry::Loading | Entry::Absent) => return None,
                None => {}
            }
            entries.insert(key.clone(), Entry::Loading);
        }
        self.load_cache(key);
        None
    }

    fn load_cache(&self, key: Key) {
        let store = self.store.clone();
        let entries = self.entries.clone();
        let changes = self.changes.clone();
        let absent = self.absent.clone();
        let on_thread = key.clone();
        let spawned = thread::Builder::new()
            .name("sql-model-cache".into())
            .spawn(move || {
                let key = on_thread;
                let model = store
                    .model_cache(&key.root)
                    .and_then(|cache| cache.load(&DataSourceId::from(key.source.as_str())).ok())
                    .filter(|m| !m.is_empty());
                let (ready, missing) = {
                    let mut entries = lock(&entries);
                    match (entries.get(&key), model) {
                        // A live model that arrived first wins.
                        (Some(Entry::Ready(_)), _) => (false, false),
                        (_, Some(model)) => {
                            entries.insert(key.clone(), Entry::Ready(Arc::new(model)));
                            (true, false)
                        }
                        (_, None) => {
                            entries.insert(key.clone(), Entry::Absent);
                            (false, true)
                        }
                    }
                };
                if ready {
                    changes.bump();
                }
                if missing {
                    let handler = absent
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    if let Some(handler) = handler {
                        handler(&key.root, &key.source);
                    }
                }
            });
        if spawned.is_err() {
            // No thread: a live model fills the entry later.
            lock(&self.entries).insert(key, Entry::Absent);
        }
    }

    /// Replaces the model of `source` with a live one.
    pub(crate) fn publish(&self, root: &Path, source: &str, model: Arc<Model>) {
        let changed = {
            let mut entries = lock(&self.entries);
            let key = Key::new(root, source);
            let same =
                matches!(entries.get(&key), Some(Entry::Ready(old)) if Arc::ptr_eq(old, &model));
            if !same {
                entries.insert(key, Entry::Ready(model));
            }
            !same
        };
        if changed {
            self.needs_connect
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&Key::new(root, source));
            self.changes.bump();
        }
    }
}
