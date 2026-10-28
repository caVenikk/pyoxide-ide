//! The model on disk (ADR 0021): one versioned JSON file per data source in
//! the user cache folder, written to a temporary file and renamed, read
//! before any connection exists. A file of another version, of another data
//! source or that does not parse is ignored; the next store replaces it.

use std::fmt;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use db_core::DataSourceId;
use db_core::schema::{ChangeToken, SchemaSnapshot};
use serde::{Deserialize, Serialize};

use crate::model::Model;
use crate::schema::Cancelled;
use crate::source::SourceStore;

/// The `format` field of a cache file: `<slug>-db-model`.
pub fn cache_format() -> String {
    format!("{}-db-model", app_info::slug())
}

/// The version of the cache file. Raise it with every change of the file's
/// shape or of what the model derives from it (node ids included): older
/// files are then ignored and rebuilt.
pub const CACHE_VERSION: u32 = 1;

const MODEL_FILE: &str = "model.json";
const SOURCES_DIR: &str = "sources";

/// What a cache file holds: the data source id, the change token and the
/// schemas in the model's canonical form, nothing about the connection (no
/// host, user or secret).
#[derive(Serialize)]
struct FileOut<'a> {
    format: &'a str,
    version: u32,
    data_source: &'a DataSourceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<&'a ChangeToken>,
    schemas: Vec<SchemaSnapshot>,
}

#[derive(Deserialize)]
struct FileIn {
    format: String,
    version: u32,
    data_source: DataSourceId,
    #[serde(default)]
    token: Option<ChangeToken>,
    schemas: Vec<SchemaSnapshot>,
}

#[derive(Deserialize)]
struct Header {
    format: String,
    version: u32,
}

/// Why [`ModelCache::load`] gave no model. Each case means "rebuild".
#[derive(Debug)]
pub enum CacheMiss {
    /// No file: the data source was never loaded on this machine.
    Missing,
    /// The file exists but cannot be read.
    Unreadable(io::Error),
    /// Not a cache file, or a damaged one.
    Corrupt(String),
    /// Written by another version of the format.
    Version(u32),
    /// The file belongs to another data source (two ids that map to one
    /// folder on a file system that ignores case).
    OtherDataSource,
    Cancelled,
}

impl fmt::Display for CacheMiss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheMiss::Missing => f.write_str("no cached model"),
            CacheMiss::Unreadable(e) => write!(f, "the cached model cannot be read: {e}"),
            CacheMiss::Corrupt(e) => write!(f, "the cached model is damaged: {e}"),
            CacheMiss::Version(v) => write!(
                f,
                "the cached model has format version {v}, this build reads {CACHE_VERSION}"
            ),
            CacheMiss::OtherDataSource => {
                f.write_str("the cached model belongs to another data source")
            }
            CacheMiss::Cancelled => f.write_str("loading the cached model was cancelled"),
        }
    }
}

impl std::error::Error for CacheMiss {}

