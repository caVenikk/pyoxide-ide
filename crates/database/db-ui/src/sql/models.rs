//! The metadata models SQL files resolve through (ADR 0021, 0022): the live
//! model of a data source as the Database tool window holds it, else the
//! model cached on disk, which is read on a thread of its own the first
//! time a file asks for it. Asking never waits: the answer is `None` until
//! a model is there, and the hub's listeners hear when it arrives.

use std::collections::HashMap;
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

fn lock(entries: &Entries) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
    entries.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Models {
    store: Arc<dyn DataSourceStore>,
    changes: Arc<Changes>,
    entries: Entries,
}

impl Models {
    pub(crate) fn new(store: Arc<dyn DataSourceStore>, changes: Arc<Changes>) -> Self {
        Self {
            store,
            changes,
            entries: Arc::default(),
        }
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
        let on_thread = key.clone();
        let spawned = thread::Builder::new()
            .name("sql-model-cache".into())
            .spawn(move || {
                let key = on_thread;
                let model = store
                    .model_cache(&key.root)
                    .and_then(|cache| cache.load(&DataSourceId::from(key.source.as_str())).ok())
                    .filter(|m| !m.is_empty());
                let ready = {
                    let mut entries = lock(&entries);
                    match (entries.get(&key), model) {
                        // A live model that arrived first wins.
                        (Some(Entry::Ready(_)), _) => false,
                        (_, Some(model)) => {
                            entries.insert(key, Entry::Ready(Arc::new(model)));
                            true
                        }
                        (_, None) => {
                            entries.insert(key, Entry::Absent);
                            false
                        }
                    }
                };
                if ready {
                    changes.bump();
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
            self.changes.bump();
        }
    }
}
