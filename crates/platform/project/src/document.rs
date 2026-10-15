//! Reading and writing documents (blocking; run off the UI thread).

use std::io::Write as _;
use std::path::Path;
use std::time::SystemTime;

use text::{DecodeError, Decoded, decode};

/// Files larger than this are not opened in the editor.
pub const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Io(String),
    #[error("the file looks binary")]
    Binary,
    #[error("the file is too large to open ({0} bytes)")]
    TooLarge(u64),
}

/// A document read from disk.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub decoded: Decoded,
    /// The file cannot be written (permissions).
    pub read_only: bool,
    /// Size and modification time of the file before it was read.
    pub stamp: Stamp,
}

/// What changes when a file is written: its size and modification time.
/// Two equal stamps mean the file is very likely unchanged (a write of the
/// same size within the file system's time resolution is not seen); a
/// different stamp means it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl Stamp {
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }

    /// The stamp of a file (blocking); `None` when it cannot be read.
    pub fn of(path: &Path) -> Option<Self> {
        std::fs::metadata(path)
            .ok()
            .map(|meta| Self::from_metadata(&meta))
    }
}

/// The marker in the names of the temporary files [`write_document`]
/// writes before it renames them over the document.
const SAVE_MARKER: &str = ".pyoxide-save-";

/// A temporary file of [`write_document`] (by its name): a file watcher
/// leaves these out.
pub fn is_save_temp(name: &str) -> bool {
    name.starts_with('.') && name.contains(SAVE_MARKER)
}

/// Reads and decodes a file.
pub fn read_document(path: &Path) -> Result<Loaded, LoadError> {
    let meta = std::fs::metadata(path).map_err(|e| LoadError::Io(e.to_string()))?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(LoadError::TooLarge(meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| LoadError::Io(e.to_string()))?;
    let decoded = decode(&bytes).map_err(|e| match e {
        DecodeError::Binary => LoadError::Binary,
    })?;
    Ok(Loaded {
        decoded,
        read_only: meta.permissions().readonly(),
        stamp: Stamp::from_metadata(&meta),
    })
}

/// Writes a document: to a temporary file next to it, then renamed over it,
/// so a failed write never leaves a truncated file. Permissions of an
/// existing file are kept.
pub fn write_document(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile_in(dir, path)?;
    let result = (|| {
        temp.1.write_all(bytes)?;
        temp.1.sync_all()?;
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp.0, meta.permissions())?;
        }
        std::fs::rename(&temp.0, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp.0);
    }
    result
}

/// Creates a new temporary file in `dir` named after `path`.
fn tempfile_in(dir: &Path, path: &Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for attempt in 0..100u32 {
        let candidate = dir.join(format!(
            ".{stem}{SAVE_MARKER}{}-{attempt}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("cannot create a temporary file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_writes_and_refuses_binary_and_huge_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.py");
        std::fs::write(&file, b"x = 1\r\n").unwrap();
        let loaded = read_document(&file).unwrap();
        assert_eq!(loaded.decoded.text, "x = 1\n");
        assert!(!loaded.read_only);

        write_document(&file, b"y = 2\r\n").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"y = 2\r\n");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temporary files remain");
        assert_eq!(
            read_document(&file).unwrap().stamp,
            Stamp::of(&file).unwrap()
        );
        assert_eq!(Stamp::of(&file).unwrap().len, 7);
        assert_eq!(Stamp::of(&dir.path().join("missing")), None);

        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, b"\0\x01\x02").unwrap();
        assert_eq!(read_document(&bin).unwrap_err(), LoadError::Binary);

        let big = dir.path().join("big.txt");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_FILE_SIZE + 1).unwrap();
        assert!(matches!(read_document(&big), Err(LoadError::TooLarge(_))));
        assert!(matches!(
            read_document(&dir.path().join("missing")),
            Err(LoadError::Io(_))
        ));
    }

    #[test]
    fn temporary_save_files_are_known_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        let (temp, _) = tempfile_in(dir.path(), &path).unwrap();
        let name = temp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(is_save_temp(&name), "{name}");
        assert!(!is_save_temp("a.py"));
        assert!(!is_save_temp(".hidden.py"));
    }

    #[cfg(unix)]
    #[test]
    fn keeps_permissions_and_reports_read_only_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("run.sh");
        std::fs::write(&file, b"echo\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_document(&file, b"echo hi\n").unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(read_document(&file).unwrap().read_only);
    }
}