/// The cache folder of the database module: `<root>/<data source>/model.json`
/// and the source texts beside it in `sources/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `db` in the application's cache folder (`app_info::cache_dir`: the
    /// product's `CACHE_DIR` environment variable, else its slug in the
    /// user's cache folder). Never the project folder.
    pub fn user() -> Option<Self> {
        Some(Self::new(app_info::cache_dir()?.join("db")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder of one data source.
    pub fn dir(&self, data_source: &DataSourceId) -> PathBuf {
        self.root.join(folder_name(data_source.as_str()))
    }

    pub fn model_path(&self, data_source: &DataSourceId) -> PathBuf {
        self.dir(data_source).join(MODEL_FILE)
    }

    /// The lazily filled store of source texts of one data source.
    pub fn sources(&self, data_source: &DataSourceId) -> SourceStore {
        SourceStore::on_disk(self.dir(data_source).join(SOURCES_DIR))
    }

    /// The cached model of `data_source`. Never panics on a bad file.
    pub fn load(&self, data_source: &DataSourceId) -> Result<Model, CacheMiss> {
        self.load_cancellable(data_source, &|| false)
    }

    pub fn load_cancellable(
        &self,
        data_source: &DataSourceId,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Model, CacheMiss> {
        let bytes = match fs::read(self.model_path(data_source)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(CacheMiss::Missing),
            Err(e) => return Err(CacheMiss::Unreadable(e)),
        };
        if cancel() {
            return Err(CacheMiss::Cancelled);
        }
        let file = match serde_json::from_slice::<FileIn>(&bytes) {
            Ok(file) => file,
            Err(e) => {
                // A file of another version may not parse as this one: say
                // which version it was.
                return Err(match serde_json::from_slice::<Header>(&bytes) {
                    Ok(h) if h.format == cache_format() && h.version != CACHE_VERSION => {
                        CacheMiss::Version(h.version)
                    }
                    _ => CacheMiss::Corrupt(e.to_string()),
                });
            }
        };
        if file.format != cache_format() {
            return Err(CacheMiss::Corrupt(format!(
                "format {:?} instead of {:?}",
                file.format,
                cache_format()
            )));
        }
        if file.version != CACHE_VERSION {
            return Err(CacheMiss::Version(file.version));
        }
        if file.data_source != *data_source {
            return Err(CacheMiss::OtherDataSource);
        }
        Model::from_parts(file.data_source, file.token, file.schemas, cancel)
            .map_err(|Cancelled| CacheMiss::Cancelled)
    }

    /// Writes `model` to its data source's file: to a temporary file in the
    /// same folder, flushed to disk, then renamed over the old one, so a
    /// crash leaves the old file or the new one, never half of one.
    pub fn store(&self, model: &Model) -> io::Result<()> {
        let dir = self.dir(model.data_source());
        fs::create_dir_all(&dir)?;
        let snapshot = model.to_snapshot();
        let format = cache_format();
        let file = FileOut {
            format: &format,
            version: CACHE_VERSION,
            data_source: model.data_source(),
            token: model.token(),
            schemas: snapshot.schemas,
        };
        write_atomically(&dir.join(MODEL_FILE), |w| {
            serde_json::to_writer(w, &file).map_err(io::Error::other)
        })
    }

    /// Forgets the cached model and source texts of `data_source` (the
    /// tree's "forget cached schemas").
    pub fn remove(&self, data_source: &DataSourceId) -> io::Result<()> {
        match fs::remove_dir_all(self.dir(data_source)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `path` through a temporary file beside it and a rename.
pub(crate) fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<fs::File>) -> io::Result<()>,
) -> io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let temp = path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut w = BufWriter::with_capacity(1 << 20, fs::File::create(&temp)?);
        write(&mut w)?;
        w.flush()?;
        w.get_ref().sync_all()?;
        drop(w);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// A folder name for a data source id: letters, digits, `-` and `_` as they
/// are, every other byte as `%XX`, so the name is valid everywhere and two
/// ids never share one (except by case on a file system that ignores it,
/// which the id stored in the file catches). Device names that Windows
/// reserves get a `%` appended.
fn folder_name(id: &str) -> String {
    if id.is_empty() {
        return "%".into();
    }
    let mut out = String::with_capacity(id.len());
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let upper = out.to_ascii_uppercase();
    let device = RESERVED.contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit());
    if device {
        out.push('%');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names() {
        assert_eq!(folder_name("reporting"), "reporting");
        assert_eq!(folder_name("prod db/eu"), "prod%20db%2Feu");
        assert_eq!(folder_name(".."), "%2E%2E");
        assert_eq!(folder_name("a%b"), "a%25b");
        assert_eq!(folder_name(""), "%");
        assert_eq!(folder_name("con"), "con%");
        assert_eq!(folder_name("COM1"), "COM1%");
        assert_eq!(folder_name("COM"), "COM");
        assert_eq!(folder_name("база"), "%D0%B1%D0%B0%D0%B7%D0%B0");
    }
}
