//! Import of settings from other IDEs, starting with PyCharm data sources
//! and their saved passwords (see the crate README), the reference's
//! application options, and the snapshot an import keeps of them
//! ([`snapshot`]).
//!
//! Output is plain data; turning it into Pyoxide profiles and storing the
//! passwords is up to the caller. Only the user's own files are read, and
//! secret values only travel as [`secrets::Secret`].

use std::path::PathBuf;

pub mod pycharm;
pub mod snapshot;
mod xml;

pub use xml::Element;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}: malformed XML: {message}", path.display())]
    Xml { path: PathBuf, message: String },
    #[error("{0}")]
    NotFound(String),
}
