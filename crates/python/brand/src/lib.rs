//! The product's name in one place (docs/rebrand.md).
//!
//! Every other crate of the product layer takes the name from here, and the
//! platform gets it at startup through [`install`] (`app-info`). The
//! values below are what existing users have on disk (settings and cache
//! folders, keychain items, the project folder, environment variables), so
//! changing one is a migration, not an edit. `scripts/check-brand.py` fails
//! on the name anywhere else in the code.

use app_info::AppInfo;

macro_rules! slug {
    () => {
        "pyoxide"
    };
}

/// The prefix of the environment variables, as a literal for `concat!`.
#[doc(hidden)]
#[macro_export]
macro_rules! env_prefix {
    () => {
        "PYOXIDE"
    };
}

/// Name of an environment variable of the product built at compile time:
/// `brand::env_name!("UV")` is `"<PREFIX>_UV"`. For `concat!` and
/// attributes; at run time `app_info::env_name` does the same.
#[macro_export]
macro_rules! env_name {
    ($suffix:literal) => {
        concat!($crate::env_prefix!(), "_", $suffix)
    };
}

/// Short name, in sentences and menus.
pub const PRODUCT_NAME: &str = "Pyoxide";

/// Full name, in window titles and user agents.
pub const PRODUCT_FULL_NAME: &str = "Pyoxide IDE";

/// Lower-case technical name: configuration and cache folders, the
/// application name of a database session, the keychain service prefix.
pub const SLUG: &str = slug!();

/// Prefix of the environment variables (`<ENV_PREFIX>_CONFIG_DIR`, ...).
pub const ENV_PREFIX: &str = env_prefix!();

/// The product's folder in a project root.
pub const PROJECT_DIR: &str = ".oxide";

/// Identifier of the application bundle.
pub const BUNDLE_ID: &str = "dev.pyoxide.ide";

/// The command-line program.
pub const CLI_NAME: &str = slug!();

/// The database command-line program, and the keychain service of the
/// database passwords it shares with the IDE.
pub const DB_CLI_NAME: &str = concat!(slug!(), "-db");

/// The IDE program.
pub const IDE_NAME: &str = concat!(slug!(), "-ide");

/// The product's names as the platform takes them.
pub const INFO: AppInfo = AppInfo {
    name: PRODUCT_NAME,
    full_name: PRODUCT_FULL_NAME,
    slug: SLUG,
    env_prefix: ENV_PREFIX,
    project_dir: PROJECT_DIR,
    bundle_id: BUNDLE_ID,
    cli_name: CLI_NAME,
    db_cli_name: DB_CLI_NAME,
    binaries: &[IDE_NAME, DB_CLI_NAME, CLI_NAME],
};

/// Tells the platform the product's names. Call first in `main`, before
/// anything reads a setting.
pub fn install() {
    app_info::install(INFO);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_values_stay_what_existing_users_have_on_disk() {
        assert_eq!(PRODUCT_NAME, "Pyoxide");
        assert_eq!(SLUG, "pyoxide");
        assert_eq!(ENV_PREFIX, "PYOXIDE");
        assert_eq!(PROJECT_DIR, ".oxide");
        assert_eq!(DB_CLI_NAME, "pyoxide-db");
        assert_eq!(IDE_NAME, "pyoxide-ide");
        assert_eq!(INFO.binaries, ["pyoxide-ide", "pyoxide-db", "pyoxide"]);
        assert_eq!(env_name!("CONFIG_DIR"), "PYOXIDE_CONFIG_DIR");
    }

    #[test]
    fn install_gives_the_platform_the_names() {
        install();
        assert_eq!(app_info::env_name("UV"), env_name!("UV"));
        assert_eq!(app_info::slug(), SLUG);
    }
}
