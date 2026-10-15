//! Run configurations of Pyoxide IDE (ADR 0012): named, repeatable
//! launches, as the reference IDE keeps them (`RunManagerImpl`,
//! `RunnerAndConfigurationSettingsImpl`), without a UI toolkit.
//!
//! - [`RunConfiguration`]: what every type has (name, type, folder,
//!   storage, temporary, allow multiple instances, environment, before-launch
//!   tasks) and the options of its type, as a table the type reads.
//! - [`RunConfigurations`]: a project's configurations with the selection,
//!   the custom order, the temporary ones and their limit, pinned ones,
//!   the history and the last executors; the reference's list order.
//! - [`store`]: shared configurations in `.oxide/run/` (one file each,
//!   meant to be committed) and everything personal in the personal
//!   project state; environment values of personal variables never reach
//!   a committed file.
//! - [`idea`] and [`import`]: the one-time import of the reference's
//!   configurations (`.idea/workspace.xml`, `.idea/runConfigurations`,
//!   `*.run.xml`); `.idea` is only read. Products convert their types with
//!   an [`Importer`]; configurations of other types are kept read-only.
//! - [`shell`]: the Shell Script type, which knows no language.
//! - [`form`]: what the Run/Debug Configurations dialog edits: the fields
//!   and "Modify options" of a type ([`TypeEditor`]).
//! - [`params`], [`paths`], [`env_file`]: parameter strings, path
//!   variables and environment files as the reference reads them.
//!
//! Everything here is blocking: callers run it off the UI thread.

pub mod env_file;
pub mod form;
pub mod idea;
pub mod import;
pub mod list;
mod model;
pub mod natural;
pub mod params;
pub mod paths;
pub mod shell;
pub mod store;
pub mod xml;

pub use form::{Field, FieldKind, OptionItem, TypeEditor};
pub use idea::{IdeaConfiguration, Importer};
pub use import::{Opened, Report};
pub use list::{RECENT_COUNT, RunConfigurations, TEMPORARY_LIMIT};
pub use model::{
    BeforeLaunchTask, ConfigKey, EnvVar, Reference, RunConfiguration, Storage, TypeInfo, Types,
    UNSUPPORTED_PREFIX,
};
pub use store::StoreError;
