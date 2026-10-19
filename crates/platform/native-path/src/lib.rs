//! Paths as the system's tools take them and users read them.
//!
//! `std::fs::canonicalize` returns the verbatim form on Windows
//! (`\\?\C:\work`, `\\?\UNC\srv\share\work`). Tools do not read it (git takes
//! `\\?\C:\...` for a host name) and users must not see it, yet paths derived
//! from a canonical project root reach the UI, run configurations and the
//! interpreter fields. [`canonicalize`] is the only way to canonicalize in the
//! workspace (`clippy.toml` bans `std::fs::canonicalize`, `Path::canonicalize`
//! and `dunce::canonicalize`): it gives the plain form (`C:\work`,
//! `\\srv\share\work`) whenever one exists, so the paths of one project are
//! compared in one form.

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

/// `path` made absolute with links, `.` and `..` resolved, as
/// `std::fs::canonicalize` does, but on Windows without the verbatim prefix.
///
/// A path that cannot be written in the plain form keeps the verbatim one (a
/// reserved device name such as `NUL` as a component, or a path longer than
/// the legacy limit).
#[allow(
    clippy::disallowed_methods,
    reason = "the helper that `clippy.toml` points to"
)]
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let canonical = dunce::canonicalize(path)?;
    Ok(match simplified(&canonical) {
        Cow::Borrowed(_) => canonical,
        Cow::Owned(plain) => plain,
    })
}

/// `path` without a Windows verbatim prefix when the plain form means the
/// same: `\\?\C:\x` is `C:\x`, `\\?\UNC\srv\share\x` is `\\srv\share\x`. Any
/// other path, and every path on other systems, is returned as it is. No file
/// system access.
pub fn simplified(path: &Path) -> Cow<'_, Path> {
    // The drive form, when it is safe to strip it; a no-op off Windows.
    let path = dunce::simplified(path);
    if cfg!(windows)
        && let Some(plain) = path.to_str().and_then(unc_to_plain)
    {
        return Cow::Owned(PathBuf::from(plain));
    }
    Cow::Borrowed(path)
}

/// `\\?\UNC\srv\share\x` as `\\srv\share\x` (`dunce` leaves the network
/// form as it is).
fn unc_to_plain(path: &str) -> Option<String> {
    path.strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verbatim_network_share_becomes_a_plain_one() {
        assert_eq!(
            unc_to_plain(r"\\?\UNC\srv\share\a\b").as_deref(),
            Some(r"\\srv\share\a\b")
        );
        assert_eq!(
            unc_to_plain(r"\\?\UNC\srv\share").as_deref(),
            Some(r"\\srv\share")
        );
    }

    #[test]
    fn other_forms_are_not_a_verbatim_network_share() {
        for path in [
            r"\\?\C:\a\b",
            r"\\srv\share\a",
            r"C:\a\b",
            "/home/a/b",
            "relative/path",
            "",
        ] {
            assert_eq!(unc_to_plain(path), None, "{path}");
        }
    }

    #[test]
    fn an_ordinary_path_is_unchanged() {
        for path in [Path::new("/home/a/b"), Path::new("rel/a"), Path::new("")] {
            assert_eq!(simplified(path), path);
        }
        #[cfg(windows)]
        assert_eq!(simplified(Path::new(r"C:\a\b")), Path::new(r"C:\a\b"));
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_drive_path_is_shown_with_the_drive() {
        assert_eq!(simplified(Path::new(r"\\?\C:\a\b")), Path::new(r"C:\a\b"));
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_network_path_is_shown_as_a_network_path() {
        assert_eq!(
            simplified(Path::new(r"\\?\UNC\srv\share\a\b")),
            Path::new(r"\\srv\share\a\b")
        );
    }

    #[test]
    fn the_canonical_path_resolves_dots_and_has_no_verbatim_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("a").join("b");
        std::fs::create_dir_all(&inner).unwrap();
        let canonical = canonicalize(inner.join("..").join("b")).unwrap();
        assert_eq!(canonical, canonicalize(&inner).unwrap());
        assert!(
            canonical.ends_with(Path::new("a").join("b")),
            "{canonical:?}"
        );
        assert!(
            !canonical.to_string_lossy().starts_with(r"\\?\"),
            "{canonical:?}"
        );
    }

    #[test]
    fn a_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = canonicalize(dir.path().join("missing")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(canonicalize(&link).unwrap(), canonicalize(&real).unwrap());
    }
}
