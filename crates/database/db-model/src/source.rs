//! Source texts of views, routines, packages, types and triggers: kept out
//! of the model (ADR 0021), filled from passes at the full level or read one
//! at a time on request, kept in memory and in files beside the cached
//! model, read back lazily.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use db_core::schema::{Delta, Level, ObjectKind, Snapshot};
use serde::{Deserialize, Serialize};

use crate::cache::write_atomically;
use crate::id::{StableHasher, schema_hash};
use crate::schema::NodeRef;

/// Which object a text belongs to: the identity a pass reports it by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceKey {
    pub schema: String,
    pub kind: ObjectKind,
    pub name: String,
    /// The identity arguments of an overloaded routine; empty otherwise.
    #[serde(default)]
    pub signature: String,
}

impl SourceKey {
    pub fn new(
        schema: impl Into<String>,
        kind: ObjectKind,
        name: impl Into<String>,
        signature: impl Into<String>,
    ) -> Self {
        Self {
            schema: schema.into(),
            kind,
            name: name.into(),
            signature: signature.into(),
        }
    }

    /// The key of a schema object of the model; `None` for columns, keys
    /// and other parts.
    pub fn of(node: NodeRef<'_>) -> Option<Self> {
        let object = node.object()?;
        Some(Self::new(
            node.schema().name(),
            object.kind,
            object.name.as_str(),
            node.signature(),
        ))
    }

    /// `<schema>-<object>-<signature>.json` in hex: the files of a schema,
    /// and of all overloads of a routine, share a prefix.
    fn file_name(&self) -> String {
        format!(
            "{}-{:016x}.json",
            object_prefix(&self.schema, self.kind, &self.name),
            hash(&[&self.signature])
        )
    }
}

fn hash(parts: &[&str]) -> u64 {
    let mut h = StableHasher::new();
    for p in parts {
        h.segment(p);
    }
    h.finish()
}

fn schema_prefix(schema: &str) -> String {
    format!("{:016x}", schema_hash(schema))
}

fn object_prefix(schema: &str, kind: ObjectKind, name: &str) -> String {
    let tag = crate::kind::NodeKind::from_object(kind).tag();
    format!("{}-{:016x}", schema_prefix(schema), hash(&[tag, name]))
}

#[derive(Serialize, Deserialize)]
struct SourceFile {
    #[serde(flatten)]
    key: SourceKey,
    text: String,
}

/// Source texts by object. Thread-safe; reads from disk only what is asked
/// for.
#[derive(Debug, Default)]
pub struct SourceStore {
    dir: Option<PathBuf>,
    memory: Mutex<HashMap<SourceKey, Arc<str>>>,
}

impl SourceStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Texts in files under `dir` (created on the first write).
    pub fn on_disk(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: Some(dir.into()),
            memory: Mutex::default(),
        }
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, HashMap<SourceKey, Arc<str>>> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The text of `key`, from memory or from its file; `None` when it was
    /// never stored (the caller reads it from the database and
    /// [`put`](Self::put)s it).
    pub fn get(&self, key: &SourceKey) -> io::Result<Option<Arc<str>>> {
        if let Some(text) = self.memory().get(key) {
            return Ok(Some(text.clone()));
        }
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let bytes = match fs::read(dir.join(key.file_name())) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        // A damaged file, or another object with the same hash, is a miss.
        let Ok(file) = serde_json::from_slice::<SourceFile>(&bytes) else {
            return Ok(None);
        };
        if file.key != *key {
            return Ok(None);
        }
        let text: Arc<str> = file.text.into();
        self.memory().insert(key.clone(), text.clone());
        Ok(Some(text))
    }

    pub fn put(&self, key: SourceKey, text: impl Into<Arc<str>>) -> io::Result<()> {
        let text = text.into();
        if let Some(dir) = &self.dir {
            fs::create_dir_all(dir)?;
            let file = SourceFile {
                key: key.clone(),
                text: text.to_string(),
            };
            write_atomically(&dir.join(key.file_name()), |w| {
                serde_json::to_writer(w, &file).map_err(io::Error::other)
            })?;
        }
        self.memory().insert(key, text);
        Ok(())
    }

    pub fn remove(&self, key: &SourceKey) -> io::Result<()> {
        self.memory().remove(key);
        self.remove_files(&key.file_name())
    }

    /// Every text of `schema`.
    pub fn remove_schema(&self, schema: &str) -> io::Result<()> {
        self.memory().retain(|k, _| k.schema != schema);
        self.remove_files(&format!("{}-", schema_prefix(schema)))
    }

    /// The texts of every overload of a routine (or of the one object of
    /// this kind and name).
    fn remove_object(&self, schema: &str, kind: ObjectKind, name: &str) -> io::Result<()> {
        self.memory()
            .retain(|k, _| !(k.schema == schema && k.kind == kind && k.name == name));
        self.remove_files(&format!("{}-", object_prefix(schema, kind, name)))
    }

    /// Removes the files whose names start with `prefix`.
    fn remove_files(&self, prefix: &str) -> io::Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(prefix))
            {
                match fs::remove_file(entry.path()) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Takes the texts of a pass: for every schema read at
    /// [`Level::Full`], the stored texts are replaced by the pass's. Call it
    /// before giving the snapshot to the model.
    pub fn absorb_snapshot(&self, snapshot: &Snapshot) -> io::Result<()> {
        for schema in snapshot.schemas.iter().filter(|s| s.level >= Level::Full) {
            self.remove_schema(&schema.info.name)?;
            for source in &schema.sources {
                let key = SourceKey::new(
                    schema.info.name.as_str(),
                    source.kind,
                    source.name.as_str(),
                    source.signature.as_str(),
                );
                self.put(key, source.text.as_str())?;
            }
        }
        Ok(())
    }

    /// Takes the texts of a delta: texts of dropped and changed objects and
    /// of dropped schemas go, the delta's texts come in.
    pub fn absorb_delta(&self, delta: &Delta) -> io::Result<()> {
        for schema in &delta.dropped_schemas {
            self.remove_schema(schema)?;
        }
        for schema in &delta.schemas {
            let name = schema.info.name.as_str();
            for dropped in &schema.dropped {
                self.remove_object(name, dropped.kind, &dropped.name)?;
            }
            for object in &schema.objects {
                let key = SourceKey::new(
                    name,
                    object.kind,
                    object.name.as_str(),
                    match &object.detail {
                        Some(db_core::schema::ObjectDetail::Routine(r)) => r.signature.as_str(),
                        _ => "",
                    },
                );
                self.remove(&key)?;
            }
            for source in &schema.sources {
                let key = SourceKey::new(
                    name,
                    source.kind,
                    source.name.as_str(),
                    source.signature.as_str(),
                );
                self.put(key, source.text.as_str())?;
            }
        }
        Ok(())
    }
}
