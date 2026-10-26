//! The debugger helper: debugpy 1.8.21, the contents of its pure-Python
//! wheel unpacked unmodified into a folder (ADR 0015). Release builds ship
//! it in the macOS bundle's `Resources/helpers/debugpy`, next to the binary
//! (`helpers/debugpy`) on Windows and Linux; development builds fetch it
//! once into the application cache with `scripts/fetch-debugpy.py`
//! (`helpers/debugpy-1.8.21`). The application never downloads it by
//! itself and never takes debugpy from the project's environment.

use std::fmt;
use std::path::{Path, PathBuf};

/// The debugpy version the IDE ships and expects (the reference's).
pub const DEBUGPY_VERSION: &str = "1.8.21";

/// The folder of the helper in a bundle or next to the binary.
const BUNDLED: &str = "debugpy";

/// The script that fetches the helper, for the message when it is missing.
pub const FETCH_SCRIPT: &str = "scripts/fetch-debugpy.py";

/// A helper folder: `debugpy/` and the wheel's `debugpy-1.8.21.dist-info/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    root: PathBuf,
}

impl Helper {
    /// The helper in `root` when it holds debugpy of the expected version.
    /// Looks at the file system.
    pub fn at(root: impl Into<PathBuf>) -> Option<Helper> {
        let root = root.into();
        let adapter = root.join("debugpy").join("adapter").join("__main__.py");
        let dist_info = root
            .join(format!("debugpy-{DEBUGPY_VERSION}.dist-info"))
            .join("RECORD");
        (adapter.is_file() && dist_info.is_file()).then_some(Helper { root })
    }

    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `debugpy` package.
    pub fn package(&self) -> PathBuf {
        self.root.join("debugpy")
    }

    /// The adapter, run as `python <adapter>` (a folder with `__main__.py`
    /// that puts the helper folder on `sys.path` itself).
    pub fn adapter(&self) -> PathBuf {
        self.package().join("adapter")
    }

    /// The first helper of [`candidates`] for the running executable and
    /// the application cache.
    pub fn locate() -> Result<Helper, HelperMissing> {
        let exe = std::env::current_exe().ok();
        Self::locate_in(&candidates(exe.as_deref(), cache_dir().as_deref()))
    }

    /// The first of `candidates` that holds the helper.
    pub fn locate_in(candidates: &[PathBuf]) -> Result<Helper, HelperMissing> {
        candidates
            .iter()
            .find_map(Helper::at)
            .ok_or_else(|| HelperMissing {
                searched: candidates.to_vec(),
            })
    }
}

/// Where the helper is looked for, in order: the macOS bundle's
/// `Contents/Resources/helpers/debugpy` (for an executable in
/// `Contents/MacOS`), `helpers/debugpy` next to the executable, then
/// `helpers/debugpy-1.8.21` in the application cache.
pub fn candidates(exe: Option<&Path>, cache: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(bin) = exe.and_then(Path::parent) {
        if cfg!(target_os = "macos")
            && bin.file_name().is_some_and(|n| n == "MacOS")
            && let Some(contents) = bin.parent()
        {
            dirs.push(contents.join("Resources").join("helpers").join(BUNDLED));
        }
        dirs.push(bin.join("helpers").join(BUNDLED));
    }
    if let Some(cache) = cache {
        dirs.push(cache_helper(cache));
    }
    dirs
}

/// The helper folder in an application cache folder.
pub fn cache_helper(cache: &Path) -> PathBuf {
    cache
        .join("helpers")
        .join(format!("debugpy-{DEBUGPY_VERSION}"))
}

/// The application's cache folder (`app_info::cache_dir`).
/// `scripts/fetch-debugpy.py` computes the same.
pub fn cache_dir() -> Option<PathBuf> {
    app_info::cache_dir()
}

/// The helper was not found: the message names the folders looked at and
/// how a development build gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperMissing {
    pub searched: Vec<PathBuf>,
}

impl fmt::Display for HelperMissing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cannot debug: the debugger helper (debugpy {DEBUGPY_VERSION}) was not found"
        )?;
        if !self.searched.is_empty() {
            f.write_str(". Looked in:")?;
            for dir in &self.searched {
                write!(f, "\n  {}", dir.display())?;
            }
        }
        write!(
            f,
            "\nA release build ships it; reinstall the application. A development build \
             fetches it once with `python3 {FETCH_SCRIPT}`."
        )
    }
}

impl std::error::Error for HelperMissing {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_helper(root: &Path) {
        let adapter = root.join("debugpy").join("adapter");
        std::fs::create_dir_all(&adapter).unwrap();
        std::fs::write(adapter.join("__main__.py"), "").unwrap();
        let info = root.join(format!("debugpy-{DEBUGPY_VERSION}.dist-info"));
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("RECORD"), "").unwrap();
    }

    #[test]
    fn a_helper_needs_the_adapter_and_the_version() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Helper::at(dir.path()), None);
        fake_helper(dir.path());
        let helper = Helper::at(dir.path()).unwrap();
        assert_eq!(helper.adapter(), dir.path().join("debugpy").join("adapter"));
        // Another version is not this helper.
        std::fs::rename(
            dir.path()
                .join(format!("debugpy-{DEBUGPY_VERSION}.dist-info")),
            dir.path().join("debugpy-1.8.20.dist-info"),
        )
        .unwrap();
        assert_eq!(Helper::at(dir.path()), None);
    }

    #[test]
    fn the_bundle_comes_before_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Pyoxide.app").join("Contents");
        let exe = app.join("MacOS").join("pyoxide-ide");
        let cache = dir.path().join("cache");
        let found = candidates(Some(&exe), Some(&cache));
        let mut expected = Vec::new();
        if cfg!(target_os = "macos") {
            expected.push(app.join("Resources").join("helpers").join("debugpy"));
        }
        expected.push(app.join("MacOS").join("helpers").join("debugpy"));
        expected.push(cache.join("helpers").join("debugpy-1.8.21"));
        assert_eq!(found, expected);

        // Only the cache holds it.
        fake_helper(&cache_helper(&cache));
        let helper = Helper::locate_in(&found).unwrap();
        assert_eq!(helper.root(), cache_helper(&cache));
        // Next to the binary wins over the cache.
        let next = app.join("MacOS").join("helpers").join("debugpy");
        fake_helper(&next);
        assert_eq!(Helper::locate_in(&found).unwrap().root(), next);
    }

    #[test]
    fn a_missing_helper_says_where_it_looked_and_what_to_run() {
        let dir = tempfile::tempdir().unwrap();
        let searched = candidates(Some(&dir.path().join("pyoxide-ide")), Some(dir.path()));
        let missing = Helper::locate_in(&searched).unwrap_err();
        let message = missing.to_string();
        assert!(message.starts_with(
            "Cannot debug: the debugger helper (debugpy 1.8.21) was not found. Looked in:"
        ));
        for dir in &searched {
            assert!(message.contains(&dir.display().to_string()), "{message}");
        }
        assert!(
            message.contains("python3 scripts/fetch-debugpy.py"),
            "{message}"
        );
    }
}
