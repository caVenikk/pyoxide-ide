//! Reading and writing documents (blocking; run off the UI thread).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

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
/// writes before it renames them over the document:
/// `.<name>.pyoxide-save-<process id>-<n>`.
const SAVE_MARKER: &str = ".pyoxide-save-";

/// The process id in the name of a temporary file of [`write_document`]
/// (`None`: not such a name).
fn save_temp_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('.')?;
    let (_, tail) = rest.rsplit_once(SAVE_MARKER)?;
    let (pid, attempt) = tail.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(pid) && digits(attempt)).then(|| pid.parse().ok())?
}

/// A temporary file of [`write_document`] (by its name): never shown in the
/// project and left out by file watchers.
pub fn is_save_temp(name: &str) -> bool {
    save_temp_pid(name).is_some()
}

/// Removes a temporary file of [`write_document`] that a session that is
/// gone left behind (a crash, a kill): its process no longer runs. A file of
/// this process (a write in flight) or of a running one is left alone.
/// Returns whether the file was removed (blocking).
pub fn remove_stale_save_temp(path: &Path) -> bool {
    let Some(pid) = path
        .file_name()
        .and_then(|n| save_temp_pid(&n.to_string_lossy()))
    else {
        return false;
    };
    if pid == std::process::id() || process::is_alive(pid) {
        return false;
    }
    std::fs::remove_file(path).is_ok()
}

/// The writes of [`write_document`] in progress: their temporary files.
static IN_FLIGHT: (Mutex<Vec<PathBuf>>, Condvar) = (Mutex::new(Vec::new()), Condvar::new());

fn in_flight() -> MutexGuard<'static, Vec<PathBuf>> {
    IN_FLIGHT.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// For the application's exit: waits up to `timeout` for the writes of
/// [`write_document`] in progress, then removes the temporary files of the
/// ones still running (their rename then fails; the document keeps its old
/// content). Blocks the calling thread for at most `timeout`.
pub fn finish_writes(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut files = in_flight();
    while !files.is_empty() {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        files = IN_FLIGHT
            .1
            .wait_timeout(files, left)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    for path in files.drain(..) {
        let _ = std::fs::remove_file(path);
    }
}

/// A temporary file being written: removed when dropped (an error, a panic)
/// unless the write [`finished`](Self::finished), and known to
/// [`finish_writes`] meanwhile.
struct TempFile {
    path: PathBuf,
    /// Removes the file when dropped.
    armed: bool,
}

impl TempFile {
    /// Creates a new temporary file in `dir` named after `path`.
    fn create(dir: &Path, path: &Path) -> std::io::Result<(Self, std::fs::File)> {
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
                Ok(file) => {
                    in_flight().push(candidate.clone());
                    return Ok((
                        Self {
                            path: candidate,
                            armed: true,
                        },
                        file,
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other("cannot create a temporary file"))
    }

    /// The file was renamed over the document: nothing to remove.
    fn finished(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
        in_flight().retain(|p| p != &self.path);
        IN_FLIGHT.1.notify_all();
    }
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
/// existing file are kept. The temporary file is removed on every way out
/// (an error, a panic), and by [`finish_writes`] when the application exits
/// during the write.
pub fn write_document(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let (temp, mut file) = TempFile::create(dir, path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    // Closed before the rename: Windows cannot rename or remove an open file.
    drop(file);
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&temp.path, meta.permissions())?;
    }
    std::fs::rename(&temp.path, path)?;
    temp.finished();
    Ok(())
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
        let (temp, _) = TempFile::create(dir.path(), &path).unwrap();
        let name = temp
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(is_save_temp(&name), "{name}");
        assert_eq!(save_temp_pid(&name), Some(std::process::id()));
        assert!(!is_save_temp("a.py"));
        assert!(!is_save_temp(".hidden.py"));
        assert!(is_save_temp(".a.py.pyoxide-save-123-0"));
        assert!(is_save_temp("..pyoxide-save-1-12"));
        assert!(!is_save_temp("a.py.pyoxide-save-123-0"));
        assert!(!is_save_temp(".a.py.pyoxide-save-x-0"));
        assert!(!is_save_temp(".a.py.pyoxide-save-123"));
        assert!(!is_save_temp(".a.py.pyoxide-save-123-"));
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_failed_write_removes_its_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        // The temporary file is written, then the rename over a folder fails.
        let target = dir.path().join("pkg");
        std::fs::create_dir(&target).unwrap();
        assert!(write_document(&target, b"x = 1\n").is_err());
        assert_eq!(names(dir.path()), ["pkg"]);
        assert!(in_flight().is_empty());

        // The folder of the document is gone: no temporary file to leave.
        let missing = dir.path().join("gone").join("a.py");
        assert!(write_document(&missing, b"x = 1\n").is_err());
        assert_eq!(names(dir.path()), ["pkg"]);
    }

    #[test]
    fn a_temporary_file_is_removed_when_the_write_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        let result = std::panic::catch_unwind(|| {
            let (_temp, _file) = TempFile::create(path.parent().unwrap(), &path).unwrap();
            panic!("the write stops");
        });
        assert!(result.is_err());
        assert!(names(dir.path()).is_empty());
        assert!(in_flight().is_empty());
    }

    #[test]
    fn quitting_during_a_write_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        std::fs::write(&path, "old\n").unwrap();
        // A write that does not end within the time the exit waits.
        let (temp, mut file) = TempFile::create(dir.path(), &path).unwrap();
        file.write_all(b"new\n").unwrap();
        assert_eq!(names(dir.path()).len(), 2);

        let started = Instant::now();
        finish_writes(Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(names(dir.path()), ["a.py"], "the temporary file is gone");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
        drop((temp, file));
    }

    #[test]
    fn quitting_waits_for_a_write_that_ends_soon() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let writer = std::thread::spawn({
            let path = path.clone();
            move || {
                let (temp, mut file) = TempFile::create(path.parent().unwrap(), &path).unwrap();
                started_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(100));
                file.write_all(b"new\n").unwrap();
                drop(file);
                std::fs::rename(&temp.path, &path).unwrap();
                temp.finished();
            }
        });
        started_rx.recv().unwrap();
        finish_writes(Duration::from_secs(30));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(names(dir.path()), ["a.py"]);
        writer.join().unwrap();
    }

    #[test]
    fn leftovers_of_a_gone_session_are_removed_and_live_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        // Not an id of a running process (see `process::is_alive`).
        let gone = dir.path().join(".a.py.pyoxide-save-2147483632-0");
        let mine = dir
            .path()
            .join(format!(".a.py.pyoxide-save-{}-7", std::process::id()));
        let plain = dir.path().join("a.py");
        for f in [&gone, &mine, &plain] {
            std::fs::write(f, "x").unwrap();
        }
        assert!(remove_stale_save_temp(&gone));
        assert!(!remove_stale_save_temp(&mine), "a write of this process");
        assert!(!remove_stale_save_temp(&plain));
        assert!(!gone.exists() && mine.exists() && plain.exists());
        assert!(!remove_stale_save_temp(&gone), "already gone");
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
