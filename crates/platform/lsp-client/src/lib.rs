//! The Language Server Protocol client of Pyoxide IDE (ADR 0010).
//!
//! A product describes its servers with [`ServerSpec`]s (command line,
//! root, languages, the features each serves, settings, a
//! [`ServerCustomizer`]); [`LspCodeIntel`] runs them and implements
//! [`code_intel::CodeIntel`] over all of them, routing each feature to the
//! servers that serve it.
//!
//! The design (one `process` child per server, a reader thread that parses
//! frames and answers pending requests through oneshot channels, requests
//! cancelled by dropping their futures, no async runtime) is in ADR 0010.
//! Protocol types (`lsp-types`) never appear in the public API.
//!
//! This is the skeleton of the public API: servers are not started yet,
//! requests answer [`code_intel::IntelError::Unsupported`].

mod customizer;
mod intel;
mod spec;

pub use customizer::{NoCustomization, ServerCustomizer, lookup_section};
pub use intel::LspCodeIntel;
pub use spec::ServerSpec;
