//! The identity of the running product, as the platform sees it (ADR 0005).
//!
//! Platform crates know no product name. The product installs an [`AppInfo`]
//! once at startup ([`install`]); until then (in the tests of a platform
//! crate, for example) [`get`] returns a neutral one. Names that users see
//! are formatted from [`name`], technical names (configuration and cache
//! folders, keychain services, environment variables, the project folder)
//! from [`slug`], [`env_prefix`] and [`project_dir`].

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The names of a product. Every field is a plain value the product owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppInfo {
    /// Short name in sentences and menus.
    pub name: &'static str,
    /// Full name where the short one is too vague (window titles, user agents).
    pub full_name: &'static str,
    /// Lower-case technical name: configuration and cache folders, the
    /// application name of a database session, thread names.
    pub slug: &'static str,
    /// Prefix of the environment variables, without the trailing `_`.
    pub env_prefix: &'static str,
    /// Name of the product's folder in a project root, with the leading dot.
    pub project_dir: &'static str,
    /// Reverse-DNS identifier of the application bundle.
    pub bundle_id: &'static str,
    /// Name of the command-line program.
    pub cli_name: &'static str,
    /// Name of the database command-line program.
    pub db_cli_name: &'static str,
    /// Every program the product ships (the IDE, the command-line programs):
    /// the ones that share the OS credential store.
    pub binaries: &'static [&'static str],
}

impl AppInfo {
    /// The names used when the product installed none.
    pub const NEUTRAL: AppInfo = AppInfo {
        name: "App",
        full_name: "App",
        slug: "app",
        env_prefix: "APP",
        project_dir: ".app",
        bundle_id: "app.example.app",
        cli_name: "app",
        db_cli_name: "app-db",
        binaries: &["app", "app-db"],
    };
}

impl Default for AppInfo {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

static INFO: OnceLock<AppInfo> = OnceLock::new();

/// Installs the product's names. The first call wins; returns whether this
/// call installed them (a second call with other names changes nothing).
pub fn install(info: AppInfo) -> bool {
    INFO.set(info).is_ok()
}

/// The installed names, or [`AppInfo::NEUTRAL`].
pub fn get() -> &'static AppInfo {
    INFO.get().unwrap_or(&AppInfo::NEUTRAL)
}

/// The short product name.
pub fn name() -> &'static str {
    get().name
}

/// The full product name.
pub fn full_name() -> &'static str {
    get().full_name
}

/// The technical name.
pub fn slug() -> &'static str {
    get().slug
}

/// The environment variable prefix, without the trailing `_`.
pub fn env_prefix() -> &'static str {
    get().env_prefix
}

/// The product's folder name in a project root.
pub fn project_dir() -> &'static str {
    get().project_dir
}

/// The bundle identifier.
pub fn bundle_id() -> &'static str {
    get().bundle_id
}

/// The command-line program's name.
pub fn cli_name() -> &'static str {
    get().cli_name
}

/// The database command-line program's name.
pub fn db_cli_name() -> &'static str {
    get().db_cli_name
}

/// The programs the product ships.
pub fn binaries() -> &'static [&'static str] {
    get().binaries
}

/// The name of the product's environment variable `suffix`:
/// `<prefix>_<suffix>`.
pub fn env_name(suffix: &str) -> String {
    format!("{}_{suffix}", env_prefix())
}

/// The value of the product's environment variable `suffix` (see
/// [`env_name`]); `None` when it is unset or not Unicode.
pub fn var(suffix: &str) -> Option<String> {
    std::env::var(env_name(suffix)).ok()
}

/// Like [`var`], without requiring Unicode.
pub fn var_os(suffix: &str) -> Option<OsString> {
    std::env::var_os(env_name(suffix))
}

/// The application's configuration folder: the environment variable
/// `<prefix>_CONFIG_DIR`, else [`slug`] in the user's configuration folder.
pub fn config_dir() -> Option<PathBuf> {
    match var_os("CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(dirs::config_dir()?.join(slug())),
    }
}

/// The application's cache folder: the environment variable
/// `<prefix>_CACHE_DIR`, else [`slug`] in the user's cache folder
/// (`~/Library/Caches` on macOS, `$XDG_CACHE_HOME` or `~/.cache` on Linux,
/// the local application data folder on Windows).
pub fn cache_dir() -> Option<PathBuf> {
    match var_os("CACHE_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(dirs::cache_dir()?.join(slug())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_names_join_prefix_and_suffix() {
        assert_eq!(env_name("CONFIG_DIR"), "APP_CONFIG_DIR");
    }

    #[test]
    fn the_neutral_names_are_the_default() {
        assert_eq!(AppInfo::default(), AppInfo::NEUTRAL);
        assert_eq!(slug(), "app");
        assert_eq!(project_dir(), ".app");
    }
}
